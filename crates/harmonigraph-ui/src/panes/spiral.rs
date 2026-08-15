//! The Spiral pane: the analyzer's current spectrum wound onto a chroma
//! spiral — one turn per octave — with the sounding MIDI notes dotted on it and
//! named around it.
//!
//! Three things are drawn over the spectrum, and each answers one question a
//! wound picture raises that a straight one does not. The twelve [`rays`] say
//! which way round the circle you are looking. The [`seam`] between turns says
//! how many octaves you are looking across, which a continuous spiral otherwise
//! leaves to be counted by eye off a curve with no ticks on it. And the
//! [`names`] on the rim say what the lit rays are CALLED, since a direction is
//! all the picture itself can say about a pitch class.
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
/// leaves the innermost turn about a fifth of the outermost one's
/// circumference — measured centre to centre, which is the room a turn
/// actually has; edge to edge it reads as a sixth, and saying which is what
/// keeps the figure checkable. Coarse either way, but still twelve directions
/// rather than a blot.
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

/// How faint the seam between one turn and the next is drawn against
/// [`theme::hairline`](crate::theme::hairline), and how wide.
///
/// Fainter than either ray, and it has to be: there is one seam per octave
/// boundary and each is a whole turn long, so what is a hairline on a ray is
/// several times the ink here. Enough to count the turns by and not enough to
/// read as part of the spectrum.
const SEAM: (f32, f32) = (0.28, 1.0);

/// Points of arc one segment of the seam covers — coarser than the strip's
/// [`SEGMENT_PT`], and it can be. What a straight segment costs a curve is its
/// sagitta, `s²/8r`, which at this length is hundredths of a point anywhere on
/// this disc; the strip is cut fine because its COLOUR changes along it, where
/// the seam is one hairline all the way round. Cut as fine as the strip it
/// would roughly double what the pane hands the tessellator, to draw the same
/// curve.
const SEAM_SEGMENT_PT: f32 = 6.0;

/// A sounding note's dot: its radius as a share of half the track, and the
/// fewest points of radius it is ever drawn at.
///
/// A SHARE, so the mark is the same size against the turns whatever the pitch
/// range and the pane are doing — a dot fixed in points is a blob between two
/// turns at the ten-octave range and a speck on one at the two-octave floor,
/// and it is which turn a note is on that the dot is there to say. Under 1.0
/// because a dot filling its track leaves nothing of the track showing either
/// side of it, which is what makes it read as sitting on one.
///
/// The floor keeps a dot legible on the thinnest tracks the pane can draw. It
/// is a length in POINTS and so carries no term for the track at all, which
/// makes it the one term here that can spend the share's own argument: at the
/// Analyzer's full range it takes the dot over from the share below a pane of
/// about 277 points, and by 200 the dot is 98% of its track.
///
/// [`Spiral::dot`] clamps it to half the track for that reason. What the clamp
/// gives up is the "under 1.0" above — a dot on a small enough pane fills the
/// turn it sits on — and what it keeps is the half of the argument that cannot
/// be given up, that the dot does not cross into the turns either side to say
/// which turn it is on. `a_notes_dot_sits_inside_its_own_turn` is what holds
/// this.
const DOT: (f32, f32) = (0.7, 3.0);

/// How much of the dot's radius is the dark backing ringing it.
///
/// A backing rather than a heavier dot, for the same reason the axis labels are
/// haloed and a note ribbon is outlined: what is behind a mark here is a
/// picture and not a background, so a low note's own colour — which is the dark
/// end of the lattice's pitch ramp — has nothing to stand out against wherever
/// the spectrum under it is loud. The dot keeps its true colour and the backing
/// buys it an edge.
///
/// A width rather than a share, being an edge rather than a feature: it is what
/// parts the note's colour from whatever is under it, and that job is the same
/// size at every zoom.
const DOT_RING_PT: f32 = 1.25;

/// How far past the track's own thickness a note's mark may reach, as a share
/// of half the track. The turns abut, so the overhang runs into the octaves
/// either side — kept small for that reason, since a mark spilling far would
/// read as the note claiming energy an octave away.
///
/// It is also a term in the FIT: [`Spiral::new`] reserves a whole reach at each
/// end of the range rather than half a track, because the top note's dot is
/// what reaches nearest the disc's edge, and every name outside it is placed
/// off that edge.
const VOICE_OVERHANG: f32 = 0.25;

/// Points of type a sounding note's name is set in on the rim, on a pane large
/// enough to give the names their whole band.
///
/// The size the analyzer's names are dialled at (`names::LABEL_PT`), quoted
/// rather than picked afresh: these are the same names, drawn by the same
/// [`draw_stacked_name`](crate::marks::draw_stacked_name).
///
/// What each pane does to that size from there is its own, and the two differ
/// because the names are doing different jobs. The analyzer's are written OVER
/// ribbons, so they follow the picture — its `REFERENCE_PITCH_LEN` and the
/// pitch zoom both. These stand outside the picture, like an axis label, and
/// the only thing that can crowd them is the band they are set in: they take
/// their dialled size wherever it fits and scale with the band where it does
/// not, which is on any pane whose short side is under about 395 points —
/// [`NAME_BAND_SHARE`] against the whole of [`NAME_BAND_PT`] is where that
/// number comes from. A dock split two ways sits above it and a dock split
/// three ways below.
const NAME_PT: f32 = 12.35;

/// The band reserved OUTSIDE the disc for those names: the air between the
/// rim and a name's ink, then the room the name itself reaches past it.
///
/// Reserved WHATEVER is sounding, so the disc is one size and does not breathe
/// as notes come and go — a picture that resized itself on every note would be
/// unreadable, and the band is empty most of the time, which is what silence
/// should look like.
///
/// The reach is measured along the ray, and a name is set square to the screen
/// whatever ray it sits on, so what has to fit is the name's box across its own
/// diagonal — the letter's ink flush at the rim plus the mark column trailing
/// it (see [`NameLead::Letter`](crate::marks::NameLead::Letter)).
const NAME_BAND_PT: (f32, f32) = (3.0, 26.0);

/// The most of the disc's radius that band may take. Where it binds, the whole
/// band scales — type, air and reach together — so the names stay inside it
/// rather than being clipped off a pane too small to hold them at full size.
const NAME_BAND_SHARE: f32 = 0.15;

/// Cents of pitch class inside which two sounding notes get ONE name.
///
/// An octave is a turn, so C4 and C5 leave the disc on the same ray and their
/// names are the same word in the same place — printed twice it is just heavier
/// ink, and printed at a bend's worth of angle apart it is a smear. The grain
/// matches the roll's own thinning (`names::LANE_CENTS`): fine enough to keep a
/// comma's two spellings apart, coarse enough that vibrato does not split one
/// note into two names.
const NAME_GRAIN_CENTS: f32 = 10.0;

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
    /// Radius kept outside the disc for the sounding notes' names — see
    /// [`NAME_BAND_PT`], whose full width this is on any pane with room for it.
    band: f32,
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
        // The names' band comes off the radius first, so everything below fits
        // the DISC and the rim it is fitted to is the rim they are placed off.
        let band = (NAME_BAND_PT.0 + NAME_BAND_PT.1).min(r_out * NAME_BAND_SHARE);
        let r_out = r_out - band;
        let hole = r_out * INNER_HOLE;
        let octaves = (max_midi - min_midi) / 12.0;
        // A note's full REACH is reserved at each end, not half a track: the
        // dot a sounding note draws is the one thing on the disc that stands
        // out past the track it sits on, so a fit that reserved only the track
        // puts the top note's dot over the disc's edge — and everything outside
        // that edge is the names' band.
        // `a_notes_dot_stays_inside_the_pane` is what holds this.
        //
        // So `dr * octaves + 2 * reach = r_out - hole` with
        // `reach = dr / 2 * (1 + VOICE_OVERHANG)`, which solves for `dr`
        // directly. Solving rather than clamping afterwards is what makes
        // `r0 > 0` a property of the fit rather than a case to catch, and it
        // holds the hole at a constant SHARE of the disc, so zooming the pitch
        // range in does not eat it. The narrowest disc this has to fit is two
        // octaves, which is what `PITCH_RANGE_MIN_SPAN` holds the range open
        // to — the widest a single turn ever gets.
        let dr = (r_out - hole) / (octaves + 1.0 + VOICE_OVERHANG);
        let r0 = hole + dr * 0.5 * (1.0 + VOICE_OVERHANG);
        Spiral { centre: rect.center(), r0, dr, band, min_midi, max_midi }
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

    /// How far a sounding note's mark may stand out from the track's centre.
    /// Wider than [`half`](Self::half) by [`VOICE_OVERHANG`], and the fit
    /// reserves it at both ends of the range.
    fn reach(&self) -> f32 {
        self.half() * (1.0 + VOICE_OVERHANG)
    }

    /// The dot a sounding note is marked with: its whole radius, backing
    /// included, and the coloured fill inside that. See [`DOT`].
    ///
    /// Capped at HALF the track, which is the bound that is not a matter of
    /// taste: a dot past it crosses into the octaves either side, and which
    /// turn the note is on is what the dot is drawn to say. It answers the fit
    /// as well, being the tighter of the two — [`reach`](Self::reach) is the
    /// same half a whole [`VOICE_OVERHANG`] wider, so a dot held inside its own
    /// turn is inside the disc without asking. What the cap binds on is the
    /// floor in [`DOT`], which is a length in points and would otherwise draw a
    /// dot a quarter wider than the track on a small enough pane.
    fn dot(&self) -> (f32, f32) {
        let backed = (self.half() * DOT.0).max(DOT.1).min(self.half());
        // The fill is never under half the dot, so a track thin enough to
        // shrink the mark below two rings' worth still has a fill to carry the
        // note's colour — which is the half of the pair that says WHICH note
        // this is.
        (backed, (backed - DOT_RING_PT).max(backed * 0.5))
    }

    /// How much of its dialled size the rim names are drawn at: the whole band
    /// scales together, so the air in front of a name and the room behind it
    /// keep their proportions to the type on a pane too small for the full one.
    fn name_scale(&self) -> f32 {
        self.band / (NAME_BAND_PT.0 + NAME_BAND_PT.1)
    }

    /// Where a name for `midi` is anchored: on its pitch class's ray, the
    /// band's own air outside the disc, with the name reaching outward from
    /// there.
    fn rim(&self, midi: f32) -> egui::Pos2 {
        self.centre + self.ray(midi) * (self.bounds().1 + NAME_BAND_PT.0 * self.name_scale())
    }

    /// The disc's bounds, inner and outer radius: everything this pane draws of
    /// the PICTURE lies between these two, dots included. The names do not —
    /// they are placed off the outer one, in the band beyond it.
    ///
    /// The track itself stops fractionally inside them — the fit reserves a
    /// note's whole reach at each end — so this is what the disc OCCUPIES
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
    seam(&painter, &spiral);
    rays(&painter, &spiral);
    let lit = sounding(&spiral, state, now);
    dots(&painter, &spiral, state, &lit);
    // The names last, and outside the disc, so nothing in the picture is over
    // them and they are over nothing in it.
    let mut labels = crate::text::TextBatch::default();
    names(&painter, &spiral, state, &lit, &mut labels);
    labels.flush(
        &painter,
        rect,
        state,
        crate::text::SPIRAL_NAMES,
        // Nothing here scrolls: a name sits on its note's ray and stays there
        // for as long as the note sounds, so the filter has no travel to
        // follow and takes the axis every still surface takes.
        harmonigraph_render::SlideAxis::Across,
    );
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
    let cap = (span * BINS_PER_SEMITONE as f32).max(MIN_STEPS);
    let steps = (arc_len(r_in, r_out, span / 12.0) / SEGMENT_PT).clamp(MIN_STEPS, cap) as usize;

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

/// How long a run of spiral between two radii is, over `turns` turns: the mean
/// circumference times the number of turns. What decides how many straight
/// segments a curve here is cut into, and so whether it reads as a curve.
fn arc_len(r_a: f32, r_b: f32, turns: f32) -> f32 {
    std::f32::consts::PI * (r_a + r_b) * turns
}

/// The seam between one turn of the spiral and the next — what an octave is
/// counted by.
///
/// ONE polyline, not a ring per octave, because the boundary between two turns
/// IS a spiral. The turns abut, so along any ray the octave above starts
/// exactly half a track out from the note below it, and tracing that point from
/// `min_midi` up to an octave short of `max_midi` passes through every boundary
/// in the disc once: a run of `octaves - 1` turns that crosses each pitch
/// class's ray once per boundary. A ring of constant radius would sit on the
/// seam at one pitch class and be a whole track's width off it by the time it
/// came back round — the same drift that makes the spectrum a strip rather than
/// a stack of annuli.
///
/// Over the spectrum, with the rays and for the rays' reason: the strip covers
/// the whole disc at every level, so a seam beneath it is a seam nobody sees.
fn seam(painter: &egui::Painter, spiral: &Spiral) {
    let span = spiral.max_midi - 12.0 - spiral.min_midi;
    // A range under one octave has no boundary between turns to draw. The
    // analyzer's own floor is two (`PITCH_RANGE_MIN_SPAN`, which `Spiral::new`
    // holds the range open to), so this never fires today — it is here so that
    // a narrower floor would cost the seam rather than wind one backwards
    // through the middle of the disc.
    if span <= 0.0 {
        return;
    }
    let (r_a, r_b) = (spiral.radius(spiral.min_midi), spiral.radius(spiral.max_midi - 12.0));
    let steps = (arc_len(r_a, r_b, span / 12.0) / SEAM_SEGMENT_PT).max(MIN_STEPS) as usize;
    let points = (0..=steps)
        .map(|i| {
            let midi = spiral.min_midi + span * i as f32 / steps as f32;
            spiral.at(midi, spiral.half())
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(SEAM.1, crate::theme::hairline().gamma_multiply(SEAM.0)),
    ));
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

/// A voice this pane has somewhere to draw: its pitch, and how lit the
/// envelope has it.
///
/// Read once and handed to both the dot and the name, so the two cannot
/// disagree about which notes are sounding or how far each has faded.
#[derive(Clone, Copy, Debug)]
struct Sounding {
    pitch: f32,
    strength: f32,
}

/// The voices the disc can show, lowest first.
///
/// Fading marks accumulate where they overlap, so the paint order is part of
/// the picture. The tracker's own order is stable but is not this one: pitch
/// decides which mark is on top, matching the Spectral pane's voice bands.
///
/// A voice outside the displayed pitch range is dropped rather than pinned to
/// the nearest end — the disc has no place for it, and drawing it at the rim
/// would put a mark on a pitch nothing is playing.
fn sounding(spiral: &Spiral, state: &SharedState, now: f64) -> Vec<Sounding> {
    let mut voices: Vec<&harmonigraph_core::Voice> = state.tracker.voices().collect();
    voices.sort_unstable_by(|a, b| {
        a.pitch.total_cmp(&b.pitch).then(a.channel.cmp(&b.channel)).then(a.note.cmp(&b.note))
    });
    // One envelope for the whole pane, as every other caller takes it: it is a
    // property of the view and the frame, and rebuilding it per voice would
    // read as if it could vary between them.
    let env = state.view.envelope(&state.frame_params);
    voices
        .into_iter()
        .filter(|v| v.pitch >= spiral.min_midi && v.pitch <= spiral.max_midi)
        .map(|v| Sounding { pitch: v.pitch, strength: v.activation(now, &env) })
        .filter(|v| v.strength > 0.0)
        .collect()
}

/// The sounding MIDI notes, each a dot on the track at its pitch.
///
/// A dot rather than a tick across the track: what the mark has to say first is
/// WHICH turn the note is on, and a dot says that by sitting on one, where a
/// tick reaching into the octaves either side has to be read past its own ends
/// to place it. Where the note sits ALONG the spiral is still the dot's centre,
/// so the partials that agree with it still line up on its ray and the ones
/// that do not sit visibly to one side.
///
/// Coloured off the lattice's own pitch ramp, through the roll's
/// [`note_color`], so a note is the same colour here, on the piano roll, and at
/// the node it lit up.
fn dots(painter: &egui::Painter, spiral: &Spiral, state: &SharedState, sounding: &[Sounding]) {
    let (backed, fill) = spiral.dot();
    for voice in sounding {
        let at = spiral.at(voice.pitch, 0.0);
        painter.circle_filled(at, backed, Color32::BLACK.gamma_multiply(0.75 * voice.strength));
        painter.circle_filled(at, fill, note_color(state, voice.pitch, voice.strength));
    }
}

/// What each sounding note is CALLED, written on the rim beyond its own ray.
///
/// Outside the disc rather than on it, which is the one place a name can go
/// here: the strip is a picture at every radius, and a name laid over it buries
/// the spectrum it was supposed to help read. On the rim it costs the picture
/// nothing, and the ray it stands at the end of is the line back to the dot.
///
/// One name per pitch CLASS ([`NAME_GRAIN_CENTS`]), and no octave number on it:
/// a name here says which direction out of the centre you are looking, and the
/// dots say which turns are lit. The spelling is the lattice's own, through the
/// analyzer's [`note_name`](super::spectral::names::note_name), so a note is
/// called the same thing here, on the roll, and at the node it lit up.
///
/// Set in [`theme::text`](crate::theme::text) rather than the note's colour,
/// which the dot already carries: half the pitch ramp is dark, and a name is
/// wanted legible more than it is wanted colour-coded.
fn names(
    painter: &egui::Painter,
    spiral: &Spiral,
    state: &SharedState,
    sounding: &[Sounding],
    batch: &mut crate::text::TextBatch,
) {
    // The wrap at the end of this is load-bearing: a note bent a few cents
    // under C rounds to the TOP of the circle rather than to zero, and the top
    // of the circle is C's own ray.
    let classes = (1200.0 / NAME_GRAIN_CENTS) as i32;
    let class =
        |pitch: f32| (pitch.rem_euclid(12.0) * 100.0 / NAME_GRAIN_CENTS).round() as i32 % classes;
    // One class per name, keeping the loudest of the octaves sounding it: they
    // land on one ray with one spelling, so the only choice left is which of
    // them the fade follows.
    let mut named: Vec<Sounding> = Vec::new();
    for voice in sounding {
        match named.iter_mut().find(|seen| class(seen.pitch) == class(voice.pitch)) {
            Some(seen) if seen.strength < voice.strength => *seen = *voice,
            Some(_) => {}
            None => named.push(*voice),
        }
    }
    if named.is_empty() {
        return;
    }
    let shown = state.shown();
    // `raster` is the rung of the size ladder the type is cut at and `magnify`
    // the rest of what the band asks for, exactly as the roll's names split
    // them — the band is a continuous size and the atlas holds a discrete set.
    let ppp = painter.ctx().pixels_per_point();
    let (raster, magnify) = crate::text::ladder(spiral.name_scale(), NAME_PT, ppp);
    // `draw_stacked_name` sizes everything off the lattice's letter, so the
    // rung crosses back into its terms here — a conversion, not a second snap.
    let scale = NAME_PT * raster / crate::marks::NAME_SIZE;
    for voice in named {
        let name = super::spectral::names::note_name(
            &state.view,
            &shown,
            &state.tuning,
            voice.pitch,
        );
        crate::marks::draw_stacked_name(
            batch,
            painter,
            spiral.rim(voice.pitch),
            name,
            crate::theme::text().gamma_multiply(voice.strength),
            crate::theme::well().gamma_multiply(voice.strength),
            scale,
            magnify,
            // Led by the LETTER's ink, growing out along the ray: the gap a
            // reader sees is between the rim and the ink, and the band outside
            // the disc is sized for a name that starts there. Placed by its box
            // instead, the font's own side bearing would open that gap by
            // however much air the glyph carries.
            crate::marks::NameLead::Letter(spiral.ray(voice.pitch)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::probe::{fresh, painted_into};
    use crate::SpectrumConfig;
    use harmonigraph_core::{NoteEvent, NoteEventKind};

    /// A square pane at an offset origin, so a mistake that assumes the rect
    /// starts at zero shows up.
    const PANE: egui::Rect =
        egui::Rect { min: egui::pos2(20.0, 30.0), max: egui::pos2(420.0, 430.0) };

    const SCREEN: egui::Vec2 = egui::vec2(500.0, 500.0);

    /// The four shapes of frame anything about FITTING has to hold in: the
    /// square a render can be, the 16:9 one it usually is — where the fit's
    /// short side is the height and the disc sits closest to an edge — the
    /// docked pane, and a column narrow enough to cap the name band by its
    /// SHARE of the radius rather than by its point size.
    ///
    /// That last one is a separate frame because `PANE` is not it and misses
    /// by five points: the share binds below a short side of about 395, and a
    /// 400-point pane takes the point size. Every frame above "cramped" draws
    /// its names at exactly their dialled size, so without it nothing here
    /// reaches [`Spiral::name_scale`]'s other answer at all.
    const FRAMES: [(&str, egui::Rect); 4] = [
        ("square", egui::Rect { min: egui::pos2(0.0, 0.0), max: egui::pos2(900.0, 900.0) }),
        ("1080p", egui::Rect { min: egui::pos2(0.0, 0.0), max: egui::pos2(1920.0, 1080.0) }),
        ("docked", PANE),
        ("cramped", egui::Rect { min: egui::pos2(20.0, 30.0), max: egui::pos2(320.0, 330.0) }),
    ];

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

    /// The rim names as a batch that has NOT been flushed, with the spiral they
    /// were placed on.
    ///
    /// Unflushed because a flush hands the glyphs to the GPU and clears exactly
    /// what a test reads — and the batch is the only place a name's letter and
    /// its drawn marks meet, those being cut from two different sheets.
    fn rim_names(
        state: &SharedState,
        rect: egui::Rect,
        now: f64,
    ) -> (crate::text::TextBatch, Spiral) {
        let spiral = Spiral::new(rect, &state.spectrum_config);
        let mut batch = crate::text::TextBatch::default();
        let _ = painted_into(SCREEN, rect, |ui| {
            let lit = sounding(&spiral, state, now);
            names(ui.painter(), &spiral, state, &lit, &mut batch);
        });
        (batch, spiral)
    }

    /// Every box the rim names put ink in: the letters' own, and the quads of
    /// the signs that are drawn rather than set.
    fn name_ink(batch: &crate::text::TextBatch) -> Vec<egui::Rect> {
        batch.pieces().iter().map(|p| p.ink).chain(batch.marks().iter().copied()).collect()
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
        // Both ends of what the range bar can produce, and the middle: the fit
        // is solved rather than clamped, so it has to hold at the two octaves
        // `PITCH_RANGE_MIN_SPAN` floors the range at as well as at the ten the
        // analyzer covers.
        //
        // Two octaves is written as `(60, 72)` and not as some narrower pair
        // that `Spiral::new` widens to it — a fixture the constructor rewrites
        // tests the row it is rewritten INTO, so a pair of them silently
        // becomes one case tested twice.
        for (low, high) in [(36.0, 96.0), (15.5, 135.1), (60.0, 72.0)] {
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

    /// A sounding note's dot is inside the pane, at the top of the range and at
    /// the bottom of it.
    ///
    /// The dot is the only painted thing on the disc that reaches past the
    /// track it sits on, so it is the only one whose extent the disc's own fit
    /// does not already answer for — and the pane paints through a clipping
    /// painter, so what overruns is cut off rather than merely tight. Landscape
    /// as well as square because the fit is driven by the SHORT side: a 16:9
    /// frame is where the outermost turn sits closest to an edge.
    #[test]
    fn a_notes_dot_stays_inside_the_pane() {
        for (name, rect) in FRAMES {
            for (low, high) in [(60.0f32, 84.0f32), (36.0, 96.0), (15.5, 135.1)] {
                let cfg = SpectrumConfig { low_midi: low, high_midi: high, ..Default::default() };
                let s = Spiral::new(rect, &cfg);
                let (backed, _) = s.dot();
                for pitch in [s.min_midi, s.max_midi] {
                    // Round the dot rather than out along the ray alone: it is
                    // a disc, so the point of it nearest the pane edge is not
                    // on the radius at any angle but the four cardinals.
                    for step in 0..12 {
                        let a = std::f32::consts::TAU * step as f32 / 12.0;
                        let edge = s.at(pitch, 0.0) + egui::vec2(a.cos(), a.sin()) * backed;
                        assert!(
                            rect.contains(edge),
                            "{name} {low}..{high}: a dot at {pitch} reaches {edge:?}, \
                             outside the pane {rect:?}",
                        );
                    }
                }
            }
        }
    }

    /// A note's dot never grows wider than the track it sits on.
    ///
    /// Which turn a note is on is the whole of what the dot is there to say,
    /// and a dot wider than its track has crossed into the octaves either
    /// side to say it — the picture [`DOT`]'s share is held under 1.0 to
    /// avoid. The share cannot break this on its own; the FLOOR can, being a
    /// length in points with no term for the track in it at all, so the sizes
    /// worth asking are the small ones, at the range that makes tracks
    /// thinnest.
    ///
    /// Small SQUARE panes rather than `FRAMES`: the fit is driven by the short
    /// side, so a square is the cheapest way to name a short side, and the
    /// ones here run from where the floor first takes over down to the width
    /// of a docked column split three ways.
    #[test]
    fn a_notes_dot_sits_inside_its_own_turn() {
        // The fresh Analyzer range, which is the ~10 octaves that makes a
        // track thin enough for any of this to bind.
        let cfg = SpectrumConfig::default();
        for side in [400.0f32, 300.0, 277.0, 250.0, 200.0, 160.0, 134.0, 120.0] {
            let rect = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(side, side));
            let s = Spiral::new(rect, &cfg);
            let (backed, fill) = s.dot();
            assert!(
                backed <= s.half(),
                "a {side}pt pane draws a dot {} across on a {} track",
                2.0 * backed,
                s.dr,
            );
            // And the coloured fill is still inside its own backing, so the
            // pair reads as a dot with an edge rather than as one flat disc.
            assert!(
                fill > 0.0 && fill < backed,
                "a {side}pt pane fills {fill} of a {backed} dot",
            );
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
            let fill = Spiral::new(PANE, &state.spectrum_config).dot().1;
            state.tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
            // The COLOURED disc of the pair, not its backing: both are circles,
            // and counting either alone counts the notes once.
            painted(&mut state, 0.1)
                .iter()
                .filter(|s| matches!(s, egui::Shape::Circle(c) if c.radius == fill))
                .count()
        };
        assert_eq!(marks(60), 1, "a note inside the range is marked once");
        assert_eq!(marks(24), 0, "a note below the range is not marked");
        assert_eq!(marks(120), 0, "a note above the range is not marked");
    }

    /// The seam is drawn where one turn stops and the next starts — half a
    /// track out from the note below it, which is half a track in from the note
    /// an octave above — and it winds once per octave boundary.
    ///
    /// Both claims, because either alone passes on a seam nobody could count
    /// by: a line at the right radius that only covered the first turn would
    /// still be "between the rows", and a line of the right length drawn on the
    /// track centres would cross every note in the disc.
    ///
    /// Read off the drawn path rather than off the placement, and the pitch it
    /// is checked against is recovered from the point's own ANGLE — so this
    /// asks where the seam landed on screen rather than asking the formula
    /// about itself.
    #[test]
    fn the_seam_runs_between_the_turns_once_per_octave() {
        let mut state = fresh();
        state.spectrum_config.low_midi = 36.0;
        state.spectrum_config.high_midi = 96.0;
        let s = Spiral::new(PANE, &state.spectrum_config);
        let points = painted(&mut state, 0.1)
            .into_iter()
            .find_map(|shape| match shape {
                egui::Shape::Path(path) => Some(path.points.clone()),
                _ => None,
            })
            .expect("the seam is the pane's one path");

        for p in &points {
            let v = *p - s.centre;
            // The inverse of `ray`: C is up and pitch ascends clockwise.
            let turn = v.x.atan2(-v.y).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
            // The lowest pitch on this ray, and then whichever octave of it
            // this point is nearest — `round` is what picks the octave, so a
            // seam drawn at the wrong OFFSET rounds to the same turn and shows
            // up as the residual below rather than being explained away.
            let low = s.min_midi + (turn * 12.0 - s.min_midi).rem_euclid(12.0);
            let octave = ((v.length() - s.radius(low) - s.half()) / s.dr).round();
            let below = low + 12.0 * octave;
            assert!(
                (v.length() - s.radius(below) - s.half()).abs() < 0.05,
                "a seam point at {p:?} is {} out, not on the edge of {below}'s track",
                v.length(),
            );
            // ...which is the same radius the octave above it starts at. The
            // turns abut, and this is the half of it the seam depends on.
            assert!(
                (s.radius(below + 12.0) - s.half() - s.radius(below) - s.half()).abs() < 1e-3,
                "the turn above {below} does not start where {below}'s own ends",
            );
        }

        // Five octaves of range is four boundaries between turns, so four
        // whole turns of seam.
        let winding: f32 = points
            .windows(2)
            .map(|w| {
                let (a, b) = (w[0] - s.centre, w[1] - s.centre);
                (a.x * b.y - a.y * b.x).atan2(a.dot(b))
            })
            .sum();
        let turns = winding / std::f32::consts::TAU;
        assert!((turns - 4.0).abs() < 1e-3, "the seam winds {turns} turns, not one per boundary");
    }

    /// Every sounding pitch class is named on the rim, once, and the name lands
    /// outside the disc rather than over the spectrum.
    ///
    /// All twelve at once, which is what makes this about the RIM rather than
    /// about one direction on it: a name is set square to the screen whatever
    /// ray it stands on, so the room it needs past the disc is different at
    /// every angle and only a full chromatic asks all of them.
    ///
    /// Every frame rather than the docked one, because the band is what holds
    /// the names in and the band is not one size: below a short side of about
    /// 395 points it is capped by its share of the radius and the whole thing
    /// scales, so the frames are what ask whether a name still fits the band
    /// once the band is the thing that shrank.
    #[test]
    fn every_sounding_pitch_class_is_named_on_the_rim() {
        for (name, rect) in FRAMES {
            let mut state = fresh();
            state.spectrum_config.low_midi = 48.0;
            state.spectrum_config.high_midi = 84.0;
            for note in 60..72 {
                state.tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
            }
            let (batch, spiral) = rim_names(&state, rect, 0.1);
            assert_eq!(batch.pieces().len(), 12, "{name}: twelve pitch classes, twelve names");
            // Nothing a name is made of leaves the pane — letters and drawn
            // marks alike, which are cut from two different sheets and meet
            // nowhere but here. What holds this is the band `Spiral::new`
            // reserves; the pane paints through a clipping painter, so an
            // overrun is cut off rather than merely tight.
            for ink in name_ink(&batch) {
                assert!(
                    rect.contains_rect(ink),
                    "{name}: a name covers {ink:?}, outside the pane {rect:?}",
                );
            }
            the_letters_stand_clear_of_the_disc(name, &batch, &spiral);
        }
    }

    /// The half of [`every_sounding_pitch_class_is_named_on_the_rim`] that is
    /// about the DISC rather than the pane.
    fn the_letters_stand_clear_of_the_disc(
        name: &str,
        batch: &crate::text::TextBatch,
        spiral: &Spiral,
    ) {
        // The LETTER stands clear of the disc, so no name is laid over the
        // spectrum it is there to help read. Asked of the letter alone because
        // that is what the placement promises: the mark column is stacked
        // around the letter rather than along the ray, and a comma sign drops
        // below it (see `NameLead::Letter`).
        let (_, r_out) = spiral.bounds();
        for piece in batch.pieces() {
            let near = piece.ink.distance_to_pos(spiral.centre);
            assert!(
                near >= r_out,
                "{name}: the name {:?} reaches in to {near}, over a disc {r_out} across",
                piece.text,
            );
        }
    }

    /// One name per pitch CLASS, however many octaves of it are sounding: they
    /// leave the disc on one ray and spell one word, so a second is ink on top
    /// of ink.
    #[test]
    fn octaves_of_one_pitch_class_are_named_once() {
        let mut state = fresh();
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        for note in [48, 60, 72] {
            state.tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
        }
        let (batch, _) = rim_names(&state, PANE, 0.1);
        assert_eq!(batch.pieces().len(), 1, "three Cs are one name");
    }

    /// A note bent to just UNDER a pitch class is named with it, not beside it.
    ///
    /// This is the wrap at the end of `names`' `class`, and it is the one
    /// thing there that only a bent note reaches: every whole MIDI number
    /// lands on a multiple of the grain, so the rounding has nowhere to go and
    /// the modulo is dead against integers. Four cents flat of C rounds to
    /// 1200 cents rather than to 0 — the top of the circle, which IS C's own
    /// ray — so without the wrap it is a class of its own and prints C's name
    /// a second time about a degree away, the smear `NAME_GRAIN_CENTS` is
    /// there to prevent.
    ///
    /// Four cents is well inside the vibrato this pane is read under, so a
    /// held note crosses this boundary rather than sitting on one side of it.
    #[test]
    fn a_note_bent_just_under_a_pitch_class_is_named_with_it() {
        let mut state = fresh();
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 72, 1.0));
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 72,
            kind: NoteEventKind::Tuning { semitones: -0.04 },
        });
        let spiral = Spiral::new(PANE, &state.spectrum_config);
        let lit = sounding(&spiral, &state, 0.1);
        // Both are on the disc, so one name is a name they SHARE rather than
        // one of them having been dropped on the way in.
        assert_eq!(lit.len(), 2, "both notes sound: {lit:?}");
        assert!((lit[1].pitch - 71.96).abs() < 1e-3, "the bend landed at {}", lit[1].pitch);
        let (batch, _) = rim_names(&state, PANE, 0.1);
        assert_eq!(batch.pieces().len(), 1, "a C and a C four cents flat are one name");
    }

    /// A voice the disc cannot show is not named either. The dots and the names
    /// read one list of sounding voices, and this is the half of that a reader
    /// would notice: a name on the rim with no dot on any turn under it.
    #[test]
    fn a_note_outside_the_range_is_not_named() {
        let mut state = fresh();
        state.spectrum_config.low_midi = 48.0;
        state.spectrum_config.high_midi = 84.0;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 24, 1.0));
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 120, 1.0));
        let (batch, _) = rim_names(&state, PANE, 0.1);
        assert_eq!(batch.pieces().len(), 0, "nothing on the disc, nothing on the rim");
    }

    /// The band the names are set in is reserved whatever is sounding, so the
    /// disc is one size and does not breathe as notes come and go.
    #[test]
    fn the_name_band_is_reserved_whether_or_not_anything_sounds() {
        let mut scaled = 0;
        for (name, rect) in FRAMES {
            let cfg = SpectrumConfig { low_midi: 36.0, high_midi: 96.0, ..Default::default() };
            let s = Spiral::new(rect, &cfg);
            let half_pane = rect.width().min(rect.height()) * 0.5;
            let (_, r_out) = s.bounds();
            assert!(
                (r_out + s.band - (half_pane - MARGIN_PT)).abs() < 1e-3,
                "{name}: the disc and its band do not fill the pane's radius",
            );
            // Capped by a share of the radius, so a name never takes a pane
            // over — and never more than its own point size asks for.
            assert!(
                s.band <= half_pane * NAME_BAND_SHARE + 1e-3
                    && s.band <= NAME_BAND_PT.0 + NAME_BAND_PT.1 + 1e-3,
                "{name}: the band is {} points wide",
                s.band,
            );
            if s.name_scale() < 1.0 {
                scaled += 1;
            }
        }
        // And at least one frame is on the far side of the cap, so the scaled
        // band is a path this suite runs rather than one it describes. Every
        // frame sat at exactly the dialled size once, and the fixture said
        // otherwise in its own doc.
        assert!(scaled > 0, "no frame here is small enough for the share to bind");
    }
}
