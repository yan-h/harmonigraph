//! Note names: every note in the roll labeled at its onset, over its own
//! ribbon, in the lattice's own hand.
//!
//! What they answer is reading the SPECTROGRAM. A band of energy tells you
//! there is something at some height on a pitch axis that is continuous in
//! cents, and the fixed marks on that axis are frequencies on the 1-2-5 series
//! — decades apart, and in the wrong currency for naming a note at all — so
//! reading the band as a pitch means interpolating between two of them by eye,
//! on a picture that is scrolling. Color cannot say it either: the roll's
//! colors are the lattice's own, which already spend themselves on channel
//! and pitch height, and a second pitch-keyed scheme laid over the first is
//! two things to read where there was one.
//!
//! So the roll says it, since the roll is already drawing the notes: a name
//! written ON each ribbon, at one of its ends. The heatmap band under a
//! ribbon is the same note, so naming the ribbon names the band.
//!
//! WHICH end is the layout's first and a setting's second: a name goes on the
//! end that READS first, so the gap a reader measures — letter to ribbon end —
//! is the same gap in all four orientations, and the setting asks for the
//! other end. See [`Anchor::of`], which holds the trade.
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
//! the same [`draw_stacked_name`](crate::marks::draw_stacked_name) — letter
//! at full size, accidental riding high, syntonic-comma mark low, septimal
//! mark in a column of its own past them, all counted rather than repeated. Not a resemblance but the same function, so the two
//! cannot drift apart. That is the errand: a name here is read against the
//! lattice, so it has to answer in the lattice's vocabulary. A cents deviation
//! off the nearest piano key would say where a pitch sits between two keys
//! when what is wanted is which node it IS — and for a just third, `E-` says
//! it where "E +14\u{a2}" does not.
//!
//! Geometry comes from [`Axes`] like everything else in the pane, so names turn
//! and flip with it. One thing here does name a screen side, and only one:
//! which END of a ribbon a name is written on ([`Anchor::of`]), because a name
//! is a word and a word is read from its own left however the picture under it
//! is turned. Everything downstream of that choice — the box, the growth, the
//! clamp — reads the direction it hands back and names no side of its own.

use std::collections::HashMap;

use harmonigraph_core::{LatticePos, NoteName, PitchClass, RollNote, Tuning};
use harmonigraph_scene::{DrawnWindow, ViewConfig};

use crate::marks;
use super::axes::{Axes, PitchScale, TimeAxis};
use crate::{theme, SharedState};

/// Point size of a name's letter. Well under the axis labels'
/// ([`MARKING_PT`](super::axes::MARKING_PT)): there are many more of
/// these, and they sit inside the picture rather than along its edge.
///
/// The size at the pitch zoom it is dialled for, that is — the pane hands
/// [`plan`] and [`draw`] a scale that grows this as the range narrows. See
/// `spectral::name_zoom`.
///
/// Where the Name size bar settles, which is what quotes this number rather
/// than any round figure. [`LABEL_PAD`] and [`REPEAT_GAP`] ride the same
/// scale, being the clear space this type demands around itself, so the bar
/// moves all three together. [`LABEL_INSET`] does NOT: it is a screen
/// distance the pane scales, not a spacing quoted against this size at all.
pub(super) const LABEL_PT: f32 = 12.35;

/// Points the name is set in from the end of the ribbon it is anchored to,
/// along the time axis. Enough that the letter reads as standing OFF that end
/// rather than as touching it — the air is what makes the name a label on the
/// ribbon instead of another mark along it, and a letter set tight against the
/// end reads as part of the drawing at the size these are set in.
///
/// A distance on the SCREEN, and the one length here that does not go up with
/// the type ([`NameScale::air`] carries it, not the type's own half of that
/// pair). The pitch zoom grows a name in
/// proportion so that it keeps its footing on a ribbon that is growing too —
/// but the gap between the ribbon's end and the letter is not part of the
/// picture being magnified, it is the join between the name and the thing it
/// names. Scaled along with everything else it opens as the range closes: a
/// name set 4 points off its note at the whole axis sits 20 off it at the
/// two-octave floor, so from a reader's side the names slide down the roll for
/// as long as the zoom is being dragged — a movement the music did not make.
///
/// It pins the letter's INK, which is the only thing a reader can measure a gap
/// against. A box does not reach all the way to the glyph: the layout carries
/// the font's side bearing, and [`name_extent`]'s estimate carries its own
/// error on top. Pinning the box instead therefore sets a name at this distance
/// PLUS two terms that both ride the type — so the gap opens as the pitch zoom
/// does, and the name drifts off its note for as long as the range is dragged.
///
/// Measured through a real context at `ppp` 2, anchor to the drawn `C` of a
/// bare name, pinning each of the two:
///
/// | zoom | pinned by the box | pinned by the ink |
/// |------|-------------------|-------------------|
/// | 1    | 4.49              | 4.00              |
/// | 2.23 | 5.53              | 4.00              |
/// | 5    | 7.01              | 4.00              |
///
/// Time running DOWN the pane is the worse of the two, and by a long way: a
/// line box stands well above its letter, so the same reading there goes 7.33 →
/// 11.19 → 19.54, better than 12 points of drift where across the pane it is
/// 2.5. The vertical term is [`LINE_HEIGHT`]'s and not [`GLYPH_ADVANCE`]'s,
/// which is why tightening the advance does nothing for it and pinning the ink
/// does. See [`marks::NameLead`], where the ink is found, and issue #349.
///
/// What it still scales by is the PANE, which is not the same concession. The
/// Render preview draws this pane a fraction of the size the offline render
/// draws it, and the two have to be one picture at two sizes — so a gap left at
/// flat points would be a small share of a name's height in the video and most
/// of one in the preview it is dialled in on, which is the divergence this
/// codebase least wants.
const LABEL_INSET: f32 = 4.0;

/// The two scales a name is laid out by, and the pitch zoom is what parts
/// them.
///
/// One struct rather than two arguments because they are the same size in every
/// picture that is not zoomed, so a call passing one for the other draws
/// correctly at the dialled view and wrongly everywhere else — a swap that a
/// look at the default pane cannot see.
#[derive(Clone, Copy, Debug)]
pub(super) struct NameScale {
    /// What the TYPE is set at, as a multiple of [`LABEL_PT`]: the pane's own
    /// size, the user's bar and the pitch zoom, all three. Everything measured
    /// in the type's own terms rides on it — the boxes, and the room the
    /// thinning hands out.
    pub(super) label: f32,
    /// What a clear space fixed on the SCREEN is scaled by: the pane's own size
    /// and nothing else. See [`LABEL_INSET`], which is the whole of what it
    /// carries.
    pub(super) air: f32,
}

/// What a monospace glyph advances, and a line box stands, as fractions of the
/// font size — and the clear space a name demands around itself, in points.
///
/// An ESTIMATE, deliberately, rather than a galley measured through egui. It
/// decides only which names are dropped for colliding, so being a few percent
/// wide costs a name that would have just fitted and nothing else; against
/// that, measuring would put a text layout per candidate per frame in front of
/// a decision that is thrown away for most of them, and would make the offline
/// render's output depend on font metrics rather than on arithmetic.
///
/// It decides only that BECAUSE it no longer places anything: a name is drawn
/// against its letter's ink ([`marks::NameLead`]), so what this is wrong by
/// costs a little spacing and never moves a glyph. The estimate is free to be
/// generous; it is not free to be wrong about the face.
///
/// Half an em is what the tree's face advances — Iosevka Fixed sets every
/// `hmtx` advance to 500/1000, which is the same fact [`marks::MARK_ADVANCE`]
/// states for the mark column. It is quoted against THIS face and not against
/// monospace in general: egui's stock monospace advances nearer 0.62, and a
/// figure carried over from it makes the letter term a quarter too wide, which
/// is a margin nobody chose and cannot say the size of.
/// `a_bare_names_estimate_is_the_advance_the_face_actually_has` is what holds
/// the two together, since nothing in the picture can — see below.
///
/// The margin that IS chosen stays: a counted mark is measured here at two full
/// cells where the draw path tracks it into its sign's
/// ([`marks::MARK_TRACK`]), so the estimate keeps its slack where too narrow
/// would let names overlap.
///
/// What it moves is the THINNING, and that is the whole of what it can move.
/// The direction is not the obvious one, so it is worth stating which way a
/// wider figure here goes: a wider name makes a wider cell, the grid offers
/// less often, and — because a refused offer still advances the lane's reach —
/// offering less often into a wall of notes gets more of them KEPT. Measured on
/// the dense `phrase` fixture, a fifth wider (0.62) draws 15 names at the
/// two-octave floor against 13, and 30 at 2.23× against 31; at the dialled size
/// the two agree at 48. Which is the grid's first defect (see [`Lane`]) showing
/// through a number that touches it, rather than anything this number decides.
const GLYPH_ADVANCE: f32 = 0.5;
const LINE_HEIGHT: f32 = 1.3;
const LABEL_PAD: f32 = 1.95;

/// Clear time a name demands beyond its own box, in points along the time
/// axis, before the next name at that pitch may take a place.
///
/// Without it, successive names at one pitch are allowed to butt together, and
/// a run of repeats reads as a word rather than as a name on each of several
/// notes. This is the "certain span" the greedy leaves between the instance it
/// picks and the next one it will take.
const REPEAT_GAP: f32 = 7.8;

/// When each pitch last gave a name away — the whole of the thinning's state.
///
/// Thinning happens along TIME and within ONE PITCH only. A repeat waits for
/// clear room after whichever instance took the name; a name at one pitch never
/// suppresses a name at another, however close on screen the two land.
///
/// Overlap across pitch is therefore accepted, deliberately and for now: at a
/// wide zoom a chord's names do land on each other, and refusing them is the
/// worse of the two failures — a name you can read through a collision is
/// worth more than a clean gap where a name should have been. A better answer
/// than either (nudging them apart, stacking them, thinning by loudness) is
/// deferred rather than guessed at.
///
/// Measured in seconds of take rather than points of pane, and decided against
/// a GRID of absolute time rather than against whatever else is on screen.
/// Both of those are what keep the names still as the picture scrolls — see
/// [`Lane`] and [`plan`].
#[derive(Default)]
struct Occupancy {
    pitches: HashMap<i32, Lane>,
}

/// One pitch's share of the thinning: the grid it offers names on, and how far
/// the last note offered one reached.
///
/// The GRID is what makes this stable. Thinning is a question about which of
/// several close repeats keeps its name, and any rule of the form "the next
/// one with room after the last name taken" is a chain — every answer resting
/// on the one before it, back to wherever the sweep began. Whatever that
/// beginning is, it MOVES: the window's oldest note scrolls off, and the
/// roll's own oldest is evicted by
/// [`NoteRoll::MAX_NOTES`](harmonigraph_core::NoteRoll::MAX_NOTES) and
/// [`MAX_AGE`](harmonigraph_core::NoteRoll::MAX_AGE). Move it and the parity of the
/// whole chain flips behind it — the suppressed name takes the ground, the one
/// after it loses it, on down the lane — which is names blinking out and back
/// as the roll scrolls. (Measured against the note cap alone, with no zoom and
/// at the dialled size: 0 blinks over eight seconds with the roll under the
/// cap, 882 at it.)
///
/// So no chain. A note is OFFERED a name only if it is the first of its pitch
/// in its grid cell, and the cells are laid out in absolute take time — so
/// which note is offered depends on that note's own time and nothing else. It
/// then has to clear the previous cell's OFFER, kept or not; that keeps two
/// names from landing together across a cell boundary without reintroducing a
/// dependence on what was kept. A note's fate therefore rests on two adjacent
/// cells of take time, and nothing outside them can reach it.
///
/// The grid is a name's room, which is what keeps the density the chain had:
/// in a run of repeats the offers fall exactly one room apart, and each clears
/// the last by exactly nothing to spare.
///
/// WHICH name is the unsound part, and there are two known defects in it. A
/// lane is [`LANE_CENTS`] — ten — wide, while a name is matched to a node at
/// `Tuning::tolerance`, half a cent by default. So a lane is not one spelling
/// and not one width: inside four cents of the just third this lattice spells
/// `E-`, `E♯-5↓` and `E`, which are 11.9, 21.8 and 7.7 points at the dialled
/// size. `a_name_is_read_from_its_own_pitch_not_a_lane_neighbours` is the same
/// fact, proved of the naming.
///
/// Every measurement below is taken with the sevens axis OPEN
/// (`extent_sevens: 1`), which is not where a fresh view starts — the captured
/// default opens flat, and the naming reach then holds the home sheet
/// alone, so no roll name carries a septimal mark and `E♯-5↓` above is not one
/// of the spellings on offer. The defects are about lattices with depth, which
/// is the case worth stating them for; opening the sevens axis is what
/// reproduces them.
///
///   - The grid is taken from whichever note the sweep reached FIRST, and the
///     sweep starts at `oldest - lookback`, which scrolls. So in a lane holding
///     two spellings the cell boundaries depend on where the sweep began, which
///     is the one dependence the paragraph above exists to remove. Measured: a
///     note that stops nine seconds before the window opens takes a 24-note run
///     from 12 names to 7 and moves every one.
///   - What an offer must clear is read off this grid while the name is DRAWN
///     at its own width, so a name wider than its lane's overruns the one
///     before it — consecutive `E♯-5↓` overlap by 4.69 points live, and up to
///     14.11 in the whole-song layout.
///
/// Both are left standing because every local repair measured worse, and the
/// two obvious ones badly: sizing the grid at the lane's CENTRE pitch names a
/// pitch nobody plays, which in any non-equal tuning is almost never within
/// tolerance of a node and so takes `note_name`'s equal-tempered fallback —
/// 171 of the 361 lane centres between MIDI 48 and 84 have no node at all
/// under `Tuning::just()`. The cell then has nothing to do with its material
/// in either direction, and where it reads narrower than its notes, `room`
/// exceeds the cell and `reached` advances on refusal too, so nothing recovers:
/// a note repeated forever in a just tuning draws NO name (measured 7 → 1, and
/// 84 lanes between 55 and 72 semitones sit in that regime). Taking the reach
/// from the note's own name instead fixes the overlap and starves the same way,
/// the cell no longer bounding what a name demands.
///
/// What is left is a decision about what a lane IS, which is why it is written
/// down rather than patched: a per-note cell width destroys the absolute
/// partition this design needs and puts the chain straight back; one grid as
/// wide as [`WIDEST_NAME`] spaces every plain `C` as though it were a
/// double-sharp with twelve commas; and splitting a lane by spelling stops
/// near-pitches contending at all, so two names ten cents apart would simply be
/// drawn on top of each other.
#[derive(Clone, Copy)]
struct Lane {
    /// Cell width in seconds: a name's room plus the gap it asks for — see the
    /// two defects above for WHOSE name, which is not reliably this pitch's.
    grid: f64,
    /// The last cell that offered a name here.
    cell: i64,
    /// How far that offer reached, whether or not it was kept.
    reached: f64,
}

impl Lane {
    fn new(grid: f64) -> Lane {
        Lane { grid, cell: i64::MIN, reached: f64::NEG_INFINITY }
    }
}

/// How far apart two pitches must be, in cents, to be different lanes for
/// thinning.
///
/// A tenth of a semitone. Not finer: at a two-octave zoom on a docked pane a
/// point is about four cents, so a grain of one cent would call pitches
/// different lanes that share a pixel row — and any material whose tuning
/// drifts between repeats (adaptive tuning, MPE expression that lands a hair
/// off where the last one did) would get no thinning at all, every note being
/// its own lane. That is the failure precisely in the material this plugin
/// exists for. Not coarser: a syntonic comma is 21.5 cents and must stay two
/// lanes, since two nodes a comma apart are two different notes.
///
/// It grades the THINNING only. A name is still chosen from the exact pitch,
/// so what a lane is called is not rounded — only whether two of them compete.
const LANE_CENTS: f32 = 10.0;

/// A pitch as an occupancy key — see [`LANE_CENTS`].
fn pitch_key(midi: f32) -> i32 {
    (midi * 100.0 / LANE_CENTS).round() as i32
}

/// How far a name reaches along the depth axis, in screen points: its padded
/// box, projected onto whichever way that axis runs.
///
/// The depth direction is axis-aligned — the screen's x when time runs across
/// the pane, its y when time runs up or down it — so projecting answers all
/// four orientations without naming a screen side.
fn depth_extent(axes: &Axes, name: &NoteName, size: f32, label_scale: f32) -> f32 {
    let extent = name_extent(name, size);
    let depth = axes.dir_depth();
    (extent.x * depth.x).abs() + (extent.y * depth.y).abs() + 2.0 * LABEL_PAD * label_scale
}

/// The stretch of TAKE TIME a name covers, from an anchor at `at`.
///
/// A name always lies from its anchor over the ribbon it names, and which way
/// through TIME that is depends on both the layout and which end the anchor is
/// ([`Anchor`]): a leading edge is the ribbon's recent end and lies back into
/// the past, an onset is its old end and lies forward. So which of two names
/// reaches across the other is answered here; the thinning above only compares
/// spans.
///
/// **The THINNING cannot feel which of the two the caller passes**, and the
/// LIFETIME can. A lane's reach is one number for the whole lane (see
/// [`Lane`]), and against a sweep in ascending anchor order the two conventions
/// are one inequality written twice — `at - reach >= prev` and
/// `at >= prev + reach` — so every thinning test in this file passes with the
/// direction forced either way. What reads it for real is
/// [`shows`](plan): a name lives while its own span still reaches the far edge,
/// and at the onset anchor that is `at + reach` where the other convention says
/// `at`. Forced backward there, a name would go the instant its onset crossed
/// the edge — the whole of it still on the pane, and gone between two frames.
fn name_span(at: f64, reach: f64, backward: bool) -> (f64, f64) {
    if backward {
        (at - reach, at)
    } else {
        (at, at + reach)
    }
}

/// A name as wide as one is ever likely to be: a double accidental, a
/// two-figure comma count and a two-figure septimal one, which is a node most
/// of a lattice away from anything anyone plays.
///
/// Used only to bound how far back of the window the thinning has to read (see
/// [`plan`]), where being generous costs a few notes of extra sweep and being
/// short costs the stillness the grid is there for. It is deliberately NOT the
/// grid itself: a grid this wide would space every plain `C` as though it were
/// this, and give up most of the names in a run of repeats.
const WIDEST_NAME: NoteName =
    NoteName { letter: 'C', sharps: 2, syntonic_commas: -12, septimal_commas: -12 };

/// One name, placed: what it says, the box it was measured into, and the point
/// its letter is drawn against.
#[derive(Clone, Copy, Debug)]
pub(super) struct NoteLabel {
    pub name: NoteName,
    /// Screen box the name covers, padded — what the THINNING reasons about,
    /// and an estimate throughout ([`name_extent`]).
    ///
    /// Not where the name is drawn, and the two are kept apart deliberately: a
    /// box is measured from arithmetic so the offline render does not hang on
    /// font metrics, and everything the box decides — which names fit, how far
    /// apart they sit — is happy with an estimate a few percent wide. Where a
    /// reader SEES the name is not happy with it, the error riding the type and
    /// so opening with the pitch zoom. See [`lead`](Self::lead) and issue #349.
    pub rect: egui::Rect,
    /// Where the letter's ink goes: [`LABEL_INSET`] off the end of the ribbon
    /// this name belongs to, along [`grow`](Self::grow), and carrying whatever
    /// `place`'s clamp did to the box.
    pub lead: egui::Pos2,
    /// The direction from that point INTO the note — see [`label_rect`].
    pub grow: egui::Vec2,
    /// Test-only: the take time this name was placed at. Which NOTE a name
    /// belongs to is the whole question when asking whether the set of them
    /// holds still as the picture scrolls, and a rect that scrolls cannot
    /// answer it.
    #[cfg(test)]
    pub at: f64,
}

/// Every name this frame draws, already thinned to the ones that fit — empty
/// when the setting is off or the pane has kept no roll region to draw in.
///
/// The two scales it lays names out by part company at the pitch zoom — see
/// [`NameScale`].
pub(super) fn plan(
    state: &SharedState,
    axes: &Axes,
    scale: &PitchScale,
    split: f32,
    now: f64,
    scales: NameScale,
) -> Vec<NoteLabel> {
    let cfg = &state.spectrum_config;
    // Names label RIBBONS, so they need ribbons. With the roll hidden there is
    // nothing under them to name and they would be text floating over the
    // heatmap at whatever pitches notes happened to have — which is not the
    // same picture at all, and would come from a checkbox in the roll's own
    // section that appeared not to turn them off.
    if !cfg.note_names || !cfg.show_roll || split >= 1.0 {
        return Vec::new();
    }
    let time = TimeAxis::new(state, split, now);
    let anchor = Anchor::of(&time, cfg);
    let roll = state.roll();

    let size = LABEL_PT * scales.label;
    // One point of the depth axis, in seconds of take. A name's reach is a
    // length on the screen and the thinning measures in TIME, so this is the
    // rate between them — one number, the time axis being linear across the
    // region.
    let seconds_per_point = time.seconds_per_point(axes);
    let gap = (REPEAT_GAP * scales.label) as f64 * seconds_per_point;
    let room = |name: &NoteName| {
        depth_extent(axes, name, size, scales.label) as f64 * seconds_per_point + gap
    };
    // The air a name stands off the end it is written on, in the same currency:
    // [`LABEL_INSET`] is ink no box carries, and the lifetime below is the one
    // reader of that difference.
    let inset = (LABEL_INSET * scales.air) as f64 * seconds_per_point;
    // Which way a name lies through TIME from where it is anchored: back over
    // the ribbon behind a leading edge, forward over the ribbon ahead of an
    // onset. See [`name_span`].
    let backward = anchor == Anchor::Leading;
    // ...and which way it lies on SCREEN, which is the same fact in the other
    // currency: from the ribbon's head the name runs into the picture (with
    // increasing depth), from its onset back out toward the now-line.
    let grow = if anchor == Anchor::Onset && !time.whole_song() {
        -axes.dir_depth()
    } else {
        axes.dir_depth()
    };

    // How far back of the window the sweep has to read. NOT the whole roll,
    // and not the window either.
    //
    // A note's fate rests on its own grid cell and the one before it (see
    // [`Lane`]), so the sweep must see whole cells back that far — and no
    // further, however long the music has been playing. Four cells of the
    // widest name any lane can want: the first cell in the range may be cut in
    // half by wherever the range begins, and the offer after it compared
    // against a cut cell's, so the two that can be wrong sit at least two
    // cells short of the window and never reach the pane.
    //
    // Which end of the region the sweep starts behind is a fact about the
    // LAYOUT — which way take time runs across it — and not about the anchor:
    // whichever end of a ribbon a name is written on, the earliest one that can
    // land on the pane is at the oldest time the region shows.
    let lookback = 4.0 * room(&WIDEST_NAME);
    let oldest = time.oldest();
    let sweep_from =
        if time.whole_song() { time.time_at(0.0) - lookback } else { oldest - lookback };
    let mut notes: Vec<(&RollNote, Edge)> = roll
        .notes()
        // On its stop first, which is the one end every note carries without
        // being asked: reading an anchor reaches into the note's bends
        // for the pitch there, and most of a long roll is nowhere near the
        // window. A note that stops before the sweep begins started before it
        // too, so this drops nothing the exact test would have kept, at either
        // anchor.
        .filter(|note| note.stop(now) >= sweep_from)
        .map(|note| (note, anchor_edge(note, now, anchor)))
        // An anchor inside the sweep, OR a note still on the pane whatever its
        // anchor is doing. The second arm is the note longer than the window:
        // its anchor can be any distance back — a drone's is unbounded — while
        // its ribbon is still filling the picture.
        //
        // What that buys differs by layout, and both halves are worth having. A
        // STILL picture crops rather than scrolls, so such a note's name is
        // drawn at the crop (see [`drawn_edge`]) and this arm is the only thing
        // that reaches it — a take rendered from its second minute names the pad
        // already sounding. LIVE the name has travelled off with the end it is
        // written on and is not drawn at all; what the arm keeps there is the
        // note's turn in the THINNING, which is a fact about the music and does
        // not wait for the pane to be able to show it. Bounded either way,
        // because what bounds it is the window rather than a constant: a note
        // only qualifies while it has ink on the pane, and the roll holds one
        // such note per key.
        .filter(|(note, edge)| edge.time >= sweep_from || note.stop(now) >= oldest)
        .collect();
    // By ANCHOR, oldest first — where the name will sit, which is what
    // the thinning is handing out — and by channel and key after it, since the
    // offline render must not depend on the order the roll happened to hand
    // them back.
    //
    // Those three are a total order at the leading edge, where two entries of
    // one key cannot share a stop. At the ONSET anchor they are not: a key
    // struck, released and struck again at one sample — the delivery
    // `one_press_is_named_once_however_the_host_delivers_it` is about — gives
    // two entries agreeing on all three, and `sort_unstable_by` leaves those
    // in an unspecified order. Harmless, and worth saying why rather than
    // reaching for a fourth key: two entries with one onset land in one cell,
    // so the second is refused whichever comes first, and one name is drawn in
    // one place. Should they ever settle at different PITCHES they are
    // different lanes, where both are named anyway.
    //
    // Oldest first, whether held or not: the order is about which instance of
    // a note takes the name, and a held note is no earlier a note for being
    // held. Held notes are lifted out for DRAWING afterwards, which is a
    // separate question from where they sit here — keeping the two apart is
    // what leaves the held-note exemption below with any teeth.
    notes.sort_unstable_by(|a, b| {
        a.1.time
            .total_cmp(&b.1.time)
            .then(a.0.channel.cmp(&b.0.channel))
            .then(a.0.note.cmp(&b.0.note))
    });

    // The name's ROOM is memoized with it, and has to be: measuring one asks
    // the name for its marks, and each of those builds a String. Per class
    // that is a few allocations a frame; per note it would be thousands.
    let mut names: HashMap<PitchClass, (NoteName, f64)> = HashMap::new();
    // Read once for the whole pass, so every name on the pane is chosen out of
    // one window even if a lattice pane redraws between two of them.
    let shown = state.shown();
    let naming = |pitch: f32, names: &mut HashMap<PitchClass, (NoteName, f64)>| {
        let class = PitchClass::from_cents(pitch.rem_euclid(12.0) * 100.0);
        *names.entry(class).or_insert_with(|| {
            let name = note_name(&state.view, &shown, &state.tuning, pitch);
            (name, room(&name))
        })
    };

    // Where a name goes once it has one: on its own ribbon at its anchor,
    // growing the way [`grow`] points. The two callers below differ in what
    // they do with the box, never in how it is measured.
    //
    // The anchor's DEPTH is read unclamped, and that is what holds the gap
    // between a letter and the end it is written on — the one distance a reader
    // measures a name by. Live, that end SCROLLS. Held inside the region
    // instead, a name whose anchor has left parks on the far edge while its own
    // note goes on sliding out from under it, so the gap opens by the whole
    // length of ribbon still showing: the name stands still in a picture where
    // everything else is moving together, which is the movement the eye follows
    // and the music did not make. Unclamped it travels with its end and the
    // pane's scissor takes it, the same cut a ribbon leaving the far end is
    // already drawn with rather than squashed against the edge
    // ([`TimeAxis::depth_of_unclamped`]).
    //
    // What that costs is that a name goes when its end goes, its own length of
    // ink past the edge and no later: a note held longer than the Span carries
    // its name off the far edge and scrolls the rest of its ribbon unnamed. The
    // lifetime below is where that is decided, and it is also what keeps an
    // unclamped depth finite.
    //
    // The WHOLE-SONG layout clamps, and the difference is the picture's rather
    // than the mode's: a still take is CROPPED at the render's start, not
    // scrolled past it, so a name held at the crop lags nothing — nothing there
    // moves for it to lag — and a note sounding across that start would
    // otherwise go unnamed for the whole video. See [`drawn_edge`], which moves
    // the pitch with the clamp so the pair stays one.
    //
    // A box growing toward the now-line is held on the PANE, and that is the
    // only thing it is held off. A note is younger than its own name for the
    // first fraction of a second of it, so a name written on the end that
    // reaches the present has no ribbon under it yet and lies over whatever is
    // in front of the note — which, that being the now-line side, is the
    // SPECTRUM.
    //
    // It is allowed to. The gap between a letter and the end it is written on
    // is what a reader measures a name by, and holding that gap through the
    // first moments of a note is worth more than keeping the two pictures off
    // each other for those moments: a name stopped at the divider instead sits
    // still while its own note scrolls out from under it, which is a movement
    // the music did not make, and it does it at the one instant the eye is on
    // the note. The name is drawn last of everything on the pane and haloed
    // (see [`draw`]), so what it crosses onto it stays legible over.
    //
    // Past the pane's own edge there is nothing to see — the batch is clipped
    // to the pane, and a neighbouring pane is not this one's to draw in — so
    // that edge is where the clamp stands: the name sits against it and travels
    // as soon as its ribbon is long enough to hold it, which is its own length
    // of scrolling and no more. A name that came and went instead would blink
    // at every note played, and one that started deeper would not be at the end
    // it names.
    //
    // This is the ONE place a name is held off the gap it is owed, and it is
    // worth saying why it is not the far edge's case rewritten. There, a name
    // is held behind a note that is leaving, and what it waits for is unbounded
    // — a drone's onset recedes for as long as the key is down. Here it is held
    // ahead of a note that has not happened yet, and what it waits for is the
    // note's own length of ink, after which the gap is exact again for the rest
    // of the note's life. It also almost never fires: the name has the whole
    // analyzer to lie over first, so it takes a spectrum share squeezed to a
    // name's width to reach the edge at all.
    //
    // What is clamped is the box DRAWN, not the anchor's time: a note's cell
    // and its reach are the music's, and must not move with what the pane had
    // room to show. Measured along `grow` and against the pane edge at the
    // name's own pitch, so no screen side is named and a box growing the other
    // way can never be caught by it.
    let toward_near = grow.dot(axes.dir_depth()) < 0.0;
    let place = |edge: &Edge, name: &NoteName| {
        // Live, the anchor's own depth however far past the far edge it has
        // gone; in a still picture, held at the crop. See above.
        let d = if time.whole_song() {
            time.depth_of(edge.time)
        } else {
            time.depth_of_unclamped(edge.time)
        };
        let t = scale.t_of(edge.pitch);
        let rect = label_rect(axes, grow, t, d, name, size, scales);
        // Where the LETTER's ink is to land, which is what a reader measures
        // the gap by and what [`draw`] finally places the name against. The
        // same inset the box is built from, off the same end, so the two agree
        // about where the name belongs and differ only in what they measure —
        // arithmetic for the thinning, ink for the picture.
        let lead = axes.at(t, d) + grow * (LABEL_INSET * scales.air);
        if !toward_near {
            return (rect, lead);
        }
        // The leading corner's reach past the edge: the box's centre projected
        // onto `grow`, plus half of what it spans that way.
        let span = (rect.width() * grow.x).abs() + (rect.height() * grow.y).abs();
        let over = (rect.center() - axes.at(t, 0.0)).dot(grow) + span * 0.5;
        if over > 0.0 {
            // Both, by the same vector: what the clamp does is hold the name
            // off an edge, and a name is its ink as much as its box.
            //
            // What it measures is still the BOX, which is looser than the ink
            // by however much line box a letter does not fill — so a clamped
            // name stands further inside the edge than it strictly needs to,
            // and the slack is the depth axis's. Time running across the pane,
            // the box's depth is the name's width and the two agree: measured,
            // the ink stands 8.12 points in either way. Time running down it,
            // the box's depth is a whole line box against a letter's cap
            // height, and the ink stands 18.01 points in where placing it by
            // that box put it at 14.18.
            //
            // Left standing, because closing it means clamping on the ink and
            // `plan` has no painter to measure ink with — the same constraint
            // that makes `name_extent` an estimate in the first place. It errs
            // into the pane, so it can only ever withhold a few points of
            // travel from a name too young to have a ribbon yet; it cannot put
            // one off the edge.
            (rect.translate(-grow * over), lead - grow * over)
        } else {
            (rect, lead)
        }
    };

    // Whether a name still has ink on the pane: its own box, laid from the end
    // it is written on, still reaching the far edge.
    //
    // The NOTE cannot answer this for it, and that is the whole of what the
    // anchor changes. At the leading edge the two questions are one — that end
    // IS `stop(now)`, and the name lies back over the ribbon from it, so the
    // name goes exactly when the note does. At the ONSET the name travels off
    // with an end that leaves FIRST, and whatever ribbon is behind that end
    // scrolls on unnamed: the name outlives its own anchor by the ink it
    // carries and no more, which is the fixed gap read the other way round.
    //
    // Measured in TAKE TIME like everything else here, and asked of the DRAWN
    // anchor rather than the true one — a still picture holds a cropped name at
    // its edge, where it always has ink, so this is a live question and answers
    // trivially there.
    let shows = |edge: &Edge, room: f64| {
        // The lane's room less the gap it demands of the next name is this
        // name's own box (the same subtraction the thinning's reach makes), and
        // the air it stands off its end is ink no box carries.
        let (_, latest) = name_span(edge.time, room - gap + inset, backward);
        latest >= oldest
    };

    let mut occupied = Occupancy::default();
    let mut placed: Vec<NoteLabel> = Vec::new();
    // Each thinned placement's lane, alongside it, and the indices of the ones
    // the clamp PARKED — both only for the pass below, which is the one thing
    // here that has to compare two placements against each other rather than
    // against the grid. A `NoteLabel` carries neither, and should not: what it
    // is for is drawing, and neither survives into the picture.
    let mut lanes: Vec<i32> = Vec::new();
    let mut parked: Vec<usize> = Vec::new();
    let mut held: Vec<NoteLabel> = Vec::new();
    for (note, edge) in notes {
        // On the pane, and so worth drawing — decided on the pitch the name
        // will be DRAWN at, not the note's pitch in general, since the two
        // differ for a bent note and it is the name that has to be visible.
        //
        // Three questions, and the last one is the anchor's: the note has
        // ribbon on the pane, that ribbon is inside the pitch zoom, and the
        // NAME still has ink there ([`shows`](plan)).
        //
        // The third is what makes a travelling name leave with the end it is
        // written on. At the leading edge it asks nothing the first has not —
        // that end IS `stop(now)` — but at the ONSET it is sharper by whatever
        // ribbon is left behind that end, and a note held longer than the Span
        // scrolls the last of itself unnamed. That is the price of a gap that
        // is a fixed distance from a note rather than a place on the pane, and
        // it is the one worth paying: a name held back while its own note slid
        // out from under it would be the only thing in the picture standing
        // still.
        //
        // The NOTE's own term is not thereby redundant. A name is drawn while
        // it has ink, and its ink runs a little past the ribbon's end (the
        // inset, and whatever the box's estimate is generous by), so without it
        // a name could outlive by a frame or two the thing it names — which is
        // a label pointing at nothing, and the one failure neither end of this
        // trade wants.
        //
        // Only DRAWING is culled. A note off the far edge still takes its turn
        // in the thinning, which is what lets the names on the pane stand still
        // while it scrolls.
        //
        // The pitch is the one the name will be DRAWN at, not the note's pitch
        // in general, since the two differ for a bent note and it is the name
        // that has to be visible.
        let drawn = drawn_edge(note, &edge, now, &time);
        let visible = note.stop(now) >= oldest
            && scale.contains(edge.pitch)
            && shows(&drawn, naming(edge.pitch, &mut names).1);
        // A held note whose name is anchored on the LEADING EDGE stands outside
        // the sweep in BOTH directions: it is named whatever is already there,
        // and it is not recorded, so it takes nothing out of the running for
        // anyone else.
        //
        // The second half is not a nicety. A held note's name sits at the
        // now-line and stays there, while every other name scrolls away from
        // it — so a held name that occupied its ground would suppress each
        // older name in turn as the two came level and hand it back once they
        // parted, which reads as names blinking out and in for as long as the
        // key is down. Exempting a name from refusal but not from refusing
        // trades one arbitrary gap for a moving one, and a moving one is the
        // worse of the two: an absent name reads as "no room", one that comes
        // and goes reads as a fault.
        //
        // What it costs is that a held name can overlap another. Briefly, and
        // it lands on top (held names are appended last, and drawn in order),
        // and it is the note being played: of the three ways to break the tie,
        // this is the only one that never withholds a name that could have
        // been shown.
        //
        // All of which is about a name standing at the now-line while the
        // picture scrolls past it, so the exemption belongs to the ANCHOR
        // rather than to the key being down: a leading edge is the only thing
        // here that does that. Anchored on the onset a held note's name is at a
        // fixed take time like every other, and is thinned like every other —
        // and has to be, or a name granted the exemption would be withdrawn at
        // the release, which is the one moment that anchor exists to make
        // uneventful.
        if note.is_live() && anchor == Anchor::Leading {
            if !visible {
                continue;
            }
            let (name, _) = naming(edge.pitch, &mut names);
            let (rect, lead) = place(&drawn, &name);
            // Two keys sounding one pitch — a doubled MIDI source, a layered
            // MPE part — would otherwise stamp the same name on the same
            // points once per voice. The name still appears; it is drawn once.
            if !held.iter().any(|l| l.name == name && l.rect == rect) {
                held.push(NoteLabel { name, rect, lead, grow, #[cfg(test)] at: edge.time });
            }
            continue;
        }
        // Everything else is offered a name only as the first of its pitch in
        // its grid cell, and then has to clear what the cell before it offered.
        // See [`Lane`] for why it is a grid and not a queue.
        let key = pitch_key(edge.pitch);
        let lane = match occupied.pitches.entry(key) {
            std::collections::hash_map::Entry::Occupied(lane) => lane.into_mut(),
            // A lane's grid is its own name's room. Every note at one pitch
            // spells the same, so this is asked once per pitch rather than
            // once per note — and the room it yields is the same whichever
            // note in the lane the sweep reaches first.
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(Lane::new(naming(edge.pitch, &mut names).1))
            }
        };
        let cell = (edge.time / lane.grid).floor() as i64;
        if cell == lane.cell {
            continue;
        }
        // The lane's grid IS a name's room here, so the reach is what is left
        // of it once the gap is taken back out.
        //
        // What the offer has to clear is the previous one's INK, with no gap
        // demanded on top: the gap is already built into the cell, so a run of
        // repeats lands its offers one room apart and clears by exactly the
        // gap. Asking for it twice would refuse an offer whenever a note fell
        // late in its cell and the next fell early — which is most of them,
        // and cost half the names in a dense run.
        let (from, to) = name_span(edge.time, lane.grid - gap, backward);
        let clear = from >= lane.reached;
        lane.cell = cell;
        lane.reached = to;
        if clear && visible {
            let (name, _) = naming(edge.pitch, &mut names);
            let (rect, lead) = place(&drawn, &name);
            // A name held at a crop is PARKED: the clamp keeps it on the edge
            // while the thinning that spaced it goes on measuring from the
            // anchor's true time, somewhere off the picture. The two agree only
            // while depth is affine in take time, which is exactly what the
            // clamp stops being true — so a parked name is the one placement the
            // grid cannot vouch for. Read off the clamp itself rather than from
            // a comparison of times, so that the two cannot come apart; noted
            // here, with its lane, rather than re-derived below where neither is
            // still in hand.
            if drawn.time != edge.time {
                parked.push(placed.len());
            }
            lanes.push(key);
            placed.push(NoteLabel { name, rect, lead, grow, #[cfg(test)] at: edge.time });
        }
    }
    // A parked name yields to one that is still at its own anchor and has
    // caught up with it.
    //
    // The thinning hands out room in TAKE TIME, which is the right currency for
    // everything it decides — a name's place is a fact about the music, and
    // measuring it in screen points is what would make the spacing breathe as
    // the picture scrolls. A parked name is the one case where that currency
    // stops converting: it is held at the crop while its own anchor lies further
    // back, so two names the grid spaced seconds apart can be drawn on top of
    // each other. The offender is always the parked one, and always at the
    // picture's own edge.
    //
    // Only a STILL picture parks — a live one lets a name travel off with the
    // end it names (see [`place`](plan)) — so what this pass runs on is a note
    // sounding across a render's start and a later strike of its pitch. It is
    // written against the clamp rather than against the layout all the same:
    // what it repairs is a name drawn somewhere its own anchor is not, and that
    // is the clamp's doing wherever the clamp is.
    //
    // So the parked name goes and the one standing at its own anchor stays.
    // That is the right way round for reading — the survivor is at its note's
    // true onset, where the parked one is only at the edge its note is cut off
    // by — and it cannot BLINK: neither name moves between frames of a still
    // picture, so the pair is decided once and answers the same way for the
    // length of the render.
    //
    // Same lane only. Two names at different pitches are different rows and the
    // pitch axis keeps them apart; it is the repeat of ONE pitch that collides,
    // which is also the only thing the grid was ever spacing.
    if !parked.is_empty() {
        // Compared against the names still at their own anchors, never against
        // another parked one. Two parked names in one lane are two voices
        // sounding one pitch, both cut off by the same edge and both drawn on
        // it, so dropping either would be a coin toss.
        let doomed: Vec<usize> = parked
            .iter()
            .copied()
            .filter(|&i| {
                (0..placed.len()).any(|j| {
                    j != i
                        && !parked.contains(&j)
                        && lanes[j] == lanes[i]
                        && placed[j].rect.intersects(placed[i].rect)
                })
            })
            .collect();
        let mut index = 0;
        placed.retain(|_| {
            index += 1;
            !doomed.contains(&(index - 1))
        });
    }
    // Sounding notes reach the sweep in the tracker's key order — stable, but
    // not an order that means anything here: it would decide which of two
    // overlapping held names lands on top, and that has to follow from where
    // they sit rather than from how they are stored. Every other name was
    // already ordered by the sweep.
    held.sort_unstable_by(|a, b| {
        a.rect.min.x.total_cmp(&b.rect.min.x).then(a.rect.min.y.total_cmp(&b.rect.min.y))
    });
    // Held names last, so that where one overlaps another it is the note under
    // your finger that stays readable.
    placed.append(&mut held);
    placed
}

/// A point on a ribbon a name can be written at: when it is, and what pitch the
/// ribbon has THERE.
///
/// A TIME, not a depth. The thinning is measured in it — a time is a fact
/// about the music, where a depth is a fact about where the window happens to
/// be — and it is the only one of the two that still says anything about a
/// note off the pane, since past either edge every depth clamps to the edge.
/// The depth follows from it (`TimeAxis::depth_of`) for the few notes that are
/// actually drawn.
#[derive(Clone, Copy)]
struct Edge {
    time: f64,
    pitch: f32,
}

/// Which end of a ribbon its name is written on.
///
/// They are the two ENDS, so every name on the pane sits somewhere different
/// under one than under the other — a released note's name at the head of its
/// ribbon or at its tail, a ribbon's length apart. What differs is not only
/// where a name starts but whether it MOVES: a leading edge tracks `now` while
/// the key is down and an onset never moves at all.
///
/// They do agree at one instant, and it is the instant a note is struck: a
/// ribbon of no length has its two ends in one place, at the now-line. So which
/// end a name is on decides what happens to it AFTER that, and every name is at
/// the same place at the moment it appears either way.
///
/// Which of the two a pane uses is [`of`](Self::of)'s, and it is not the
/// setting's alone: the orientation picks the end that READS first and the
/// setting asks for the other one — with the whole-song layout outside both,
/// on the onset always.
#[derive(Clone, Copy, PartialEq)]
enum Anchor {
    /// The end that touches the now-line: the low-depth end live, which is the
    /// side [`SpectralOrientation`](crate::SpectralOrientation) is named for.
    ///
    /// While the key is down the note keeps reaching the present, so this edge
    /// IS the now-line: the name sits still there, at the head of a ribbon
    /// growing out behind it, for as long as the note is held, and starts
    /// travelling at the release. A name you can read in one place while you
    /// play, at the price of a movement the music did not make and of a drone
    /// whose name never scrolls at all.
    ///
    /// Live only — see [`of`](Self::of) for why a static layout cannot use it.
    Leading,
    /// The onset — the moment the key went down, wherever the layout puts it.
    ///
    /// Fixed in take time, so the name scrolls with the picture from the first
    /// frame of the note and nothing about it changes at the release: not where
    /// it sits, not whether the thinning kept it. The price is a note longer
    /// than the window, whose onset leaves the far edge with ribbon still to
    /// come: the name goes off the edge with it, its own length of ink later,
    /// and the rest of that ribbon scrolls unnamed. Holding it back on the edge
    /// instead is the trade [`place`](plan) declines — the gap between a letter
    /// and the end it names is what a name IS here, and a name that outstays its
    /// own end has given that up to stay on screen.
    ///
    /// The whole-song layout has no other option, for a reason that is about
    /// the TAKE rather than about reading order — see [`of`](Self::of).
    Onset,
}

impl Anchor {
    /// The end a reader's eye reaches FIRST — the pane's left where time runs
    /// across it, its top where time runs down it — unless
    /// [`note_names_travel`](crate::SpectrumConfig::note_names_travel) asks for
    /// the other one.
    ///
    /// This is the one place in this file that names a screen side, and it is
    /// forced to: a name is a WORD, and a word is read from its own left
    /// whatever the picture under it is doing. So a name belongs on the end of
    /// its ribbon that comes first, with the note running away under the rest
    /// of it — mirror the picture and the geometry mirrors, but the reading
    /// does not, and a name on the other end reads out of its note instead of
    /// into it. What a reader measures is the gap between the letter and the
    /// end it starts from, and that gap is the same one in every orientation
    /// only if this is.
    ///
    /// WHICH end reads first is the orientation's: live, depth is AGE, so a
    /// ribbon's shallow end is its newest — the leading edge — and it is drawn
    /// at the pane's left or top where time runs the screen's own way, at the
    /// right or the bottom where it runs back against it
    /// ([`SpectralOrientation::is_time_reversed`]). So the reading-first end is
    /// the leading edge in the two orientations that agree with the screen and
    /// the onset in the two that do not.
    ///
    /// So what the setting DOES depends on the orientation, and saying that
    /// plainly is better than the alternative: with the spectrum on the left a
    /// name is on the leading edge by default and waits at the now-line while
    /// you hold a key, and with it on the right the same default is the onset
    /// and the name travels from the first frame. The two pictures are both
    /// still reachable in every orientation — the setting is what reaches the
    /// other one — and it is the GEOMETRY that is held fixed across the four
    /// rather than the held-note behaviour, because the geometry is what a
    /// reader sees on every note rather than only on the one under their
    /// finger.
    ///
    /// **The whole-song layout is outside all of it, and takes the ONSET in
    /// every orientation.** Reading order would name the note's stop in the two
    /// reversed ones, and a stop is not a fact about the take the way an onset
    /// is: a note the recording never released stops at `stop(now)`, which is
    /// the PLAYHEAD, so its name would ride the sweep across a picture that is
    /// otherwise a still. The thinning cannot hold that. It hands out names by
    /// grid cell (see [`Lane`]) and an anchor that walks through cell after
    /// cell is offered a name in one and refused in the next, which is a name
    /// blinking on and off for the length of a rendered video — the very defect
    /// the held-note exemption exists to prevent live, arriving where that
    /// exemption cannot help because nothing in a still picture scrolls away
    /// from anything. The onset is fixed for every note the take contains, so
    /// it is the one end a static layout can anchor to at all.
    ///
    /// [`SpectralOrientation::is_time_reversed`]:
    ///     crate::SpectralOrientation::is_time_reversed
    fn of(time: &TimeAxis, cfg: &crate::SpectrumConfig) -> Anchor {
        if time.whole_song() {
            return Anchor::Onset;
        }
        let reads_first = if cfg.orientation.is_time_reversed() {
            Anchor::Onset
        } else {
            Anchor::Leading
        };
        if cfg.note_names_travel {
            reads_first.other()
        } else {
            reads_first
        }
    }

    /// The ribbon's other end.
    fn other(self) -> Anchor {
        match self {
            Anchor::Leading => Anchor::Onset,
            Anchor::Onset => Anchor::Leading,
        }
    }
}

/// The anchor as the picture can actually draw it: the [`Edge`] itself where
/// the picture reaches it, and the last of the ribbon still showing where a
/// CROP has cut it off.
///
/// Only a still picture crops. A live one scrolls, and there a name travels off
/// with the end it is written on rather than being held against the edge — see
/// [`place`](plan), where that trade is made and measured — so this is a no-op
/// in every live frame, at either anchor.
///
/// Where it does bite, the PITCH has to move with the time. [`place`](plan)
/// holds the whole-song depth inside the region (`depth_of` clamps), so without
/// this the pitch alone would go on describing a point the picture does not
/// contain, and the pair [`anchor_edge`] returns would come apart exactly where
/// the clamp does. That puts a name off its own ribbon: a note gliding while
/// its onset sits before the render's start is at one pitch where the name is
/// written and another where the ribbon crosses the edge, which is a semitone
/// for a modest bend and a quarter of the axis for a wide glide. Reading the
/// pitch at the CLAMPED time closes it — the name lands on the ribbon at the
/// point the reader's eye actually meets it.
///
/// Only the position is clamped, never the note's IDENTITY: the spelling, the
/// lane and the grid cell all keep the true anchor, on the same argument the
/// depth clamp is already made under — a note's cell and its reach are the
/// music's and must not move with what the pane had room to show. So a glided
/// note parked on a crop is drawn where its ribbon is and still spelled for the
/// note that was struck.
fn drawn_edge(note: &RollNote, edge: &Edge, now: f64, time: &TimeAxis) -> Edge {
    let oldest = time.oldest();
    if !time.whole_song() || edge.time >= oldest {
        return *edge;
    }
    Edge { time: oldest, pitch: pitch_at(note, now, oldest) }
}

/// The pitch a note is sounding at take time `t`, straight off the segments the
/// ribbon is drawn from — so a name placed by it lands on the ribbon rather
/// than near it.
///
/// Reads the SEGMENTS rather than interpolating the bends directly, because
/// they are what the roll draws: between two breakpoints the pitch is a straight
/// line ([`RollNote::segments`]), and taking the same line here is what makes
/// "on the ribbon" exact instead of close.
///
/// Off either end it answers the nearest end's pitch, which is the same thing
/// the ribbon shows there. Never `None`: a note's segments are never empty.
fn pitch_at(note: &RollNote, now: f64, t: f64) -> f32 {
    let mut last = note.settled_pitch();
    for ((t0, p0), (t1, p1)) in note.segments(now) {
        if t <= t0 {
            return p0;
        }
        if t <= t1 {
            // Straight line between the two breakpoints, exactly as the ribbon
            // is sheared between them. A segment of no duration is the
            // just-pressed note, where both ends are one point anyway.
            let span = t1 - t0;
            return if span > 0.0 {
                p0 + (p1 - p0) * ((t - t0) / span) as f32
            } else {
                p1
            };
        }
        last = p1;
    }
    last
}

/// Where on a ribbon its name goes, and the pitch the ribbon has there.
///
/// Asked of the two ends' TIMES rather than of their depths, which is the same
/// question — depth is monotone in time — and answerable for a note nowhere
/// near the pane, where a depth says only which edge the note is past.
///
/// **The pitch has to come from the same end as the time**, which is the whole
/// reason this returns a pair. A bent note is at a different pitch at each end:
/// `settled_pitch` is where it began once its tuning had landed, `end_pitch`
/// where it is sounding now. Taking the depth from one end and the pitch from
/// the other puts the name off the ribbon entirely — a semitone off for a modest
/// bend, a quarter of the pitch axis for a wide glide, and over some other
/// note's lane wherever it lands. A held-and-bent note shows it worst at the
/// leading edge: the name stands at the now-line while the ribbon head slides
/// out from under it, and a held note is the one always named there.
///
/// Neither end is bounded here, and the pair is the true one however far off
/// the picture it lies. What a pane can draw is [`plan`]'s question: live it
/// lets a name leave with the end it names, and in a still picture
/// [`drawn_edge`] holds a cropped one at the edge — moving this pitch with it.
fn anchor_edge(note: &RollNote, now: f64, anchor: Anchor) -> Edge {
    match anchor {
        // The onset end, so the pitch the note SETTLED on rather than the key
        // it was pressed at — a retuned note reaches its real pitch a moment
        // after its note-on, and the ribbon is drawn from there. It is also the
        // one pitch on a bent note that stops moving, which is what lets a name
        // anchored here hold both its place and its spelling while the note
        // glides under it.
        Anchor::Onset => Edge { time: note.start, pitch: note.settled_pitch() },
        // Live, time runs from the now-line outward, so the ribbon's leading
        // edge is where it most recently sounded.
        Anchor::Leading => Edge { time: note.stop(now), pitch: note.end_pitch() },
    }
}

/// The screen box a name covers on a ribbon at pitch `p` whose anchor is at
/// depth `d`, padded by the clear space it demands around itself.
///
/// `grow` is the direction from that anchor INTO the note, which is the depth
/// axis for a name at the ribbon's head and against it for one at its onset
/// live — see [`Anchor`]. Everything below reads it rather than the axis, so
/// the two differ in one vector and not in a second set of arithmetic.
///
/// ON the ribbon across the pitch axis — centred on the note's own line, not
/// standing off it. The note is what the name is about, so the name sits on
/// it; the halo every label here carries is what keeps the letter legible
/// against whatever colour the ribbon is (see [`draw`]).
///
/// Along the time axis it grows from the anchor INTO the note, so a name
/// lies over its own ribbon rather than over the picture in front of it —
/// except where the growth runs backward and the name carries marks, which is
/// the trade named at the bottom of this comment and measured in issue #151.
///
/// This is a box and not a position: what a reader sees is placed against the
/// letter's ink by [`draw`], off the same anchor and the same [`LABEL_INSET`].
/// The job here is to say where that ink will LAND, closely enough that the
/// thinning is honest about which names touch — so the box tracks the drawn
/// name's own shape, and the shape has a handedness.
///
/// [`draw_stacked_name`] always sets the letter first and lets the
/// accidental/comma columns trail after it, and the ink is led by that letter.
/// Growth running the screen's own way (left-to-right or top-to-bottom) puts
/// the letter at the anchor end and the marks deeper into the note, so the box
/// runs `[inset, inset + name]` and covers it. Growth running backward puts the
/// letter at the anchor end still — that is what leading by it means — and the
/// marks then trail the OTHER way, back over the anchor. Measuring the pure
/// letter's reach in that branch is what puts the box where they go: it lands
/// at `[inset + letter - name, inset + letter]`, which is the same box slid
/// back by the width of the mark column, and that is exactly the ground the
/// ink covers.
///
/// WHICH growth a name has is the setting's and not the orientation's: a name
/// on the end that reads first grows the screen's own way in all four
/// orientations (that is what reading first MEANS — see [`Anchor::of`]), and a
/// name on the far end grows backward in all four.
///
/// What that costs is worth stating at its real size, because it is not a
/// rounding error: a name whose marks are wider than [`LABEL_INSET`] — which is
/// every marked name — puts its mark column PAST the end it is anchored to,
/// over whatever the picture holds beyond it. Measured as INK on a 300pt pane,
/// a `B♭↓` crosses by 4.81 points at the dialled size and 38.73 at the
/// two-octave floor: the marks grow with the type and [`LABEL_INSET`] does not,
/// so what the inset buys back is the same 4 points at five times the size.
///
/// The two constraints cannot both hold while [`draw_stacked_name`] typesets the
/// marks after the letter: leading by the letter holds it still and lets the
/// marks travel, and containing the name puts the letter back on however wide
/// those marks are. Leading by the letter is the choice — a reader lines a
/// column of names up by their letters — and it is one edit in
/// [`marks::NameLead`] to reverse. Issue #151 holds the measurements and the
/// candidate ways out; this comment exists so the spill reads as a known price
/// rather than as a bug nobody noticed.
///
/// It is not the Right orientation's alone, which is how #151 first read, and
/// the spilling case is not the one the growth's sign picks out. What spills is
/// whichever direction the MARKS run against, and they always run to the right
/// of the letter and above it: growth leftward (either horizontal orientation
/// writing on the far end) sends the mark column back over the anchor, and
/// growth DOWNWARD — Top's own default — sends the accidental up over it.
/// Measured at the same pane, `B♭↓`'s ink reaches 4.81 points past
/// the end growing leftward and 2.05 growing down, against 4.00 clear in the
/// two directions the marks trail into the note. The vertical case merely
/// LOOKED contained while the box placed the name — a line box stands tall
/// enough above its letter to hide the mark riding there — and it clears the
/// end at the dialled size, crossing only once the zoom opens past about 2.2.
/// That threshold is [`LABEL_INSET`]'s to move and nothing else's: every other
/// length in the comparison rides the type, so it is proportional to the inset.
///
/// [`draw_stacked_name`]: crate::marks::draw_stacked_name
fn label_rect(
    axes: &Axes,
    grow: egui::Vec2,
    p: f32,
    d: f32,
    name: &NoteName,
    size: f32,
    scales: NameScale,
) -> egui::Rect {
    let extent = name_extent(name, size);
    // How far the box reaches the way it grows: text always runs across the
    // screen, so that is its width when time runs across the pane and its height
    // when time runs up or down it. Projecting answers all four without naming a
    // screen side.
    let along = (extent.x * grow.x).abs() + (extent.y * grow.y).abs();
    // The same projection, but of the bare letter alone -- no accidental,
    // comma, or septimal mark -- which is what backward growth measures from.
    let bare = NoteName { letter: name.letter, sharps: 0, syntonic_commas: 0, septimal_commas: 0 };
    let letter_extent = name_extent(&bare, size);
    let letter_along = (letter_extent.x * grow.x).abs() + (letter_extent.y * grow.y).abs();
    let inset = LABEL_INSET * scales.air;
    // `grow.x + grow.y` is its own sign: +1 forward (the box grows the screen's
    // own way), -1 backward. Backward is where the letter and the box disagree
    // on which end is "first" -- see above.
    let growth = if grow.x + grow.y < 0.0 {
        letter_along - along * 0.5
    } else {
        along * 0.5
    };
    let centre = axes.at(p, d) + grow * (inset + growth);
    egui::Rect::from_center_size(centre, extent).expand(LABEL_PAD * scales.label)
}

/// What a name covers, estimated from the sizes its pieces are laid out at.
///
/// A stacked name is a letter with a column of marks after it — see
/// [`marks::draw_stacked_name`] — so its width is the letter plus the wider
/// mark, and its height is the letter's line box, which the marks are sized to
/// stay inside.
///
/// A counted mark is measured here at two full cells, which the draw path no
/// longer spends: `marks::MARK_TRACK` sets a count into its sign's cell, so
/// this reads `0.06 · mark_size` wide per counted column. Deliberately not
/// mirrored. This estimate drives the thinning and the label boxes, where too
/// wide only spaces labels further apart than their ink needs and too narrow
/// lets them overlap — so the error belongs on this side, and chasing a
/// sub-point refinement would move roll layout for nothing.
fn name_extent(name: &NoteName, size: f32) -> egui::Vec2 {
    let marks = name
        .accidental_mark()
        .chars()
        .count()
        .max(name.comma_mark().chars().count());
    let mark_size = size * marks::MARK_SIZE / marks::NAME_SIZE;
    // The septimal mark takes a column PAST those two, with air before it,
    // so a name carrying one is wider than its accidental stack suggests —
    // see `marks::draw_stacked_name`. Missing it here would let a `B♭↓`
    // overlap whatever the thinning decided it cleared.
    let septimal = name.septimal_mark().chars().count();
    let gap = if septimal == 0 { 0.0 } else { marks::SEPTIMAL_GAP * mark_size };
    egui::vec2(
        (size + (marks + septimal) as f32 * mark_size) * GLYPH_ADVANCE + gap,
        size * LINE_HEIGHT,
    )
}

/// A note's name: the LATTICE's spelling of its pitch.
///
/// No octave number, because a lattice node is a pitch class and wears none
/// either — and on this pane the octave is already said by where the name
/// sits, which is its height on the axis.
///
/// The [`spiral`](crate::panes::spiral) names notes through here too and drops
/// the octave for a different reason: there a name stands on the rim at one
/// fixed radius and serves every octave of its class at once, so where it sits
/// says the pitch CLASS and nothing about which octave is sounding. What
/// answers that is the dots on the turns, which are not the name.
///
/// The REACH is asked first and the picture's own window
/// ([`SharedState::shown`](crate::SharedState::shown)) only where the reach
/// comes back empty, which is what keeps two things true at once. A name is
/// stable: the reach is the same block whatever the camera is doing, so
/// panning and zooming do not respell a note that was already named, and the
/// walk is over a thousand positions rather than the twenty thousand a drawn
/// window can reach — per played pitch, per frame. And a name agrees with the
/// picture: where the lattice is drawing a node the reach cannot spell, that
/// node names the note, instead of the pitch dropping to a spelling the
/// lattice is visibly contradicting one pane away.
///
/// The equal-tempered fallback is what is left when NEITHER window has a node
/// — still a [`NoteName`], so it draws identically and there is one rendering
/// path rather than two. It is a real case, and a narrower one than the red
/// band's: the band asks the picture alone, so a pitch the reach can spell
/// while the pane is not drawing it wears a band and still gets its lattice
/// name. That is the two answering the two different questions they are for —
/// the band says what is on screen, the name says what the note is called —
/// and a name that changed under a pan would be the worse of the two to make
/// agree. A note with no name at all would just look like a bug.
pub(crate) fn note_name(
    view: &ViewConfig,
    shown: &DrawnWindow,
    tuning: &Tuning,
    midi: f32,
) -> NoteName {
    // Cents from C, measured from MIDI 0 (which IS a C) — the same reduction
    // the pane's hover makes before asking the same question.
    let pc = PitchClass::from_cents(midi.rem_euclid(12.0) * 100.0);
    let reach = view.reach();
    match naming_node(&reach, view, tuning, pc)
        .or_else(|| naming_node(shown, view, tuning, pc))
    {
        Some(pos) => crate::panes::display_note_name(pos, view.tempered()),
        None => equal_tempered_name(midi),
    }
}

/// The node in `window` to name a pitch by: the closest match, and among
/// matches equally close the one that spells most plainly.
///
/// Its own function rather than
/// [`nearest_shown_node`](crate::panes::nearest_shown_node), which it otherwise
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
fn naming_node(
    window: &DrawnWindow,
    view: &ViewConfig,
    tuning: &Tuning,
    pc: PitchClass,
) -> Option<LatticePos> {
    window
        .positions()
        .filter(|&pos| tuning.matches(pc, tuning.pitch_class(pos)))
        .min_by_key(|&pos| {
            (
                pc.distance_to(tuning.pitch_class(pos)),
                spelling_cost(crate::panes::display_note_name(pos, view.tempered()), pos),
            )
        })
}

/// How hard a spelling is to read, worst first: comma marks, then
/// accidentals, then how far out the node sits, then which side of the origin
/// it sits on.
///
/// The first three are in that order because that is the order the marks cost
/// a reader. Four fifths up from C and one just third up from C are the same
/// pitch in an equal temperament, and they spell `E` and `E-`; the plain
/// letter is the name for it, even though the comma'd node is nearer the
/// origin.
///
/// BOTH comma marks count toward the first term, and they have to. The
/// sevens axis used to add no mark, so an off-sheet node spelled exactly
/// like the node two fifths down and the choice between them was invisible
/// — which meant the distance term silently decided it, and decided it
/// wrong: in an equal temperament `(2,0,-1)` is nearer the origin than
/// `(4,0,0)`, so a plain `E` was being named off the sevens sheet. It only
/// became visible when that node started spelling `E↑`.
///
/// The last term settles what the first three cannot, and exists because there
/// is a real tie they cannot reach: in an equal temperament the tritone is six
/// fifths up (`F♯`) and six fifths down (`G♭`), which cost the same on every
/// other count. Left unbroken, `min_by_key` returns whichever the node
/// iteration happened to reach first — so the spelling of every tritone on the
/// pane would hang on the order `positions_within` walks its ranges, and would
/// flip silently if that were ever changed for an unrelated reason. Sharps are
/// preferred, which is a convention rather than a deduction; what matters is
/// that it is written down here and not implied by a loop elsewhere.
fn spelling_cost(name: NoteName, pos: LatticePos) -> (i32, i32, i32, i32) {
    (
        name.syntonic_commas.abs() + name.septimal_commas.abs(),
        name.sharps.abs(),
        pos.threes.abs() + pos.fives.abs() + pos.sevens.abs(),
        -pos.threes,
    )
}

/// The nearest piano key, spelled with sharps — [`KEY_NAMES`](crate::panes::KEY_NAMES)
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
    // No septimal component: this is the fallback for a pitch the visible
    // lattice has no node for, so there is no sevens axis to be off.
    NoteName { letter, sharps, syntonic_commas: 0, septimal_commas: 0 }
}

/// The names, into whichever batch the pane is drawing its labels from.
///
/// Drawn by [`marks::draw_stacked_name`] — the lattice's own label code, not
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
    //
    // `want` is what the pitch zoom asks for and is continuous; `scale` is the
    // rung of the ladder it is rasterized on, and `magnify` the rest. Splitting
    // them here rather than in `text_scales` is deliberate: everything ABOVE
    // this -- which names fit, how far apart they sit (`plan`) -- is laid out
    // against the size the names are really drawn at, so the spacing follows a
    // zoom as smoothly as the ribbons do.
    // Quoted against LABEL_PT rather than against the lattice's letter, which
    // is what puts the ladder's anchor ON the size these names are dialled at:
    // scale 1 IS 12.35pt, so a pane sitting at its default zoom is one rung
    // exactly and the only residual left is the pixel grain -- 24.7 physical
    // pixels at 2x, which no raster can be, so it draws at 24.7 off a 25-pixel
    // cell. Anchored at the lattice's 30pt instead, 12.35 falls BETWEEN two
    // rungs and a pane that is not zooming at all pays several times that for a
    // continuity it is not using.
    let ppp = painter.ctx().pixels_per_point();
    let (raster, magnify) = crate::text::ladder(label_scale, LABEL_PT, ppp);
    // `draw_stacked_name` sizes everything off the lattice's letter, so the
    // rung crosses back into its terms here — a conversion, not a second snap.
    let scale = LABEL_PT * raster / marks::NAME_SIZE;
    for label in labels {
        marks::draw_stacked_name(
            batch,
            painter,
            label.lead,
            label.name,
            theme::text(),
            theme::well(),
            scale,
            magnify,
            // Against the LETTER's ink, not the box's centre — which is the
            // whole of issue #349's fix and the reason `NoteLabel` carries a
            // point of its own. The box is an estimate and has to stay one
            // (`plan` has no painter, and the offline render must not depend on
            // font metrics); what a reader measures the gap by is the ink, and
            // the two disagree by the estimate's error plus the letter's side
            // bearing — both of which ride the type, so both open with the
            // pitch zoom.
            marks::NameLead::Letter(label.grow),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::probe::{fresh, frame_full, painted_full, themed_at};
    use crate::{SpectralOrientation, SpectrumConfig};
    use harmonigraph_core::{NoteEvent, NoteEventKind};

    /// The window a batch of names is drawn on. Larger than [`PANE`] on both
    /// axes, so a name placed off the pane still lands in the shapes rather
    /// than being clipped away before a test can find it.
    const SCREEN: egui::Vec2 = egui::vec2(400.0, 400.0);

    /// 300 points along the time axis, 100 across pitch — the same pane the
    /// roll's tests use.
    const PANE: egui::Rect =
        egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 120.0) };

    /// A pitch axis the size a docked pane actually has — see
    /// `spectral::REFERENCE_PITCH_LEN`, which is that size and the one the
    /// type is quoted against.
    ///
    /// The pitch range cannot be zoomed under two octaves
    /// ([`PITCH_RANGE_MIN_SPAN`](crate::PITCH_RANGE_MIN_SPAN)), so across 100
    /// points a semitone is four of them — less than a name is tall. Anything
    /// about naming NEIGHBOURING pitches therefore has to be asked of a pane
    /// with room to draw them apart, or it is asking about the test fixture.
    const BIG: egui::Rect = egui::Rect {
        min: egui::pos2(10.0, 20.0),
        max: egui::pos2(310.0, 20.0 + super::super::axes::REFERENCE_PITCH_LEN),
    };

    /// The dialled size: the type at its built-in [`LABEL_PT`] and the air in
    /// front of it at its built-in [`LABEL_INSET`].
    ///
    /// A SCALE, not a pane — the pane the tests below hand it ([`PANE`]) would
    /// derive 100/860 for both. Saying one number and meaning both is what
    /// almost every test here wants, since almost none of them are about the
    /// zoom that parts the two.
    const FLAT: NameScale = NameScale { label: 1.0, air: 1.0 };

    /// A pane zoomed in: the names drawn `label` times their built-in size on a
    /// picture whose own size has not changed, so the air stays put.
    fn zoomed(label: f32) -> NameScale {
        NameScale { label, air: 1.0 }
    }

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
        turned(range, span, SpectralOrientation::Left)
    }

    /// The same pane with the names anchored on their onsets — the "Name the
    /// far end" setting, which in this Left-facing fixture is the onset and in
    /// a reversed orientation would be the leading edge.
    fn travelling(range: f32, span: f32) -> SharedState {
        let mut state = state(range, span);
        state.spectrum_config.note_names_travel = true;
        state
    }

    fn turned(range: f32, span: f32, orientation: SpectralOrientation) -> SharedState {
        let mut state = fresh();
        state.spectrum_config = SpectrumConfig {
            orientation,
            low_midi: 60.0 - range * 0.5,
            high_midi: 60.0 + range * 0.5,
            roll_seconds: span,
            roll_fraction: 1.0,
            ..SpectrumConfig::default()
        };
        state
    }

    /// The names `state` would draw at `now`, placed exactly the way
    /// [`spectral_pane`](super::super::spectral_pane) places them.
    fn labels(state: &SharedState, now: f64) -> Vec<NoteLabel> {
        labels_in(state, now, PANE)
    }

    fn labels_in(state: &SharedState, now: f64, rect: egui::Rect) -> Vec<NoteLabel> {
        let cfg = &state.spectrum_config;
        let axes = Axes::new(rect, cfg);
        let min_midi = cfg.low_midi;
        let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
        let scale = PitchScale { min_midi, max_midi, span: max_midi - min_midi };
        let split = super::super::axes::spectrum_share(cfg);
        plan(state, &axes, &scale, split, now, FLAT)
    }

    /// A phrase dense enough that its names have to compete for room: three
    /// pitches struck together every 0.9 seconds, for longer than the window
    /// holds. `from` cuts the roll's memory back to notes still sounding then,
    /// which is what a sweep anchored on the window's own edge amounts to.
    fn phrase(from: f64) -> SharedState {
        let mut state = state(24.0, 10.0);
        let mut t = 0.0;
        while t < 24.0 {
            for (i, note) in [60u8, 62, 64].iter().enumerate() {
                let at = t + i as f64 * 0.11;
                if at + 0.25 >= from {
                    state.tracker.handle_event(on(at, *note));
                    state.tracker.handle_event(off(at + 0.25, *note));
                }
            }
            t += 0.9;
        }
        state
    }

    /// How many times a name vanishes from the picture and comes back, over
    /// eight seconds of scrolling at `label_scale`. Names are followed by the
    /// NOTE each belongs to, not by where it is drawn: every name is moving,
    /// so a position says nothing about identity.
    fn blinks(state_at: impl Fn(f64) -> SharedState, label_scale: f32) -> usize {
        let mut seen: HashMap<(String, i64), Vec<usize>> = HashMap::new();
        for frame in 0..480 {
            let now = 14.0 + frame as f64 / 60.0;
            let state = state_at(now);
            let cfg = state.spectrum_config;
            let split = super::super::axes::spectrum_share(&cfg);
            let axes = Axes::new(BIG, &cfg);
            let labels =
                plan(&state, &axes, &scale_of(&state), split, now, zoomed(label_scale));
            for label in labels {
                let key = (label.name.to_string(), (label.at * 1000.0).round() as i64);
                seen.entry(key).or_default().push(frame);
            }
        }
        seen.values().map(|f| f.windows(2).filter(|w| w[1] != w[0] + 1).count()).sum()
    }

    /// The same ostinato played on and on, so that the roll reaches the cap it
    /// keeps and starts evicting its own oldest note on every release —
    /// counted over eight seconds once it is there.
    fn blinks_at_the_roll_cap() -> usize {
        let (step, voices) = (0.35, [60u8, 62, 64]);
        let mut events: Vec<(f64, u8, bool)> = Vec::new();
        let mut t = 0.0;
        while t < 520.0 {
            for note in voices {
                events.push((t, note, true));
                events.push((t + 0.15, note, false));
            }
            t += step;
        }
        events.sort_by(|a, b| a.0.total_cmp(&b.0));

        let start = 500.0;
        let mut state = state(24.0, 10.0);
        let mut next = 0;
        let feed = |state: &mut SharedState, next: &mut usize, until: f64| {
            while *next < events.len() && events[*next].0 <= until {
                let (at, note, down) = events[*next];
                state.tracker.handle_event(if down { on(at, note) } else { off(at, note) });
                *next += 1;
            }
        };
        feed(&mut state, &mut next, start);
        assert!(
            state.tracker.roll().notes().count() >= harmonigraph_core::NoteRoll::MAX_NOTES,
            "the roll has to be AT its cap for this to be the test it says it is",
        );

        let cfg = state.spectrum_config;
        let axes = Axes::new(BIG, &cfg);
        let split = super::super::axes::spectrum_share(&cfg);
        let mut seen: HashMap<(String, i64), Vec<usize>> = HashMap::new();
        for frame in 0..480 {
            let now = start + frame as f64 / 60.0;
            feed(&mut state, &mut next, now);
            for label in plan(&state, &axes, &scale_of(&state), split, now, FLAT) {
                seen.entry((label.name.to_string(), (label.at * 1000.0).round() as i64))
                    .or_default()
                    .push(frame);
            }
        }
        seen.values().map(|f| f.windows(2).filter(|w| w[1] != w[0] + 1).count()).sum()
    }

    /// A name never vanishes and comes back as the roll scrolls.
    ///
    /// Thinning has to decide which of several close repeats keeps its name,
    /// and any rule of the form "the next one with room after the last name
    /// taken" is a chain resting on wherever it began. Everything available to
    /// begin at MOVES — the window's oldest note scrolls off, the roll's own
    /// oldest is evicted at [`NoteRoll::MAX_NOTES`] — and moving it flips the
    /// parity of the whole chain behind it, which is every other name in the
    /// lane blinking out and back. So there is no chain: an absolute grid
    /// decides which note is offered a name, and nothing outside two adjacent
    /// cells of take time can reach it. See [`Lane`].
    ///
    /// Three arms, because the two anchors that moved were fixed one at a time
    /// and each has to stay fixed. The first two scroll a window across a
    /// roll that comfortably holds everything; the third plays on until the
    /// roll is evicting a note for every one it takes, which is the case that
    /// survived the first fix — 882 blinks over these same eight seconds, at
    /// the dialled size and with no zoom in it at all.
    #[test]
    fn a_name_never_blinks_out_and_back_as_the_roll_scrolls() {
        const ZOOMED: f32 = 2.23;
        // Vacuity guard: names must actually be competing here, or "nothing
        // blinked" is a statement about a pane with nothing to thin.
        let state = phrase(f64::NEG_INFINITY);
        let cfg = state.spectrum_config;
        let split = super::super::axes::spectrum_share(&cfg);
        let axes = Axes::new(BIG, &cfg);
        let placed = plan(&state, &axes, &scale_of(&state), split, 20.0, zoomed(ZOOMED));
        let on_pane = state
            .tracker
            .roll()
            .notes()
            .filter(|note| note.stop(20.0) >= 20.0 - cfg.roll_seconds as f64)
            .count();
        assert!(
            placed.len() < on_pane,
            "{} notes on the pane and {} names: nothing is being thinned",
            on_pane,
            placed.len(),
        );

        assert_eq!(blinks(|_| phrase(f64::NEG_INFINITY), ZOOMED), 0);
        assert_eq!(blinks(|_| phrase(f64::NEG_INFINITY), 1.0), 0, "...and at the dialled size");
        assert_eq!(blinks_at_the_roll_cap(), 0, "...and with the roll evicting as it plays");
    }

    fn said(labels: &[NoteLabel]) -> Vec<String> {
        labels.iter().map(|l| l.name.to_string()).collect()
    }

    fn scale_of(state: &SharedState) -> PitchScale {
        let cfg = &state.spectrum_config;
        let min_midi = cfg.low_midi;
        let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
        PitchScale { min_midi, max_midi, span: max_midi - min_midi }
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

    /// A name stands the same distance off the end it is written on however
    /// far the pitch range is zoomed in.
    ///
    /// The zoom grows a name in proportion, so that it keeps its footing on a
    /// ribbon which is growing by the same factor
    /// ([`name_zoom`](super::super::axes::name_zoom)) — and the gap in front of
    /// it went up with the rest, which is a name sliding down its own roll for
    /// as long as the range is being dragged. The type still follows the zoom;
    /// the join between the name and the note does not. See [`LABEL_INSET`].
    ///
    /// Measured to the estimated ink and not to the box [`label_rect`] returns,
    /// the two differing by [`LABEL_PAD`] — clear space the thinning asks for,
    /// which does scale with the type. (The pad is not invisible everywhere: a
    /// name held off the near edge is clamped by its PADDED corner, so a
    /// just-struck note's name at the onset anchor stands the pad clear of the
    /// now-line. It is invisible here, where nothing is clamped.)
    ///
    /// The estimate, note, and not the glyph egui draws. The two are placed off
    /// the same anchor by the same inset but measure different things, so this
    /// cannot see where the ink lands —
    /// [`a_names_letter_stands_the_same_distance_off_its_note_at_every_zoom`]
    /// is that reading, and [`LABEL_INSET`] explains the split.
    ///
    /// Both growth directions, since which one a name has is the anchor's and
    /// not the orientation's — every orientation draws both (see [`Anchor`]).
    #[test]
    fn a_name_keeps_its_distance_from_its_note_through_the_zoom() {
        let cfg = SpectrumConfig::default();
        let axes = Axes::new(PANE, &cfg);
        let name = NoteName { letter: 'C', sharps: 0, syntonic_commas: 0, septimal_commas: 0 };
        let anchor = axes.at(0.5, 0.5);
        for grow in [axes.dir_depth(), -axes.dir_depth()] {
            for zoom in [1.0, 2.23, 5.0] {
                let size = LABEL_PT * zoom;
                let rect = label_rect(&axes, grow, 0.5, 0.5, &name, size, zoomed(zoom));
                let ink = egui::Rect::from_center_size(rect.center(), name_extent(&name, size));
                // The end of the ink nearest the anchor, measured the way the
                // box grows — so this names no screen side and reads the same
                // in both directions.
                let reach = |p: egui::Pos2| (p - anchor).dot(grow);
                let gap = reach(ink.min).min(reach(ink.max));
                assert!(
                    (gap - LABEL_INSET).abs() < 0.01,
                    "growing {grow:?} at zoom {zoom}: the name sits {gap} off its note, not \
                     {LABEL_INSET}",
                );
            }
        }
    }

    /// ...and the pane is the one thing that DOES move it, because the Render
    /// preview and the video it previews have to be one picture at two sizes.
    #[test]
    fn a_name_keeps_its_distance_as_a_fraction_of_the_pane() {
        let cfg = SpectrumConfig::default();
        let axes = Axes::new(PANE, &cfg);
        let name = NoteName { letter: 'C', sharps: 0, syntonic_commas: 0, septimal_commas: 0 };
        let anchor = axes.at(0.5, 0.5);
        let grow = axes.dir_depth();
        let gap = |air: f32| {
            let rect = label_rect(&axes, grow, 0.5, 0.5, &name, LABEL_PT, NameScale { label: 1.0, air });
            let ink = egui::Rect::from_center_size(rect.center(), name_extent(&name, LABEL_PT));
            let reach = |p: egui::Pos2| (p - anchor).dot(grow);
            reach(ink.min).min(reach(ink.max))
        };
        assert!((gap(0.5) - LABEL_INSET * 0.5).abs() < 0.01, "half a pane, half the air");
        assert!((gap(2.0) - LABEL_INSET * 2.0).abs() < 0.01, "twice the pane, twice the air");
    }

    /// The LETTER lands in the same place whether or not its name carries an
    /// accidental — in every orientation, not only the ones where the box
    /// happens to grow the same way the letter is typeset.
    ///
    /// Right's leftward time is the one where the two disagree: the box grows
    /// away from the leading edge, but [`draw_stacked_name`] always sets the
    /// letter first and the accidental after it, so growing away would drag
    /// the letter along with however wide the accidental happens to be.
    /// `rect.min.x` is where that letter lands (see
    /// [`a_name_sits_on_its_ribbon_at_the_leading_edge`]), so that is what has
    /// to agree between a plain letter and one carrying a mark.
    ///
    /// Asked of BOTH growth directions in each orientation, because the
    /// orientation does not decide which one a pane is in: a name anchored at
    /// the onset ([`Anchor::Onset`]) grows back against the depth axis, so
    /// every orientation has a leftward case somewhere in it.
    ///
    /// [`draw_stacked_name`]: crate::marks::draw_stacked_name
    #[test]
    fn the_letter_lines_up_with_or_without_an_accidental() {
        let plain = NoteName { letter: 'C', sharps: 0, syntonic_commas: 0, septimal_commas: 0 };
        let sharp = NoteName { letter: 'C', sharps: 1, syntonic_commas: 0, septimal_commas: 0 };
        for orientation in [SpectralOrientation::Left, SpectralOrientation::Right] {
            let cfg = SpectrumConfig { orientation, ..SpectrumConfig::default() };
            let axes = Axes::new(PANE, &cfg);
            for grow in [axes.dir_depth(), -axes.dir_depth()] {
                let plain_rect = label_rect(&axes, grow, 0.5, 0.5, &plain, 12.0, FLAT);
                let sharp_rect = label_rect(&axes, grow, 0.5, 0.5, &sharp, 12.0, FLAT);
                assert!(
                    (plain_rect.min.x - sharp_rect.min.x).abs() < 0.01,
                    "{orientation:?} growing {grow:?}: C's letter at {} but C♯'s at {}",
                    plain_rect.min.x,
                    sharp_rect.min.x,
                );
            }
        }
    }

    /// The same claim as
    /// [`the_letter_lines_up_with_or_without_an_accidental`], but read off
    /// the glyphs [`draw`] actually queues through a real `egui::Context`, so
    /// it is about the letters a reader sees rather than about the boxes they
    /// were chosen in.
    ///
    /// What it holds is that the mark column cannot reach the letter's
    /// placement: the two names differ by a comma sign, and the drawn letter
    /// does not move. The arithmetic-only test asks that of `label_rect`, where
    /// the extent is what decides it; here nothing consults the extent at all
    /// — the lead is a point and [`marks::NameLead::Letter`] measures the glyph
    /// — so the two are the same sentence proved of two different mechanisms.
    #[test]
    fn the_drawn_letter_lines_up_across_notes_in_right_orientation() {
        let cfg =
            SpectrumConfig { orientation: SpectralOrientation::Right, ..SpectrumConfig::default() };
        let axes = Axes::new(PANE, &cfg);
        let names = [
            NoteName { letter: 'C', sharps: 0, syntonic_commas: 0, septimal_commas: 0 },
            NoteName { letter: 'E', sharps: 0, syntonic_commas: 1, septimal_commas: 0 },
        ];
        let labels: Vec<NoteLabel> = names
            .iter()
            .map(|&name| NoteLabel {
                name,
                rect: label_rect(&axes, axes.dir_depth(), 0.5, 0.5, &name, LABEL_PT, FLAT),
                lead: axes.at(0.5, 0.5) + axes.dir_depth() * (LABEL_INSET * FLAT.air),
                grow: axes.dir_depth(),
                #[cfg(test)]
                at: 0.0,
            })
            .collect();

        let mut batch = crate::text::TextBatch::default();
        let _ = painted_full(SCREEN, |ui| draw(ui.painter(), &labels, 1.0, &mut batch));

        let left_of = |letter: &str| {
            batch
                .pieces()
                .iter()
                .find(|p| p.text == letter)
                .unwrap_or_else(|| panic!("no {letter:?} drawn, got {:?}", batch.pieces()))
                .galley
                .left()
        };
        let (c, e) = (left_of("C"), left_of("E"));
        assert!((c - e).abs() < 0.5, "C's letter drawn at {c} but E's (with a comma) at {e}");
    }

    /// A name sits on the ribbon the ROLL DREW, read from the roll's own
    /// geometry rather than recomputed here.
    ///
    /// Hand-computing the expected edge is how this last went wrong: the Gap
    /// setting shaved a released note's tail back and the name kept anchoring
    /// on the unshaved stop, so it sat off the head of its own ribbon — and
    /// the test could not see it, because the test's arithmetic agreed with
    /// `names`' arithmetic and both disagreed with the roll. Gap is gone, so
    /// the two ends agree again by construction; this reads `note_instances`
    /// anyway, which is what would catch the next thing to move a ribbon's
    /// head without telling the name.
    #[test]
    fn a_name_sits_on_the_ribbon_the_roll_drew() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(2.0, 60));
        state.tracker.handle_event(off(6.0, 60));

        let split = super::super::axes::spectrum_share(&state.spectrum_config);
        let axes = Axes::new(PANE, &state.spectrum_config);
        let ribbon =
            super::super::roll::note_instances(&axes, &scale_of(&state), &state, split, 10.0, 2.0);
        assert_eq!(ribbon.len(), 1, "one note, one ribbon");
        // Horizontal pane: depth is x, and the head is the near end.
        let head = ribbon[0].center[0] - ribbon[0].half_extent[1];

        let placed = labels(&state, 10.0);
        assert_eq!(placed.len(), 1);
        assert!(
            placed[0].rect.min.x >= head,
            "the name starts at {} but its ribbon only begins at {head}",
            placed[0].rect.min.x,
        );
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

    /// One press puts ONE name on the roll, even delivered the way a host
    /// really delivers it: an on, an off and a second on all stamped at the
    /// same sample.
    ///
    /// The off/on pair in the middle otherwise leaves a roll entry that begins
    /// and ends together, and it is the NAME that shows rather than the
    /// ribbon — a ribbon of no length is floored to a couple of pixels and
    /// reads as grain, while the name on it is full size and anchored where it
    /// ended. So a single press put a second letter on the pane that scrolled
    /// away from the letter held at the now-line, as though the key had been
    /// played twice. Asked here rather than only of the roll because the roll
    /// entry is the cause and this is the symptom.
    #[test]
    fn one_press_is_named_once_however_the_host_delivers_it() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(1.0, 60));
        state.tracker.handle_event(off(1.0, 60));
        state.tracker.handle_event(on(1.0, 60));

        let early = labels(&state, 1.5);
        assert_eq!(said(&early), ["C"], "one press, one name");
        let later = labels(&state, 3.0);
        assert_eq!(said(&later), ["C"]);
        assert_eq!(
            early[0].rect.min.x, later[0].rect.min.x,
            "and it holds the now-line while the key is down, rather than travelling",
        );
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

    /// A TRAVELLING name is moving from the first frame of its note, and the
    /// release is not an event in its life at all.
    ///
    /// The other anchor ([`Anchor::Onset`]), and the whole of what it is for.
    /// Where the leading edge holds a held note's name at the now-line and
    /// starts it moving at the key-up — a movement nothing in the music made —
    /// this pins the name to the moment the key went DOWN, which is a fact
    /// about the take and stops changing the instant it happens.
    ///
    /// Both halves are asserted, because either alone is met by something
    /// wrong: a name that moves but jumps at the release is the defect this
    /// replaces, and one that never moves is the leading edge again. The
    /// release is compared against a state where the key is still down at the
    /// same moment, so what is proved is that the name cannot tell.
    #[test]
    fn a_travelling_name_starts_moving_at_once_and_the_release_is_not_an_event() {
        let played = |release: Option<f64>| {
            let mut state = travelling(24.0, 10.0);
            state.tracker.handle_event(on(1.0, 60));
            if let Some(t) = release {
                state.tracker.handle_event(off(t, 60));
            }
            state
        };
        let at = |state: &SharedState, now| labels(state, now)[0].rect.min.x;

        let held = played(None);
        assert!(at(&held, 4.0) > at(&held, 2.0), "held, the name is already travelling");
        assert!(at(&held, 6.0) > at(&held, 4.0), "...and keeps travelling");

        let released = played(Some(4.0));
        for now in [4.5, 6.0, 8.0] {
            assert_eq!(
                at(&released, now),
                at(&held, now),
                "at {now}s the name moved because the key came up",
            );
        }
    }

    /// A travelling name lies over its own ribbon, which live is the picture
    /// BEHIND the onset — the opposite screen direction from the one a name at
    /// the leading edge grows in.
    ///
    /// The direction is the anchor's, not the layout's, and getting it from the
    /// layout would put every travelling name past the tail of its own note and
    /// over whatever is older than it.
    #[test]
    fn a_travelling_name_lies_over_its_ribbon_toward_the_now_line() {
        let mut state = travelling(24.0, 10.0);
        state.tracker.handle_event(on(2.0, 60));
        state.tracker.handle_event(off(6.0, 60));

        let placed = labels(&state, 10.0);
        assert_eq!(placed.len(), 1);
        let axes = Axes::new(PANE, &state.spectrum_config);
        // Horizontal: depth is x with now at the left, so the ribbon runs from
        // the onset (8s back, depth 0.8) to the release (4s back, depth 0.4)
        // and the name is written at the onset, growing back toward now.
        let onset = axes.at(0.5, 0.8);
        let rect = placed[0].rect;
        assert!(
            (rect.center().y - onset.y).abs() < 1.0,
            "the name is centred on the note's own line, not lifted off it",
        );
        assert!(rect.max.x <= onset.x, "it ends at the onset");
        assert!(rect.max.x > onset.x - 2.0 * LABEL_INSET, "...and right at it");
        assert!(rect.min.x > axes.at(0.5, 0.4).x, "growing into the note, not past its head");
    }

    /// A name lies over its OWN ribbon, from the end it is anchored to, at
    /// either anchor and in every orientation.
    ///
    /// The direction the box grows in is the anchor's rather than the layout's,
    /// and the two point opposite ways live — so taking it from the layout lays
    /// a travelling name over the picture BEHIND its note instead of over the
    /// note. Read by projecting onto the depth axis, since nothing here may
    /// name a screen side, and swept over [`SpectralOrientation::ALL`] so a
    /// fifth orientation cannot skip it.
    ///
    /// WHICH end each pass expects is [`Anchor::of`]'s rule restated: reading
    /// order picks one and the setting asks for the other, so the two swap in a
    /// reversed orientation. Restating it is the point — a mapping hardcoded
    /// here would agree with the code in half the sweep and be checked by
    /// neither assertion, both of which only bracket the name between the
    /// ribbon's ends. The gap asserted last is what makes it bite: written on
    /// the wrong end, a name stands a ribbon's length from the one it is
    /// measured against rather than [`LABEL_INSET`].
    #[test]
    fn a_name_lies_over_its_own_ribbon_at_either_anchor() {
        // A plain `C`, whose box does not overrun its anchor: a name carrying
        // marks does, by up to 17 points, and that is the pinning trade
        // measured in [`label_rect`] rather than anything about the anchor.
        for orientation in SpectralOrientation::ALL {
            for travel in [false, true] {
                let mut state = turned(24.0, 10.0, orientation);
                state.spectrum_config.note_names_travel = travel;
                state.tracker.handle_event(on(2.0, 60));
                state.tracker.handle_event(off(6.0, 60));

                // Square, so the same pane serves the vertical orientations.
                let square =
                    egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 320.0) };
                let placed = labels_in(&state, 10.0, square);
                assert_eq!(placed.len(), 1, "{orientation:?}, travel {travel}");

                // The ribbon's two ends, and how far the name sits from the one
                // it is anchored to along the depth axis — signed, so a name
                // laid the wrong way reads as a negative reach.
                let axes = Axes::new(square, &state.spectrum_config);
                let t = scale_of(&state).t_of(60.0);
                let (head, onset) = (axes.at(t, 0.4), axes.at(t, 0.8));
                // The leading edge reads first where time runs the screen's own
                // way, the onset where it runs back against it; the setting
                // asks for the other end of whichever that is.
                let on_head = orientation.is_time_reversed() == travel;
                let (anchor, other) = if on_head { (head, onset) } else { (onset, head) };
                let toward = (other - anchor).normalized();
                let reach = (placed[0].rect.center() - anchor).dot(toward);
                assert!(
                    reach > 0.0,
                    "{orientation:?}, travel {travel}: the name lies off the far side of \
                     its anchor, {reach} points from it",
                );
                assert!(
                    reach < (other - anchor).length(),
                    "{orientation:?}, travel {travel}: the name overruns the far end of \
                     its own ribbon",
                );
                let gap = (placed[0].lead - anchor).dot(toward);
                assert!(
                    (gap - LABEL_INSET).abs() < 0.01,
                    "{orientation:?}, travel {travel}: the letter stands {gap} off the end \
                     it is written on, not {LABEL_INSET} — so it is on the other end",
                );
            }
        }
    }

    /// A name reaches over the SPECTRUM rather than let go of the end it is
    /// written on — and stops at the pane's own edge, which is the only thing
    /// that does hold it.
    ///
    /// A name written on the end that reaches the present grows toward the
    /// now-line, and a note younger than its own name has no ribbon yet to fill
    /// it, so the box crosses the divider and lies over the spectrum's curve.
    /// That is the picture: the gap between the letter and the end it names is
    /// what a reader reads a name by, and a name stopped at the divider instead
    /// would stand still for those first moments while its own note scrolled
    /// out from under it.
    ///
    /// What still holds it is the PANE, past which the batch is clipped and the
    /// picture is another pane's. Every other fixture in this file gives the
    /// roll the whole pane (`roll_fraction: 1.0`, so `split` is 0 and the two
    /// edges are the same line), which is exactly where the two cannot be told
    /// apart. This one keeps the fresh 0.55 to part them.
    #[test]
    fn a_name_crosses_the_spectrum_but_never_leaves_the_pane() {
        // The crossing, at the anchor that grows toward the now-line: Left's
        // far end, which is the onset.
        let mut state = travelling(24.0, 10.0);
        state.spectrum_config.roll_fraction = 0.55; // the fresh value
        state.tracker.handle_event(on(5.0, 60));

        let axes = Axes::new(PANE, &state.spectrum_config);
        let split = super::super::axes::spectrum_share(&state.spectrum_config);
        // Left: depth is x with the now-line at the roll's near edge, so the
        // spectrum owns everything left of it, and the pane's own edge is the
        // far side of that.
        let divider = axes.at(0.5, split).x;
        let struck = labels(&state, 5.0);
        assert_eq!(struck.len(), 1);
        assert!(
            struck[0].rect.min.x < divider,
            "struck this instant, the name starts at {} rather than crossing the divider \
             at {divider} onto the spectrum",
            struck[0].rect.min.x,
        );
        // ...and it holds its gap from the onset from that first frame, which
        // is the whole reason it is allowed to cross.
        for now in [5.0, 5.05, 5.1, 5.2] {
            let placed = labels(&state, now);
            assert_eq!(placed.len(), 1, "at {now}s");
            let time = TimeAxis::new(&state, split, now);
            let onset = axes.at(scale_of(&state).t_of(60.0), time.depth_of(5.0));
            let gap = (placed[0].lead - onset).dot(placed[0].grow);
            assert!(
                (gap - LABEL_INSET).abs() < 0.01,
                "at {now}s the name stands {gap} off its onset, not {LABEL_INSET}",
            );
        }

        // The pane's edge, where the whole depth axis is the roll's and there
        // is nothing between the now-line and the outside.
        let mut state = travelling(24.0, 10.0);
        state.tracker.handle_event(on(5.0, 60));
        for now in [5.0, 5.05, 5.1, 5.2] {
            let placed = labels(&state, now);
            assert_eq!(placed.len(), 1, "at {now}s");
            assert!(
                placed[0].rect.min.x >= PANE.left(),
                "at {now}s the name reaches to {} where the pane only begins at {}",
                placed[0].rect.min.x,
                PANE.left(),
            );
        }

        // ...and the same edge in a REVERSED orientation with a spectrum
        // present, which is the case the two above cannot tell apart: with the
        // roll given all but a sliver of the axis the analyzer is narrower than
        // a name, so a name struck this instant crosses what there is of it and
        // meets the pane. `split` is 0.02 here rather than 0, so a clamp still
        // measuring against the divider would leave the name 6 points out.
        let mut state = turned(24.0, 10.0, SpectralOrientation::Right);
        state.spectrum_config.roll_fraction = 0.98;
        state.tracker.handle_event(on(5.0, 60));
        let axes = Axes::new(PANE, &state.spectrum_config);
        let split = super::super::axes::spectrum_share(&state.spectrum_config);
        let t = scale_of(&state).t_of(60.0);
        let divider = axes.at(t, split).x;
        let placed = labels(&state, 5.0);
        assert_eq!(placed.len(), 1);
        assert!(
            placed[0].rect.max.x <= PANE.right() + 0.01,
            "the name reaches to {} where the pane ends at {}",
            placed[0].rect.max.x,
            PANE.right(),
        );
        assert!(
            placed[0].rect.max.x > divider,
            "the clamp pulled the name back onto the roll at {} rather than leaving it \
             over what analyzer there is, past {divider}",
            placed[0].rect.max.x,
        );
    }

    /// With the spectrum on the RIGHT, a name keeps its gap from the note's
    /// LEFT end — the onset there — from the note's very first frame, and
    /// travels with it from that frame on.
    ///
    /// This is the mirror of what the Left orientation does with the leading
    /// edge, and it is the point of choosing the anchor by reading order: the
    /// distance a reader measures, letter to ribbon end, is one distance in
    /// both. It costs the crossing — at the strike the onset IS the now-line,
    /// so the name is written wholly over the analyzer and slides off it as the
    /// note scrolls away.
    #[test]
    fn with_the_spectrum_on_the_right_a_name_holds_the_notes_left_end() {
        let mut state = turned(24.0, 10.0, SpectralOrientation::Right);
        state.spectrum_config.roll_fraction = 0.55; // the fresh value
        state.tracker.handle_event(on(5.0, 60));

        let axes = Axes::new(PANE, &state.spectrum_config);
        let split = super::super::axes::spectrum_share(&state.spectrum_config);
        let t = scale_of(&state).t_of(60.0);
        // Right: time runs leftward, so the analyzer owns everything right of
        // the divider and the note grows away from it.
        let divider = axes.at(t, split).x;

        let struck = labels(&state, 5.0);
        assert_eq!(said(&struck), ["C"]);
        assert!(
            struck[0].lead.x >= divider,
            "struck this instant, the name's letter is at {} rather than out on the \
             analyzer past {divider}",
            struck[0].lead.x,
        );

        let mut previous = f32::INFINITY;
        for now in [5.0, 5.1, 5.4, 6.0, 8.0] {
            let placed = labels(&state, now);
            assert_eq!(placed.len(), 1, "at {now}s");
            let time = TimeAxis::new(&state, split, now);
            let onset = axes.at(t, time.depth_of(5.0));
            let gap = (placed[0].lead - onset).dot(placed[0].grow);
            assert!(
                (gap - LABEL_INSET).abs() < 0.01,
                "at {now}s the name stands {gap} off the note's left end, not {LABEL_INSET}",
            );
            assert!(
                placed[0].lead.x < previous,
                "at {now}s the name is at {} rather than left of where it was ({previous})",
                placed[0].lead.x,
            );
            previous = placed[0].lead.x;
        }
    }

    /// Whichever end of a ribbon READS first is the end named, in every
    /// orientation — and the name stands the same [`LABEL_INSET`] off it, with
    /// the note running away under the rest of the name.
    ///
    /// The sweep is the point: this is the one thing in the module that names a
    /// screen side, so it is asked of all four rather than of the two a person
    /// happens to use, and a fifth orientation cannot skip it.
    #[test]
    fn a_name_is_written_on_the_end_that_reads_first() {
        for orientation in SpectralOrientation::ALL {
            let mut state = turned(24.0, 10.0, orientation);
            state.tracker.handle_event(on(2.0, 60));
            state.tracker.handle_event(off(6.0, 60));

            // Square, so the same pane serves the vertical orientations.
            let square = egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 320.0) };
            let placed = labels_in(&state, 10.0, square);
            assert_eq!(placed.len(), 1, "{orientation:?}");

            // The ribbon's two ends — the release 4s back, the onset 8s back —
            // and the way a reader's eye runs over the pane: rightward where
            // time is across it, downward where time runs down it.
            let axes = Axes::new(square, &state.spectrum_config);
            let t = scale_of(&state).t_of(60.0);
            let (head, onset) = (axes.at(t, 0.4), axes.at(t, 0.8));
            let reading = if orientation.is_time_vertical() {
                egui::vec2(0.0, 1.0)
            } else {
                egui::vec2(1.0, 0.0)
            };
            let (first, second) =
                if (head - onset).dot(reading) < 0.0 { (head, onset) } else { (onset, head) };

            let lead = placed[0].lead;
            let gap = (lead - first).dot(reading);
            assert!(
                (gap - LABEL_INSET).abs() < 0.01,
                "{orientation:?}: the letter stands {gap} off the end that reads first, \
                 not {LABEL_INSET}",
            );
            assert!(
                (lead - second).dot(reading) < 0.0,
                "{orientation:?}: the name is written on the end that reads SECOND",
            );
        }
    }

    /// The ink [`plan`] finally puts on the pane stands [`LABEL_INSET`] off the
    /// ribbon end — and, where the clamp fires, stays on the pane.
    ///
    /// Everything else that reads the drawn glyphs builds its [`NoteLabel`] by
    /// hand, which means it restates `plan`'s own arithmetic rather than
    /// checking it: the lead can be taken straight off the anchor with the inset
    /// dropped, or the clamp can be left off it while still moving the box, and
    /// every one of those tests goes on passing. Both were tried. This is the
    /// one that goes end to end, so it is the one that fails.
    ///
    /// The second half is the clamp's, and it is the half a box cannot answer.
    /// A name growing toward the now-line is held on the pane while its note is
    /// younger than its own name, and it is the BOX that is measured against
    /// that edge; the ink inside it is what a reader sees leaving the picture.
    /// `a_name_crosses_the_spectrum_but_never_leaves_the_pane` asserts on the
    /// box, so a lead left uncorrected there draws the letter off the pane with
    /// that test still green.
    #[test]
    fn the_ink_plan_places_stands_off_its_ribbon_and_inside_the_pane() {
        const PPP: f32 = 2.0;
        let ctx = themed_at(PPP);
        // The letter's ink, drawn exactly as the pane draws it, projected onto
        // the way the name runs from `from`.
        let ink_from = |label: &NoteLabel, from: egui::Pos2| {
            let mut batch = crate::text::TextBatch::default();
            let _ = frame_full(&ctx, SCREEN, |ui| {
                draw(ui.painter(), std::slice::from_ref(label), FLAT.label, &mut batch)
            });
            let letter = label.name.letter.to_string();
            let ink = batch
                .pieces()
                .iter()
                .find(|p| p.text == letter)
                .unwrap_or_else(|| panic!("no {letter} drawn"))
                .ink;
            let corners =
                [ink.left_top(), ink.right_top(), ink.left_bottom(), ink.right_bottom()];
            corners.iter().map(|&c| (c - from).dot(label.grow)).fold(f32::INFINITY, f32::min)
        };

        // Nothing clamped: a released note, named on its leading edge.
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(2.0, 60));
        state.tracker.handle_event(off(6.0, 60));
        let placed = labels(&state, 10.0);
        assert_eq!(placed.len(), 1);
        let axes = Axes::new(PANE, &state.spectrum_config);
        // The release, 4 seconds back of a 10-second window, at middle C.
        let gap = ink_from(&placed[0], axes.at(0.5, 0.4));
        assert!(
            (gap - LABEL_INSET).abs() < 0.1,
            "the drawn letter stands {gap} off the end it is written on, not {LABEL_INSET}",
        );

        // Clamped: struck this instant at the far anchor, so the name is longer
        // than the ribbon under it and is held on the pane. The roll has the
        // whole depth axis here (`roll_fraction` 1.0), so the pane's own edge
        // is where the now-line is and there is nothing between them.
        let mut state = travelling(24.0, 10.0);
        state.tracker.handle_event(on(5.0, 60));
        for now in [5.0, 5.05, 5.1] {
            let placed = labels(&state, now);
            assert_eq!(placed.len(), 1, "at {now}s");
            let mut batch = crate::text::TextBatch::default();
            let _ = frame_full(&ctx, SCREEN, |ui| {
                draw(ui.painter(), &placed, FLAT.label, &mut batch)
            });
            let ink = batch.pieces().iter().find(|p| p.text == "C").expect("a C drawn").ink;
            assert!(
                ink.left() >= PANE.left(),
                "at {now}s the drawn letter reaches to {} where the pane only begins at \
                 {}, so it is written off it",
                ink.left(),
                PANE.left(),
            );
        }
    }

    /// A travelling name is thinned like any other — a held note has no
    /// exemption once its name is anchored somewhere that holds still.
    ///
    /// The exemption is for a name standing at the now-line while the picture
    /// scrolls past it, which is what the OTHER anchor does. Kept here it would
    /// hand a held note a name the thinning had no room for and take it away
    /// again at the release, which is the one moment this anchor exists to make
    /// uneventful.
    #[test]
    fn a_travelling_name_is_thinned_like_any_other_and_stays_thinned() {
        // The same pitch twice, the second following close enough that its name
        // has nowhere clear to go, and held.
        let played = |travel: bool, release: Option<f64>| {
            let mut state = if travel { travelling(24.0, 10.0) } else { state(24.0, 10.0) };
            state.tracker.handle_event(on(1.0, 60));
            state.tracker.handle_event(off(1.05, 60));
            state.tracker.handle_event(on(1.1, 60));
            if let Some(t) = release {
                state.tracker.handle_event(off(t, 60));
            }
            state
        };
        assert_eq!(
            labels(&played(false, None), 2.0).len(),
            2,
            "at the leading edge the held note is named however crowded it is",
        );
        let travelling = labels(&played(true, None), 2.0);
        assert_eq!(travelling.len(), 1, "travelling, it waits for room like everything else");

        // ...and the answer does not change when the key comes up: same name,
        // same place, whether it is still down or was released a second ago.
        let after = labels(&played(true, Some(1.5)), 2.5);
        let still_down = labels(&played(true, None), 2.5);
        assert_eq!(said(&after), said(&still_down));
        assert_eq!(after[0].rect.min.x, still_down[0].rect.min.x);
    }

    /// One pitch sounded by TWO voices is named once at either anchor, and one
    /// press delivered as on/off/on is still one press.
    ///
    /// Both are the same question asked of the two anchors, and each answers it
    /// somewhere else. At the leading edge a doubled MIDI source is caught by
    /// an explicit check, held names being outside the thinning; at the onset
    /// it falls to the grid, since two voices struck together share a cell and
    /// the second is refused. Worth pinning because the second reading rests on
    /// the grid doing a job nothing asked it to do, and a fourth sort key or a
    /// per-note cell would quietly take it away.
    #[test]
    fn one_pitch_from_two_voices_is_named_once_at_either_anchor() {
        let voiced = |travel: bool| {
            let mut state = if travel { travelling(24.0, 10.0) } else { state(24.0, 10.0) };
            // The same pitch on two channels, struck together and held — a
            // doubled source, or one MPE part layered over another.
            for channel in [0, 1] {
                state.tracker.handle_event(NoteEvent {
                    time: 1.0,
                    channel,
                    note: 60,
                    kind: NoteEventKind::On { velocity: 0.8 },
                });
            }
            state
        };
        assert_eq!(said(&labels(&voiced(false), 2.0)), ["C"], "held, at the leading edge");
        assert_eq!(said(&labels(&voiced(true), 2.0)), ["C"], "and travelling, through the grid");

        // ...and a press the host delivers as on/off/on at one sample is one
        // press at either anchor, the two entries sharing an onset.
        let pressed = |travel: bool| {
            let mut state = if travel { travelling(24.0, 10.0) } else { state(24.0, 10.0) };
            state.tracker.handle_event(on(1.0, 60));
            state.tracker.handle_event(off(1.0, 60));
            state.tracker.handle_event(on(1.0, 60));
            state
        };
        assert_eq!(said(&labels(&pressed(false), 1.5)), ["C"]);
        assert_eq!(said(&labels(&pressed(true), 1.5)), ["C"]);
    }

    /// A name leaves with the END IT IS WRITTEN ON, and never outlasts its
    /// ribbon — in every orientation.
    ///
    /// Swept over [`SpectralOrientation::ALL`] because the orientation is what
    /// picks the anchor (see [`Anchor::of`]): the two that agree with the screen
    /// name a ribbon's leading edge, the two that reverse it name the onset. The
    /// two answer differently on purpose, and the difference IS the note's
    /// length. A leading edge is the last of a note to leave, so its name goes
    /// when the ribbon goes. An onset leaves first, so its name goes a note's
    /// length earlier and the rest of the ribbon scrolls unnamed — which is what
    /// a fixed gap costs, and is the whole trade [`place`](plan) makes. Held
    /// back at the edge instead, the name would stand still in a moving picture
    /// with the gap opening behind it.
    ///
    /// Two lengths, because that shortfall IS the note's length: a test on one
    /// note cannot tell an anchor that leaves with its end from one that has
    /// simply been shifted.
    ///
    /// The slack is a second either way, against note lengths of 4 and 12: what
    /// it covers is the ink each end carries past its own box — the name's inset
    /// and the ribbon's outline — and what it cannot cover is a name on the
    /// other end of its note.
    ///
    /// A SQUARE pane, so that the depth axis is one length in all four
    /// orientations. A name's own reach along it is not: text runs across the
    /// screen, so a name is its width deep where time runs across the pane and a
    /// whole LINE BOX deep where time runs down it, twice as much. On the wide
    /// pane the rest of this file uses, that is two and a half seconds of a
    /// ten-second window — longer than a short note — and the sweep would be
    /// measuring the shape of the type rather than which end a name is on.
    #[test]
    fn a_name_leaves_with_the_end_it_is_written_on_and_never_outlasts_its_ribbon() {
        let square = egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 320.0) };
        for orientation in SpectralOrientation::ALL {
            for length in [4.0f64, 12.0] {
                let mut state = turned(24.0, 10.0, orientation);
                state.tracker.handle_event(on(1.0, 60));
                state.tracker.handle_event(off(1.0 + length, 60));

                let cfg = &state.spectrum_config;
                let axes = Axes::new(square, cfg);
                let split = super::super::axes::spectrum_share(cfg);
                let scale = scale_of(&state);
                // The last moment each survives, walked at a step fine enough
                // that the two cannot be a step apart by rounding alone.
                let (mut named, mut drawn) = (f64::NAN, f64::NAN);
                let mut now = 1.0;
                while now < 40.0 {
                    if !labels_in(&state, now, square).is_empty() {
                        named = now;
                    }
                    let ribbons = super::super::roll::note_instances(
                        &axes, &scale, &state, split, now, 2.0,
                    );
                    if !ribbons.is_empty() {
                        drawn = now;
                    }
                    now += 0.02;
                }
                // Restated from [`Anchor::of`]'s rule rather than mapped from a
                // list of orientations, so that a fifth one cannot be added
                // against a table nobody remembers to extend. The setting is off
                // here, so the anchor is whichever end reads first.
                let expected = if orientation.is_time_reversed() { length } else { 0.0 };
                let over = drawn - named;
                assert!(
                    over >= -0.001,
                    "{orientation:?}, a {length} s note: the name outlasted its own ribbon \
                     by {} s (name {named}, ribbon {drawn})",
                    -over,
                );
                assert!(
                    (over - expected).abs() < 1.0,
                    "{orientation:?}, a {length} s note: the name went {over} s before the \
                     ribbon did, where the end it is written on leaves {expected} s before \
                     it (name {named}, ribbon {drawn})",
                );
            }
        }
    }

    /// A travelling name goes off the far edge WITH the onset it is written on,
    /// sliding out under the pane rather than stopping against it.
    ///
    /// The moment the onset crosses is the one to watch, and there are three
    /// wrong things a name can do at it. It can POP — drawn whole one frame and
    /// gone the next, which is what culling on the anchor did. It can PARK —
    /// held on the edge while its own note goes on scrolling out from under it,
    /// the gap opening by the whole length of ribbon still showing. Or it can
    /// stay for ever, which for a drone is a name minutes from the note it was
    /// written on. What it should do is leave the way a ribbon does: cut by the
    /// pane's own edge, over its own length of scrolling.
    ///
    /// So the crossing is not asserted at a hardcoded moment — the last frame
    /// the name is drawn is MEASURED, and what is asserted is where that frame
    /// falls and what the picture looks like there. That keeps the test honest
    /// if [`LABEL_INSET`] or the type size is ever retuned, which move the exact
    /// moment and none of the three claims.
    #[test]
    fn a_travelling_name_leaves_the_pane_with_the_onset_it_is_written_on() {
        let mut state = travelling(24.0, 10.0);
        state.tracker.handle_event(on(0.0, 67)); // still held, and never released

        let placed = labels(&state, 5.0);
        assert_eq!(said(&placed), ["G"], "on the pane, and named");
        let axes = Axes::new(PANE, &state.spectrum_config);
        assert!(placed[0].rect.min.x > axes.at(0.5, 0.0).x, "travelling, not at the now-line");
        // The window is ten seconds and the onset is at 0, so the onset crosses
        // the far edge at exactly 10.
        assert_eq!(said(&labels(&state, 9.5)), ["G"], "still on, just inside the far edge");

        // The last frame the name is drawn, and the box it is drawn in there.
        let (mut last, mut leaving) = (f64::NAN, None);
        let mut now = 9.5;
        while now < 20.0 {
            if let Some(label) = labels(&state, now).first() {
                last = now;
                leaving = Some(label.rect);
            }
            now += 0.01;
        }
        let far = axes.at(0.5, 1.0).x;
        assert!(
            last > 10.0,
            "the name popped at {last}, before its onset had even reached the edge at 10",
        );
        assert!(
            last < 11.0,
            "the name was still drawn at {last}, a second after the onset it is written on \
             left the pane: it parked instead of travelling",
        );
        // ...and on that last frame it is a name being CUT by the edge rather
        // than one sitting inside it: the box is placed at the onset's own
        // depth, which is past the edge, and only the tail of it is still on the
        // pane.
        let rect = leaving.expect("drawn at 9.5 at the latest");
        assert!(
            rect.max.x > far,
            "the last frame drew the name whole, {} points inside the edge — so it stopped \
             against the edge rather than sliding out under it",
            far - rect.max.x,
        );

        // The price, stated: minutes on, the ribbon still fills the pane and
        // carries no name, its onset being a long way off the picture. A name
        // is a distance from an end, and this note has no end left to measure
        // one from.
        assert!(labels(&state, 120.0).is_empty(), "a drone kept a name it had no end for");
        let cfg = &state.spectrum_config;
        let split = super::super::axes::spectrum_share(cfg);
        let ribbons = super::super::roll::note_instances(
            &axes,
            &scale_of(&state),
            &state,
            split,
            120.0,
            2.0,
        );
        assert!(!ribbons.is_empty(), "the fixture is vacuous: the drone's ribbon left too");
    }

    /// A travelling name moves with the picture on every frame it is drawn,
    /// right through the moment its own anchor leaves the pane — in every
    /// orientation.
    ///
    /// This is the fixed gap read as a MOVEMENT, and it is the sharper of the
    /// two readings: a gap measured on one frame is met by any placement that
    /// happens to be right there, while a name that stops for even a few frames
    /// is a name the eye sees stop. Parking is exactly that failure — the step
    /// falls to zero at the crossing and stays there — so what this asserts is
    /// one step, the picture's own, from the first frame to the last.
    ///
    /// The ONSET anchor in all four orientations, since it is the one whose end
    /// leaves while its ribbon is still on the pane: reading order picks it in
    /// the two reversed orientations and the setting asks for it in the other
    /// two (see [`Anchor::of`]). Held throughout, so nothing but its own
    /// departure can end the name.
    #[test]
    fn a_travelling_name_scrolls_at_the_pictures_own_rate_until_it_is_gone() {
        for orientation in SpectralOrientation::ALL {
            let mut state = turned(24.0, 10.0, orientation);
            state.spectrum_config.note_names_travel = !orientation.is_time_reversed();
            state.tracker.handle_event(on(1.0, 60)); // held for the whole sweep

            // Square, so the same pane serves the vertical orientations.
            let square =
                egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 320.0) };
            let axes = Axes::new(square, &state.spectrum_config);
            let depth = axes.dir_depth();

            // From a second into the note — clear of the near edge, where a name
            // younger than its own ribbon is held against the pane — to well
            // past the moment the onset crosses the far edge at 11.
            let mut steps: Vec<f32> = Vec::new();
            let mut previous: Option<egui::Pos2> = None;
            let mut last = f64::NAN;
            let mut now = 2.0;
            while now < 13.0 {
                match labels_in(&state, now, square).first() {
                    Some(label) => {
                        if let Some(prev) = previous {
                            steps.push((label.lead - prev).dot(depth));
                        }
                        previous = Some(label.lead);
                        last = now;
                    }
                    None => previous = None,
                }
                now += 1.0 / 60.0;
            }
            assert!(
                last > 11.0 && last < 12.0,
                "{orientation:?}: the name was last drawn at {last}, where its onset crosses \
                 the far edge at 11",
            );
            let lo = steps.iter().copied().fold(f32::INFINITY, f32::min);
            let hi = steps.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            assert!(steps.len() > 400, "{orientation:?}: only {} frames swept", steps.len());
            assert!(lo > 0.1, "{orientation:?}: the name stalled — a frame moved it {lo} points");
            assert!(
                hi - lo < 0.01,
                "{orientation:?}: the name moves between {lo} and {hi} points a frame, so \
                 something other than the picture is placing it",
            );
        }
    }

    /// Travelling names hold still against each other as the roll scrolls, the
    /// same as names at the leading edge do.
    ///
    /// The thinning's grid is what buys that, and the grid is measured against
    /// the anchor — so moving the anchor is exactly the kind of change that
    /// could put the blinking back. Same measurement as
    /// [`a_name_never_blinks_out_and_back_as_the_roll_scrolls`], run with the
    /// setting on; the vacuity guard lives there.
    #[test]
    fn travelling_names_never_blink_out_and_back_either() {
        let travelling = |_now: f64| {
            let mut state = phrase(f64::NEG_INFINITY);
            state.spectrum_config.note_names_travel = true;
            state
        };
        assert_eq!(blinks(travelling, 2.23), 0);
        assert_eq!(blinks(travelling, 1.0), 0, "...and at the dialled size");
    }

    /// Notes off the pitch zoom are not named, and the zoom is the ordinary
    /// way to look at a few semitones of a piece that spans four octaves.
    #[test]
    fn notes_outside_the_pitch_zoom_are_left_out() {
        let mut state = state(24.0, 10.0); // 48..72
        for note in [36, 60, 84] {
            state.tracker.handle_event(on(0.0, note));
            state.tracker.handle_event(off(0.5, note));
        }
        assert_eq!(said(&labels(&state, 5.0)), ["C"], "only the one inside 48..72");
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
        // where a name plus its gap is nearer twenty.
        for i in 0..40 {
            let t = i as f64 * 0.1;
            state.tracker.handle_event(on(t, 60));
            state.tracker.handle_event(off(t + 0.05, 60));
        }
        let placed = labels(&state, 4.5);
        assert!(placed.len() > 1, "several of them are named");
        assert!(placed.len() <= 10, "but nothing like all forty: {}", placed.len());

        // Never touching, and spaced by more than their own boxes: a literal
        // rather than REPEAT_GAP itself, since a threshold taken from the
        // constant under test would hold at any value of it — and one only as
        // wide as a name's box is met by the box alone, with the gap at zero.
        let mut xs: Vec<f32> = placed.iter().map(|l| l.rect.min.x).collect();
        xs.sort_by(f32::total_cmp);
        for pair in xs.windows(2) {
            assert!(pair[1] - pair[0] >= 15.0, "names crowd at {pair:?}");
            // ...and by ONE gap, not two: the room a name demands is added to
            // whoever is tested against it, never stored on both sides.
            assert!(pair[1] - pair[0] < 26.0, "names sit twice as far apart as asked: {pair:?}");
        }
    }

    /// Names that are already placed HOLD THEIR PLACE as new notes arrive.
    ///
    /// This is the property the oldest-first sweep is for, and the reason it
    /// is not merely a taste: deciding newest-first lets every note played
    /// evict names anywhere on the pane, so the picture reshuffles under you
    /// at exactly the moment you are reading it. Asserted as the property, not
    /// as an ordering, so it keeps its teeth however the sort is later spelled.
    #[test]
    fn arriving_notes_do_not_move_the_names_already_placed() {
        let played = |extra: Option<f64>| {
            let mut state = state(24.0, 10.0);
            for i in 0..12 {
                let t = i as f64 * 0.25;
                state.tracker.handle_event(on(t, 60));
                state.tracker.handle_event(off(t + 0.1, 60));
            }
            if let Some(t) = extra {
                state.tracker.handle_event(on(t, 60));
                state.tracker.handle_event(off(t + 0.1, 60));
            }
            state
        };
        let xs = |state: &SharedState| -> Vec<f32> {
            labels(state, 4.0).iter().map(|l| l.rect.min.x).collect()
        };
        let before = xs(&played(None));
        // One more note struck at the now-line, after all of them.
        let after = xs(&played(Some(3.8)));
        assert!(before.len() > 2, "there are names to disturb: {}", before.len());
        for x in &before {
            assert!(
                after.contains(x),
                "a placed name moved when a note arrived: {before:?} -> {after:?}",
            );
        }
    }

    /// The pane turns, and the names turn with it: nothing here names a screen
    /// side, so a Top pane places them by the same arithmetic with the
    /// axes swapped.
    #[test]
    fn names_place_the_same_way_on_a_top_pane() {
        // Top: pitch runs left to right, time runs DOWN, so the leading
        // edge is the top of a ribbon and names grow downward from it.
        let mut state = turned(24.0, 10.0, SpectralOrientation::Top);
        for i in 0..6 {
            let t = i as f64 * 1.2;
            state.tracker.handle_event(on(t, 60));
            state.tracker.handle_event(off(t + 0.2, 60));
        }
        let tall = egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(110.0, 320.0) };
        let placed = labels_in(&state, 6.0, tall);
        assert!(placed.len() > 1, "several names: {}", placed.len());

        let axes = Axes::new(tall, &state.spectrum_config);
        // Every name sits on middle C's line, which with time vertical is an x.
        let lane = axes.at(scale_of(&state).t_of(60.0), 0.0).x;
        for label in &placed {
            assert!((label.rect.center().x - lane).abs() < 1.0, "off the ribbon's line");
        }
        // ...and they are spread along the TIME axis, which here is y.
        let mut ys: Vec<f32> = placed.iter().map(|l| l.rect.min.y).collect();
        ys.sort_by(f32::total_cmp);
        for pair in ys.windows(2) {
            assert!(pair[1] - pair[0] >= 12.0, "names crowd along time at {pair:?}");
        }
    }

    /// The offline whole-song layout names a ribbon at its ONSET rather than
    /// at its release, in every orientation — the one place the anchor is not
    /// the orientation's to decide. See [`Anchor::of`] for why a still picture
    /// has only the one end it can anchor to.
    #[test]
    fn whole_song_names_a_ribbon_at_its_onset() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(2.0, 60));
        state.tracker.handle_event(off(6.0, 60));
        let roll = state.tracker.roll().clone();
        state.whole_song = Some(crate::WholeSong {
            columns: Vec::new(),
            roll,
            start: 0.0,
            span: 10.0,
        });

        let placed = labels(&state, 4.0);
        assert_eq!(said(&placed), ["C"]);
        let axes = Axes::new(PANE, &state.spectrum_config);
        // Laid out statically from the near edge: the onset at 2 s of 10 is a
        // fifth of the way along, and the name starts there and grows later.
        let onset = axes.at(scale_of(&state).t_of(60.0), 0.2);
        assert!(placed[0].rect.min.x >= onset.x, "named at the onset, growing into the note");
        assert!(placed[0].rect.min.x < onset.x + 8.0);

        // ...and the static layout does not move as the playhead sweeps.
        let later = labels(&state, 9.0);
        assert_eq!(later[0].rect.min.x, placed[0].rect.min.x);
    }

    /// Whole-song keeps a note that began BEFORE the render's start, named at
    /// the near edge it reaches over.
    ///
    /// The layout's own edge of the clamp: a name is owed to every note with
    /// ribbon on the pane, and this is the one whose anchor is off it in the
    /// direction only a static layout has. What it keeps is a take rendered
    /// from its second minute still naming the notes already sounding at that
    /// moment — `--start` past the first note is an ordinary way to render an
    /// excerpt.
    ///
    /// Asked at two depths, and the second is the one the sweep's own
    /// `lookback` decides. Just before the start, the note is inside that bound
    /// and would be named however the bound were written; a pad that began a
    /// minute earlier is outside it, and reaches the picture only because a
    /// note with ink on the pane is kept whatever its anchor is doing.
    #[test]
    fn whole_song_keeps_a_note_that_began_before_the_render() {
        let named_from = |onset: f64, release: f64, start: f64| {
            let mut state = state(24.0, 10.0);
            state.tracker.handle_event(on(onset, 60)); // before the render's start
            state.tracker.handle_event(off(release, 60));
            let roll = state.tracker.roll().clone();
            state.whole_song =
                Some(crate::WholeSong { columns: Vec::new(), roll, start, span: 10.0 });
            let placed = labels(&state, start + 2.0);
            let axes = Axes::new(PANE, &state.spectrum_config);
            (said(&placed), placed.first().map(|l| l.rect), axes.at(0.5, 0.0).x)
        };

        let (names, rect, near) = named_from(1.0, 8.0, 3.0);
        assert_eq!(names, ["C"], "still sounding at the render's start, still named");
        assert!(
            rect.expect("named").min.x >= near,
            "clamped onto the near edge rather than drawn off the pane",
        );

        // A pad from a minute before the excerpt: its onset is many times
        // `lookback` (four of the widest name's rooms, a few seconds here) off
        // the near edge, so nothing but the ribbon keeps it.
        let (names, rect, near) = named_from(1.0, 60.0, 30.0);
        assert_eq!(names, ["C"], "a note held from long before the excerpt lost its name");
        assert!(
            rect.expect("named").min.x >= near,
            "clamped onto the near edge rather than drawn off the pane",
        );
    }

    /// In the whole-song layout a note the PLAYHEAD is inside is thinned like
    /// any other, so a render's names are decided once for the whole take.
    ///
    /// The held-note exemption is for a name standing at the now-line while the
    /// picture scrolls past it, and whole-song has no such name: the take is
    /// laid out statically and every name is anchored at an onset. Exempting a
    /// note because the playhead happens to be inside it makes a name appear
    /// and go as the playhead sweeps, which in a rendered video is a name
    /// blinking for no reason a viewer can see.
    #[test]
    fn a_whole_song_render_thins_a_sounding_note_like_any_other() {
        // Two strikes of one pitch too close for both names, the second still
        // sounding — the case the exemption used to hand a name to.
        //
        // Swept over every orientation, because a note the take never released
        // is live in EVERY frame of a render: a layout that anchored these
        // names on the leading edge would put that note's name on the playhead,
        // where it both takes the exemption and moves under it.
        for orientation in SpectralOrientation::ALL {
            let mut state = turned(24.0, 10.0, orientation);
            state.tracker.handle_event(on(1.0, 60));
            state.tracker.handle_event(off(1.05, 60));
            state.tracker.handle_event(on(1.1, 60));
            let roll = state.tracker.roll().clone();
            state.whole_song =
                Some(crate::WholeSong { columns: Vec::new(), roll, start: 0.0, span: 10.0 });

            // The playhead inside the second note, then well past where it
            // would have ended: the same one name, in the same place,
            // throughout.
            let inside = labels(&state, 1.5);
            assert_eq!(
                inside.len(),
                1,
                "{orientation:?}: thinned, exactly as a released pair would be",
            );
            for now in [3.0, 6.0, 9.0] {
                let later = labels(&state, now);
                assert_eq!(
                    said(&later),
                    said(&inside),
                    "{orientation:?}: the playhead moved a name at {now}s",
                );
                assert_eq!(later[0].rect.min, inside[0].rect.min, "{orientation:?} at {now}s");
            }
        }
    }

    /// The tiebreak's stated job is to keep a name from moving when the view
    /// is panned. In a collapsed tuning many nodes answer to one pitch, and
    /// which of them the iteration reaches first changes with the view — so
    /// without a rule the name changes with it too.
    #[test]
    fn panning_the_lattice_does_not_rename_a_pitch() {
        let equal = harmonigraph_core::Tuning::default();
        let named = |centre: i32| {
            let view = harmonigraph_scene::ViewConfig {
                center_threes: centre,
                ..harmonigraph_scene::ViewConfig::default()
            };
            note_name(&view, &view.reach(), &equal, 60.0).to_string()
        };
        assert_eq!(named(0), "C");
        for centre in [-2, -1, 1, 2] {
            assert_eq!(named(centre), "C", "panned to {centre}, middle C is still C");
        }
    }

    /// A pitch the reach cannot spell but the PICTURE has a node for is named
    /// off that node, not off equal temperament.
    ///
    /// The fallback exists for a note nothing on the lattice is showing, which
    /// is the case the red band is drawn for — so where the lattice is
    /// visibly lighting a node, taking it would put the analyzer's name and
    /// the node's own label in disagreement one pane apart.
    #[test]
    fn a_pitch_the_picture_shows_is_named_off_the_picture() {
        let view = harmonigraph_scene::ViewConfig::default();
        let just = harmonigraph_core::Tuning::just();
        // A node past the reach, and the pitch that sounds it.
        let far = harmonigraph_core::LatticePos::new(0, 25, 0);
        assert!(!view.reach().contains(far));
        let midi = 60.0 + just.pitch_class(far).to_cents() / 100.0;

        let equal_tempered = note_name(&view, &view.reach(), &just, midi).to_string();
        assert_eq!(
            equal_tempered,
            equal_tempered_name(midi).to_string(),
            "with only the reach to ask, this pitch has no lattice spelling at all",
        );

        let window = harmonigraph_scene::DrawnWindow {
            min: harmonigraph_core::LatticePos::new(-2, -2, 0),
            max: harmonigraph_core::LatticePos::new(2, 30, 0),
        };
        assert_eq!(
            note_name(&view, &window, &just, midi).to_string(),
            crate::panes::display_note_name(far, view.tempered()).to_string(),
            "the picture is drawing this node and the name ignored it",
        );
    }

    /// The tritone is a genuine tie — six fifths up spells F♯, six down spells
    /// G♭, and in an equal temperament they are one pitch costing the same on
    /// every count that reads. Something has to break it, and it has to be
    /// written down: left to `min_by_key` it falls to whichever the node
    /// iteration reaches first, so the spelling of every tritone on the pane
    /// would hang on the order a range walk happens to take.
    #[test]
    fn a_tie_between_two_spellings_is_broken_by_a_rule_not_by_iteration_order() {
        let view = harmonigraph_scene::ViewConfig::default();
        let equal = harmonigraph_core::Tuning::default();
        assert_eq!(note_name(&view, &view.reach(), &equal, 66.0).to_string(), "F\u{266F}");
    }

    /// A name at one pitch never suppresses a name at another, however close on
    /// screen the two land — the thinning is along TIME, within one pitch.
    ///
    /// Overlap across pitch is accepted for now. At a wide zoom a chord's names
    /// do land on each other, and refusing them is the worse of the two
    /// failures: a name you can read through a collision is worth more than a
    /// clean gap where a name should have been. A better answer than either is
    /// deferred rather than guessed at.
    #[test]
    fn names_at_different_pitches_never_thin_each_other() {
        let mut state = state(24.0, 10.0);
        // Six chromatic neighbours struck together, and released, so none of
        // them takes the held-note exception: at 100 points across two octaves
        // they are four points apart, where a name is a dozen tall. Every one
        // of them is still named.
        for note in 60..66 {
            state.tracker.handle_event(on(5.0, note));
            state.tracker.handle_event(off(5.2, note));
        }
        assert_eq!(labels(&state, 5.5).len(), 6, "all six, overlap and all");
        // ...and on a pane with room to draw them apart, unchanged.
        assert_eq!(labels_in(&state, 5.5, BIG).len(), 6);
    }

    /// A note you are HOLDING is named whatever else is in the way, and keeps
    /// its name until it is released.
    ///
    /// The one exception to the greedy, and the reason there is one: a note
    /// under your finger is the note you are most likely to be asking about,
    /// so whether it is named must not depend on what the rest of the picture
    /// happens to be doing around it.
    /// Two presses of ONE pitch, hard on each other's heels — the sweep gives
    /// the name to the first and refuses the second. Unless the second is
    /// being held, which is the exception.
    #[test]
    fn a_held_note_is_named_however_crowded_it_is() {
        // The same pitch twice, the second following close enough that its
        // name has nowhere clear to go.
        let strike = |held: bool| {
            let mut state = state(24.0, 10.0);
            state.tracker.handle_event(on(1.0, 60));
            state.tracker.handle_event(off(1.9, 60));
            state.tracker.handle_event(on(1.95, 60));
            if !held {
                state.tracker.handle_event(off(2.0, 60));
            }
            state
        };
        assert_eq!(labels(&strike(false), 2.0).len(), 1, "released, the second is refused");
        assert_eq!(labels(&strike(true), 2.0).len(), 2, "held, it is named regardless");

        // ...and it keeps the name for as long as it is held.
        assert_eq!(labels(&strike(true), 2.4).len(), 2);
    }

    /// A held note takes NOTHING out of the running for anyone else — its name
    /// is not in the reckoning the other names are placed against.
    ///
    /// The other half of the exception, and the half that is not a nicety. A
    /// held note's name sits at the now-line and stays there while every other
    /// name scrolls away from it, so a held name that occupied its ground
    /// would suppress each older name in turn as the two came level and hand
    /// it back once they parted — names blinking out and in, for as long as
    /// the key is down. Exempting a name from refusal but not from refusing
    /// only trades one arbitrary gap for a moving one.
    ///
    /// Stated as the property rather than as a placement, so it holds however
    /// the sweep is later ordered: whatever would be named with no key down is
    /// still named with one down.
    #[test]
    fn a_held_note_takes_no_name_away_from_an_older_one() {
        let played = |hold: bool| {
            let mut state = state(24.0, 10.0);
            // A run at one pitch, dense enough that the sweep is already
            // refusing most of it.
            for i in 0..20 {
                let t = i as f64 * 0.09;
                state.tracker.handle_event(on(t, 60));
                state.tracker.handle_event(off(t + 0.04, 60));
            }
            if hold {
                // ...and the same pitch pressed and held, right at the
                // now-line where its name would sweep across all of them.
                state.tracker.handle_event(on(1.9, 60));
            }
            state
        };

        // Every name shown with nothing held is still shown with a key down,
        // at each of a series of moments as the picture scrolls past it.
        for now in [2.0, 2.3, 2.6, 3.0, 4.0] {
            let alone = labels(&played(false), now);
            let holding = labels(&played(true), now);
            let places: Vec<f32> = holding.iter().map(|l| l.rect.min.x).collect();
            for label in &alone {
                assert!(
                    places.contains(&label.rect.min.x),
                    "at {now}s a name blinked out because a key was down: {:?} vs {places:?}",
                    alone.iter().map(|l| l.rect.min.x).collect::<Vec<_>>(),
                );
            }
            assert!(holding.len() > alone.len(), "and the held note is named too");
        }
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
        let mut state = state(24.0, 10.0);
        // A JUST tuning, which is the one the distinction lives in: an equal
        // temperament tempers the syntonic comma out by construction, so there
        // is no node a comma below E to name.
        state.tuning = harmonigraph_core::Tuning::just();
        state.tracker.handle_event(on(1.0, 64));
        state.tracker.handle_event(tuning(1.01, 64, -0.137));
        state.tracker.handle_event(off(2.0, 64));

        // The just third is a lattice node, and says so with the comma mark
        // the lattice draws on that node — the whole reason to spell a name
        // the lattice's way rather than as a piano key and a cents offset.
        assert_eq!(said(&labels_in(&state, 5.0, BIG)), ["E-"]);
    }

    /// A name is read from the note's OWN pitch, whatever else is on the pane.
    ///
    /// The thinning grain is ten cents, twenty times the half-cent tolerance a
    /// name is matched at, so one lane holds pitches that spell differently:
    /// 70.00 is the lattice's `B♭`, and 70.02 is already past the tolerance and
    /// falls back to the piano's `A♯`. Naming once per LANE and reusing it
    /// gives both of them whichever was reached first — so an in-tune note
    /// wears the spelling of a neighbour that was two cents sharp, and changes
    /// spelling again when that neighbour scrolls out of the window. The pane
    /// renames a note nobody touched, which reads as the plugin arguing with
    /// itself about what it just heard.
    #[test]
    fn a_name_is_read_from_its_own_pitch_not_a_lane_neighbours() {
        // The same two pitches every time; only which one is played first
        // differs. Both spellings are in one lane, so a per-lane memo cannot
        // tell them apart.
        let named = |bent_first: bool, now: f64| {
            let (early, late) = if bent_first { (0.02, 0.0) } else { (0.0, 0.02) };
            let mut state = state(24.0, 10.0);
            state.tracker.handle_event(on(1.0, 70));
            state.tracker.handle_event(tuning(1.01, 70, early));
            state.tracker.handle_event(off(2.0, 70));
            state.tracker.handle_event(on(5.0, 70));
            state.tracker.handle_event(tuning(5.01, 70, late));
            state.tracker.handle_event(off(6.0, 70));
            said(&labels_in(&state, now, BIG))
        };

        assert_eq!(named(false, 9.0), ["B♭", "A♯"], "in tune first, then two cents sharp");
        assert_eq!(named(true, 9.0), ["A♯", "B♭"], "the same pair, played the other way round");

        // And the survivor keeps its own name once the other has scrolled off:
        // at 13 s the note that ended at 2 s is past the ten-second window.
        assert_eq!(named(true, 13.0), ["B♭"], "an in-tune note left alone is still B♭");
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
        let view = harmonigraph_scene::ViewConfig::default();
        let equal = harmonigraph_core::Tuning::default();
        let name = |midi| note_name(&view, &view.reach(), &equal, midi).to_string();

        assert_eq!(name(60.0), "C", "the origin, not a remote spelling of it");
        assert_eq!(name(67.0), "G", "a fifth up");
        assert_eq!(name(65.0), "F", "a fifth down");
        // Four fifths up spells E; one just third up spells E-, and in this
        // tuning they are the same pitch. The plain letter wins.
        assert_eq!(name(64.0), "E");
    }

    /// A name carrying a septimal mark measures WIDER than its accidental
    /// stack alone would suggest, because the mark takes a column past them
    /// with air before it.
    ///
    /// `name_extent` is what the thinning believes a name occupies, so a
    /// short measurement here is not a rounding error, it is two names
    /// overlapping on the picture.
    #[test]
    fn a_septimal_mark_widens_what_a_name_is_measured_at() {
        let size = LABEL_PT;
        let plain =
            NoteName { letter: 'B', sharps: -1, syntonic_commas: 0, septimal_commas: 0 };
        let marked = NoteName { septimal_commas: -1, ..plain };
        let (plain_box, marked_box) = (name_extent(&plain, size), name_extent(&marked, size));

        // A whole column plus the gap wider, not a rounding's worth.
        let mark_size = size * marks::MARK_SIZE / marks::NAME_SIZE;
        let grew = marked_box.x - plain_box.x;
        assert!(
            grew > marks::SEPTIMAL_GAP * mark_size,
            "a septimal mark widened the name by only {grew}"
        );
        // The mark sits inside the line it shares, so nothing grows taller.
        assert_eq!(plain_box.y, marked_box.y, "a mark should not raise the line");
        // And a counted mark carries its digit, so it is wider still.
        let counted = NoteName { septimal_commas: -5, ..plain };
        assert!(
            name_extent(&counted, size).x > marked_box.x,
            "a counted mark takes a digit's width past a bare one"
        );
    }

    /// [`GLYPH_ADVANCE`] is the advance of the face the tree actually ships.
    ///
    /// The estimate no longer places anything — a name is drawn against its
    /// letter's ink ([`marks::NameLead`]) — so nothing about the PICTURE moves
    /// if this number drifts, and every other test here is blind to it: the two
    /// that read a name's position substitute [`label_rect`]'s own centre back
    /// in, where the extent cancels algebraically, and
    /// [`a_septimal_mark_widens_what_a_name_is_measured_at`] reads only
    /// differences, where a common factor cancels too. What would move is the
    /// thinning, silently and everywhere.
    ///
    /// So it is asserted where it can be: against a galley egui lays out through
    /// the shipped face. Iosevka Fixed advances every glyph at 500/1000 em, so
    /// the estimate of a bare name is the drawn advance up to the rasterizer's
    /// own rounding — which is real and is why the bound is not zero: at
    /// [`LABEL_PT`] the galley comes back 6.1875 against an arithmetic 6.175,
    /// egui having rounded the advance into its atlas cell.
    ///
    /// A percent of the size is the bound, and the margin either side of it is
    /// what makes the test worth having: the rounding is a tenth of a percent,
    /// while the constant this catches was out by 24.
    #[test]
    fn a_bare_names_estimate_is_the_advance_the_face_actually_has() {
        let ctx = themed_at(2.0);
        for size in [LABEL_PT, LABEL_PT * 5.0, marks::NAME_SIZE] {
            let name = NoteName { letter: 'C', sharps: 0, syntonic_commas: 0, septimal_commas: 0 };
            let mut drawn = 0.0;
            let _ = frame_full(&ctx, SCREEN, |ui| {
                drawn = ui
                    .painter()
                    .layout_no_wrap(
                        "C".to_owned(),
                        egui::FontId::monospace(size),
                        egui::Color32::PLACEHOLDER,
                    )
                    .size()
                    .x;
            });
            let estimated = name_extent(&name, size).x;
            assert!(
                (estimated - drawn).abs() < size * 0.01,
                "at {size}pt the thinning believes a bare name is {estimated} wide where the \
                 face lays it out at {drawn}: GLYPH_ADVANCE has drifted from the shipped font",
            );
        }
    }

    /// A septimal mark costs a reader what a syntonic one does, so the
    /// spelling chooser weighs them together.
    ///
    /// This is the shape of a bug that only exists where two branches meet.
    /// The sevens axis used to add no mark at all, so an off-sheet node
    /// spelled exactly like the node two fifths down and no test could tell
    /// which had been picked — leaving the DISTANCE term to decide it, and
    /// decide it wrong: in an equal temperament `(2,0,-1)` is nearer the
    /// origin than `(4,0,0)`, so a plain `E` was being named off the sevens
    /// sheet. Neither branch was wrong on its own; the naming became visible
    /// and the choice became visibly wrong in the same commit.
    #[test]
    fn a_plain_spelling_beats_an_off_sheet_one_at_the_same_pitch() {
        // A view with depth, so off-sheet nodes are candidates at all.
        let view = harmonigraph_scene::ViewConfig { extent_sevens: 1, ..Default::default() };
        let equal = harmonigraph_core::Tuning::default();
        let name = |midi| note_name(&view, &view.reach(), &equal, midi).to_string();

        // Every one of these has an equal-tempered twin one sevens step off,
        // nearer the origin, that would win on distance alone.
        assert_eq!(name(64.0), "E", "not the sevens-sheet node two fifths nearer");
        assert_eq!(name(66.0), "F\u{266F}");
        assert_eq!(name(60.0), "C");
        // And the cost is symmetric: the mark counts whichever way it points.
        for pos in [LatticePos::new(2, 0, -1), LatticePos::new(-2, 0, 1)] {
            let spelled = crate::panes::display_note_name(pos, view.tempered());
            assert!(
                spelling_cost(spelled, pos).0 > 0,
                "a septimal mark should cost like a comma, {spelled} did not"
            );
        }
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

    /// The setting turns them off — and so does hiding the thing they label.
    ///
    /// A name labels a RIBBON. With the roll hidden and the heatmap on, the
    /// pane still keeps a far region, so nothing geometric stops the names
    /// drawing — they would just be text floating over the heatmap, from a
    /// checkbox in the roll's own section that appeared not to turn them off.
    #[test]
    fn the_setting_turns_them_off_and_so_does_hiding_the_roll() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(0.0, 60));
        assert_eq!(labels(&state, 1.0).len(), 1);

        state.spectrum_config.note_names = false;
        assert!(labels(&state, 1.0).is_empty());

        state.spectrum_config.note_names = true;
        state.spectrum_config.show_roll = false;
        state.spectrum_config.show_spectrogram = true;
        state.spectrum_config.roll_fraction = 0.55;
        assert!(labels(&state, 1.0).is_empty(), "no ribbons, so nothing to name");
    }

    /// With the far region shut there is nowhere to draw, and `plan` says so
    /// rather than collapsing every name onto the edge.
    #[test]
    fn a_shut_roll_region_draws_no_names() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(0.0, 60));
        // The divider dragged all the way over: the spectrum owns everything.
        state.spectrum_config.roll_fraction = 0.0;
        assert!(labels(&state, 1.0).is_empty());
    }

    /// A BENT note's name follows the ribbon. The name's pitch and its depth
    /// have to come from the same end of the note, or they describe two
    /// different moments and the name floats off the thing it names.
    ///
    /// Live, the leading edge is where the note is sounding NOW, so that is
    /// the pitch the name sits at — `settled_pitch` is where the note began,
    /// which for a glide is somewhere else entirely. Taken from the onset, a
    /// note glided two semitones would put its name two semitones off the
    /// ribbon head, over another note's lane; a wide glide puts it a quarter
    /// of the pitch axis away, on a lane with no ribbon near it at all.
    ///
    /// Held-and-bent is the worst of it, and the case the design goes out of
    /// its way to always name: the name parks at the now-line while the ribbon
    /// head slides out from under it.
    #[test]
    fn a_bent_notes_name_rides_the_ribbon_rather_than_its_onset() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(1.0, 60));
        // Glided up a fifth over a second, and still held.
        state.tracker.handle_event(tuning(2.0, 60, 7.0));

        let placed = labels(&state, 3.0);
        assert_eq!(placed.len(), 1);
        let axes = Axes::new(PANE, &state.spectrum_config);
        // Sounding G at the leading edge (the now-line), not the C it began on.
        let sounding = axes.at(scale_of(&state).t_of(67.0), 0.0);
        assert!(
            (placed[0].rect.center().y - sounding.y).abs() < 1.0,
            "the name sits on the ribbon head at {}, not at {}",
            placed[0].rect.center().y,
            sounding.y,
        );
        assert_eq!(said(&placed), ["G"], "and it says what is sounding there");
    }

    /// A note glided clean out of the pitch zoom takes its name with it, and
    /// one glided INTO the zoom is named once it arrives — the cull has to ask
    /// about the pitch the name would be drawn at, not the note in general.
    #[test]
    fn the_cull_follows_the_name_not_the_notes_onset() {
        let mut out = state(24.0, 10.0); // 48..72
        out.tracker.handle_event(on(1.0, 60));
        out.tracker.handle_event(tuning(1.5, 60, 30.0)); // gone to MIDI 90
        assert!(labels(&out, 3.0).is_empty(), "its name left with it");

        let mut into = state(24.0, 10.0);
        into.tracker.handle_event(on(1.0, 30));
        into.tracker.handle_event(tuning(1.5, 30, 30.0)); // arrived at MIDI 60
        assert_eq!(said(&labels(&into, 3.0)), ["C"], "and arrives with it");
    }

    /// A wall of notes is thinned by the pane's own geometry, not by a cap on
    /// how many are looked at — so the names that survive are spread across
    /// the whole window rather than bunched at whichever end a cap kept.
    ///
    /// The obvious bound — consider only the newest N — is the wrong shape for
    /// a greedy that names from the far end inward: it would leave the older
    /// half of the pane bare however much room was going spare there. What
    /// bounds the work instead is that the placed stretches at one pitch are
    /// disjoint, so however many notes are offered, there are only ever a
    /// pane's width of them to test against.
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

    /// A name's drawn position is a straight line in the clock: over ninety
    /// frames of scrolling it advances by the same distance every time.
    ///
    /// Read off the GLYPHS, not off [`plan`]'s box, so everything between a
    /// take time and a letter's ink is inside it — the time axis, the box, the
    /// centring, and the size ladder [`draw`] rasterizes on. Any of those
    /// quantizing to the pixel grid would put a stair in a picture whose
    /// ribbons glide, which is judder against a name's own subject and reads
    /// as the name twitching.
    ///
    /// The rate is deliberately NOT a whole pixel per frame — the one speed at
    /// which a stair and a straight line are the same picture, and the reason
    /// the fixture's span is 7 seconds rather than the 10 its neighbours use.
    /// At 300 points over 7 seconds a frame is 1.4286 physical pixels, so the
    /// name lands on a different sub-pixel offset almost every frame and a
    /// stair anywhere would show as a step that is not that number.
    #[test]
    fn a_names_drawn_position_advances_by_the_same_step_every_frame() {
        const PPP: f32 = 2.0;
        let mut st = state(24.0, 7.0);
        st.tracker.handle_event(on(10.0, 60));
        st.tracker.handle_event(off(10.3, 60));

        // One context across the run: the galley cache is what makes a name's
        // ink comparable from frame to frame.
        let ctx = themed_at(PPP);

        let mut drawn = Vec::new();
        for frame in 0..90 {
            let now = 12.0 + f64::from(frame) / 60.0;
            let labels = labels(&st, now);
            let mut batch = crate::text::TextBatch::default();
            let _ = frame_full(&ctx, SCREEN, |ui| draw(ui.painter(), &labels, 1.0, &mut batch));
            if let Some(piece) = batch.pieces().iter().find(|p| p.text == "C") {
                drawn.push(piece.ink.left() * PPP);
            }
        }
        assert!(drawn.len() > 60, "the name has to be on the pane to be measured: {}", drawn.len());

        let steps: Vec<f32> = drawn.windows(2).map(|w| w[1] - w[0]).collect();
        let lo = steps.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = steps.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        // A tenth of a pixel: far under what a stair would cost (a whole one,
        // taken every frame or two) and far over f32's own noise at these
        // magnitudes, which measures below a thousandth.
        assert!(
            hi - lo < 0.1,
            "the name advances between {lo} and {hi} pixels a frame: its position stairs",
        );
    }

    /// What a reader actually sees, read off the glyphs [`draw`] queues through
    /// a real context rather than off the box that placed them: the letter's
    /// INK stands [`LABEL_INSET`] off the end of its ribbon, and stays there
    /// through the zoom, in every orientation and either growth.
    ///
    /// [`a_name_keeps_its_distance_from_its_note_through_the_zoom`] is the same
    /// question asked of the arithmetic, and it CANNOT see this: substitute
    /// `label_rect`'s own centre into it and the extent cancels, so it holds for
    /// any extent function whatsoever, at any bearing. This is the one that
    /// notices the lead being taken off the box again.
    ///
    /// It does NOT notice [`GLYPH_ADVANCE`], and nothing about a drawn name can:
    /// [`draw`] reads the lead, the name and the growth, and never the box the
    /// estimate built. That is the point of the split rather than a gap in it,
    /// and it is why the constant is pinned against the face directly, by
    /// [`a_bare_names_estimate_is_the_advance_the_face_actually_has`].
    ///
    /// A MARKED name as well as a bare one, and that is the half a single
    /// spelling cannot ask: the letter has to land in the same place whatever
    /// trails it, or a column of names stops reading as one. It was 1.31 points
    /// out between `C` and `B♭↓` while the box did the placing, at the dialled
    /// size alone.
    ///
    /// The bound is a tenth of a point — the ink is placed by the same
    /// measurement the assertion reads, so what is left is the arithmetic's own
    /// noise and not a design margin. Against it: 5.5 points of creep across
    /// the zoom with time running across the pane before this, and 12.2 with it
    /// running down. Issue #349.
    #[test]
    fn a_names_letter_stands_the_same_distance_off_its_note_at_every_zoom() {
        const PPP: f32 = 2.0;
        let ctx = themed_at(PPP);
        let plain = NoteName { letter: 'C', sharps: 0, syntonic_commas: 0, septimal_commas: 0 };
        let marked = NoteName { letter: 'B', sharps: -1, syntonic_commas: 0, septimal_commas: -1 };
        for orientation in [
            SpectralOrientation::Left,
            SpectralOrientation::Right,
            SpectralOrientation::Top,
            SpectralOrientation::Bottom,
        ] {
            let cfg = SpectrumConfig { orientation, ..SpectrumConfig::default() };
            let axes = Axes::new(PANE, &cfg);
            let anchor = axes.at(0.5, 0.5);
            // Both growths, which is both anchors: a name at the ribbon's head
            // runs one way through time and one at its onset the other. See
            // [`Anchor`].
            for grow in [axes.dir_depth(), -axes.dir_depth()] {
                for name in [plain, marked] {
                    for zoom in [1.0f32, 2.23, 5.0] {
                        let size = LABEL_PT * zoom;
                        let scales = NameScale { label: zoom, air: 1.0 };
                        let label = NoteLabel {
                            name,
                            rect: label_rect(&axes, grow, 0.5, 0.5, &name, size, scales),
                            lead: anchor + grow * (LABEL_INSET * scales.air),
                            grow,
                            at: 0.0,
                        };
                        let mut batch = crate::text::TextBatch::default();
                        let _ = frame_full(&ctx, SCREEN, |ui| {
                            draw(ui.painter(), std::slice::from_ref(&label), zoom, &mut batch)
                        });
                        let letter = name.letter.to_string();
                        let ink = batch
                            .pieces()
                            .iter()
                            .find(|p| p.text == letter)
                            .unwrap_or_else(|| panic!("no {letter} drawn at zoom {zoom}"))
                            .ink;
                        // The ink's own trailing edge, against the way the name
                        // runs — projecting the corners answers all four
                        // orientations without naming a screen side.
                        let corners =
                            [ink.left_top(), ink.right_top(), ink.left_bottom(), ink.right_bottom()];
                        let gap = corners
                            .iter()
                            .map(|&corner| (corner - anchor).dot(grow))
                            .fold(f32::INFINITY, f32::min);
                        assert!(
                            (gap - LABEL_INSET).abs() < 0.1,
                            "{orientation:?} growing {grow:?}: {letter}'s ink stands {gap} off \
                             its note at zoom {zoom}, not the {LABEL_INSET} it is placed at",
                        );
                    }
                }
            }
        }
    }

    /// A parked name yields to a name of the SAME pitch that has caught up with
    /// it, rather than the two being drawn on top of each other.
    ///
    /// The thinning spaces names in take time, which is the right currency for
    /// everything it decides — until a name is parked, when the clamp holds it
    /// on the picture's edge while its own anchor lies further back. Two names
    /// the grid spaced two seconds apart are then drawn five points apart, boxes
    /// 10.1 points wide: the picture
    /// `the_first_instance_takes_the_name_and_the_next_waits_for_room` exists to
    /// forbid, arriving by a route that test cannot see.
    ///
    /// A WHOLE-SONG render, because a still picture is the only one that parks
    /// — live, a name travels off with the end it names rather than being held
    /// (see [`place`](plan)). A note sounding across the render's start is
    /// cropped and drawn at that edge; a second strike of its pitch, just after
    /// the start, is the one that catches up with it. The pair is read twice,
    /// with the second strike far along the take and then close to the crop, so
    /// that a fixture where they never met could not pass.
    #[test]
    fn a_parked_name_yields_to_one_of_its_own_pitch_that_has_caught_up() {
        let render = |second: f64| {
            let mut state = state(24.0, 10.0);
            state.tracker.handle_event(on(1.0, 60)); // sounding across the start
            state.tracker.handle_event(off(4.0, 60));
            state.tracker.handle_event(on(second, 60));
            state.tracker.handle_event(off(second + 1.0, 60));
            let roll = state.tracker.roll().clone();
            state.whole_song =
                Some(crate::WholeSong { columns: Vec::new(), roll, start: 3.0, span: 10.0 });
            labels(&state, 5.0)
        };

        // Far apart: the cropped note's name parked on the near edge, the
        // second strike's out in the middle of the picture.
        let apart = render(6.0);
        assert_eq!(said(&apart), ["C", "C"], "both names are owed while they are apart");
        assert!(
            !apart[0].rect.intersects(apart[1].rect),
            "the fixture is vacuous: the two names already overlap at {:?} / {:?}",
            apart[0].rect,
            apart[1].rect,
        );

        // ...and where the second strike lands close to the crop, one name, and
        // it is the one standing at its own onset.
        let met = render(3.15);
        assert_eq!(said(&met), ["C"], "two names of one pitch were drawn on top of each other");
        // By ANCHOR and not by box, which is the only thing that says WHICH of
        // the two survived: the parked name is drawn at the crop and the other
        // at its own onset, and only the take times tell them apart.
        assert_eq!(
            met[0].at, 3.15,
            "the wrong one yielded: the parked name (onset 1.0) should go, not the one \
             standing at its own onset (3.15)",
        );
    }

    /// A parked name of a BENT note is written where its ribbon crosses the
    /// picture's edge, not where the note began.
    ///
    /// [`anchor_edge`] returns a time and the pitch the ribbon has AT that time,
    /// as a pair, and its own doc is about why they must stay one: take the
    /// depth from one end and the pitch from the other and the name lands off
    /// the ribbon entirely. Parking clamps the depth, so the pitch has to be
    /// read at the clamped time or the pair comes apart exactly where the clamp
    /// bites.
    ///
    /// A whole-song render, which is where the clamp now lives, of a note that
    /// glides a fifth before the render's start and is held past it: measured,
    /// the name sat 29.2 points from its ribbon on a 100-point pitch axis — a
    /// name saying C in clear air, with the ribbon it belongs to a quarter of
    /// the axis away.
    #[test]
    fn a_parked_name_of_a_bent_note_lands_on_its_ribbon_and_not_on_its_onset() {
        let mut state = state(24.0, 10.0);
        state.tracker.handle_event(on(1.0, 60));
        state.tracker.handle_event(tuning(1.5, 60, 7.0)); // C4 -> G4, then held
        let now = 8.0;
        let roll = state.tracker.roll().clone();
        state.whole_song =
            Some(crate::WholeSong { columns: Vec::new(), roll, start: 4.0, span: 10.0 });

        let placed = labels(&state, now);
        assert_eq!(said(&placed), ["C"], "the note is on the pane and owed a name");
        // Where the ribbon actually is at the far edge — read off the roll's own
        // geometry rather than assumed, so this cannot pass against a name and a
        // ribbon that have BOTH moved.
        let cfg = &state.spectrum_config;
        let axes = Axes::new(PANE, cfg);
        let scale = scale_of(&state);
        let split = super::super::axes::spectrum_share(cfg);
        let ribbons =
            super::super::roll::note_instances(&axes, &scale, &state, split, now, 2.0);
        let crossing = ribbons
            .iter()
            .max_by(|a, b| a.half_extent[1].total_cmp(&b.half_extent[1]))
            .expect("the glided note draws a ribbon");
        // The name's own row against the ribbon's, across pitch. Time is
        // horizontal in this orientation, so pitch is the screen's y.
        let off_by = (placed[0].rect.center().y - crossing.center[1]).abs();
        assert!(
            off_by < 4.0,
            "the name sits {off_by} points off the ribbon it names ({:?} vs {:?})",
            placed[0].rect.center(),
            crossing.center,
        );
        // The onset's own row is where it used to land, and is a long way from
        // the ribbon — so the assertion above is about the fix and not about a
        // pitch axis too small to tell the two apart.
        let onset_row = axes.at(scale.t_of(60.0), 1.0).y;
        assert!(
            (onset_row - crossing.center[1]).abs() > 20.0,
            "the fixture is vacuous: the onset's pitch and the ribbon's agree",
        );
    }
}
