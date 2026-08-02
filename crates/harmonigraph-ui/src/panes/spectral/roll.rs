//! The Spectral pane's piano roll: incoming MIDI drawn as ribbons over
//! the same pitch axis the spectrum uses.
//!
//! Not a piano roll in the DAW sense — there are no black/white key lanes
//! and no bar grid, because the pitch axis is the *lattice's* axis: it is
//! continuous in cents, so a bent or microtonally tuned note sits between
//! the keys rather than being quantized onto one. What it keeps from the
//! DAW idea is the shape: pitch across, time along, one ribbon per note.
//!
//! Geometry comes entirely from [`Axes`](super::axes::Axes), so the roll
//! turns with the rest of the pane and this file never names a screen side.
//! Its share of the depth axis runs from `split` (now) to 1 (the
//! oldest note still on screen), so time flows *away* from the spectrum
//! and a note crossing the split meets the peak it is making.

use egui::Color32;
use harmonigraph_core::RollNote;
use harmonigraph_render::{RollAxes, RollInstance};
use harmonigraph_scene::channel_color;

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

/// How wide the white keyline riding a note's outer edge is, in points.
///
/// Fixed rather than a setting: a full point is the only width worth having.
/// The keyline is the brightest thing on a note, and a *bright* sub-point line
/// shimmers as the roll scrolls, its peak intensity wobbling with every
/// sub-pixel step across the grid (worst on a Hi-DPI display, where 0.6 points
/// is barely over one physical pixel). Wider, and a highlight on a ribbon a
/// few points thick starts reading as a second ribbon. Edge is the knob that
/// matters here, and it sets how BRIGHT the line is.
const KEYLINE_PX: f32 = 1.0;

/// The roll's glow reads this much brighter than the raw Edge fraction, so a
/// modest Edge setting still lands a crisp highlight over a bright spectrogram.
/// Only the roll's glow is boosted; the spectrum profile's edge ([`keyline`])
/// is left at the fraction, since it is one line on a filled slab and does not
/// have a dark backing to be seen against.
const GLOW_INTENSITY: f32 = 2.0;

/// How strong the Edge rim is here: the Edge setting scaled by the note's own
/// opacity, or `None` when there is too little of it to draw. The gate for the
/// keyline, which is the whole of a note's rim.
fn edge_strength(cfg: &crate::SpectrumConfig, alpha: f32) -> Option<f32> {
    let strength = cfg.keyline.clamp(0.0, 1.0) * alpha;
    (strength > 0.004).then_some(strength)
}

/// The light edge drawn along the spectrum's profile, at `cfg.keyline`
/// strength — `None` when the setting is off.
///
/// The curve's own colors come from the spectrogram palette, so where it is
/// quiet it is drawn in that palette's dark end against the pane's dark
/// background, with no edge, and the shape stops existing. A light rim gives
/// it an edge to be seen by. It is a setting because how much is right depends
/// entirely on which palette is in play — Mono runs to white and swallows a rim
/// the even ramps leave standing.
///
/// The roll's notes carry a brighter version of this (see [`rim`]); the profile
/// keeps the plain fraction, being one line on a filled slab rather than a
/// shape to pick out of a picture.
pub(super) fn keyline(cfg: &crate::SpectrumConfig, alpha: f32) -> Option<Color32> {
    edge_strength(cfg, alpha).map(|s| Color32::WHITE.gamma_multiply(s))
}

/// The one band standing outside a note: `(width, color)` of the white
/// keyline, brighter than the raw Edge fraction (see [`GLOW_INTENSITY`]).
///
/// A note carries no solid black outline under this. A solid note is its own
/// separation from whatever the spectrogram is doing behind it, and a black
/// outline reads as a second, heavier one around every note — most of all on
/// the thin ribbons this pane is used at, where two dark lines and a bright one
/// is more edge than note.
///
/// Off comes back zero-width AND transparent, never one or the other: the width
/// is what the quad grows by to make room for the band (and what the far-edge
/// cull keeps a leaving note alive for), so a band that will not paint must not
/// be paid for either.
fn keyline_band(cfg: &crate::SpectrumConfig, alpha: f32) -> (f32, Color32) {
    if edge_strength(cfg, alpha).is_none() {
        return (0.0, Color32::TRANSPARENT);
    }
    let s = (cfg.keyline.clamp(0.0, 1.0) * GLOW_INTENSITY).min(1.0) * alpha;
    (KEYLINE_PX, Color32::WHITE.gamma_multiply(s))
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
    let dir = |v: egui::Vec2| [v.x, v.y];
    painter.add(harmonigraph_render::roll_paint_callback(
        axes.rect,
        notes,
        RollAxes { pitch_dir: dir(axes.dir_pitch()), depth_dir: dir(axes.dir_depth()) },
        state.target_format,
        surface as u64,
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
    // Whole-song (offline playhead): the render lays the whole take out at once
    // from a full roll built up front. Live: the causal tracker's rolling
    // window, filling in as notes arrive.
    let roll = match state.whole_song.as_ref() {
        Some(ws) => &ws.roll,
        None => state.tracker.roll(),
    };
    // The whole look of a note, decided once for the roll rather than per
    // note: the keyline standing outside its long edges. The note itself is a
    // solid rectangle of its own color and has nothing else to decide.
    let (keyline_px, light) = keyline_band(cfg, 1.0);

    // Cull to the visible window BEFORE sorting: the roll can remember
    // thousands of notes while only a handful are on screen, and sorting the
    // survivors alone (rather than every remembered note) keeps the same
    // deterministic paint order for far less work.
    //   - Entirely past the window's far end, or
    //   - entirely off the octave zoom (both endpoints outside and on the
    //     same side, so a note that merely crosses an edge still draws its
    //     visible part).
    // A note paints past its own box: the keyline standing against it, and the
    // antialiasing ramp (`reach` in roll.wgsl). The box
    // is what the window is tested against, so without the overhang below a
    // note vanishes while that ink is still owed — the ribbon pops out of
    // existence a few points short of the edge instead of sliding under it.
    // In time that is the span divided by the roll's own length, so it grows
    // with the Span: ~200 ms across 10 s, over a second across a minute.
    //
    // The far edge moves out by exactly that, for the cull AND for the clamp
    // below, so the box overhangs and the pane's scissor takes the ink off as
    // it leaves. Bounded on purpose — an unclamped overhang is what makes a
    // ten-minute note a ten-minute quad.
    //
    // Not in whole-song mode: there the region's far end is the take's start,
    // and overhanging it would paint into the spectrum curve above, which no
    // scissor cuts.
    //
    // `min_half_depth` is in the overhang because the length floor grows a
    // leaving note's box back toward the region as its true length is truncated
    // to nothing — so without it the last sliver of a floored note would appear
    // INSIDE the far edge on the frame before the cull takes it, which is the
    // pop the overhang exists to prevent.
    let ink_px = keyline_px + 1.0 + min_half_depth;
    // Seconds per point of the roll — the pane's depth axis is shared with the
    // spectrum, so it is the ROLL's share of it that a point is measured
    // against. What every screen-space length here (the ink overhang) is
    // converted through.
    let per_point = f64::from(1.0 / (axes.depth_len() * (1.0 - split)).max(1.0)) * time.window();
    let ink_seconds = if time.whole_song() { 0.0 } else { f64::from(ink_px) * per_point };
    let edge = oldest - ink_seconds;
    let mut notes: Vec<&RollNote> = roll
        .notes()
        .filter(|note| {
            if note.stop(now) < edge {
                return false;
            }
            let (a, b) = (note.start_pitch(), note.end_pitch());
            let (lo, hi) = (a.min(b), a.max(b));
            hi >= scale.min_midi - cfg.roll_thickness && lo <= scale.max_midi + cfg.roll_thickness
        })
        .collect();
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
        for ((t0, p0), (t1, p1)) in note.segments(now) {
            let (t0, t1) = (t0.max(edge), t1.max(edge));
            if t1 < edge {
                continue;
            }
            // Unclamped: `edge` already bounds how far past the region these
            // can reach, and clamping is what squashed the leaving ribbon
            // against the far end rather than letting it slide out.
            let (d0, d1) = (time.depth_of_unclamped(t0), time.depth_of_unclamped(t1));
            let (a0, a1) = (scale.t_of(p0), scale.t_of(p1));

            // Notes always draw fully opaque — how much of the heatmap comes
            // through a note is the Fill setting's business, not an opacity
            // here, and a released note fades on the lattice's fade.
            let alpha = 1.0;
            let pitch = (p0 + p1) * 0.5;
            // The note's TRUE color, so a note matches the node it lit up on
            // the lattice. It is painted solid, edge to edge — the interior
            // and the boundary are one thing, and a heatmap cell showing
            // through a note said neither clearly.
            let core = note_color(note, state, pitch, alpha);
            // Reading outward: the note, the bright white keyline standing
            // against its long edges, then whatever the spectrogram is doing.
            //
            // The keyline stands entirely OUTSIDE the note, never as a stroke
            // of its path — a centered stroke grows inward exactly as much as
            // outward, and a note is only a few pixels thick at the pitch
            // ranges this pane is actually used at, so the two long edges meet
            // in the middle and paint over it. Here that is structural: the
            // shader reads the band off the DISTANCE to the note's edge, and a
            // band at distance 0..1 cannot reach back inside it whatever the
            // ribbon's thickness.
            //
            // Nothing else rides the rim — no solid black outline under the
            // keyline, and none approximating the lattice's bloom. The bloom
            // is the lattice's alone (there it is a real post-process pass,
            // not hand-placed bands standing in for one), and the black is
            // edge a solid note does not need.

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
            // Floored the same way the width is, and centered the same way — on
            // the middle of what the segment actually was, so a brief note reads
            // up to half the floor early at one end and late at the other rather
            // than being pushed off its own moment in one direction.
            let half_depth = (depth_px.abs() * 0.5).max(min_half_depth);
            // How far the note's center line drifts along the pitch axis per
            // point of depth: 0 for a held note, non-zero for a glide, which
            // shears the box into the parallelogram the ribbon follows. Guarded
            // because a segment can have no duration at all — a note pressed
            // this frame is one — and a slope is meaningless there.
            //
            // Taken against the box's OWN depth, which is why it is derived
            // after the floor. The shader reaches `|shear| * half_extent[1]`
            // along pitch, so the shear and the length are one product: a shear
            // left at the segment's true rate while the floor lengthens the box
            // multiplies that reach by however much the floor won by, which is
            // unbounded as the segment shortens. Per-note tuning hands us that
            // case on every retuned note — the tuning lands a block after the
            // note-on (`RollNote::SETTLE`), so the opening segment is
            // milliseconds long and carries the whole offset — and it drew a
            // diagonal streak, keyline and all, through pitches nothing
            // sounded. Scaled to the drawn box, the reach is the segment's real
            // drift at every length.
            //
            // What that costs is the shear itself: a segment the floor
            // lengthened draws its drift spread over the floored length, so it
            // reads as a shallower bend than it was. That is the honest way
            // round. A slope is a ratio of two things, and the floor has
            // already overstated the denominator on purpose; overstating the
            // numerator to match would put ink where no note was, and pitch is
            // the axis this pane exists to be read precisely on.
            let slope = if depth_px.abs() > 1e-6 {
                (a1 - a0) * axes.pitch_len() / depth_px * (depth_px.abs() * 0.5 / half_depth)
            } else {
                0.0
            };
            instances.push(RollInstance {
                center: [center.x, center.y],
                half_extent: [half_pitch, half_depth],
                shear: slope,
                keyline: keyline_px,
                core: core.to_array(),
                glow: light.to_array(),
            });
        }
    }
    instances
}

/// The color of a note at `pitch`: the lattice's own, by the same
/// [`channel_color`] the nodes are painted through, so a ribbon and the node
/// it lit up are the same color.
///
/// The only coloring there is, deliberately. The two obvious alternatives —
/// the low-to-high pitch ramp on every channel, and one flat accent — both
/// break the identity that makes the roll readable beside the lattice: what a
/// color means has to be the same thing in both pictures, or reading across
/// them is a translation.
fn note_color(note: &RollNote, state: &SharedState, pitch: f32, alpha: f32) -> Color32 {
    let (darkest, brightest) =
        (state.frame_params.darkest_pitch, state.frame_params.brightest_pitch);
    scene_color(channel_color(note.channel, pitch, darkest, brightest), alpha)
}


#[cfg(test)]
mod tests {
    use super::*;
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
    /// [`spectral_pane`](super::super::axes::spectral_pane) derives it
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
    /// rim geometry is under the most pressure.
    fn ribbon_with_range(keyline: f32, range: f32) -> Vec<RollInstance> {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 60.0 - range * 0.5;
        state.spectrum_config.high_midi = 60.0 + range * 0.5;
        state.spectrum_config.roll_thickness = 2.0;
        state.spectrum_config.keyline = keyline;
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        instances(&state, 0.05)
    }

    /// The note is 12 semitones across a 100-point axis (a thick ribbon), so
    /// it is bounded rather than drawn as a bare spine.
    fn ribbon(keyline: f32) -> Vec<RollInstance> {
        ribbon_with_range(keyline, 12.0)
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
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 10.0;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.tracker.handle_event(NoteEvent {
            time: 1.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        state.tracker.handle_event(NoteEvent {
            time: 1.5,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Off,
        });

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

    /// The rim is a fixed pixel thickness whatever the note's own width — an
    /// outline should not thin out just because the ribbon it wraps did — and
    /// it is expressed as a DISTANCE OUTSIDE the note's outline, never as a
    /// wider stroke of it.
    ///
    /// That distinction is the flood fix, and it is now structural: the shader
    /// reads the keyline off distances `w/2 .. w/2 + keyline`, so it cannot
    /// reach back inside the ribbon however thin it is. What is left to check
    /// here is that the thickness is handed over unscaled.
    #[test]
    fn the_rim_is_the_same_thickness_at_any_note_width() {
        let thick = ribbon_with_range(0.5, 12.0);
        // ~120 semitones over 100 points: the ribbon is under 2 points thick,
        // which is where centered strokes meet in the middle and paint the
        // interior white.
        let thin = ribbon_with_range(0.5, 120.0);
        assert_eq!(one(&thick).keyline, KEYLINE_PX, "the keyline is not the width it is fixed at");
        assert_eq!(one(&thin).keyline, one(&thick).keyline, "the keyline thinned with the note");
        assert!(
            one(&thin).half_extent[0] < one(&thick).half_extent[0],
            "the two notes are the same thickness; the comparison is vacuous",
        );
    }

    /// The keyline is bright: reading outward, the note's color, then a punchy
    /// highlight. It reads stronger than the raw Edge fraction — that boost is
    /// the "more intense" ask.
    ///
    /// Edge is the whole of the gate now that the black outline is gone: at 0
    /// the band is off, which has to mean zero width AND no color, never one
    /// or the other. The width is what the quad grows by to make room for the
    /// band (and what keeps a leaving note alive past the far edge), so a band
    /// that will not paint must not be paid for either.
    #[test]
    fn the_keyline_is_bright_and_edge_turns_it_off_completely() {
        // A modest Edge, below the point where the boosted keyline clips to
        // full, so "brighter than the fraction" is a real comparison.
        let edge = 0.3;
        let lit = ribbon(edge);
        let note = one(&lit);
        let glow_alpha = f32::from(note.glow[3]) / 255.0;
        assert!(
            glow_alpha > edge + 0.05,
            "the keyline ({glow_alpha}) is no brighter than the Edge fraction {edge}",
        );
        assert_eq!(note.keyline, KEYLINE_PX, "the keyline is not the width it is fixed at");

        let dark = ribbon(0.0);
        let note = one(&dark);
        assert_eq!(note.keyline, 0.0, "Edge 0 still made room for a keyline");
        assert_eq!(note.glow[3], 0, "Edge 0 left a keyline color behind");
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
    #[test]
    fn a_brief_note_is_floored_at_the_length_it_can_scroll_without_flickering() {
        // A 10 s span across 300 points of depth, of which the roll takes its
        // share: about 60 ms per point, so a 20 ms tap is well under the floor
        // and a 2 s note is well over it.
        let tap = |length: f64| {
            let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
            state.spectrum_config.orientation = SpectralOrientation::Left;
            state.spectrum_config.roll_seconds = 10.0;
            state.spectrum_config.low_midi = 48.0;
            state.spectrum_config.high_midi = 84.0;
            state.tracker.handle_event(NoteEvent {
                time: 2.0,
                channel: 0,
                note: 60,
                kind: NoteEventKind::On { velocity: 1.0 },
            });
            state.tracker.handle_event(NoteEvent {
                time: 2.0 + length,
                channel: 0,
                note: 60,
                kind: NoteEventKind::Off,
            });
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
        assert_eq!(brief.half_extent[1], floor, "a brief note was left to flicker");

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
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
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

    /// The floor is in DEVICE pixels, so it is half as many points on a 2x
    /// display as on a 1x one — the antialiasing ramp it is sized against is
    /// one physical pixel wide, and a floor in points would be right on
    /// exactly one class of display.
    #[test]
    fn the_length_floor_follows_the_display_density() {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 10.0;
        state.tracker.handle_event(NoteEvent {
            time: 2.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        state.tracker.handle_event(NoteEvent {
            time: 2.001,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Off,
        });
        let cfg = &state.spectrum_config;
        let axes = Axes::new(PANE, cfg);
        let scale = PitchScale { min_midi: 48.0, max_midi: 84.0, span: 36.0 };
        let split = super::super::axes::spectrum_share(cfg);
        let at = |ppp| {
            let notes = note_instances(&axes, &scale, &state, split, 5.0, ppp);
            one(&notes).half_extent[1]
        };
        assert_eq!(at(1.0), 0.5 * MIN_LENGTH_DEVICE_PX, "1x: two points");
        assert_eq!(at(2.0), 0.25 * MIN_LENGTH_DEVICE_PX, "2x: one point");
        assert!(at(1.0) > at(2.0), "the floor did not follow the density at all");
    }

    /// A floored segment must not paint outside the pitch it covered.
    ///
    /// `shear` is a RATE — pitch points per point of depth — and the shader
    /// reaches `|shear| * half_extent[1]` along pitch, so the length floor and
    /// the shear are one product. Leaving the shear at the true rate while the
    /// floor lengthens the box multiplies that reach by however much the floor
    /// won by, which is unbounded as the segment shortens: it draws a diagonal
    /// streak, keyline and all, through pitches the note never sounded.
    ///
    /// Per-note tuning makes that the ordinary case rather than a corner. A
    /// note-on arrives at the key's pitch and the tuning expression lands a
    /// block later (see `RollNote::SETTLE` — 11 ms at 48 kHz), so every retuned
    /// note opens with a segment a few milliseconds long carrying its whole
    /// offset. Measured before the bound, at the Span below: 13.8 points of
    /// drawn reach against 0.4 of real drift, and 49 semitones at the Span
    /// bar's top.
    #[test]
    fn a_floored_segment_stays_inside_the_pitch_it_covered() {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.roll_seconds = 60.0;
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.tracker.handle_event(NoteEvent {
            time: 2.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        // One 512-frame block at 48 kHz behind the note-on, which is what a
        // host's per-note tuning actually does.
        state.tracker.handle_event(NoteEvent {
            time: 2.011,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: 0.3 },
        });
        let axes = Axes::new(PANE, &state.spectrum_config);
        // The whole drift any segment of this note can carry, as a half-extent
        // in points: 0.3 semitones of a 36-semitone axis. No segment may reach
        // further than this along pitch, whatever the floor did to its length.
        let bound = 0.3 / 36.0 * axes.pitch_len() * 0.5;

        let notes = instances(&state, 5.0);
        assert!(notes.len() >= 2, "the tuning should have split the note into segments");
        let mut floored = 0;
        for note in &notes {
            let reach = note.shear.abs() * note.half_extent[1];
            assert!(
                reach <= bound + 1e-3,
                "a segment reaches {reach} points along pitch, against {bound} of real drift",
            );
            if note.half_extent[1] > min_half_depth_for(2.0) - 1e-6
                && note.half_extent[1] < min_half_depth_for(2.0) + 1e-6
            {
                floored += 1;
            }
        }
        assert!(floored > 0, "no segment was short enough to be floored; the test is vacuous");
    }

    /// [`note_instances`]' length floor in points, for a test that needs to
    /// recognise a floored extent.
    fn min_half_depth_for(ppp: f32) -> f32 {
        0.5 * MIN_LENGTH_DEVICE_PX / ppp
    }

    /// A glide is the same instance sheared, not a second kind of shape: the
    /// note's center line drifts along pitch as it runs down the depth axis,
    /// which makes the box a parallelogram and costs nothing else.
    #[test]
    fn a_glide_shears_the_note_rather_than_needing_another_shape() {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 55.0;
        state.spectrum_config.high_midi = 67.0;
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
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
    #[test]
    fn a_scrolling_note_moves_sub_pixel_rather_than_in_whole_pixel_jumps() {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.spectrum_config.orientation = SpectralOrientation::Left;
        state.spectrum_config.low_midi = 55.0;
        state.spectrum_config.high_midi = 67.0;
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        // A step that scrolls the roll by well under one point.
        let axes = Axes::new(PANE, &state.spectrum_config);
        let window = f64::from(state.spectrum_config.roll_seconds);
        let step = 0.3 * window / f64::from(axes.depth_len());
        let at = |now: f64| {
            let notes = instances(&state, now);
            let note = one(&notes);
            egui::pos2(note.center[0], note.center[1])
        };
        let moves: Vec<f32> = (0..4)
            .map(|i| at(1.0 + step * (i + 1) as f64) - at(1.0 + step * i as f64))
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
