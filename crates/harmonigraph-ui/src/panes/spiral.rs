//! The Spiral pane: the analyzer's current spectrum wound onto a chroma
//! spiral — one turn per octave — with the sounding MIDI notes marked on it.
//!
//! Angle is the pitch CLASS and radius the octave, so a pitch class is a
//! DIRECTION: every C sits on one ray out of the centre whatever octave it is
//! in, and a note's harmonic series lands on the rays of the intervals it
//! makes — the octave and the double octave back on its own ray, the twelfth a
//! fifth round, the seventeenth a major third. That is the one reading the
//! straight pitch axis in [`spectral`](super::spectral) cannot give: there an
//! octave is a distance like any other, and here it is the same place.
//!
//! This draws the analyzer's CURRENT frame and nothing else — no trail, no
//! history, and no sharpening of any kind. The picture is the existing
//! [`AudioSpectrum::display`](crate::AudioSpectrum::display) buckets in polar
//! coordinates, which is why it costs no DSP at all.
//!
//! It shares the Analyzer's [`SpectrumConfig`](crate::SpectrumConfig) whole
//! rather than carrying settings of its own, so the pitch range, level window,
//! tilt and gradient are the ones dialled in the Display tab's Analyzer
//! section, and "loud" means the same thing here as it does there — the same
//! property already held between that pane's curve and its heatmap. It reads
//! through their [`loudness`] and [`cell_color`] unmodified to get it.
//!
//! One consequence of sharing worth expecting: `tilt` pivots at 1 kHz, so on a
//! spiral it lifts by RADIUS rather than along a straight axis — a brightness
//! that grows outward from the middle turns.

use egui::Color32;

use super::spectral::axes::loudness;
use super::spectral::roll::note_color;
use super::spectral::spectrogram::{cell_color, power_mean};
use crate::SharedState;

/// How much of the disc's radius the hole in the middle keeps, as a share of
/// the outer radius.
///
/// It has to be positive, and generously so: radius is linear in octave, so an
/// innermost turn drawn near the centre gets almost no circumference to spell
/// twelve pitch classes across. At the analyzer's full ~10-octave range this
/// leaves the innermost turn about a sixth of the outer one's circumference,
/// which is coarse but still reads as twelve directions rather than as a blot.
/// Narrowing the Analyzer's pitch range is what buys the inner turns room, and
/// is the intended way to read this pane closely.
const INNER_HOLE: f32 = 0.16;

/// Points of air between the outermost turn and the pane edge.
const MARGIN_PT: f32 = 4.0;

/// Roughly how many points of arc one drawn segment covers. The strip is a
/// polygon, so this is what decides whether its turns read as curves.
const SEGMENT_PT: f32 = 1.5;

/// Fewest segments the whole strip is ever cut into, so a pane too small (or a
/// range too narrow) to earn segments by arc length still draws a curve.
const MIN_STEPS: f32 = 96.0;

/// How faint the twelve pitch-class rays are drawn against
/// [`theme::hairline`](crate::theme::hairline): C first, the other eleven
/// second.
///
/// The same two-weight shape as the Spectral pane's frequency rulings, and for
/// the same reason — a ladder of identical marks is a ruler with no zero on it.
/// Here the stronger one is C because the ray it names is what tells you which
/// way round the circle you are looking, and every other ray is counted from
/// it.
const RAY_FADE: (f32, f32) = (0.8, 0.32);

/// Width of a sounding note's radial mark, and of the dark backing under it, in
/// points.
///
/// A backing rather than a heavier line, for the same reason the axis labels
/// are haloed and a note ribbon is outlined: what is behind a mark here is a
/// picture and not a background, so a low note's own colour — which is the dark
/// end of the lattice's pitch ramp — has nothing to stand out against wherever
/// the spectrum under it is loud. The mark keeps its true colour and the
/// backing buys it an edge.
const VOICE_PT: (f32, f32) = (2.0, 4.0);

/// How far past the track's own thickness a note's mark reaches at each end, as
/// a share of it. The turns abut, so the overhang runs into the octaves either
/// side — kept small for that reason, since a long tick would read as the note
/// claiming energy an octave away.
///
/// It is also a term in the FIT: [`Spiral::new`] reserves a whole reach at each
/// end of the range rather than half a track, because the top note's mark is
/// what reaches nearest the pane edge and the pane's painter clips.
const VOICE_OVERHANG: f32 = 0.25;

/// Where a MIDI pitch lands in the pane.
///
/// Two numbers do all of it: `angle = 2π · frac(midi / 12)` and
/// `radius = r0 + (midi - min_midi) / 12 · dr`. Everything drawn here speaks
/// MIDI pitch and a radial offset, and only this knows where that is on screen.
#[derive(Clone, Copy)]
struct Spiral {
    centre: egui::Pos2,
    /// Radius of the track's CENTRE at `min_midi`. Strictly positive — see
    /// [`INNER_HOLE`].
    r0: f32,
    /// Radius gained per octave, which is also the track's full thickness: the
    /// turns abut, so the drawn annulus is continuous and a pitch class reads
    /// as one unbroken ray through every octave it sounds in.
    dr: f32,
    min_midi: f32,
    max_midi: f32,
}

impl Spiral {
    /// Fit the spiral to `rect` over the Analyzer's pitch range.
    fn new(rect: egui::Rect, cfg: &crate::SpectrumConfig) -> Spiral {
        let min_midi = cfg.low_midi;
        // Never trust the pair to be ordered, exactly as the Spectral pane does
        // not: a zero or negative span divides by zero below and paints NaN
        // geometry, which egui panics on — and a panic here takes the editor,
        // and with it the host, down. The range bar cannot produce one; a
        // hand-edited state blob can.
        let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
        let r_out = (rect.width().min(rect.height()) * 0.5 - MARGIN_PT).max(1.0);
        let hole = r_out * INNER_HOLE;
        let octaves = (max_midi - min_midi) / 12.0;
        // A note's full REACH is reserved at each end, not half a track: the
        // mark a sounding note draws is the one thing here that stands out past
        // the track it sits on, so a fit that reserved only the track puts the
        // top note's mark over the pane edge, where the clipping painter cuts
        // it off. `a_notes_mark_stays_inside_the_pane` is what holds this.
        //
        // So `dr * octaves + 2 * reach = r_out - hole` with
        // `reach = dr / 2 * (1 + VOICE_OVERHANG)`, which solves for `dr`
        // directly. Solving rather than clamping afterwards is what makes
        // `r0 > 0` a property of the fit rather than a case to catch, and it
        // holds the hole at a constant SHARE of the disc, so zooming the pitch
        // range in does not eat it.
        let dr = (r_out - hole) / (octaves + 1.0 + VOICE_OVERHANG);
        let r0 = hole + dr * 0.5 * (1.0 + VOICE_OVERHANG);
        Spiral { centre: rect.center(), r0, dr, min_midi, max_midi }
    }

    /// The unit vector pointing out of the centre at `midi`'s pitch class. C is
    /// straight up and pitch ascends clockwise, which is the chroma circle as
    /// everyone draws it.
    fn ray(&self, midi: f32) -> egui::Vec2 {
        let turn = (midi / 12.0).rem_euclid(1.0);
        let (sin, cos) = (std::f32::consts::TAU * turn).sin_cos();
        egui::vec2(sin, -cos)
    }

    fn radius(&self, midi: f32) -> f32 {
        self.r0 + (midi - self.min_midi) / 12.0 * self.dr
    }

    /// The screen point at `midi`, `offset` points out from the track's centre.
    fn at(&self, midi: f32, offset: f32) -> egui::Pos2 {
        self.centre + self.ray(midi) * (self.radius(midi) + offset)
    }

    /// Half the track's thickness — the offset of either of its edges.
    fn half(&self) -> f32 {
        self.dr * 0.5
    }

    /// How far a sounding note's mark stands out from the track's centre. Wider
    /// than [`half`](Self::half) by [`VOICE_OVERHANG`], and the fit reserves it
    /// at both ends of the range.
    fn reach(&self) -> f32 {
        self.half() * (1.0 + VOICE_OVERHANG)
    }

    /// The disc's bounds, inner and outer radius: everything this pane draws
    /// lies between these two, marks included.
    ///
    /// The track itself stops fractionally inside them — the fit reserves a
    /// note's whole reach at each end — so this is what the pane OCCUPIES
    /// rather than where the spectrum is painted.
    fn bounds(&self) -> (f32, f32) {
        (self.radius(self.min_midi) - self.reach(), self.radius(self.max_midi) + self.reach())
    }
}

/// The analyzer's current spectrum in polar coordinates, with the sounding
/// voices marked on it.
///
/// A pointer over this pane changes nothing on it, matching the Spectral pane:
/// the zoom is the Analyzer section's pitch range, which both panes read.
pub(crate) fn spiral_pane(ui: &mut egui::Ui, state: &mut SharedState, now: f64) {
    let cfg = state.spectrum_config;
    let (rect, _response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    if rect.width() < 10.0 || rect.height() < 10.0 {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, crate::theme::well());

    let spiral = Spiral::new(rect, &cfg);
    painter.add(egui::Shape::mesh(strip(&spiral, state, &cfg, now)));
    rays(&painter, &spiral);
    voices(&painter, &spiral, state, now);
}

/// The spectrum itself: one triangle strip winding out from the centre, two
/// vertices per step (the track's two edges), coloured by the analyzer's own
/// gradient through its own loudness mapping.
///
/// Drawn even with no audio flowing, as a black annulus. That is not a spare
/// case to skip — it is the pane saying where its picture IS, the same job the
/// Spectral pane's black bed does under its heatmap, and without it a silent
/// pane is an empty rectangle with no spiral in it to start reading.
fn strip(
    spiral: &Spiral,
    state: &SharedState,
    cfg: &crate::SpectrumConfig,
    now: f64,
) -> egui::Mesh {
    use harmonigraph_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    let levels = state.spectrum.display(now);
    let span = spiral.max_midi - spiral.min_midi;

    // One step per SEGMENT_PT of arc, capped at the analyzer's own resolution:
    // past one step per bucket the extra vertices carry no new reading, and the
    // full range is 3828 buckets, so the cap is what holds the mesh at a few
    // thousand vertices however large the pane is.
    let (r_in, r_out) = spiral.bounds();
    let arc = std::f32::consts::PI * (r_in + r_out) * span / 12.0;
    let cap = (span * BINS_PER_SEMITONE as f32).max(MIN_STEPS);
    let steps = (arc / SEGMENT_PT).clamp(MIN_STEPS, cap) as usize;

    // The run of buckets one step covers, read by the heatmap's own power mean
    // and not by a MAX — the same read the Spectral pane's curve takes, and for
    // the same reason: the largest of N buckets grows with N, so a max would
    // lift this pane's noise floor as the pitch range was widened.
    let half_step = span / (2.0 * steps as f32);
    let level = |midi: f32| {
        let Some(levels) = levels else { return 0.0 };
        let bucket = |m: f32| {
            (((m - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32) as isize)
                .clamp(0, levels.len() as isize - 1) as usize
        };
        let (b0, b1) = (bucket(midi - half_step), bucket(midi + half_step));
        power_mean(&levels[b0..=b1.max(b0)])
    };

    let mut mesh = egui::Mesh::default();
    for i in 0..=steps {
        let midi = spiral.min_midi + span * i as f32 / steps as f32;
        // Opaque, and untinted by anything of this pane's: the gradient's dark
        // end is black, so silence recedes into the disc rather than letting
        // the pane's own `well` through in rings between the turns.
        let color = cell_color(cfg.spectrogram_gradient, loudness(cfg, level(midi), midi));
        mesh.colored_vertex(spiral.at(midi, -spiral.half()), color);
        mesh.colored_vertex(spiral.at(midi, spiral.half()), color);
        if i > 0 {
            let (a, b) = (2 * (i as u32 - 1), 2 * i as u32);
            mesh.add_triangle(a, a + 1, b);
            mesh.add_triangle(a + 1, b + 1, b);
        }
    }
    mesh
}

/// The twelve pitch-class rays, over the spectrum rather than under it.
///
/// Under is where the Spectral pane puts its frequency rulings, and it is not
/// available here: that pane's curve fills only as far as it is loud, where
/// this strip covers the whole annulus at every level, so a ray beneath it is a
/// ray nobody sees. Over and quiet is the same trade its now-line already
/// makes — a hairline across a picture costs the picture nothing, and the
/// picture is unreadable without knowing which way round it goes.
fn rays(painter: &egui::Painter, spiral: &Spiral) {
    let (r_in, r_out) = spiral.bounds();
    for pc in 0..12 {
        let fade = if pc == 0 { RAY_FADE.0 } else { RAY_FADE.1 };
        let dir = spiral.ray(pc as f32);
        painter.line_segment(
            [spiral.centre + dir * r_in, spiral.centre + dir * r_out],
            egui::Stroke::new(1.0, crate::theme::hairline().gamma_multiply(fade)),
        );
    }
}

/// The sounding MIDI notes, each a radial tick across the track at its pitch.
///
/// Radial rather than a dot because what a mark has to say is WHERE ALONG the
/// spiral the note is, and the spiral runs across it: a tick crossing the track
/// names one pitch on it, and the partials that agree with the note line up
/// under the tick while the ones that do not sit visibly to one side.
///
/// Coloured off the lattice's own pitch ramp, through the roll's
/// [`note_color`], so a note is the same colour here, on the piano roll, and at
/// the node it lit up.
fn voices(painter: &egui::Painter, spiral: &Spiral, state: &SharedState, now: f64) {
    let mut sounding: Vec<&harmonigraph_core::Voice> = state.tracker.voices().collect();
    // Fading marks accumulate where they overlap, so the paint order is part of
    // the picture. The tracker's own order is stable but is not this one: pitch
    // decides which mark is on top, matching the Spectral pane's voice bands.
    sounding.sort_unstable_by(|a, b| {
        a.pitch.total_cmp(&b.pitch).then(a.channel.cmp(&b.channel)).then(a.note.cmp(&b.note))
    });
    // One envelope for the whole pane, as every other caller takes it: it is a
    // property of the view and the frame, and rebuilding it per voice would
    // read as if it could vary between them.
    let env = state.view.envelope(&state.frame_params);
    let reach = spiral.reach();
    for voice in sounding {
        let strength = voice.activation(now, &env);
        if strength <= 0.0 || voice.pitch < spiral.min_midi || voice.pitch > spiral.max_midi {
            continue;
        }
        let ends = [spiral.at(voice.pitch, -reach), spiral.at(voice.pitch, reach)];
        let backing = Color32::BLACK.gamma_multiply(0.75 * strength);
        let color = note_color(state, voice.pitch, strength);
        painter.line_segment(ends, egui::Stroke::new(VOICE_PT.1, backing));
        painter.line_segment(ends, egui::Stroke::new(VOICE_PT.0, color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::probe::{fresh, painted_into};
    use crate::SpectrumConfig;
    use harmonigraph_core::NoteEvent;

    /// A square pane at an offset origin, so a mistake that assumes the rect
    /// starts at zero shows up.
    const PANE: egui::Rect =
        egui::Rect { min: egui::pos2(20.0, 30.0), max: egui::pos2(420.0, 430.0) };

    const SCREEN: egui::Vec2 = egui::vec2(500.0, 500.0);

    fn spiral(low: f32, high: f32) -> Spiral {
        let cfg = SpectrumConfig { low_midi: low, high_midi: high, ..Default::default() };
        Spiral::new(PANE, &cfg)
    }

    fn painted(state: &mut SharedState, now: f64) -> Vec<egui::Shape> {
        painted_into(SCREEN, PANE, |ui| spiral_pane(ui, state, now))
            .shapes
            .into_iter()
            .map(|s| s.shape)
            .collect()
    }

    /// The premise of the whole pane: an octave is one whole turn, so two
    /// pitches an octave apart are the SAME DIRECTION out of the centre and
    /// differ only in radius.
    ///
    /// Worth pinning rather than reading off the formula, because getting it
    /// wrong costs nothing visible: any winding at all draws a plausible
    /// spiral, and only the harmonics failing to line up says the turn is not
    /// an octave — which is the one thing nobody can check by eye on a picture
    /// they have never seen right.
    #[test]
    fn an_octave_is_one_turn_of_the_spiral() {
        let s = spiral(36.0, 96.0);
        for midi in [36.0f32, 41.5, 47.0, 60.0, 83.25] {
            let (near, far) = (s.at(midi, 0.0) - s.centre, s.at(midi + 12.0, 0.0) - s.centre);
            // The SINE of the angle between them: the cross product of two
            // screen offsets carries their lengths, which grow with the turn,
            // so an unnormalized one measures the radius as much as the angle.
            let sin = (near.x * far.y - near.y * far.x) / (near.length() * far.length());
            assert!(
                sin.abs() < 1e-5,
                "midi {midi} and {} are {sin} off one ray",
                midi + 12.0,
            );
            assert!(far.length() > near.length(), "the octave above must be further out");
            assert!(
                (far.length() - near.length() - s.dr).abs() < 1e-3,
                "an octave must be exactly one `dr` of radius",
            );
        }
    }

    /// C is straight up and pitch ascends clockwise — the chroma circle the
    /// way it is always drawn, which is what makes the ray a reader can find
    /// (the strong one in [`rays`]) the one they expect it to be.
    #[test]
    fn c_is_at_the_top_and_pitch_ascends_clockwise() {
        let s = spiral(36.0, 96.0);
        let up = s.ray(60.0);
        assert!(up.x.abs() < 1e-6 && up.y < 0.0, "C points up, not {up:?}");
        // A quarter turn on is a minor third, and it is to the RIGHT.
        let third = s.ray(63.0);
        assert!(third.y.abs() < 1e-6 && third.x > 0.0, "D# points right, not {third:?}");
    }

    /// The hole in the middle is real and the disc stays inside the pane.
    ///
    /// Both ends of the same algebra, and the reason it is algebra rather than
    /// a clamp: radius is linear in octave, so a spiral drawn from radius zero
    /// gives its innermost turn no circumference to spell twelve pitch classes
    /// across, and one drawn to the pane's own half-width loses the outermost
    /// turn's outer half over the edge.
    #[test]
    fn the_disc_keeps_its_hole_and_stays_in_the_pane() {
        // Across the range the Analyzer offers, plus the extremes of the bar:
        // the fit is solved rather than clamped, so it has to hold at a tenth
        // of an octave as well as at ten.
        for (low, high) in [(36.0, 96.0), (15.5, 135.1), (60.0, 61.0), (60.0, 72.0)] {
            let s = spiral(low, high);
            let (r_in, r_out) = s.bounds();
            let half_pane = PANE.width().min(PANE.height()) * 0.5;
            assert!(r_in > 0.0, "{low}..{high} drew the innermost turn through the centre");
            assert!(
                r_out <= half_pane,
                "{low}..{high} drew out to {r_out}, past the pane's {half_pane}",
            );
            // And the hole is a share of the disc rather than a fixed inset,
            // so zooming in does not eat it.
            assert!((r_in / r_out - INNER_HOLE).abs() < 1e-3, "{low}..{high}: hole drifted");
        }
    }

    /// A tone lands on its own pitch class, and the disc is not one flat
    /// colour.
    ///
    /// The only test here that runs the strip against a NON-EMPTY spectrum, and
    /// the reason it has to exist: every other fixture builds `fresh()`, whose
    /// analyzer has no samples, so `display` answers `None` and the level
    /// closure returns 0.0 at every step. Without this, the bucket lookup, the
    /// bucket-run slice, `power_mean`, `loudness` and `cell_color` are reached
    /// by nothing at all, and the picture's whole colour path reads as covered
    /// while being untested.
    ///
    /// Two claims, because either alone passes on a broken pane: the peak is at
    /// the right PITCH (a strip built in the wrong order fails this) and it is
    /// in the right DIRECTION on screen (an angle map that is not
    /// `frac(midi/12)` fails this, and would otherwise draw a perfectly
    /// plausible spiral).
    ///
    /// What it does not pin is the ORDER of the power mean — a single-bucket
    /// read would still put the peak here. That choice is argued in `strip`'s
    /// own comment and is shared with the Spectral pane, whose
    /// `power_mean` tests hold it.
    #[test]
    fn a_tone_lands_on_its_own_pitch_class() {
        let mut state = fresh();
        state.spectrum_config.low_midi = 60.0;
        state.spectrum_config.high_midi = 96.0;
        // 1 kHz, the tilt's own pivot, so the slope takes nothing off the level
        // and the peak is the tone's alone.
        let sr = 48_000.0;
        let samples: Vec<f32> =
            (0..48_000).map(|i| (std::f32::consts::TAU * 1_000.0 * i as f32 / sr).sin()).collect();
        let cfg = state.spectrum_config;
        state.spectrum.push_samples(&samples, 1, sr, 1.0, &cfg);
        let tone_midi = 69.0 + 12.0 * (1_000.0f32 / 440.0).log2();

        let meshes: Vec<egui::Mesh> = painted(&mut state, 1.0)
            .into_iter()
            .filter_map(|s| match s {
                egui::Shape::Mesh(m) => Some((*m).clone()),
                _ => None,
            })
            .collect();
        let mesh = meshes.first().expect("the strip is a mesh");

        let lum = |c: egui::Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        let shades: std::collections::HashSet<[u8; 4]> =
            mesh.vertices.iter().map(|v| v.color.to_array()).collect();
        assert!(
            shades.len() > 4,
            "the strip is {} shade(s) — the level path never ran",
            shades.len(),
        );

        let (peak, _) = mesh
            .vertices
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| lum(v.color))
            .expect("the strip has vertices");
        // Two vertices per step, and the steps run min_midi..max_midi in order.
        let steps = mesh.vertices.len() / 2 - 1;
        let s = Spiral::new(PANE, &cfg);
        let at_peak = s.min_midi + (s.max_midi - s.min_midi) * (peak / 2) as f32 / steps as f32;
        // Half a semitone, and it is the peak's PLATEAU that sets the floor
        // under this rather than the analyzer: a colour is quantized to 8 bits
        // a channel, so the vertices either side of a peak share one exact
        // colour and which of them comes back as the maximum is arbitrary
        // within the run. Still two orders tighter than any wrong pitch
        // mapping, the axis being three octaves wide.
        const NEAR: f32 = 0.5;
        assert!(
            (at_peak - tone_midi).abs() < NEAR,
            "the brightest bucket is at MIDI {at_peak}, not the tone's {tone_midi}",
        );

        // ...and it is drawn on that pitch's own ray, which is the claim the
        // pitch alone cannot make. The same tolerance as an angle: a semitone
        // is a twelfth of a turn, so half of one is 15 degrees.
        //
        // The expected direction is written out from the geometry rather than
        // taken from [`Spiral::ray`]. Asking `ray` where the tone should be and
        // then checking it is drawn there is a comparison of `ray` with itself,
        // which passes at any winding at all — this test read `ray` on both
        // sides and went on passing with the turn set to a THIRTEENTH of an
        // octave.
        let angle = std::f32::consts::TAU * (tone_midi / 12.0).fract();
        let want = egui::vec2(angle.sin(), -angle.cos());
        let got = (mesh.vertices[peak].pos - s.centre).normalized();
        let sin = want.x * got.y - want.y * got.x;
        assert!(
            sin.abs() < (std::f32::consts::TAU * NEAR / 12.0).sin() && want.dot(got) > 0.0,
            "the peak is {} degrees off the tone's ray",
            sin.asin().to_degrees(),
        );
    }

    /// A sounding note's mark is inside the pane, at the top of the range and
    /// at the bottom of it.
    ///
    /// The mark is the only painted thing that reaches past the track it sits
    /// on, so it is the only one whose extent the disc's own fit does not
    /// already answer for — and the pane paints through a clipping painter, so
    /// what overruns is cut off rather than merely tight. Landscape as well as
    /// square because the fit is driven by the SHORT side: a 16:9 frame is
    /// where the outermost turn sits closest to an edge.
    #[test]
    fn a_notes_mark_stays_inside_the_pane() {
        let frames = [
            ("square", egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 900.0))),
            ("1080p", egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1920.0, 1080.0))),
            ("docked", PANE),
        ];
        for (name, rect) in frames {
            for (low, high) in [(60.0f32, 84.0f32), (36.0, 96.0), (15.5, 135.1)] {
                let cfg = SpectrumConfig { low_midi: low, high_midi: high, ..Default::default() };
                let s = Spiral::new(rect, &cfg);
                for pitch in [s.min_midi, s.max_midi] {
                    for end in [s.at(pitch, -s.reach()), s.at(pitch, s.reach())] {
                        assert!(
                            rect.contains(end),
                            "{name} {low}..{high}: a mark at {pitch} reaches {end:?}, \
                             outside the pane {rect:?}",
                        );
                    }
                }
            }
        }
    }

    /// A pitch range that a bar cannot produce but a hand-edited blob can —
    /// collapsed, or the wrong way round — draws finite geometry.
    ///
    /// The Spectral pane guards the same pair for the same reason: NaN
    /// geometry is not a wrong picture, it is a panic inside egui's
    /// tessellator, and a panic in the editor takes the host down with it.
    #[test]
    fn a_collapsed_pitch_range_paints_no_nan() {
        for (low, high) in [(60.0f32, 60.0f32), (96.0, 36.0), (60.0, 59.0)] {
            let mut state = fresh();
            state.spectrum_config.low_midi = low;
            state.spectrum_config.high_midi = high;
            state.tracker.handle_event(NoteEvent::on(0.0, 0, 69, 1.0));
            let shapes = painted(&mut state, 0.1);
            assert!(!shapes.is_empty(), "{low}..{high} drew nothing at all");
            for shape in &shapes {
                let bounds = shape.visual_bounding_rect();
                assert!(
                    !bounds.any_nan(),
                    "{low}..{high} painted NaN geometry: {shape:?}",
                );
            }
        }
    }

    /// A sounding note is marked, and only where the pane can show it: a voice
    /// outside the displayed pitch range has no place on the disc, and drawing
    /// it at the nearest end would put a mark on a pitch nothing is playing.
    #[test]
    fn only_the_notes_the_range_reaches_are_marked() {
        let marks = |note: u8| {
            let mut state = fresh();
            state.spectrum_config.low_midi = 48.0;
            state.spectrum_config.high_midi = 84.0;
            state.tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
            painted(&mut state, 0.1)
                .iter()
                .filter(|s| matches!(s, egui::Shape::LineSegment { stroke, .. }
                    if stroke.width == VOICE_PT.0))
                .count()
        };
        assert_eq!(marks(60), 1, "a note inside the range is marked once");
        assert_eq!(marks(24), 0, "a note below the range is not marked");
        assert_eq!(marks(120), 0, "a note above the range is not marked");
    }
}
