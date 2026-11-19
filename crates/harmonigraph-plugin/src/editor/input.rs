use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use egui_baseview::baseview::FrameTiming;

use super::clock::ClockMapper;

/// The clock frames draw by, the only part of the input a frame reads: it
/// stays under the editor lock, while the input itself ([`LiveInput`]) is the
/// input worker's.
pub(crate) struct DisplayClock {
    /// Latest rate is for recording controls, never for interpreting queued audio.
    pub(super) sample_rate_bits: Arc<AtomicU32>,
    /// Wall-clock epoch, also used to age the picture when callbacks stop.
    pub(super) start: Instant,
    /// Audio->GUI clock mapping (see ClockMapper).
    pub(super) mapper: ClockMapper,
}

/// The sole display and analyzer ingress, retained for the plugin lifetime.
/// Synchronous draining never acquires the audio callback or a window.
///
/// The input worker owns it ([`crate::background`]) and nothing under the
/// editor lock reads it: a step takes the lock for what the drain writes,
/// the tracker and the [`DisplayClock`], and to hand the columns to the
/// spectrum. So the analysis never leaves it, and a step that panicked
/// anywhere leaves it here for [`Self::restart_analysis`].
pub(crate) struct LiveInput {
    consumer: harmonigraph_record::publication::Consumer,
    audio_consumer: crate::audio_ingress::Consumer,
    /// Expected next source frame and epoch; gaps restart the streaming analyzer.
    audio_position: Option<(u64, u64)>,
    /// Presentation seconds of the retained run’s frame zero, before mapping.
    audio_origin: f64,
    /// What a [`Taken`] carries, in buffers kept from one to the next, so a
    /// step allocates only when it needs more than any before it.
    scratch: Scratch,
    /// The analysis over this input, dated from `audio_origin`.
    analysis: harmonigraph_ui::SpectrumAnalysis,
    recorder: crate::tape::notes::Recorder,
}

/// The audio one [`LiveInput::take`] moved out of the ring, until
/// [`Taken::give`] hands what [`Taken::analyze`] made of it to the spectrum:
/// a borrow of the input, so the three run in that order, once each, and
/// nothing else drains the input in between. A step holds it across the
/// lock's release, which is where the FFTs run.
pub(crate) struct Taken<'a> {
    input: &'a mut LiveInput,
}

/// [`Taken`]'s buffers.
#[derive(Default)]
struct Scratch {
    /// The settings when the audio was taken, which analyze it and smooth
    /// the curve it moves.
    config: harmonigraph_ui::SpectrumConfig,
    /// The presentation clock's offset onto the display's, at take.
    offset: f64,
    /// Every block's samples, interleaved, back to back.
    samples: Vec<f32>,
    blocks: Vec<TakenBlock>,
    deliveries: Vec<harmonigraph_ui::Delivery>,
}

/// One callback's block of [`Scratch::samples`].
struct TakenBlock {
    /// The block starts a new retained source run.
    restart: bool,
    /// Frame zero of its run on the GUI clock, mapped at take.
    origin: f64,
    len: usize,
    /// The descriptor as drained: its format for the analysis, all of it for
    /// the tape.
    block: crate::audio_ingress::Block,
}

impl Taken<'_> {
    /// Each block's descriptor and samples, in drain order.
    pub(crate) fn blocks(&self) -> impl Iterator<Item = (crate::audio_ingress::Block, &[f32])> {
        let scratch = &self.input.scratch;
        let mut start = 0;
        scratch.blocks.iter().map(move |block| {
            let samples = &scratch.samples[start..start + block.len];
            start += block.len;
            (block.block, samples)
        })
    }
    /// Each block's descriptor and the display time of its first frame.
    pub(crate) fn clocked_blocks(
        &self,
    ) -> impl Iterator<Item = (crate::audio_ingress::Block, f64)> + '_ {
        let scratch = &self.input.scratch;
        let offset = scratch.offset;
        scratch.blocks.iter().map(move |taken| (taken.block, taken.block.presentation() + offset))
    }
    /// Whether this step found audio, and has analyzed none of it yet.
    #[cfg(test)]
    pub(crate) fn unanalyzed_audio(&self) -> bool {
        let scratch = &self.input.scratch;
        !scratch.blocks.is_empty() && scratch.deliveries.is_empty()
    }
    /// The columns the analysis has made so far, as `(time, power)`.
    #[cfg(test)]
    pub(crate) fn columns(&self) -> Vec<(f64, Vec<f32>)> {
        (self.input.scratch.deliveries.iter())
            .filter_map(harmonigraph_ui::Delivery::column)
            .map(|column| (column.time(), column.power().to_vec()))
            .collect()
    }
    /// Run the FFTs over what was taken, with no lock held.
    pub(crate) fn analyze(&mut self) {
        let LiveInput { analysis, scratch, .. } = &mut *self.input;
        let mut start = 0;
        for block in &scratch.blocks {
            if block.restart {
                analysis.restart();
                scratch.deliveries.push(harmonigraph_ui::Delivery::Restart);
            }
            let samples = &scratch.samples[start..start + block.len];
            start += block.len;
            analysis.feed_source(
                samples,
                block.block.format.channels,
                block.block.format.sample_rate,
                block.origin,
                &scratch.config,
                |delivery| scratch.deliveries.push(delivery),
            );
        }
    }
    /// Hand what [`Self::analyze`] made to the spectrum. Whether that brought
    /// sound back to a spectrum gone still
    /// ([`AudioSpectrum::give`](harmonigraph_ui::AudioSpectrum::give)).
    pub(crate) fn give(self, runtime: &mut harmonigraph_ui::VisualRuntime) -> bool {
        let scratch = &mut self.input.scratch;
        runtime.spectrum.give(scratch.deliveries.drain(..), &scratch.config)
    }
}

impl LiveInput {
    pub(crate) fn new(
        consumer: harmonigraph_record::publication::Consumer,
        audio_consumer: crate::audio_ingress::Consumer,
    ) -> Self {
        LiveInput {
            consumer,
            audio_consumer,
            audio_position: None,
            audio_origin: 0.0,
            scratch: Scratch::default(),
            analysis: Default::default(),
            recorder: Default::default(),
        }
    }
    /// Drain exactly what the process callback sent to the live analyzer.
    /// Test-only because production drains through [`Self::take`], which also
    /// maintains the analyzer state around it.
    #[cfg(test)]
    pub(crate) fn drain_analysis_audio_for_test(&mut self) -> Vec<f32> {
        let mut samples = Vec::new();
        self.audio_consumer.drain(|_, chunk| samples.extend_from_slice(chunk));
        samples
    }
    /// Drain note events from the audio thread into the tracker, mapping
    /// their sample-clock timestamps onto the GUI clock, and what the display
    /// did to its voices into `marks`, at presentation times, for the tape to
    /// place by song position (bounded by [`crate::tape::notes::NOTES`]).
    /// A fresh audio heartbeat anchors the whole stream before historical
    /// records are mapped; delayed batches never reset that anchor.
    fn drain_into_tracker(
        &mut self,
        clock: &mut DisplayClock,
        picture: &mut harmonigraph_ui::PictureState,
        marks: &mut Vec<(f64, crate::tape::notes::Mark)>,
        now: f64,
    ) {
        let clock = &mut clock.mapper;
        if let Some(observation) = self.consumer.clock() {
            clock.observe(observation, now);
        }
        let Some(offset) = clock.offset else { return };
        let tracker = &mut picture.runtime.tracker;
        let recorder = &mut self.recorder;
        self.consumer.drain(|event| {
            let result = tracker.handle_canonical_mapped(event, offset);
            debug_assert!(result.is_ok(), "validated canonical publication");
            recorder.record(tracker, event.time(), marks);
            true
        });
    }
    /// Move the audio ring's blocks out, mapped to the GUI clock with the
    /// offset [`Self::drain_into_tracker`] has just observed; `None` until a
    /// heartbeat has given that offset, leaving the audio in the ring as
    /// before, and when the ring held none.
    ///
    /// Always drains AND (in [`Taken::analyze`]) always feeds. There is no
    /// display state that makes the samples unwanted: the curve and the
    /// spectrogram read this one analyzer, the curve is always drawn, and
    /// feeding it is also what drives smooth repaint while anything it heard
    /// is still on screen (`harmonigraph_ui::Animating::audible`).
    fn take_audio(
        &mut self,
        clock: &DisplayClock,
        picture: &harmonigraph_ui::PictureState,
    ) -> Option<Taken<'_>> {
        let audio = &mut self.scratch;
        // Emptied at every take rather than once given, so what a step that
        // panicked before giving left behind is never given again.
        audio.samples.clear();
        audio.blocks.clear();
        audio.deliveries.clear();
        // One clock conversion per take, shared with note delivery and made
        // in the same hold, observed and then mapped as a frame's drain did.
        let offset = clock.mapper.offset?;
        audio.config = picture.appearance.spectrum;
        audio.offset = offset;
        self.audio_consumer.drain(|block, samples| {
            let position = (block.epoch, block.first_frame);
            let restart = self.audio_position != Some(position);
            if restart {
                self.audio_origin = block.presentation();
            }
            audio.samples.extend_from_slice(samples);
            audio.blocks.push(TakenBlock {
                restart,
                origin: self.audio_origin + offset,
                len: samples.len(),
                block,
            });
            self.audio_position = Some((block.epoch, block.first_frame + block.frames as u64));
        });
        // Nothing to analyze: spare the caller a second hold to give back
        // nothing.
        (!self.scratch.blocks.is_empty()).then_some(Taken { input: self })
    }
    /// The lag the analysis runs at, which tells a test the resolution it
    /// analyzes under.
    #[cfg(test)]
    pub(crate) fn analysis_lag(&self) -> f64 {
        self.analysis.lag()
    }
    /// Start a fresh analysis after a step panicked, with the one it left
    /// in whatever state the panic did, and start the source run over, so the
    /// next block re-anchors the fresh analyzer's grid instead of extending
    /// the interrupted run's. Needs no lock: nothing under it is touched.
    pub(crate) fn restart_analysis(&mut self) {
        self.analysis = Default::default();
        self.audio_position = None;
    }
    /// The locked half of a step: the notes into the tracker and their
    /// marks into `marks` ([`Self::drain_into_tracker`]), then the audio out
    /// of its ring, for [`Taken::analyze`] then [`Taken::give`].
    /// Input feeding does not prune: its callers age the picture after it,
    /// the input worker in the same hold and a frame in `begin_frame`.
    pub(crate) fn take(
        &mut self,
        clock: &mut DisplayClock,
        picture: &mut harmonigraph_ui::PictureState,
        marks: &mut Vec<(f64, crate::tape::notes::Mark)>,
        now: f64,
    ) -> Option<Taken<'_>> {
        self.drain_into_tracker(clock, picture, marks, now);
        self.take_audio(clock, picture)
    }
    /// A whole step with the lock held throughout: take, analyze, give. The
    /// input worker splits the same three around its lock; this is them in
    /// one, for its final drain and for tests.
    pub(crate) fn drain(
        &mut self,
        clock: &mut DisplayClock,
        picture: &mut harmonigraph_ui::PictureState,
        marks: &mut Vec<(f64, crate::tape::notes::Mark)>,
        now: f64,
    ) {
        if let Some(mut taken) = self.take(clock, picture, marks, now) {
            taken.analyze();
            taken.give(&mut picture.runtime);
        }
    }
    /// [`Self::drain`] with a delivery tap: the columns its analysis made, as
    /// `(time, power)`, whether or not the history kept them.
    #[cfg(test)]
    pub(crate) fn drain_columns(
        &mut self,
        clock: &mut DisplayClock,
        picture: &mut harmonigraph_ui::PictureState,
        now: f64,
    ) -> Vec<(f64, Vec<f32>)> {
        let Some(mut taken) = self.take(clock, picture, &mut Vec::new(), now) else {
            return Vec::new();
        };
        taken.analyze();
        let columns = taken.columns();
        taken.give(&mut picture.runtime);
        columns
    }
}

impl DisplayClock {
    /// The host sample rate the audio thread published, as an f32. It lives
    /// in an `AtomicU32` as the f32's bit pattern (a lock-free f32); this
    /// names the bit-cast so its readers can't spell it inconsistently.
    pub(super) fn sample_rate(&self) -> f32 {
        f32::from_bits(self.sample_rate_bits.load(Ordering::Relaxed))
    }
    /// The wall clock used to observe incoming batches: seconds since
    /// the PLUGIN was instantiated, not since the window opened.
    ///
    /// That distinction is what makes a closed window recoverable at all. The
    /// clock runs across one, so a column analyzed while nothing was drawing
    /// lands on the same axis as the columns either side of it, and the
    /// heatmap has no seam to hide.
    pub(crate) fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
    /// The wall time a frame draws at: the refresh the display link means it
    /// to be on screen by, on [`Self::now`]'s clock, or `observed` for a frame
    /// the link did not ask for. Such a frame between link frames is behind
    /// the last target, so [`Self::display_now`] holds it at the time already
    /// drawn, for that one frame.
    ///
    /// The refresh rather than when the callback got to run, because the
    /// callback runs on the host's main thread whenever the host lets it —
    /// in a Bitwig session from under a millisecond to nearly two refreshes
    /// after the vsync it was issued for. Timed by the callback, 8.8% of the
    /// frames that reached the glass one refresh after the last stepped their
    /// content more than a millisecond off that, and the roll scrolled
    /// unevenly with no frame missed. Timed by the refresh, 2.2% did, which
    /// are the switches in drawable depth (#1551).
    ///
    /// The target, not the vsync the callback was issued for, because the
    /// target is still ahead when the frame starts (on all but 2 of 11,425
    /// frames in that log), so content the drain just stamped is never dated
    /// past the drawn time. Every drawn time moves about a refresh later than
    /// the callback's, which is when the glass shows it.
    ///
    /// A target [`FrameTiming::target_at`] refuses, as too far from the link's
    /// fire to be this frame's, draws at `observed` too: the drawn time
    /// never rewinds, so one far ahead would hold the picture still.
    pub(crate) fn frame_wall(&self, timing: Option<FrameTiming>, observed: f64) -> f64 {
        timing
            .and_then(|timing| timing.target_at())
            .map_or(observed, |target| target.saturating_duration_since(self.start).as_secs_f64())
    }
    pub(crate) fn display_now(&mut self, wall_now: f64) -> f64 {
        self.mapper.now(wall_now)
    }
}

#[cfg(test)]
mod tests {
    use super::super::shared::EditorShared;
    use super::{FrameTiming, LiveInput};
    use harmonigraph_core::notes::{NoteEvent, SourceId};
    use std::sync::Arc;

    fn editor() -> EditorShared {
        let (_recorder, control) = harmonigraph_record::channel();
        EditorShared::new(Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())), control)
    }

    fn audio_harness(capacity: usize) -> (crate::audio_ingress::Producer, EditorShared, LiveInput) {
        let (_notes, consumer) = harmonigraph_record::publication::channel();
        let (audio, audio_consumer) = crate::audio_ingress::channel(capacity);
        let mut shared = editor();
        shared.clock.mapper.observe(0.0, 0.0);
        (audio, shared, LiveInput::new(consumer, audio_consumer))
    }

    fn drain_audio(input: &mut LiveInput, shared: &mut EditorShared, now: f64) {
        input.drain(&mut shared.clock, &mut shared.ui.picture, &mut Vec::new(), now);
    }

    /// Callbacks a 144 Hz display link ran late by these amounts — the spread
    /// a Bitwig session's frame log shows, p50 0.65 ms to over a refresh — draw
    /// one refresh apart, as the glass shows them, and no earlier than they ran
    /// unless they ran over a refresh late; a frame the link did not ask for
    /// draws when it ran.
    #[test]
    fn frames_draw_at_their_refresh_however_late_the_callback_ran() {
        let (_audio, mut shared, _input) = audio_harness(1024);
        let refresh = 1.0 / 144.0;
        let start = shared.clock.start;
        // The media clock's origin is unrelated to the plugin's.
        let media = 322_510.0;
        let mut drawn = Vec::new();
        for (frame, late_ms) in [0.3, 5.1, 0.4, 9.0, 0.6, 2.2, 0.2].into_iter().enumerate() {
            let vsync = 1.0 + frame as f64 * refresh;
            let fired = vsync + late_ms / 1000.0;
            let timing = FrameTiming {
                fired: media + fired,
                fired_at: start + std::time::Duration::from_secs_f64(fired),
                timestamp: media + vsync,
                target_timestamp: media + vsync + refresh,
                refresh,
                divisor: 1,
            };
            let wall = shared.clock.frame_wall(Some(timing), fired);
            if late_ms / 1000.0 < refresh {
                assert!(wall >= fired, "drawn before its own callback ran, at {late_ms} ms late");
            }
            drawn.push(shared.clock.display_now(wall));
        }
        for step in drawn.windows(2).map(|pair| pair[1] - pair[0]) {
            assert!((step - refresh).abs() < 1e-6, "stepped {step} s against {refresh} s");
        }

        let observed = drawn.last().unwrap() + 0.05;
        assert_eq!(shared.clock.frame_wall(None, observed), observed);
        // A target AppKit got wrong, which would hold the picture still.
        for target_timestamp in [media + observed + 5.0, f64::NAN] {
            let fired_at = start + std::time::Duration::from_secs_f64(observed);
            let timing = FrameTiming {
                fired: media + observed,
                fired_at,
                timestamp: media + observed,
                target_timestamp,
                refresh,
                divisor: 1,
            };
            assert_eq!(shared.clock.frame_wall(Some(timing), observed), observed);
        }
    }

    #[test]
    fn source_audio_columns_survive_callback_partitions_and_delayed_drains() {
        let frames = 50_017;
        let samples: Vec<_> = (0..frames)
            .flat_map(|i| {
                let x = (i as f32 * 0.047).sin();
                [x, -x]
            })
            .collect();
        let mut reference = None;
        for (callback, drain_every, delay) in
            [(frames, 1, 0.0), (127, 1, 0.0), (511, 7, 0.3), (4093, 99, 3.0)]
        {
            let (mut tx, mut shared, mut input) = audio_harness(crate::AUDIO_RING_CAPACITY);
            let mut count = 0;
            let mut actual = Vec::new();
            for first in (0..frames).step_by(callback) {
                let end = (first + callback).min(frames);
                tx.publish(
                    end - first,
                    crate::audio_ingress::Format {
                        channels: 2,
                        sample_rate: 48_000.0,
                        sidechain: false,
                    },
                    10.0 + first as f64 / 48_000.0,
                    None,
                    crate::audio_ingress::TapePlay::Idle,
                    samples[first * 2..end * 2].iter().copied(),
                );
                count += 1;
                if count % drain_every == 0 {
                    let now = 10.0 + end as f64 / 48_000.0 + delay;
                    actual.extend(input.drain_columns(
                        &mut shared.clock,
                        &mut shared.ui.picture,
                        now,
                    ));
                }
            }
            actual.extend(input.drain_columns(
                &mut shared.clock,
                &mut shared.ui.picture,
                15.0 + delay,
            ));
            assert!(actual.len() > 50, "fixture must fill the window and reach many FFT hops");
            if let Some(expected) = &reference {
                assert_eq!(&actual, expected, "callback={callback}, drain_every={drain_every}");
            } else {
                reference = Some(actual);
            }
        }
    }

    /// A step that finds the whole ring waiting, as a faster-than-realtime
    /// bounce leaves it with the window shut, must not leave megabytes in the
    /// spare delivery list for the plugin's lifetime. It holds an entry per
    /// callback, and 64-frame blocks filled 2,048 slots, 31 MB of them while
    /// every slot was sized for a column.
    #[test]
    fn a_full_ring_step_leaves_a_small_spare_delivery_list() {
        let (mut tx, mut shared, mut input) = audio_harness(crate::AUDIO_RING_CAPACITY);
        let format =
            crate::audio_ingress::Format { channels: 2, sample_rate: 48_000.0, sidechain: false };
        let block = 64;
        let frames = crate::AUDIO_RING_CAPACITY / 2;
        for first in (0..frames).step_by(block) {
            tx.publish(
                block,
                format,
                first as f64 / 48_000.0,
                None,
                crate::audio_ingress::TapePlay::Idle,
                (0..block * 2).map(|i| ((first * 2 + i) as f32 * 0.047).sin()),
            );
        }
        drain_audio(&mut input, &mut shared, 5.0);
        let spare = &input.scratch;
        assert!(spare.deliveries.capacity() >= frames / block, "the step must take every block");
        let held = spare.deliveries.capacity() * size_of::<harmonigraph_ui::Delivery>();
        assert!(held < 100_000, "the spare delivery list holds {held} bytes");
    }

    /// Several seconds across many drains, so a wall-clock mapper has time to
    /// squeeze successive batches together. Compare the real analyzer and note
    /// publication paths, not just timestamps computed by the clock in isolation.
    #[test]
    fn fast_bounce_preserves_the_realtime_picture_timeline() {
        let run = |speed: f64| {
            let (mut notes, consumer) = harmonigraph_record::publication::channel();
            let (mut audio, audio_consumer) =
                crate::audio_ingress::channel(crate::AUDIO_RING_CAPACITY);
            let mut shared = editor();
            let mut input = LiveInput::new(consumer, audio_consumer);
            notes.observe_clock(0.0);
            drain_audio(&mut input, &mut shared, 0.0);
            let format = crate::audio_ingress::Format {
                channels: 1,
                sample_rate: 48_000.0,
                sidechain: false,
            };
            let mut now = 0.0;
            let mut columns = Vec::new();
            for batch in 0..16usize {
                let start = batch as f64 * 0.25;
                let end = start + 0.25;
                audio.publish(
                    12_000,
                    format,
                    start,
                    None,
                    crate::audio_ingress::TapePlay::Idle,
                    (0..12_000).map(|i| ((batch * 12_000 + i) as f32 * 0.047).sin()),
                );
                notes
                    .note(
                        NoteEvent::on(start + 0.125, SourceId::DIRECT, 0, 48 + batch as u8, 0.8)
                            .into(),
                    )
                    .unwrap();
                notes.observe_clock(end);
                columns.extend(input.drain_columns(
                    &mut shared.clock,
                    &mut shared.ui.picture,
                    end / speed,
                ));
                now = shared.clock.display_now(end / speed);
                shared.ui.picture.runtime.advance_time(now, &shared.ui.picture.appearance);
            }
            let onsets: Vec<_> =
                shared.ui.picture.runtime.tracker.roll().notes().map(|n| n.start).collect();
            assert!(columns.len() > 400, "fixture must retain four seconds of FFT hops");
            assert_eq!(onsets.len(), 16);
            (now, columns, onsets)
        };
        let realtime = run(1.0);
        assert_eq!(realtime.0, 4.0);
        for speed in [8.0, 32.0] {
            let fast = run(speed);
            assert_eq!(fast.0, realtime.0, "{speed}x changed the playhead");
            assert_eq!(fast.2, realtime.2, "{speed}x retimed the notes");
            assert!(fast.1 == realtime.1, "{speed}x changed the spectrogram columns");
        }
    }

    #[test]
    fn source_discontinuities_refill_the_window_without_erasing_history() {
        let window = harmonigraph_ui::SpectrumConfig::default().window.frame(48_000.0);
        let (mut tx, mut shared, mut input) = audio_harness(window + 384);
        let mono =
            crate::audio_ingress::Format { channels: 1, sample_rate: 48_000.0, sidechain: false };
        // Retain a full window plus a hop, then drop an entire callback.
        tx.publish(
            window + 384,
            mono,
            0.0,
            None,
            crate::audio_ingress::TapePlay::Idle,
            std::iter::repeat(0.5),
        );
        tx.publish(
            window,
            mono,
            1.0,
            None,
            crate::audio_ingress::TapePlay::Idle,
            std::iter::repeat(-0.5),
        );
        drain_audio(&mut input, &mut shared, 1.0);
        let before = shared.ui.picture.runtime.spectrum.columns_taken();
        assert!(before > 0);
        // A gap cannot complete a window using the previous run's tail.
        tx.publish(
            window / 2,
            mono,
            2.0,
            None,
            crate::audio_ingress::TapePlay::Idle,
            std::iter::repeat(-0.5),
        );
        drain_audio(&mut input, &mut shared, 2.0);
        assert_eq!(shared.ui.picture.runtime.spectrum.columns_taken(), before);
        // Queue old-format partial audio before a reset and new-format audio.
        tx.publish(
            window / 4,
            mono,
            3.0,
            None,
            crate::audio_ingress::TapePlay::Idle,
            std::iter::repeat(0.9),
        );
        tx.reset();
        let stereo =
            crate::audio_ingress::Format { channels: 2, sample_rate: 96_000.0, sidechain: true };
        // The frame is a length of time, so at 96 kHz it is twice the samples.
        let refill = harmonigraph_ui::SpectrumConfig::default().window.frame(96_000.0);
        tx.publish(
            refill / 4,
            stereo,
            4.0,
            None,
            crate::audio_ingress::TapePlay::Idle,
            std::iter::repeat(0.0),
        );
        let mut refilled = input.drain_columns(&mut shared.clock, &mut shared.ui.picture, 4.0);
        assert_eq!(shared.ui.picture.runtime.spectrum.columns_taken(), before);
        for i in 1..=4 {
            tx.publish(
                refill / 4,
                stereo,
                4.0 + i as f64 * refill as f64 / 4.0 / 96_000.0,
                None,
                crate::audio_ingress::TapePlay::Idle,
                std::iter::repeat(0.0),
            );
            refilled.extend(input.drain_columns(&mut shared.clock, &mut shared.ui.picture, 5.0));
        }
        assert!(!refilled.is_empty(), "new format must refill and emit columns");
        for (time, power) in &refilled {
            assert!(*time > 4.0, "no window may straddle the reset");
            assert!(
                power.iter().all(|p| harmonigraph_core::spectrogram::quantize(*p) == 0),
                "no old-format energy may leak into silence"
            );
        }
    }

    #[test]
    fn drain_observes_the_batch_before_mapping_it() {
        // The audio clock reads ~100s while the GUI clock reads ~7s; the
        // drained voices must land on the GUI clock with their intra-batch
        // spacing intact. (This is the integration the ClockMapper unit
        // tests below can't cover: observe-newest-THEN-map ordering.)
        let (mut producer, consumer) = harmonigraph_record::publication::channel();
        let (_audio_producer, audio_consumer) = crate::audio_ingress::channel(64);
        let mut shared = editor();
        let mut input = LiveInput::new(consumer, audio_consumer);
        for (source, time) in [(1, 99.950), (2, 99.995)] {
            let event = NoteEvent::on(time, SourceId(source), 0, 60, 1.0);
            producer
                .note(harmonigraph_core::canonical::NoteDelta {
                    assignment: None,
                    event,
                    sequence: 1,
                    lifetime: 1,
                    pitch_microcents: None,
                })
                .unwrap();
        }

        producer.observe_clock(99.995);
        input.drain_into_tracker(&mut shared.clock, &mut shared.ui.picture, &mut Vec::new(), 7.0);
        let mut on_times: Vec<f64> =
            shared.ui.picture.runtime.tracker.voices().map(|v| v.on_time).collect();
        on_times.sort_by(f64::total_cmp);
        assert_eq!(on_times.len(), 2);
        assert!((on_times[1] - on_times[0] - 0.045).abs() < 1e-9, "spacing lost: {on_times:?}");
        assert!(on_times[1] <= 7.0, "never maps into the GUI future");
        assert_eq!(
            shared.ui.picture.runtime.tracker.voices().map(|v| v.source.0).collect::<Vec<_>>(),
            vec![1, 2]
        );

        // An idle poll, a second and a half on, re-reads the same heartbeat.
        // That is not a new clock: it must not re-anchor the mapping, which
        // would read as a callback pause.
        let anchored = shared.clock.mapper.offset;
        input.drain_into_tracker(&mut shared.clock, &mut shared.ui.picture, &mut Vec::new(), 8.5);
        assert_eq!(shared.clock.mapper.offset, anchored, "an idle poll re-anchored the clock");
        producer.observe_clock(11.0);
        // Audio now=11 and GUI now=21. A delayed event from audio=2 belongs
        // at GUI=12; neither its batch nor a later idle poll is a new clock.
        use harmonigraph_core::canonical::NoteDelta;
        let accepted = NoteDelta {
            assignment: None,
            event: NoteEvent::on(2.0, SourceId(3), 0, 72, 0.8),
            sequence: 1,
            lifetime: 61,
            pitch_microcents: None,
        };
        producer.note(accepted).unwrap();
        let event = NoteEvent::on(2.0, SourceId::DIRECT, 0, 72, 0.8);
        producer.note(NoteDelta { event, lifetime: 62, ..accepted }).unwrap();
        input.drain_into_tracker(&mut shared.clock, &mut shared.ui.picture, &mut Vec::new(), 21.0);
        assert_eq!(
            shared
                .ui
                .picture
                .runtime
                .tracker
                .voices()
                .find(|v| v.source == SourceId::DIRECT)
                .unwrap()
                .on_time,
            12.0
        );
        use harmonigraph_core::canonical::{SourceBaseline, VoiceBaseline};
        let baseline = SourceBaseline::new(
            SourceId::DIRECT,
            3.0,
            1,
            true,
            &[VoiceBaseline {
                note: 72,
                lifetime: 62,
                actual_onset: 2.0,
                velocity: 0.8,
                pitch_microcents: 7_200_000_000,
                ..Default::default()
            }],
        )
        .unwrap();
        producer.observe_clock(12.0);
        producer.baseline(&baseline).unwrap();
        input.drain_into_tracker(&mut shared.clock, &mut shared.ui.picture, &mut Vec::new(), 22.2);
        let note = shared
            .ui
            .picture
            .runtime
            .tracker
            .roll()
            .notes()
            .find(|n| n.source == SourceId::DIRECT)
            .unwrap();
        assert_eq!(note.start, 12.0);
        assert!(note.history_complete, "the baseline continues the press its lifetime names");
        assert_eq!(
            shared
                .ui
                .picture
                .runtime
                .tracker
                .voices()
                .find(|v| v.source == SourceId::DIRECT)
                .unwrap()
                .on_time,
            12.0
        );
        assert_eq!(
            shared
                .ui
                .picture
                .runtime
                .tracker
                .roll()
                .notes()
                .filter(|n| n.source == SourceId::DIRECT)
                .count(),
            1
        );
        // A baseline restating source 3's note at a new pitch, which breaks
        // its line at the baseline's own time.
        let restated = SourceBaseline::new(
            SourceId(3),
            3.5,
            3,
            true,
            &[VoiceBaseline {
                note: 72,
                lifetime: 61,
                actual_onset: 2.0,
                velocity: 0.8,
                pitch_microcents: 7_225_000_000,
                ..Default::default()
            }],
        )
        .unwrap();
        producer.baseline(&restated).unwrap();
        input.drain_into_tracker(&mut shared.clock, &mut shared.ui.picture, &mut Vec::new(), 22.3);
        let voice =
            shared.ui.picture.runtime.tracker.voices().find(|v| v.source == SourceId(3)).unwrap();
        let note = shared
            .ui
            .picture
            .runtime
            .tracker
            .roll()
            .notes()
            .find(|v| v.source == SourceId(3))
            .unwrap();
        assert_eq!(
            (voice.on_time, note.start),
            (12.0, 12.0),
            "an accepted lifetime keeps its already-mapped onset"
        );
        // The baseline's own time, audio 3.5, is mapped onto the GUI clock too:
        // the restated pitch starts there.
        assert!(
            note.segments(22.3).any(|((start, _), _)| (start - 13.5).abs() < 1e-9),
            "the restating baseline lands at GUI 13.5"
        );
    }
}
