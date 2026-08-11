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
//! the same [`draw_stacked_name`](crate::marks::draw_stacked_name) — letter
//! at full size, accidental riding high, syntonic-comma mark low, septimal
//! mark in a column of its own past them, all counted rather than repeated. Not a resemblance but the same function, so the two
//! cannot drift apart. That is the errand: a name here is read against the
//! lattice, so it has to answer in the lattice's vocabulary. A cents deviation
//! off the nearest piano key would say where a pitch sits between two keys
//! when what is wanted is which node it IS — and for a just third, `E-` says
//! it where "E +14\u{a2}" does not.
//!
//! Geometry comes from [`Axes`] like everything else in
//! the pane, so names turn and flip with it and nothing here names a screen
//! side.

use std::collections::HashMap;

use harmonigraph_core::{LatticePos, NoteName, PitchClass, RollNote, Tuning};
use harmonigraph_scene::ViewConfig;

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
/// Rebased by 1.3 from the 9.5 it was drawn at before the Name size bar
/// existed, that being where the bar settled once there was one; every length
/// below moved with it, since they are the spacing AROUND type of this size
/// and were being scaled by the same bar.
pub(super) const LABEL_PT: f32 = 12.35;

/// Points the name is set in from the ribbon's leading edge, along the time
/// axis. Enough that the letter is not touching the end it starts from.
///
/// Scaled with the type, like every other length here — the scale the pane
/// hands down carries the pitch zoom, the user's bar and the pane's own size,
/// and a spacing left in fixed points would be the same air on a pane half the
/// size as on one twice it. The Render preview and the video it previews are
/// where that shows worst: the render diverging from the pane the look was
/// dialled in on is the one thing this codebase most wants it not to do.
const LABEL_INSET: f32 = 2.6;

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
/// default opens flat, and `visible_positions` then yields the home sheet
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

/// The stretch of TAKE TIME a name covers, from a leading edge at `at`.
///
/// A name always lies from its leading edge over the ribbon it names, which is
/// to say toward increasing depth — and depth runs backward through time in
/// the live layout (the picture scrolls into the past) and forward in the
/// whole-song one. So which of two names reaches across the other is a
/// question about the layout, and this is where it is answered; the thinning
/// above only compares spans.
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

/// One name, placed: what it says and the box it was measured into.
#[derive(Clone, Copy, Debug)]
pub(super) struct NoteLabel {
    pub name: NoteName,
    /// Screen box the name covers, padded. Its centre is where the name is
    /// drawn — [`marks::draw_stacked_name`] centres on its anchor, while the
    /// placement here works in boxes that grow away from theirs.
    pub rect: egui::Rect,
    /// Test-only: the take time this name was placed at. Which NOTE a name
    /// belongs to is the whole question when asking whether the set of them
    /// holds still as the picture scrolls, and a rect that scrolls cannot
    /// answer it.
    #[cfg(test)]
    pub at: f64,
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
    // Names label RIBBONS, so they need ribbons. With the roll hidden there is
    // nothing under them to name and they would be text floating over the
    // heatmap at whatever pitches notes happened to have — which is not the
    // same picture at all, and would come from a checkbox in the roll's own
    // section that appeared not to turn them off.
    if !cfg.note_names || !cfg.show_roll || split >= 1.0 {
        return Vec::new();
    }
    let time = TimeAxis::new(state, split, now);
    let roll = state.roll();

    let size = LABEL_PT * label_scale;
    // One point of the depth axis, in seconds of take. A name's reach is a
    // length on the screen and the thinning measures in TIME, so this is the
    // rate between them — one number, the time axis being linear across the
    // region.
    let seconds_per_point = time.seconds_per_point(axes);
    let gap = (REPEAT_GAP * label_scale) as f64 * seconds_per_point;
    let room = |name: &NoteName| {
        depth_extent(axes, name, size, label_scale) as f64 * seconds_per_point + gap
    };
    // Live, the picture scrolls into the past, so a name lies back over its
    // ribbon; the whole-song layout runs the other way. See [`name_span`].
    let backward = !time.whole_song();

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
    let lookback = 4.0 * room(&WIDEST_NAME);
    let oldest = time.oldest();
    let sweep_from = if backward { oldest - lookback } else { time.time_at(0.0) - lookback };
    let mut notes: Vec<(&RollNote, Edge)> = roll
        .notes()
        // On its stop first, which is the one end every note carries without
        // being asked: reading a leading edge reaches into the note's bends
        // for the pitch there, and most of a long roll is nowhere near the
        // window. A note that stops before the sweep begins started before it
        // too, so this drops nothing the exact test would have kept.
        .filter(|note| note.stop(now) >= sweep_from)
        .map(|note| (note, leading_edge(&time, note, now)))
        .filter(|(_, edge)| edge.time >= sweep_from)
        .collect();
    // By LEADING EDGE, oldest first — where the name will sit, which is what
    // the thinning is handing out — and a total order, since the offline
    // render must not depend on the order the roll happened to hand them back.
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
    let naming = |pitch: f32, names: &mut HashMap<PitchClass, (NoteName, f64)>| {
        let class = PitchClass::from_cents(pitch.rem_euclid(12.0) * 100.0);
        *names.entry(class).or_insert_with(|| {
            let name = note_name(&state.view, &state.tuning, pitch);
            (name, room(&name))
        })
    };

    let mut occupied = Occupancy::default();
    let mut placed: Vec<NoteLabel> = Vec::new();
    let mut held: Vec<NoteLabel> = Vec::new();
    for (note, edge) in notes {
        // On the pane, and so worth drawing — decided on the pitch the name
        // will be DRAWN at, not the note's pitch in general, since the two
        // differ for a bent note and it is the name that has to be visible.
        //
        // Only DRAWING is culled here. A note off the far edge still takes its
        // turn in the thinning, which is what lets the names on the pane stand
        // still while it scrolls.
        let visible = note.stop(now) >= oldest && scale.contains(edge.pitch);
        // A held note stands outside the sweep in BOTH directions: it is named
        // whatever is already there, and it is not recorded, so it takes
        // nothing out of the running for anyone else.
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
        if note.is_live() {
            if !visible {
                continue;
            }
            let (name, _) = naming(edge.pitch, &mut names);
            let rect =
                label_rect(axes, scale.t_of(edge.pitch), time.depth_of(edge.time), &name, size, label_scale);
            // Two keys sounding one pitch — a doubled MIDI source, a layered
            // MPE part — would otherwise stamp the same name on the same
            // points once per voice. The name still appears; it is drawn once.
            if !held.iter().any(|l| l.name == name && l.rect == rect) {
                held.push(NoteLabel { name, rect, #[cfg(test)] at: edge.time });
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
            let rect =
                label_rect(axes, scale.t_of(edge.pitch), time.depth_of(edge.time), &name, size, label_scale);
            placed.push(NoteLabel { name, rect, #[cfg(test)] at: edge.time });
        }
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

/// A ribbon's LEADING edge: when it is, and what pitch the ribbon has THERE.
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

/// The end of a ribbon that comes first in reading order — the low-depth end,
/// which is the pane's now-line side in every orientation (the side
/// [`SpectralOrientation`](crate::SpectralOrientation) is named for) — and the
/// pitch it sits at.
///
/// Found by comparing the two ends rather than by naming one, because which
/// end it is differs between the layouts and the arithmetic does not: live,
/// time runs from the now-line outward, so a ribbon's leading edge is where it
/// most recently sounded; whole-song lays the take out in reading order, so it
/// is the onset.
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
/// **The pitch has to come from the same end as the depth**, which is the
/// whole reason this returns a pair. A bent note is at a different pitch at
/// each end: `settled_pitch` is where it began once its tuning had landed,
/// `end_pitch` where it is sounding now. Taking the depth from one end and the
/// pitch from the other puts the name off the ribbon entirely — a semitone off
/// for a modest bend, a quarter of the pitch axis for a wide glide, and over
/// some other note's lane wherever it lands. A held-and-bent note shows it
/// worst: the name parks at the now-line while the ribbon head slides out from
/// under it, and a held note is the one always named.
///
/// Both ends are CLAMPED into the region on the way, so a note reaching past
/// either edge is named at the last of it still on the pane.
fn leading_edge(time: &TimeAxis, note: &RollNote, now: f64) -> Edge {
    // Which end leads is a question about the LAYOUT, and `backward` is the
    // whole of the answer: depth runs into the past live (so the ribbon's
    // recent end is its head) and forward in the whole-song layout (so the
    // onset is). Asked of the two ends' times rather than of their depths,
    // which is the same question — depth is monotone in time — and answerable
    // for a note nowhere near the pane, where every depth clamps to the same
    // edge and the comparison stops meaning anything.
    if time.whole_song() {
        // The onset end, so the pitch the note SETTLED on rather than the key
        // it was pressed at — a retuned note reaches its real pitch a moment
        // after its note-on, and the ribbon is drawn from there.
        Edge { time: note.start, pitch: note.settled_pitch() }
    } else {
        Edge { time: note.stop(now), pitch: note.end_pitch() }
    }
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
/// lies over its own ribbon rather than over the picture in front of it —
/// except where the growth runs backward and the name carries marks, which is
/// the trade named at the bottom of this comment and measured in issue #151.
///
/// The LETTER's own position inside that box has to be independent of the
/// growth direction, or `C` and `C♯` disagree about where the letter goes and
/// a column of names stops reading as one. [`draw_stacked_name`] always sets
/// the letter first and lets the accidental/comma columns trail after it, so
/// growth that runs the same way (time left-to-right or top-to-bottom) already
/// puts the letter flush against the leading edge, same as it would with no
/// marks at all -- nothing to do there. Growth that runs backward (Right's
/// leftward time) is the mismatch: "first" is still the box's FAR edge from
/// the leading edge, not its near one, so centring on the FULL name drags the
/// letter along with however wide its marks happen to be. Measuring the pure
/// letter's reach instead of the whole name's is what keeps it still; the
/// marks are what absorb the difference.
///
/// What that costs is worth stating at its real size, because it is not a
/// rounding error: the box's near edge lands at `inset + letter - along`, so a
/// name whose marks are wider than [`LABEL_INSET`] — which is every marked name
/// — puts its mark column PAST the leading edge, over the picture in front of
/// the note. Measured on a 300pt pane at `LABEL_PT`: a bare `C` clears it by
/// 0.65pt, `C♯` crosses by 3.4pt, `B♭↓` by 9.0pt, the widest spelling by 17.2pt,
/// and it scales with the pitch zoom.
///
/// The two constraints cannot both hold while [`draw_stacked_name`] typesets the
/// marks after the letter: pinning the letter fixes the box's near edge and lets
/// its far edge travel, and containing the box puts the letter back on however
/// wide the marks are. Issue #151 holds the measurements and the candidate ways
/// out; this comment exists so the spill reads as a known price rather than as a
/// bug nobody noticed.
///
/// [`draw_stacked_name`]: crate::marks::draw_stacked_name
fn label_rect(
    axes: &Axes,
    p: f32,
    d: f32,
    name: &NoteName,
    size: f32,
    label_scale: f32,
) -> egui::Rect {
    let extent = name_extent(name, size);
    let depth = axes.dir_depth();
    // How far the box reaches along the depth axis: text always runs across the
    // screen, so that is its width when time runs across the pane and its height
    // when time runs up or down it. Projecting answers all four without naming a
    // screen side.
    let along_depth = (extent.x * depth.x).abs() + (extent.y * depth.y).abs();
    // The same projection, but of the bare letter alone -- no accidental,
    // comma, or septimal mark -- which is what backward growth measures from.
    let bare = NoteName { letter: name.letter, sharps: 0, syntonic_commas: 0, septimal_commas: 0 };
    let letter_extent = name_extent(&bare, size);
    let letter_along_depth =
        (letter_extent.x * depth.x).abs() + (letter_extent.y * depth.y).abs();
    let inset = LABEL_INSET * label_scale;
    // `depth.x + depth.y` is `depth`'s own sign: +1 forward (time runs the
    // screen's own way), -1 backward. Backward is where the letter and the box
    // disagree on which end is "first" -- see above.
    let growth = if depth.x + depth.y < 0.0 {
        letter_along_depth - along_depth * 0.5
    } else {
        along_depth * 0.5
    };
    let centre = axes.at(p, d) + depth * (inset + growth);
    egui::Rect::from_center_size(centre, extent).expand(LABEL_PAD * label_scale)
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
/// either — and the octave is already said by where the name sits on the axis.
///
/// The fallback, for a pitch the visible lattice has no node for, is the
/// equal-tempered spelling — still a [`NoteName`], so it draws identically and
/// there is one rendering path rather than two. It is a real case: the pane
/// already flags notes sounding off the lattice with a band down the spectrum,
/// and a note with no name at all would just look like a bug.
fn note_name(view: &ViewConfig, tuning: &Tuning, midi: f32) -> NoteName {
    // Cents from C, measured from MIDI 0 (which IS a C) — the same reduction
    // the pane's hover makes before asking the same question.
    let pc = PitchClass::from_cents(midi.rem_euclid(12.0) * 100.0);
    match naming_node(view, tuning, pc) {
        Some(pos) => crate::panes::display_note_name(pos, view.tempered()),
        None => equal_tempered_name(midi),
    }
}

/// The visible node to name a pitch by: the closest match, and among matches
/// equally close the one that spells most plainly.
///
/// Its own function rather than
/// [`nearest_visible_node`](crate::panes::nearest_visible_node), which it otherwise
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
            label.rect.center(),
            label.name,
            theme::text(),
            theme::well(),
            scale,
            magnify,
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
        plan(state, &axes, &scale, split, now, 1.0)
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
            let labels =
                plan(&state, &Axes::new(BIG, &cfg), &scale_of(&state), split, now, label_scale);
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
            for label in plan(&state, &axes, &scale_of(&state), split, now, 1.0) {
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
        let placed = plan(&state, &Axes::new(BIG, &cfg), &scale_of(&state), split, 20.0, ZOOMED);
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
    /// [`draw_stacked_name`]: crate::marks::draw_stacked_name
    #[test]
    fn the_letter_lines_up_with_or_without_an_accidental() {
        let plain = NoteName { letter: 'C', sharps: 0, syntonic_commas: 0, septimal_commas: 0 };
        let sharp = NoteName { letter: 'C', sharps: 1, syntonic_commas: 0, septimal_commas: 0 };
        for orientation in [SpectralOrientation::Left, SpectralOrientation::Right] {
            let cfg = SpectrumConfig { orientation, ..SpectrumConfig::default() };
            let axes = Axes::new(PANE, &cfg);
            let plain_rect = label_rect(&axes, 0.5, 0.5, &plain, 12.0, 1.0);
            let sharp_rect = label_rect(&axes, 0.5, 0.5, &sharp, 12.0, 1.0);
            assert!(
                (plain_rect.min.x - sharp_rect.min.x).abs() < 0.01,
                "{orientation:?}: C's letter at {} but C♯'s at {}",
                plain_rect.min.x,
                sharp_rect.min.x,
            );
        }
    }

    /// The same claim as
    /// [`the_letter_lines_up_with_or_without_an_accidental`], but read off
    /// the glyphs [`draw`] actually queues through a real `egui::Context` —
    /// real font metrics, not [`name_extent`]'s estimate — so it would catch
    /// the estimate and the real measurement disagreeing by enough to move
    /// the letter visibly, which the arithmetic-only test cannot see.
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
                rect: label_rect(&axes, 0.5, 0.5, &name, LABEL_PT, 1.0),
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

    /// The offline whole-song layout lays the take out in reading order, so a
    /// ribbon's leading edge is its ONSET there rather than its release — the
    /// one place the two layouts disagree about which end that is.
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
            note_name(&view, &equal, 60.0).to_string()
        };
        assert_eq!(named(0), "C");
        for centre in [-2, -1, 1, 2] {
            assert_eq!(named(centre), "C", "panned to {centre}, middle C is still C");
        }
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
        assert_eq!(note_name(&view, &equal, 66.0).to_string(), "F\u{266F}");
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
        let name = |midi| note_name(&view, &equal, midi).to_string();

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
        let name = |midi| note_name(&view, &equal, midi).to_string();

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

}
