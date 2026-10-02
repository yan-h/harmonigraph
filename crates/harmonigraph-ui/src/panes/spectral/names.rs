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

use std::collections::{HashMap, HashSet};

use harmonigraph_core::{LatticePos, NoteName, PitchClass, RollNote, Tempered, Tuning};
use harmonigraph_scene::{DrawnWindow, ViewConfig};

use super::axes::{Axes, PitchScale, TimeAxis};
use crate::marks;
use crate::{theme, PictureState};

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
/// the font's side bearing, and the old estimated box carried its own
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
/// 2.5. The vertical error comes from line height rather than glyph advance,
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

/// Clear space around actual name ink, in logical points.
const LABEL_PAD: f32 = 1.95;

/// Clear time a name demands beyond its own box, in points along the time
/// axis, before the next name at that pitch may take a place.
///
/// Without it, successive names at one pitch are allowed to butt together, and
/// a run of repeats reads as a word rather than as a name on each of several
/// notes. This is the "certain span" the greedy leaves between the instance it
/// picks and the next one it will take.
const REPEAT_GAP: f32 = 7.8;

/// Previous winners on one drawn surface. Musical identity excludes pitch,
/// spelling and position: expression and scrolling must not create new labels.
#[derive(Default)]
pub(crate) struct Thinning {
    visible: HashSet<(harmonigraph_core::VoiceKey, u64)>,
    suppressed: HashSet<(harmonigraph_core::VoiceKey, u64)>,
    now: Option<f64>,
    layout: Option<ThinningLayout>,
}

#[derive(PartialEq)]
struct ThinningLayout {
    size: egui::Vec2,
    pitch: [f32; 2],
    seconds: f32,
    split: f32,
    scale: [f32; 2],
    grow: egui::Vec2,
    depth: egui::Vec2,
}

/// A generously marked spelling bounds the coarse time lookback. Actual
/// candidate bounds decide visibility and collisions after this cheap filter.
const WIDEST_NAME: NoteName =
    NoteName { letter: 'C', sharps: 2, syntonic_commas: -12, septimal_commas: -12 };

/// One name, placed: what it says, the box it was measured into, and the point
/// its letter is drawn against.
#[derive(Clone, Copy, Debug)]
pub(super) struct NoteLabel {
    pub name: NoteName,
    /// Musical onset, independent of the label anchor or release time.
    pub onset: f64,
    /// Drawn pitch height, for simultaneous notes (including cropped bends).
    pub pitch: f32,
    /// Actual glyph bounds, padded for separation from neighbouring names.
    pub rect: egui::Rect,
    /// Where the letter's ink goes: [`LABEL_INSET`] off the end of the ribbon
    /// this name belongs to, along [`grow`](Self::grow), and carrying whatever
    /// `place`'s clamp did to the box.
    pub lead: egui::Pos2,
    /// The direction from that point INTO the note — see the measured rectangle.
    pub grow: egui::Vec2,
    /// Test-only: the take time this name was placed at. Which NOTE a name
    /// belongs to is the whole question when asking whether the set of them
    /// holds still as the picture scrolls, and a rect that scrolls cannot
    /// answer it.
    #[cfg(test)]
    pub at: f64,
}

#[cfg(test)]
fn plan_for_test(
    state: &PictureState,
    axes: &Axes,
    scale: &PitchScale,
    split: f32,
    now: f64,
    scales: NameScale,
) -> Vec<NoteLabel> {
    plan_with_thinning_for_test(state, axes, scale, split, now, scales, &mut Thinning::default())
}

#[cfg(test)]
fn plan_with_thinning_for_test(
    state: &PictureState,
    axes: &Axes,
    scale: &PitchScale,
    split: f32,
    now: f64,
    scales: NameScale,
    thinning: &mut Thinning,
) -> Vec<NoteLabel> {
    thread_local! { static CONTEXT: egui::Context = crate::tests::probe::themed_at(2.0); }
    CONTEXT.with(|ctx| {
        let mut result = Vec::new();
        let _ = crate::tests::probe::frame_full(ctx, egui::vec2(1920.0, 1080.0), |ui| {
            let painter = ui.painter();
            let mut namer =
                Namer::new(&state.appearance.view, state.shown(), &state.runtime.tuning);
            result = plan(painter, state, axes, scale, split, now, scales, &mut namer, thinning);
        });
        result
    })
}

/// Every name this frame draws, already thinned to the ones that fit — empty
/// when the setting is off or the pane has kept no roll region to draw in.
///
/// The two scales it lays names out by part company at the pitch zoom — see
/// [`NameScale`].
#[allow(clippy::too_many_arguments)]
pub(super) fn plan(
    painter: &egui::Painter,
    state: &PictureState,
    axes: &Axes,
    scale: &PitchScale,
    split: f32,
    now: f64,
    scales: NameScale,
    namer: &mut Namer,
    thinning: &mut Thinning,
) -> Vec<NoteLabel> {
    if thinning.now.is_some_and(|last| now < last) {
        thinning.visible.clear();
        thinning.suppressed.clear();
    }
    thinning.now = Some(now);
    let cfg = &state.appearance.spectrum;
    // Names label RIBBONS, so they need ribbons. With the roll hidden there is
    // nothing under them to name and they would be text floating over the
    // heatmap at whatever pitches notes happened to have — which is not the
    // same picture at all, and would come from a checkbox in the roll's own
    // section that appeared not to turn them off.
    if !cfg.note_names || !cfg.show_roll || split >= 1.0 {
        thinning.visible.clear();
        thinning.suppressed.clear();
        return Vec::new();
    }
    let time = TimeAxis::new(state, split, now);
    let anchor = Anchor::of(cfg);
    let roll = state.roll();

    // One point of the depth axis, in seconds of take. A name's reach is a
    // length on the screen and the thinning measures in TIME, so this is the
    // rate between them — one number, the time axis being linear across the
    // region.
    let seconds_per_point = time.seconds_per_point(axes);
    // Direction along the ribbon from its chosen anchor.
    let grow = if anchor == Anchor::Onset { -axes.dir_depth() } else { axes.dir_depth() };
    let layout = ThinningLayout {
        size: axes.rect.size(),
        pitch: [scale.min_midi, scale.max_midi],
        seconds: cfg.roll_seconds,
        split,
        scale: [scales.label, scales.air],
        grow,
        depth: axes.dir_depth(),
    };
    if thinning.layout.as_ref() != Some(&layout) {
        // Explicit changes to the view can make room again. Ordinary scrolling
        // keeps retirements, and surviving winners retain their priority.
        thinning.suppressed.clear();
        thinning.layout = Some(layout);
    }

    // Include names whose anchor just left the frame but whose ink still
    // reaches it. Each candidate is culled again with its own spelling below.
    let oldest = time.oldest();
    let mut widest = crate::text::TextBatch::default();
    draw_name(&mut widest, painter, egui::Pos2::ZERO, WIDEST_NAME, scales.label, grow);
    let reach = widest.bounds().size().dot(axes.dir_depth().abs())
        + 2.0 * LABEL_PAD * scales.label
        + LABEL_INSET * scales.air;
    let sweep_from = oldest - f64::from(reach) * seconds_per_point;
    let notes = roll
        .notes()
        .filter(|note| note.stop(now) >= sweep_from)
        .map(|note| (note, anchor_edge(note, now, anchor)));

    // Measure each pitch class once per frame. Font, scale and direction are
    // common to this pass; musical expression does not invalidate a cache.
    let mut names: HashMap<PitchClass, (NoteName, egui::Rect)> = HashMap::new();
    let mut naming = |pitch: f32, names: &mut HashMap<PitchClass, (NoteName, egui::Rect)>| {
        let class = PitchClass::from_cents(pitch.rem_euclid(12.0) * 100.0);
        *names.entry(class).or_insert_with(|| {
            let name = namer.name(pitch);
            let mut batch = crate::text::TextBatch::default();
            draw_name(&mut batch, painter, egui::Pos2::ZERO, name, scales.label, grow);
            (name, batch.bounds())
        })
    };

    // The letter stays pinned to the musical anchor, including when it
    // scrolls past the far edge. Only the near pane boundary can move it,
    // while a young ribbon is shorter than its name.
    let toward_near = grow.dot(axes.dir_depth()) < 0.0;
    let place = |edge: &Edge, ink: egui::Rect| {
        let d = time.depth_of_unclamped(edge.time);
        let t = scale.t_of(edge.pitch);
        let lead = axes.at(t, d) + grow * (LABEL_INSET * scales.air);
        let rect = ink.translate(lead.to_vec2()).expand(LABEL_PAD * scales.label);
        if !toward_near {
            return (rect, lead);
        }
        let span = (rect.width() * grow.x).abs() + (rect.height() * grow.y).abs();
        let over = ((rect.center() - axes.at(t, 0.0)).dot(grow) + span * 0.5).max(0.0);
        (rect.translate(-grow * over), lead - grow * over)
    };

    // The pitches whose ribbon still reaches into the pitch zoom: the zoom
    // widened by half a ribbon, whose width is set in semitones. A ribbon's
    // ink meets the edge that half width before its centre does, so asked of
    // the centre alone a name went with half its ribbon still drawn (#825).
    //
    // The RIBBON's width and not the name's box, so a name taller than its
    // ribbon goes with the edge of its box still inside, and the ribbon's
    // outline, which the roll adds in points, is not waited for either. Past
    // the edge the scissor cuts the name as it cuts the ribbon. The far edge
    // no longer makes this trade — there a name stays until its own box has
    // left (see `visible` below) — so the two edges now differ.
    let half_ribbon = cfg.roll_thickness * 0.5;
    let ribbon_reach = scale.min_midi - half_ribbon..=scale.max_midi + half_ribbon;

    let mut candidates = Vec::new();
    for (note, edge) in notes {
        let (name, ink) = naming(edge.pitch, &mut names);
        if !ribbon_reach.contains(&edge.pitch) {
            continue;
        }
        let (rect, lead) = place(&edge, ink);
        if !rect.intersects(axes.rect) {
            continue;
        }
        let id = (note.key(), note.start.to_bits());
        candidates.push((
            id,
            note.is_live(),
            NoteLabel {
                name,
                onset: note.start,
                pitch: edge.pitch,
                rect,
                lead,
                grow,
                #[cfg(test)]
                at: edge.time,
            },
        ));
    }
    // Held notes win, then the labels already visible, then chronological
    // onset/pitch/voice order. Expression strength never changes priority.
    candidates.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| thinning.visible.contains(&b.0).cmp(&thinning.visible.contains(&a.0)))
            .then_with(|| a.2.onset.total_cmp(&b.2.onset))
            .then_with(|| a.2.pitch.total_cmp(&b.2.pitch))
            .then_with(|| a.0.cmp(&b.0))
    });
    let eligible: HashSet<_> = candidates.iter().map(|c| c.0).collect();
    thinning.suppressed.retain(|id| eligible.contains(id));
    let mut visible = HashSet::new();
    let mut boxes: Vec<egui::Rect> = Vec::new();
    let mut placed = Vec::new();
    let air = axes.dir_depth().abs() * (REPEAT_GAP * scales.label * 0.5);
    for (id, _, label) in candidates {
        if thinning.suppressed.contains(&id) {
            continue;
        }
        let bounds = label.rect.expand2(air);
        if boxes.iter().any(|other| other.intersects(bounds)) {
            // A displaced winner retires until it leaves this viewport;
            // otherwise a held note repeatedly hides and reveals old names.
            if thinning.visible.contains(&id) {
                thinning.suppressed.insert(id);
            }
            continue;
        }
        visible.insert(id);
        boxes.push(bounds);
        placed.push(label);
    }
    thinning.visible = visible;

    // Paint oldest first, then low to high for simultaneous onsets. Releasing
    // a note or reversing the time axis must not change its stacking order.
    placed.sort_by(|a, b| a.onset.total_cmp(&b.onset).then(a.pitch.total_cmp(&b.pitch)));
    placed
}

/// A point on a ribbon a name can be written at: when it is, and what pitch the
/// ribbon has THERE.
///
/// A take time and sounding pitch, independent of the viewport. Placement
/// maps these to the current surface before collision selection.
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
/// setting asks for the other one.
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
    /// [`SpectralOrientation::is_time_reversed`]:
    ///     crate::SpectralOrientation::is_time_reversed
    fn of(cfg: &crate::SpectrumConfig) -> Anchor {
        let reads_first =
            if cfg.orientation.is_time_reversed() { Anchor::Onset } else { Anchor::Leading };
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
/// out from under it. Held notes have first priority when labels collide.
///
/// Neither end is bounded here, and the pair is the true one however far off
/// the picture it lies. [`plan`] lets a name leave with the end it names.
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

/// One frame's answer to naming and visible-node questions. The roll and red
/// bands share it; Spiral makes one for its own frame.
///
/// A note's name is the LATTICE's spelling of its pitch.
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
/// ([`PictureState::shown`](crate::PictureState::shown)) only where the reach
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
pub(crate) struct Namer {
    reach: DrawnWindow,
    shown: DrawnWindow,
    tuning: Tuning,
    tempered: Tempered,
    reach_nodes: Option<Vec<(LatticePos, PitchClass)>>,
    shown_nodes: Option<Vec<(LatticePos, PitchClass)>>,
    visible: HashMap<PitchClass, bool>,
}

impl Namer {
    pub(crate) fn new(view: &ViewConfig, shown: DrawnWindow, tuning: &Tuning) -> Self {
        Self {
            reach: view.reach(),
            shown,
            tuning: *tuning,
            tempered: view.tempered(),
            reach_nodes: None,
            shown_nodes: None,
            visible: HashMap::new(),
        }
    }

    fn nodes(window: DrawnWindow, tuning: Tuning) -> Vec<(LatticePos, PitchClass)> {
        window.positions().map(|pos| (pos, tuning.pitch_class(pos))).collect()
    }

    pub(crate) fn name(&mut self, midi: f32) -> NoteName {
        // Cents from C, measured from MIDI 0 (which IS a C).
        let pc = PitchClass::from_cents(midi.rem_euclid(12.0) * 100.0);
        let nodes = self.reach_nodes.get_or_insert_with(|| Self::nodes(self.reach, self.tuning));
        let pos = naming_node_from(nodes.iter().copied(), self.tempered, &self.tuning, pc).or_else(
            || {
                if self.shown == self.reach {
                    return None;
                }
                let nodes =
                    self.shown_nodes.get_or_insert_with(|| Self::nodes(self.shown, self.tuning));
                naming_node_from(nodes.iter().copied(), self.tempered, &self.tuning, pc)
            },
        );
        match pos {
            Some(pos) => crate::panes::display_note_name(pos, self.tempered),
            None => equal_tempered_name(midi),
        }
    }

    /// Whether the drawn window holds a node for this pitch. Stop on the
    /// first match in the usual case; after a miss, prepare the shown list so
    /// further bent voices and fallback names reuse its pitch calculations.
    pub(crate) fn shows_node(&mut self, pc: PitchClass) -> bool {
        if let Some(&visible) = self.visible.get(&pc) {
            return visible;
        }
        let visible = match &self.shown_nodes {
            Some(nodes) => nodes.iter().any(|&(_, pitch)| self.tuning.matches(pc, pitch)),
            None => {
                let visible = self
                    .shown
                    .positions()
                    .any(|pos| self.tuning.matches(pc, self.tuning.pitch_class(pos)));
                if !visible {
                    self.shown_nodes = Some(Self::nodes(self.shown, self.tuning));
                }
                visible
            }
        };
        self.visible.insert(pc, visible);
        visible
    }
}

#[cfg(test)]
fn note_name(view: &ViewConfig, shown: &DrawnWindow, tuning: &Tuning, midi: f32) -> NoteName {
    Namer::new(view, *shown, tuning).name(midi)
}

/// The node in `window` to name a pitch by: the closest match, and among
/// matches equally close the one that spells most plainly.
///
/// The second half of that sentence is the whole function. A plain minimum
/// over the same filter would do everywhere the tiebreak has nothing to go on,
/// and would be wrong everywhere else.
///
/// In a JUST tuning there is nothing to break: distinct lattice positions are
/// distinct pitches, so at the half-cent tolerance exactly one node can match.
/// In an EQUAL temperament — which is the default this plugin opens on — the
/// lattice collapses: twelve fifths are seven octaves exactly, so the origin
/// and `(12,0,0)` are one pitch, three major thirds are an octave, and a dozen
/// visible nodes answer to middle C. All are the same distance (zero) from it,
/// so a plain minimum returns whichever the iteration reached first, which is
/// the CORNER of the visible window: middle C names itself `F♭5+6`, and
/// renames itself whenever the view is panned. True, useless, and not what the
/// lattice shows you, which is the lit node you were looking at.
///
/// Kept apart from [`Namer::shows_node`], which asks only WHETHER a node
/// matches and can stop at the first, while naming must choose AMONG matches.
fn naming_node_from(
    nodes: impl Iterator<Item = (LatticePos, PitchClass)>,
    tempered: Tempered,
    tuning: &Tuning,
    pc: PitchClass,
) -> Option<LatticePos> {
    nodes
        .filter(|&(_, pitch)| tuning.matches(pc, pitch))
        .min_by_key(|&(pos, pitch)| {
            (
                pc.distance_to(pitch),
                spelling_cost(crate::panes::display_note_name(pos, tempered), pos),
            )
        })
        .map(|(pos, _)| pos)
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
fn draw_name(
    batch: &mut crate::text::TextBatch,
    painter: &egui::Painter,
    lead: egui::Pos2,
    name: NoteName,
    label_scale: f32,
    grow: egui::Vec2,
) {
    let (raster, magnify) =
        crate::text::ladder(label_scale, LABEL_PT, painter.ctx().pixels_per_point());
    marks::draw_stacked_name(
        batch,
        painter,
        lead,
        name,
        marks::NameInk { fill: theme::picture_name(), outline: theme::picture() },
        marks::NameSize { scale: LABEL_PT * raster / marks::NAME_SIZE, magnify },
        marks::NameLead::Letter(grow),
    );
}

pub(super) fn draw(
    painter: &egui::Painter,
    labels: &[NoteLabel],
    label_scale: f32,
    batch: &mut crate::text::TextBatch,
) {
    batch.finish_layer();
    for label in labels {
        draw_name(batch, painter, label.lead, label.name, label_scale, label.grow);
        batch.finish_layer();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::probe::{frame_full, fresh_picture as fresh, painted_full, themed_at};
    use crate::{SpectralOrientation, SpectrumConfig};
    use harmonigraph_core::{NoteEvent, NoteEventKind, SourceId};

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
        NoteEvent {
            source: SourceId::DIRECT,
            time,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 0.8 },
        }
    }

    fn off(time: f64, note: u8) -> NoteEvent {
        NoteEvent { source: SourceId::DIRECT, time, channel: 0, note, kind: NoteEventKind::Off }
    }

    /// A per-note tuning on a sounding note, in semitones off its key.
    fn tuning(time: f64, note: u8, semitones: f32) -> NoteEvent {
        NoteEvent {
            source: SourceId::DIRECT,
            time,
            channel: 0,
            note,
            kind: NoteEventKind::Tuning { semitones },
        }
    }

    /// A state whose pane shows `range` semitones around middle C over a
    /// `span`-second window, with the whole depth axis given to the roll.
    fn state(range: f32, span: f32) -> PictureState {
        turned(range, span, SpectralOrientation::Left)
    }

    /// The same pane with the names anchored on their onsets — the "Name the
    /// far end" setting, which in this Left-facing fixture is the onset and in
    /// a reversed orientation would be the leading edge.
    fn travelling(range: f32, span: f32) -> PictureState {
        let mut state = state(range, span);
        state.appearance.spectrum.note_names_travel = true;
        state
    }

    fn turned(range: f32, span: f32, orientation: SpectralOrientation) -> PictureState {
        let mut state = fresh();
        // These fixtures measure lattice spellings themselves. The shipped
        // comma locks are a view choice that would respell the same positions
        // before the naming rule under test sees them.
        state.appearance.view.meantone = false;
        state.appearance.view.marvel = false;
        state.appearance.spectrum = SpectrumConfig {
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
    fn labels(state: &PictureState, now: f64) -> Vec<NoteLabel> {
        labels_in(state, now, PANE)
    }

    fn labels_in(state: &PictureState, now: f64, rect: egui::Rect) -> Vec<NoteLabel> {
        let cfg = &state.appearance.spectrum;
        let axes = Axes::new(rect, cfg);
        let min_midi = cfg.low_midi;
        let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
        let scale = PitchScale { min_midi, max_midi, span: max_midi - min_midi };
        let split = super::super::axes::spectrum_share(cfg);
        plan_for_test(state, &axes, &scale, split, now, FLAT)
    }

    /// A phrase dense enough that its names have to compete for room: three
    /// pitches struck together every 0.9 seconds, for longer than the window
    /// holds. Feed only events that have happened by this frame, as the live
    /// tracker does; future notes are not valid scroll/zoom candidates.
    fn phrase(until: f64) -> PictureState {
        let mut state = state(24.0, 10.0);
        let mut t = 0.0;
        while t <= until {
            for (i, note) in [60u8, 62, 64].iter().enumerate() {
                let at = t + i as f64 * 0.11;
                if at <= until {
                    state.runtime.tracker.handle_event(on(at, *note));
                    if at + 0.25 <= until {
                        state.runtime.tracker.handle_event(off(at + 0.25, *note));
                    }
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
    fn blinks(state_at: impl Fn(f64) -> PictureState, label_scale: f32) -> usize {
        let mut seen: HashMap<(String, i64), Vec<usize>> = HashMap::new();
        let mut thinning = Thinning::default();
        for frame in 0..480 {
            let now = 14.0 + frame as f64 / 60.0;
            let state = state_at(now);
            let cfg = state.appearance.spectrum;
            let split = super::super::axes::spectrum_share(&cfg);
            let axes = Axes::new(BIG, &cfg);
            let labels = plan_with_thinning_for_test(
                &state,
                &axes,
                &scale_of(&state),
                split,
                now,
                zoomed(label_scale),
                &mut thinning,
            );
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
        let feed = |state: &mut PictureState, next: &mut usize, until: f64| {
            while *next < events.len() && events[*next].0 <= until {
                let (at, note, down) = events[*next];
                state.runtime.tracker.handle_event(if down { on(at, note) } else { off(at, note) });
                *next += 1;
            }
        };
        feed(&mut state, &mut next, start);
        assert!(
            state.runtime.tracker.roll().notes().count() >= harmonigraph_core::NoteRoll::MAX_NOTES,
            "the roll has to be AT its cap for this to be the test it says it is",
        );

        let cfg = state.appearance.spectrum;
        let axes = Axes::new(BIG, &cfg);
        let split = super::super::axes::spectrum_share(&cfg);
        let mut seen: HashMap<(String, i64), Vec<usize>> = HashMap::new();
        let mut thinning = Thinning::default();
        for frame in 0..480 {
            let now = start + frame as f64 / 60.0;
            feed(&mut state, &mut next, now);
            for label in plan_with_thinning_for_test(
                &state,
                &axes,
                &scale_of(&state),
                split,
                now,
                FLAT,
                &mut thinning,
            ) {
                seen.entry((label.name.to_string(), (label.at * 1000.0).round() as i64))
                    .or_default()
                    .push(frame);
            }
        }
        seen.values().map(|f| f.windows(2).filter(|w| w[1] != w[0] + 1).count()).sum()
    }

    /// Scrolling and roll eviction must not make a displaced winner reappear.
    #[test]
    fn a_name_never_blinks_out_and_back_as_the_roll_scrolls() {
        const ZOOMED: f32 = 2.23;
        // Vacuity guard: names must actually be competing here, or "nothing
        // blinked" is a statement about a pane with nothing to thin.
        let state = phrase(20.0);
        let cfg = state.appearance.spectrum;
        let split = super::super::axes::spectrum_share(&cfg);
        let axes = Axes::new(BIG, &cfg);
        let placed = plan_for_test(&state, &axes, &scale_of(&state), split, 20.0, zoomed(ZOOMED));
        let on_pane = state
            .runtime
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

        assert_eq!(blinks(phrase, ZOOMED), 0);
        assert_eq!(blinks(phrase, 1.0), 0, "...and at the dialled size");
        assert_eq!(blinks_at_the_roll_cap(), 0, "...and with the roll evicting as it plays");
    }

    fn said(labels: &[NoteLabel]) -> Vec<String> {
        labels.iter().map(|l| l.name.to_string()).collect()
    }

    fn scale_of(state: &PictureState) -> PitchScale {
        let cfg = &state.appearance.spectrum;
        let min_midi = cfg.low_midi;
        let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
        PitchScale { min_midi, max_midi, span: max_midi - min_midi }
    }

    #[test]
    fn surviving_names_are_disjoint_and_paint_in_musical_order() {
        for orientation in SpectralOrientation::ALL {
            for travel in [false, true] {
                let mut state = turned(24.0, 10.0, orientation);
                state.appearance.spectrum.note_names_travel = travel;
                for event in [on(1.0, 67), on(1.0, 64), on(2.0, 60), off(2.5, 60)] {
                    state.runtime.tracker.handle_event(event);
                }
                let names = labels(&state, 3.0);
                assert!(!names.is_empty());
                assert!(names
                    .windows(2)
                    .all(|w| (w[0].onset, w[0].pitch) <= (w[1].onset, w[1].pitch)));
                for (i, a) in names.iter().enumerate() {
                    assert!(
                        names[i + 1..].iter().all(|b| !a.rect.intersects(b.rect)),
                        "{orientation:?}"
                    );
                }
            }
        }
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
            state.runtime.tracker.handle_event(on(t, 60));
            state.runtime.tracker.handle_event(off(t + 0.5, 60));
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
        state.runtime.tracker.handle_event(on(2.0, 60));
        state.runtime.tracker.handle_event(off(6.0, 60));

        let placed = labels(&state, 10.0);
        assert_eq!(placed.len(), 1);
        let axes = Axes::new(PANE, &state.appearance.spectrum);
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

    /// The same claim as
    /// [`the_letter_lines_up_with_or_without_an_accidental`], but read off
    /// the glyphs [`draw`] actually queues through a real `egui::Context`, so
    /// it is about the letters a reader sees rather than about the boxes they
    /// were chosen in.
    ///
    /// What it holds is that the mark column cannot reach the letter's
    /// placement: the two names differ by a comma sign, and the drawn letter
    /// does not move. The former arithmetic-only test asked it of an estimate, where
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
                onset: 0.0,
                pitch: 60.0,
                rect: egui::Rect::NOTHING,
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
        state.runtime.tracker.handle_event(on(2.0, 60));
        state.runtime.tracker.handle_event(off(6.0, 60));

        let split = super::super::axes::spectrum_share(&state.appearance.spectrum);
        let axes = Axes::new(PANE, &state.appearance.spectrum);
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
        state.runtime.tracker.handle_event(on(1.0, 60));

        let at = |state: &PictureState, now| labels(state, now)[0].rect.min.x;
        let early = at(&state, 2.0);
        let later = at(&state, 4.0);
        assert_eq!(early, later, "held, the name holds its place while the ribbon grows");

        state.runtime.tracker.handle_event(off(4.0, 60));
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
        state.runtime.tracker.handle_event(on(1.0, 60));
        state.runtime.tracker.handle_event(off(1.0, 60));
        state.runtime.tracker.handle_event(on(1.0, 60));

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
        state.runtime.tracker.handle_event(on(0.0, 67)); // still held
        let placed = labels(&state, 100.0);
        assert_eq!(said(&placed), ["G"]);

        let axes = Axes::new(PANE, &state.appearance.spectrum);
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
            state.runtime.tracker.handle_event(on(1.0, 60));
            if let Some(t) = release {
                state.runtime.tracker.handle_event(off(t, 60));
            }
            state
        };
        let at = |state: &PictureState, now| labels(state, now)[0].rect.min.x;

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
        state.runtime.tracker.handle_event(on(2.0, 60));
        state.runtime.tracker.handle_event(off(6.0, 60));

        let placed = labels(&state, 10.0);
        assert_eq!(placed.len(), 1);
        let axes = Axes::new(PANE, &state.appearance.spectrum);
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
        // measured in the measured rectangle rather than anything about the anchor.
        for orientation in SpectralOrientation::ALL {
            for travel in [false, true] {
                let mut state = turned(24.0, 10.0, orientation);
                state.appearance.spectrum.note_names_travel = travel;
                state.runtime.tracker.handle_event(on(2.0, 60));
                state.runtime.tracker.handle_event(off(6.0, 60));

                // Square, so the same pane serves the vertical orientations.
                let square =
                    egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 320.0) };
                let placed = labels_in(&state, 10.0, square);
                assert_eq!(placed.len(), 1, "{orientation:?}, travel {travel}");

                // The ribbon's two ends, and how far the name sits from the one
                // it is anchored to along the depth axis — signed, so a name
                // laid the wrong way reads as a negative reach.
                let axes = Axes::new(square, &state.appearance.spectrum);
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
        state.appearance.spectrum.roll_fraction = 0.55; // the fresh value
        state.runtime.tracker.handle_event(on(5.0, 60));

        let axes = Axes::new(PANE, &state.appearance.spectrum);
        let split = super::super::axes::spectrum_share(&state.appearance.spectrum);
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
        state.runtime.tracker.handle_event(on(5.0, 60));
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
        state.appearance.spectrum.roll_fraction = 0.98;
        state.runtime.tracker.handle_event(on(5.0, 60));
        let axes = Axes::new(PANE, &state.appearance.spectrum);
        let split = super::super::axes::spectrum_share(&state.appearance.spectrum);
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
        state.appearance.spectrum.roll_fraction = 0.55; // the fresh value
        state.runtime.tracker.handle_event(on(5.0, 60));

        let axes = Axes::new(PANE, &state.appearance.spectrum);
        let split = super::super::axes::spectrum_share(&state.appearance.spectrum);
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
            state.runtime.tracker.handle_event(on(2.0, 60));
            state.runtime.tracker.handle_event(off(6.0, 60));

            // Square, so the same pane serves the vertical orientations.
            let square = egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 320.0) };
            let placed = labels_in(&state, 10.0, square);
            assert_eq!(placed.len(), 1, "{orientation:?}");

            // The ribbon's two ends — the release 4s back, the onset 8s back —
            // and the way a reader's eye runs over the pane: rightward where
            // time is across it, downward where time runs down it.
            let axes = Axes::new(square, &state.appearance.spectrum);
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
            let corners = [ink.left_top(), ink.right_top(), ink.left_bottom(), ink.right_bottom()];
            corners.iter().map(|&c| (c - from).dot(label.grow)).fold(f32::INFINITY, f32::min)
        };

        // Nothing clamped: a released note, named on its leading edge.
        let mut state = state(24.0, 10.0);
        state.runtime.tracker.handle_event(on(2.0, 60));
        state.runtime.tracker.handle_event(off(6.0, 60));
        let axes = Axes::new(PANE, &state.appearance.spectrum);
        for air in [0.5, 1.0, 2.0] {
            let placed = plan_for_test(
                &state,
                &axes,
                &scale_of(&state),
                0.0,
                10.0,
                NameScale { label: 1.0, air },
            );
            assert_eq!(placed.len(), 1);
            let gap = ink_from(&placed[0], axes.at(0.5, 0.4));
            assert!((gap - LABEL_INSET * air).abs() < 0.1, "air {air}, gap {gap}");
        }

        // Clamped: struck this instant at the far anchor, so the name is longer
        // than the ribbon under it and is held on the pane. The roll has the
        // whole depth axis here (`roll_fraction` 1.0), so the pane's own edge
        // is where the now-line is and there is nothing between them.
        let mut state = travelling(24.0, 10.0);
        state.runtime.tracker.handle_event(on(5.0, 60));
        for now in [5.0, 5.05, 5.1] {
            let placed = labels(&state, now);
            assert_eq!(placed.len(), 1, "at {now}s");
            let mut batch = crate::text::TextBatch::default();
            let _ =
                frame_full(&ctx, SCREEN, |ui| draw(ui.painter(), &placed, FLAT.label, &mut batch));
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

    #[test]
    fn a_held_travelling_name_keeps_priority_after_release() {
        let mut state = travelling(24.0, 10.0);
        for event in [on(1.0, 60), off(1.05, 60), on(1.1, 60)] {
            state.runtime.tracker.handle_event(event);
        }
        let axes = Axes::new(PANE, &state.appearance.spectrum);
        let split = super::super::axes::spectrum_share(&state.appearance.spectrum);
        let mut history = Thinning::default();
        let before = plan_with_thinning_for_test(
            &state,
            &axes,
            &scale_of(&state),
            split,
            2.0,
            FLAT,
            &mut history,
        );
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].onset, 1.1);
        state.runtime.tracker.handle_event(off(2.0, 60));
        let after = plan_with_thinning_for_test(
            &state,
            &axes,
            &scale_of(&state),
            split,
            2.5,
            FLAT,
            &mut history,
        );
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].onset, 1.1);
    }

    /// Coincident voices compete for one measured box at either anchor.
    #[test]
    fn one_pitch_from_two_voices_is_named_once_at_either_anchor() {
        let voiced = |travel: bool| {
            let mut state = if travel { travelling(24.0, 10.0) } else { state(24.0, 10.0) };
            // The same pitch on two channels, struck together and held — a
            // doubled source, or one MPE part layered over another.
            for channel in [0, 1] {
                state.runtime.tracker.handle_event(NoteEvent {
                    source: SourceId::DIRECT,
                    time: 1.0,
                    channel,
                    note: 60,
                    kind: NoteEventKind::On { velocity: 0.8 },
                });
            }
            state
        };
        assert_eq!(said(&labels(&voiced(false), 2.0)), ["C"], "held, at the leading edge");
        assert_eq!(
            said(&labels(&voiced(true), 2.0)),
            ["C"],
            "and travelling, through the same collision pass"
        );

        // ...and a press the host delivers as on/off/on at one sample is one
        // press at either anchor, the two entries sharing an onset.
        let pressed = |travel: bool| {
            let mut state = if travel { travelling(24.0, 10.0) } else { state(24.0, 10.0) };
            state.runtime.tracker.handle_event(on(1.0, 60));
            state.runtime.tracker.handle_event(off(1.0, 60));
            state.runtime.tracker.handle_event(on(1.0, 60));
            state
        };
        assert_eq!(said(&labels(&pressed(false), 1.5)), ["C"]);
        assert_eq!(said(&labels(&pressed(true), 1.5)), ["C"]);
    }

    /// A name leaves the far edge when its OWN BOX has left the pane — not
    /// before, whatever its ribbon is doing, and not after — in every
    /// orientation.
    ///
    /// Swept over [`SpectralOrientation::ALL`] because the orientation is what
    /// picks the anchor (see [`Anchor::of`]): the two that agree with the screen
    /// name a ribbon's leading edge, the two that reverse it name the onset. At
    /// a leading edge the name lies back over its ribbon and the two leave
    /// together. At an onset they part both ways: a note longer than its name
    /// keeps scrolling after the name has gone, and a note SHORTER than its name
    /// is gone first, the name lying past its end.
    ///
    /// That last case is the one this is for. Culled with its ribbon, such a
    /// name blinked out with most of itself still on the pane — the moment it
    /// reached the edge, and on nearly every note at the default Span, where a
    /// name is seconds of take time long. So what is asserted is the last frame
    /// the name is drawn: its box must already have left the pane (to within a
    /// step), or something that could still be seen was taken away; and it must
    /// have left only just, or the name was held back rather than scrolling.
    ///
    /// Three lengths, the shortest under a name's own reach, and the guard
    /// below proves the fixture gets there: at the onset that name outlasts its
    /// ribbon. The pane is SQUARE so the depth axis is one length in all four
    /// orientations; a name's own depth is not, being its width where time runs
    /// across the pane and a whole line box where it runs down it.
    #[test]
    fn a_name_leaves_the_far_edge_only_once_its_own_box_has() {
        let square = egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 320.0) };
        let step = 0.02;
        for orientation in SpectralOrientation::ALL {
            for length in [0.3f64, 4.0, 12.0] {
                let mut state = turned(24.0, 10.0, orientation);
                state.runtime.tracker.handle_event(on(1.0, 60));
                state.runtime.tracker.handle_event(off(1.0 + length, 60));

                let cfg = &state.appearance.spectrum;
                let axes = Axes::new(square, cfg);
                let split = super::super::axes::spectrum_share(cfg);
                let scale = scale_of(&state);
                let (mut named, mut drawn, mut last) = (f64::NAN, f64::NAN, None);
                let mut now = 1.0;
                while now < 40.0 {
                    if let Some(label) = labels_in(&state, now, square).first() {
                        named = now;
                        last = Some(label.rect);
                    }
                    let ribbons =
                        super::super::roll::note_instances(&axes, &scale, &state, split, now, 2.0);
                    if !ribbons.is_empty() {
                        drawn = now;
                    }
                    now += step;
                }
                let rect = last.expect("named at the onset at the latest");
                // How much of the box still stood on the pane on that last
                // frame, measured along the depth axis from the far edge to the
                // box's shallowest point. Negative once it has wholly left.
                let depth = axes.dir_depth();
                let reach = (rect.width() * depth.x).abs() + (rect.height() * depth.y).abs();
                let showing = (axes.at(0.5, 1.0) - rect.center()).dot(depth) + reach * 0.5;
                let travel =
                    (step / TimeAxis::new(&state, split, 0.0).seconds_per_point(&axes)) as f32;
                assert!(
                    showing <= travel + 1e-3,
                    "{orientation:?}, a {length} s note: the name went with {showing} points of \
                     its box still on the pane (name {named}, ribbon {drawn})",
                );
                assert!(
                    showing > -(LABEL_INSET + LABEL_PAD) - travel,
                    "{orientation:?}, a {length} s note: the name stayed {} points past the \
                     edge after its box had left (name {named}, ribbon {drawn})",
                    -showing,
                );
                // The note's stop leaves the far edge a window after it.
                let gone = 1.0 + length + 10.0;
                if orientation.is_time_reversed() && length < 1.0 {
                    assert!(
                        named > gone + step,
                        "{orientation:?}: the fixture is vacuous, the short note's name did not \
                         outlast its note's end leaving at {gone} (name {named})",
                    );
                }
            }
        }
    }

    /// A name survives its centre crossing the pitch edge while its ink and
    /// ribbon still reach the pane, and never outlasts the ribbon.
    ///
    /// Swept by panning the range across a picture held still, off the bottom
    /// and off the top. At the widest ribbon the bar allows, because the gap
    /// being measured IS the ribbon's half width: at the default 0.3 st it is
    /// 0.15 st, inside the slack. On [`BIG`], where a semitone is tens of points
    /// and the ribbon's outline — ink past its box that the name does not wait
    /// for — is a small part of one.
    #[test]
    fn a_name_leaves_the_pitch_range_with_its_ribbon() {
        for direction in [1.0f32, -1.0] {
            let mut state = state(24.0, 10.0);
            state.appearance.spectrum.roll_thickness = 2.0;
            state.runtime.tracker.handle_event(on(1.0, 60));
            state.runtime.tracker.handle_event(off(3.0, 60));

            let axes = Axes::new(BIG, &state.appearance.spectrum);
            let split = super::super::axes::spectrum_share(&state.appearance.spectrum);
            // How far the range has been panned, in semitones: the note's centre
            // is on the edge at 12, its ribbon's box clears it at 13.
            let (mut named, mut drawn) = (f32::NAN, f32::NAN);
            let mut shift = 10.0f32;
            while shift < 15.0 {
                let cfg = &mut state.appearance.spectrum;
                cfg.low_midi = 48.0 + direction * shift;
                cfg.high_midi = 72.0 + direction * shift;
                if !labels_in(&state, 5.0, BIG).is_empty() {
                    named = shift;
                }
                let scale = scale_of(&state);
                if !super::super::roll::note_instances(&axes, &scale, &state, split, 5.0, 2.0)
                    .is_empty()
                {
                    drawn = shift;
                }
                shift += 0.01;
            }
            assert!(drawn < 14.9, "the fixture is vacuous: the ribbon never left ({drawn})");
            let over = drawn - named;
            assert!(
                over >= 0.0,
                "panned {direction}: the name outlasted its ribbon by {} st (name {named}, \
                 ribbon {drawn})",
                -over,
            );
            assert!(
                named > 12.0 && named <= 13.0,
                "panned {direction}: name should leave once its own ink or ribbon has left (name {named}, ribbon {drawn})",
            );
        }
    }

    /// A travelling name goes off the far edge WITH the onset it is written on,
    /// sliding out under the pane rather than stopping against it.
    ///
    /// The moment the onset crosses is the one to watch, and there are three
    /// wrong things a name can do at it. It can POP — drawn whole one frame and
    /// gone the next, which is what culling on the anchor would do. It can PARK —
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
        state.runtime.tracker.handle_event(on(0.0, 67)); // still held, and never released

        let placed = labels(&state, 5.0);
        assert_eq!(said(&placed), ["G"], "on the pane, and named");
        let axes = Axes::new(PANE, &state.appearance.spectrum);
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
        let cfg = &state.appearance.spectrum;
        let split = super::super::axes::spectrum_share(cfg);
        let ribbons =
            super::super::roll::note_instances(&axes, &scale_of(&state), &state, split, 120.0, 2.0);
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
            state.appearance.spectrum.note_names_travel = !orientation.is_time_reversed();
            state.runtime.tracker.handle_event(on(1.0, 60)); // held for the whole sweep

            // Square, so the same pane serves the vertical orientations.
            let square = egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 320.0) };
            let axes = Axes::new(square, &state.appearance.spectrum);
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

    /// The same stable collision policy applies at the onset anchor.
    #[test]
    fn travelling_names_never_blink_out_and_back_either() {
        let travelling = |now: f64| {
            let mut state = phrase(now);
            state.appearance.spectrum.note_names_travel = true;
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
            state.runtime.tracker.handle_event(on(0.0, note));
            state.runtime.tracker.handle_event(off(0.5, note));
        }
        assert_eq!(said(&labels(&state, 5.0)), ["C"], "only the one inside 48..72");
    }

    /// Notes that have scrolled off the far end are not named either — their
    /// ribbons are gone, so a name would be labelling nothing.
    #[test]
    fn notes_that_have_left_the_window_are_left_out() {
        let mut state = state(24.0, 10.0);
        state.runtime.tracker.handle_event(on(0.0, 60));
        state.runtime.tracker.handle_event(off(0.5, 60));
        state.runtime.tracker.handle_event(on(8.0, 67));
        state.runtime.tracker.handle_event(off(8.5, 67));

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
            state.runtime.tracker.handle_event(on(t, 60));
            state.runtime.tracker.handle_event(off(t + 0.05, 60));
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
            // One clear gap, not two full gaps around each label.
            assert!(pair[1] - pair[0] < 45.0, "names sit two cells apart: {pair:?}");
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
                state.runtime.tracker.handle_event(on(t, 60));
                state.runtime.tracker.handle_event(off(t + 0.1, 60));
            }
            if let Some(t) = extra {
                state.runtime.tracker.handle_event(on(t, 60));
                state.runtime.tracker.handle_event(off(t + 0.1, 60));
            }
            state
        };
        let xs = |state: &PictureState| -> Vec<f32> {
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
            state.runtime.tracker.handle_event(on(t, 60));
            state.runtime.tracker.handle_event(off(t + 0.2, 60));
        }
        let tall = egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(110.0, 320.0) };
        let placed = labels_in(&state, 6.0, tall);
        assert!(placed.len() > 1, "several names: {}", placed.len());

        let axes = Axes::new(tall, &state.appearance.spectrum);
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
        // The red-band and name queries use this same frame-local lookup.
        // A matching band query stops early; a miss prepares the shown list,
        // which the subsequent fallback name then reads without rebuilding.
        let mut namer = Namer::new(&view, window, &just);
        let far_class = PitchClass::from_cents(midi.rem_euclid(12.0) * 100.0);
        assert!(namer.shows_node(far_class));
        assert!(namer.shown_nodes.is_none());
        let bent = PitchClass::from_cents(314.15);
        assert!(!namer.shows_node(bent), "the fixture must exercise a full off-node miss");
        assert!(namer.shown_nodes.is_some());
        assert!(namer.shows_node(just.pitch_class(LatticePos::ORIGIN)));
        assert_eq!(
            namer.name(midi).to_string(),
            crate::panes::display_note_name(far, view.tempered()).to_string(),
        );
        // Exercise the roll's prepared lookup too: it must populate the
        // distinct shown window when the naming reach has no match.
        let mut state = state(24.0, 10.0);
        state.appearance.view = view;
        state.runtime.tuning = just;
        state.surfaces.drawn = Some(window);
        state.runtime.tracker.handle_event(on(1.0, 60));
        state.runtime.tracker.handle_event(tuning(1.01, 60, midi - 60.0));
        state.runtime.tracker.handle_event(off(2.0, 60));
        assert_eq!(
            said(&labels_in(&state, 5.0, BIG)),
            [crate::panes::display_note_name(far, state.appearance.view.tempered()).to_string()],
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

    #[test]
    fn names_at_different_pitches_share_collision_space() {
        let mut state = state(24.0, 10.0);
        for note in 60..66 {
            state.runtime.tracker.handle_event(on(5.0, note));
            state.runtime.tracker.handle_event(off(5.2, note));
        }
        let small = labels(&state, 5.5);
        assert!(!small.is_empty() && small.len() < 6);
        for (i, a) in small.iter().enumerate() {
            assert!(small[i + 1..].iter().all(|b| !a.rect.intersects(b.rect)));
        }
        assert_eq!(labels_in(&state, 5.5, BIG).len(), 6);
    }

    #[test]
    fn a_held_note_wins_a_collision_with_a_released_note() {
        let mut state = state(24.0, 10.0);
        for event in [on(1.0, 60), off(1.9, 60), on(1.95, 60)] {
            state.runtime.tracker.handle_event(event);
        }
        let held = labels(&state, 2.0);
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].onset, 1.95);
        state.runtime.tracker.handle_event(off(2.0, 60));
        let released = labels(&state, 2.0);
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].onset, 1.0);
    }

    #[test]
    fn a_displaced_name_stays_retired_until_it_leaves_the_view() {
        let mut state = travelling(24.0, 10.0);
        for event in [on(1.0, 60), off(1.1, 60)] {
            state.runtime.tracker.handle_event(event);
        }
        let axes = Axes::new(PANE, &state.appearance.spectrum);
        let split = super::super::axes::spectrum_share(&state.appearance.spectrum);
        let mut history = Thinning::default();
        let first = plan_with_thinning_for_test(
            &state,
            &axes,
            &scale_of(&state),
            split,
            1.2,
            FLAT,
            &mut history,
        );
        assert_eq!(first[0].onset, 1.0);
        state.runtime.tracker.handle_event(on(1.2, 60));
        let next = plan_with_thinning_for_test(
            &state,
            &axes,
            &scale_of(&state),
            split,
            1.3,
            FLAT,
            &mut history,
        );
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].onset, 1.2);
        assert_eq!(history.suppressed.len(), 1);
        state.runtime.tracker.handle_event(off(1.4, 60));
        for now in [2.0, 3.0, 5.0] {
            let names = plan_with_thinning_for_test(
                &state,
                &axes,
                &scale_of(&state),
                split,
                now,
                FLAT,
                &mut history,
            );
            assert!(names.iter().all(|n| n.onset != 1.0));
        }
        let roomy = Axes::new(
            egui::Rect::from_min_size(PANE.min, egui::vec2(3000.0, 100.0)),
            &state.appearance.spectrum,
        );
        let expanded = plan_with_thinning_for_test(
            &state,
            &roomy,
            &scale_of(&state),
            split,
            5.0,
            FLAT,
            &mut history,
        );
        assert!(
            expanded.iter().any(|n| n.onset == 1.0),
            "view change did not restore a label with room"
        );
        plan_with_thinning_for_test(
            &state,
            &axes,
            &scale_of(&state),
            split,
            30.0,
            FLAT,
            &mut history,
        );
        assert!(history.visible.is_empty() && history.suppressed.is_empty());
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
        state.runtime.tuning = harmonigraph_core::Tuning::just();
        state.runtime.tracker.handle_event(on(1.0, 64));
        state.runtime.tracker.handle_event(tuning(1.01, 64, -0.137));
        state.runtime.tracker.handle_event(off(2.0, 64));

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
            state.runtime.tracker.handle_event(on(1.0, 70));
            state.runtime.tracker.handle_event(tuning(1.01, 70, early));
            state.runtime.tracker.handle_event(off(2.0, 70));
            state.runtime.tracker.handle_event(on(5.0, 70));
            state.runtime.tracker.handle_event(tuning(5.01, 70, late));
            state.runtime.tracker.handle_event(off(6.0, 70));
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
        let name = |midi| {
            let name = note_name(&view, &view.reach(), &equal, midi).to_string();
            let mut state = state(24.0, 10.0);
            state.appearance.view = view.clone();
            state.runtime.tuning = equal;
            state.runtime.tracker.handle_event(on(1.0, midi as u8));
            state.runtime.tracker.handle_event(off(2.0, midi as u8));
            assert_eq!(said(&labels_in(&state, 5.0, BIG)).as_slice(), std::slice::from_ref(&name));
            name
        };

        assert_eq!(name(60.0), "C", "the origin, not a remote spelling of it");
        assert_eq!(name(67.0), "G", "a fifth up");
        assert_eq!(name(65.0), "F", "a fifth down");
        // Four fifths up spells E; one just third up spells E-, and in this
        // tuning they are the same pitch. The plain letter wins.
        assert_eq!(name(64.0), "E");
        assert_eq!(name(66.0), "F\u{266F}", "the prepared lookup keeps the spelling tiebreak");
    }

    #[test]
    fn a_septimal_mark_is_in_the_measured_collision_box() {
        let plain = NoteName { letter: 'B', sharps: -1, syntonic_commas: 0, septimal_commas: 0 };
        let ctx = themed_at(2.0);
        let mut widths = Vec::new();
        frame_full(&ctx, SCREEN, |ui| {
            for commas in [0, -1, -5] {
                let mut batch = crate::text::TextBatch::default();
                draw_name(
                    &mut batch,
                    ui.painter(),
                    egui::Pos2::ZERO,
                    NoteName { septimal_commas: commas, ..plain },
                    1.0,
                    egui::Vec2::X,
                );
                widths.push(batch.bounds().width());
            }
        });
        assert!(widths[1] > widths[0] + 2.0);
        assert!(widths[2] > widths[1]);
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
        let view = harmonigraph_scene::ViewConfig {
            min_sevens: -1,
            max_sevens: 1,
            meantone: false,
            marvel: false,
            ..Default::default()
        };
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
        state.runtime.tracker.handle_event(on(0.0, 60));
        assert_eq!(labels(&state, 1.0).len(), 1);

        state.appearance.spectrum.note_names = false;
        assert!(labels(&state, 1.0).is_empty());

        state.appearance.spectrum.note_names = true;
        state.appearance.spectrum.show_roll = false;
        state.appearance.spectrum.show_spectrogram = true;
        state.appearance.spectrum.roll_fraction = 0.55;
        assert!(labels(&state, 1.0).is_empty(), "no ribbons, so nothing to name");
    }

    /// With the far region shut there is nowhere to draw, and `plan` says so
    /// rather than collapsing every name onto the edge.
    #[test]
    fn a_shut_roll_region_draws_no_names() {
        let mut state = state(24.0, 10.0);
        state.runtime.tracker.handle_event(on(0.0, 60));
        // The divider dragged all the way over: the spectrum owns everything.
        state.appearance.spectrum.roll_fraction = 0.0;
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
        state.runtime.tracker.handle_event(on(1.0, 60));
        // Glided up a fifth over a second, and still held.
        state.runtime.tracker.handle_event(tuning(2.0, 60, 7.0));

        let placed = labels(&state, 3.0);
        assert_eq!(placed.len(), 1);
        let axes = Axes::new(PANE, &state.appearance.spectrum);
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
        out.runtime.tracker.handle_event(on(1.0, 60));
        out.runtime.tracker.handle_event(tuning(1.5, 60, 30.0)); // gone to MIDI 90
        assert!(labels(&out, 3.0).is_empty(), "its name left with it");

        let mut into = state(24.0, 10.0);
        into.runtime.tracker.handle_event(on(1.0, 30));
        into.runtime.tracker.handle_event(tuning(1.5, 30, 30.0)); // arrived at MIDI 60
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
            state.runtime.tracker.handle_event(on(t, 60));
            state.runtime.tracker.handle_event(off(t + 0.1, 60));
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
        st.runtime.tracker.handle_event(on(10.0, 60));
        st.runtime.tracker.handle_event(off(10.3, 60));

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
                        let scales = NameScale { label: zoom, air: 1.0 };
                        let label = NoteLabel {
                            name,
                            onset: 0.0,
                            pitch: 60.0,
                            rect: egui::Rect::NOTHING,
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
                        let corners = [
                            ink.left_top(),
                            ink.right_top(),
                            ink.left_bottom(),
                            ink.right_bottom(),
                        ];
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
}
