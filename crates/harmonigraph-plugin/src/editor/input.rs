use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use egui_baseview::baseview::FrameTiming;

use super::clock::ClockMapper;

/// The sole display and analyzer ingress, retained for the plugin lifetime.
/// Synchronous draining never acquires the audio callback or a window.
pub(crate) struct LiveInput {
    pub(super) consumer: harmonigraph_record::publication::Consumer,
    pub(super) audio_consumer: crate::audio_ingress::Consumer,
    /// Latest rate is for recording controls, never for interpreting queued audio.
    pub(super) sample_rate_bits: Arc<AtomicU32>,
    /// Expected next source frame and epoch; gaps restart the streaming analyzer.
    pub(super) audio_position: Option<(u64, u64)>,
    /// Presentation seconds of the retained run’s frame zero, before mapping.
    pub(super) audio_origin: f64,
    /// Wall-clock epoch, also used to age the picture when callbacks stop.
    pub(super) start: Instant,
    /// Audio->GUI clock mapping (see ClockMapper).
    pub(super) clock: ClockMapper,
    /// The buffers the last [`Taken`] came back in, so a step allocates only
    /// when it needs more than any before it.
    pub(super) spare: TakenAudio,
}

/// The audio one [`LiveInput::take`] moved out of the ring, and
/// `runtime.spectrum`'s analysis lent with it: what [`LiveInput::analyze`]
/// runs without the lock.
pub(crate) struct Taken {
    lent: harmonigraph_ui::LentAnalysis,
    audio: TakenAudio,
}

/// [`Taken`]'s buffers, which outlive it in [`LiveInput::spare`].
#[derive(Default)]
pub(crate) struct TakenAudio {
    /// The settings when the audio was taken, which analyze it and smooth
    /// the curve it moves.
    config: harmonigraph_ui::SpectrumConfig,
    /// Every block's samples, interleaved, back to back.
    samples: Vec<f32>,
    blocks: Vec<TakenBlock>,
    deliveries: Vec<harmonigraph_ui::Delivery>,
}

/// One callback's block of [`TakenAudio::samples`].
struct TakenBlock {
    /// The block starts a new retained source run.
    restart: bool,
    /// Frame zero of its run on the GUI clock, mapped at take.
    origin: f64,
    channels: usize,
    sample_rate: f32,
    len: usize,
}

impl Taken {
    /// Whether this step found audio, and has analyzed none of it yet.
    #[cfg(test)]
    pub(crate) fn unanalyzed_audio(&self) -> bool {
        !self.audio.blocks.is_empty() && self.audio.deliveries.is_empty()
    }
}

impl LiveInput {
    /// The host sample rate the audio thread published, as an f32. It lives
    /// in an `AtomicU32` as the f32's bit pattern (a lock-free f32); this
    /// names the bit-cast so its readers can't spell it inconsistently.
    pub(super) fn sample_rate(&self) -> f32 {
        f32::from_bits(self.sample_rate_bits.load(Ordering::Relaxed))
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
    /// their sample-clock timestamps onto the GUI clock. A fresh audio
    /// heartbeat anchors the whole stream before historical records are
    /// mapped; delayed batches never reset that anchor. Returns true when
    /// events arrived, in which case the
    /// caller should repaint this tick rather than at the idle poll.
    fn drain_into_tracker(
        &mut self,
        runtime: &mut harmonigraph_ui::VisualRuntime,
        now: f64,
    ) -> bool {
        if let Some(observation) = self.consumer.clock() {
            self.clock.observe(observation, now);
        }
        let Some(offset) = self.clock.offset else { return false };
        let tracker = &mut runtime.tracker;
        self.consumer.drain(|delivery, _| {
            if let harmonigraph_record::publication::Delivery::Event(event) = delivery {
                let result = tracker.handle_canonical_mapped(event, offset);
                debug_assert!(result.is_ok(), "validated canonical publication");
            }
            true
        }) != 0
    }
    /// Move the audio ring's blocks out, mapped to the GUI clock with the
    /// offset [`Self::drain_into_tracker`] has just observed, and lend the
    /// analysis with them; `None` until a heartbeat has given that offset,
    /// leaving the audio in the ring as before.
    ///
    /// Always drains AND (in [`Self::analyze`]) always feeds. There is no
    /// display state that makes the samples unwanted: the curve and the
    /// spectrogram read this one analyzer, the curve is always drawn, and
    /// feeding it is also what drives smooth repaint (via `is_flowing`).
    fn take_audio(
        &mut self,
        runtime: &mut harmonigraph_ui::VisualRuntime,
        appearance: &harmonigraph_ui::AppearanceDocument,
    ) -> Option<Taken> {
        // One clock conversion per take, shared with note delivery and made
        // in the same hold, observed and then mapped as a frame's drain did.
        let offset = self.clock.offset?;
        let mut audio = std::mem::take(&mut self.spare);
        audio.config = appearance.spectrum;
        self.audio_consumer.drain(|block, samples| {
            let position = (block.epoch, block.first_frame);
            let restart = self.audio_position != Some(position);
            if restart {
                self.audio_origin =
                    block.origin + block.first_frame as f64 / f64::from(block.format.sample_rate);
            }
            audio.samples.extend_from_slice(samples);
            audio.blocks.push(TakenBlock {
                restart,
                origin: self.audio_origin + offset,
                channels: block.format.channels,
                sample_rate: block.format.sample_rate,
                len: samples.len(),
            });
            self.audio_position = Some((block.epoch, block.first_frame + block.frames as u64));
        });
        // Nothing to analyze: leave the analysis where it is, and spare the
        // caller a second hold to give back nothing.
        if audio.blocks.is_empty() {
            self.spare = audio;
            return None;
        }
        // A lend never given back, which no step leaves, would otherwise drop
        // this audio and every take's after it. Healed as a panicking step is,
        // with a fresh analysis starting a fresh run at this audio.
        let lost = runtime.spectrum.recover();
        debug_assert!(!lost, "the analysis was given back");
        audio.blocks[0].restart |= lost;
        let lent = runtime.spectrum.lend().expect("the analysis is home");
        Some(Taken { lent, audio })
    }
    /// Run the FFTs over what [`Self::take`] moved out, with no lock held.
    pub(crate) fn analyze(taken: &mut Taken) {
        let Taken { lent, audio } = taken;
        let mut start = 0;
        for block in &audio.blocks {
            if block.restart {
                lent.analysis.restart();
                audio.deliveries.push(harmonigraph_ui::Delivery::Restart);
            }
            let samples = &audio.samples[start..start + block.len];
            start += block.len;
            lent.analysis.feed_source(
                samples,
                block.channels,
                block.sample_rate,
                block.origin,
                &audio.config,
                |delivery| audio.deliveries.push(delivery),
            );
        }
    }
    /// Put a fresh analysis back if a panic between [`Self::take`] and
    /// [`Self::give`] lost the lent one, and start the source run over, so the
    /// next block re-anchors the fresh analyzer's grid instead of extending
    /// the interrupted run's.
    pub(crate) fn recover_analysis(&mut self, runtime: &mut harmonigraph_ui::VisualRuntime) {
        if runtime.spectrum.recover() {
            self.audio_position = None;
        }
    }
    /// Hand what [`Self::analyze`] made to the store, and the analysis back.
    pub(crate) fn give(&mut self, runtime: &mut harmonigraph_ui::VisualRuntime, taken: Taken) {
        let Taken { lent, mut audio } = taken;
        runtime.spectrum.give_back(lent, audio.deliveries.drain(..), &audio.config);
        audio.samples.clear();
        audio.blocks.clear();
        self.spare = audio;
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
        self.clock.now(wall_now)
    }
    /// The locked half of a step: the notes into the tracker, then the
    /// audio out of its ring with the analysis lent alongside. Whether notes
    /// arrived, and the audio for [`Self::analyze`] then [`Self::give`].
    /// Input feeding does not prune against stale parameters: callers advance
    /// time after observing their parameters, or retain the last mirrors while
    /// closed.
    pub(crate) fn take(
        &mut self,
        runtime: &mut harmonigraph_ui::VisualRuntime,
        appearance: &harmonigraph_ui::AppearanceDocument,
        now: f64,
    ) -> (bool, Option<Taken>) {
        let notes = self.drain_into_tracker(runtime, now);
        (notes, self.take_audio(runtime, appearance))
    }
    /// A whole step with the lock held throughout: take, analyze, give. The
    /// input worker splits the same three around its lock; this is them in
    /// one, for its final drain and for tests. Whether notes arrived.
    pub(crate) fn drain(
        &mut self,
        runtime: &mut harmonigraph_ui::VisualRuntime,
        appearance: &harmonigraph_ui::AppearanceDocument,
        now: f64,
    ) -> bool {
        let (notes, taken) = self.take(runtime, appearance, now);
        if let Some(mut taken) = taken {
            Self::analyze(&mut taken);
            self.give(runtime, taken);
        }
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::super::shared::EditorShared;
    use super::FrameTiming;
    use harmonigraph_core::notes::{NoteEvent, SourceId};
    use std::sync::Arc;

    fn audio_harness(capacity: usize) -> (crate::audio_ingress::Producer, EditorShared) {
        let (_notes, consumer) = harmonigraph_record::publication::channel();
        let (audio, audio_consumer) = crate::audio_ingress::channel(capacity);
        let (_recorder, control) = harmonigraph_record::channel();
        let mut shared = EditorShared::new(
            consumer,
            audio_consumer,
            Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())),
            control,
        );
        shared.input.clock.observe(0.0, 0.0);
        (audio, shared)
    }

    fn drain_audio(shared: &mut EditorShared, now: f64) {
        shared.input.drain(&mut shared.ui.picture.runtime, &shared.ui.picture.appearance, now);
    }

    /// Callbacks a 144 Hz display link ran late by these amounts — the spread
    /// a Bitwig session's frame log shows, p50 0.65 ms to over a refresh — draw
    /// one refresh apart, as the glass shows them, and no earlier than they ran
    /// unless they ran over a refresh late; a frame the link did not ask for
    /// draws when it ran.
    #[test]
    fn frames_draw_at_their_refresh_however_late_the_callback_ran() {
        let (_audio, mut shared) = audio_harness(1024);
        let refresh = 1.0 / 144.0;
        let start = shared.input.start;
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
            let wall = shared.input.frame_wall(Some(timing), fired);
            if late_ms / 1000.0 < refresh {
                assert!(wall >= fired, "drawn before its own callback ran, at {late_ms} ms late");
            }
            drawn.push(shared.input.display_now(wall));
        }
        for step in drawn.windows(2).map(|pair| pair[1] - pair[0]) {
            assert!((step - refresh).abs() < 1e-6, "stepped {step} s against {refresh} s");
        }

        let observed = drawn.last().unwrap() + 0.05;
        assert_eq!(shared.input.frame_wall(None, observed), observed);
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
            assert_eq!(shared.input.frame_wall(Some(timing), observed), observed);
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
        let columns = |shared: &EditorShared| {
            shared
                .ui
                .picture
                .runtime
                .spectrum
                .history()
                .iter()
                .map(|c| (c.time, c.db().clone()))
                .collect::<Vec<_>>()
        };
        let mut reference = None;
        for (callback, drain_every, delay) in
            [(frames, 1, 0.0), (127, 1, 0.0), (511, 7, 0.3), (4093, 99, 3.0)]
        {
            let (mut tx, mut shared) = audio_harness(crate::AUDIO_RING_CAPACITY);
            let mut count = 0;
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
                    samples[first * 2..end * 2].iter().copied(),
                );
                count += 1;
                if count % drain_every == 0 {
                    drain_audio(&mut shared, 10.0 + end as f64 / 48_000.0 + delay);
                }
            }
            drain_audio(&mut shared, 15.0 + delay);
            let actual = columns(&shared);
            assert!(actual.len() > 50, "fixture must fill the window and reach many FFT hops");
            if let Some(expected) = &reference {
                assert_eq!(&actual, expected, "callback={callback}, drain_every={drain_every}");
            } else {
                reference = Some(actual);
            }
        }
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
            let (_recorder, control) = harmonigraph_record::channel();
            let mut shared = EditorShared::new(
                consumer,
                audio_consumer,
                Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())),
                control,
            );
            notes.observe_clock(0.0);
            drain_audio(&mut shared, 0.0);
            let format = crate::audio_ingress::Format {
                channels: 1,
                sample_rate: 48_000.0,
                sidechain: false,
            };
            let mut now = 0.0;
            for batch in 0..16usize {
                let start = batch as f64 * 0.25;
                let end = start + 0.25;
                audio.publish(
                    12_000,
                    format,
                    start,
                    (0..12_000).map(|i| ((batch * 12_000 + i) as f32 * 0.047).sin()),
                );
                notes
                    .note(
                        NoteEvent::on(start + 0.125, SourceId::DIRECT, 0, 48 + batch as u8, 0.8)
                            .into(),
                        Default::default(),
                    )
                    .unwrap();
                notes.observe_clock(end);
                drain_audio(&mut shared, end / speed);
                now = shared.input.display_now(end / speed);
                shared.ui.picture.runtime.advance_time(now, &shared.ui.picture.appearance);
            }
            let columns: Vec<_> = shared
                .ui
                .picture
                .runtime
                .spectrum
                .history()
                .iter()
                .map(|c| (c.time, c.db().clone()))
                .collect();
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
        let (mut tx, mut shared) = audio_harness(window + 384);
        let mono =
            crate::audio_ingress::Format { channels: 1, sample_rate: 48_000.0, sidechain: false };
        // Retain a full window plus a hop, then drop an entire callback.
        tx.publish(window + 384, mono, 0.0, std::iter::repeat(0.5));
        tx.publish(window, mono, 1.0, std::iter::repeat(-0.5));
        drain_audio(&mut shared, 1.0);
        let before = shared.ui.picture.runtime.spectrum.history().len();
        assert!(before > 0);
        // A gap cannot complete a window using the previous run's tail.
        tx.publish(window / 2, mono, 2.0, std::iter::repeat(-0.5));
        drain_audio(&mut shared, 2.0);
        assert_eq!(shared.ui.picture.runtime.spectrum.history().len(), before);
        // Queue old-format partial audio before a reset and new-format audio.
        tx.publish(window / 4, mono, 3.0, std::iter::repeat(0.9));
        tx.reset();
        let stereo =
            crate::audio_ingress::Format { channels: 2, sample_rate: 96_000.0, sidechain: true };
        tx.publish(window / 4, stereo, 4.0, std::iter::repeat(0.0));
        drain_audio(&mut shared, 4.0);
        assert_eq!(shared.ui.picture.runtime.spectrum.history().len(), before);
        for i in 1..=4 {
            tx.publish(
                window / 4,
                stereo,
                4.0 + i as f64 * window as f64 / 4.0 / 96_000.0,
                std::iter::repeat(0.0),
            );
            drain_audio(&mut shared, 5.0);
        }
        let spectrum = &shared.ui.picture.runtime.spectrum;
        assert!(spectrum.history().len() > before, "new format must refill and emit columns");
        for column in spectrum.history().iter().skip(before) {
            assert!(column.time > 4.0, "no window may straddle the reset");
            assert!(
                column.db().iter().all(|v| *v == 0),
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
        let (_recorder, take_control) = harmonigraph_record::channel();
        let mut shared = EditorShared::new(
            consumer,
            audio_consumer,
            std::sync::Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())),
            take_control,
        );
        for (source, time) in [(1, 99.950), (2, 99.995)] {
            let event = NoteEvent::on(time, SourceId(source), 0, 60, 1.0);
            producer
                .note(
                    harmonigraph_core::canonical::NoteDelta {
                        assignment: None,
                        partial_output: false,
                        event,
                        sequence: 1,
                        lifetime: 1,
                        provenance: harmonigraph_core::confirmed::PitchProvenance::AcceptedOutput,
                        timing: Some(harmonigraph_core::canonical::EventTiming {
                            clock: Default::default(),
                            input: 0,
                            planned: None,
                            sample: 0,
                            sample_rate: 48000.0,
                        }),
                        pitch_microcents: None,
                    },
                    Default::default(),
                )
                .unwrap();
        }

        producer.observe_clock(99.995);
        assert!(
            shared.input.drain_into_tracker(&mut shared.ui.picture.runtime, 7.0),
            "events arrived -> repaint"
        );
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

        // Empty ring: no work, no repaint request.
        assert!(!shared.input.drain_into_tracker(&mut shared.ui.picture.runtime, 7.1));
        producer.observe_clock(11.0);
        // Audio now=11 and GUI now=21. A delayed event from audio=2 belongs
        // at GUI=12; neither its batch nor a later idle poll is a new clock.
        use harmonigraph_core::canonical::{ClockId, EventTiming, NoteDelta};
        use harmonigraph_core::confirmed::PitchProvenance;
        let accepted = NoteDelta {
            assignment: None,
            partial_output: false,
            event: NoteEvent::on(2.0, SourceId(3), 0, 72, 0.8),
            sequence: 1,
            lifetime: 61,
            provenance: PitchProvenance::AcceptedOutput,
            timing: Some(EventTiming {
                clock: ClockId::default(),
                input: 96000,
                planned: None,
                sample: 96000,
                sample_rate: 48000.0,
            }),
            pitch_microcents: None,
        };
        producer.note(accepted, Default::default()).unwrap();
        let event = NoteEvent::on(2.0, SourceId::DIRECT, 0, 72, 0.8);
        producer.note(event.into(), Default::default()).unwrap();
        assert!(shared.input.drain_into_tracker(&mut shared.ui.picture.runtime, 21.0));
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
        assert!(!shared.input.drain_into_tracker(&mut shared.ui.picture.runtime, 22.0));
        use harmonigraph_core::canonical::{SourceBaseline, VoiceBaseline};
        let baseline = SourceBaseline::new(
            SourceId::DIRECT,
            1,
            3.0,
            0,
            true,
            &[VoiceBaseline {
                note: 72,
                actual_onset: 2.0,
                input_onset: 2.0,
                velocity: 0.8,
                pitch_microcents: 7_200_000_000,
                ..Default::default()
            }],
        )
        .unwrap();
        producer.observe_clock(12.0);
        producer.baseline(&baseline, Default::default()).unwrap();
        shared.input.drain_into_tracker(&mut shared.ui.picture.runtime, 22.2);
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
        assert!(note.history_complete, "baseline retains the matching observed lifetime");
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
        use harmonigraph_core::canonical::{GapReason, PublicationGap};
        producer
            .gap(
                PublicationGap {
                    source: Some(SourceId(3)),
                    time: 2.5,
                    through: 3.0,
                    first: 2,
                    last: 3,
                    reason: GapReason::PublicationFull,
                },
                Default::default(),
            )
            .unwrap();
        let resumed = SourceBaseline::new(
            SourceId(3),
            1,
            3.5,
            3,
            true,
            &[VoiceBaseline {
                note: 72,
                lifetime: 61,
                actual_onset: 2.0,
                input_onset: 2.0,
                onset: accepted.timing,
                velocity: 0.8,
                pitch_microcents: 7_200_000_000,
                provenance: PitchProvenance::AcceptedOutput,
                ..Default::default()
            }],
        )
        .unwrap();
        producer.baseline(&resumed, Default::default()).unwrap();
        shared.input.drain_into_tracker(&mut shared.ui.picture.runtime, 22.3);
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
            "accepted lifetime resumes its already-mapped onset after gap"
        );
        // The baseline's own time, audio 3.5, is mapped onto the GUI clock too:
        // the row resumes there.
        assert!(
            note.segments(22.3).any(|((start, _), _)| (start - 13.5).abs() < 1e-9),
            "the resuming baseline lands at GUI 13.5"
        );
    }

    /// `LiveInput::drain`'s ANSWER, which is the only thing that asks for a repaint on
    /// the tick a note arrives rather than at the idle poll (see `frame`).
    ///
    /// Its own test because the drain and the answer fail independently: every
    /// other test of this path reads the tracker afterwards, so a `LiveInput::drain`
    /// that drained perfectly and always answered `false` would satisfy all of
    /// them while costing every note played the latency of the idle poll.
    #[test]
    fn catch_up_answers_whether_notes_arrived() {
        let (mut producer, consumer) = harmonigraph_record::publication::channel();
        let (_audio_producer, audio_consumer) = crate::audio_ingress::channel(64);
        let (_recorder, take_control) = harmonigraph_record::channel();
        let mut shared = EditorShared::new(
            consumer,
            audio_consumer,
            std::sync::Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())),
            take_control,
        );

        // An empty ring is not a repaint.
        assert!(
            !shared.input.drain(&mut shared.ui.picture.runtime, &shared.ui.picture.appearance, 7.0),
            "nothing arrived, so nothing needs drawing"
        );

        producer.observe_clock(1.0);
        producer
            .note(NoteEvent::on(1.0, SourceId::DIRECT, 0, 60, 1.0).into(), Default::default())
            .unwrap();
        assert!(
            shared.input.drain(&mut shared.ui.picture.runtime, &shared.ui.picture.appearance, 7.1),
            "a note arrived and the frame was not told"
        );

        // And the ring is empty again, so the next tick asks for nothing.
        assert!(
            !shared.input.drain(&mut shared.ui.picture.runtime, &shared.ui.picture.appearance, 7.2),
            "the same note asked for a second repaint"
        );
    }
}
