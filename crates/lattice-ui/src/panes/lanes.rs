//! Pitch lanes: a hairline at every pitch that has been played, running from
//! where that pitch first sounded to the present, with the pitch's name at the
//! end it starts from.
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
//! So it is said with position and text, and only for the pitches actually in
//! play. That is what makes it fit: a piece uses a few dozen of the axis's
//! hundred-odd semitones, so a mark at each of those is a legend rather than a
//! grid. And it is why the name goes where the pitch FIRST sounded, with the
//! line running forward from there — music repeats its pitches far more often
//! than it introduces new ones, so nearly every note lands on a line that was
//! named long before it arrived, and the naming happens at the rate new
//! material does.
//!
//! The name is the LATTICE's, in the lattice's own hand: the same
//! [`NoteName`] the node carries, drawn by the same
//! [`draw_stacked_name`](super::lattice::draw_stacked_name) — letter at full
//! size, accidental riding high, syntonic-comma mark low. Not a resemblance
//! but the same function, so the two cannot drift apart. That is the errand:
//! a lane is read against the lattice, so it has to answer in the lattice's
//! vocabulary. A cents deviation off the nearest piano key would say where a
//! pitch sits between two keys when what is wanted is which node it IS — and
//! for a just third, `E-` says it where "E +14\u{a2}" does not.
//!
//! Geometry comes from [`Axes`](super::spectral::Axes) like everything else in
//! the pane, so lanes turn and flip with it and nothing here names a screen
//! side.

use std::collections::HashMap;

use lattice_core::{LatticePos, NoteName, NoteRoll, PitchClass, Tuning};
use lattice_scene::ViewConfig;

use super::lattice;
use super::spectral::{Axes, PitchScale, TimeAxis};
use crate::{theme, PitchLanes, SharedState};

/// Pitches within this many cents of each other are one lane.
///
/// Not zero. A pitch arrives as a key number plus a tuning offset in f32, so
/// two presses of one key agree bit for bit only while the tuning behind them
/// is untouched — re-learning it, or an MPE bend that lands a hair off where
/// the last one did, would otherwise open a second lane a thousandth of a
/// semitone from the first. A cent is under the resolution of the analysis
/// behind the heatmap and far under a pixel at any zoom the pane offers, so
/// nothing that could be told apart on screen is merged by it.
const LANE_CENTS: f32 = 1.0;

/// Lanes closer together than this on screen, in points, draw as one.
///
/// Two hairlines a pixel apart are one hairline with a wrong count behind it.
/// The bound this puts on the drawn count is the point: [`LANE_CENTS`] leaves
/// a heavily retuned piece free to open a lane every cent, and at a wide pitch
/// zoom that is thousands of segments for a picture of a solid wash. Merged,
/// the count can never exceed the pitch axis in points.
const LANE_MERGE_PX: f32 = 1.5;

/// The lane line's width in points, and how bright it is over the heatmap.
///
/// White at low alpha rather than a color from the skin: it is drawn over the
/// spectrogram, whose palettes run from black to near-white, and the one thing
/// every one of them has in common is a dark end. Faint on purpose — the lane
/// is there to carry a pitch level across the QUIET stretches between one
/// instance of a note and the next, and a loud stretch already says where the
/// pitch is far more loudly than a line could.
const LANE_WIDTH: f32 = 1.0;
const LANE_ALPHA: f32 = 0.22;

/// Point size of a lane's name. Half a point under the axis labels': there are
/// many more of these, and they sit inside the picture rather than along its
/// edge.
pub(super) const LABEL_PT: f32 = 9.5;

/// Points the name is lifted off its own line, along the pitch axis, and set
/// in from the lane's start along the time axis.
///
/// The lift clears the hairline so the two do not touch; the inset puts the
/// text on the side its lane runs toward, so a name always sits over the lane
/// it names rather than over the empty axis before it.
const LABEL_LIFT: f32 = 2.5;
const LABEL_INSET: f32 = 3.0;

/// What a monospace glyph advances, and a line box stands, as fractions of the
/// font size — and the clear space a name demands around itself, in points.
///
/// An ESTIMATE, deliberately, rather than a galley measured through egui. It
/// decides only which names are dropped for colliding, so being a few percent
/// wide costs a name that would have just fitted and nothing else; against
/// that, measuring would put a text layout per lane per frame in front of a
/// decision that is thrown away for most of them, and would make the offline
/// render's output depend on font metrics rather than on arithmetic.
const GLYPH_ADVANCE: f32 = 0.62;
const LINE_HEIGHT: f32 = 1.3;
const LABEL_PAD: f32 = 1.5;

/// How many places along its own lane a name may be tried before it gives up.
///
/// A name wants to sit at its lane's start, and the starts crowd — every lane
/// whose first press has scrolled out lies against the region's far edge, so a
/// piece that has been playing a while puts most of its names at one depth
/// separated only by pitch. At a wide pitch zoom that is more of them than fit,
/// and a first-come rule would simply drop the rest.
///
/// Sliding the ones that collide a little way along their own lanes turns that
/// pile into a staggered legend. What it costs is that a name is not always
/// exactly on the moment its line begins — which the line's own end still
/// says, and which is meaningless anyway for the clamped ones, since their
/// real start is off the pane.
const STAGGER_STEPS: usize = 4;

/// One pitch that has been played: where it sits on the axis, and when it
/// first sounded.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lane {
    /// The pitch in MIDI note units — the units the axis is scaled in.
    ///
    /// The pitch as PLAYED, not the [`LANE_CENTS`] bucket that grouped it.
    /// The bucket is a grouping key and nothing else: rounding a pitch to the
    /// cent before naming it would spend up to half a cent of the tuning's
    /// own half-cent match tolerance, so a just third could quantize itself
    /// clear of the node it is, and the lane would fall back to naming a
    /// piano key.
    pub midi: f32,
    /// Take time of the earliest press at this pitch the roll still holds.
    pub first: f64,
}

/// The lane set derived from a roll, kept across frames.
///
/// Deriving it is a scan of every remembered note — up to
/// [`NoteRoll::MAX_NOTES`] of them — for the earliest onset at each pitch, and
/// the answer moves only when a note joins or leaves, which is exactly what
/// [`NoteRoll::revision`] reports. Uncached this would be a second full pass
/// over the roll on every frame, next to the one
/// [`note_instances`](super::roll::note_instances) already makes; cached, a
/// frame that plays no new note pays a `u64` comparison.
#[derive(Default)]
pub(crate) struct LaneCache {
    /// The roll these came from: its revision, and whether it was the offline
    /// whole-song roll. That flag is not decoration — the two rolls count
    /// revisions of their own from zero, so without it a render switching to
    /// the whole-song layout would find a matching number against a completely
    /// different set of notes.
    key: Option<(u64, bool)>,
    lanes: Vec<Lane>,
}

impl LaneCache {
    pub(crate) fn lanes(&self) -> &[Lane] {
        &self.lanes
    }
}

/// Re-derive the lane set if the roll has changed under it.
fn refresh(state: &mut SharedState) {
    // Whole-song (offline playhead) reads the take's own roll, built up front;
    // live reads the causal tracker's. Disjoint fields of `state`, so the read
    // and the cache write below do not fight.
    let (roll, whole) = match state.whole_song.as_ref() {
        Some(ws) => (&ws.roll, true),
        None => (state.tracker.roll(), false),
    };
    let key = Some((roll.revision(), whole));
    if state.lanes.key == key {
        return;
    }
    state.lanes.key = key;
    derive(roll, &mut state.lanes.lanes);
}

/// Every pitch in `roll`, with the earliest press at it, in pitch order.
fn derive(roll: &NoteRoll, lanes: &mut Vec<Lane>) {
    // Keyed by the cent bucket, holding the earliest press in it and the pitch
    // that press actually sounded at — see [`Lane::midi`].
    let mut first: HashMap<i32, (f64, f32)> = HashMap::new();
    for note in roll.notes() {
        // The pitch it SETTLED on, not the key it was pressed at: a retuned
        // note reaches its actual pitch through an expression just after its
        // note-on, so keying on the press would put every lane in a
        // just-intoned piece on one of twelve equal-tempered heights — which
        // is the reading this whole file exists to replace. See
        // [`RollNote::settled_pitch`].
        let pitch = note.settled_pitch();
        let key = (pitch / LANE_CENTS * 100.0).round() as i32;
        let at = first.entry(key).or_insert((note.start, pitch));
        if note.start < at.0 {
            *at = (note.start, pitch);
        }
    }
    lanes.clear();
    lanes.extend(first.into_values().map(|(first, midi)| Lane { midi, first }));
    // Pitch order, not the map's. A HashMap iterates differently between runs,
    // and the labels below are placed first-come — so an unsorted set would
    // hand a different picture to two renders of one take, which the offline
    // render's determinism test forbids.
    lanes.sort_unstable_by(|a, b| a.midi.total_cmp(&b.midi));
}

/// One lane placed on the pane: where its line runs, and its name if it won
/// one.
#[derive(Clone, Debug)]
pub(super) struct LaneDraw {
    pub lane: Lane,
    /// Pitch fraction the line sits at.
    pub p: f32,
    /// Depth of the end the lane STARTS from: its first onset, or the region's
    /// far edge once that has scrolled out past it.
    pub d_first: f32,
    /// Depth of the other end — the now-line live, the take's end whole-song.
    pub d_now: f32,
    /// Points the name was slid along the lane to find room — see
    /// [`STAGGER_STEPS`]. Zero for a name that fitted where it wanted to.
    pub slide: f32,
    /// The lattice's spelling of this pitch, once it has won room for it.
    pub name: Option<NoteName>,
}

/// A lane's name: the LATTICE's own spelling of that pitch — the same
/// [`NoteName`] the node carries, drawn by the same code (see
/// [`push_labels`]), so a lane and the node it points at are one label in two
/// places rather than two descriptions of one thing.
///
/// That is the whole errand. A lane exists to be read against the lattice, so
/// a naming of its own — a cents deviation off the nearest piano key, say —
/// would answer a different question in the same typeface: it would say where
/// the pitch sits between two keys, when what is wanted is which node it IS.
/// The lattice's spelling carries that in the accidental and the comma mark:
/// a just third is `E-`, and no amount of "E +14\u{a2}" says so.
///
/// No octave, for the same reason. A lattice node is a pitch class and wears
/// no octave number, so a lane matching one must not invent it; the lanes for
/// two octaves of a pitch are far apart on the axis and read as what they are.
///
/// The fallback, for a pitch the visible lattice has no node for, is the
/// equal-tempered spelling — still a [`NoteName`], so it draws identically and
/// there is one rendering path rather than two. It is a real case: the pane
/// already flags notes sounding off the lattice with a band down the spectrum,
/// and a lane with no name at all would just look like a bug.
fn lane_name(view: &ViewConfig, tuning: &Tuning, midi: f32) -> NoteName {
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

/// The nearest piano key, spelled with sharps — [`KEY_NAMES`] as a
/// [`NoteName`] rather than as text, so it reaches the same drawing code.
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
    egui::vec2(
        (size + marks as f32 * mark_size) * GLYPH_ADVANCE,
        size * LINE_HEIGHT,
    )
}

/// Where a lane's name is anchored at a given `slide` down the lane, and which
/// way it grows from there.
fn anchor_at(
    axes: &Axes,
    draw: &LaneDraw,
    size: f32,
    slide: f32,
) -> (egui::Pos2, egui::Align2) {
    // Set in toward the end the lane runs to, so the text lies along its own
    // lane instead of off the start of it — and scaled with the type, so it
    // keeps clearing the line at a larger label size.
    let inset = LABEL_INSET * size / LABEL_PT + slide;
    let into = if draw.d_now > draw.d_first { inset } else { -inset };
    axes.text_anchor(draw.p, draw.d_first, LABEL_LIFT * size / LABEL_PT, into)
}

/// What a name covers on screen, as (its box padded by the clear space it
/// demands, how far along the DEPTH axis that box reaches).
///
/// The second is what a stagger step has to clear, and it is not one of the
/// box's sides in particular: text always runs across the screen, so on a
/// pane laid out along its long side the depth axis is the text's WIDTH and on
/// an upright one it is the text's height. Projecting onto the depth direction
/// answers both without naming a screen side, which is the rule the whole pane
/// is written to.
pub(super) fn label_box(
    axes: &Axes,
    draw: &LaneDraw,
    name: &NoteName,
    size: f32,
    slide: f32,
) -> (egui::Rect, f32) {
    let (pos, align) = anchor_at(axes, draw, size, slide);
    let est = name_extent(name, size);
    let depth = axes.dir_depth();
    let along_depth = (est.x * depth.x).abs() + (est.y * depth.y).abs();
    (align.anchor_size(pos, est).expand(LABEL_PAD), along_depth + 2.0 * LABEL_PAD)
}

/// Place every lane the pane can show: which lines are drawn, and which of
/// them get a name.
///
/// Pure, so the placement can be read back without a GPU or a frame — which is
/// most of what there is to get wrong here, since the two rules that matter
/// (a lane's start clamps rather than re-anchoring, and a name gives way to
/// the one already there) are both invisible in a still picture.
#[allow(clippy::too_many_arguments)]
pub(super) fn lane_draws(
    lanes: &[Lane],
    axes: &Axes,
    scale: &PitchScale,
    time: &TimeAxis,
    split: f32,
    // `None` to draw the lines bare; the lattice to name them against
    // otherwise — see [`lane_name`].
    naming: Option<(&ViewConfig, &Tuning)>,
    size: f32,
) -> Vec<LaneDraw> {
    // Live, time runs from the now-line at `split` out to the past; whole-song
    // lays the whole take out from the near edge, so "later" is the far one.
    let d_now = if time.whole_song() { 1.0 } else { split };
    let pitch_len = axes.pitch_len().max(1.0);

    let mut draws: Vec<LaneDraw> = Vec::new();
    for lane in lanes.iter().filter(|lane| scale.contains(lane.midi)) {
        let p = scale.t_of(lane.midi);
        // Merge what would draw on top of itself, keeping the EARLIER onset of
        // the pair: the two are within a point of each other, so either name
        // is right to a pixel, but a lane that started earlier reaches further
        // back and dropping that would shorten the line for no reason.
        if let Some(last) = draws.last_mut() {
            if (p - last.p) * pitch_len < LANE_MERGE_PX {
                last.lane.first = last.lane.first.min(lane.first);
                last.d_first = time.depth_of(last.lane.first);
                continue;
            }
        }
        draws.push(LaneDraw {
            lane: *lane,
            p,
            // CLAMPED into the region, which is what keeps the start still.
            // The alternative — re-anchoring to the oldest instance still on
            // screen — jumps the whole lane forward the moment its first press
            // scrolls off, by however long the gap to the next press was. The
            // roll remembers past the longest Span the pane offers, so a lane
            // whose start has left simply lies against the far edge.
            d_first: time.depth_of(lane.first),
            d_now,
            slide: 0.0,
            name: None,
        });
    }

    if let Some((view, tuning)) = naming {
        // Oldest onset first, so an established name holds its place and a
        // newly-introduced pitch is the one that gives way where the two would
        // collide. The opposite reads as flicker: every new note in a dense
        // passage would evict a name that had been sitting there.
        //
        // Stable, over an input already in pitch order, so two lanes
        // introduced in the same instant resolve by pitch rather than by
        // whatever the map handed back.
        let mut order: Vec<usize> = (0..draws.len()).collect();
        order.sort_by(|&a, &b| draws[a].lane.first.total_cmp(&draws[b].lane.first));
        let mut placed: Vec<egui::Rect> = Vec::new();
        let depth_len = axes.depth_len();
        // One naming per pitch CLASS rather than per lane. Naming walks every
        // visible lattice node looking for the match that spells best, and a
        // piece's lanes are the same handful of classes spread over several
        // octaves — so this is the difference between a walk per lane and a
        // walk per distinct note of the music.
        let mut names: HashMap<i32, NoteName> = HashMap::new();
        for i in order {
            let midi = draws[i].lane.midi;
            let class = (midi.rem_euclid(12.0) * 100.0).round() as i32;
            let name = *names.entry(class).or_insert_with(|| lane_name(view, tuning, midi));
            // A step is the name's own reach along the lane, so consecutive
            // places butt together rather than overlap.
            let (_, step) = label_box(axes, &draws[i], &name, size, 0.0);
            // How much lane there is to slide along. A name past its own
            // line's end would be pointing at nothing, so a lane with no
            // length yet — a pitch introduced this instant — gets its one try
            // where it starts and no more.
            let room = (draws[i].d_now - draws[i].d_first).abs() * depth_len;
            let spot = (0..STAGGER_STEPS)
                .map(|k| k as f32 * step)
                .take_while(|&slide| slide <= room)
                .map(|slide| (slide, label_box(axes, &draws[i], &name, size, slide).0))
                .find(|(_, rect)| !placed.iter().any(|r| r.intersects(*rect)));
            if let Some((slide, rect)) = spot {
                placed.push(rect);
                draws[i].slide = slide;
                draws[i].name = Some(name);
            }
        }
    }
    draws
}

/// Every lane this frame draws, placed — empty when the setting is off or the
/// pane has kept no far region to draw them in.
///
/// The one entry point, so the cache is refreshed exactly where the lanes are
/// wanted: a pane with the setting off, or with the roll and heatmap both
/// hidden, never derives a lane set at all.
pub(super) fn plan(
    state: &mut SharedState,
    axes: &Axes,
    scale: &PitchScale,
    split: f32,
    now: f64,
    label_scale: f32,
) -> Vec<LaneDraw> {
    let mode = state.spectrum_config.pitch_lanes;
    if mode == PitchLanes::Off || split >= 1.0 {
        return Vec::new();
    }
    refresh(state);
    let time = TimeAxis::new(state, split, now);
    let naming = (mode == PitchLanes::Named).then_some((&state.view, &state.tuning));
    lane_draws(state.lanes.lanes(), axes, scale, &time, split, naming, LABEL_PT * label_scale)
}

/// The hairlines. Drawn over the heatmap they are there to let you read, and
/// under the note ribbons — a line crossing a ribbon would cut the one shape
/// on the pane whose color carries meaning.
pub(super) fn draw_lines(painter: &egui::Painter, axes: &Axes, draws: &[LaneDraw]) {
    let stroke =
        egui::Stroke::new(LANE_WIDTH, egui::Color32::WHITE.gamma_multiply(LANE_ALPHA));
    for draw in draws {
        painter.line_segment([axes.at(draw.p, draw.d_first), axes.at(draw.p, draw.d_now)], stroke);
    }
}

/// The names, into whichever batch the pane is drawing its labels from.
///
/// Drawn by [`lattice::draw_stacked_name`] — the lattice's own label code, not
/// a copy of it — so a lane's name is the same glyphs in the same arrangement
/// as the node it points at: the letter at full size with its accidental
/// riding high and its comma mark low, counted rather than repeated. Sharing
/// the function rather than the look is what keeps them from drifting apart
/// the next time either is touched, and it is why a name is carried this far
/// as a [`NoteName`] rather than as a string.
///
/// Haloed like the axis labels and for the same reason: what is behind them is
/// a picture, not a background, and a name over a bright heatmap slab has no
/// contrast of its own to rely on.
pub(super) fn push_labels(
    painter: &egui::Painter,
    axes: &Axes,
    draws: &[LaneDraw],
    label_scale: f32,
    batch: &mut crate::text::TextBatch,
) {
    let size = LABEL_PT * label_scale;
    // `draw_stacked_name` sizes everything off the lattice's own letter size,
    // so ask it for the lane's smaller one as a fraction of that.
    let scale = size / lattice::NAME_SIZE;
    for draw in draws {
        let Some(name) = draw.name else { continue };
        // It centers a name on its anchor, where the placement above works in
        // boxes that grow away from theirs — so hand it the box's middle, and
        // the name lands exactly where it was measured to fit.
        let (rect, _) = label_box(axes, draw, &name, size, draw.slide);
        lattice::draw_stacked_name(
            batch,
            painter,
            rect.center(),
            name,
            theme::text(),
            theme::well(),
            scale,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SharedState, SpectralOrientation, SpectrumConfig};
    use lattice_core::{NoteEvent, NoteEventKind};

    /// 300 points along the time axis, 100 across pitch — the same pane the
    /// roll's tests use. Small enough that names crowd each other at any zoom
    /// the range control allows, which is what makes it the pane to test
    /// crowding on.
    const PANE: egui::Rect =
        egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 120.0) };

    /// A pitch axis the size a docked pane actually has: 700 points.
    ///
    /// The pitch range cannot be zoomed under an octave
    /// ([`PITCH_RANGE_MIN_SPAN`](crate::PITCH_RANGE_MIN_SPAN)), so across 100
    /// points a semitone is eight of them and a comma is one — under the
    /// merge, and under a line's own width. Anything about telling NEAR
    /// pitches apart therefore has to be asked of a pane with room to draw
    /// them apart, or it is asking about the test fixture.
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

    /// The lanes `state` would draw at `now`, derived and placed exactly the
    /// way [`spectral_pane`](super::super::spectral::spectral_pane) does it.
    fn draws(state: &mut SharedState, now: f64) -> Vec<LaneDraw> {
        draws_in(state, now, PANE)
    }

    fn draws_in(state: &mut SharedState, now: f64, rect: egui::Rect) -> Vec<LaneDraw> {
        let cfg = &state.spectrum_config;
        let axes = Axes::new(rect, cfg);
        let min_midi = cfg.low_midi;
        let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
        let scale = PitchScale { min_midi, max_midi, span: max_midi - min_midi };
        let split = super::super::spectral::spectrum_share(cfg);
        plan(state, &axes, &scale, split, now, 1.0)
    }

    fn named(draws: &[LaneDraw]) -> Vec<String> {
        draws.iter().filter_map(|d| d.name.map(|n| n.to_string())).collect()
    }

    /// The premise the whole feature rests on: a pitch is named once, however
    /// often it is played. A lane per PRESS would be a label on every note,
    /// which is the picture this exists to avoid.
    #[test]
    fn a_pitch_gets_one_lane_however_often_it_is_played() {
        let mut state = state(24.0, 10.0);
        for i in 0..8 {
            let t = i as f64;
            state.tracker.handle_event(on(t, 60));
            state.tracker.handle_event(off(t + 0.5, 60));
        }
        state.tracker.handle_event(on(3.0, 67));
        state.tracker.handle_event(off(3.5, 67));

        // Named the way the lattice names its nodes: a pitch class, with no
        // octave number, because a lattice node does not wear one either.
        let draws = draws(&mut state, 8.0);
        assert_eq!(draws.len(), 2, "two pitches, eight presses");
        assert_eq!(named(&draws), ["C", "G"]);
    }

    /// A lane starts where its pitch FIRST sounded, so the name sits at the
    /// moment the material was introduced and every later press lands on a
    /// line that is already named.
    #[test]
    fn a_lane_starts_where_its_pitch_was_first_played() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(2.0, 60)); // 8s ago at now = 10
        state.tracker.handle_event(off(3.0, 60));
        state.tracker.handle_event(on(9.0, 60)); // ...and again, 1s ago

        let draws = draws(&mut state, 10.0);
        assert_eq!(draws.len(), 1);
        // Depth runs 0 (now) to 1 (a window ago); the first press is 8 of the
        // window's 10 seconds back.
        assert!(
            (draws[0].d_first - 0.8).abs() < 1e-4,
            "the lane reaches back to the FIRST press, not the latest: {}",
            draws[0].d_first,
        );
        assert_eq!(draws[0].d_now, 0.0, "and runs forward to the now-line");
    }

    /// Once the first press has scrolled out, the lane lies against the far
    /// edge and STAYS there.
    ///
    /// The rule this pins is that the start clamps rather than re-anchoring to
    /// the oldest press still on screen. Re-anchoring looks identical in a
    /// still frame and is wrong in motion: the instant the first press left,
    /// the whole lane and its name would jump forward by however long the gap
    /// to the next press was — here, a third of the pane, out of nowhere, on
    /// one frame.
    #[test]
    fn a_lane_whose_start_has_scrolled_out_lies_against_the_far_edge() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(0.0, 60));
        state.tracker.handle_event(off(0.5, 60));
        state.tracker.handle_event(on(7.0, 60));
        state.tracker.handle_event(off(7.5, 60));

        // now = 9.9: the first press is 9.9s into a 10s window, still on.
        let before = draws(&mut state, 9.9);
        assert!((before[0].d_first - 0.99).abs() < 1e-4, "{}", before[0].d_first);
        // now = 10.1, and it has left. The next press is at 7.0 — depth 0.31 —
        // and the lane must NOT snap there.
        let after = draws(&mut state, 10.1);
        assert_eq!(after[0].d_first, 1.0, "the start clamps to the far edge");
        let later = draws(&mut state, 30.0);
        assert_eq!(later[0].d_first, 1.0, "and stays there however far it scrolls");
    }

    /// Per-note tuning, which is what this plugin is for: two instances of one
    /// key a comma apart are two lanes with two names, not one name written
    /// twice — which would say the exact opposite of what a lane is for.
    ///
    /// It also pins where the pitch is read from. A retuned note is a note-on
    /// at the equal-tempered pitch followed by a tuning expression, so keying
    /// on the press would collapse both of these onto one "E4" lane and leave
    /// the ribbons sitting a comma off the line that claims to name them.
    #[test]
    fn one_key_retuned_two_ways_is_two_lanes_with_two_names() {
        // A full-size pitch axis at the closest zoom the range control allows,
        // which is what it takes for a comma to be several points wide; drawn
        // smaller the two merge, which is the rule below.
        let mut state = state(12.0, 10.0);
        // ...and a JUST tuning, which is the one the distinction lives in. An
        // equal temperament tempers the syntonic comma out by construction, so
        // there is no node a comma below E for a lane to be named after and
        // both would read "E" — correctly, since in that tuning they ARE one
        // note.
        state.tuning = lattice_core::Tuning::just();
        state.tracker.handle_event(on(0.0, 64));
        state.tracker.handle_event(off(0.5, 64));
        // The same key again, retuned down a syntonic comma (~13.7 cents under
        // the equal-tempered third, which lands it on the just one) a block
        // after its note-on.
        state.tracker.handle_event(on(1.0, 64));
        state.tracker.handle_event(tuning(1.01, 64, -0.137));

        let draws = draws_in(&mut state, 5.0, BIG);
        assert_eq!(draws.len(), 2, "a comma is two pitches");
        let names = named(&draws);
        // The just third is a lattice node, and says so with the comma mark
        // the lattice draws on that node — the whole reason to spell a lane
        // the lattice's way rather than as a piano key and a cents offset.
        assert_eq!(names[0], "E-");
        // The equal-tempered third is a different pitch, so a different name.
        // WHICH name is not worth pinning: at a half-cent tolerance it lands
        // on whatever remote node happens to coincide with it — a real node,
        // really on screen, but a schisma coincidence, and asserting the
        // spelling of one would be testing the tuning's arithmetic.
        assert_ne!(names[1], names[0]);
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
        let name = |midi| lane_name(&view, &equal, midi).to_string();

        assert_eq!(name(60.0), "C", "the origin, not a remote spelling of it");
        assert_eq!(name(67.0), "G", "a fifth up");
        assert_eq!(name(65.0), "F", "a fifth down");
        // Four fifths up spells E; one just third up spells E-, and in this
        // tuning they are the same pitch. The plain letter wins.
        assert_eq!(name(64.0), "E");
    }

    /// Float noise in a pitch must not open a second lane a thousandth of a
    /// semitone from the first — that is one line carrying two names, and it
    /// would happen every time a tuning was re-learned.
    ///
    /// Asserted on the DERIVED set rather than on what is drawn, because the
    /// drawn set merges by screen distance as well and would pass this on the
    /// strength of the wrong rule.
    #[test]
    fn pitches_inside_a_cent_of_each_other_are_one_lane() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(0.0, 60));
        state.tracker.handle_event(off(0.5, 60));
        state.tracker.handle_event(on(1.0, 60));
        state.tracker.handle_event(tuning(1.0, 60, 0.002));

        refresh(&mut state);
        assert_eq!(state.lanes.lanes().len(), 1);
    }

    /// Two lanes the pane cannot draw apart are drawn as one, keeping the
    /// earlier onset of the pair.
    ///
    /// What this bounds is the count: a cent is the resolution the set is
    /// derived at, so a piece that retunes freely can open lanes a cent apart
    /// all the way up the axis — thousands of segments, for a picture of a
    /// solid wash. Merged, the drawn count can never exceed the pitch axis in
    /// points.
    #[test]
    fn lanes_the_pane_cannot_draw_apart_are_drawn_as_one() {
        // Four octaves across 100 points: a comma is a third of a point.
        let mut state = state(48.0, 10.0);
        state.tracker.handle_event(on(0.0, 64));
        state.tracker.handle_event(off(0.5, 64));
        state.tracker.handle_event(on(2.0, 64));
        state.tracker.handle_event(tuning(2.01, 64, -0.137));

        refresh(&mut state);
        assert_eq!(state.lanes.lanes().len(), 2, "two pitches were played");
        let draws = draws(&mut state, 5.0);
        assert_eq!(draws.len(), 1, "...on one line");
        assert_eq!(draws[0].lane.first, 0.0, "which starts at the earlier of the two");
    }

    /// Zoomed out far enough, neighbouring names would sit on top of each
    /// other — so the crowded ones are dropped rather than stacked. The LINES
    /// stay: a lane you cannot name is still a level you can trace.
    #[test]
    fn names_that_would_overlap_are_dropped_and_their_lines_kept() {
        // Six chromatic neighbours across a five-octave range: 100 points of
        // pitch axis, so they land about a point and a half apart.
        let mut far = state(60.0, 10.0);
        for (i, note) in (60..66).enumerate() {
            far.tracker.handle_event(on(i as f64 * 0.1, note));
            far.tracker.handle_event(off(i as f64 * 0.1 + 0.05, note));
        }
        let wide = draws(&mut far, 5.0);
        assert_eq!(wide.len(), 6, "every lane is drawn");
        let names = named(&wide);
        assert!(!names.is_empty(), "the least crowded one still gets its name");
        assert!(names.len() < 6, "but not all six fit: {names:?}");
        // Oldest first, so the pitch introduced first is the one that keeps
        // its name.
        assert_eq!(names[0], "C");

        // The same six notes with room for all of them keep all their names.
        let mut close = state(12.0, 10.0);
        for (i, note) in (60..66).enumerate() {
            close.tracker.handle_event(on(i as f64 * 0.1, note));
            close.tracker.handle_event(off(i as f64 * 0.1 + 0.05, note));
        }
        assert_eq!(named(&draws_in(&mut close, 5.0, BIG)).len(), 6);
    }

    /// Names that collide slide along their own lanes before any of them is
    /// dropped.
    ///
    /// This is the case that decides whether the feature holds up over a piece
    /// rather than over a bar. Every lane whose first press has scrolled out
    /// lies against the far edge, so after a few minutes ALL of the names want
    /// one depth and only pitch keeps them apart — and at a wide pitch zoom
    /// that is fewer than fit. Dropping them there would leave the legend
    /// emptying out exactly as the piece accumulated the material it is for.
    #[test]
    fn a_crowded_name_slides_along_its_lane_before_any_is_dropped() {
        // Six chromatic neighbours over five octaves: closer together than a
        // name is tall, so at one depth they cannot all fit.
        let mut state = state(60.0, 10.0);
        for (i, note) in (60..66).enumerate() {
            state.tracker.handle_event(on(i as f64 * 0.01, note));
            state.tracker.handle_event(off(i as f64 * 0.01 + 0.005, note));
        }
        // Long enough after that every lane has clamped to the far edge.
        let draws = draws_in(&mut state, 60.0, BIG);
        assert!(draws.iter().all(|d| d.d_first == 1.0), "every start has scrolled out");

        let slid: Vec<f32> =
            draws.iter().filter_map(|d| d.name.is_some().then_some(d.slide)).collect();
        assert_eq!(slid.len(), 6, "all six are named: {slid:?}");
        assert!(slid.iter().any(|&s| s > 0.0), "by some of them moving down their lane");
    }

    /// Lanes off the pitch zoom are not drawn, and the zoom is the ordinary
    /// way to look at a few semitones of a piece that spans four octaves.
    #[test]
    fn lanes_outside_the_pitch_zoom_are_left_out() {
        let mut state = state(12.0, 10.0); // 54..66
        for note in [48, 60, 72] {
            state.tracker.handle_event(on(0.0, note));
            state.tracker.handle_event(off(0.5, note));
        }
        let draws = draws(&mut state, 5.0);
        assert_eq!(named(&draws), ["C"], "only the one inside 54..66");
    }

    /// The setting is three positions, and the middle one is the whole reason
    /// it is not a checkbox: once you know which lanes are which, the lines
    /// alone are the reading aid and the names are clutter.
    #[test]
    fn the_setting_turns_off_the_names_before_it_turns_off_the_lines() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(0.0, 60));
        state.tracker.handle_event(off(0.5, 60));

        assert_eq!(named(&draws(&mut state, 5.0)).len(), 1, "Named draws both");

        state.spectrum_config.pitch_lanes = PitchLanes::Lines;
        let lines = draws(&mut state, 5.0);
        assert_eq!(lines.len(), 1, "the line is still there");
        assert!(lines[0].name.is_none(), "with no name on it");

        state.spectrum_config.pitch_lanes = PitchLanes::Off;
        assert!(draws(&mut state, 5.0).is_empty());
    }

    /// The cache is the only thing keeping this off the per-frame budget, and
    /// a cache that never hits is invisible — the picture is identical and
    /// only the frame time says otherwise.
    #[test]
    fn the_lane_set_is_re_derived_only_when_the_roll_changes() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(0.0, 60));
        refresh(&mut state);
        let key = state.lanes.key;
        assert_eq!(state.lanes.lanes().len(), 1);

        // A frame with nothing played: same roll, same answer, no scan.
        state.tracker.prune(1.0, 1.0);
        refresh(&mut state);
        assert_eq!(state.lanes.key, key, "an untouched roll must not re-derive");

        state.tracker.handle_event(on(1.0, 67));
        refresh(&mut state);
        assert_ne!(state.lanes.key, key, "a new note must");
        assert_eq!(state.lanes.lanes().len(), 2);
    }

    /// The set is handed over in pitch order whatever order the notes arrived
    /// in — it comes out of a HashMap, whose iteration order varies per run,
    /// and the names are placed first-come. Two renders of one take must not
    /// disagree about which names fitted.
    #[test]
    fn the_lane_set_is_ordered_by_pitch_however_the_notes_arrived() {
        let mut state = state(48.0, 10.0);
        for (i, note) in [72u8, 55, 61, 60, 67, 48].into_iter().enumerate() {
            state.tracker.handle_event(on(i as f64 * 0.1, note));
            state.tracker.handle_event(off(i as f64 * 0.1 + 0.05, note));
        }
        refresh(&mut state);
        let pitches: Vec<f32> = state.lanes.lanes().iter().map(|l| l.midi).collect();
        assert_eq!(pitches, [48.0, 55.0, 60.0, 61.0, 67.0, 72.0]);
    }

    /// A pitch the visible lattice has no node for still gets a name, spelled
    /// the equal-tempered way — and still as a [`NoteName`], so it draws
    /// through the same code as every other lane rather than down a second
    /// path that could look different.
    ///
    /// It is not a corner: the pane already flags notes sounding off the
    /// lattice with a band down the spectrum, so the case is expected, and a
    /// lane with no name at all would read as a bug rather than as an answer.
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
}
