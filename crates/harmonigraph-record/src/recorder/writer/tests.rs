//! Recorder-output integration tests: the producer rings, GUI observations,
//! and private writer state share the unchanged transport fixtures here.

use super::*;
use harmonigraph_take::RenderConfig;

/// The three fields [`header_for`] sets, each asserted against a value the
/// header could not have arrived at on its own.
///
/// That is the whole point of the test. `Header`'s `Default` is not a
/// zeroed struct — it carries 48 kHz, `None` and `""` — so a field that
/// stops being set produces a header that still serializes, still opens a
/// take the renderer will accept, and is simply wrong about the recording.
/// Nothing downstream can tell that apart from a take genuinely recorded
/// at 48 kHz with no look saved.
///
/// So the rate here is deliberately NOT 48 kHz: at the default value the
/// assertion would hold whether or not the field were set at all.
#[test]
fn the_header_carries_the_rate_the_look_and_the_source() {
    let blob = "(camera:(distance:9.0),palette:magma)".to_owned();
    let header = header_for(44_100.0, blob.clone());
    assert_ne!(
        44_100.0,
        harmonigraph_take::Header::default().sample_rate,
        "the fixture's rate has to differ from the default to test anything",
    );
    assert_eq!(header.sample_rate, 44_100.0, "the take's time base is the one it was given");
    assert_eq!(
        header.appearance.as_deref(),
        Some(blob.as_str()),
        "the look the take was recorded under has to reach the replay",
    );
    // A literal on both sides on purpose: this string is what the renderer
    // matches a Harmonigraph take on, so it is a format contract, and a
    // test that read it off a constant would follow a rename that broke
    // every take already written.
    assert_eq!(header.source, "harmonigraph", "the take says what wrote it");
}

/// One known instant, checked against a date computed by hand, plus the
/// epoch and a leap-year February to pin the boundaries
/// [`civil_from_days`] could get wrong.
#[test]
fn a_take_stamp_reads_as_the_calendar_date_and_time_it_names() {
    // 2024-02-29 18:05:09 UTC — a leap day, so the month/day arithmetic
    // has to fall through the extra day rather than assume 28.
    assert_eq!(stamp_for(1_709_229_909), "2024-02-29_18-05-09");
    assert_eq!(stamp_for(0), "1970-01-01_00-00-00", "the Unix epoch itself");
}

/// Two takes landing on the same stamp — the second starting within the
/// same UTC second as the first — number `_1`, `_2`, ... rather than the
/// second silently truncating the first's file. Covers both the `.take`
/// and the `.wav` companion, since either already existing is a collision.
#[test]
fn a_repeated_stamp_counts_up_instead_of_overwriting() {
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-disambiguate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let base = dir.join("take-2026-08-25_12-00-00.take");

    assert_eq!(disambiguate(base.clone()), base, "a free name is used as-is");

    std::fs::write(&base, "").expect("write base take");
    let first_dup = disambiguate(base.clone());
    assert_eq!(first_dup, dir.join("take-2026-08-25_12-00-00_1.take"));

    // A free `.take` name whose `.wav` companion is already taken is
    // still a collision — the audio would clobber, even though the take
    // file itself would not.
    std::fs::write(first_dup.with_extension("wav"), "").expect("write wav companion");
    assert_eq!(
        disambiguate(base),
        dir.join("take-2026-08-25_12-00-00_2.take"),
        "the wav-only collision at _1 is skipped, not just the take file's",
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// A [`Recorder`] whose rings the test keeps the far end of.
///
/// [`channel`] hands both consumers to the writer thread, which drains them
/// within `DRAIN_IDLE` — so a test that asked what the audio thread had
/// actually PUSHED would be racing that thread for the answer, and would
/// usually lose. Holding the consumers here is what makes the take's
/// CONTENTS assertable rather than only the counters beside them, and those
/// are different claims: `audio` can reserve correctly, count nothing
/// dropped, and write not one sample.
struct Bench {
    rec: Recorder,
    entries: rtrb::Consumer<Entry>,
    samples: rtrb::Consumer<f32>,
    end_at_rewind: Arc<AtomicBool>,
    hit_rewind: Arc<AtomicBool>,
    stop_at_bar: Arc<StopAtBar>,
    dropped: Arc<AtomicU64>,
}

impl Bench {
    fn new() -> Bench {
        let (producer, entries) = rtrb::RingBuffer::new(1024);
        let (audio, samples) = rtrb::RingBuffer::new(1024);
        let end_at_rewind = Arc::new(AtomicBool::new(false));
        let hit_rewind = Arc::new(AtomicBool::new(false));
        let stop_at_bar = Arc::new(StopAtBar::default());
        let dropped = Arc::new(AtomicU64::new(0));
        Bench {
            rec: Recorder {
                _writer_lifetime: None,
                publication: publication::channel().0,
                display: publication::channel().0,
                outage: publication::Lanes::default(),
                fence: Arc::new(RecordFence::default()),
                record_epoch: 0,
                record_pass: 1,
                closed_epoch: 0,
                last_configuration: None,
                producer,
                audio,
                with_audio: Arc::new(AtomicBool::new(false)),
                dropped: dropped.clone(),
                last_params: [f32::NAN; ParamKey::ALL.len()],
                was_armed: false,
                last_position: None,
                rolling: Arc::new(AtomicBool::new(false)),
                audio_started: false,
                end_at_rewind: end_at_rewind.clone(),
                captured: Arc::new(AtomicU64::new(0)),
                hit_rewind: hit_rewind.clone(),
                stop_at_bar: stop_at_bar.clone(),
                last_bar: None,
                finished: false,
                advanced: false,
                pending_split: false,
                rolled: Arc::new(AtomicBool::new(false)),
            },
            entries,
            samples,
            end_at_rewind,
            hit_rewind,
            stop_at_bar,
            dropped,
        }
    }

    /// Take the arming edge, as `process` does on a take's first block.
    fn arm(&mut self) {
        self.rec.fence.intent.store((self.rec.fence.epoch() + 1) << 1 | 1, Ordering::Release);
        assert!(self.rec.is_armed(), "the arming edge");
    }

    fn end_at_rewind(&self) {
        self.end_at_rewind.store(true, Ordering::Relaxed);
    }

    /// What the GUI publishes for OnTransportStop: a rewind ends the take.
    fn on_transport_stop(&self) {
        self.end_at_rewind();
    }

    fn hit_rewind(&self) -> bool {
        self.hit_rewind.load(Ordering::Relaxed)
    }

    fn stop_at_bar(&self, bar: f64) {
        self.stop_at_bar.set(Some(bar));
    }

    fn hit_stop_bar(&self) -> bool {
        self.stop_at_bar.hit.load(Ordering::Relaxed)
    }

    /// Everything pushed since the last call, rendered as comparable
    /// strings — [`Entry`] is a `Copy` payload for the audio thread and
    /// carries no `Debug` of its own.
    fn pushed(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(entry) = self.entries.pop() {
            out.push(match entry {
                Entry::Note { t, source, channel, note, kind } => {
                    let kind = match kind {
                        NoteEventKind::On { .. } => "on",
                        NoteEventKind::Off => "off",
                        NoteEventKind::Tuning { .. } => "tuning",
                        NoteEventKind::SessionReset => "session-reset",
                        NoteEventKind::SourceReset => "source-reset",
                    };
                    format!("note {note} ch{channel} source{} {kind} @{t}", source.0)
                }
                Entry::Param { t, key, value } => format!("param {key}={value} @{t}"),
                Entry::AudioStart(t) => format!("audio-start @{t}"),
                Entry::Configuration(config) => {
                    format!("configuration {} @{}", config.revision, config.t)
                }
                Entry::NewPass => "new-pass".to_owned(),
                Entry::ConfigurationAt { .. }
                | Entry::ConfigurationPassComplete(_)
                | Entry::ConfigurationEpochComplete(_)
                | Entry::ProducerClosed(_)
                | Entry::AudioSamples(_) => "configuration-protocol".to_owned(),
            });
        }
        out
    }

    /// Every sample written to the audio ring since the last call.
    fn written(&mut self) -> Vec<f32> {
        let mut out = Vec::new();
        while let Ok(sample) = self.samples.pop() {
            out.push(sample);
        }
        out
    }
}

/// `audio` reserves in whole FRAMES, not samples. This ring is
/// stereo-interleaved and lands in a WAV, so half a frame written here
/// swaps that file's channels from there on — permanently, and the writer
/// cannot notice, because it derives its frame count by dividing the bytes
/// it was handed.
///
/// An odd `samples` is what a caller hands over the moment it sizes a block
/// by the host's channel count instead of [`TAKE_CHANNELS`]. The tail
/// sample has to be dropped rather than written out of phase, and counted
/// as the hole in the recording that it is.
///
/// This pins the BLOCK-size half of the reservation, which is the half that
/// does the work: the ring here is only ever written in whole frames, so
/// its free count cannot go odd on its own. The free-space half is proved
/// exhaustively over on `interleaved_reservation`, since driving this ring
/// to an odd occupancy would mean reaching past the writer thread.
#[test]
fn recorded_audio_reserves_whole_frames_and_counts_the_dropped_tail() {
    let (mut rec, ctrl) = channel();
    ctrl.recording.store(true, Ordering::Relaxed);

    // Even, into an empty ring: the whole block fits, nothing is dropped.
    rec.audio(&mut std::iter::repeat_n(0.25f32, 8), 8);
    ctrl.tick(true, 0);
    assert!(
        !ctrl.status().contains("DROPPED"),
        "an even block fits whole, but got: {}",
        ctrl.status()
    );

    // Odd: the last sample is half a frame and must not reach the ring.
    rec.audio(&mut std::iter::repeat_n(0.25f32, 9), 9);
    ctrl.tick(true, 0);
    assert!(
        ctrl.dropped.load(Ordering::Relaxed) == 1 && ctrl.fence.failed.load(Ordering::Acquire),
        "an odd tail must be dropped, not written out of phase; got: {}",
        ctrl.status()
    );
}

/// The phase-slip guard, exhaustively. A reservation that is not a whole
/// number of frames leaves the ring one sample out of alignment, and every
/// frame after it hands the left channel's samples to the right — for as
/// long as the plugin runs, with nothing downstream able to tell that from
/// the signal itself.
#[test]
fn a_reservation_is_always_a_whole_number_of_frames() {
    for channels in 1..=8usize {
        for free_slots in 0..96usize {
            for samples in 0..24usize {
                let want = interleaved_reservation(free_slots, samples, channels);
                assert_eq!(
                    want % channels,
                    0,
                    "{want} samples is a partial frame at {channels} channels \
                         (free_slots={free_slots}, samples={samples})"
                );
                assert!(want <= free_slots, "reserved {want} of {free_slots} free slots");
                assert!(
                    want <= samples * channels,
                    "reserved {want}, more than the {} this block holds",
                    samples * channels
                );
            }
        }
    }
}

/// What the reservation does at each end: a ring with room takes the whole
/// block, a full one drops it rather than stalling the audio thread, and a
/// nearly-full one takes whole frames and drops the tail.
#[test]
fn a_full_ring_drops_the_block_rather_than_stalling() {
    // Room to spare: the whole block, interleaved.
    assert_eq!(interleaved_reservation(4096, 512, 2), 1024);
    // Exactly enough.
    assert_eq!(interleaved_reservation(1024, 512, 2), 1024);
    // Full: nothing, and the caller skips the write entirely.
    assert_eq!(interleaved_reservation(0, 512, 2), 0);
    // Nearly full, with the free space a PARTIAL frame: round down, so the
    // tail is dropped on a frame boundary rather than mid-frame.
    assert_eq!(interleaved_reservation(7, 512, 2), 6);
    assert_eq!(interleaved_reservation(7, 512, 4), 4);
    // Less free space than a single frame: nothing at all.
    assert_eq!(interleaved_reservation(3, 512, 4), 0);
}

/// Both callers gate on a positive channel count, so no live path reaches
/// this. It is pinned because it is what lets the function be called
/// without checking first — a total function is reusable, and reuse is how
/// `Recorder::audio` came to share the guard.
#[test]
fn a_zero_channel_bus_reserves_nothing() {
    assert_eq!(interleaved_reservation(4096, 512, 0), 0);
}

#[test]
fn at_loop_end_ends_the_take_on_the_first_wrap_without_splitting() {
    let (mut rec, ctrl) = channel();
    ctrl.fence.intent.store((ctrl.fence.epoch() + 1) << 1 | 1, Ordering::Release);
    ctrl.set_end_at_rewind(true);
    assert!(rec.is_armed(), "arming clears last_position and the done latch");

    // One loop's worth of forward motion.
    assert!(rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    assert!(rec.observe_transport(1.0, true, 64.0 / 48_000.0));
    assert!(rec.observe_transport(2.0, true, 64.0 / 48_000.0));
    assert!(!ctrl.hit_rewind(), "still mid-loop");

    // The transport wraps back to the loop start: end the take here, and
    // signal the GUI — do NOT keep rolling into a second pass.
    assert!(!rec.observe_transport(0.0, true, 64.0 / 48_000.0), "the wrap ends the take");
    assert!(ctrl.hit_rewind(), "GUI is told to stop and render the pass");

    // Latched: nothing rolls again until a fresh arm.
    assert!(!rec.observe_transport(1.0, true, 64.0 / 48_000.0));
}

#[test]
fn a_wrap_without_at_loop_end_splits_and_keeps_rolling() {
    let (mut rec, ctrl) = channel();
    ctrl.fence.intent.store((ctrl.fence.epoch() + 1) << 1 | 1, Ordering::Release);
    // end_at_rewind stays off — the default OnDisarm/looping behavior.
    assert!(rec.is_armed());
    assert!(rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    assert!(rec.observe_transport(2.0, true, 64.0 / 48_000.0));
    // The wrap starts a new pass but keeps recording, as before.
    assert!(rec.observe_transport(0.0, true, 64.0 / 48_000.0), "a normal loop keeps going");
    assert!(!ctrl.hit_rewind());
}

#[test]
fn re_arming_clears_the_loop_end_latch() {
    let (mut rec, ctrl) = channel();
    ctrl.fence.intent.store((ctrl.fence.epoch() + 1) << 1 | 1, Ordering::Release);
    ctrl.set_end_at_rewind(true);
    assert!(rec.is_armed());
    assert!(rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    assert!(rec.observe_transport(2.0, true, 64.0 / 48_000.0));
    assert!(!rec.observe_transport(0.0, true, 64.0 / 48_000.0), "the wrap ends the first take");
    assert!(ctrl.hit_rewind());

    // Disarm, then re-arm: the done latch and the loop-end flag clear, so
    // the next take records from scratch rather than starting finished.
    ctrl.fence.intent.store(ctrl.fence.epoch() << 1, Ordering::Release);
    assert!(!rec.is_armed());
    ctrl.fence.intent.store((ctrl.fence.epoch() + 1) << 1 | 1, Ordering::Release);
    assert!(rec.is_armed(), "re-arm");
    assert!(!ctrl.hit_rewind(), "the latch cleared on re-arm");
    assert!(rec.observe_transport(0.0, true, 64.0 / 48_000.0), "records again");
}

#[test]
fn at_loop_end_ignores_the_jump_to_the_loop_start_when_playback_begins() {
    let (mut rec, ctrl) = channel();
    ctrl.fence.intent.store((ctrl.fence.epoch() + 1) << 1 | 1, Ordering::Release);
    ctrl.set_end_at_rewind(true);
    assert!(rec.is_armed());

    // Playhead parked PAST the loop start, transport stopped.
    assert!(!rec.observe_transport(5.0, false, 64.0 / 48_000.0), "parked, not rolling");

    // Hit play: the transport snaps back to the loop start. This is the bug
    // that produced empty takes — it must NOT end the take, because nothing
    // has been recorded yet. It begins the pass instead.
    assert!(rec.observe_transport(0.0, true, 64.0 / 48_000.0), "the jump-to-start begins the pass");
    assert!(!ctrl.hit_rewind(), "the initial jump is not a loop end");

    // Now it rolls forward through the loop...
    assert!(rec.observe_transport(1.0, true, 64.0 / 48_000.0));
    assert!(rec.observe_transport(2.0, true, 64.0 / 48_000.0));

    // ...and THIS wrap, after real forward motion, is the loop end.
    assert!(!rec.observe_transport(0.0, true, 64.0 / 48_000.0), "the real wrap ends the take");
    assert!(ctrl.hit_rewind());
}

/// Rolling is the union of "the position advanced continuously" and "the host says
/// playing", so a position that moves records the block whatever the flag
/// says.
///
/// During an offline export some hosts report `playing = false` — nothing
/// is being played, after all. Reading the flag alone would record nothing
/// for the whole export and leave a take that replays as an empty lattice,
/// with nothing anywhere to say why.
#[test]
fn a_position_that_advances_rolls_even_when_the_host_says_it_is_not_playing() {
    let mut b = Bench::new();
    b.arm();
    // Nothing to compare against on the first block, so the flag IS all
    // there is, and it says no.
    assert!(!b.rec.observe_transport(0.0, false, 64.0 / 48_000.0));
    assert!(
        b.rec.observe_transport(64.0 / 48_000.0, false, 64.0 / 48_000.0),
        "the position advanced, so the block belongs in the take"
    );
    assert!(b.rec.observe_transport(128.0 / 48_000.0, false, 64.0 / 48_000.0));
}

/// The tolerance includes its endpoints, excludes the next sample outside
/// them, and follows the previous callback even if the next one changes size.
/// A tiny scrub in the same interval has exactly the accepted block's cost.
#[test]
fn stopped_continuity_edges_emit_only_the_accepted_blocks() {
    for (frames, rate) in [(64, 48_000.0), (256, 44_100.0)] {
        let duration = frames as f64 / rate;
        for (step, accepted) in [
            (0.0, false),
            (duration * 0.5 - 1.0 / rate, false),
            (duration * 0.5, true),
            (duration, true),
            (duration * 1.5, true),
            (duration * 1.5 + 1.0 / rate, false),
            (20.0, false),
        ] {
            let mut b = Bench::new();
            b.arm();
            b.on_transport_stop();
            assert!(!b.rec.observe_transport(5.0, false, duration));
            // The NEXT block is twice as long. Using its duration to judge
            // this step would wrongly reject the lower tolerance edge.
            let t = 5.0 + step;
            assert_eq!(b.rec.observe_transport(t, false, duration * 2.0), accepted);
            if accepted {
                b.rec.params(t, [0.5; ParamKey::ALL.len()]);
                b.rec.configuration(
                    t,
                    harmonigraph_core::configuration::ConfigReducer::default().resolved(),
                );
                b.rec.mark_audio_start(t);
                b.rec.audio(&mut std::iter::repeat_n(0.25, frames * 4), frames * 4);
            }
            let entries: Vec<_> = std::iter::from_fn(|| b.entries.pop().ok()).collect();
            assert_eq!(entries.len(), if accepted { ParamKey::ALL.len() + 3 } else { 0 });
            if accepted {
                assert!(entries.iter().all(|entry| match entry {
                    Entry::Param { t: time, .. } | Entry::AudioStart(time) => *time == t,
                    Entry::Configuration(config) => config.t == t,
                    Entry::AudioSamples(n) => *n == frames * 4,
                    _ => false,
                }));
            }
            assert_eq!(b.written(), vec![0.25; if accepted { frames * 4 } else { 0 }]);
            assert!(!b.rec.observe_transport(5.0, false, duration));
            assert_eq!(b.hit_rewind(), accepted, "tiny accepted scrubs also qualify the rewind");
        }
    }
}

/// The other half of the union, and its floor: a host that reports
/// `playing` before its position has moved still gets its first block
/// captured, and only a block where BOTH say no is skipped — which is
/// exactly a parked transport.
#[test]
fn only_a_parked_transport_stops_a_block_being_recorded() {
    let mut b = Bench::new();
    b.arm();
    assert!(!b.rec.observe_transport(4.0, false, 64.0 / 48_000.0));
    assert!(
        !b.rec.observe_transport(4.0, false, 64.0 / 48_000.0),
        "neither the position nor the flag: this is a parked transport"
    );
    assert!(b.rec.observe_transport(4.0, true, 64.0 / 48_000.0), "the flag alone still counts");
}

/// AtLoopEnd has to survive the playhead being parked for MORE THAN ONE
/// block before playback starts.
///
/// `advanced` is what separates the transport snapping back to the loop
/// start from a loop actually wrapping, and only real forward motion may
/// set it. A transport standing still republishes the same position every
/// block — which is what a parked playhead does for as long as it is parked
/// — and counting that as motion would arm the latch before anything was
/// recorded, so the jump to the loop start would end the take: an empty
/// file, and a render of nothing.
#[test]
fn a_playhead_parked_for_several_blocks_has_not_advanced() {
    let mut b = Bench::new();
    b.arm();
    b.end_at_rewind();

    // Parked past the loop start, transport stopped, for several blocks.
    for _ in 0..4 {
        assert!(!b.rec.observe_transport(5.0, false, 64.0 / 48_000.0), "parked");
    }

    // Hit play: the transport snaps back to the loop start. Nothing has
    // been recorded yet, so this begins the pass rather than ending it.
    assert!(
        b.rec.observe_transport(0.0, true, 64.0 / 48_000.0),
        "the jump-to-start begins the pass"
    );
    assert!(!b.hit_rewind(), "a parked playhead has not advanced");
    // Beginning the pass is not splitting it: AtLoopEnd only ever wants one
    // file, and a `NewPass` here would leave the take's notes in the second
    // one while the first — the one that renders — stayed empty.
    let begun = b.pushed();
    assert!(begun.is_empty(), "the jump-to-start must not split the take: {begun:?}");

    // Real forward motion, and only then does a wrap mean the loop end.
    assert!(b.rec.observe_transport(1.0, true, 64.0 / 48_000.0));
    assert!(!b.rec.observe_transport(0.0, true, 64.0 / 48_000.0), "the real wrap ends the take");
    assert!(b.hit_rewind());
}

/// Play a block at a time towards the stop bar. The block that reaches it
/// ends the take and is itself excluded, so nothing at or past the bar
/// lands in the file.
///
/// Blocks of 0.05 bar rather than a couple of big steps: the crossing test
/// is bounded by step SIZE as well as by direction, and a fixture that
/// arrived in one leap would pass the direction half while proving nothing
/// about the half that separates playback from a drag.
#[test]
fn playing_through_the_stop_bar_ends_the_take_there() {
    let mut b = Bench::new();
    b.arm();
    // Bar 5 as `observe_bar` counts, which is the arranger's bar 6.
    b.stop_at_bar(5.0);

    let mut bar = 4.8;
    while bar < 4.99 {
        assert!(!b.rec.observe_bar(Some(bar)), "still short of the bar");
        assert!(!b.hit_stop_bar());
        bar += 0.05;
    }
    assert!(b.rec.observe_bar(Some(bar + 0.05)), "this block reaches the bar");
    assert!(b.hit_stop_bar(), "the GUI is told to stop and render");
    assert!(
        !b.rec.observe_transport(99.0, true, 64.0 / 48_000.0),
        "and nothing after it is recorded, whatever the transport does"
    );
}

/// Arming with the playhead ALREADY past the stop bar records until you
/// disarm, rather than ending immediately on an empty file.
///
/// This is why the test is a crossing and not a level. A take that ends
/// before it captures anything renders a video of nothing, which is #569's
/// failure — and the position it would end at is one the transport never
/// played through.
#[test]
fn arming_past_the_stop_bar_never_ends_the_take() {
    let mut b = Bench::new();
    b.arm();
    b.stop_at_bar(5.0);

    for step in 0..40 {
        let bar = 12.0 + f64::from(step) * 0.05;
        assert!(!b.rec.observe_bar(Some(bar)), "never below the bar, so never through it");
    }
    assert!(!b.hit_stop_bar());
}

/// A playhead DRAGGED across the stop bar is not playback through it.
///
/// Nothing in this API separates the two by the host's own flag — an
/// offline export reports `playing = false` for its whole length while its
/// position climbs, which is the case the trigger exists for. Size is what
/// separates them: a block is a hundredth of a bar, a drag is bars.
#[test]
fn a_playhead_dragged_across_the_stop_bar_does_not_end_the_take() {
    let mut b = Bench::new();
    b.arm();
    b.stop_at_bar(33.0);

    assert!(!b.rec.observe_bar(Some(1.0)), "parked near the top");
    assert!(!b.rec.observe_bar(Some(80.0)), "dragged to bar 81 to look at something");
    assert!(!b.hit_stop_bar(), "a drag is not a take's end");

    // And the take is still live: dragging back and playing through the bar
    // ends it the way it should have all along.
    assert!(!b.rec.observe_bar(Some(32.9)), "dragged back");
    assert!(b.rec.observe_bar(Some(33.0)), "played through");
    assert!(b.hit_stop_bar());
}

/// A host with no beats timeline reports no bar, and a take under this
/// trigger then simply runs until you disarm — the same graceful nothing
/// AtLoopEnd does with looping switched off.
///
/// The `None`s sit BETWEEN two bars that would otherwise cross, so the test
/// fails if a missing bar is skipped over rather than remembered as absent.
#[test]
fn a_host_that_reports_no_bar_never_ends_the_take() {
    let mut b = Bench::new();
    b.arm();
    b.stop_at_bar(5.0);

    assert!(!b.rec.observe_bar(Some(4.9)));
    assert!(!b.rec.observe_bar(None), "no beats timeline");
    assert!(!b.rec.observe_bar(None));
    assert!(!b.rec.observe_bar(Some(5.1)), "the crossing was never observed");
    assert!(!b.hit_stop_bar());
}

/// Re-arming clears the latch and the previous take's bar, so the next take
/// has to cross the bar for itself.
#[test]
fn re_arming_clears_the_stop_bar_latch() {
    let mut b = Bench::new();
    b.arm();
    b.stop_at_bar(5.0);
    assert!(!b.rec.observe_bar(Some(4.9)));
    assert!(b.rec.observe_bar(Some(5.0)));
    assert!(b.hit_stop_bar());

    b.rec.fence.intent.store(b.rec.fence.epoch() << 1, Ordering::Release);
    assert!(!b.rec.is_armed(), "disarmed");
    b.arm();
    assert!(!b.hit_stop_bar(), "the latch cleared on re-arm");
    // Bar 5 again, and with `last_bar` cleared it is a level rather than a
    // crossing — so the new take runs on, exactly as one armed past the bar
    // does.
    assert!(!b.rec.observe_bar(Some(5.0)), "no remembered bar to have crossed from");
    assert!(!b.rec.observe_bar(Some(5.0)), "nor from the bar itself");
}

/// A backward jump SPLITS an AtBar take rather than ending it, and the pass
/// that reaches the bar is the one that ends there.
///
/// `ends_at_rewind` is false for this trigger, so play / scrub back / play
/// again leaves the last run through the range as the take — which is what
/// a second attempt at a section is for.
#[test]
fn a_rewind_restarts_an_at_bar_take_rather_than_ending_it() {
    let mut b = Bench::new();
    b.arm();
    b.stop_at_bar(5.0);
    // `end_at_rewind` deliberately NOT set: that is `ends_at_rewind()`'s
    // answer for AtBar.

    assert!(b.rec.observe_transport(1.0, true, 64.0 / 48_000.0));
    assert!(!b.rec.observe_bar(Some(2.0)));
    // Back to the top, and forward again through the bar.
    assert!(
        b.rec.observe_transport(0.0, true, 64.0 / 48_000.0),
        "a rewind splits, as under OnDisarm"
    );
    assert!(!b.hit_rewind(), "and does not end the take");
    assert!(!b.rec.observe_bar(Some(0.0)));
    assert!(!b.rec.observe_bar(Some(4.98)));
    assert!(b.rec.observe_bar(Some(5.02)), "the second pass reaches the bar");
    assert!(b.hit_stop_bar());
}

/// A block that both pays an owed split and crosses the stop bar must not
/// open a pass, because it is about to finish one.
///
/// An AtBar take splits at a backward jump, and a jump made with the
/// transport STOPPED only owes the split — it is paid by the next block
/// that records. Park the playhead a hair before the stop bar and hit play
/// and that is the same block: pay the debt first and the take rolls over
/// into a fresh file which this block immediately finishes with nothing in
/// it, and the newest file is the one that renders. Asking the bar first is
/// what stops it, by latching `finished` before `observe_transport` reaches
/// its debt.
///
/// The fixture has to park WITHIN a block of the bar, or the crossing is
/// too wide a step to fire and the test passes without reaching the hazard
/// at all.
#[test]
fn a_block_that_both_owes_a_split_and_crosses_the_bar_opens_no_pass() {
    let mut b = Bench::new();
    b.arm();
    b.stop_at_bar(5.0);

    // Rolling, well short of the bar, then on past it to somewhere else.
    assert!(!b.rec.observe_bar(Some(4.0)));
    assert!(b.rec.observe_transport(8.0, true, 64.0 / 48_000.0));
    assert!(!b.rec.observe_bar(Some(20.0)), "a leap, not playback");
    assert!(b.rec.observe_transport(40.0, true, 64.0 / 48_000.0));
    let _ = b.pushed();

    // Dragged back to a hair before the bar with the transport stopped: a
    // split is owed, not paid.
    assert!(!b.rec.observe_bar(Some(4.99)));
    assert!(!b.rec.observe_transport(9.98, false, 64.0 / 48_000.0), "parked after the drag");

    // Hit play, and the first block that would record is also the one that
    // crosses the bar.
    assert!(b.rec.observe_bar(Some(5.02)), "the crossing");
    assert!(!b.rec.observe_transport(10.04, true, 64.0 / 48_000.0), "the take is already over");
    let after = b.pushed();
    assert!(after.is_empty(), "no pass may be opened for a block that ends the take: {after:?}");
}

/// The backward-jump threshold is there to ignore a host's own jitter
/// around a loop point, so a step back smaller than it is not a wrap.
///
/// Splitting on jitter would cut the take into a second file mid-phrase,
/// and under AtLoopEnd it would end the take outright — both at a position
/// the user asked nothing of. The same threshold is what keeps a transport
/// standing still from reading as a wrap.
#[test]
fn a_step_back_smaller_than_the_threshold_is_jitter_rather_than_a_wrap() {
    let mut b = Bench::new();
    b.arm();
    assert!(b.rec.observe_transport(1.00, true, 64.0 / 48_000.0));
    assert!(b.rec.observe_transport(1.04, true, 64.0 / 48_000.0));
    // 0.02 back, under BACKWARD_JUMP: the host is jittering, not looping.
    assert!(b.rec.observe_transport(1.02, true, 64.0 / 48_000.0));
    let jitter = b.pushed();
    assert!(jitter.is_empty(), "jitter must not split the take, but pushed {jitter:?}");

    // A real wrap is far larger, and does split it.
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    assert_eq!(b.pushed(), ["new-pass"]);
}

/// A plain wrap splits the take into a second file; an AtLoopEnd wrap does
/// not.
///
/// The writer opens the next pass eagerly, so a `NewPass` pushed at the
/// moment the take ends would leave the empty second file as the one that
/// renders. The two wraps are identical from the transport's side and only
/// the mode tells them apart, so each is held to pushing what it should and
/// nothing besides.
#[test]
fn a_plain_wrap_splits_the_file_and_a_loop_end_wrap_does_not() {
    let mut b = Bench::new();
    b.arm();
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    assert!(b.rec.observe_transport(2.0, true, 64.0 / 48_000.0));
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0), "a plain wrap keeps recording");
    assert_eq!(b.pushed(), ["new-pass"], "the plain wrap opens the next pass");

    let mut b = Bench::new();
    b.arm();
    b.end_at_rewind();
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    assert!(b.rec.observe_transport(2.0, true, 64.0 / 48_000.0));
    assert!(!b.rec.observe_transport(0.0, true, 64.0 / 48_000.0), "the loop end ends the take");
    let split = b.pushed();
    assert!(
        split.is_empty(),
        "a split here leaves the empty second file as the one that renders: {split:?}"
    );
}

/// The shape a faster-than-realtime audio export leaves behind: the take
/// rolls the whole song through, then the host puts the playhead back where
/// it was — one backward block, with the transport stopped.
///
/// Splitting there is what rendered an empty video out of a full export. The
/// take is finished by the GUI a beat later, and it renders the LAST pass, so
/// a pass opened by the restore is the one it finds. `playing = false`
/// throughout is deliberate and not an edge case: it is what a host reports
/// while nothing is being played, which is every block of an offline render.
#[test]
fn a_playhead_restored_after_an_export_neither_records_nor_splits() {
    let mut b = Bench::new();
    b.arm();
    assert!(
        !b.rec.observe_transport(40.0, false, 64.0 / 48_000.0),
        "nothing to compare on the first block"
    );
    assert!(
        b.rec.observe_transport(40.0 + 64.0 / 48_000.0, false, 64.0 / 48_000.0),
        "the position advancing IS the export"
    );
    assert!(b.rec.observe_transport(40.0 + 128.0 / 48_000.0, false, 64.0 / 48_000.0));
    b.pushed();

    assert!(
        !b.rec.observe_transport(5.0, false, 64.0 / 48_000.0),
        "the restore is not the take rolling"
    );
    let restore = b.pushed();
    assert!(restore.is_empty(), "the restore must not split the take, but pushed {restore:?}");

    // Parked, which is what lets the GUI's stop debounce run out.
    assert!(!b.rec.observe_transport(5.0, false, 64.0 / 48_000.0));
    assert!(b.pushed().is_empty());
}

/// A rewind the transport then rolls away from IS a new pass. The split just
/// waits for the block that records, so it lands ahead of that block's own
/// events rather than opening a file nothing follows into — and it is owed
/// once, not re-paid every block after.
#[test]
fn a_rewind_while_stopped_splits_at_the_block_that_records_again() {
    let mut b = Bench::new();
    b.arm();
    assert!(b.rec.observe_transport(60.0, true, 64.0 / 48_000.0));
    assert!(b.rec.observe_transport(90.0, true, 64.0 / 48_000.0));
    b.pushed();

    assert!(
        !b.rec.observe_transport(0.0, false, 64.0 / 48_000.0),
        "dragged back with the transport stopped"
    );
    assert!(b.pushed().is_empty(), "no file is owed one yet");

    assert!(b.rec.observe_transport(0.5, true, 64.0 / 48_000.0), "playing again");
    assert_eq!(b.pushed(), ["new-pass"]);
    assert!(b.rec.observe_transport(1.0, true, 64.0 / 48_000.0));
    assert!(b.pushed().is_empty(), "the split is owed once, not every block after");
}

/// #523: even an audio-only stopped export ends at its restore, before
/// the background debounce can run. Unlike the original fixture's 82-second
/// leaps, every export step here is one 64-frame callback at 48 kHz.
#[test]
fn a_playhead_restored_after_an_export_ends_the_take_there() {
    let mut b = Bench::new();
    b.arm();
    b.on_transport_stop();
    let duration = 64.0 / 48_000.0;
    assert!(!b.rec.observe_transport(4.254, false, duration));
    for block in 1..=64 {
        let t = 4.254 + f64::from(block) * duration;
        assert!(b.rec.observe_transport(t, false, duration));
        b.rec.mark_audio_start(t);
        b.rec.audio(&mut [0.25, -0.25].into_iter().cycle().take(128), 128);
        assert_eq!(b.written(), [0.25, -0.25].repeat(64));
    }
    let records = b.pushed();
    assert_eq!(records.iter().filter(|r| r.starts_with("audio-start")).count(), 1);
    assert!(!b.rec.observe_transport(4.254, false, duration), "the restore ends the take");
    assert!(b.hit_rewind());
    assert!(b.pushed().is_empty(), "restore opens no pass and records nothing");
    assert!(!b.rec.observe_transport(4.35, true, duration));
    assert!(b.pushed().is_empty());
}

/// #569/#838: a discontinuous pre-play scrub is rejected without ending
/// the take. Use the same Control setter as the editor.
#[test]
fn a_scrub_forward_and_back_before_playing_does_not_end_the_take() {
    let (mut rec, ctrl) = channel();
    ctrl.fence.intent.store((ctrl.fence.epoch() + 1) << 1 | 1, Ordering::Release);
    ctrl.set_end_at_rewind(true);
    assert!(rec.is_armed());
    let duration = 64.0 / 48_000.0;
    for position in [10.0, 30.0, 5.0] {
        assert!(!rec.observe_transport(position, false, duration));
        assert!(!ctrl.has_rolled());
    }
    assert!(!ctrl.hit_rewind());
    assert!(rec.observe_transport(5.0, true, duration));
    assert!(ctrl.has_rolled());
}

/// A one-file trigger drops a split the take owed from before it was chosen,
/// rather than carrying it into the pass that begins here — that split would
/// leave the take's notes in a second file while the first is what renders.
#[test]
fn switching_to_a_one_file_trigger_drops_a_split_the_take_owed() {
    let mut b = Bench::new();
    b.arm();
    // OnDisarm: one block recorded, then dragged back with the transport
    // stopped, which owes a split and has advanced nothing. The block has to
    // record, or the split is dropped as one owed by an empty take and this
    // passes without reaching the one-file rule at all.
    assert!(b.rec.observe_transport(10.0, true, 64.0 / 48_000.0));
    assert!(!b.rec.observe_transport(5.0, false, 64.0 / 48_000.0));
    assert!(b.pushed().is_empty());

    // Finish changes to a trigger that wants one file, and the take resumes.
    b.end_at_rewind();
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    let split = b.pushed();
    assert!(split.is_empty(), "one file, so the owed split is dropped: {split:?}");
}

/// The same debt, coming due the other way: the take resumes FORWARD of
/// where it was dragged to, so no backward jump is there to drop the owed
/// split. A one-file trigger still has to refuse it — a second pass here
/// takes the notes with it and leaves the first file, the one that renders,
/// holding the music that came before the drag.
#[test]
fn a_one_file_trigger_refuses_an_owed_split_that_comes_due_rolling_forward() {
    let mut b = Bench::new();
    b.arm();
    assert!(b.rec.observe_transport(10.0, true, 64.0 / 48_000.0), "music recorded before the drag");
    assert!(
        !b.rec.observe_transport(5.0, false, 64.0 / 48_000.0),
        "dragged back with the transport stopped"
    );
    b.pushed();

    b.end_at_rewind();
    assert!(
        b.rec.observe_transport(6.0, true, 64.0 / 48_000.0),
        "playing on from where it was dragged to"
    );
    let split = b.pushed();
    assert!(split.is_empty(), "one file, so the owed split is dropped: {split:?}");

    // And the debt is settled rather than merely deferred: switching back to
    // the splitting trigger must not resurrect it.
    b.end_at_rewind.store(false, Ordering::Relaxed);
    assert!(b.rec.observe_transport(7.0, true, 64.0 / 48_000.0));
    assert!(b.pushed().is_empty(), "a dropped split stays dropped");
}

/// A take that ends on a pass with no notes renders the pass that has them.
///
/// "The last file opened" and "the file worth rendering" are different
/// questions, and an unvoiced tail is not empty enough to tell apart by size:
/// a split rewrites every parameter into the pass it opens, so the file has
/// content and draws nothing.
#[test]
fn a_take_ending_on_an_unvoiced_pass_renders_the_pass_that_was_played() {
    let dir = std::env::temp_dir().join(format!("harmonigraph-unvoiced-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let base = dir.join("take.take");
    let status = Mutex::new(String::new());
    let (mut producer, mut consumer) = rtrb::RingBuffer::new(64);
    let mut open =
        Open::create(header_for(48_000.0, String::new()), base.clone(), 1, None, &status);
    assert!(open.is_some(), "the fixture has to actually open a file to write into");

    let note = Entry::Note {
        t: 0.0,
        source: SourceId::DIRECT,
        channel: 0,
        note: 60,
        kind: NoteEventKind::On { velocity: 1.0 },
    };
    producer.push(note).expect("ring has room");
    producer.push(Entry::NewPass).expect("ring has room");
    producer.push(Entry::Param { t: 1.0, key: 0, value: 0.5 }).expect("ring has room");
    drain(&mut consumer, &mut open, &status);
    assert_eq!(open.as_ref().expect("still open").pass, 2, "the split did open a second file");
    assert_eq!(
        open.expect("still open").take_path(),
        base,
        "the pass with the notes is the take that renders",
    );

    // A voiced tail renders itself, which is the ordinary loop-recording
    // case and the reason this cannot just always pick the first pass.
    let mut open =
        Open::create(header_for(48_000.0, String::new()), base.clone(), 1, None, &status);
    producer.push(note).expect("ring has room");
    producer.push(Entry::NewPass).expect("ring has room");
    producer.push(note).expect("ring has room");
    drain(&mut consumer, &mut open, &status);
    assert_eq!(
        open.expect("still open").take_path(),
        Open::path_for(&base, 2),
        "the second pass was played too, so it is the take",
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// Exercise the actual recorder ring AND writer conversion before parsing.
#[test]
fn source_scoped_notes_reach_the_take_with_their_original_times() {
    let mut b = Bench::new();
    b.arm();
    let events = [
        NoteEvent::on(0.125, SourceId(1), 3, 60, 0.8),
        NoteEvent::on(0.25, SourceId(2), 3, 60, 0.6),
        NoteEvent {
            time: 0.5,
            source: SourceId(2),
            channel: 3,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: -0.25 },
        },
        NoteEvent::off(0.75, SourceId(1), 3, 60),
        NoteEvent::source_reset(1.0, SourceId(1)),
        NoteEvent::session_reset(1.25),
    ];
    for event in events {
        b.rec.note(event.time, event.source, event.channel, event.note, event.kind);
    }
    let dir = std::env::temp_dir().join(format!("harmonigraph-source-take-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("source.take");
    let status = Mutex::new(String::new());
    let mut open =
        Open::create(header_for(48_000.0, String::new()), path.clone(), 1, None, &status);
    assert!(open.is_some(), "fixture must reach the file writer");
    assert!(drain(&mut b.entries, &mut open, &status));
    drop(open);
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(!take.truncated);
    assert_eq!(take.notes().map(NoteEvent::from).collect::<Vec<_>>(), events);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn configuration_pass_capacity_requires_actual_retirement_before_reuse() {
    for retire in [false, true] {
        let mut b = Bench::new();
        b.rec.enable_configuration();
        b.rec.fence.intent.store(3, Ordering::Release);
        assert!(b.rec.is_armed());
        let fence = b.rec.fence.clone();
        let dir = std::env::temp_dir()
            .join(format!("harmonigraph-pass-bound-{}-{retire}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let status = Mutex::new(String::new());
        let mut open = Open::create(
            header_for(48_000.0, String::new()),
            dir.join("record.take"),
            1,
            None,
            &status,
        );
        open.as_mut().unwrap().epoch = 1;
        open.as_mut().unwrap().configuration_enabled = true;
        assert!(b.rec.observe_transport(10.0, true, 64.0 / 48_000.0));
        for _ in 1..RECORD_PASSES {
            assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
            assert!(b.rec.observe_transport(10.0, true, 64.0 / 48_000.0));
        }
        drain_with_audio(&mut b.entries, Some(&mut b.samples), &mut open, &status, Some(&fence));
        assert_eq!(open.as_ref().unwrap().retained.len(), RECORD_PASSES - 1);
        assert!(!fence.failed.load(Ordering::Acquire));
        if retire {
            b.rec.configuration_pass_complete(RecordAddress { epoch: 1, pass: 1 });
        }
        assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
        drain_with_audio(&mut b.entries, Some(&mut b.samples), &mut open, &status, Some(&fence));
        assert_eq!(fence.failed.load(Ordering::Acquire), !retire);
        assert_eq!(
            open.is_some(),
            retire,
            "the 129th unclosed pass must not be silently finalized or dropped"
        );
        drop(open);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn configuration_stop_preserves_the_observed_callback_and_drop_refuses_unclosed_work() {
    let mut b = Bench::new();
    b.rec.enable_configuration();
    b.rec.fence.intent.store(3, Ordering::Release);
    let observed = b.rec.capture_recording_intent();
    // Main thread stops after the callback captured its intent.
    b.rec.fence.intent.store(2, Ordering::Release);
    assert!(b.rec.is_armed_at(observed));
    b.rec.audio(&mut [1.0, 2.0, 3.0, 4.0].into_iter(), 4);
    assert!(matches!(b.entries.pop(), Ok(Entry::AudioSamples(4))));
    assert!(!b.rec.is_armed());
    assert!(matches!(b.entries.pop(), Ok(Entry::ProducerClosed(1))));
    let fence = b.rec.fence.clone();
    assert!(!fence.failed.load(Ordering::Acquire));
    drop(b.rec);
    assert!(
        fence.failed.load(Ordering::Acquire),
        "producer closure alone cannot prove deferred configuration complete"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn retirement_failure_closes_after_a_full_publication_lane_and_its_final_loss() {
    let (mut recorder, mut capture) = testing::channel();
    recorder.enable_configuration();
    recorder.enable_canonical();
    capture.arm();
    assert!(recorder.is_armed());
    let address = recorder.configuration_address().unwrap();
    let directory = std::env::temp_dir()
        .join(format!("harmonigraph-held-full-publication-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("record.take");
    let mut writer = testing::FileWriter::new(&capture, path.clone(), None);
    recorder.hold_retired_publication();
    recorder.fail_configuration();
    writer.drain(&mut capture);
    assert_eq!(writer.current_pass(), Some(1), "empty lanes cannot close a held failed take");
    let route = publication::Route { address: Some(address), time_offset: 0.0 };
    for i in 0..publication::PUBLICATION_RING - 1 {
        let time = i as f64 / 48000.0;
        let event = if i % 2 == 0 {
            harmonigraph_core::NoteEvent::on(time, SourceId::DIRECT, 0, 60, 0.8)
        } else {
            harmonigraph_core::NoteEvent::off(time, SourceId::DIRECT, 0, 60)
        };
        recorder.publish_note(event.into(), route).expect_both();
    }
    assert_eq!(recorder.publication_free(), publication::Lanes::both(0));
    recorder.publication_lost(4096.0 / 48000.0, route);
    recorder.retired_publication_complete();
    assert_eq!(
        recorder.publication_free(),
        publication::Lanes::both(0),
        "hold release needs no ordinary publication slot"
    );
    writer.drain(&mut capture);
    assert!(
        writer.current_pass().is_none(),
        "full lane and independent loss must drain before failure closes"
    );
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_some());
    assert_eq!(
        take.events
            .iter()
            .filter(|record| matches!(record, harmonigraph_take::CanonicalRecord::Delta(_)))
            .count(),
        publication::PUBLICATION_RING - 1
    );
    assert!(take.events.iter().any(|record| matches!(record, harmonigraph_take::CanonicalRecord::Gap(gap) if gap.first == 4096 && gap.last == 4096)));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn configuration_audio_exhaustion_marks_the_recording_incomplete() {
    let mut b = Bench::new();
    b.rec.enable_configuration();
    b.rec.audio(&mut std::iter::repeat(0.0), 1026);
    assert!(b.rec.fence.failed.load(Ordering::Acquire));
    assert_eq!(b.dropped.load(Ordering::Relaxed), 1);
    assert_eq!(b.samples.slots(), 1024, "the fixture must fill the actual ring");
}

/// `mark_audio_start` is idempotent per PASS: the first call fixes where
/// the WAV sits against the notes, and that pass's header cannot be revised
/// afterwards. A wrap begins a new pass, whose audio starts somewhere else.
#[test]
fn the_audio_start_is_declared_once_per_pass() {
    let mut b = Bench::new();
    b.arm();
    b.rec.mark_audio_start(0.25);
    b.rec.mark_audio_start(0.30);
    b.rec.mark_audio_start(9.0);
    assert_eq!(b.pushed(), ["audio-start @0.25"], "the first call is the one that counts");

    // The wrap re-arms it, because the next pass's audio starts elsewhere.
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    assert!(b.rec.observe_transport(2.0, true, 64.0 / 48_000.0));
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    b.rec.mark_audio_start(7.5);
    assert_eq!(b.pushed(), ["new-pass", "audio-start @7.5"]);
}

/// Only parameters that MOVED are recorded — a full set every block would
/// bury the take — and a wrap rewrites all of them, because the new pass's
/// file starts empty and would otherwise replay with whatever the previous
/// pass happened to end on.
#[test]
fn params_record_only_changes_and_a_wrap_rewrites_every_one() {
    let mut b = Bench::new();
    b.arm();
    let mut values = [0.0f32; ParamKey::ALL.len()];
    b.rec.params(0.0, values);
    assert_eq!(
        b.pushed().len(),
        ParamKey::ALL.len(),
        "arming resets to NaN, so a take's first block writes a full set"
    );

    b.rec.params(1.0, values);
    let unmoved = b.pushed();
    assert!(unmoved.is_empty(), "an unmoved parameter is not re-recorded: {unmoved:?}");

    values[0] = 0.5;
    b.rec.params(2.0, values);
    assert_eq!(b.pushed(), ["param 0=0.5 @2"], "only the one that moved");

    // The wrap opens an empty file, so every parameter is written again
    // even though none of them changed.
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    assert!(b.rec.observe_transport(2.0, true, 64.0 / 48_000.0));
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    b.rec.params(3.0, values);
    let after = b.pushed();
    assert_eq!(
        after.len(),
        ParamKey::ALL.len() + 1,
        "the new pass, then a full set for it: {after:?}"
    );
    assert_eq!(after[0], "new-pass");
}

#[test]
fn resolved_boundaries_keep_sample_times_and_restart_each_take_pass() {
    let mut b = Bench::new();
    b.arm();
    let mut reducer = harmonigraph_core::configuration::ConfigReducer::default();
    let first = reducer.resolved();
    b.rec.configuration(0.0, first);
    b.rec.configuration(0.5, first);
    reducer.apply(harmonigraph_core::configuration::ConfigMutation::Edit(
        harmonigraph_core::configuration::ConfigEdit::axis(1, 696_000_000),
    ));
    let second = reducer.resolved();
    b.rec.configuration(31.0 / 48000.0, second);
    for (time, expected) in [(0.0, first), (31.0 / 48000.0, second)] {
        let Entry::Configuration(record) = b.entries.pop().unwrap() else {
            panic!("configuration record");
        };
        assert_eq!(record.t, time);
        assert_eq!(record.resolved(), expected);
    }
    assert!(b.entries.pop().is_err());
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    assert!(b.rec.observe_transport(2.0, true, 64.0 / 48_000.0));
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    assert!(matches!(b.entries.pop().unwrap(), Entry::NewPass));
    b.rec.configuration(0.0, second);
    assert!(
        matches!(b.entries.pop().unwrap(), Entry::Configuration(_)),
        "a new empty pass must carry its initial configuration"
    );
}

/// The reserved samples actually reach the ring.
///
/// The counter beside them is a different claim: `audio` could reserve the
/// right amount, count nothing dropped, and write nothing at all, and every
/// assertion about `dropped` would still hold while the WAV came out empty.
#[test]
fn reserved_audio_is_actually_written() {
    let mut b = Bench::new();
    b.arm();
    b.rec.audio(&mut [0.25f32, -0.25, 0.5, -0.5].into_iter(), 4);
    assert_eq!(b.written(), [0.25, -0.25, 0.5, -0.5]);
    assert_eq!(b.dropped.load(Ordering::Relaxed), 0);

    // An odd block still writes its whole frames; only the half frame on
    // the end is dropped.
    b.rec.audio(&mut [1.0f32, -1.0, 0.75].into_iter(), 3);
    assert_eq!(b.written(), [1.0, -1.0], "the whole frame lands, the half frame does not");
    assert_eq!(b.dropped.load(Ordering::Relaxed), 1);
}

/// The take's state resets on the arming EDGE, not on the armed LEVEL.
///
/// `process` calls `is_armed` every block, so a reset keyed off "armed"
/// rather than "newly armed" would fire on every block of the take:
/// `last_position` would forget the previous block, so no wrap could ever
/// be detected, and the loop-end latch would clear as fast as it was set.
#[test]
fn re_checking_armed_mid_take_does_not_reset_the_take() {
    let mut b = Bench::new();
    b.arm();
    b.end_at_rewind();
    assert!(b.rec.observe_transport(0.0, true, 64.0 / 48_000.0));
    assert!(b.rec.is_armed(), "still armed, one block later");
    assert!(b.rec.observe_transport(2.0, true, 64.0 / 48_000.0));
    assert!(b.rec.is_armed());

    // The wrap is still seen as one, and still ends the take.
    assert!(!b.rec.observe_transport(0.0, true, 64.0 / 48_000.0), "the wrap ends the take");
    assert!(b.hit_rewind());
    assert!(b.rec.is_armed());
    assert!(b.hit_rewind(), "the latch survives the next block's arm check");
    assert!(!b.rec.observe_transport(1.0, true, 64.0 / 48_000.0), "and the take stays finished");
}

/// The status line separates a take that is capturing from one that is
/// waiting, and from one that captured something and then stopped.
///
/// It is the only feedback there is that arming while the transport is
/// parked records nothing — "paused (0 events)" and "armed — waiting" look
/// alike but mean opposite things about whether the take has anything in
/// it.
#[test]
fn the_status_line_separates_waiting_from_paused_from_recording() {
    let (_rec, ctrl) = channel();
    ctrl.recording.store(true, Ordering::Relaxed);

    ctrl.tick(false, 0);
    assert_eq!(ctrl.status(), "armed — waiting for the transport to roll");

    ctrl.tick(true, 12);
    assert_eq!(ctrl.status(), "recording — 12 events");

    // Rolled, then stopped: what it caught is worth saying, and it is not
    // the same as never having started.
    ctrl.tick(false, 12);
    assert_eq!(ctrl.status(), "paused (12 events) — transport stopped");
}

#[test]
fn gui_ticks_cannot_hide_a_nondrop_recording_ownership_failure() {
    let (recorder, mut control) = channel();
    // Isolate the GUI's status cell from worker scheduling: the actual
    // shared recorder failure flag must make tick itself publish the error.
    control.status = Arc::new(Mutex::new(String::new()));
    control.recording.store(true, Ordering::Relaxed);
    recorder.enable_configuration();
    recorder.fail_configuration();
    assert_eq!(control.dropped.load(Ordering::Relaxed), 0);
    for (rolling, events) in [(false, 0), (true, 12), (false, 12)] {
        control.tick(rolling, events);
        assert!(control.status().contains("recording incomplete"), "{}", control.status());
    }
}

/// `stop` is only for a take that is running, and it stops the audio
/// capture along with the notes.
///
/// The Video pane reaches it from more than one place — the toggle, and the
/// loop-end handler that fires when the audio thread latches `hit_rewind`
/// — so it has to be safe to call twice. Disarming twice is harmless, but
/// sending a second `Stop` is not: the writer would close a file it had
/// already closed and launch a second render of the take the first `Stop`
/// is still finishing.
#[test]
fn stopping_a_take_that_is_not_running_does_nothing() {
    let (mut rec, ctrl) = channel();
    ctrl.fence.intent.store((ctrl.fence.epoch() + 1) << 1 | 1, Ordering::Release);
    ctrl.with_audio.store(true, Ordering::Relaxed);
    assert!(rec.is_armed());
    ctrl.stop(None);
    assert!(
        ctrl.fence.intent.load(Ordering::Acquire) & 1 != 0,
        "not recording, so there is nothing to stop"
    );
    assert!(rec.wants_audio(), "and nothing to stop reading the selected audio for");

    ctrl.recording.store(true, Ordering::Relaxed);
    ctrl.stop(None);
    assert_eq!(ctrl.fence.intent.load(Ordering::Acquire) & 1, 0, "a running take disarms");
    assert!(!ctrl.is_recording());
    assert!(rec.wants_audio(), "the observed callback still owns its audio");
    assert!(!rec.is_armed());
    assert!(!rec.wants_audio(), "and the audio thread stops reading the selected audio");
}

/// The rings the plugin ships with have room in them.
///
/// A record that does not fit IS counted and surfaced — but the surfacing
/// is a status line the user has to be looking at, and a ring sized to
/// nothing would drop every record of every take while the pane went on
/// saying it was recording. The capacities are the only thing between that
/// and a silently empty video, and nothing else here reads them.
#[test]
fn the_shipped_rings_have_room_for_what_the_audio_thread_pushes() {
    let (mut rec, ctrl) = channel();
    ctrl.recording.store(true, Ordering::Relaxed);
    rec.note(0.0, SourceId::DIRECT, 0, 60, NoteEventKind::On { velocity: 1.0 });
    rec.audio(&mut std::iter::repeat_n(0.0f32, 512), 512);
    ctrl.tick(true, 1);
    assert!(!ctrl.status().contains("DROPPED"), "a shipped ring dropped: {}", ctrl.status());
}

/// What the audio thread saw is what the GUI reads back.
///
/// `rolling` is published by `observe_transport` and reached only through
/// `Control` — it is the GUI's one honest view of whether the transport is
/// moving, and both the status line and the "waiting for the transport"
/// message hang off it. A getter wired to the wrong atomic would leave the
/// pane confidently describing a take that is not being recorded.
#[test]
fn the_gui_reads_back_the_rolling_the_audio_thread_published() {
    let (mut rec, ctrl) = channel();
    ctrl.fence.intent.store((ctrl.fence.epoch() + 1) << 1 | 1, Ordering::Release);
    assert!(rec.is_armed());

    assert!(!rec.observe_transport(3.0, false, 64.0 / 48_000.0));
    assert!(!ctrl.is_rolling(), "parked");

    assert!(rec.observe_transport(3.0 + 64.0 / 48_000.0, false, 64.0 / 48_000.0));
    assert!(ctrl.is_rolling(), "the position advanced");

    assert!(!rec.observe_transport(3.0 + 64.0 / 48_000.0, false, 64.0 / 48_000.0));
    assert!(!ctrl.is_rolling(), "parked again");
}

/// "Re-render take" with nothing recorded yet says so, rather than
/// appearing to work.
#[test]
fn re_rendering_with_no_take_yet_explains_itself() {
    let (_rec, ctrl) = channel();
    assert_eq!(ctrl.last_take(), None, "nothing recorded this session");
    ctrl.fence.fail_with_message("cannot write take.wav".into());
    *ctrl.status.lock() = CONFIGURATION_FAILURE.into();
    assert!(ctrl.status().contains("cannot write take.wav"));
    ctrl.render_now(RenderRequest::render_now(&RenderConfig::default(), "(dummy)".into()));
    assert_eq!(ctrl.status(), "no take recorded yet to render");
    ctrl.start(48_000.0, String::new(), true);
    assert!(ctrl.status().contains("reload the plugin"));

    // And the finished take the writer thread reports is the one the button
    // reaches for.
    let path = std::path::PathBuf::from("/takes/take-1.take");
    *ctrl.last_take.lock() = Some(path.clone());
    assert_eq!(ctrl.last_take(), Some(path));
}

/// The first pass keeps the take's own name; later passes are suffixed.
///
/// `path_for` belongs to the writer thread but is pure, so unlike the rest
/// of that half it is reachable from here — and worth reaching, because
/// getting the boundary backwards would have pass 1 write `-1` beside the
/// name its header advertises, and pass 2 write over the file pass 1 had
/// just finished.
#[test]
fn the_first_pass_keeps_the_takes_name_and_later_passes_are_suffixed() {
    let base = std::path::Path::new("/takes/take-1700000000.take");
    assert_eq!(Open::path_for(base, 1), base);
    assert_eq!(Open::path_for(base, 2), std::path::Path::new("/takes/take-1700000000-2.take"));
    assert_eq!(Open::path_for(base, 3), std::path::Path::new("/takes/take-1700000000-3.take"));
}
