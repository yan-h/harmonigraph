//! Recorder-output integration tests: the producer rings, GUI observations,
//! and private writer state share the transport fixtures here.

use super::*;

// 64 frames at 48 kHz; 120 BPM in 4/4 gives two seconds per bar.
const BLOCK_SECONDS: f64 = 64.0 / 48_000.0;
const BLOCK_BARS: f64 = BLOCK_SECONDS / 2.0;

/// The two fields [`header_for`] sets, each asserted against a value the
/// header could not have arrived at on its own: `Header`'s `Default` carries
/// `None` and `""`, so a field that stops being set produces a header that
/// still serializes and is simply wrong about the recording.
#[test]
fn the_header_carries_the_look_and_the_source() {
    let blob = "(camera:(distance:9.0),palette:magma)".to_owned();
    let header = header_for(blob.clone());
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

/// Independent workers receive the same candidate before either creates it.
/// Distinct headers/audio prove that both retained their own file pair.
#[test]
fn simultaneous_recordings_own_distinct_pairs() {
    let dir = std::env::temp_dir().join(format!("harmonigraph-collision-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let base = dir.join("same.take");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let paths = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|index| {
                let barrier = barrier.clone();
                let base = base.clone();
                scope.spawn(move || {
                    barrier.wait();
                    let mut recording =
                        Recording::create(header_for(index.to_string()), base).unwrap();
                    let audio = recording.audio.as_mut().unwrap();
                    audio.write(48_000.0, &[index as f32; 4]).unwrap();
                    (index, recording.finish().unwrap())
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
    });
    assert_ne!(paths[0].1, paths[1].1);
    for (index, path) in paths {
        let take = harmonigraph_take::Take::read(&path).unwrap();
        assert_eq!(take.header.appearance, Some(index.to_string()));
        let wav = path.with_extension("wav");
        assert_eq!(take.audio_file.as_deref(), wav.file_name().unwrap().to_str());
        let samples = std::fs::read(&wav).unwrap();
        assert_eq!(samples.len(), 60);
        assert_eq!(&samples[44..48], &(index as f32).to_le_bytes());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_collision_removes_only_owned_files() {
    let dir = std::env::temp_dir().join(format!("harmonigraph-owned-pair-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let base = dir.join("same.take");
    std::fs::write(base.with_extension("wav"), b"preexisting audio").unwrap();
    let recording = Recording::create(header_for(String::new()), base.clone()).unwrap();
    assert_eq!(recording.path, dir.join("same_1.take"));
    assert!(!base.exists(), "failed companion acquisition removes our empty take");
    assert_eq!(std::fs::read(base.with_extension("wav")).unwrap(), b"preexisting audio");
    drop(recording);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A writer-side failure reaches the status line whole: its cause, and that
/// no render started. A take that cannot be created is the case the status
/// read before the next GUI refresh showed as the bare cause (#1843).
#[test]
fn a_take_that_cannot_be_created_says_so_whole_on_the_status_line() {
    let Bench { ctrl, .. } = Bench::new();
    let file = std::env::temp_dir().join(format!("harmonigraph-not-a-dir-{}", std::process::id()));
    std::fs::write(&file, b"").unwrap();
    let mut pump = Pump::default();
    let header = header_for(String::new());
    pump.start(header, file.join("take.take"), &ctrl.status, &ctrl.fence);
    std::fs::remove_file(&file).unwrap();
    let status = ctrl.status();
    assert!(status.contains("cannot create") && status.contains("no render started"), "{status}");
}

/// A [`Recorder`] and its [`Control`] over small rings whose far ends the
/// test holds, with no writer thread.
///
/// A writer thread drains the rings within `DRAIN_IDLE`, so a test that asked
/// what the audio thread had actually PUSHED would be racing it for the
/// answer, and would usually lose. Holding the ends here is what makes the
/// take's CONTENTS assertable rather than only the counters beside them, and
/// those are different claims: `audio` can reserve correctly, count nothing
/// dropped, and write not one sample.
///
/// The transport and bar rules themselves are the lifecycle's, tabled in
/// `lifecycle/tests.rs`; what is tested here is how the two halves carry them.
struct Bench {
    rec: Recorder,
    ctrl: Control,
    ends: Ends,
}

impl Bench {
    /// Rings of 1,024 slots: room for every test here, and small enough that
    /// the overflow tests fill them.
    fn new() -> Bench {
        Bench::with_rings(1024, 1024)
    }

    fn with_rings(records: usize, audio: usize) -> Bench {
        let (rec, ctrl, ends) = endpoints(records, audio);
        Bench { rec, ctrl, ends }
    }

    /// Take the arming edge, as `process` does on a take's first block.
    fn arm(&mut self) {
        self.ctrl.fence.arm(self.ctrl.fence.epoch() + 1);
        assert!(self.rec.is_armed(), "the arming edge");
    }

    /// Everything pushed since the last call, rendered as comparable
    /// strings — [`Entry`] is a `Copy` payload for the audio thread and
    /// carries no `Debug` of its own.
    fn pushed(&mut self) -> Vec<String> {
        std::iter::from_fn(|| self.ends.records.pop().ok())
            .map(|entry| match entry {
                Entry::Param { t, key, value } => format!("param {key}={value} @{t}"),
                Entry::Run { song, .. } => format!("run @{song}"),
                Entry::RunEnd { .. } => "run-end".to_owned(),
                Entry::Configuration(_) => "configuration".to_owned(),
                Entry::AudioSamples { samples, .. } => format!("audio {samples}"),
            })
            .collect()
    }

    /// Every sample written to the audio ring since the last call.
    fn written(&mut self) -> Vec<f32> {
        std::iter::from_fn(|| self.ends.audio.pop().ok()).collect()
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
    let Bench { mut rec, ctrl, .. } = Bench::new();
    ctrl.recording.store(true, Ordering::Relaxed);

    // Even, into an empty ring: the whole block fits, nothing is dropped.
    rec.audio(&mut std::iter::repeat_n(0.25f32, 8), 8, 48_000.0);
    ctrl.tick(true, 0);
    assert!(
        !ctrl.status().contains("DROPPED"),
        "an even block fits whole, but got: {}",
        ctrl.status()
    );

    // Odd: the last sample is half a frame and must not reach the ring.
    rec.audio(&mut std::iter::repeat_n(0.25f32, 9), 9, 48_000.0);
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

/// Loop end, from the GUI's setting to the end it reads back: the setting
/// reaches the audio thread, the wrap ends the take there with nothing after
/// it recorded, and re-arming clears the end so the next take records from
/// scratch rather than starting finished.
#[test]
fn the_guis_loop_end_ends_the_take_at_the_wrap_until_the_next_arm() {
    let mut b = Bench::new();
    b.ctrl.set_end_at_rewind(true);
    b.arm();
    assert!(b.rec.observe_transport(1.0, true, BLOCK_SECONDS).is_some());
    assert!(b.rec.observe_transport(1.0 + 2.0 * BLOCK_SECONDS, true, BLOCK_SECONDS).is_some());
    assert!(!b.ctrl.has_ended(), "still mid-loop");
    assert!(b.rec.observe_transport(0.0, true, BLOCK_SECONDS).is_none(), "the wrap ends the take");
    assert!(b.ctrl.has_ended(), "the GUI is told to stop and render the take");
    assert!(b.rec.observe_transport(1.0, true, BLOCK_SECONDS).is_none(), "and it stays ended");

    b.ctrl.fence.disarm();
    assert!(!b.rec.is_armed());
    b.arm();
    assert!(!b.ctrl.has_ended(), "the end cleared on re-arm");
    assert!(b.rec.observe_transport(0.0, true, BLOCK_SECONDS).is_some(), "records again");
}

/// The GUI's stop bar reaches the audio thread through `Control`, and the
/// end comes back the same way.
#[test]
fn the_guis_stop_bar_reaches_the_audio_thread_and_its_end_comes_back() {
    let mut b = Bench::new();
    b.arm();
    b.ctrl.set_stop_bar(Some(5.0));
    assert!(!b.rec.observe_bar(Some(5.0 - BLOCK_BARS)));
    assert!(!b.ctrl.has_ended(), "short of the bar");
    assert!(b.rec.observe_bar(Some(5.0)));
    assert!(b.ctrl.has_ended(), "the GUI is told to stop and render");
}

#[test]
fn configuration_audio_exhaustion_marks_the_recording_incomplete() {
    let mut b = Bench::new();
    b.rec.audio(&mut std::iter::repeat(0.0), 1026, 48_000.0);
    assert!(b.ctrl.fence.failed.load(Ordering::Acquire));
    assert_eq!(b.ctrl.dropped.load(Ordering::Relaxed), 1);
    assert_eq!(b.ends.audio.slots(), 1024, "the fixture must fill the actual ring");
}

/// Only parameters that MOVED are recorded — a full set every block would
/// bury the take — after a full set as the run opens.
#[test]
fn params_record_only_changes() {
    let mut b = Bench::new();
    b.arm();
    b.rec.record(0.0, Some(0.0));
    assert_eq!(b.pushed(), ["run @0"]);
    let mut values = [0.0f32; ParamKey::ALL.len()];
    b.rec.params(0.0, values);
    assert_eq!(
        b.pushed().len(),
        ParamKey::ALL.len(),
        "a run opens with every parameter unwritten, so its first block writes a full set"
    );

    b.rec.params(1.0, values);
    let unmoved = b.pushed();
    assert!(unmoved.is_empty(), "an unmoved parameter is not re-recorded: {unmoved:?}");

    values[0] = 0.5;
    b.rec.params(2.0, values);
    assert_eq!(b.pushed(), ["param 0=0.5 @2"], "only the one that moved");
}

/// A configuration is written only when it changes, and each run writes the
/// one in force at its first block however unchanged since the run before:
/// an export opens at its take's first configuration, so a take without one
/// opens at the fresh tuning, and a run the reader cannot join to the one
/// before it replays from its own. Nothing is written, or remembered as
/// written, while no run is open, since the reader would drop it (#1832).
#[test]
fn each_run_opens_with_its_configuration_and_records_only_changes() {
    use harmonigraph_core::configuration::{ConfigEdit, ConfigMutation, ConfigReducer, TuningAxis};
    let mut reducer = ConfigReducer::default();
    let held = reducer.resolved();
    reducer.apply(ConfigMutation::Edit(ConfigEdit::axis(TuningAxis::Three, 690_000_000)));
    let edited = reducer.resolved();
    assert_ne!(held, edited, "the fixture's edit must change the configuration");
    let mut b = Bench::new();
    b.arm();
    b.rec.configuration(0.0, held);
    assert!(b.pushed().is_empty(), "no run is open");
    // Each run ends on the configuration the next opens with.
    for run in 0..2 {
        let opens = f64::from(run) * 10.0;
        b.rec.record(opens, Some(opens));
        for (t, config) in [(0.0, held), (1.0, held), (2.0, edited), (3.0, edited), (4.0, held)] {
            b.rec.configuration(opens + t, config);
        }
        b.rec.run_end(opens + 5.0);
        let written = b.pushed().iter().filter(|e| *e == "configuration").count();
        assert_eq!(written, 3, "run {run}: its opening configuration, then each change");
    }
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
    b.rec.audio(&mut [0.25f32, -0.25, 0.5, -0.5].into_iter(), 4, 48_000.0);
    assert_eq!(b.written(), [0.25, -0.25, 0.5, -0.5]);
    assert_eq!(b.pushed(), ["audio 4"], "and the ring says where they belong");
    assert_eq!(b.ctrl.dropped.load(Ordering::Relaxed), 0);

    // An odd block still writes its whole frames; only the half frame on
    // the end is dropped.
    b.rec.audio(&mut [1.0f32, -1.0, 0.75].into_iter(), 3, 48_000.0);
    assert_eq!(b.written(), [1.0, -1.0], "the whole frame lands, the half frame does not");
    assert_eq!(b.ctrl.dropped.load(Ordering::Relaxed), 1);
}

/// The take's state resets on the arming EDGE, not on the armed LEVEL.
///
/// `process` calls `is_armed` every block, so a reset keyed off "armed"
/// rather than "newly armed" would fire on every block of the take:
/// the position history would forget the previous block, so no wrap could ever
/// be detected, and the loop-end latch would clear as fast as it was set.
#[test]
fn re_checking_armed_mid_take_does_not_reset_the_take() {
    let mut b = Bench::new();
    b.arm();
    b.ctrl.set_end_at_rewind(true);
    assert!(b.rec.observe_transport(1.0, true, BLOCK_SECONDS).is_some());
    assert!(b.rec.is_armed(), "still armed, one block later");
    assert!(b.rec.observe_transport(1.0 + 2.0 * BLOCK_SECONDS, true, BLOCK_SECONDS).is_some());
    assert!(b.rec.is_armed());

    // The wrap is still seen as one, and still ends the take.
    assert!(
        !b.rec.observe_transport(0.0, true, 64.0 / 48_000.0).is_some(),
        "the wrap ends the take"
    );
    assert!(b.ctrl.has_ended());
    assert!(b.rec.is_armed());
    assert!(b.ctrl.has_ended(), "the latch survives the next block's arm check");
    assert!(
        !b.rec.observe_transport(1.0, true, 64.0 / 48_000.0).is_some(),
        "and the take stays finished"
    );
}

/// The status line separates a take that is capturing from one that is
/// waiting, and from one that captured something and then stopped.
///
/// It is the only feedback there is that arming while the transport is
/// parked records nothing — "paused · 0 events" and "waiting for playback" look
/// alike but mean opposite things about whether the take has anything in
/// it.
#[test]
fn the_status_line_separates_waiting_from_paused_from_recording() {
    let Bench { ctrl, .. } = Bench::new();
    ctrl.recording.store(true, Ordering::Relaxed);

    ctrl.tick(false, 0);
    assert_eq!(ctrl.status(), "waiting for playback");

    ctrl.tick(true, 12);
    assert_eq!(ctrl.status(), "recording · 12 events");

    // Rolled, then stopped: what it caught is worth saying, and it is not
    // the same as never having started.
    ctrl.tick(false, 12);
    assert_eq!(ctrl.status(), "paused · 12 events");
}

/// The shared failure flag alone makes `tick` publish the failure, whatever
/// the take is doing.
#[test]
fn gui_ticks_cannot_hide_a_nondrop_recording_ownership_failure() {
    let Bench { rec, ctrl, .. } = Bench::new();
    ctrl.recording.store(true, Ordering::Relaxed);
    rec.fail_configuration();
    assert_eq!(ctrl.dropped.load(Ordering::Relaxed), 0);
    for (rolling, events) in [(false, 0), (true, 12), (false, 12)] {
        ctrl.tick(rolling, events);
        assert!(ctrl.status().contains("recording incomplete"), "{}", ctrl.status());
    }
}

/// `stop` is only for a take that is running, and it stops the audio
/// capture along with the notes.
///
/// The Video pane reaches it from more than one place — the toggle, and the
/// end-of-take poll that fires when the audio thread latches an end — so it
/// has to be safe to call twice. Disarming twice is harmless, but sending a
/// second `Stop` is not: the writer would close a file it had already closed
/// and launch a second render of the take the first `Stop` is still
/// finishing.
#[test]
fn stopping_a_take_that_is_not_running_does_nothing() {
    let mut b = Bench::new();
    b.arm();
    b.ctrl.stop();
    assert!(b.ctrl.fence.intent().is_armed(), "not recording, so there is nothing to stop");
    assert!(b.ends.orders.try_recv().is_err(), "and no Stop reaches the writer");
    assert!(b.rec.wants_audio(), "and nothing to stop reading the selected audio for");

    b.ctrl.recording.store(true, Ordering::Relaxed);
    b.ctrl.stop();
    assert!(!b.ctrl.fence.intent().is_armed(), "a running take disarms");
    assert!(!b.ctrl.is_recording());
    assert!(b.rec.wants_audio(), "the observed callback still owns its audio");
    assert!(!b.rec.is_armed());
    assert!(!b.rec.wants_audio(), "and the audio thread stops reading the selected audio");
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
    let mut b = Bench::with_rings(TAKE_RING_CAPACITY, AUDIO_RING_CAPACITY);
    b.arm();
    b.rec.record(0.0, Some(0.0));
    b.rec.params(0.0, [0.5; ParamKey::ALL.len()]);
    b.rec.audio(&mut std::iter::repeat_n(0.0f32, 512), 512, 48_000.0);
    let pushed = b.pushed();
    assert_eq!(pushed.len(), ParamKey::ALL.len() + 2, "the run, every parameter, the audio");
    assert_eq!(pushed.last().map(String::as_str), Some("audio 512"), "{pushed:?}");
    assert_eq!(b.written().len(), 512, "every sample");
    assert_eq!(b.ctrl.dropped.load(Ordering::Relaxed), 0);
}

/// What the audio thread saw is what the GUI reads back.
///
/// `rolling` is published by `observe_transport` and reached only through
/// `Control` — it is the GUI's one honest view of whether the transport is
/// moving, and both the status line and the "waiting for the transport"
/// message hang off it. A getter wired to the wrong atomic would leave the
/// pane confidently describing a take that is not being recorded. `rolled`
/// beside it is what the Transport-stop countdown waits on: scrubs before
/// playback do not set it.
#[test]
fn the_gui_reads_back_the_rolling_the_audio_thread_published() {
    let mut b = Bench::new();
    b.arm();

    for position in [10.0, 30.0, 3.0] {
        assert!(!b.rec.observe_transport(position, false, BLOCK_SECONDS).is_some());
        assert!(!b.ctrl.is_rolling(), "parked");
        assert!(!b.ctrl.has_rolled(), "a scrub before playback has not rolled");
    }

    assert!(b.rec.observe_transport(3.0 + BLOCK_SECONDS, false, BLOCK_SECONDS).is_some());
    assert!(b.ctrl.is_rolling(), "the position advanced");
    assert!(b.ctrl.has_rolled());

    assert!(!b.rec.observe_transport(3.0 + BLOCK_SECONDS, false, BLOCK_SECONDS).is_some());
    assert!(!b.ctrl.is_rolling(), "parked again");
    assert!(b.ctrl.has_rolled(), "having rolled lasts the take");
}

/// Recording failure and last finished take remain available to the shell.
#[test]
fn the_shell_reads_recording_failure_and_last_finished_take() {
    let Bench { ctrl, .. } = Bench::new();
    assert_eq!(ctrl.last_take(), None, "nothing recorded this session");
    ctrl.fence.fail_with_message("cannot write take.wav".into());
    *ctrl.status.lock() = ctrl.fence.failure_status();
    assert!(ctrl.status().contains("cannot write take.wav"));

    // And the finished take the writer thread reports is the one the button
    // reaches for.
    let path = std::path::PathBuf::from("/takes/take-1.take");
    *ctrl.last_take.lock() = Some(path.clone());
    assert_eq!(ctrl.last_take(), Some(path));
}

/// `Invalid` is the one take-lane outcome with no gap to describe it, so a
/// refused publication while a take is armed fails the recording rather than
/// leaving a silent hole. One while nothing is armed is only counted.
#[test]
fn an_invalid_publication_into_an_armed_take_fails_the_recording() {
    let mut b = Bench::new();
    let note = harmonigraph_core::NoteEvent::on(f64::NAN, SourceId::DIRECT, 0, 60, 0.8).into();
    let invalid = Err(publication::PublishError::Invalid);
    assert_eq!(b.rec.publish_note(note).take, invalid, "an unusable time");
    assert_eq!(b.ctrl.dropped.load(Ordering::Relaxed), 1);
    assert!(!b.ctrl.fence.failed.load(Ordering::Acquire));
    b.arm();
    assert_eq!(b.rec.publish_note(note).take, invalid);
    assert!(b.ctrl.fence.failed.load(Ordering::Acquire), "a hole in a take fails it");
}

/// A Stop and a new Start with no callback between them still make an arming
/// edge: the second take records from its own first block, rather than
/// inheriting the first's history and ending at its own first rewind.
#[test]
fn a_take_re_armed_with_no_callback_between_records_from_its_first_block() {
    let mut b = Bench::new();
    b.arm();
    b.ctrl.set_end_at_rewind(true);
    assert!(b.rec.observe_transport(5.0, true, BLOCK_SECONDS).is_some());
    assert!(b.rec.observe_transport(5.0 + BLOCK_SECONDS, true, BLOCK_SECONDS).is_some());
    // Stop, then Start, before any callback observes the disarm.
    b.ctrl.fence.disarm();
    b.ctrl.fence.arm(b.ctrl.fence.epoch() + 1);
    assert!(b.rec.is_armed());
    // Rewound and played: the second take's first block.
    assert_eq!(b.rec.observe_transport(0.0, true, BLOCK_SECONDS), Some(0.0));
    assert!(!b.ctrl.has_ended(), "the first take's motion does not end the second");
}

/// Every run the recorder opens takes a new serial, since each costs a
/// snapshot of every sounding note (#1129): at a resume after a block that did
/// not record, at a new take, and after a host reset closed the last — and at
/// nothing else, a loop wrap included. The recorder opens and closes it
/// itself, from each block's origin.
#[test]
fn every_run_the_recorder_opens_takes_a_new_serial() {
    fn block(b: &mut Bench, position: f64, playing: bool) -> Option<u64> {
        let origin = b.rec.observe_transport(position, playing, BLOCK_SECONDS);
        b.rec.record(position, origin);
        b.rec.open_run()
    }
    let mut b = Bench::new();
    b.arm();
    let first = block(&mut b, 1.0, true);
    assert!(first.is_some(), "the first block recorded opens a run");
    assert_eq!(block(&mut b, 1.0 + BLOCK_SECONDS, true), first, "playback continues it");
    // Stopped where it was: not even the contiguous block an export plays.
    for _ in 0..2 {
        assert_eq!(block(&mut b, 1.0 + BLOCK_SECONDS, false), None, "a pause records nothing");
    }
    let resumed = block(&mut b, 1.0 + BLOCK_SECONDS, true);
    assert!(resumed.is_some() && resumed != first, "the resume opens one");
    block(&mut b, 1.0 + 2.0 * BLOCK_SECONDS, true);
    assert_eq!(block(&mut b, 0.0, true), resumed, "a plain wrap keeps recording in the same run");
    // Stop, then Start, before any callback observes the disarm.
    b.ctrl.fence.disarm();
    b.ctrl.fence.arm(b.ctrl.fence.epoch() + 1);
    assert!(b.rec.is_armed());
    let rearmed = block(&mut b, BLOCK_SECONDS, true);
    assert!(rearmed.is_some() && rearmed != resumed, "the new take opens its own");
    // A host reset closes the run mid-take, and playback rolls straight on.
    b.rec.run_end(2.0 * BLOCK_SECONDS);
    let restarted = block(&mut b, 2.0 * BLOCK_SECONDS, true);
    assert!(restarted.is_some() && restarted != rearmed, "the run after a reset is a new one");
}

/// Samples whose counting entry has no room in the record ring are not
/// written: left uncounted in the audio ring, they would open the next take's
/// soundtrack once the failure is cleared.
#[test]
fn audio_with_no_room_for_its_entry_stays_out_of_the_ring() {
    let mut b = Bench::with_rings(1, 1024);
    b.arm();
    b.rec.audio(&mut [0.5, -0.5].into_iter(), 2, 48_000.0);
    assert_eq!(b.ends.audio.slots(), 2, "the first block and its entry");
    b.rec.audio(&mut [0.25, -0.25].into_iter(), 2, 48_000.0);
    assert!(b.ctrl.fence.failed.load(Ordering::Acquire), "the full record ring fails the take");
    assert_eq!(b.ends.audio.slots(), 2, "the second block's samples went in uncounted");
}
