//! The deck and the worker together, without threads: the worker's steps are
//! called by hand between blocks, which is what the input worker does on its
//! own clock.

use harmonigraph_ui::tape::{Meter, Speed, TapeRequest, TapeView};
use harmonigraph_ui::SpectrumConfig;

use super::deck::{BlockIn, BlockOut};
use super::store::{FADE_SECONDS, SETTLE_SECONDS};
use super::*;
use crate::audio_ingress::{Block, Format, Song};

/// A low rate keeps chunks, fades and leads a few hundred frames long.
const RATE: f64 = 1000.0;
const FRAMES: usize = 100;
const STEREO: Format = Format { channels: 2, sample_rate: RATE as f32, sidechain: false };

fn pair() -> (Deck, Worker) {
    channel(Arc::new(crate::HarmonigraphParams::default()))
}

/// The recorded mix at song frame `frame`: distinct per frame and channel, so
/// a misplaced or swapped sample cannot pass.
fn recorded(frame: i64, channel: usize) -> f32 {
    let value = (frame + 1) as f32 * 1e-4;
    if channel == 0 {
        value
    } else {
        -value
    }
}

/// One analyzer-ring block of the song from song frame `song`, or of silence
/// with no song position when `song` is `None`.
fn ring_block(first_frame: u64, song: Option<i64>, format: Format) -> (Block, Vec<f32>) {
    ring_block_of(FRAMES, first_frame, song, format)
}

/// [`ring_block`] with a block length of its own.
fn ring_block_of(
    frames: usize,
    first_frame: u64,
    song: Option<i64>,
    format: Format,
) -> (Block, Vec<f32>) {
    let block = Block {
        epoch: 0,
        origin: 0.0,
        first_frame,
        frames,
        format,
        song: song.map(|frame| Song { seconds: frame as f64 / RATE, meter: None }),
        tape: crate::audio_ingress::TapePlay::Idle,
    };
    let samples = (0..frames as i64)
        .flat_map(|i| (0..format.channels).map(move |c| song.map_or(0.0, |s| recorded(s + i, c))))
        .collect();
    (block, samples)
}

/// Record song frames `from..to` as one pass, and stop.
fn record(worker: &mut Worker, source: &mut u64, from: i64, to: i64) {
    for song in (from..to).step_by(FRAMES) {
        let (block, samples) = ring_block(*source, Some(song), STEREO);
        worker.capture(block, &samples);
        *source += FRAMES as u64;
    }
    let (block, samples) = ring_block(*source, None, STEREO);
    worker.capture(block, &samples);
    *source += FRAMES as u64;
}

/// One block through the deck, over a silent stereo buffer.
fn run(deck: &mut Deck, position: f64, playing: bool, enabled: bool) -> ([Vec<f32>; 2], BlockOut) {
    run_of(FRAMES, deck, position, playing, enabled)
}

/// [`run`] with a block length of its own.
fn run_of(
    frames: usize,
    deck: &mut Deck,
    position: f64,
    playing: bool,
    enabled: bool,
) -> ([Vec<f32>; 2], BlockOut) {
    let mut left = vec![0.0; frames];
    let mut right = vec![0.0; frames];
    let block = BlockIn { position: Some(position), playing, frames, rate: RATE, enabled };
    let out = {
        let mut planes = [left.as_mut_slice(), right.as_mut_slice()];
        deck.process(block, &mut planes)
    };
    ([left, right], out)
}

/// Tape mode asks for a cue at `position` while the host reports the
/// transport stopped at `parked`: the next block cues the tape.
fn cue(deck: &mut Deck, worker: &mut Worker, parked: f64, position: f64) {
    worker.request(TapeRequest::Cue(position));
    let (audio, out) = run(deck, parked, false, true);
    assert!(out.sounding, "a request cues the tape on the next block");
    assert!(audio[0].iter().all(|&s| s == 0.0), "nothing assembled yet");
}

fn events(worker: &mut Worker) -> Vec<String> {
    worker.report();
    std::mem::take(&mut worker.logs)
}

#[test]
fn a_cue_plays_the_recording_from_there_and_rolling_cuts_it() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    // The deck heard the same playback, then the stop, which leaves the
    // playhead where playback stopped.
    for song in (0..3000).step_by(FRAMES) {
        let (audio, out) = run(&mut deck, song as f64 / RATE, true, true);
        assert_eq!(
            out,
            BlockOut {
                song: Some(song as f64 / RATE),
                sounding: false,
                play: crate::audio_ingress::TapePlay::Idle,
            }
        );
        assert!(audio[0].iter().all(|&s| s == 0.0));
    }
    let parked = 2.9;
    for _ in 0..5 {
        assert!(!run(&mut deck, parked, false, true).1.sounding, "a stop never starts the tape");
    }

    cue(&mut deck, &mut worker, parked, 1.0);
    worker.play();
    let (audio, out) = run(&mut deck, parked, false, true);
    assert!(out.sounding && out.song.is_none());
    let crate::audio_ingress::TapePlay::Heard { seconds, frames, speed, .. } = out.play else {
        panic!("{:?}", out.play);
    };
    assert_eq!((seconds, frames, speed), (1.0, FRAMES, 1.0), "the block says what it played");
    let fade = (FADE_SECONDS * RATE).ceil() as usize;
    for (i, (&l, &r)) in audio[0].iter().zip(&audio[1]).enumerate() {
        let gain = if i < fade { (i + 1) as f32 / fade as f32 } else { 1.0 };
        assert_eq!(
            (l, r),
            (recorded(1000 + i as i64, 0) * gain, recorded(1000 + i as i64, 1) * gain)
        );
    }
    let lines = events(&mut worker);
    assert!(
        lines.iter().any(|l| l.contains("audition from 1.000 s (warm (from the song's start)")),
        "{lines:?}"
    );

    // The transport starts from the parked playhead: a cut, with not one
    // sample of tape in playback's first block.
    let (audio, out) = run(&mut deck, parked, true, true);
    assert!(!out.sounding);
    assert!(audio.iter().flatten().all(|&s| s == 0.0));
    let lines = events(&mut worker);
    assert!(
        lines.iter().any(|l| l.contains("stopped after 0.10 s by the transport started")),
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l.contains("from the parked playhead")), "{lines:?}");
}

#[test]
fn audition_ends_at_the_edge_of_what_is_recorded_and_where_nothing_is() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 1000);
    cue(&mut deck, &mut worker, 5.0, 0.8);
    let mut heard = 0;
    for _ in 0..10 {
        worker.play();
        let (audio, out) = run(&mut deck, 5.0, false, true);
        heard += audio[0].iter().filter(|&&s| s != 0.0).count();
        if !out.sounding {
            break;
        }
    }
    assert_eq!(heard, 200, "exactly the 0.2 s recorded after the cue");
    assert!(!deck.sounding());
    let lines = events(&mut worker);
    assert!(lines.iter().any(|l| l.contains("by the end of what is recorded")), "{lines:?}");

    cue(&mut deck, &mut worker, 5.0, 7.0);
    worker.play();
    let (audio, out) = run(&mut deck, 5.0, false, true);
    assert!(!out.sounding && audio[0].iter().all(|&s| s == 0.0));
    let lines = events(&mut worker);
    assert!(
        lines.iter().any(|l| l.contains("audition from 7.000 s (nothing recorded there)")),
        "{lines:?}"
    );
}

/// An export reports `playing = false` while its position climbs. It rolls,
/// so a cue asked during it is dropped, and not played once it ends either.
#[test]
fn a_cue_asked_during_an_export_is_dropped() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    run(&mut deck, 2.0, false, true);
    // The export starts elsewhere: one stopped move, then it climbs.
    let mut position = 0.0;
    for block in 0..20 {
        if block == 5 {
            worker.request(TapeRequest::Cue(1.0));
        }
        worker.play();
        let (audio, out) = run(&mut deck, position, false, true);
        assert!(!out.sounding && audio.iter().flatten().all(|&s| s == 0.0));
        // Every block after the first rolls, so the tape records the export.
        assert_eq!(out.song.is_some(), block > 0);
        position += FRAMES as f64 / RATE;
    }
    for _ in 0..5 {
        worker.play();
        assert!(!run(&mut deck, position, false, true).1.sounding);
    }
    let lines = events(&mut worker);
    assert!(!lines.iter().any(|l| l.contains("audition")), "{lines:?}");
}

/// Assert `audio` is a fade-out over its first [`FADE_SECONDS`] and silence
/// after.
fn assert_faded(audio: &[Vec<f32>; 2]) {
    let fade = (FADE_SECONDS * RATE).ceil() as usize;
    assert!(
        audio[0][..fade].iter().all(|&s| s != 0.0) && audio[0][fade..].iter().all(|&s| s == 0.0)
    );
    assert!(audio[0][fade - 1].abs() < audio[0][0].abs());
}

#[test]
fn leaving_main_and_stop_fade_out_and_a_reset_cuts_at_once() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    cue(&mut deck, &mut worker, 0.0, 1.0);
    worker.play();
    run(&mut deck, 0.0, false, true);
    worker.play();
    let (audio, out) = run(&mut deck, 0.0, false, false);
    assert!(!out.sounding);
    assert_faded(&audio);

    cue(&mut deck, &mut worker, 0.0, 1.5);
    worker.play();
    run(&mut deck, 0.0, false, true);
    worker.request(TapeRequest::Stop);
    worker.play();
    let (audio, out) = run(&mut deck, 0.0, false, true);
    assert!(!out.sounding);
    assert_faded(&audio);

    cue(&mut deck, &mut worker, 0.0, 2.0);
    worker.play();
    run(&mut deck, 0.0, false, true);
    // A click while the plugin was inactive is not played on its return.
    worker.request(TapeRequest::Cue(0.5));
    deck.reset();
    assert!(!deck.sounding());
    let (audio, out) = run(&mut deck, 0.0, false, true);
    assert!(!out.sounding, "a request from before the reset is dropped");
    assert!(audio.iter().flatten().all(|&s| s == 0.0), "stale pieces are skipped after a reset");
    let lines = events(&mut worker);
    assert!(lines.iter().any(|l| l.contains("by the analysis input leaving Main")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("by Stop")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("plugin reset while the tape sounded")), "{lines:?}");
}

#[test]
fn mixing_and_observing_never_allocate() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    let mut left = vec![0.0; FRAMES];
    let mut right = vec![0.0; FRAMES];
    let mut block = |deck: &mut Deck, position: f64, playing: bool| {
        let mut planes = [left.as_mut_slice(), right.as_mut_slice()];
        let input = BlockIn {
            position: Some(position),
            playing,
            frames: FRAMES,
            rate: RATE,
            enabled: true,
        };
        nice_assert_no_alloc::assert_no_alloc(|| deck.process(input, &mut planes))
    };
    for song in 0..30 {
        block(&mut deck, song as f64 * 0.1, true);
    }
    for _ in 0..10 {
        block(&mut deck, 2.9, false);
    }
    worker.request(TapeRequest::Cue(1.0));
    block(&mut deck, 2.9, false);
    worker.play();
    assert!(block(&mut deck, 2.9, false).sounding);
    // A cue mid-audition fades and cues again, then rolling cuts and a reset.
    worker.request(TapeRequest::Cue(1.5));
    block(&mut deck, 2.9, false);
    block(&mut deck, 2.9, false);
    worker.request(TapeRequest::Stop);
    block(&mut deck, 2.9, false);
    block(&mut deck, 2.9, true);
    nice_assert_no_alloc::assert_no_alloc(|| deck.reset());
}

#[test]
fn passes_split_at_jumps_and_stops_and_a_dropped_stretch_leaves_a_gap() {
    let (_, mut worker) = pair();
    let mut source = 0;
    // One pass from the song's start, with 200 frames dropped by a full ring.
    for song in [0, 100, 200, 500, 600] {
        if song == 500 {
            source += 200;
        }
        let (block, samples) = ring_block(source, Some(song), STEREO);
        worker.capture(block, &samples);
        source += FRAMES as u64;
    }
    // A jump starts a second pass at 2.0 s, and a sidechain block ends it.
    for song in [2000, 2100] {
        let (block, samples) = ring_block(source, Some(song), STEREO);
        worker.capture(block, &samples);
        source += FRAMES as u64;
    }
    let (block, samples) = ring_block(source, Some(2200), Format { sidechain: true, ..STEREO });
    worker.capture(block, &samples);
    let chunks: Vec<_> = worker
        .store()
        .unwrap()
        .chunks()
        .map(|c| (c.start, c.end, c.stamp.pass, c.stamp.pass_start))
        .collect();
    assert_eq!(chunks, [(0, 300, 1, 0), (500, 700, 1, 0), (2000, 2200, 2, 2000)]);
}

#[test]
fn a_cold_pass_does_not_replace_warm_audio_until_it_has_settled() {
    let (_, mut worker) = pair();
    let mut source = 0;
    let settle = (SETTLE_SECONDS * RATE) as i64;
    record(&mut worker, &mut source, 0, 1000 + settle + 2000);
    // Played again from 1.0 s: cold for the settling time, warm after.
    record(&mut worker, &mut source, 1000, 1000 + settle + 2000);
    let store = worker.store().unwrap();
    let owner =
        |frame: i64| store.chunks().find(|c| (c.start..c.end).contains(&frame)).unwrap().stamp.pass;
    assert_eq!(owner(1000), 1, "cold never overwrites warm");
    assert_eq!(owner(1000 + settle - 1), 1);
    assert_eq!(owner(1000 + settle + 500), 2, "once settled, the fresher pass wins");
}

/// A cue cut partway through a piece must leave nothing of that piece in the
/// playback ring: the next cue plays from its own first frame.
#[test]
fn a_cue_during_an_audition_plays_the_new_spot_from_its_first_frame() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    cue(&mut deck, &mut worker, 0.0, 1.0);
    worker.play();
    // One block of a piece that is longer than a block: the cut lands inside it.
    let (audio, _) = run(&mut deck, 0.0, false, true);
    assert!(audio[0].iter().any(|&s| s != 0.0));

    // Another cue fades the first; the next block cues the second.
    worker.request(TapeRequest::Cue(2.0));
    run(&mut deck, 0.0, false, true);
    let (_, out) = run(&mut deck, 0.0, false, true);
    assert!(out.sounding);
    worker.play();
    let (audio, _) = run(&mut deck, 0.0, false, true);
    let fade = (FADE_SECONDS * RATE).ceil() as usize;
    for (i, (&l, &r)) in audio[0].iter().zip(&audio[1]).enumerate() {
        let gain = if i < fade { (i + 1) as f32 / fade as f32 } else { 1.0 };
        assert_eq!(
            (l, r),
            (recorded(2000 + i as i64, 0) * gain, recorded(2000 + i as i64, 1) * gain),
            "frame {i} after the second cue"
        );
    }
}

/// A slowed cue counts the song at its speed, so a block says how much of the
/// song it played; a new speed while it sounds cues again from the spot heard,
/// under a new serial though it continues where the last block ended.
#[test]
fn a_slowed_cue_counts_the_song_at_its_speed_and_a_new_speed_cues_again() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    let mut view = TapeView::default();
    worker.exchange(&mut view, &SpectrumConfig::default(), true);
    cue(&mut deck, &mut worker, 0.0, 1.0);
    // Assembled at full speed, and not yet heard: a new speed cues it again
    // from its own spot.
    worker.play();
    view.speed = Speed::Half;
    worker.exchange(&mut view, &SpectrumConfig::default(), true);
    let (_, out) = run(&mut deck, 0.0, false, true);
    assert!(out.sounding && out.play == crate::audio_ingress::TapePlay::Waiting);
    worker.play();
    let heard = |out: BlockOut| match out.play {
        crate::audio_ingress::TapePlay::Heard { cue, seconds, frames, speed } => {
            (cue, seconds, frames, speed)
        }
        play => panic!("{play:?}"),
    };
    let (audio, out) = run(&mut deck, 0.0, false, true);
    assert!(audio[0].iter().any(|&s| s != 0.0));
    let (slowed, seconds, frames, speed) = heard(out);
    assert_eq!((seconds, frames, speed), (1.0, FRAMES, 0.5));
    let (cue, seconds, _, _) = heard(run(&mut deck, 0.0, false, true).1);
    assert_eq!(cue, slowed, "one cue across its blocks");
    assert!((seconds - 1.05).abs() < 1e-9, "100 frames heard are 50 of song: {seconds}");

    view.speed = Speed::Full;
    worker.exchange(&mut view, &SpectrumConfig::default(), true);
    run(&mut deck, 0.0, false, true);
    let (_, out) = run(&mut deck, 0.0, false, true);
    assert!(out.sounding);
    worker.play();
    let (cue, seconds, frames, speed) = heard(run(&mut deck, 0.0, false, true).1);
    assert!((seconds - 1.1).abs() < 1e-9 && frames == FRAMES && speed == 1.0, "{seconds}");
    assert_ne!(cue, slowed, "the spot heard, cued again");
    let lines = events(&mut worker);
    let recued = lines.iter().filter(|l| l.contains("audition from 1.000 s (")).count();
    assert_eq!(recued, 2, "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("stopped after 0.10 s by another cue")), "{lines:?}");
}

/// A pass opens with the lattice map selected then and records a change of
/// selection where it happens, as it does the tuning.
#[test]
fn a_pass_records_the_map_selection_where_it_changes() {
    use nice_plug::params::InternalParamMut;
    let params = Arc::new(crate::HarmonigraphParams::default());
    // SAFETY: as the host wrapper sets a parameter, with the parameter object
    // alive throughout.
    let select = |slot| assert!(unsafe { params.map._internal_set_plain_value(slot) });
    select(2);
    let (_, mut worker) = channel(params.clone());
    for (i, song) in [0, 100].into_iter().enumerate() {
        if i == 1 {
            select(3);
        }
        let (block, samples) = ring_block((i * FRAMES) as u64, Some(song), STEREO);
        worker.capture(block, &samples);
        worker.settle();
    }
    let slots: Vec<(i64, usize)> =
        worker.passes()[&1].drawn.iter().map(|(f, d)| (*f, d.map.slot)).collect();
    assert_eq!(slots, [(0, 2), (199, 3)]);
}

/// Two cold passes over the same stretch: the one that started earlier has
/// the longer run-up at every frame they share, so the later start only fills
/// what the earlier one left.
#[test]
fn a_cold_pass_that_started_later_never_replaces_one_that_started_earlier() {
    let (_, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 4000, 7000);
    record(&mut worker, &mut source, 4500, 7500);
    let store = worker.store().unwrap();
    let owners: Vec<(i64, i64, u64)> =
        store.chunks().map(|c| (c.start, c.end, c.stamp.pass)).collect();
    for &(start, end, pass) in &owners {
        assert_eq!(pass == 1, end <= 7000, "{start}..{end} belongs to pass {pass}: {owners:?}");
    }
}

/// A host block shorter than the fade: the cue a new one replaces still
/// fades all the way down before the new one starts.
#[test]
fn a_cue_lets_the_old_one_finish_fading_when_blocks_are_shorter_than_the_fade() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    cue(&mut deck, &mut worker, 0.0, 1.0);
    worker.play();
    run(&mut deck, 0.0, false, true);

    let fade = (FADE_SECONDS * RATE).ceil() as usize;
    assert!(fade > 2, "the fixture needs a fade longer than its two-frame blocks");
    // The request, then blocks of two frames.
    worker.request(TapeRequest::Cue(2.0));
    let mut faded = Vec::new();
    for _ in 0..fade.div_ceil(2) {
        let (audio, _) = run_of(2, &mut deck, 0.0, false, true);
        faded.extend(audio[0].iter().copied());
    }
    assert!(!deck.sounding(), "the old cue is over and the new one has not started");
    assert!(faded[..fade].iter().all(|&s| s != 0.0), "the fade ran its whole length: {faded:?}");
    assert!(faded[..fade].windows(2).all(|w| w[1] < w[0]), "{faded:?}");
    assert!(faded[fade..].iter().all(|&s| s == 0.0));
    // Only then does the new spot cue.
    run_of(2, &mut deck, 0.0, false, true);
    worker.play();
    let (audio, _) = run(&mut deck, 0.0, false, true);
    assert_eq!(audio[0][fade], recorded(2000 + fade as i64, 0));
}

/// A new sample rate clears the tape on the first block that carries it,
/// rolling or not: a cue before the transport next rolls must find nothing,
/// not the old recording at the wrong speed.
#[test]
fn a_stopped_block_at_a_new_rate_clears_the_tape() {
    let (_, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 1000);
    assert!(worker.store().is_some());
    let faster = Format { sample_rate: 2.0 * RATE as f32, ..STEREO };
    let (block, samples) = ring_block(source, None, faster);
    worker.capture(block, &samples);
    assert!(worker.store().is_none());
    // The sidechain's own format is not the main input's: it clears nothing.
    record(&mut worker, &mut source, 0, 1000);
    let sidechain = Format { channels: 1, sidechain: true, ..faster };
    let (block, samples) = ring_block(source, None, sidechain);
    worker.capture(block, &samples);
    assert!(worker.store().is_some());
}

/// A chunk closes on the block that takes it past its second. Blocks that do
/// not divide the second overrun the reserved buffer, which must not be kept
/// at the doubled size it grew to.
#[test]
fn a_committed_chunk_keeps_no_spare_half_of_its_buffer() {
    let (_, mut worker) = pair();
    let frames = 150;
    for i in 0..20 {
        let first = (i * frames) as u64;
        let (block, samples) = ring_block_of(frames, first, Some(first as i64), STEREO);
        worker.capture(block, &samples);
    }
    let (block, samples) = ring_block_of(frames, 3000, None, STEREO);
    worker.capture(block, &samples);
    let store = worker.store().unwrap();
    assert!(store.chunks().any(|c| c.end - c.start > RATE as i64), "no chunk overran its second");
    for chunk in store.chunks() {
        assert!(
            chunk.samples.capacity() < chunk.samples.len() * 3 / 2,
            "{} samples held in a buffer of {}",
            chunk.samples.len(),
            chunk.samples.capacity()
        );
    }
}

/// Record song frames `from..to` as one pass whose host reports `tempo`, and
/// stop.
fn at_tempo(worker: &mut Worker, source: &mut u64, from: i64, to: i64, tempo: f64) {
    let meter = Meter { beats: 0.0, tempo, bar_start: 0.0, bar: 0, numerator: 4, denominator: 4 };
    for song in (from..to).step_by(FRAMES) {
        let (mut block, samples) = ring_block(*source, Some(song), STEREO);
        block.song.as_mut().unwrap().meter = Some(meter);
        worker.capture(block, &samples);
        *source += FRAMES as u64;
    }
    let (block, samples) = ring_block(*source, None, STEREO);
    worker.capture(block, &samples);
    *source += FRAMES as u64;
}

/// Tape mode sees one span per pass and warmth, with gaps where nothing
/// was recorded, and the meter of the newest pass wherever passes overlap,
/// even where its audio lost.
#[test]
fn tape_mode_sees_spans_by_pass_and_warmth_the_newest_meter_and_the_spectrogram() {
    let (_, mut worker) = pair();
    let mut source = 0;
    // Warm from the song's start, then a cold stretch further on.
    at_tempo(&mut worker, &mut source, 0, 3000, 120.0);
    at_tempo(&mut worker, &mut source, 20_000, 21_000, 120.0);
    let mut view = TapeView::default();
    worker.exchange(&mut view, &SpectrumConfig::default(), true);
    let spans: Vec<(f64, f64, bool)> =
        view.spans.iter().map(|s| (s.start, s.end, s.warm)).collect();
    assert_eq!(spans, [(0.0, 3.0, true), (20.0, 21.0, false)]);
    assert_eq!(view.meters.iter().map(|(at, _)| *at).collect::<Vec<_>>(), [0.0, 1.0, 2.0, 20.0]);

    // A cold replay over warm audio loses every frame, but the song it heard
    // there is the newest: its meter replaces the old one.
    at_tempo(&mut worker, &mut source, 1000, 2000, 90.0);
    worker.exchange(&mut view, &SpectrumConfig::default(), true);
    let again: Vec<(f64, f64, bool)> =
        view.spans.iter().map(|s| (s.start, s.end, s.warm)).collect();
    assert_eq!(again, spans);
    let tempos: Vec<f64> = view.meters.iter().map(|(_, m)| m.tempo).collect();
    assert_eq!(tempos, [120.0, 90.0, 120.0, 120.0]);
    assert!(view.present && view.bytes > 0);

    // The spectrogram covers the recording once analyzed, a column every
    // 32 ms of the 4 s recorded, and reaches the view at the next exchange.
    assert!(view.spectrogram.is_empty());
    worker.analyze_all();
    worker.exchange(&mut view, &SpectrumConfig::default(), true);
    let columns = view.spectrogram.columns() as f64;
    assert!((columns - 4.0 / 0.032).abs() <= 2.0, "{columns} columns");
}

/// A pass replaces the meter only over what it recorded: a short pass across
/// another's anchor rules from its start to its end, and the meter it covered
/// rules again from there.
#[test]
fn a_short_pass_replaces_the_meter_only_where_it_played() {
    let (_, mut worker) = pair();
    let mut source = 0;
    at_tempo(&mut worker, &mut source, 0, 3000, 120.0);
    at_tempo(&mut worker, &mut source, 500, 1300, 90.0);
    let mut view = TapeView::default();
    worker.exchange(&mut view, &SpectrumConfig::default(), true);
    // 4/4: a bar is two seconds at 120, 2.67 at 90.
    let bar = |at: f64| (view.bar_seconds(at).unwrap() * 1000.0).round() / 1000.0;
    assert_eq!(bar(0.4), 2.0);
    assert_eq!(bar(1.2), 2.667);
    assert_eq!(bar(1.5), 2.0, "past the short pass's end");
    assert_eq!(view.meters.len(), 4, "{:?}", view.meters);
}

/// A block with no song position carries no tape, so rolling after one is not
/// rolling after tape.
#[test]
fn a_block_with_no_position_clears_the_tape_before_rolling() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    cue(&mut deck, &mut worker, 0.0, 1.0);
    worker.play();
    let (audio, _) = run(&mut deck, 0.0, false, true);
    assert!(audio[0].iter().any(|&s| s != 0.0), "the tape sounded");
    let block =
        BlockIn { position: None, playing: false, frames: FRAMES, rate: RATE, enabled: true };
    deck.process(block, &mut [&mut [0.0; FRAMES], &mut [0.0; FRAMES]]);
    run(&mut deck, 0.0, true, true);
    let lines = events(&mut worker);
    assert!(!lines.iter().any(|l| l.contains("carried tape")), "{lines:?}");
}

/// Stop while a new cue waits for the old one to fade: the fade finishes and
/// the waiting cue never starts.
#[test]
fn a_stop_cancels_a_cue_still_waiting_for_a_fade() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    cue(&mut deck, &mut worker, 0.0, 1.0);
    worker.play();
    run(&mut deck, 0.0, false, true);
    worker.request(TapeRequest::Cue(2.0));
    run_of(2, &mut deck, 0.0, false, true);
    worker.request(TapeRequest::Stop);
    for _ in 0..10 {
        worker.play();
        run_of(2, &mut deck, 0.0, false, true);
    }
    assert!(!deck.sounding());
    let lines = events(&mut worker);
    assert!(!lines.iter().any(|l| l.contains("audition from 2.000 s")), "{lines:?}");
    assert!(
        lines.iter().any(|l| l.contains("stopped after") && l.contains("by Stop")),
        "{lines:?}"
    );
}

/// Notes land at the song frame they sounded at, a note that arrives before
/// its block waits for it, and a pass opens with the voices already held
/// when it started, which a note struck before the transport rolled is.
#[test]
fn notes_are_placed_by_song_frame_and_a_pass_opens_with_what_is_held() {
    use super::notes::{key, Mark, Sounding};
    let (_, mut worker) = pair();
    let held = |note: u8| Mark::On {
        key: key(1, note),
        sounding: Sounding {
            velocity: 1.0,
            pitch: f32::from(note),
            expressions: harmonigraph_core::Expressions::NEUTRAL,
        },
    };
    let mut source = 0;
    // Stopped, at presentation 0..0.1 s: a 67 struck before playback.
    let (block, samples) = ring_block(source, None, STEREO);
    worker.capture(block, &samples);
    source += FRAMES as u64;
    worker.inbox.push((0.05, held(67)));
    worker.settle();
    // Rolling from the song's start at presentation 0.1 s. A 60 at 0.35 s
    // arrives a block before its audio does.
    for (i, song) in (0..500).step_by(FRAMES).enumerate() {
        if i == 1 {
            worker.inbox.push((0.35, held(60)));
        }
        let (block, samples) = ring_block(source, Some(song), STEREO);
        worker.capture(block, &samples);
        source += FRAMES as u64;
        worker.settle();
    }
    let events: Vec<(i64, Mark)> = worker.passes()[&1].events.clone();
    assert_eq!(events, [(0, held(67)), (250, held(60))]);

    // Stopped, the pass's audio is in the store, and Tape mode's roll, built
    // between holds, reaches the view at the exchange after.
    let (block, samples) = ring_block(source, None, STEREO);
    worker.capture(block, &samples);
    let mut view = TapeView { showing: true, ..Default::default() };
    // Not built while the roll is off.
    worker.exchange(&mut view, &SpectrumConfig { show_roll: false, ..Default::default() }, true);
    worker.roll();
    let rolled = SpectrumConfig { show_roll: true, ..Default::default() };
    worker.exchange(&mut view, &rolled, true);
    assert_eq!(view.notes.len(), 0, "built while the roll was off");
    worker.exchange(&mut view, &rolled, true);
    worker.roll();
    assert_eq!(view.notes.len(), 0, "built with no lock held, handed over next time");
    worker.exchange(&mut view, &rolled, true);
    let mut notes: Vec<(u8, f64, Option<f64>)> =
        view.notes.notes().map(|n| (n.note, n.start, n.end)).collect();
    notes.sort_by(|a, b| a.1.total_cmp(&b.1));
    assert_eq!(notes, [(67, 0.0, Some(0.5)), (60, 0.25, Some(0.5))]);
}

/// A mark dated before a pass's start but drained after the pass opened is
/// what that pass opened without: a loop's last note-off arriving a step
/// late would otherwise hold that note through the whole of the next pass.
#[test]
fn a_late_mark_from_before_a_pass_reaches_the_pass_that_opened_without_it() {
    use super::notes::{key, Mark, Sounding};
    let (_, mut worker) = pair();
    let held = Mark::On {
        key: key(1, 67),
        sounding: Sounding {
            velocity: 1.0,
            pitch: 67.0,
            expressions: harmonigraph_core::Expressions::NEUTRAL,
        },
    };
    let mut source = 0;
    let (block, samples) = ring_block(source, None, STEREO);
    worker.capture(block, &samples);
    source += FRAMES as u64;
    worker.inbox.push((0.05, held));
    worker.settle();
    // The pass starts at presentation 0.1 s and opens holding the 67 ...
    let (block, samples) = ring_block(source, Some(0), STEREO);
    worker.capture(block, &samples);
    worker.settle();
    // ... whose release, at 0.08 s, arrives only now.
    let off = Mark::Off { key: key(1, 67) };
    worker.inbox.push((0.08, off));
    worker.settle();
    assert_eq!(worker.passes()[&1].events, [(0, held), (0, off)]);
}
