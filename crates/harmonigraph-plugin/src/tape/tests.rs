//! The deck and the worker together, without threads: the worker's steps are
//! called by hand between blocks, which is what the input worker does on its
//! own clock.

use std::sync::Arc;

use super::deck::{BlockIn, BlockOut};
use super::store::{FADE_SECONDS, SETTLE_SECONDS};
use super::*;
use crate::audio_ingress::{Block, Format};

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
        song: song.map(|frame| frame as f64 / RATE),
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
    worker.settle();
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
    let block =
        BlockIn { position: Some(position), playing, frames, rate: RATE, enabled, silent: true };
    let out = {
        let mut planes = [left.as_mut_slice(), right.as_mut_slice()];
        deck.process(block, &mut planes)
    };
    ([left, right], out)
}

/// Park the playhead at `position` for long enough that the stop's grace has
/// passed.
fn park(deck: &mut Deck, position: f64) {
    for _ in 0..(deck::STOP_GRACE_SECONDS * RATE / FRAMES as f64) as usize + 2 {
        assert!(!run(deck, position, false, true).1.sounding);
    }
}

/// Click at `position` and hold it there for a block: the cue starts.
fn click(deck: &mut Deck, position: f64) {
    let (audio, out) = run(deck, position, false, true);
    assert!(!out.sounding && audio[0].iter().all(|&s| s == 0.0), "a click alone starts nothing");
    let (audio, out) = run(deck, position, false, true);
    assert!(out.sounding, "a click held still for a block cues the tape");
    assert!(audio[0].iter().all(|&s| s == 0.0), "nothing assembled yet");
}

fn events(worker: &mut Worker) -> Vec<String> {
    worker.report(std::time::Instant::now());
    std::mem::take(&mut worker.logs)
}

#[test]
fn a_stopped_click_plays_the_recording_from_there_and_rolling_cuts_it() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    // The deck heard the same playback, then the stop with its return.
    for song in (0..3000).step_by(FRAMES) {
        let (audio, out) = run(&mut deck, song as f64 / RATE, true, true);
        assert_eq!(out, BlockOut { song: Some(song as f64 / RATE), sounding: false });
        assert!(audio[0].iter().all(|&s| s == 0.0));
    }
    // Bitwig puts the playhead back where playback began: never a click.
    park(&mut deck, 0.0);

    click(&mut deck, 1.0);
    worker.play();
    let (audio, out) = run(&mut deck, 1.0, false, true);
    assert!(out.sounding && out.song.is_none());
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
    let (audio, out) = run(&mut deck, 1.0, true, true);
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
    park(&mut deck, 5.0);
    click(&mut deck, 0.8);
    let mut heard = 0;
    for _ in 0..10 {
        worker.play();
        let (audio, out) = run(&mut deck, 0.8, false, true);
        heard += audio[0].iter().filter(|&&s| s != 0.0).count();
        if !out.sounding {
            break;
        }
    }
    assert_eq!(heard, 200, "exactly the 0.2 s recorded after the cue");
    assert!(!deck.sounding());
    let lines = events(&mut worker);
    assert!(lines.iter().any(|l| l.contains("by the end of what is recorded")), "{lines:?}");

    click(&mut deck, 7.0);
    worker.play();
    let (audio, out) = run(&mut deck, 7.0, false, true);
    assert!(!out.sounding && audio[0].iter().all(|&s| s == 0.0));
    let lines = events(&mut worker);
    assert!(
        lines.iter().any(|l| l.contains("audition from 7.000 s (nothing recorded there)")),
        "{lines:?}"
    );
}

#[test]
fn a_climbing_position_with_playing_false_is_an_export_and_never_cues() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    park(&mut deck, 2.0);
    // The export starts elsewhere: one stopped move, then it climbs.
    let mut position = 0.0;
    for block in 0..20 {
        worker.play();
        let (audio, out) = run(&mut deck, position, false, true);
        assert!(!out.sounding && audio.iter().flatten().all(|&s| s == 0.0));
        // Every block after the first rolls, so the tape records the export.
        assert_eq!(out.song.is_some(), block > 0);
        position += FRAMES as f64 / RATE;
    }
    let lines = events(&mut worker);
    assert!(!lines.iter().any(|l| l.contains("audition")), "{lines:?}");
}

#[test]
fn switching_audition_off_fades_out_and_a_reset_cuts_at_once() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    park(&mut deck, 0.0);
    click(&mut deck, 1.0);
    worker.play();
    run(&mut deck, 1.0, false, true);
    worker.play();
    let (audio, out) = run(&mut deck, 1.0, false, false);
    let fade = (FADE_SECONDS * RATE).ceil() as usize;
    assert!(!out.sounding);
    assert!(
        audio[0][..fade].iter().all(|&s| s != 0.0) && audio[0][fade..].iter().all(|&s| s == 0.0)
    );
    assert!(audio[0][fade - 1].abs() < audio[0][0].abs());

    click(&mut deck, 1.5);
    worker.play();
    run(&mut deck, 1.5, false, true);
    deck.reset();
    assert!(!deck.sounding());
    let (audio, _) = run(&mut deck, 1.5, false, true);
    assert!(audio.iter().flatten().all(|&s| s == 0.0), "stale pieces are skipped after a reset");
    let lines = events(&mut worker);
    assert!(lines.iter().any(|l| l.contains("by Audition off")), "{lines:?}");
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
            silent: true,
        };
        nice_assert_no_alloc::assert_no_alloc(|| deck.process(input, &mut planes))
    };
    for song in 0..30 {
        block(&mut deck, song as f64 * 0.1, true);
    }
    for _ in 0..10 {
        block(&mut deck, 0.0, false);
    }
    block(&mut deck, 1.0, false);
    block(&mut deck, 1.0, false);
    worker.play();
    assert!(block(&mut deck, 1.0, false).sounding);
    // A click mid-audition fades, a drag, then rolling cuts and a reset.
    block(&mut deck, 1.5, false);
    block(&mut deck, 1.6, false);
    block(&mut deck, 1.7, true);
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
    worker.settle();
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

#[test]
fn notes_land_at_their_song_position_and_tuning_opens_each_chunk() {
    use harmonigraph_core::notes::{NoteEvent, SourceId};
    let (_, mut worker) = pair();
    let note =
        |time: f64| (time, Mark::Note(NoteEvent::on(time, SourceId::DIRECT, 0, 60, 0.8).into()));
    // Presentation 0.05 s is in the first rolling block, at song 4.05 s; 0.95
    // s arrives before its audio does; 0.15 s fell in a stopped block.
    worker.inbox.extend([note(0.05), note(0.95)]);
    for (first, song) in [(0, Some(4000)), (100, None), (200, Some(4200))] {
        let (block, samples) = ring_block(first, song, STEREO);
        worker.capture(block, &samples);
    }
    worker.inbox.push(note(0.15));
    worker.settle();
    for first in (300..2300).step_by(FRAMES) {
        let (block, samples) = ring_block(first, Some(4200 + first as i64 - 200), STEREO);
        worker.capture(block, &samples);
    }
    let (block, samples) = ring_block(2300, None, STEREO);
    worker.capture(block, &samples);
    worker.settle();
    let store = worker.store().unwrap();
    let notes: Vec<i64> = store
        .chunks()
        .flat_map(|c| c.marks.iter().filter(|(_, m)| matches!(m, Mark::Note(_))).map(|(t, _)| *t))
        .collect();
    assert_eq!(notes, [4050, 4950]);
    // The first tuning sample lands where the second pass is, and every chunk
    // after it opens with that tuning.
    assert_eq!(store.chunks().filter(|c| c.stamp.pass == 2).count(), 3);
    for chunk in store.chunks().filter(|c| c.stamp.pass == 2) {
        assert!(matches!(chunk.marks.first(), Some((t, Mark::Tuning(_))) if *t == chunk.start));
    }
}

/// A cue cut partway through a piece must leave nothing of that piece in the
/// playback ring: the next cue plays from its own first frame.
#[test]
fn a_click_during_an_audition_plays_the_new_spot_from_its_first_frame() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    park(&mut deck, 0.0);
    click(&mut deck, 1.0);
    worker.play();
    // One block of a piece that is longer than a block: the cut lands inside it.
    let (audio, _) = run(&mut deck, 1.0, false, true);
    assert!(audio[0].iter().any(|&s| s != 0.0));

    // Another click fades the first cue; held still, it cues the second.
    run(&mut deck, 2.0, false, true);
    let (_, out) = run(&mut deck, 2.0, false, true);
    assert!(out.sounding);
    worker.play();
    let (audio, _) = run(&mut deck, 2.0, false, true);
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

/// A host block shorter than the fade: the cue a click replaces still fades
/// all the way down before the new one starts.
#[test]
fn a_click_lets_the_old_cue_finish_fading_when_blocks_are_shorter_than_the_fade() {
    let (mut deck, mut worker) = pair();
    let mut source = 0;
    record(&mut worker, &mut source, 0, 3000);
    park(&mut deck, 0.0);
    click(&mut deck, 1.0);
    worker.play();
    run(&mut deck, 1.0, false, true);

    let fade = (FADE_SECONDS * RATE).ceil() as usize;
    assert!(fade > 2, "the fixture needs a fade longer than its two-frame blocks");
    // The click, then the playhead holding still, two frames at a time.
    let mut faded = Vec::new();
    for _ in 0..fade.div_ceil(2) {
        let (audio, _) = run_of(2, &mut deck, 2.0, false, true);
        faded.extend(audio[0].iter().copied());
    }
    assert!(!deck.sounding(), "the old cue is over and the new one has not started");
    assert!(faded[..fade].iter().all(|&s| s != 0.0), "the fade ran its whole length: {faded:?}");
    assert!(faded[..fade].windows(2).all(|w| w[1] < w[0]), "{faded:?}");
    assert!(faded[fade..].iter().all(|&s| s == 0.0));
    // Only then does the new spot cue.
    run_of(2, &mut deck, 2.0, false, true);
    worker.play();
    let (audio, _) = run(&mut deck, 2.0, false, true);
    assert_eq!(audio[0][fade], recorded(2000 + fade as i64, 0));
}

/// A new sample rate clears the tape on the first block that carries it,
/// rolling or not: a click before the transport next rolls must find nothing,
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
