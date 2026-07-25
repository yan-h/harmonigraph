//! Note names: every note in the roll labeled at its onset, over its own
//! ribbon, in the lattice's own hand.
//!
//! What they answer is reading the SPECTROGRAM. A band of energy tells you
//! there is something at some height on a pitch axis that is continuous in
//! cents, and the only fixed marks on that axis are C gridlines a full octave
//! apart — so naming the band means counting semitones by eye from the nearest
//! C, on a picture that is scrolling. Color cannot say it either: the roll's
//! colors are already spent on pitch height (see [`crate::RollColor`]), and a
//! second pitch-keyed scheme laid over the first is two things to read where
//! there was one.
//!
//! So the roll says it, since the roll is already drawing the notes: a name at
//! the moment each one was struck, riding just clear of the ribbon it belongs
//! to. The heatmap band under a ribbon is the same note, so naming the ribbon
//! names the band.
//!
//! EVERY note, not one per pitch. The alternative — name a pitch the first
//! time it is played and rule a line forward to carry it — reads well on paper
//! and badly on the pane: the name lands wherever the material happened to
//! start, which before long is off the far edge, so the legend collects in the
//! oldest corner of the picture and the lines ruled to reach it cross
//! everything else. A name on each onset needs no line at all, because the
//! thing it names is already drawn underneath it.
//!
//! The name is the LATTICE's: the same [`NoteName`] the node carries, drawn by
//! the same [`draw_stacked_name`](super::lattice::draw_stacked_name) — letter
//! at full size, accidental riding high, syntonic-comma mark low, both counted
//! rather than repeated. Not a resemblance but the same function, so the two
//! cannot drift apart. That is the errand: a name here is read against the
//! lattice, so it has to answer in the lattice's vocabulary. A cents deviation
//! off the nearest piano key would say where a pitch sits between two keys
//! when what is wanted is which node it IS — and for a just third, `E-` says
//! it where "E +14\u{a2}" does not.
//!
//! Geometry comes from [`Axes`](super::spectral::Axes) like everything else in
//! the pane, so names turn and flip with it and nothing here names a screen
//! side.

use std::collections::HashMap;

use lattice_core::{LatticePos, NoteName, PitchClass, RollNote, Tuning};
use lattice_scene::ViewConfig;

use super::lattice;
use super::spectral::{Axes, PitchScale, TimeAxis};
use crate::{theme, SharedState};

/// Point size of a name's letter. Half a point under the axis labels': there
/// are many more of these, and they sit inside the picture rather than along
/// its edge.
const LABEL_PT: f32 = 9.5;

/// Points of clear air between a ribbon's edge and the name riding above it,
/// and how far the name is set in from the onset along the time axis.
///
/// The lift keeps the name off the ribbon rather than across it. A ribbon is a
/// couple of points thick at the zooms this pane is used at and a name is ten,
/// so a name centred on the note would have the note's own colour running
/// through the middle of the letter — losing both the letter and the one thing
/// the ribbon's colour is for.
const LABEL_LIFT: f32 = 2.0;
const LABEL_INSET: f32 = 2.0;

/// What a monospace glyph advances, and a line box stands, as fractions of the
/// font size — and the clear space a name demands around itself, in points.
///
/// An ESTIMATE, deliberately, rather than a galley measured through egui. It
/// decides only which names are dropped for colliding, so being a few percent
/// wide costs a name that would have just fitted and nothing else; against
/// that, measuring would put a text layout per candidate per frame in front of
/// a decision that is thrown away for most of them, and would make the offline
/// render's output depend on font metrics rather than on arithmetic.
const GLYPH_ADVANCE: f32 = 0.62;
const LINE_HEIGHT: f32 = 1.3;
const LABEL_PAD: f32 = 1.5;

/// Most notes considered for a name in one frame, newest first.
///
/// A bound on the placement, which is quadratic in what it accepts: every
/// candidate is tested against every name already placed. The Span reaches ten
/// minutes and the roll remembers four thousand notes, so without this a long
/// span would spend milliseconds proving that a wall of notes is too dense to
/// label — the most work at exactly the zoom where the fewest names fit.
///
/// Newest first because that is where the reading happens: the notes at the
/// now-line are the ones being played, so a name that lost its place to
/// something older would be the wrong one to drop.
const MAX_CANDIDATES: usize = 384;

/// One name, placed: what it says and the box it was measured into.
#[derive(Clone, Copy, Debug)]
pub(super) struct NoteLabel {
    pub name: NoteName,
    /// Screen box the name covers, padded. Its centre is where the name is
    /// drawn — [`lattice::draw_stacked_name`] centres on its anchor, while the
    /// placement here works in boxes that grow away from theirs.
    pub rect: egui::Rect,
}

/// Every name this frame draws, already thinned to the ones that fit — empty
/// when the setting is off or the pane has kept no roll region to draw in.
pub(super) fn plan(
    state: &SharedState,
    axes: &Axes,
    scale: &PitchScale,
    split: f32,
    now: f64,
    label_scale: f32,
) -> Vec<NoteLabel> {
    let cfg = &state.spectrum_config;
    if !cfg.note_names || split >= 1.0 {
        return Vec::new();
    }
    let time = TimeAxis::new(state, split, now);
    // Whole-song (offline playhead) reads the take's own roll, laid out
    // statically; live reads the causal tracker's scrolling one.
    let roll = match state.whole_song.as_ref() {
        Some(ws) => &ws.roll,
        None => state.tracker.roll(),
    };

    // Cull to what is on the pane before anything else: the roll remembers
    // thousands of notes while a handful are in the window, and everything
    // below is per-candidate work.
    let oldest = time.oldest();
    let mut notes: Vec<&RollNote> = roll
        .notes()
        .filter(|note| note.stop(now) >= oldest && scale.contains(note.settled_pitch()))
        .collect();
    // Newest first, and a total order: the offline render must not depend on
    // the order the roll happened to hand them back, and a name's place is
    // decided first-come.
    notes.sort_unstable_by(|a, b| {
        b.start
            .total_cmp(&a.start)
            .then(a.channel.cmp(&b.channel))
            .then(a.note.cmp(&b.note))
    });
    notes.truncate(MAX_CANDIDATES);

    let size = LABEL_PT * label_scale;
    // Time runs from the now-line at `split` out to the past live; whole-song
    // lays the whole take out from the near edge, so "later" is the far one.
    let d_now = if time.whole_song() { 1.0 } else { split };
    // Half a ribbon, in points — how far a name has to be lifted to clear the
    // note it names. In semitones of the pitch axis, like the ribbon itself.
    let half_ribbon = (cfg.roll_thickness * 0.5 / scale.span).max(0.0) * axes.pitch_len();

    // One naming per pitch CLASS rather than per note. Naming walks every
    // visible lattice node looking for the match that spells best, and a
    // passage is the same handful of classes over and over.
    let mut names: HashMap<i32, NoteName> = HashMap::new();
    let mut placed: Vec<NoteLabel> = Vec::new();
    for note in notes {
        let pitch = note.settled_pitch();
        let class = (pitch.rem_euclid(12.0) * 100.0).round() as i32;
        let name = *names
            .entry(class)
            .or_insert_with(|| note_name(&state.view, &state.tuning, pitch));
        // The onset, CLAMPED into the region: a note that began before the
        // window still has a ribbon on the pane, and the visible start of that
        // ribbon is where its name belongs.
        let d = time.depth_of(note.start);
        let rect = label_rect(axes, scale.t_of(pitch), d, d_now, &name, size, half_ribbon);
        if placed.iter().any(|other| other.rect.intersects(rect)) {
            continue;
        }
        placed.push(NoteLabel { name, rect });
    }
    placed
}

/// The screen box a name covers at pitch `p` and depth `d`, padded by the
/// clear space it demands around itself.
fn label_rect(
    axes: &Axes,
    p: f32,
    d: f32,
    d_now: f32,
    name: &NoteName,
    size: f32,
    half_ribbon: f32,
) -> egui::Rect {
    // Set in toward the end the note runs to, so the name lies along its own
    // ribbon rather than off the start of it.
    let into = if d_now > d { LABEL_INSET } else { -LABEL_INSET };
    // Lifted clear of the ribbon's own edge, so the note's colour stays
    // readable underneath its name.
    let (pos, align) = axes.text_anchor(p, d, half_ribbon + LABEL_LIFT, into);
    align.anchor_size(pos, name_extent(name, size)).expand(LABEL_PAD)
}

/// What a name covers, estimated from the sizes its pieces are laid out at.
///
/// A stacked name is a letter with a column of marks after it — see
/// [`lattice::draw_stacked_name`] — so its width is the letter plus the wider
/// mark, and its height is the letter's line box, which the marks are sized to
/// stay inside.
fn name_extent(name: &NoteName, size: f32) -> egui::Vec2 {
    let mark_size = size * lattice::MARK_SIZE / lattice::NAME_SIZE;
    let marks = name
        .accidental_mark()
        .chars()
        .count()
        .max(name.comma_mark().chars().count());
    egui::vec2((size + marks as f32 * mark_size) * GLYPH_ADVANCE, size * LINE_HEIGHT)
}

/// A note's name: the LATTICE's spelling of its pitch.
///
/// No octave number, because a lattice node is a pitch class and wears none
/// either — and the octave is already said by where the name sits on the axis.
///
/// The fallback, for a pitch the visible lattice has no node for, is the
/// equal-tempered spelling — still a [`NoteName`], so it draws identically and
/// there is one rendering path rather than two. It is a real case: the pane
/// already flags notes sounding off the lattice with a band down the spectrum,
/// and a note with no name at all would just look like a bug.
fn note_name(view: &ViewConfig, tuning: &Tuning, midi: f32) -> NoteName {
    // Cents from C, measured from MIDI 0 (which IS a C) — the same reduction
    // the pane's hover readout makes before asking the same question.
    let pc = PitchClass::from_cents(midi.rem_euclid(12.0) * 100.0);
    match naming_node(view, tuning, pc) {
        Some(pos) => super::display_note_name(pos, view.meantone),
        None => equal_tempered_name(midi),
    }
}

/// The visible node to name a pitch by: the closest match, and among matches
/// equally close the one that spells most plainly.
///
/// Its own function rather than
/// [`nearest_visible_node`](super::nearest_visible_node), which it otherwise
/// mirrors exactly, because of the tiebreak — and the tiebreak matters only
/// where that function has nothing to go on.
///
/// In a JUST tuning the two agree and there is nothing to break: distinct
/// lattice positions are distinct pitches, so at the half-cent tolerance
/// exactly one node can match. In an EQUAL temperament — which is the default
/// this plugin opens on — the lattice collapses: twelve fifths are seven
/// octaves exactly, so the origin and `(12,0,0)` are one pitch, three major
/// thirds are an octave, and a dozen visible nodes answer to middle C. All are
/// the same distance (zero) from it, so the plain minimum returns whichever
/// the iteration reached first, which is the CORNER of the visible window:
/// middle C names itself `F♭5+6`, and renames itself whenever the view is
/// panned. True, useless, and not what the lattice shows you, which is the lit
/// node you were looking at.
///
/// Left where it is rather than pushed into the shared function because the
/// shared one answers "which node do I light", where any of a collapsed set
/// will do, and this one answers "what do I call it", where they differ.
fn naming_node(view: &ViewConfig, tuning: &Tuning, pc: PitchClass) -> Option<LatticePos> {
    view.visible_positions()
        .filter(|&pos| tuning.matches(pc, tuning.pitch_class(pos)))
        .min_by_key(|&pos| {
            (
                pc.distance_to(tuning.pitch_class(pos)),
                spelling_cost(super::display_note_name(pos, view.meantone), pos),
            )
        })
}

/// How hard a spelling is to read, worst first: comma marks, then
/// accidentals, then how far out the node sits.
///
/// In that order because that is the order the marks cost a reader. Four
/// fifths up from C and one just third up from C are the same pitch in an
/// equal temperament, and they spell `E` and `E-`; the plain letter is the
/// name for it, even though the comma'd node is nearer the origin. The final
/// term only settles what the marks cannot, and keeps the answer from moving
/// with the view.
fn spelling_cost(name: NoteName, pos: LatticePos) -> (i32, i32, i32) {
    (
        name.syntonic_commas.abs(),
        name.sharps.abs(),
        pos.threes.abs() + pos.fives.abs() + pos.sevens.abs(),
    )
}

/// The nearest piano key, spelled with sharps — [`KEY_NAMES`](super::KEY_NAMES)
/// as a [`NoteName`] rather than as text, so it reaches the same drawing code.
fn equal_tempered_name(midi: f32) -> NoteName {
    const SPELLINGS: [(char, i32); 12] = [
        ('C', 0),
        ('C', 1),
        ('D', 0),
        ('D', 1),
        ('E', 0),
        ('F', 0),
        ('F', 1),
        ('G', 0),
        ('G', 1),
        ('A', 0),
        ('A', 1),
        ('B', 0),
    ];
    let (letter, sharps) = SPELLINGS[(midi.round() as i32).rem_euclid(12) as usize];
    NoteName { letter, sharps, syntonic_commas: 0 }
}

/// The names, into whichever batch the pane is drawing its labels from.
///
/// Drawn by [`lattice::draw_stacked_name`] — the lattice's own label code, not
/// a copy of it — so a note's name is the same glyphs in the same arrangement
/// as the node it lights up. Sharing the function rather than the look is what
/// keeps them from drifting apart the next time either is touched, and it is
/// why a name is carried this far as a [`NoteName`] rather than as a string.
///
/// Haloed like the axis labels and for the same reason: what is behind them is
/// a picture, not a background, and a name over a bright heatmap slab or a lit
/// ribbon has no contrast of its own to rely on.
pub(super) fn draw(
    painter: &egui::Painter,
    labels: &[NoteLabel],
    label_scale: f32,
    batch: &mut crate::text::TextBatch,
) {
    // `draw_stacked_name` sizes everything off the lattice's own letter size,
    // so ask it for the roll's smaller one as a fraction of that.
    let scale = LABEL_PT * label_scale / lattice::NAME_SIZE;
    for label in labels {
        lattice::draw_stacked_name(
            batch,
            painter,
            label.rect.center(),
            label.name,
            theme::text(),
            theme::well(),
            scale,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SpectralOrientation, SpectrumConfig};
    use lattice_core::{NoteEvent, NoteEventKind};

    /// 300 points along the time axis, 100 across pitch — the same pane the
    /// roll's tests use.
    const PANE: egui::Rect =
        egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 120.0) };

    /// A pitch axis the size a docked pane actually has: 700 points.
    ///
    /// The pitch range cannot be zoomed under an octave
    /// ([`PITCH_RANGE_MIN_SPAN`](crate::PITCH_RANGE_MIN_SPAN)), so across 100
    /// points a semitone is eight of them — less than a name is tall. Anything
    /// about naming NEIGHBOURING pitches therefore has to be asked of a pane
    /// with room to draw them apart, or it is asking about the test fixture.
    const BIG: egui::Rect =
        egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 720.0) };

    fn on(time: f64, note: u8) -> NoteEvent {
        NoteEvent { time, channel: 0, note, kind: NoteEventKind::On { velocity: 0.8 } }
    }

    fn off(time: f64, note: u8) -> NoteEvent {
        NoteEvent { time, channel: 0, note, kind: NoteEventKind::Off }
    }

    /// A per-note tuning on a sounding note, in semitones off its key.
    fn tuning(time: f64, note: u8, semitones: f32) -> NoteEvent {
        NoteEvent { time, channel: 0, note, kind: NoteEventKind::Tuning { semitones } }
    }

    /// A state whose pane shows `range` semitones around middle C over a
    /// `span`-second window, with the whole depth axis given to the roll.
    fn state(range: f32, span: f32) -> SharedState {
        let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config = SpectrumConfig {
            orientation: SpectralOrientation::Horizontal,
            low_midi: 60.0 - range * 0.5,
            high_midi: 60.0 + range * 0.5,
            roll_seconds: span,
            roll_fraction: 1.0,
            ..SpectrumConfig::default()
        };
        state
    }

    /// The names `state` would draw at `now`, placed exactly the way
    /// [`spectral_pane`](super::super::spectral::spectral_pane) places them.
    fn labels(state: &SharedState, now: f64) -> Vec<NoteLabel> {
        labels_in(state, now, PANE)
    }

    fn labels_in(state: &SharedState, now: f64, rect: egui::Rect) -> Vec<NoteLabel> {
        let cfg = &state.spectrum_config;
        let axes = Axes::new(rect, cfg);
        let min_midi = cfg.low_midi;
        let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
        let scale = PitchScale { min_midi, max_midi, span: max_midi - min_midi };
        let split = super::super::spectral::spectrum_share(cfg);
        plan(state, &axes, &scale, split, now, 1.0)
    }

    fn said(labels: &[NoteLabel]) -> Vec<String> {
        labels.iter().map(|l| l.name.to_string()).collect()
    }

    /// Every note is named, repeats of one pitch included — that is the whole
    /// difference from marking a pitch once and ruling a line forward from it.
    /// A repeat is where you look to ask "what was that again", and it is the
    /// note under your eye, not the one that introduced the pitch ten bars
    /// ago, that has to answer.
    #[test]
    fn every_note_is_named_repeats_included() {
        let mut state = state(24.0, 10.0);
        for i in 0..4 {
            let t = i as f64 * 2.0;
            state.tracker.handle_event(on(t, 60));
            state.tracker.handle_event(off(t + 0.5, 60));
        }
        // Four presses of one pitch, far enough apart in time that no two
        // names collide.
        assert_eq!(said(&labels(&state, 8.0)), ["C", "C", "C", "C"]);
    }

    /// A name sits at the note's ONSET and rides clear of the ribbon rather
    /// than across it: a ribbon is a couple of points thick where a name is
    /// ten, so a name centred on the note would have the note's own colour
    /// running through the middle of the letter.
    #[test]
    fn a_name_sits_at_the_onset_and_clear_of_the_ribbon() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(2.0, 60));
        state.tracker.handle_event(off(9.0, 60));

        let placed = labels(&state, 10.0);
        assert_eq!(placed.len(), 1);
        let axes = Axes::new(PANE, &state.spectrum_config);
        // Depth runs 0 (now) to 1 (a window ago); the onset is 8 of the
        // window's 10 seconds back. Horizontal: depth is the x axis, and pitch
        // climbs with -y.
        let onset = axes.at(0.5, 0.8);
        let rect = placed[0].rect;
        assert!(
            rect.max.x <= onset.x + LABEL_PAD + 0.01,
            "the name lies back along the ribbon from the onset, not past it",
        );
        assert!(rect.max.y < onset.y, "and clears the ribbon rather than crossing it");
    }

    /// A note that began before the window still has a ribbon on the pane, so
    /// it still gets a name — at the visible start of that ribbon. A drone
    /// held since before the Span reaches back is exactly the note whose name
    /// is hardest to recover any other way.
    #[test]
    fn a_note_that_began_before_the_window_is_named_at_the_edge() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(0.0, 67)); // still held
        let placed = labels(&state, 100.0);
        assert_eq!(said(&placed), ["G"]);

        let axes = Axes::new(PANE, &state.spectrum_config);
        let far = axes.at(0.5, 1.0);
        assert!(placed[0].rect.max.x <= far.x + LABEL_PAD + 0.01, "clamped to the far edge");
    }

    /// Notes off the pitch zoom are not named, and the zoom is the ordinary
    /// way to look at a few semitones of a piece that spans four octaves.
    #[test]
    fn notes_outside_the_pitch_zoom_are_left_out() {
        let mut state = state(12.0, 10.0); // 54..66
        for note in [48, 60, 72] {
            state.tracker.handle_event(on(0.0, note));
            state.tracker.handle_event(off(0.5, note));
        }
        assert_eq!(said(&labels(&state, 5.0)), ["C"], "only the one inside 54..66");
    }

    /// Notes that have scrolled off the far end are not named either — their
    /// ribbons are gone, so a name would be labelling nothing.
    #[test]
    fn notes_that_have_left_the_window_are_left_out() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(0.0, 60));
        state.tracker.handle_event(off(0.5, 60));
        state.tracker.handle_event(on(8.0, 67));
        state.tracker.handle_event(off(8.5, 67));

        assert_eq!(said(&labels(&state, 9.0)), ["G", "C"], "newest first, both on the pane");
        // now = 12: the first note ended at 0.5, a window and more ago.
        assert_eq!(said(&labels(&state, 12.0)), ["G"]);
    }

    /// Names that would land on top of each other are dropped rather than
    /// stacked, and the NEWEST note keeps its place — the notes at the
    /// now-line are the ones being played, so a name losing its place to
    /// something older would be the wrong one to drop.
    #[test]
    fn a_crowded_name_gives_way_to_the_newer_note() {
        let mut state = state(24.0, 10.0);
        // Six chromatic neighbours struck together: at 100 points across two
        // octaves they are four points apart, where a name is ten tall.
        for note in 60..66 {
            state.tracker.handle_event(on(5.0, note));
        }
        let placed = labels(&state, 5.5);
        assert!(!placed.is_empty(), "the least crowded still gets its name");
        assert!(placed.len() < 6, "but not all six fit: {:?}", said(&placed));

        // The same six with room for all of them keep all their names.
        assert_eq!(labels_in(&state, 5.5, BIG).len(), 6);
    }

    /// Per-note tuning, which is what this plugin is for: a note is named by
    /// the pitch it SETTLED on, not the key it was pressed at.
    ///
    /// A retuned note is a note-on at its equal-tempered pitch followed by a
    /// tuning expression, so naming the press would put every note in a
    /// just-intoned piece on one of twelve spellings — the exact reading these
    /// names replace, and the ribbon would sit a comma off the name over it.
    #[test]
    fn a_note_is_named_by_the_pitch_its_tuning_lands_it_at() {
        let mut state = state(12.0, 10.0);
        // A JUST tuning, which is the one the distinction lives in: an equal
        // temperament tempers the syntonic comma out by construction, so there
        // is no node a comma below E to name.
        state.tuning = lattice_core::Tuning::just();
        state.tracker.handle_event(on(1.0, 64));
        state.tracker.handle_event(tuning(1.01, 64, -0.137));
        state.tracker.handle_event(off(2.0, 64));

        // The just third is a lattice node, and says so with the comma mark
        // the lattice draws on that node — the whole reason to spell a name
        // the lattice's way rather than as a piano key and a cents offset.
        assert_eq!(said(&labels_in(&state, 5.0, BIG)), ["E-"]);
    }

    /// In an EQUAL temperament the lattice collapses — twelve fifths are seven
    /// octaves exactly — so a dozen visible nodes answer to middle C, all at
    /// distance zero. Something has to choose between them, and the choice has
    /// to be the plain one.
    ///
    /// Taking the first match instead names middle C `F♭5+6`: true, unreadable,
    /// and it changes whenever the view is panned, because "first" means the
    /// corner of the visible window. This is the default tuning the plugin
    /// opens on, so it is the naming most sessions would actually see.
    #[test]
    fn a_collapsed_tuning_names_a_pitch_plainly_rather_than_from_the_corner() {
        let view = lattice_scene::ViewConfig::default();
        let equal = lattice_core::Tuning::default();
        let name = |midi| note_name(&view, &equal, midi).to_string();

        assert_eq!(name(60.0), "C", "the origin, not a remote spelling of it");
        assert_eq!(name(67.0), "G", "a fifth up");
        assert_eq!(name(65.0), "F", "a fifth down");
        // Four fifths up spells E; one just third up spells E-, and in this
        // tuning they are the same pitch. The plain letter wins.
        assert_eq!(name(64.0), "E");
    }

    /// A pitch the visible lattice has no node for still gets a name, spelled
    /// the equal-tempered way — and still as a [`NoteName`], so it draws
    /// through the same code as every other name rather than down a second
    /// path that could look different.
    ///
    /// It is not a corner: the pane already flags notes sounding off the
    /// lattice with a band down the spectrum, so the case is expected, and a
    /// note with no name at all would read as a bug rather than as an answer.
    #[test]
    fn a_pitch_the_lattice_cannot_show_falls_back_to_its_piano_spelling() {
        assert_eq!(equal_tempered_name(60.0).to_string(), "C");
        assert_eq!(equal_tempered_name(66.0).to_string(), "F\u{266F}");
        assert_eq!(equal_tempered_name(69.0).to_string(), "A");
        // Rounded to the nearest key, and carrying no comma mark: the
        // equal-tempered grid has no commas to report.
        assert_eq!(equal_tempered_name(64.004).to_string(), "E");
        assert_eq!(equal_tempered_name(63.9).to_string(), "E");
    }

    /// The setting turns them off outright.
    #[test]
    fn the_setting_turns_them_off() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(0.0, 60));
        assert_eq!(labels(&state, 1.0).len(), 1);

        state.spectrum_config.note_names = false;
        assert!(labels(&state, 1.0).is_empty());
    }

    /// The placement is quadratic in what it accepts, and the Span reaches ten
    /// minutes over a roll that remembers four thousand notes — so what it
    /// accepts is bounded, and bounded at the END that is being read.
    #[test]
    fn a_dense_span_considers_a_bounded_number_of_notes() {
        let mut state = state(24.0, 600.0);
        // Two thousand notes at one pitch, a fifth of a second apart — every
        // one of them inside the window at the moment asked about.
        for i in 0..2000 {
            let t = i as f64 * 0.2;
            state.tracker.handle_event(on(t, 60));
            state.tracker.handle_event(off(t + 0.1, 60));
        }
        let placed = labels(&state, 400.0);
        assert!(placed.len() <= MAX_CANDIDATES, "bounded: {}", placed.len());
        assert!(!placed.is_empty(), "and the newest of them are still named");
    }
}
