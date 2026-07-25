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
//! So the roll says it, since the roll is already drawing the notes: a name
//! written ON each ribbon, at its leading edge. The heatmap band under a
//! ribbon is the same note, so naming the ribbon names the band.
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

/// Points the name is set in from the ribbon's leading edge, along the time
/// axis. Enough that the letter is not touching the end it starts from.
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

/// Clear time a name demands beyond its own box, in points along the time
/// axis, before the next name may take a place.
///
/// Without it, successive names at one pitch are allowed to butt together, and
/// a run of repeats reads as a word rather than as a name on each of several
/// notes. This is the "certain span" the greedy leaves between the instance it
/// picks and the next one it will take.
///
/// Along the TIME axis only. Across pitch, two names may sit as close as the
/// type allows: they are different notes, and the pitch axis is the one thing
/// separating them already.
const REPEAT_GAP: f32 = 6.0;

/// Screen cell for the occupancy grid, in points — comfortably wider than a
/// name, so a candidate can only ever touch the few cells around it.
const CELL: f32 = 24.0;

/// The names placed so far, bucketed on a coarse screen grid.
///
/// The placement is greedy over every note on the pane, and a plain sweep
/// tests each candidate against every name already placed — quadratic, over a
/// set that reaches four thousand notes at a ten-minute Span, and worst at
/// exactly the density where almost every test fails. The grid makes it a
/// handful of comparisons instead: a name is a dozen points across and a cell
/// is wider than that, so a candidate can only touch the cells it overlaps.
#[derive(Default)]
struct Occupancy {
    cells: HashMap<(i32, i32), Vec<egui::Rect>>,
}

impl Occupancy {
    fn cells_of(rect: egui::Rect) -> impl Iterator<Item = (i32, i32)> {
        let cell = |v: f32| (v / CELL).floor() as i32;
        let (x0, x1) = (cell(rect.min.x), cell(rect.max.x));
        let (y0, y1) = (cell(rect.min.y), cell(rect.max.y));
        (x0..=x1).flat_map(move |x| (y0..=y1).map(move |y| (x, y)))
    }

    /// Whether `rect` — a candidate's box already grown by the clear space it
    /// demands — touches nothing placed.
    fn free(&self, rect: egui::Rect) -> bool {
        Self::cells_of(rect).all(|cell| match self.cells.get(&cell) {
            Some(placed) => !placed.iter().any(|other| other.intersects(rect)),
            None => true,
        })
    }

    /// Record a name's own box. The gap it demands is added to whatever is
    /// TESTED against this, not stored here, so the clear space between two
    /// names is one gap rather than two.
    fn insert(&mut self, rect: egui::Rect) {
        for cell in Self::cells_of(rect) {
            self.cells.entry(cell).or_default().push(rect);
        }
    }
}

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
    // Held notes first, then the rest oldest first — and a total order within
    // each, since the offline render must not depend on the order the roll
    // happened to hand them back and a name's place is decided first-come.
    //
    // OLDEST first is what makes the picture hold still while you play. The
    // greedy gives a name to the first instance of a note it reaches and then
    // to the next one with room after it, so the names are decided from the
    // far end of the window inward — and a note arriving at the now-line is
    // last in the order, so it fits in around what is already named instead of
    // evicting it. Newest-first reshuffles the whole pane on every note played,
    // which is precisely when you are trying to read it.
    //
    // Held notes are the exception, and they are placed BEFORE the sweep, not
    // merely early in it: a note you are holding down is the one you are most
    // likely to be asking about, so its name is not the greedy's to refuse.
    notes.sort_unstable_by(|a, b| {
        b.is_live()
            .cmp(&a.is_live())
            .then(a.start.total_cmp(&b.start))
            .then(a.channel.cmp(&b.channel))
            .then(a.note.cmp(&b.note))
    });

    let size = LABEL_PT * label_scale;

    // One naming per pitch CLASS rather than per note. Naming walks every
    // visible lattice node looking for the match that spells best, and a
    // passage is the same handful of classes over and over.
    let mut names: HashMap<i32, NoteName> = HashMap::new();
    let mut occupied = Occupancy::default();
    let mut placed: Vec<NoteLabel> = Vec::new();
    for note in notes {
        let pitch = note.settled_pitch();
        let class = (pitch.rem_euclid(12.0) * 100.0).round() as i32;
        let name = *names
            .entry(class)
            .or_insert_with(|| note_name(&state.view, &state.tuning, pitch));
        let lead = leading_depth(&time, note, now);
        let rect = label_rect(axes, scale.t_of(pitch), lead, &name, size);
        // A held note takes its place whatever is already there; everything
        // else has to find room, including room for the gap it will demand of
        // whoever comes next.
        if !note.is_live() && !occupied.free(keep_out(axes, rect)) {
            continue;
        }
        occupied.insert(rect);
        placed.push(NoteLabel { name, rect });
    }
    placed
}

/// A name's box grown by the clear space it demands of its neighbours: the
/// [`REPEAT_GAP`] along the time axis, and nothing across pitch.
///
/// Directional without naming a screen side — the depth axis is the screen's x
/// on a pane laid out along its long side and its y on an upright one, so the
/// gap is projected onto it rather than added to a named edge.
fn keep_out(axes: &Axes, rect: egui::Rect) -> egui::Rect {
    let depth = axes.dir_depth();
    rect.expand2(egui::vec2(REPEAT_GAP * depth.x.abs(), REPEAT_GAP * depth.y.abs()))
}

/// The depth of a ribbon's LEADING edge — the end of it that comes first in
/// reading order, which is the low-depth end in either layout and in either
/// orientation (the left of a pane laid out along its long side, the top of an
/// upright one).
///
/// Taken as a minimum rather than by naming an end, because which end of a
/// note that is differs between the two layouts and the arithmetic does not:
/// live, time runs from the now-line outward, so a note's leading edge is
/// where it most recently sounded; whole-song lays the take out in reading
/// order, so it is the onset.
///
/// What this buys live is the behaviour a held note ought to have. While the
/// key is down the note keeps reaching the present, so its leading edge IS the
/// now-line: the name sits still there, at the head of a ribbon growing out
/// behind it, for as long as the note is held. The moment it is released that
/// edge is the release, and the name travels away with the note it belongs to.
/// Anchoring on the onset instead would slide the name away from the note the
/// whole time it was being played — which is exactly when you are looking at
/// it.
///
/// Both ends are CLAMPED into the region on the way, so a note that reaches
/// past either edge is named at the last of it still on the pane.
fn leading_depth(time: &TimeAxis, note: &RollNote, now: f64) -> f32 {
    time.depth_of(note.start).min(time.depth_of(note.stop(now)))
}

/// The screen box a name covers on a ribbon at pitch `p` whose leading edge is
/// at depth `d`, padded by the clear space it demands around itself.
///
/// ON the ribbon across the pitch axis — centred on the note's own line, not
/// standing off it. The note is what the name is about, so the name sits on
/// it; the halo every label here carries is what keeps the letter legible
/// against whatever colour the ribbon is (see [`draw`]).
///
/// Along the time axis it grows from the leading edge INTO the note, so a name
/// lies over its own ribbon rather than over the picture in front of it.
fn label_rect(axes: &Axes, p: f32, d: f32, name: &NoteName, size: f32) -> egui::Rect {
    let est = name_extent(name, size);
    let depth = axes.dir_depth();
    // How far the box reaches along the depth axis: text always runs across
    // the screen, so that is its width on a pane laid out along its long side
    // and its height on an upright one. Projecting answers both without naming
    // a screen side.
    let along_depth = (est.x * depth.x).abs() + (est.y * depth.y).abs();
    let centre = axes.at(p, d) + depth * (LABEL_INSET + along_depth * 0.5);
    egui::Rect::from_center_size(centre, est).expand(LABEL_PAD)
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

    /// A name sits ON its ribbon — centred across the note's own line, not
    /// standing off it — and at the ribbon's LEADING edge, growing back into
    /// the note from there.
    #[test]
    fn a_name_sits_on_its_ribbon_at_the_leading_edge() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(2.0, 60));
        state.tracker.handle_event(off(6.0, 60));

        let placed = labels(&state, 10.0);
        assert_eq!(placed.len(), 1);
        let axes = Axes::new(PANE, &state.spectrum_config);
        // Horizontal: depth is the x axis with now at the left, and pitch
        // climbs with -y. The ribbon runs from the release (4s back, depth
        // 0.4) to the onset (8s back, depth 0.8), so its leading edge is the
        // release.
        let rect = placed[0].rect;
        let lead = axes.at(0.5, 0.4);
        assert!(
            (rect.center().y - lead.y).abs() < 1.0,
            "the name is centred on the note's own line, not lifted off it",
        );
        assert!(rect.min.x >= lead.x, "it starts at the leading edge");
        assert!(rect.min.x < lead.x + 2.0 * LABEL_INSET, "...and right at it");
        assert!(rect.max.x < axes.at(0.5, 0.8).x, "growing back into the note, not past it");
    }

    /// A HELD note's name stays put at the now-line, and starts travelling
    /// only once the note is released.
    ///
    /// This is what anchoring on the leading edge rather than the onset buys.
    /// While the key is down the note keeps reaching the present, so its
    /// leading edge IS the now-line and the name sits still at the head of a
    /// ribbon growing out behind it. Anchored on the onset, the name would
    /// slide away down the pane for the whole time the note was sounding —
    /// which is exactly when you are looking at it.
    #[test]
    fn a_held_notes_name_waits_at_the_now_line_and_leaves_when_released() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(1.0, 60));

        let at = |state: &SharedState, now| labels(state, now)[0].rect.min.x;
        let early = at(&state, 2.0);
        let later = at(&state, 4.0);
        assert_eq!(early, later, "held, the name holds its place while the ribbon grows");

        state.tracker.handle_event(off(4.0, 60));
        assert!(at(&state, 5.0) > later, "released, it travels away with the note");
        assert!(at(&state, 7.0) > at(&state, 5.0), "...and keeps travelling");
    }

    /// A note that began before the window still has a ribbon on the pane, so
    /// it still gets a name. A drone held since before the Span reaches back
    /// is exactly the note whose name is hardest to recover any other way —
    /// and being held, its name waits at the now-line.
    #[test]
    fn a_note_that_began_before_the_window_is_still_named() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(0.0, 67)); // still held
        let placed = labels(&state, 100.0);
        assert_eq!(said(&placed), ["G"]);

        let axes = Axes::new(PANE, &state.spectrum_config);
        assert!(placed[0].rect.min.x >= axes.at(0.5, 0.0).x, "at the now-line, held");
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

        assert_eq!(said(&labels(&state, 9.0)), ["C", "G"], "oldest first, both on the pane");
        // now = 12: the first note ended at 0.5, a window and more ago.
        assert_eq!(said(&labels(&state, 12.0)), ["G"]);
    }

    /// Among repeats of one note, the FIRST instance the sweep reaches gets
    /// the name, and the next one only once there is clear room after it.
    ///
    /// The order is what keeps the picture still while you play: names are
    /// decided from the far end of the window inward, so a note arriving at
    /// the now-line fits in around what is already named instead of evicting
    /// it. Deciding newest-first reshuffles the whole pane on every note
    /// played, which is precisely when you are trying to read it.
    #[test]
    fn the_first_instance_takes_the_name_and_the_next_waits_for_room() {
        let mut state = state(24.0, 10.0);
        // A run of one pitch, far too fast for every name to fit: the roll is
        // 300 points wide over 10 seconds, so a tenth of a second is 3 points
        // where a name plus its gap is a dozen.
        for i in 0..40 {
            let t = i as f64 * 0.1;
            state.tracker.handle_event(on(t, 60));
            state.tracker.handle_event(off(t + 0.05, 60));
        }
        let placed = labels(&state, 4.5);
        assert!(placed.len() > 1, "several of them are named");
        assert!(placed.len() < 20, "but nothing like all forty: {}", placed.len());

        // Named from the far (oldest) end inward, and never touching: each
        // name sits clear of the one before it by at least the gap.
        let mut xs: Vec<f32> = placed.iter().map(|l| l.rect.min.x).collect();
        xs.sort_by(f32::total_cmp);
        for pair in xs.windows(2) {
            assert!(pair[1] - pair[0] >= REPEAT_GAP, "names crowd at {pair:?}");
        }
    }

    /// Names that would land on top of each other are dropped rather than
    /// stacked — but the LINES are not what is being thinned, only the names.
    #[test]
    fn names_too_crowded_to_read_are_dropped() {
        let mut state = state(24.0, 10.0);
        // Six chromatic neighbours struck together, and released, so none of
        // them takes the held-note exception: at 100 points across two octaves
        // they are four points apart, where a name is ten tall.
        for note in 60..66 {
            state.tracker.handle_event(on(5.0, note));
            state.tracker.handle_event(off(5.2, note));
        }
        let placed = labels(&state, 5.5);
        assert!(!placed.is_empty(), "the least crowded still gets its name");
        assert!(placed.len() < 6, "but not all six fit: {:?}", said(&placed));

        // The same six with room for all of them keep all their names.
        assert_eq!(labels_in(&state, 5.5, BIG).len(), 6);
    }

    /// A note you are HOLDING is named whatever else is in the way, and keeps
    /// its name until it is released.
    ///
    /// The one exception to the greedy, and the reason there is one: a note
    /// under your finger is the note you are most likely to be asking about,
    /// so whether it is named must not depend on what the rest of the picture
    /// happens to be doing around it.
    #[test]
    fn a_held_note_is_named_however_crowded_it_is() {
        let mut state = state(24.0, 10.0);
        // Two notes a semitone apart and all but on top of each other: at 100
        // points across two octaves they are four points of pitch axis apart,
        // where a name is a dozen tall. Only one of the two can be named.
        //
        // The neighbour is the OLDER, so the plain sweep gives it the name and
        // middle C — struck later, and held — is the one that would lose. (The
        // lattice spells that neighbour D♭, not C♯; the name comes from the
        // node, not from a piano keyboard.)
        state.tracker.handle_event(on(1.0, 61));
        state.tracker.handle_event(off(1.9, 61));
        state.tracker.handle_event(on(1.5, 60));

        assert_eq!(said(&labels(&state, 2.0)), ["C"], "held, it takes the place regardless");

        // ...and keeps it for as long as it is held, however the picture
        // around it moves.
        assert!(said(&labels(&state, 3.0)).contains(&"C".to_owned()));

        // Released at the same instant it is asked about, so it stands in
        // exactly the place it did while held: now it takes its chances with
        // the sweep like everything else, and the older note wins.
        state.tracker.handle_event(off(2.0, 60));
        assert_eq!(said(&labels(&state, 2.0)), ["D\u{266D}"], "released, it is one of the crowd");
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

    /// A wall of notes is thinned by the pane's own geometry, not by a cap on
    /// how many are looked at — so the names that survive are spread across
    /// the whole window rather than bunched at whichever end a cap kept.
    ///
    /// This is what the occupancy grid buys. The obvious bound — consider only
    /// the newest N — is the wrong shape for a greedy that names from the far
    /// end inward: it would leave the older half of the pane bare however much
    /// room was going spare there.
    #[test]
    fn a_wall_of_notes_is_thinned_by_the_room_there_is_for_names() {
        let mut state = state(24.0, 600.0);
        // Three thousand notes at one pitch, a fifth of a second apart, filling
        // the whole ten-minute window at the moment asked about.
        for i in 0..3000 {
            let t = i as f64 * 0.2;
            state.tracker.handle_event(on(t, 60));
            state.tracker.handle_event(off(t + 0.1, 60));
        }
        let placed = labels(&state, 600.0);
        assert!(!placed.is_empty());
        // The roll is 300 points wide and a name plus its gap is a dozen or so,
        // so what fits is tens, not thousands.
        assert!(placed.len() < 40, "thinned to what fits: {}", placed.len());

        // Spread across the window, not gathered at one end: the oldest and
        // newest names are nearly the whole pane apart.
        let xs: Vec<f32> = placed.iter().map(|l| l.rect.center().x).collect();
        let lo = xs.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = xs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(hi - lo > 250.0, "names span the window: {lo}..{hi}");
    }
}
