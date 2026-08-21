//! The Spectral pane's piano roll: incoming MIDI drawn as ribbons over
//! the same pitch axis the spectrum uses.
//!
//! Not a piano roll in the DAW sense — there are no black/white key lanes
//! and no bar grid, because the pitch axis is the *lattice's* axis: it is
//! continuous in cents, so a bent or microtonally tuned note sits between
//! the keys rather than being quantized onto one. What it keeps from the
//! DAW idea is the shape: pitch across, time along, one ribbon per note.
//!
//! Geometry comes entirely from [`Axes`], so the roll
//! turns with the rest of the pane and this file never names a screen side.
//! Its share of the depth axis runs from `split` (now) to 1 (the
//! oldest note still on screen), so time flows *away* from the spectrum
//! and a note crossing the split meets the peak it is making. A SOUNDING note
//! carries a little way past that line and fades out there, which is the one
//! thing the roll draws outside its own share — see [`lead`], and
//! [`lead_alpha`] for what becomes of it at the release.

use egui::Color32;
use harmonigraph_core::RollNote;
use harmonigraph_render::{RollAxes, RollInstance};
use harmonigraph_scene::pitch_lut_color;

use super::axes::{Axes, PitchScale, TimeAxis};
use crate::panes::scene_color;
use crate::SharedState;

/// Narrowest a note may draw across PITCH, in points. Note width is in
/// SEMITONES of the pitch axis, so a wide zoom takes a ribbon under a pixel
/// and a filled rectangle simply disappears; the floor is what keeps a played
/// note visible at every zoom, at the cost of the widths below it reading
/// alike.
const MIN_RIBBON_PX: f32 = 1.5;

/// Shortest a note may draw along TIME, in DEVICE pixels — the floor that stops
/// a brief note flickering as the roll scrolls.
///
/// The same shape of problem as [`MIN_RIBBON_PX`] on the other axis, with a
/// sharper threshold, because this is the axis a note MOVES along. The shader
/// antialiases with a one-pixel box filter (`band`/`inside` in roll.wgsl),
/// which conserves a shape's total ink under any sub-pixel offset but not its
/// PEAK — and peak is what the eye reads on something a pixel or two across.
/// Measured on the real pipeline, sweeping a note through eight sub-pixel
/// offsets and taking the spread of its brightest pixel:
///
/// | drawn length | peak spread | total ink spread |
/// |--------------|-------------|------------------|
/// | 4 px         | 0%          | 0.0%             |
/// | 2 px         | 0%          | 0.2%             |
/// | 1.5 px       | 25%         | 0.3%             |
/// | 1 px         | 50%         | 0.4%             |
/// | 0.5 px       | 67%         | 33%              |
/// | 0.2 px       | 83%         | 66%              |
///
/// Two pixels is where it goes to zero, and the reason is exact rather than
/// empirical: the coverage profile is a trapezoid whose ramps are one pixel
/// wide, so its flat top is `length - 1` pixels across and a sample lands on
/// the top for every offset once that reaches one pixel. Under two the top
/// shrinks, some offsets miss it, and the note pulses at whatever rate it is
/// scrolling. Under one pixel even the ink stops being conserved and it
/// pulses in brightness AND in weight.
///
/// In DEVICE pixels, unlike [`MIN_RIBBON_PX`]'s points, because that is what
/// the argument is about — the filter is one physical pixel wide, so on a 2x
/// display this is one point and on a 1x display two. A floor in points would
/// be right on exactly one class of display.
///
/// The cost is a note drawn longer than it lasted, up to this, and at a long
/// Span that is a real overstatement — two pixels of a ten-minute Span is four
/// seconds. It is the trade [`MIN_RIBBON_PX`] already makes: past the point
/// where a zoom can resolve one note from the next, what the roll owes the
/// reader is that a note was played, not how long it was held.
///
/// A BENT segment pays a second time, in its shear: the box it is drawn in is
/// longer than the drift it carries, so the drift spreads over the floored
/// length and reads as a shallower bend. That is deliberate, and the
/// alternative is much worse — see the shear in [`note_instances`], where the
/// two are one product and holding the rate instead puts ink at pitches
/// nothing sounded.
const MIN_LENGTH_DEVICE_PX: f32 = 2.0;

/// The light edge drawn along the spectrum's profile, at `cfg.keyline`
/// strength — `None` when the setting is off or too faint to be worth a shape.
///
/// The curve takes its colors from the spectrogram gradient, so where it is
/// quiet it is drawn at that gradient's dark end against the pane's dark
/// background, with no edge, and the shape stops existing. A light rim gives
/// it an edge to be seen by. It is a setting because how much is right depends
/// entirely on the gradient in play — one running to white swallows a rim that
/// a dimmer or more colored one leaves standing, and the bars reach both.
///
/// The threshold is a fade-out floor: below it the line is too faint to be
/// worth a shape at all.
///
/// The profile's edge alone. A note's is the outline [`outline`] draws, which
/// is a dark surround with its own reach and fade — the profile is one line on
/// a filled slab rather than a shape to pick out of a picture, and needs
/// neither.
pub(super) fn keyline(cfg: &crate::SpectrumConfig, alpha: f32) -> Option<Color32> {
    let strength = cfg.keyline.clamp(0.0, 1.0) * alpha;
    (strength > 0.004).then_some(Color32::WHITE.gamma_multiply(strength))
}

/// The dark surround standing outside a note: `(reach, fade, color)` — how far
/// past the note's edge it goes, how much of that it spends fading out, and
/// what it is at full coverage.
///
/// BLACK, and not a setting. The outline's job is to give a note a boundary
/// against a heatmap, and the pane's own background is the color that tells
/// against the most of one: every preset opens at `L*` 0 and climbs, so black
/// is what the cells are furthest from nearly everywhere. A note then reads as
/// an object sitting over the picture, with the picture darkened around it. A
/// colored surround would be a second hue competing with the ribbon's own,
/// which is the one thing on a note that carries meaning.
///
/// A gradient dark at BOTH ends is reachable on the bars, and there this
/// separates nothing — a picture someone dialled rather than one the pane can
/// be left in by accident, and the ribbon still carries its own color where
/// the surround has stopped saying anything.
///
/// Opaque where it is solid, and translucent only through the fade — never a
/// flat tint. A uniformly translucent surround takes its color from whatever is
/// behind it, so the same note comes out with a different edge over a loud cell
/// than over a quiet one, and washes out entirely against the bright end of a
/// palette — the one place a note most needs an edge, since that is the cell the
/// note itself is making. The fade is a shape's ending rather than a strength:
/// it is opaque against the note and gone at its reach, wherever it is drawn.
///
/// Neither number scales with anything: the reach is in points, so it is the
/// same edge at every zoom and every ribbon width. That is what makes the
/// outline a constant of the picture rather than a reading of it — see
/// [`SpectrumConfig::roll_outline`](crate::SpectrumConfig) for the trade, which
/// is real and is at the wide end.
///
/// Off comes back zero-reach AND transparent, never one or the other: the reach
/// is what the quad grows by to make room for the outline (and what the
/// far-edge cull keeps a leaving note alive for), so an outline that will not
/// paint must not be paid for either.
fn outline(cfg: &crate::SpectrumConfig) -> (f32, f32, Color32) {
    let reach = cfg.roll_outline.max(0.0);
    if reach <= 0.0 {
        return (0.0, 0.0, Color32::TRANSPARENT);
    }
    (reach, cfg.roll_outline_fade.clamp(0.0, reach), Color32::BLACK)
}

/// How far a SOUNDING note's ribbon carries past the now-line, and how much of
/// that it spends fading out: `(reach, fade)` in POINTS, both 0 where there is
/// no line to cross.
///
/// A sounding note ends ON the now-line, and the picture on the other side of
/// it is the spectrum peak that note is making. The lead is what carries the
/// ribbon across the join rather than stopping it square: a stretch of the
/// note's own color over the divider, dissolving into the curve. Which notes
/// are down then reads off the picture at a glance, in the one place the pane
/// already puts the answer.
///
/// The setting is a FRACTION OF THE ANALYZER and this is where it becomes a
/// length: the spectrum owns `0..split` of the depth axis, so its share in
/// points is `split * depth_len` and the lead is the set share of that. Two
/// things fall out, and both are the point rather than a side effect — the
/// tongue is the same share of the picture at every pane size, and it shrinks
/// with the spectrum as the divider is dragged over it, so it can never be more
/// of the analyzer than the bar says. See
/// [`SpectrumConfig::roll_lead`](crate::SpectrumConfig) for why the outline
/// beside it is measured the other way.
///
/// The divider is drawn AFTER the roll and is not chewed by this — see
/// `spectral_pane`, where the reasoning for that order is set out. A lead
/// passes UNDER an unbroken hairline, which is what keeps the boundary
/// followable while a chord is reaching over it.
///
/// Zero where the pane has no spectrum to reach into, which is two layouts and
/// not a guard against a bad number:
///
///   - Whole-song (the offline `--playhead` render) hands the roll the entire
///     depth axis and draws no spectrum at all, so `split` is 0 and there is no
///     line anywhere on the pane to cross.
///   - The divider dragged all the way over the curve leaves the same picture
///     from the other direction.
///
/// Both fall out of the arithmetic rather than needing a test of their own —
/// a `split` of 0 is a spectrum of no length, and the share of nothing is
/// nothing — but the guard is kept because `whole_song` does not always mean a
/// zero split to the reader, and because a lead is a claim about a picture that
/// is not being drawn at all there.
///
/// Off comes back zero-reach AND zero-fade, never one or the other, for
/// [`outline`]'s reason: the reach is what the geometry grows by, so a lead
/// that will not be drawn must not be paid for in a fade either.
pub(super) fn lead(
    cfg: &crate::SpectrumConfig,
    axes: &Axes,
    whole_song: bool,
    split: f32,
) -> (f32, f32) {
    if whole_song || split <= 0.0 {
        return (0.0, 0.0);
    }
    let spectrum_px = split * axes.depth_len();
    let reach = cfg.roll_lead.max(0.0) * spectrum_px;
    if reach <= 0.0 {
        return (0.0, 0.0);
    }
    (reach, (cfg.roll_lead_fade.max(0.0) * spectrum_px).min(reach))
}

/// How much of a note's lead is still standing, 0..1: all of it while the key
/// is down, then falling to nothing over the Lead release once it comes up.
///
/// A ramp in TAKE TIME rather than in frames, so a release lasts as long
/// offline as it does live and the same take renders the same picture at any
/// fps — which is what the whole draw path is built to keep true.
///
/// Linear, and that is worth one line because a fade usually should not be: a
/// curve here would be shaping an OPACITY over a few hundred milliseconds
/// against a spectrogram whose own brightness is moving underneath it, where
/// nothing in the picture is stable enough for the difference to be read. The
/// lattice's envelope is the place a curve earns its keep, and it has one.
///
/// A release of 0 takes the lead the instant the note stops, which is the pop
/// the setting exists to answer, and is still what someone dialling the bar to
/// its bottom is asking for.
fn lead_alpha(note: &RollNote, now: f64, release: f32) -> f32 {
    let Some(end) = note.end else {
        return 1.0;
    };
    if release <= 0.0 {
        return 0.0;
    }
    1.0 - ((now - end) / f64::from(release)).clamp(0.0, 1.0) as f32
}

/// Draw every remembered note that falls inside the pane's time window and
/// pitch range. `split` is the depth fraction the roll starts at; `now` is
/// the shell clock, the same one the tracker's events are stamped with.
/// `surface` names the roll (0 the docked pane / offline render, 1 the
/// Render preview), so two live copies get their own instance buffer.
///
/// One paint callback for the whole roll, drawing one instanced quad per
/// note segment — NOT a stack of stroked rounded rects through egui's
/// tessellator. The roll is a scrolling picture of immutable content, and
/// immediate mode re-uploads every vertex of it every frame: 100k+
/// vertices, 4-5 ms of pure upload, the dominant cost of the frame. Four
/// vertices a note and a distance field for the bands costs neither. See
/// `harmonigraph_render::roll` for the measurement and the shape of the fix.
pub(super) fn draw_roll(
    painter: &egui::Painter,
    axes: &Axes,
    scale: &PitchScale,
    state: &SharedState,
    split: f32,
    now: f64,
    surface: usize,
) {
    let ppp = painter.ctx().pixels_per_point().max(1.0);
    let notes = note_instances(axes, scale, state, split, now, ppp);
    if surface == 0 {
        // What the roll costs, for the performance overlay: this geometry
        // does not pass through egui's vertex buffer, so the `verts` row
        // cannot see it. The Render preview is a second roll and does
        // not publish (see `Instruments::roll_notes`).
        state
            .instruments
            .roll_notes
            .store(notes.len() as u32, std::sync::atomic::Ordering::Relaxed);
    }
    if notes.is_empty() {
        return;
    }
    // The roll's own share of the pane, and the whole of what its ink may
    // reach. The outline stands off EVERY side of a note, so a note sounding
    // now — whose leading end is the now-line itself — paints its edge across
    // the line and into the spectrum unless something stops it, and how far the
    // roll may draw on the spectrum is the [`lead`]'s decision and no one
    // else's. The clip is what holds it to that: a sounding note reaches its
    // lead past the line and not a point further, and with no lead set its
    // ribbon ends square on the line as everything the roll draws around it
    // does.
    //
    // The lead is in POINTS and the axis in fractions, so it converts through
    // the axis' own length. Floored at the pane's near edge, which costs
    // nothing — the pane's clip cuts there anyway — and keeps the bloom chain
    // this rect also sizes from spending resolution off the pane.
    //
    // It bounds the far end and the pitch range too, which the pane's own clip
    // already did — a note is deliberately allowed to overhang the window's
    // oldest edge and slide out under the scissor (see `note_instances`), and
    // this is the same edge in the same place.
    let (lead_px, _) = lead(&state.spectrum_config, axes, state.whole_song.is_some(), split);
    let near = (split - lead_px / axes.depth_len().max(1.0)).max(0.0);
    let region = egui::Rect::from_two_pos(axes.at(0.0, near), axes.at(1.0, 1.0));
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(region));
    let dir = |v: egui::Vec2| [v.x, v.y];
    painter.add(harmonigraph_render::roll_paint_callback(
        region,
        notes,
        RollAxes { pitch_dir: dir(axes.dir_pitch()), depth_dir: dir(axes.dir_depth()) },
        // The LATTICE's bloom, on the roll's notes. One setting for both
        // pictures rather than a second bar here: the two are showing the same
        // notes in the same colors, and a light a node has that its ribbon does
        // not is a difference between them that says nothing. Through the
        // renderer's own bound for the same reason — a strength the lattice
        // clamps and the roll does not is that difference in the other
        // direction.
        harmonigraph_render::bloom_strength(state.view.bloom_strength),
        state.target_format,
        crate::panes::lattice::pane_id(surface),
    ));
}

/// Every visible note segment as one GPU instance, in paint order.
///
/// The whole geometry of the roll, and the only place it is decided — the
/// shader turns an instance into pixels but invents nothing. Split out from
/// [`draw_roll`] so the geometry can be read back in tests without a GPU.
///
/// Rebuilt from scratch every frame, deliberately: see the note on
/// append-and-evict in `harmonigraph_render::roll`. It also keeps the far-edge
/// truncation below honest — a note crossing the window's oldest edge has
/// its geometry rewritten while it leaves, so it is not the immutable thing
/// a cache would need it to be.
pub(super) fn note_instances(
    axes: &Axes,
    scale: &PitchScale,
    state: &SharedState,
    split: f32,
    now: f64,
    // Physical pixels per point, which [`MIN_LENGTH_DEVICE_PX`] is quoted in.
    ppp: f32,
) -> Vec<RollInstance> {
    let cfg = &state.spectrum_config;
    // Shared time<->depth mapping: a `now`-anchored scrolling window live, or
    // the whole take laid out statically (offline playhead mode).
    let time = TimeAxis::new(state, split, now);
    let oldest = time.oldest();

    // Every note in the roll is the same width, so this is decided once for
    // the whole build rather than per segment. Floored rather than switched to
    // a bare line: note width is in SEMITONES, so a wide zoom takes a ribbon
    // under a pixel, where a rectangle fades out to nothing and the roll stops
    // saying a note was played there.
    let half_pitch = (cfg.roll_thickness * 0.5 / scale.span).max(0.0) * axes.pitch_len();
    let half_pitch = half_pitch.max(MIN_RIBBON_PX * 0.5);
    // The other axis' floor, in points at this display's density — the one that
    // stops a brief note pulsing as it scrolls. See [`MIN_LENGTH_DEVICE_PX`].
    let min_half_depth = 0.5 * MIN_LENGTH_DEVICE_PX / ppp.max(1e-3);

    // Build in START order. The tracker hands notes back finished-first and
    // then sounding, which is release order followed by key order — stable,
    // but not this one. Instances rasterize in buffer order, so where two
    // translucent notes overlap the paint order is visible, and the comparator
    // has to be TOTAL or the sort is free to vary between runs: a retrigger
    // with no intervening note-off puts two entries at the same start, which
    // is what the channel and note tails settle.
    let roll = state.roll();
    // The whole look of a note, decided once for the roll rather than per
    // note: the outline standing outside it. The note itself is a solid
    // rectangle of its own color and has nothing else to decide.
    let (outline_px, outline_fade_px, outline_color) = outline(cfg);
    // ...and how far a SOUNDING note carries past the now-line, which is
    // decided once for the same reason. See [`lead`]. How much of that a
    // particular note still has is [`lead_alpha`]'s, and is per note: it is a
    // reading of that note's own release.
    let (lead_px, lead_fade_px) = lead(cfg, axes, time.whole_song(), split);
    // The antialiasing ramp the shader feathers every edge with — one physical
    // pixel, in points at this display's density, the same figure it takes as
    // `feather`. Ink reaches half of it past whatever it edges.
    let feather_px = 1.0 / ppp.max(1e-3);

    // Cull to the visible window BEFORE sorting: the roll can remember
    // thousands of notes while only a handful are on screen, and sorting the
    // survivors alone (rather than every remembered note) keeps the same
    // deterministic paint order for far less work.
    //
    // On TIME, and only on time — the pitch test is per segment, below, for a
    // reason the margin here cannot fix.
    //
    // A note paints past its own box, and the box is what the window is tested
    // against — so without an overhang a note vanishes while that ink is still
    // owed, popping out of existence short of the edge instead of sliding under
    // it. In time that overhang is a screen-space length divided by the roll's
    // own length, so it grows with the Span: ~200 ms across 10 s, over a second
    // across a minute.
    //
    // The far edge moves out by exactly that, for the cull AND for the clamp
    // below, so the box overhangs and the pane's scissor takes the ink off as
    // it leaves. Bounded on purpose — an unclamped overhang is what makes a
    // ten-minute note a ten-minute quad.
    //
    // Not in whole-song mode. That layout is static: nothing scrolls, so no
    // note ever crosses an edge and there is no pop to prevent. What an
    // overhang would buy there is a note held from before the render started
    // beginning a few points outside the region rather than square on its
    // edge, and the render starts where it starts.
    //
    // `min_half_depth` is in the overhang because the length floor grows a
    // leaving note's box back toward the region as its true length is truncated
    // to nothing — so without it the last sliver of a floored note would appear
    // INSIDE the far edge on the frame before the cull takes it, which is the
    // pop the overhang exists to prevent.
    // The outline and the ramp: the outline wraps the note's ENDS as well as
    // its flanks, so a note's ink runs the outline's whole reach past its box
    // along this axis too, and half a ramp past that.
    let ink_px = min_half_depth + outline_px + 0.5 * feather_px;
    // Seconds per point of the roll — the pane's depth axis is shared with the
    // spectrum, so it is the ROLL's share of it that a point is measured
    // against. What every screen-space length here (the ink overhang) is
    // converted through.
    let per_point = time.seconds_per_point(axes);
    // One POINT of depth back from the present moment, signed: what a length
    // in points is multiplied by to move a box INTO THE PAST. Read off the
    // mapping rather than assumed, because the two modes run time opposite ways
    // along this axis — the live window pins `now` to the near edge and scrolls
    // history outward, while the whole-song layout starts the take at that edge
    // and runs it forward.
    let into_past = time.depth_of_unclamped(now - per_point) - time.depth_of_unclamped(now);
    let ink_seconds = if time.whole_song() { 0.0 } else { f64::from(ink_px) * per_point };
    let edge = oldest - ink_seconds;
    // Across pitch there is no margin to be had at the NOTE's level, and that
    // is the whole reason the test moved down into the loop.
    //
    // How far a segment's outline stands out along pitch is `skew * reach`,
    // where `skew` is a reading of that ONE segment's slope (see the reach in
    // [`note_instances`]'s loop, and `vs_note`) — so a note's pitch endpoints,
    // which are all a filter here can see, do not bound it. A segment can be
    // steeper than the note it belongs to, and its drawn length has no floor of
    // its own: the floor is the note's, so a note long enough to draw honestly
    // can still carry a segment a hundredth of a point long across a semitone,
    // which per-note tuning produces routinely. A flat margin of the outline's
    // own reach, as this was, drops such a glide while `(skew - 1)` times that
    // reach of its outline is still owed inside the range.
    //
    // What that costs is sorting the notes that are inside the WINDOW but off
    // the octave zoom — a comparison each. What it buys is a cull that is
    // exact instead of one that is wrong by however steep the picture is.
    let mut notes: Vec<&RollNote> =
        roll.notes().filter(|note| note.stop(now) >= edge).collect();
    notes.sort_unstable_by(|a, b| {
        a.start
            .total_cmp(&b.start)
            .then(a.channel.cmp(&b.channel))
            .then(a.note.cmp(&b.note))
    });

    // One segment per note is the common case (a note is bent rarely), so the
    // note count is the right first guess at how many instances this makes.
    let mut instances = Vec::with_capacity(notes.len());
    for note in notes {
        // The length floor is the NOTE's, not each segment's.
        //
        // A note's segments tile its own span end to end, so flooring them one
        // by one makes consecutive boxes OVERLAP by nearly the whole floor —
        // and an overlap is the later segment's rim painted over the earlier
        // one's color, which draws a bent note as a ladder of rim rather than a
        // ribbon. A note bent under per-note tuning is the ordinary case: the
        // tuning lands a block after the note-on (`RollNote::SETTLE`), so the
        // opening segment is milliseconds long and the floor is many times its
        // length.
        //
        // Stretched about the note's own midpoint instead, the segments still
        // tile: the floor decides how much of the depth axis the NOTE covers,
        // and never how much of one segment another segment covers too. A note
        // long enough to draw honestly is left alone, however brief its
        // segments are — a segment shorter than a pixel inside a long ribbon is
        // covered by its neighbours, which is the flicker the floor exists for.
        let opens = time.depth_of_unclamped(note.start.max(edge));
        let closes = time.depth_of_unclamped(note.stop(now).max(edge));
        let mid = (opens + closes) * 0.5;
        let span_px = ((closes - opens) * axes.depth_len()).abs();
        // A note with no duration at all — pressed this frame — has nothing to
        // scale up, so the floor reaches its one segment directly instead.
        let zero_span = span_px <= 1e-6;
        let stretch =
            if zero_span { 1.0 } else { (2.0 * min_half_depth / span_px).max(1.0) };
        // What the note covers either side of its own midpoint once drawn, in
        // points: the floor where it is too brief to have a length of its own,
        // its own half span where it is long enough to draw honestly.
        let half_true = 0.5 * span_px;
        let half_drawn = half_true.max(min_half_depth);
        // Where the note's own stop sits, in points into the past from `now`.
        // Negative only for a note that has not been played yet, which is
        // something only the whole-song layout has.
        let stop_px = ((now - note.stop(now)) / per_point) as f32;
        // The floor may lengthen a note into the past; it may not carry ink
        // into the FUTURE, past the moment the picture calls now.
        //
        // A note still sounding is where that bites hardest, because its stop
        // IS `now`, which is exactly where the now-line is drawn: a floor
        // centered on it puts half of itself on the SPECTRUM's side of the
        // join, under an opaque one-point hairline. On the 2x display the
        // plugin is used on, the floor is one point and the line covers the
        // whole of it, so a just-pressed note is invisible until it has
        // scrolled clear — at a long Span still true a second later, and the
        // floor exists precisely so that "a note was played" stays visible. At
        // 1x the floor is two points against the same line and what survives is
        // half a point of note color stranded on the far side of it, a dash
        // detached from its own ribbon in the spectrum's territory. A render
        // draws at that density whenever its frame is no wider than
        // `default_scale`'s reference, or `--scale 1` is asked for.
        //
        // A CLAMP rather than a re-anchoring, and the difference is the whole
        // of why this is one expression. The centering is right everywhere it
        // does not cross the line, and a note's box is left on its own midpoint
        // for as long as the floor fits behind `now` — so this binds by
        // exactly the overshoot and by nothing else, and lets go continuously
        // as the note scrolls clear.
        //
        // Measured from the note's own STOP rather than gated on whether the
        // key is still down, and that is not a generalization for its own sake:
        // a gate would spend the whole clamp in the single frame the key comes
        // up, snapping a staccato note back across the line by half a floor and
        // handing #239 straight back to every note short enough to need the
        // floor at all. The stop moves continuously through the release, so
        // this does too.
        //
        // What it costs is an onset reading up to one floor OLDER than it is —
        // the same overstatement the floor already makes about a note's LENGTH,
        // and the truer of the two statements either way: a note has not
        // sounded into the future, and drawing it as though it had is what put
        // ink on the spectrum's side of the boundary to begin with.
        //
        // A note whose stop is AHEAD of now is left alone, which is the
        // whole-song layout's case and not a live one. That layout draws the
        // take's future as well as its past, so a note the playhead has not
        // reached has not been played and belongs where it is; clamping it
        // would drag the rest of the take onto the playhead and pop each note
        // forward as the sweep passed it.
        //
        // The LEAD still crosses, and is measured from the clamped end: that is
        // a distance a setting asks for, drawn as a tongue that fades out, and
        // not a floor's rounding landing in the middle of the analyzer.
        let shift = if stop_px >= 0.0 {
            (half_drawn - half_true - stop_px).max(0.0) * into_past
        } else {
            0.0
        };
        // How much lead this note still owns: all of it while the key is down,
        // and whatever its release has left once it is up. Per note and decided
        // once for it, since every segment of a note releases together.
        //
        // NOT `alpha`, which the segment loop below already uses for the note's
        // own opacity — a different number about a different part of the
        // picture, and one that shadows this where the two meet.
        let standing = lead_alpha(note, now, cfg.roll_lead_release);
        // Peekable so the loop can tell which segment is the LAST, which is the
        // one the lead extends: a note's segments run oldest first, so its
        // leading end is the far end of the last of them. The rest end on the
        // next bend, in the middle of the ribbon, where there is nothing in
        // front of them to lead into.
        let mut segments = note.segments(now).peekable();
        while let Some(((t0, p0), (t1, p1))) = segments.next() {
            let last = segments.peek().is_none();
            let (t0, t1) = (t0.max(edge), t1.max(edge));
            if t1 < edge {
                continue;
            }
            // Unclamped: `edge` already bounds how far past the region these
            // can reach, and clamping is what squashed the leaving ribbon
            // against the far end rather than letting it slide out.
            let (d0, d1) = (time.depth_of_unclamped(t0), time.depth_of_unclamped(t1));
            // The note's floor, carried to this segment: an affine map about
            // the note's midpoint, so a brief note reads half the floor either
            // side of the moment it was rather than being pushed off it in one
            // direction, and the segments keep their proportions inside it.
            // Then `shift`, which takes back however much of that centering
            // would have landed past `now` — nothing, for a note far enough
            // from the line to have the room. See it above.
            let (d0, d1) =
                (mid + (d0 - mid) * stretch + shift, mid + (d1 - mid) * stretch + shift);
            let (a0, a1) = (scale.t_of(p0), scale.t_of(p1));

            // Whether this segment carries a lead at all — which is not the
            // same question as whether any of it is still visible.
            //
            // A lead the release has spent is kept, at no opacity, for as long
            // as the note's own end is close enough to the now-line for its
            // OUTLINE to cross it. That is not a nicety: the clip opens by the
            // lead, and what the lead opens belongs to the lead. Drop the lead
            // and the box ends at the note, so that end's cap is the box's own
            // outline — drawn at its full reach, with nothing left here able to
            // say otherwise, and landing on the analyzer. It is the exact ink
            // the clip has stopped since before there was a lead.
            //
            // Kept, the note's end stays INSIDE the box, where the cap is
            // `cap_px`'s to meter and can be given only the room it has.
            //
            // How far the note's own end sits inside the roll, in points, and
            // how far the outline stands off it: the same reach `vs_note` grows
            // the quad by, since it is the same ink.
            let behind_px = (d1 - split) * axes.depth_len();
            let outline_reach_px = outline_px + 0.5 * feather_px;
            let leads = lead_px > 0.0 && last && (standing > 0.0 || behind_px < outline_reach_px);
            // How far that cap may reach while the lead is still on the box.
            //
            // The shader stands the cap against the note's own end UNDER the
            // lead, so the RELEASE needs no ramp here: the lead's own ink
            // covers the cap while the lead is opaque and uncovers it as it
            // goes, so the edge comes up through the tongue over the whole
            // release. The alternative is an edge that arrives whole the frame
            // the lead is dropped, which lands on a ribbon that has spent that
            // release dissolving — nothing else in the picture moves then, and
            // it is the one moment an edge appearing reads as a fault.
            //
            // What is left to decide is ROOM, which is the paragraph above: the
            // cap stands OUTSIDE the note's end, in the stretch the lead opened
            // past the line, and that stretch is not the cap's to take. So the
            // cap is allowed only as far as the note's own end has scrolled
            // clear, and it grows out of that end rather than fading in over
            // it — wholly behind the line at every width, instead of faintly
            // across it. The half-feather is the shader's antialiasing ramp,
            // which reaches that far past whatever the reach says.
            //
            // At its full reach this is the same picture the box's own outline
            // draws, and that is the point `leads` above gives up the lead at —
            // so the two meet at one number and nothing moves at the handover.
            //
            // Zeroed with the rest of the lead's numbers where there is no
            // lead, below. It is read only inside one, and `behind_px` is not
            // even the leading end for every segment that reaches here — a
            // number that decides nothing is still worth not carrying.
            let cap_px = outline_px.min((behind_px - 0.5 * feather_px).max(0.0));

            // Notes always draw fully opaque — how much of the heatmap comes
            // through a note is the Fill setting's business, not an opacity
            // here, and a released note fades on the lattice's fade.
            let alpha = 1.0;
            let pitch = (p0 + p1) * 0.5;
            // The note's TRUE color, so a note matches the node it lit up on
            // the lattice. It is painted solid, edge to edge — the interior
            // and the boundary are one thing, and a heatmap cell showing
            // through a note said neither clearly.
            let core = note_color(state, pitch, alpha);
            // Reading outward: the note, the dark outline standing against
            // every one of its edges and fading out, then whatever the
            // spectrogram is doing.
            //
            // The outline stands entirely OUTSIDE the note, never as a stroke
            // of its path — a centered stroke grows inward exactly as much as
            // outward, and a note is only a few pixels thick at the pitch
            // ranges this pane is actually used at, so the two long edges meet
            // in the middle and paint over it. Here that is structural: the
            // shader reads the outline off the DISTANCE to the note's edge, and
            // coverage taken at a positive distance cannot reach back inside it
            // whatever the ribbon's thickness.
            //
            // Wrapping the ENDS costs the notes around it nothing, and that is
            // a fact about the ORDER they are drawn in rather than about the
            // outline: `harmonigraph_render::roll` lays every outline down and
            // then every body over them, so an outline can darken the picture
            // and never another note.
            //
            // It has to be that way round rather than something gentler at the
            // seam. Coverage is OPAQUE where the outline meets its own note
            // whatever the fade is set to (`outline_coverage`, and the reason
            // is in [`outline`]: a surround that is translucent against the
            // note takes its color from the cell behind it and washes out over
            // a bright one). Composited with its own note, that opacity landed
            // on the neighbour it reached into — repeats of one key butt
            // together along time, and the later one blanked the tail of the
            // earlier.
            //
            // What it does cost is the seam between two notes that TOUCH:
            // same key, no gap, and the two bodies now meet directly in one
            // color. A gap of a point or more still reads as two notes, the
            // outline filling it.
            //
            // The reach is still the setting to be careful with at the wide
            // end, and on the shortest notes: a tapped key floored to a point
            // or two of length wears the full reach at both ends, so at the
            // top of the bar it is a dot inside its own surround. See
            // [`SpectrumConfig::roll_outline`](crate::SpectrumConfig).
            //
            // Nothing else rides the outline — in particular, no band
            // approximating the bloom. The bloom the notes carry is the
            // lattice's own post-process, run over the notes themselves
            // (`harmonigraph_render::roll`), and a band standing in for one
            // would be a second thing to keep looking like it.

            // Geometry in the pane's own two axes, which is all the shader is
            // told: `Axes` maps those onto perpendicular screen axes, so
            // nothing here names a screen side either.
            //
            // NOT snapped to whole pixels, which egui does to rects by default
            // (TessellationOptions::round_rects_to_pixels) to keep static
            // chrome crisp. These scroll: snapping holds a note still until it
            // has drifted a whole pixel and then jumps it, so the roll advanced
            // in steps while the spectrogram — a mesh, never snapped — slid
            // smoothly underneath, and the notes read as jittering against it.
            // A distance field has no pixel grid to snap to, so the sub-pixel
            // placement is now simply what it does.
            let center = axes.at((a0 + a1) * 0.5, (d0 + d1) * 0.5);
            let depth_px = (d1 - d0) * axes.depth_len();
            // The stretch above is where the floor already landed, so this is
            // the drawn length and not a second floor. Flooring here as well is
            // what makes the boxes overlap.
            let half_depth = if zero_span { min_half_depth } else { depth_px.abs() * 0.5 };
            // How far the note's center line drifts along the pitch axis per
            // point of depth: 0 for a held note, non-zero for a glide, which
            // shears the box into the parallelogram the ribbon follows. Guarded
            // because a segment can have no duration at all — a note pressed
            // this frame is one — and a slope is meaningless there.
            //
            // Taken against the box as DRAWN. The shader reaches
            // `|shear| * half_extent[1]` along pitch, so the shear and the
            // length are one product: a shear left at the segment's true rate
            // while the floor lengthens the box multiplies that reach by
            // however much the floor won by, which is unbounded as the segment
            // shortens — it would draw a diagonal streak, rim and all, through
            // pitches nothing sounded. Against the drawn length, the reach is
            // the segment's real drift whatever the floor did.
            //
            // What that costs is the shear itself: a note the floor stretched
            // draws its drift spread over the stretched length, so it reads as
            // a shallower bend than it was. That is the honest way round. A
            // slope is a ratio of two things, and the floor has already
            // overstated the denominator on purpose; overstating the numerator
            // to match would put ink where no note was, and pitch is the axis
            // this pane exists to be read precisely on.
            let slope = if depth_px.abs() > 1e-6 {
                (a1 - a0) * axes.pitch_len() / depth_px
            } else {
                0.0
            };
            // Off the octave zoom entirely, tested against the ink this
            // segment actually reaches rather than the note's endpoints — the
            // cull the filter above deliberately leaves undone.
            //
            // Term for term what `vs_note` grows the quad by along pitch: the
            // drawn ribbon's own half width, the center line's drift over the
            // quad's half length (ends included), and the outline's reach
            // turned into a distance ALONG pitch, which on a sheared note is
            // `skew` times what it is across the note's long edges. Measured
            // from the segment's own center, which is where the shader
            // measures it from.
            let skew = (1.0 + slope * slope).sqrt();
            let ink_pitch_px = half_pitch
                + slope.abs() * (half_depth + outline_reach_px + 0.5 * feather_px)
                + skew * outline_reach_px;
            let ink_pitch = ink_pitch_px / axes.pitch_len().max(1e-3);
            let center_pitch = (a0 + a1) * 0.5;
            if center_pitch + ink_pitch < 0.0 || center_pitch - ink_pitch > 1.0 {
                continue;
            }
            // The lead: the ribbon carried on past the now-line, so a sounding
            // note crosses into the spectrum peak it is making instead of
            // stopping square on the join. Half of it goes on the length and
            // half on moving the box the other way along depth, which is the
            // same thing as growing one end.
            //
            // The box grows by the whole lead however far the release has taken
            // it, and the fading is left to the shader. Retracting the geometry
            // instead would take the tongue back toward the note as it went,
            // which is a lead that ends by being pulled in rather than by going
            // out — and it would make the release a second thing moving on a
            // ribbon that is already scrolling.
            //
            // Depth runs AWAY from the spectrum, so the leading end is at the
            // box's low-depth side whichever way the pane is turned, and this
            // names no screen side either. It is applied after the pitch cull
            // above because it reaches along DEPTH alone: the cull is about how
            // far the ink goes across pitch, and a lead moves none of it there.
            //
            // The `slope` term is what keeps the extension on the ribbon's own
            // center line rather than parallel to it — shifting the center of a
            // SHEARED box along depth alone slides the whole note sideways by
            // `slope * shift`, which would kink the ribbon at the join. It is
            // zero today at every note: a note's last segment is flat by
            // construction (`RollNote::segments` closes the note at the pitch
            // of its final bend), so the one segment a lead can reach has no
            // shear to correct for. Written out because the correction belongs
            // to the shift rather than to that fact about the segments.
            let (center, half_depth, lead_px, lead_fade_px, standing, cap_px) = if leads {
                let half = lead_px * 0.5;
                (
                    center - axes.dir_depth() * half - axes.dir_pitch() * (slope * half),
                    half_depth + half,
                    lead_px,
                    lead_fade_px,
                    standing,
                    cap_px,
                )
            } else {
                (center, half_depth, 0.0, 0.0, 0.0, 0.0)
            };
            instances.push(RollInstance {
                center: [center.x, center.y],
                half_extent: [half_pitch, half_depth],
                shear: slope,
                outline_reach: outline_px,
                outline_fade: outline_fade_px,
                lead: lead_px,
                lead_fade: lead_fade_px,
                lead_alpha: standing,
                cap_reach: cap_px,
                core: core.to_array(),
                outline: outline_color.to_array(),
            });
        }
    }
    instances
}

/// The color of a note at `pitch`: the lattice's own, off the same
/// [`pitch_lut_color`] ramp the nodes are painted through, so a ribbon and the
/// node it lit up are the same color. Shared with the Spiral pane's radial
/// marks, which are the same notes drawn somewhere else.
///
/// The only coloring there is, deliberately. The obvious alternative — one
/// flat accent for the roll — breaks the identity that makes it readable
/// beside the lattice: what a color means has to be the same thing in every
/// picture, or reading across them is a translation.
pub(crate) fn note_color(state: &SharedState, pitch: f32, alpha: f32) -> Color32 {
    let (darkest, brightest) =
        (state.frame_params.darkest_pitch, state.frame_params.brightest_pitch);
    scene_color(pitch_lut_color(pitch, darkest, brightest, state.view.pitch_gradient), alpha)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::probe::fresh;
    use crate::{SharedState, SpectralOrientation};
    use harmonigraph_core::{NoteEvent, NoteEventKind};

    /// The pane the tests paint into: 300 points along the time axis, 100
    /// across pitch.
    const PANE: egui::Rect =
        egui::Rect { min: egui::pos2(10.0, 20.0), max: egui::pos2(310.0, 120.0) };

    /// The display density these tests derive geometry at — a Retina screen,
    /// which is what the plugin is looked at on, and where
    /// [`MIN_LENGTH_DEVICE_PX`] comes to one point.
    const PPP: f32 = 2.0;

    /// The roll's geometry for `state`, derived exactly the way
    /// [`spectral_pane`](super::super::spectral_pane) derives it
    /// before handing over — same axes, same pitch scale, same split.
    fn instances(state: &SharedState, now: f64) -> Vec<RollInstance> {
        let cfg = &state.spectrum_config;
        let axes = Axes::new(PANE, cfg);
        let min_midi = cfg.low_midi;
        let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
        let scale = PitchScale { min_midi, max_midi, span: max_midi - min_midi };
        let split = super::super::axes::spectrum_share(cfg);
        note_instances(&axes, &scale, state, split, now, PPP)
    }

    /// One held note, and the instances the roll would draw for it. `range`
    /// is the pitch span in semitones — the pane is 100 points across the
    /// pitch axis, so a wide range makes a thin ribbon, which is where the
    /// outline geometry is under the most pressure.
    fn ribbon_with_range(outline: f32, range: f32) -> Vec<RollInstance> {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 60.0 - range * 0.5;
        state.spectrum_config.high_midi = 60.0 + range * 0.5;
        state.spectrum_config.roll_thickness = 2.0;
        state.spectrum_config.roll_outline = outline;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        instances(&state, 0.05)
    }

    /// The note is 12 semitones across a 100-point axis (a thick ribbon), so
    /// it is bounded rather than drawn as a bare spine.
    fn ribbon(outline: f32) -> Vec<RollInstance> {
        ribbon_with_range(outline, 12.0)
    }

    /// A note must stay on screen until the last of its INK is past the far
    /// edge, not until the last of its box is.
    ///
    /// The shader paints the keyline and an antialiasing ramp outside the box
    /// it is handed, so testing the box against the window
    /// dropped the note while a few points of ribbon were still owed — it
    /// popped short of the edge rather than sliding under it. The overhang is
    /// screen-space, so in time it scales with the Span, which is why it reads
    /// as "notes vanish early" more strongly the further out you zoom.
    #[test]
    fn a_note_keeps_drawing_until_its_outline_has_left_too() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 10.0;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.tracker.handle_event(NoteEvent::on(1.0, 0, 60, 1.0));
        state.tracker.handle_event(NoteEvent::off(1.5, 0, 60));

        // The moment the note's BOX leaves: released at 1.5 with a 10 s window.
        let box_gone = 11.5;
        assert!(
            !instances(&state, box_gone + 0.01).is_empty(),
            "the note stopped drawing the instant its box left, with its outline \
             and rim still owed",
        );
        // It does still go, and close behind: the ink is a few points, which at
        // this span is a fraction of a second.
        let mut last = box_gone;
        let mut t = box_gone;
        while t < box_gone + 2.0 {
            if !instances(&state, t).is_empty() {
                last = t;
            }
            t += 0.01;
        }
        assert!(
            last > box_gone && last < box_gone + 0.5,
            "the ink should outlast the box by a little, not by nothing ({last} vs {box_gone})",
        );
    }

    fn one(rects: &[RollInstance]) -> &RollInstance {
        assert_eq!(rects.len(), 1, "expected one note segment, got {}", rects.len());
        &rects[0]
    }

    /// The outline stands the same distance off at every zoom and every note
    /// width — an edge does not thin out because the ribbon it wraps did, and
    /// does not thicken because the range was zoomed in.
    ///
    /// Constant is the requirement, not a simplification of one. An outline
    /// tied to the ribbon would hold a steadier ratio between the two, at the
    /// price of making the zoom change what a note IS rather than how much of
    /// the axis it covers — and the wide end, where a scaled outline would give
    /// way, is exactly where a picture full of notes needs its edges most.
    ///
    /// It is expressed as a DISTANCE OUTSIDE the note's edge, never as a wider
    /// stroke of it; that distinction is the flood fix, and it is structural,
    /// since the shader reads the outline off distances outside the note's own
    /// edge. What is left to check here is that the reach is handed over
    /// unscaled.
    #[test]
    fn the_outline_stands_the_same_distance_off_at_any_note_width() {
        let thick = ribbon_with_range(1.5, 12.0);
        // ~120 semitones over 100 points: the ribbon is under 2 points thick,
        // which is where a centered stroke meets itself in the middle and
        // paints the interior black.
        let thin = ribbon_with_range(1.5, 120.0);
        assert_eq!(one(&thick).outline_reach, 1.5, "the outline is not the reach it was set to");
        assert_eq!(
            one(&thin).outline_reach,
            one(&thick).outline_reach,
            "the outline thinned with the note",
        );
        assert!(
            one(&thin).half_extent[0] < one(&thick).half_extent[0],
            "the two notes are the same thickness; the comparison is vacuous",
        );
        // And nothing in between is a ramp: the reach is one number, not a
        // reading of the ribbon it stands against.
        let middling = ribbon_with_range(1.5, 200.0 / 3.0);
        assert_eq!(one(&middling).outline_reach, 1.5, "the outline scaled with the ribbon");
    }

    /// The outline is OPAQUE black where it meets the note, at every reach that
    /// draws it at all, and its fade is what takes it out from there.
    ///
    /// Opacity against the note is the point rather than a detail. A uniformly
    /// translucent surround takes its color from the spectrogram behind it, so
    /// the same note reads differently over a loud cell than a quiet one, and
    /// washes out entirely against the bright end of a palette — the cell a
    /// note makes for itself. Anything less than 255 here is that bug returning
    /// quietly, and dialling the fade is not how you would ask for it.
    ///
    /// Off has to mean zero reach AND no color, never one or the other: the
    /// reach is what the quad grows by to make room for the outline (and what
    /// keeps a leaving note alive past the far edge), so an outline that will
    /// not paint must not be paid for either.
    #[test]
    fn the_outline_is_opaque_black_at_every_reach_that_draws_it() {
        // Three settings across the bar's whole travel: the least it can be set
        // to above nothing, a modest one, and full.
        for reach in [0.05, 2.0, crate::ROLL_OUTLINE_MAX] {
            let lit = ribbon(reach);
            let note = one(&lit);
            assert_eq!(note.outline_reach, reach, "the outline is not the reach it was set to");
            assert_eq!(
                note.outline,
                [0, 0, 0, 255],
                "the outline is not opaque black at reach {reach}: {:?}",
                note.outline,
            );
        }

        let off = ribbon(0.0);
        let note = one(&off);
        assert_eq!(note.outline_reach, 0.0, "an outline of no reach still made room for one");
        assert_eq!(note.outline[3], 0, "an outline of no reach left a color behind");
    }

    /// The fade rides its own setting, and is bounded by the reach it softens:
    /// past that it would be asking for an outline that never meets the note
    /// solid, which is the one thing the outline is for.
    ///
    /// The same bound the lattice puts on its gutter fade, and the reason the
    /// two are separate settings at all — a fade derived from the reach makes a
    /// wider outline always a blurrier one, so the roll takes both and clamps
    /// rather than deriving one from the other.
    #[test]
    fn the_fade_is_its_own_setting_and_stops_at_the_reach() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        let fade_at = |state: &mut SharedState, reach: f32, fade: f32| {
            state.spectrum_config.roll_outline = reach;
            state.spectrum_config.roll_outline_fade = fade;
            let notes = instances(state, 0.05);
            one(&notes).outline_fade
        };
        assert_eq!(fade_at(&mut state, 3.0, 0.0), 0.0, "a fade of 0 is a hard edge, not a default");
        assert_eq!(fade_at(&mut state, 3.0, 1.0), 1.0, "the fade is not its own setting");
        assert_eq!(fade_at(&mut state, 3.0, 3.0), 3.0, "a fade of the whole reach is allowed");
        assert_eq!(fade_at(&mut state, 3.0, 9.0), 3.0, "a fade past the reach was not clamped");
        // And an outline that is not drawn carries no fade to draw it with.
        assert_eq!(fade_at(&mut state, 0.0, 2.0), 0.0, "an outline of no reach kept a fade");
    }

    /// Note width is in SEMITONES, so a wide zoom takes a ribbon under a
    /// pixel — where a filled rectangle fades out to nothing and the roll
    /// stops saying a note was played. The width is floored at the point it
    /// can still be seen at, and left alone above that.
    #[test]
    fn a_hairline_ribbon_is_floored_at_the_width_it_can_be_seen_at() {
        let thick = ribbon_with_range(0.5, 12.0);
        let wide = one(&thick).half_extent[0];
        assert!(
            wide > MIN_RIBBON_PX * 0.5,
            "a readable ribbon ({wide}) should be wider than the floor, or the \
             comparison below is vacuous",
        );

        let thin = ribbon_with_range(0.5, 600.0);
        assert_eq!(
            one(&thin).half_extent[0],
            MIN_RIBBON_PX * 0.5,
            "a hairline ribbon was not floored at the width it can be seen at",
        );
    }

    /// A note too brief to fill two device pixels is drawn at that length
    /// anyway, centered on the moment it was — the floor that stops it
    /// flickering as it scrolls. See [`MIN_LENGTH_DEVICE_PX`] for the
    /// measurement behind the number.
    ///
    /// Both halves matter. Long enough, and the length is the note's own, to
    /// the point — a floor that rounded every note up would be a roll that
    /// cannot say how long anything was held. Short enough, and it is the
    /// floor, and the note still sits on its own midpoint rather than being
    /// pushed off it in one direction.
    ///
    /// The note here is FINISHED, which is what makes the midpoint the moment
    /// it was. A note still sounding has its midpoint half a floor into the
    /// analyzer instead, and is the one case the centering is wrong for — see
    /// [`a_sounding_note_is_floored_into_the_past_alone`].
    #[test]
    fn a_brief_note_is_floored_at_the_length_it_can_scroll_without_flickering() {
        // A 10 s span across 300 points of depth, of which the roll takes its
        // share: about 60 ms per point, so a 20 ms tap is well under the floor
        // and a 2 s note is well over it.
        let tap = |length: f64| {
            let mut state = fresh();
            state.spectrum_config.orientation = SpectralOrientation::Left;
            state.spectrum_config.roll_seconds = 10.0;
            state.spectrum_config.low_midi = 48.0;
            state.spectrum_config.high_midi = 84.0;
            state.tracker.handle_event(NoteEvent::on(2.0, 0, 60, 1.0));
            state.tracker.handle_event(NoteEvent::off(2.0 + length, 0, 60));
            let notes = instances(&state, 5.0);
            let note = *one(&notes);
            let axes = Axes::new(PANE, &state.spectrum_config);
            let split = super::super::axes::spectrum_share(&state.spectrum_config);
            // What the segment would have measured unfloored: its true seconds
            // over the roll's own share of the depth axis.
            let per_point = f64::from(state.spectrum_config.roll_seconds)
                / f64::from(axes.depth_len() * (1.0 - split));
            (note, (length / per_point) as f32)
        };

        let floor = 0.5 * MIN_LENGTH_DEVICE_PX / PPP;
        let (brief, true_half) = tap(0.02);
        assert!(true_half * 0.5 < floor, "the brief note ({true_half} pt) is not under the floor");
        // Within float error rather than to the bit: the floor reaches a
        // segment by scaling the note's own span up to it, so the arithmetic
        // that lands on it is a multiply and a pair of subtractions rather than
        // a `max` against the constant.
        assert!(
            (brief.half_extent[1] - floor).abs() < 1e-3,
            "a brief note was left to flicker: {} against a floor of {floor}",
            brief.half_extent[1],
        );

        let (held, true_half) = tap(2.0);
        assert!(true_half * 0.5 > floor, "the held note ({true_half} pt) is not over the floor");
        assert!(
            (held.half_extent[1] - true_half * 0.5).abs() < 0.01,
            "a note long enough to draw honestly was rounded up: {} vs {}",
            held.half_extent[1],
            true_half * 0.5,
        );

        // Centered on the note, not pushed off it: the floored box sits on the
        // depth of the note's own mid-time, so it reaches half the floor either
        // side of the moment it was rather than the whole floor in one
        // direction. Depth runs away from the now-line, so the box's far end is
        // `+ half_extent` and its near end `-`, and the moment the note happened
        // is the midpoint between them.
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 10.0;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        let axes = Axes::new(PANE, &state.spectrum_config);
        let split = super::super::axes::spectrum_share(&state.spectrum_config);
        let time = super::super::axes::TimeAxis::new(&state, split, 5.0);
        let scale = PitchScale { min_midi: 48.0, max_midi: 84.0, span: 36.0 };
        let want = axes.at(scale.t_of(60.0), time.depth_of_unclamped(2.01));
        assert!(
            (brief.center[0] - want.x).abs() < 0.01 && (brief.center[1] - want.y).abs() < 0.01,
            "the floored note sits at {:?}, not on its own mid-time {want:?}",
            brief.center,
        );
    }

    /// Where an instance's box lands along the DEPTH axis, in points from the
    /// region's near edge, as `(low, high)`.
    ///
    /// Which END of time each of those is depends on the mode, and the callers
    /// say so rather than this: the live window runs the past outward from the
    /// near edge, so `low` is the end at the now-line; the whole-song layout
    /// runs the take forward from that edge, so `low` is the earlier end.
    ///
    /// Read through [`Axes`] rather than off a screen coordinate, because the
    /// pane can be turned any of four ways and none of them is a screen side.
    fn depth_from_edge(axes: &Axes, split: f32, inst: &RollInstance) -> (f32, f32) {
        let origin = axes.at(0.0, 0.0);
        let center = egui::pos2(inst.center[0], inst.center[1]);
        let mid = (center - origin).dot(axes.dir_depth()) - split * axes.depth_len();
        (mid - inst.half_extent[1], mid + inst.half_extent[1])
    }

    /// The floor grows a note that is still SOUNDING into the past alone, so
    /// none of it is drawn on the spectrum's side of the now-line.
    ///
    /// Such a note's leading end is `now`, which is where the line is drawn, so
    /// a floor centered on it lands half of itself under an opaque one-point
    /// hairline. On the 2x display the plugin is used on the two are the same
    /// width and a just-pressed note is invisible; at 1x the floor is twice the
    /// line and what survives is a dash of note color detached from its own
    /// ribbon, in the spectrum's territory. Issue #239 has both measured.
    ///
    /// Both ends are asserted, and the trailing one is not a formality: a clamp
    /// that bought the clearance by SHORTENING the note would satisfy the
    /// leading end and give back the visibility the floor exists for.
    #[test]
    fn a_sounding_note_is_floored_into_the_past_alone() {
        // Lead at 0. A sounding note's tongue reaches well past the line at the
        // default (a quarter of the analyzer), which would be the ink standing
        // clear here — and this is about what the FLOOR draws, which is what
        // the line covers.
        let held = |ppp: f32, span: f32, dt: f64| {
            let mut state = fresh();
            state.spectrum_config.orientation = SpectralOrientation::Left;
            state.spectrum_config.roll_seconds = span;
            state.spectrum_config.roll_lead = 0.0;
            state.spectrum_config.low_midi = 48.0;
            state.spectrum_config.high_midi = 84.0;
            state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
            let cfg = &state.spectrum_config;
            let axes = Axes::new(PANE, cfg);
            let scale = PitchScale { min_midi: 48.0, max_midi: 84.0, span: 36.0 };
            let split = super::super::axes::spectrum_share(cfg);
            let ins = note_instances(&axes, &scale, &state, split, dt, ppp);
            depth_from_edge(&axes, split, one(&ins))
        };

        // A 12 s Span, where the floor is spent inside a frame, and a 10 min
        // one, where a note is still under the floor a second after the press —
        // which is the case the floor is carrying on its own.
        for ppp in [1.0_f32, 2.0] {
            let floor = MIN_LENGTH_DEVICE_PX / ppp;
            for (span, dt) in [(12.0_f32, 0.0_f64), (600.0, 0.0), (600.0, 1.0)] {
                let (leading, trailing) = held(ppp, span, dt);
                assert!(
                    leading > -1e-3,
                    "{ppp}x, Span {span}, {dt}s after the press: the note reaches \
                     {leading} pt onto the spectrum's side of the now-line",
                );
                assert!(
                    (trailing - floor).abs() < 1e-3,
                    "{ppp}x, Span {span}, {dt}s after the press: {trailing} pt of note \
                     stands past the now-line, not the floor's {floor}",
                );
            }
        }

        // Long enough to draw honestly, and the clamp has nothing to do: the
        // leading end is the note's own stop, which is `now` and so the line,
        // and the length is the note's rather than the floor's.
        let (leading, trailing) = held(2.0, 12.0, 4.0);
        assert!(leading.abs() < 1e-3, "a held note starts {leading} pt off the now-line");
        assert!(trailing > 4.0 * MIN_LENGTH_DEVICE_PX, "a 4 s note was floored: {trailing} pt");
    }

    /// The clearance survives the key coming up: a note keeps its ink off the
    /// spectrum's side of the line while it scrolls away, rather than snapping
    /// back across it the frame it is released.
    ///
    /// This is what makes the clamp a clamp instead of a gate on whether the
    /// key is down. A gate spends its whole correction in one frame — the note
    /// is clear of the line while sounding and half a floor over it immediately
    /// after — which hands #239 back to every note short enough to need the
    /// floor, staccato playing at a long Span being the ordinary case rather
    /// than a corner of one.
    #[test]
    fn a_notes_clearance_survives_the_key_coming_up() {
        // A 10 min Span, where a 50 ms tap is far under the floor and takes
        // nearly two seconds of scrolling to earn its own length.
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 600.0;
        state.spectrum_config.roll_lead = 0.0;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        state.tracker.handle_event(NoteEvent::off(0.05, 0, 60));

        let cfg = &state.spectrum_config;
        let axes = Axes::new(PANE, cfg);
        let scale = PitchScale { min_midi: 48.0, max_midi: 84.0, span: 36.0 };
        let split = super::super::axes::spectrum_share(cfg);
        let at = |now: f64| {
            let ins = note_instances(&axes, &scale, &state, split, now, 2.0);
            depth_from_edge(&axes, split, one(&ins))
        };

        // Across the release and well past it. The key comes up at 0.05, so the
        // second sample is the frame a gate would have let go on.
        let mut previous = at(0.05).0;
        for step in 0..40 {
            let now = 0.05 + f64::from(step) * 0.05;
            let (leading, trailing) = at(now);
            assert!(
                leading > -1e-3,
                "{now}s: the note reaches {leading} pt onto the spectrum's side of the \
                 now-line, {:.2}s after the key came up",
                now - 0.05,
            );
            assert!(trailing > leading, "{now}s: the box collapsed");
            // Into the past, never back toward the line, and never by more than
            // the roll scrolled between the two frames — a jump is the snap
            // this test exists for, whichever direction it is in.
            let scrolled = 0.05
                / super::super::axes::TimeAxis::new(&state, split, now)
                    .seconds_per_point(&axes);
            assert!(
                leading >= previous - 1e-3 && leading - previous <= scrolled as f32 + 1e-3,
                "{now}s: the note's near end jumped from {previous} pt to {leading}",
            );
            previous = leading;
        }
    }

    /// The clamp is on a note the picture has already reached, not on every box
    /// against the playhead. The whole-song layout draws the take's future as
    /// well as its past, so a note that has not been played yet belongs where it
    /// is — clamping every box would drag the rest of the take onto the playhead
    /// and pop each note forward as the sweep passed it.
    #[test]
    fn the_whole_song_layout_keeps_drawing_the_take_ahead_of_the_playhead() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        // The whole take at once, the way the offline renderer lays it out: a
        // brief note that is over well before the playhead reaches it, so the
        // floor is what draws it and there is a clamp to get wrong.
        state.tracker.handle_event(NoteEvent::on(6.0, 0, 60, 1.0));
        state.tracker.handle_event(NoteEvent::off(6.02, 0, 60));
        let roll = state.tracker.roll().clone();
        state.whole_song =
            Some(crate::WholeSong { columns: Vec::new(), roll, start: 0.0, span: 10.0 });

        let cfg = &state.spectrum_config;
        let axes = Axes::new(PANE, cfg);
        let scale = PitchScale { min_midi: 48.0, max_midi: 84.0, span: 36.0 };
        // The mode gives the roll the whole depth axis; there is no spectrum
        // beside it to leave room for.
        let split = 0.0;
        let ins = note_instances(&axes, &scale, &state, split, 1.0, 2.0);
        let (low, high) = depth_from_edge(&axes, split, one(&ins));

        // Depth runs forward in time here, so the note sits on its own
        // mid-time: 6.01 of the take's 10 seconds, over 300 points of axis.
        // Against the MIDPOINT and to a tolerance well inside half a floor,
        // because the two answers this is separating — centered, or dragged
        // onto the playhead — differ by exactly what the floor added.
        let want = 6.01 / 10.0 * axes.depth_len();
        let mid = (low + high) * 0.5;
        assert!(
            (mid - want).abs() < 0.05,
            "a note six seconds into the take was drawn at {mid} pt, not {want} — \
             a clamp meant for a note already played has pulled it toward the playhead",
        );
    }

    /// Which way the clamp moves a note in the whole-song layout, with margin.
    ///
    /// That layout runs time FORWARD from the near edge while the live window
    /// runs it backward, so the direction the floor grows into is read off the
    /// mapping rather than assumed. The sign is the whole of that reading, and
    /// nothing else in the suite would notice it flipped: the live tests cannot
    /// see it, and a released note far from the playhead is not clamped at all.
    ///
    /// A note ending ON the playhead is the case that pins it. Floored, its box
    /// is half a floor longer than the note in each direction; clamped the right
    /// way it ends on the playhead, and clamped the wrong way it reaches a whole
    /// floor past it, into a part of the take that has not been played.
    #[test]
    fn the_whole_song_clamp_grows_a_note_back_toward_the_takes_start() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.tracker.handle_event(NoteEvent::on(6.0, 0, 60, 1.0));
        state.tracker.handle_event(NoteEvent::off(6.02, 0, 60));
        let roll = state.tracker.roll().clone();
        state.whole_song =
            Some(crate::WholeSong { columns: Vec::new(), roll, start: 0.0, span: 10.0 });

        let cfg = &state.spectrum_config;
        let axes = Axes::new(PANE, cfg);
        let scale = PitchScale { min_midi: 48.0, max_midi: 84.0, span: 36.0 };
        let split = 0.0;
        // The playhead exactly on the note's release, where the clamp binds by
        // the whole of what the floor added.
        let now = 6.02;
        let ins = note_instances(&axes, &scale, &state, split, now, 2.0);
        let (low, high) = depth_from_edge(&axes, split, one(&ins));

        let playhead = (now / 10.0) as f32 * axes.depth_len();
        let floor = MIN_LENGTH_DEVICE_PX / 2.0;
        assert!(
            high < playhead + 1e-3,
            "the note reaches {high} pt, past the playhead at {playhead} — the floor \
             grew it into the take's future, so `into_past` has the wrong sign here",
        );
        assert!(
            (high - playhead).abs() < 1e-3,
            "the note ends at {high} pt rather than on the playhead at {playhead}",
        );
        assert!(
            (high - low - floor).abs() < 1e-3,
            "the note is {} pt long, not the floor's {floor}",
            high - low,
        );
    }

    /// The floor is in DEVICE pixels, so it is half as many points on a 2x
    /// display as on a 1x one — the antialiasing ramp it is sized against is
    /// one physical pixel wide, and a floor in points would be right on
    /// exactly one class of display.
    #[test]
    fn the_length_floor_follows_the_display_density() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 10.0;
        state.tracker.handle_event(NoteEvent::on(2.0, 0, 60, 1.0));
        state.tracker.handle_event(NoteEvent::off(2.001, 0, 60));
        let cfg = &state.spectrum_config;
        let axes = Axes::new(PANE, cfg);
        let scale = PitchScale { min_midi: 48.0, max_midi: 84.0, span: 36.0 };
        let split = super::super::axes::spectrum_share(cfg);
        let at = |ppp| {
            let notes = note_instances(&axes, &scale, &state, split, 5.0, ppp);
            one(&notes).half_extent[1]
        };
        // To float error rather than to the bit: the floor is reached by
        // scaling the note's own span up to it, not by a `max` against it.
        assert!((at(1.0) - 0.5 * MIN_LENGTH_DEVICE_PX).abs() < 1e-3, "1x: two points, {}", at(1.0));
        assert!(
            (at(2.0) - 0.25 * MIN_LENGTH_DEVICE_PX).abs() < 1e-3,
            "2x: one point, {}",
            at(2.0),
        );
        assert!(at(1.0) > at(2.0), "the floor did not follow the density at all");
    }

    /// A floored note's segments must not paint outside the pitch they covered.
    ///
    /// `shear` is a RATE — pitch points per point of depth — and the shader
    /// reaches `|shear| * half_extent[1]` along pitch, so the length floor and
    /// the shear are one product. Leaving the shear at the true rate while the
    /// floor lengthens the box multiplies that reach by however much the floor
    /// won by, which is unbounded as the segment shortens: it draws a diagonal
    /// streak, rim and all, through pitches the note never sounded.
    ///
    /// Per-note tuning makes that the ordinary case rather than a corner. A
    /// note-on arrives at the key's pitch and the tuning expression lands a
    /// block later (see `RollNote::SETTLE` — 11 ms at 48 kHz), so every retuned
    /// note opens with a segment a few milliseconds long carrying its whole
    /// offset. Measured before the bound, at the Span below: 13.8 points of
    /// drawn reach against 0.4 of real drift, and 49 semitones at the Span
    /// bar's top.
    #[test]
    fn a_floored_note_stays_inside_the_pitch_it_covered() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 60.0;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.tracker.handle_event(NoteEvent::on(2.0, 0, 60, 1.0));
        // One 512-frame block at 48 kHz behind the note-on, which is what a
        // host's per-note tuning actually does.
        state.tracker.handle_event(NoteEvent {
            time: 2.011,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: 0.3 },
        });
        // Released almost at once, so the NOTE is under the length floor and
        // the floor is what draws it — which is the case this is about, now
        // that a segment is only stretched as its note is.
        state.tracker.handle_event(NoteEvent::off(2.02, 0, 60));
        let axes = Axes::new(PANE, &state.spectrum_config);
        // The whole drift any segment of this note can carry, as a half-extent
        // in points: 0.3 semitones of a 36-semitone axis. No segment may reach
        // further than this along pitch, whatever the floor did to its length.
        let bound = 0.3 / 36.0 * axes.pitch_len() * 0.5;

        let notes = instances(&state, 5.0);
        assert!(notes.len() >= 2, "the tuning should have split the note into segments");
        let drawn: f32 = notes.iter().map(|n| n.half_extent[1] * 2.0).sum();
        assert!(
            (drawn - 2.0 * min_half_depth_for(PPP)).abs() < 1e-3,
            "the note drew {drawn} points along depth rather than the floor it is too \
             brief for; nothing here is floored and the test is vacuous",
        );
        for note in &notes {
            let reach = note.shear.abs() * note.half_extent[1];
            assert!(
                reach <= bound + 1e-3,
                "a segment reaches {reach} points along pitch, against {bound} of real drift",
            );
        }
    }

    /// The segments of one note TILE the depth it covers — they may touch, and
    /// they must never overlap.
    ///
    /// The floor is the note's, not each segment's, for exactly this reason.
    /// Segments run end to end, so flooring them one by one pushes each box
    /// past its neighbour's start by nearly the whole floor; and since the rim
    /// is opaque, the later box's dark band lands on the earlier one's color
    /// and takes its hue out. A bent note comes out a ladder of rim rather than
    /// a ribbon, worst on the vibrato and fast bends that make the most
    /// segments.
    ///
    /// A long note with segments far under the floor is the ordinary shape of
    /// this: the note has no flicker to fix, and its short segments are covered
    /// by their neighbours rather than being alone on the axis.
    #[test]
    fn the_segments_of_a_bent_note_tile_instead_of_overlapping() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        // 60 s over the roll's share of 300 points: about 0.44 s to the point,
        // so the 20 ms steps below are each a twentieth of the length floor.
        state.spectrum_config.roll_seconds = 60.0;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.tracker.handle_event(NoteEvent::on(2.0, 0, 60, 1.0));
        for (i, at) in [2.02, 2.04, 2.06, 2.08].into_iter().enumerate() {
            state.tracker.handle_event(NoteEvent {
                time: at,
                channel: 0,
                note: 60,
                kind: NoteEventKind::Tuning { semitones: 0.2 * (i + 1) as f32 },
            });
        }
        let notes = instances(&state, 5.0);
        assert!(notes.len() >= 5, "expected a segment per bend, got {}", notes.len());
        // Along the depth axis, whichever way the pane has it running.
        let axes = Axes::new(PANE, &state.spectrum_config);
        let dir = axes.dir_depth();
        let along = |n: &RollInstance| n.center[0] * dir.x + n.center[1] * dir.y;
        let mut brief = 0;
        for pair in notes.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let clear = (along(b) - along(a)).abs() - (a.half_extent[1] + b.half_extent[1]);
            assert!(
                clear >= -1e-3,
                "two consecutive segments overlap by {} points; the later one's rim \
                 paints over the earlier one's color",
                -clear,
            );
            if a.half_extent[1] < min_half_depth_for(PPP) {
                brief += 1;
            }
        }
        assert!(
            brief > 0,
            "no segment is shorter than the floor, so nothing here could have overlapped",
        );
    }

    /// [`note_instances`]' length floor in points, for a test that needs to
    /// recognise a floored extent.
    fn min_half_depth_for(ppp: f32) -> f32 {
        0.5 * MIN_LENGTH_DEVICE_PX / ppp
    }

    /// How far past the now-line a segment's LEADING end reaches, in points —
    /// negative when it stops short of the line, as every ribbon but a held
    /// note's does.
    ///
    /// Read along the depth axis, which runs away from the spectrum, so the
    /// leading end is the box's low-depth side and this names no screen side.
    fn past_the_line(note: &RollInstance, axes: &Axes, split: f32) -> f32 {
        // The depth direction is axis-aligned in all four orientations, so
        // projecting a screen point onto it drops the pitch coordinate and
        // leaves a position on the depth axis alone.
        let dir = axes.dir_depth();
        let along = |x: f32, y: f32| x * dir.x + y * dir.y;
        let line = axes.at(0.0, split);
        let tip = along(note.center[0], note.center[1]) - note.half_extent[1];
        along(line.x, line.y) - tip
    }

    /// A SOUNDING note carries its ribbon past the now-line and into the
    /// spectrum, by exactly the share of the analyzer it is set to, in every
    /// orientation.
    ///
    /// The lead is the whole point of the setting rather than a decoration on
    /// it: a sounding note's peak is on the other side of that line, and a
    /// ribbon that reaches over says which peak belongs to which note without
    /// the eye having to carry a pitch across a boundary.
    ///
    /// A SHARE OF THE SPECTRUM rather than a length, which is why the expected
    /// reach is derived from the split here instead of being the number the
    /// setting holds: what the bar promises is a fraction of the analyzer, and a
    /// lead that came out the same points at two splits would be breaking it.
    #[test]
    fn a_sounding_note_carries_its_ribbon_past_the_now_line() {
        for orientation in [
            SpectralOrientation::Left,
            SpectralOrientation::Right,
            SpectralOrientation::Top,
            SpectralOrientation::Bottom,
        ] {
            // Two splits as well as three reaches: the spectrum's own share is
            // what the fraction is taken of, so a roll given more of the pane
            // must come out with a proportionally shorter tongue.
            for roll_fraction in [0.3f32, 0.7] {
                for reach in [0.01f32, 0.05, crate::ROLL_LEAD_MAX] {
                    let mut state = fresh();
                    state.spectrum_config.orientation = orientation;
                    state.spectrum_config.roll_seconds = 10.0;
                    state.spectrum_config.low_midi = 48.0;
                    state.spectrum_config.high_midi = 84.0;
                    state.spectrum_config.roll_fraction = roll_fraction;
                    state.spectrum_config.roll_lead = reach;
                    state.spectrum_config.roll_lead_fade = 0.0;
                    // Held since 2 s and never released, so its leading end is
                    // the now-line itself.
                    state.tracker.handle_event(NoteEvent::on(2.0, 0, 60, 1.0));

                    let axes = Axes::new(PANE, &state.spectrum_config);
                    let split = super::super::axes::spectrum_share(&state.spectrum_config);
                    // The bar's promise, in points: that share of the spectrum's
                    // own share of the depth axis.
                    let want = reach * split * axes.depth_len();
                    let held = instances(&state, 5.0);
                    let over = past_the_line(one(&held), &axes, split);
                    assert!(
                        (over - want).abs() < 1e-2,
                        "a sounding note reaches {over} points past the line in \
                         {orientation:?} at a split of {split}, not the {want} \
                         that {reach} of the analyzer comes to",
                    );
                }
            }
        }
    }

    /// ...and a note LONG let go does not. Its leading end is far back in the
    /// roll with no line in front of it to cross, so a lead there would only
    /// draw every note longer than it was played.
    ///
    /// The same note in all three readings, differing only in when — or whether
    /// — the key came up, which is what makes this a test of the release rather
    /// than of the arithmetic.
    #[test]
    fn a_note_long_released_keeps_its_ribbons_own_end() {
        let ribbon = |release: Option<f64>, now: f64| {
            let mut state = fresh();
            state.spectrum_config.orientation = SpectralOrientation::Left;
            state.spectrum_config.roll_seconds = 10.0;
            state.spectrum_config.low_midi = 48.0;
            state.spectrum_config.high_midi = 84.0;
            state.spectrum_config.roll_lead = 0.05;
            state.spectrum_config.roll_lead_release = 0.25;
            state.tracker.handle_event(NoteEvent::on(2.0, 0, 60, 1.0));
            if let Some(at) = release {
                state.tracker.handle_event(NoteEvent::off(at, 0, 60));
            }
            let axes = Axes::new(PANE, &state.spectrum_config);
            let split = super::super::axes::spectrum_share(&state.spectrum_config);
            let notes = instances(&state, now);
            let note = *one(&notes);
            (past_the_line(&note, &axes, split), note.lead, note.lead_alpha)
        };

        let axes = Axes::new(PANE, &fresh().spectrum_config);
        let split = super::super::axes::spectrum_share(&fresh().spectrum_config);
        let want = 0.05 * split * axes.depth_len();
        let (held, lead, alpha) = ribbon(None, 5.0);
        assert!(
            (held - want).abs() < 1e-2 && alpha == 1.0,
            "the sounding note leads {held} at {alpha}, not {want} at full; the \
             case is vacuous",
        );
        assert!(lead > 0.0, "the sounding note carries no lead for the shader to draw");
        // Let go a whole second ago, against a release of a quarter of one: the
        // ribbon is back to its own end, and carries nothing for the shader to
        // fade.
        let (gone, lead, alpha) = ribbon(Some(4.0), 5.0);
        assert!(gone < 0.0, "a note long released still reached {gone} points past the line");
        assert_eq!((lead, alpha), (0.0, 0.0), "a spent lead was still handed to the shader");
    }

    /// What the lead opens in the clip belongs to the LEAD. A cap standing at
    /// the note's own end may not spend it.
    ///
    /// That cap is the exact thing the clip has always existed to stop — see
    /// [`the_rolls_ink_stops_at_the_now_line`](super::super::tests). Before the
    /// lead the clip sat on the now-line and cut every cap that reached it; the
    /// lead opens a budget past the line, and a released note whose own end is
    /// still near the line will spend that budget on a slab of opaque black in
    /// the middle of the analyzer unless something stops it.
    ///
    /// What stops it is the cap's REACH, which is metered to the room the
    /// note's end has actually scrolled clear. So the assertion is on the ink
    /// and not on the arrangement: wherever the cap stands — at the note's own
    /// end inside a lead, or at the box's end without one — everything it
    /// paints is on the roll's side of the line.
    ///
    /// The fixtures below are the three ways in, and the first two are the ways
    /// the cap gets NO room. A release of 0 puts the note's end ON the line at
    /// the moment the lead goes; a long Span puts it a fraction of a point off
    /// the line for seconds after even a slow one, because how far a note
    /// scrolls in the release is the Span's business and the cap's size is not.
    ///
    /// The third is the one that binds, and the test is worth little without
    /// it: a note far enough past the line to be given SOME cap and not the
    /// whole of it. There `cap_px` is `behind_px` less the antialiasing margin
    /// rather than the outline's own reach, so the ink lands exactly on the
    /// line and the assertion is tight against it — drop the margin from
    /// `cap_px` and this is the fixture that fails, where the other two go on
    /// passing because a cap of no reach paints nothing either way.
    #[test]
    fn a_note_that_is_not_leading_keeps_its_ink_behind_the_line() {
        for (release, span, when, hint) in [
            (0.0f32, 12.0f32, 4.0f64, "a release of 0, at the note-off"),
            (0.25, 600.0, 4.3, "a slow Span, just past a quarter-second release"),
            (0.0, 12.0, 4.2, "a release of 0, a fifth of a second on"),
        ] {
            let mut state = fresh();
            state.spectrum_config.orientation = SpectralOrientation::Left;
            state.spectrum_config.roll_seconds = span;
            state.spectrum_config.low_midi = 48.0;
            state.spectrum_config.high_midi = 84.0;
            state.spectrum_config.roll_lead = 0.05;
            state.spectrum_config.roll_lead_fade = 0.04;
            state.spectrum_config.roll_lead_release = release;
            state.spectrum_config.roll_outline = crate::ROLL_OUTLINE_MAX;
            state.tracker.handle_event(NoteEvent::on(2.0, 0, 60, 1.0));
            state.tracker.handle_event(NoteEvent::off(4.0, 0, 60));

            let axes = Axes::new(PANE, &state.spectrum_config);
            let split = super::super::axes::spectrum_share(&state.spectrum_config);
            let note = *one(&instances(&state, when));
            assert_eq!(
                note.lead_alpha, 0.0,
                "{hint}: the lead is still standing at {}, so this says nothing",
                note.lead_alpha,
            );
            // Where the cap stands and how far it reaches, which is one of two
            // pairs. Either the box still carries the (spent) lead, and the cap
            // is at the note's own end a lead inside the tip, reaching by
            // whatever room it was given; or the lead is gone, the box ends at
            // the note, and the cap is the box's own outline at its full reach.
            // Plus the shader's antialiasing ramp, which reaches half a pixel
            // past either.
            //
            // A reach of 0 is the third answer and needs no place: the shader
            // leaves at its first line, so the cap paints nothing anywhere.
            // That is the picture at the note-off itself, when the note's end
            // is ON the line and has no room to be given.
            let tip = past_the_line(&note, &axes, split);
            let (drawn, over) = if note.lead > 0.0 {
                (note.cap_reach > 0.0, tip - note.lead + note.cap_reach + 0.5 / PPP)
            } else {
                (true, tip + note.outline_reach + 0.5 / PPP)
            };
            assert!(
                !drawn || over <= 0.0,
                "{hint}: a note that is not leading reaches {over} points onto \
                 the analyzer",
            );
        }
    }

    /// In between, a released note's lead FADES rather than going: it holds its
    /// full length and its opacity runs down over the Lead release, so the
    /// tongue dissolves where it stands instead of vanishing in a frame.
    ///
    /// Both halves are the setting. The length holding is what makes it a fade
    /// rather than a retraction — a tongue pulled back toward its note is a
    /// second thing moving on a ribbon that is already scrolling — and the
    /// opacity running to nothing on the note's own clock is what makes it a
    /// release rather than a look.
    ///
    /// Read at take times rather than frames, which is the same reason the ramp
    /// is: at 30 fps or 120, offline or live, a note a tenth of a second past
    /// its release is drawn the same.
    #[test]
    fn a_released_notes_lead_fades_out_over_its_release_time() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 10.0;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.spectrum_config.roll_lead = 0.05;
        state.spectrum_config.roll_lead_fade = 0.04;
        state.spectrum_config.roll_lead_release = 0.4;
        state.tracker.handle_event(NoteEvent::on(2.0, 0, 60, 1.0));
        state.tracker.handle_event(NoteEvent::off(4.0, 0, 60));

        let at = |state: &SharedState, now: f64| {
            let notes = instances(state, now);
            let note = *one(&notes);
            (note.lead, note.lead_alpha)
        };
        let (full_lead, _) = at(&state, 4.0);
        assert!(full_lead > 0.0, "the note carries no lead at the moment of release");
        // Quarters of the way through the release, and the opacity is the
        // fraction of it left.
        for (elapsed, want) in [(0.0f64, 1.0f32), (0.1, 0.75), (0.2, 0.5), (0.3, 0.25)] {
            let (lead, alpha) = at(&state, 4.0 + elapsed);
            assert!(
                (alpha - want).abs() < 1e-3,
                "{elapsed}s into a 0.4s release the lead stands at {alpha}, not {want}",
            );
            assert!(
                (lead - full_lead).abs() < 1e-3,
                "the lead retracted to {lead} from {full_lead} while fading",
            );
        }
        // And it is spent at the end of it, geometry and all.
        assert_eq!(at(&state, 4.4), (0.0, 0.0), "the lead outlived its own release");

        // A release of 0 is the instant drop, which is what someone dialling
        // the bar to its bottom is asking for and what the setting exists to
        // answer. Nothing of the lead paints — but the note is still ON the
        // line, so its geometry is held at no opacity rather than snapped back,
        // which is what keeps its outline off the analyzer. See
        // [`a_note_that_is_not_leading_keeps_its_ink_behind_the_line`].
        state.spectrum_config.roll_lead_release = 0.0;
        let (lead, alpha) = at(&state, 4.0);
        assert_eq!(alpha, 0.0, "a release of 0 kept the lead standing past the note-off");
        assert!(lead > 0.0, "the note is still on the line; its box must not snap back yet");
    }

    /// The cap at the note's own end is standing WHILE the lead fades, so the
    /// tongue dissolves off a dark edge that was already there.
    ///
    /// It is the other half of
    /// [`a_released_notes_lead_fades_out_over_its_release_time`], and the half
    /// a released note is actually watched through. The shader draws that cap
    /// under the lead (`harmonigraph_render::RollInstance`'s `cap_reach`), so
    /// what the release does is UNCOVER ink rather than deliver it — but only
    /// as far as this hands the cap room, and a cap with no room is no cap.
    /// Hand it none for the whole release and the edge still arrives in one
    /// frame at the end of it; the pop simply moves.
    ///
    /// So: none at the note-off, when the note's end is on the line and the
    /// space past it is the lead's; growing as the note scrolls its own end
    /// clear; and the outline's full reach well before the lead is spent, which
    /// is what makes the rest of the release a crossfade.
    #[test]
    fn a_released_notes_cap_stands_while_its_lead_is_still_fading() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 10.0;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.spectrum_config.roll_lead = 0.05;
        state.spectrum_config.roll_lead_fade = 0.04;
        state.spectrum_config.roll_lead_release = 0.4;
        state.tracker.handle_event(NoteEvent::on(2.0, 0, 60, 1.0));
        state.tracker.handle_event(NoteEvent::off(4.0, 0, 60));

        let at = |now: f64| *one(&instances(&state, now));
        // On the line, where the space past it is the lead's alone.
        let off = at(4.0);
        assert_eq!(off.cap_reach, 0.0, "a note's end on the line was given cap to spend there");

        // Growing with the room, and never shrinking — the cap comes out of the
        // note's own end rather than being faded in over it.
        let mut previous = 0.0f32;
        let mut full_at = None;
        let mut elapsed = 0.0f64;
        while elapsed <= 0.4 {
            let note = at(4.0 + elapsed);
            // Past the handover the box ends at the note and that end's cap is
            // the box's own outline, so this field says nothing and is zeroed
            // with the rest of the lead's numbers. That the cap is WHOLE by
            // then is [`the_lead_is_given_up_only_once_its_cap_is_whole`]'s,
            // which has the short release this one does not.
            if note.lead <= 0.0 {
                break;
            }
            assert!(
                note.cap_reach >= previous - 1e-4,
                "the cap pulled back to {} from {previous} at {elapsed}s",
                note.cap_reach,
            );
            assert!(
                note.cap_reach <= note.outline_reach + 1e-4,
                "the cap reached {} past an outline of {}",
                note.cap_reach,
                note.outline_reach,
            );
            if full_at.is_none() && note.cap_reach >= note.outline_reach - 1e-4 {
                full_at = Some(elapsed);
            }
            previous = note.cap_reach;
            elapsed += 0.01;
        }

        // And it got there with most of the tongue left to dissolve off it.
        let full_at = full_at.expect("the cap never reached the outline's own reach");
        let standing = at(4.0 + full_at).lead_alpha;
        assert!(
            standing > 0.5,
            "the cap was only whole with {standing} of the lead left ({full_at}s in) — the \
             release is not a crossfade at that point, it is the pop moved",
        );
    }

    /// The lead is given up only once the cap standing inside it is whole, so
    /// the handover from the one to the other moves nothing.
    ///
    /// `leads` and `cap_px` are metered by the same `outline_reach_px`, and
    /// this is the test that says they have to be. Give the lead up while the
    /// cap is still short and the note's end loses ink for a frame and then
    /// gets it back — the pop again, smaller and in the other direction.
    ///
    /// A release of 0 is the fixture, because at any real release the question
    /// does not arise: a note has scrolled points clear of the line before its
    /// tongue is spent, so the cap is long since whole and any two thresholds
    /// would agree. At a release of 0 the lead goes at the note-off and the
    /// handover falls inside the two points of scrolling it takes the note to
    /// clear its own outline, which is the only place the two can disagree.
    #[test]
    fn the_lead_is_given_up_only_once_its_cap_is_whole() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 10.0;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.spectrum_config.roll_lead = 0.05;
        state.spectrum_config.roll_lead_fade = 0.04;
        state.spectrum_config.roll_lead_release = 0.0;
        state.tracker.handle_event(NoteEvent::on(2.0, 0, 60, 1.0));
        state.tracker.handle_event(NoteEvent::off(4.0, 0, 60));

        let at = |elapsed: f64| *one(&instances(&state, 4.0 + elapsed));
        // BISECTED rather than stepped. The cap grows continuously with the
        // scroll, so the last SAMPLE before the handover is short of the cap by
        // whatever the note moved in one step — 0.006 points at a 2 ms step,
        // which is sampling granularity and not the step this is looking for.
        // Squeezing the bracket to nothing is what tells the two apart.
        assert!(at(0.0).lead > 0.0, "the lead was never on the box at all");
        let (mut with, mut without) = (0.0f64, 1.0f64);
        assert!(at(without).lead <= 0.0, "the lead outlived a release of 0 by a second");
        for _ in 0..60 {
            let mid = 0.5 * (with + without);
            if at(mid).lead > 0.0 {
                with = mid;
            } else {
                without = mid;
            }
        }
        let last = at(with);
        assert!(
            last.cap_reach >= last.outline_reach - 1e-4,
            "the lead was given up at {without}s with its cap at {} of {} — the \
             handover is a step, not a continuation",
            last.cap_reach,
            last.outline_reach,
        );
    }

    /// Only the note's LAST segment leads. The rest end on the next bend, in
    /// the middle of the ribbon, and a lead on one of those would tear a bent
    /// note into overlapping pieces at every breakpoint.
    #[test]
    fn only_the_leading_segment_of_a_sounding_note_leads() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 10.0;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.spectrum_config.roll_lead = 0.05;
        state.spectrum_config.roll_lead_fade = 0.03;
        state.tracker.handle_event(NoteEvent::on(2.0, 0, 60, 1.0));
        for (i, at) in [2.5, 3.0, 3.5].into_iter().enumerate() {
            state.tracker.handle_event(NoteEvent {
                time: at,
                channel: 0,
                note: 60,
                kind: NoteEventKind::Tuning { semitones: 0.4 * (i + 1) as f32 },
            });
        }
        let notes = instances(&state, 5.0);
        assert!(notes.len() >= 4, "expected a segment per bend, got {}", notes.len());
        let leading: Vec<&RollInstance> = notes.iter().filter(|n| n.lead > 0.0).collect();
        assert_eq!(
            leading.len(),
            1,
            "{} of {} segments lead; only the last one may",
            leading.len(),
            notes.len(),
        );
        // And it is the LAST one — the segments are built in time order, and
        // depth runs away from the now-line, so the leading segment is the one
        // nearest it.
        let axes = Axes::new(PANE, &state.spectrum_config);
        let split = super::super::axes::spectrum_share(&state.spectrum_config);
        let nearest = notes
            .iter()
            .max_by(|a, b| {
                past_the_line(a, &axes, split).total_cmp(&past_the_line(b, &axes, split))
            })
            .expect("the roll drew at least one segment");
        assert!(nearest.lead > 0.0, "the segment that leads is not the one nearest the line");
        // The segments still TILE: the lead grows the last one's leading end,
        // which is the end no other segment is behind, so it can overlap none
        // of them.
        let dir = axes.dir_depth();
        let along = |n: &RollInstance| n.center[0] * dir.x + n.center[1] * dir.y;
        for pair in notes.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let clear = (along(b) - along(a)).abs() - (a.half_extent[1] + b.half_extent[1]);
            assert!(
                clear >= -1e-3,
                "the lead pushed two segments {} points into each other",
                -clear,
            );
        }
    }

    /// The lead's fade rides its own setting and is bounded by the reach it
    /// softens, exactly as the outline's is and for the same reason: past that
    /// there is nothing left to fade, and the bar would have no pair of points
    /// to put on its axis.
    ///
    /// Off has to mean zero reach AND no fade: the reach is what the geometry
    /// grows by, so a lead that will not be drawn must not be paid for in a
    /// fade the shader would then apply to the ribbon's own square end.
    #[test]
    fn the_leads_fade_is_its_own_setting_and_stops_at_the_reach() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        // The fractions come back as points, so each expectation is stated in
        // the currency the bar is dialled in and converted the way [`lead`]
        // converts it.
        let axes = Axes::new(PANE, &state.spectrum_config);
        let split = super::super::axes::spectrum_share(&state.spectrum_config);
        let px = |share: f32| share * split * axes.depth_len();
        let fade_at = |state: &mut SharedState, reach: f32, fade: f32| {
            state.spectrum_config.roll_lead = reach;
            state.spectrum_config.roll_lead_fade = fade;
            let notes = instances(state, 0.05);
            one(&notes).lead_fade
        };
        // Within float error rather than to the bit: a fraction reaches the
        // instance through two multiplications, and the clamped case reaches it
        // through a `min` of two numbers that took different routes there.
        let same = |got: f32, want: f32, what: &str| {
            assert!((got - want).abs() < 1e-3, "{what}: {got} against {want}");
        };
        same(fade_at(&mut state, 0.06, 0.0), 0.0, "a fade of 0 is a square end, not a default");
        same(fade_at(&mut state, 0.06, 0.02), px(0.02), "the fade is not its own setting");
        same(fade_at(&mut state, 0.06, 0.06), px(0.06), "a fade of the whole lead is allowed");
        same(fade_at(&mut state, 0.06, 0.09), px(0.06), "a fade past the lead was not clamped");
        same(fade_at(&mut state, 0.0, 0.04), 0.0, "a lead of no reach kept a fade");
    }

    /// A pane with no spectrum on it has no line to cross, and nothing leads.
    ///
    /// Two layouts reach that state and both are ordinary rather than
    /// degenerate: the offline `--playhead` render lays the whole take out
    /// statically and gives the roll the entire depth axis, and the divider
    /// dragged all the way over the curve leaves the same picture live. A lead
    /// in either would be a ribbon poking off the end of the pane.
    #[test]
    fn a_pane_with_no_spectrum_has_no_line_to_lead_over() {
        let cfg = crate::SpectrumConfig {
            roll_lead: 0.05,
            roll_lead_fade: 0.04,
            ..fresh().spectrum_config
        };
        let axes = Axes::new(PANE, &cfg);
        let px = |share: f32| share * 0.45 * axes.depth_len();
        let (reach, fade) = lead(&cfg, &axes, false, 0.45);
        assert!(
            (reach - px(0.05)).abs() < 1e-3 && (fade - px(0.04)).abs() < 1e-3,
            "an ordinary pane leads ({reach}, {fade}) rather than ({}, {})",
            px(0.05),
            px(0.04),
        );
        assert_eq!(
            lead(&cfg, &axes, true, 0.45),
            (0.0, 0.0),
            "the whole-song layout drew a lead",
        );
        assert_eq!(
            lead(&cfg, &axes, false, 0.0),
            (0.0, 0.0),
            "a pane with no spectrum drew a lead",
        );
    }

    /// A note off the octave zoom is dropped when its INK is off it, and how
    /// far its ink reaches along pitch is a reading of its own slope.
    ///
    /// `vs_note` grows the quad by `skew * reach` along pitch, where `skew` is
    /// `sqrt(1 + slope^2)` — so the outline of a steep glide stands a multiple
    /// of its own reach out along that axis, and a cull that used the plain
    /// reach dropped such a note while ink was still owed inside the range.
    /// Two notes with the SAME pitch endpoints settle it: the cull cannot be
    /// reading only those, or the two come out alike.
    #[test]
    fn a_steep_glide_is_kept_for_the_outline_it_still_owes_the_range() {
        // The bend applied `after` seconds into a note held from 2 s to 5 s.
        // Immediately (one block, what per-note tuning actually does) it is a
        // segment a fraction of a point long carrying the whole semitone;
        // spread over two seconds it is a shallow ramp. Same endpoints either
        // way, and both sit above the zoom.
        let glide = |after: f64| {
            let mut state = fresh();
            state.spectrum_config.orientation = SpectralOrientation::Left;
            state.spectrum_config.roll_seconds = 10.0;
            // The narrowest zoom there is (`PITCH_RANGE_MIN_SPAN`); anything
            // narrower is widened back out under the test's feet.
            state.spectrum_config.low_midi = 36.0;
            state.spectrum_config.high_midi = 60.0;
            state.spectrum_config.roll_outline = crate::ROLL_OUTLINE_MAX;
            state.tracker.handle_event(NoteEvent::on(2.0, 0, 62, 1.0));
            state.tracker.handle_event(NoteEvent {
                time: 2.0 + after,
                channel: 0,
                note: 62,
                kind: NoteEventKind::Tuning { semitones: 1.0 },
            });
            instances(&state, 5.0)
        };

        // Shallow: two semitones clear of the zoom is further than an outline
        // of any width reaches, so nothing is owed and nothing is drawn.
        assert!(
            glide(2.0).is_empty(),
            "a note two semitones off the zoom drew anyway; the cull is doing nothing",
        );
        // Steep: the same note, bent in one block instead of over two seconds
        // — the same two endpoints, and an outline that does reach in.
        let steep = glide(0.011);
        assert!(
            !steep.is_empty(),
            "a steep glide was culled while its outline still stood inside the range",
        );
        assert!(
            steep.iter().any(|n| n.shear.abs() > 1.0),
            "no segment is steep enough for the skew to matter; the case is vacuous",
        );
    }

    /// A glide is the same instance sheared, not a second kind of shape: the
    /// note's center line drifts along pitch as it runs down the depth axis,
    /// which makes the box a parallelogram and costs nothing else.
    #[test]
    fn a_glide_shears_the_note_rather_than_needing_another_shape() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 55.0;
        state.spectrum_config.high_midi = 67.0;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        let held = instances(&state, 1.0);
        assert_eq!(one(&held).shear, 0.0, "a held note should not be sheared");

        // Bend it a semitone up over the next second.
        state.tracker.handle_event(NoteEvent {
            time: 1.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: 1.0 },
        });
        let bent = instances(&state, 2.0);
        bent.iter()
            .find(|n| n.shear != 0.0)
            .expect("the bent segment should be sheared");
    }

    /// Notes are placed sub-pixel, and that is load-bearing. egui snaps rects
    /// to whole pixels by default (`round_rects_to_pixels`) to keep static
    /// chrome crisp; these scroll, and snapping held a note still until it had
    /// drifted a whole pixel and then jumped it, while the spectrogram — a
    /// mesh, never snapped — slid smoothly underneath, so the notes read as
    /// jittering against it. Four frames a fraction of a pixel apart must
    /// move the note by that same fraction each time, not by 0 then 1.
    ///
    /// Measured on a note well past the LENGTH FLOOR, and the fixture checks
    /// that rather than trusting the Span. A sounding note still under the
    /// floor does not scroll at all — its leading end is `now`, which is the
    /// now-line, and the floor stands in for a length it has not reached, so
    /// the box is pinned there until it has one (see
    /// [`a_sounding_note_is_floored_into_the_past_alone`]). A fixture straddling
    /// that crossover reads the transition as a snap and fails for a reason
    /// that has nothing to do with rounding. At the default 180 s Span a point
    /// is over a second, which puts the crossover a second in.
    #[test]
    fn a_scrolling_note_moves_sub_pixel_rather_than_in_whole_pixel_jumps() {
        let mut state = fresh();
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 55.0;
        state.spectrum_config.high_midi = 67.0;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        // A step that scrolls the roll by well under one point.
        let axes = Axes::new(PANE, &state.spectrum_config);
        let window = f64::from(state.spectrum_config.roll_seconds);
        let step = 0.3 * window / f64::from(axes.depth_len());
        let at = |now: f64| {
            let notes = instances(&state, now);
            let note = one(&notes);
            egui::pos2(note.center[0], note.center[1])
        };
        let from = 5.0;
        assert!(
            one(&instances(&state, from)).half_extent[1] > MIN_LENGTH_DEVICE_PX / PPP,
            "the fixture sits on the length floor, where the note is pinned rather \
             than scrolling, and the crossover would read as a snap",
        );
        let moves: Vec<f32> = (0..4)
            .map(|i| at(from + step * (i + 1) as f64) - at(from + step * i as f64))
            .map(|d| d.length())
            .collect();
        for step in &moves {
            assert!(
                *step > 0.0 && *step < 0.5,
                "a sub-pixel scroll moved the note by {step} points: {moves:?}",
            );
        }
        let spread = moves.iter().fold(0.0f32, |a, &b| a.max(b))
            - moves.iter().fold(f32::MAX, |a, &b| a.min(b));
        assert!(spread < 1e-3, "the note scrolled unevenly (snapped?): {moves:?}");
    }
}
