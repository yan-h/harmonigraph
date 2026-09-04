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
//! This draws the analyzer's CURRENT frame and nothing else — no trail or
//! history. This evaluation branch can add one of two sparse sharpening
//! overlays from a temporary right-click menu; dense-only stays the default,
//! and no choice is persisted.
//!
//! It shares the Analyzer's [`SpectrumConfig`](crate::SpectrumConfig) whole
//! rather than carrying settings of its own. Its geometry follows the
//! analyzer's pitch and level windows, while color comes from the volume-color
//! range and gradient dialled on the Colors page.
//!
//! One consequence of sharing worth expecting: `tilt` pivots at 1 kHz, so on a
//! spiral it lifts by RADIUS rather than along a straight axis — a brightness
//! that grows outward from the middle turns.
//!
//! What this pane carries of its OWN is the framing — [`SpiralView`], a
//! magnifier over the fit and a point of the disc to look at. That is not one
//! more analyzer setting: the settings above say what the disc is showing, and
//! the framing says how closely it is being looked at. It is the lattice's
//! camera on a flat picture, and it answers the lattice's gestures — see
//! [`navigate`], which is also where the reason the wheel here does not mean
//! what the wheel next door means is written down.

use egui::Color32;

use super::spectral::axes::{power_db, spectrogram_level_db};
use super::spectral::roll::note_color;
use super::spectral::spectrogram::{cell_color, footprint_mean};
use crate::SharedState;

/// Evaluation-only choice of what, if anything, sharpens the dense Spiral.
/// Stored in egui's temporary memory by [`spiral_pane`], never in plugin state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SpectrumComparison {
    #[default]
    Dense,
    RefinedPeaks,
    Reassigned,
}

impl SpectrumComparison {
    fn label(self) -> &'static str {
        match self {
            Self::Dense => "Dense only",
            Self::RefinedPeaks => "Dense + parabolic peaks",
            Self::Reassigned => "Dense + reassignment",
        }
    }

    fn overlay(
        self,
        spectrum: &crate::AudioSpectrum,
        now: f64,
    ) -> Option<&crate::spectrum::SpectrumBuckets> {
        match self {
            Self::Dense => None,
            Self::RefinedPeaks => spectrum.refined_peaks(now),
            Self::Reassigned => spectrum.reassigned(now),
        }
    }
}

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
/// Two things buy the inner turns room, and they are worth telling apart:
/// narrowing the Analyzer's pitch range gives them more of the disc, and
/// magnifying the disc ([`SpiralView`]) gives them more POINTS of the same
/// share of it. The first changes what the picture holds and the second how
/// closely it is being read; either is the intended way to read this pane
/// closely, and they compose.
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
/// the seam is one hairline all the way round.
///
/// What that saves depends on the pane, both curves being cut by arc length and
/// only then capped at one step per bucket. At the fresh range: on a docked pane
/// the seam is under a quarter of the strip's 3826 steps and cutting it at the
/// strip's grain would roughly double what the pane hands the tessellator; at
/// 1080p it is two thirds; and at 3840x2160 the seam runs into the cap, which is
/// where the coarser grain stops saving anything and starts being the reason the
/// cap is not felt. The sagitta holds at every one of those, so what grows is a
/// curve already smoother than it needs to be rather than one going rough.
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

/// The retired fixed backing's inset. It remains only as the colored dot's
/// established body size; the backing itself is now the inherited spectral
/// geometry shadow.
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

/// How far the disc may be magnified, as a multiple of its own fit.
///
/// The FLOOR is 1 and is not a matter of taste: the fit already draws the whole
/// disc inside the pane, and radius is the octave from the innermost turn to the
/// rim, so there is nothing outside the picture to pull back to. A zoom under 1
/// would draw the same picture smaller with well around it — which is where this
/// parts company with the lattice's own zoom out, that one revealing more nodes
/// because the lattice is unbounded.
///
/// The CEILING is measured off the reason to magnify at all. At the analyzer's
/// full range the innermost turn has about a fifth of the outermost one's
/// circumference (see [`INNER_HOLE`]), so five times the radius is what gives
/// the inner turn the room the outer one has at the fit; eight leaves headroom
/// past that for a reader going in on one turn. Past it a pane holds a couple of
/// turns and no spiral, which is a picture this one has stopped being.
const ZOOM: (f32, f32) = (1.0, 8.0);

/// How far off centre the pane may look, in units of the drawn disc's outer
/// radius — 1 puts the RIM under the pane's centre.
///
/// A bound on where a reader may LOOK rather than on where the disc may go, and
/// stated in the disc's own radius so that it means the same thing at every zoom
/// and on every pane. What it buys is that the middle of the pane always has
/// spiral under it: outside the rim there is only the names' band and the well,
/// so a pan that could bring those to the middle is a pan onto nothing, with
/// the whole picture off the pane and the double-click the only way to find it
/// again.
const LOOK_MAX: f32 = 1.0;

/// How the disc is FRAMED in the pane: the magnifier over its fit, and which
/// point of it the pane's centre is looking through that at.
///
/// A magnifier and not a second fit. Nothing here changes which pitch is where —
/// an octave is still one turn, a partial still lands on its own ray — so every
/// reading the picture offers survives being looked at closely, and what changes
/// is only how much of the pane one turn gets. Narrowing the Analyzer's pitch
/// range is the other way to spend the pane on fewer octaves and it is a
/// different thing: it changes what the disc is SHOWING. The two compose, and
/// [`INNER_HOLE`] says which is which.
///
/// Persisted, like the lattice's [`Camera`](harmonigraph_scene::Camera) and for
/// the lattice's reason: a framing is dialled in by hand, a take renders from
/// the blob, and a disc dialled in on its inner turns has to export the picture
/// it was dialled to. What that costs is that a framing left somewhere odd is
/// still there next session — the same trade the camera already makes, and the
/// double-click is the way out of it.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SpiralView {
    /// How much larger than its fit the disc is drawn. 1 is the whole disc in
    /// the pane; see [`ZOOM`] for both ends.
    pub zoom: f32,
    /// Which point of the disc the pane's CENTRE looks at, in units of the
    /// disc's own outer radius: the origin is the spiral's centre, and length 1
    /// is somewhere on the rim (see [`LOOK_MAX`]).
    ///
    /// A point of the PICTURE rather than an offset in points, which is what
    /// makes one saved framing the same framing at every size it is drawn at —
    /// the docked pane, the Video preview and an export at any resolution alike.
    /// The same argument the roll's own divider is a share for
    /// (`SpectrumConfig::roll_fraction`), and the reason a magnified disc can be
    /// composed into a video at all.
    pub look: glam::Vec2,
}

impl Default for SpiralView {
    fn default() -> Self {
        SpiralView { zoom: ZOOM.0, look: glam::Vec2::ZERO }
    }
}

impl SpiralView {
    /// Magnify by `factor` about `anchor` — where the pointer is, as an offset
    /// from the point the pane's centre is looking at, in disc radii — so
    /// whatever is under it stays where it is on screen.
    ///
    /// Everywhere but the rim, and [`LOOK_MAX`] is why: the look is written
    /// through [`looking_at`], so where the anchor asks for a look past the rim
    /// the shortened one is what lands, and the anchored point slides under the
    /// pointer by the difference. The clamp wins deliberately — the middle of
    /// the pane always having spiral under it is what keeps the picture findable
    /// at all, where an anchor held exactly is what makes one gesture feel
    /// right — so with the rim already centred and the pointer on the outward
    /// side, a zoom creeps instead of pivoting.
    /// `a_zoom_at_the_rim_holds_the_look_and_lets_the_anchor_slide` is what pins
    /// which of the two gives.
    fn zoom_about(&mut self, factor: f32, anchor: egui::Vec2) {
        let zoom = (self.zoom * factor).clamp(ZOOM.0, ZOOM.1);
        // `anchor` is in radii at the CURRENT zoom, so what the look has to
        // travel is the share of it the magnification changes that unit by —
        // and nothing at all where the clamp refused the zoom, which is what
        // keeps a wheel spun at either end from walking the picture sideways.
        let travel = 1.0 - self.zoom / zoom;
        self.look = looking_at(self.look + glam::vec2(anchor.x, anchor.y) * travel);
        self.zoom = zoom;
    }

    /// Slide the picture with the hand: `by` is the drag in disc radii. The
    /// content follows the pointer, so the point being looked AT travels the
    /// other way.
    fn slide(&mut self, by: egui::Vec2) {
        self.look = looking_at(self.look - glam::vec2(by.x, by.y));
    }

    /// Fit a deserialized framing to what the gestures can produce.
    ///
    /// The same door and the same argument as
    /// [`Camera::sanitize`](harmonigraph_scene::Camera::sanitize): a gesture
    /// cannot write either field out of range, a hand-edited blob can, and both
    /// of these MULTIPLY the geometry the pane paints — a NaN zoom is a disc of
    /// NaN vertices, which is a panic inside egui's tessellator rather than a
    /// wrong picture, and a panic in the editor takes the host with it.
    ///
    /// The look is repaired as a whole vector, as the camera's target is: the
    /// two components are read together and one NaN takes the other through
    /// [`looking_at`]'s length.
    pub(crate) fn sanitize(&mut self) {
        let fresh = SpiralView::default();
        self.zoom =
            if self.zoom.is_finite() { self.zoom } else { fresh.zoom }.clamp(ZOOM.0, ZOOM.1);
        self.look = looking_at(if self.look.is_finite() { self.look } else { fresh.look });
    }
}

/// A look held to the disc — see [`LOOK_MAX`]. Every write of the field goes
/// through this, so "the pane's centre is on the picture" is a property of the
/// type rather than something each gesture remembers.
fn looking_at(look: glam::Vec2) -> glam::Vec2 {
    look.clamp_length_max(LOOK_MAX)
}

/// Where a MIDI pitch lands in the pane.
///
/// Two numbers do all of it: `angle = 2π · frac(midi / 12)` and
/// `radius = r0 + (midi - min_midi) / 12 · dr`. Everything drawn here speaks
/// MIDI pitch and a radial offset, and only this knows where that is on screen.
///
/// The framing is folded into those two numbers and the centre by
/// [`framed`](Self::framed), so nothing that draws has to know whether a hand
/// has been over the pane: there is one geometry, and it is the one the picture
/// is at.
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
    /// Fit the spiral to `rect` over the Analyzer's pitch range — the whole
    /// disc, centred, which is what the pane opens at and what
    /// [`framed`](Self::framed) is a transform on.
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

    /// The same disc under a [`SpiralView`]: magnified about the pane's centre,
    /// then slid until the point the view is looking at is at that centre.
    ///
    /// Two multiplies and an offset, on top of a fit left exactly as
    /// [`new`](Self::new) solved it. That is the whole reason the framing is a
    /// second step rather than a term in the fit: every argument there — the hole
    /// as a share, the reach reserved at both ends, the band taken off the radius
    /// first — is about the picture's proportions, and a magnifier keeps all of
    /// them. What it does not keep is the promise that the disc is INSIDE the
    /// pane, which is what magnifying means; the pane paints through a clipping
    /// painter, so what hangs over the edge is cut off.
    ///
    /// The names' [`band`](Self::band) is the one thing not magnified. It is
    /// measured in points because it holds TYPE, and the type it holds is set at
    /// the size the Display tab dialled it to (see [`NAME_PT`]) — a name grown
    /// eight times over would be a word across the pane. So a magnified disc
    /// carries the same names at the same size, standing off the rim by the same
    /// air. The band still comes off the FIT's radius, since reserving room is
    /// only ever a question at the fit: magnified, the rim it stands outside is
    /// off the pane anyway.
    fn framed(self, view: &SpiralView) -> Spiral {
        // The drawn outer radius — the unit `look` is measured in. Read off the
        // fit's own bounds so the unit is the disc a reader can see rather than
        // any of the radii it is built from.
        let unit = self.bounds().1 * view.zoom;
        Spiral {
            centre: self.centre - egui::vec2(view.look.x, view.look.y) * unit,
            r0: self.r0 * view.zoom,
            dr: self.dr * view.zoom,
            ..self
        }
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
    fn dot(&self) -> f32 {
        let backed = (self.half() * DOT.0).max(DOT.1).min(self.half());
        // The fill is never under half the dot, so a track thin enough to
        // shrink the mark below two rings' worth still has a fill to carry the
        // note's colour — which is the half of the pair that says WHICH note
        // this is.
        (backed - DOT_RING_PT).max(backed * 0.5)
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
/// A pointer over this pane moves the FRAMING and nothing else — see
/// [`navigate`], which runs before the picture is drawn so that a drag lands in
/// the frame the hand moved in rather than the one after it.
///
/// Every copy of the pane answers whatever pointer is over IT, which today is
/// the docked one and nothing else: the offline renderer has no pointer, and the
/// Video tab's preview cannot reach a spiral at all (`panes::render`'s
/// `Pane::Spiral` arm). Were that arm ever to become live, this would want the
/// Analyzer's own `DOCKED_SURFACE` gate for the Analyzer's reason — a wheel
/// spent zooming inside a scrolling settings tab is a wheel that tab cannot be
/// scrolled with.
///
/// `surface` is which live copy this is, and the two things the pane holds
/// between frames are keyed on it: the halo's bloom chain and the rim names'
/// instance buffer. The docked tab is 0; offline a layout hands each placement
/// its index (see [`draw_pane`](crate::draw_pane)), so a `.ron` naming the
/// spiral twice grows a chain per rect instead of tearing one down and
/// rebuilding it between the two.
pub(crate) fn spiral_pane(ui: &mut egui::Ui, state: &mut SharedState, now: f64, surface: usize) {
    let cfg = state.spectrum_config;
    let (rect, response) =
        ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
    let comparison_id = egui::Id::new(("spiral-spectrum-comparison", surface));
    let mut comparison =
        ui.data(|data| data.get_temp::<SpectrumComparison>(comparison_id)).unwrap_or_default();
    response.context_menu(|ui| {
        ui.label("Spiral spectrum");
        ui.selectable_value(&mut comparison, SpectrumComparison::Dense, "Dense only");
        ui.selectable_value(
            &mut comparison,
            SpectrumComparison::RefinedPeaks,
            "Dense + parabolic peaks",
        );
        ui.selectable_value(
            &mut comparison,
            SpectrumComparison::Reassigned,
            "Dense + reassignment",
        );
    });
    ui.data_mut(|data| data.insert_temp(comparison_id, comparison));
    if rect.width() < 10.0 || rect.height() < 10.0 {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, crate::theme::well());

    let fit = Spiral::new(rect, &cfg);
    navigate(ui, &response, &fit, &mut state.spiral_view);
    let spiral = fit.framed(&state.spiral_view);
    painter.add(egui::Shape::mesh(strip(&spiral, state, &cfg, now)));
    if let Some(levels) = comparison.overlay(&state.spectrum, now) {
        painter.add(egui::Shape::mesh(overlay_strip(&spiral, levels, &cfg)));
        painter.text(
            rect.left_top() + egui::vec2(8.0, 7.0),
            egui::Align2::LEFT_TOP,
            comparison.label(),
            egui::FontId::monospace(11.0),
            crate::theme::text_dim(),
        );
    }
    seam(&painter, &spiral);
    rays(&painter, &spiral);
    let lit = sounding(&spiral, state, now);
    let marks = dots(&spiral, state, &lit);
    let dot_shadow = state.view.shadow.spectral_geometry.clamped();
    if dot_shadow.casts() && !marks.is_empty() {
        painter.add(harmonigraph_render::dot_shadow_paint_callback(
            rect,
            marks.clone(),
            dot_shadow,
            state.target_format,
            crate::panes::lattice::pane_id(surface),
            crate::text::spiral_shadow_surface(surface),
            painter.ctx().cumulative_pass_nr(),
        ));
    }
    for mark in &marks {
        painter.circle_filled(
            egui::pos2(mark.center[0], mark.center[1]),
            mark.radius,
            Color32::from_rgba_premultiplied(
                mark.color[0],
                mark.color[1],
                mark.color[2],
                mark.color[3],
            ),
        );
    }
    // The LATTICE's bloom, on the dots. One setting for every picture rather
    // than a bar of its own here: the lattice's nodes, the roll's ribbons and
    // these dots are the same notes in the same colors, and a light one of them
    // has that the others do not is a difference between them that says
    // nothing. Through the renderer's own bound for the same reason.
    //
    // Skipped whole when there is nothing to light, which is what keeps a
    // reader who never turns the bloom on from paying for its pipelines at all
    // — the callback would decline the work, but not before building them.
    let bloom = harmonigraph_render::bloom_strength(state.view.bloom_strength);
    if bloom > 0.0 && !marks.is_empty() {
        painter.add(harmonigraph_render::glow_paint_callback(
            rect,
            marks,
            bloom,
            state.target_format,
            crate::panes::lattice::pane_id(surface),
            painter.ctx().cumulative_pass_nr(),
        ));
    }
    // The names last, and outside the disc, so nothing in the picture is over
    // them and they are over nothing in it — the halo above included, which is
    // the lattice's rule for a label as well.
    let mut labels = crate::text::TextBatch::default();
    names(&painter, &spiral, state, &lit, &mut labels);
    labels.flush(
        &painter,
        rect,
        state,
        crate::text::spiral_names(surface),
        // Nothing here scrolls: a name sits on its note's ray for as long as
        // the note sounds, so the filter has no travel to follow and takes the
        // axis every still surface takes. A pan is travel, but it is a
        // DIAGONAL one lasting only while the hand moves, which is a case this
        // pair cannot name and would not pay for — see `SlideAxis`, where the
        // lattice's orbiting camera is the same answer.
        harmonigraph_render::SlideAxis::Across,
        Some(state.view.shadow.spectral_text),
        Some(crate::text::spiral_shadow_surface(surface)),
    );
    painter.add(harmonigraph_render::spectral_shadow_prepare_callback(
        rect,
        crate::text::spiral_shadow_surface(surface),
        painter.ctx().cumulative_pass_nr(),
    ));
}

/// Drag to slide the disc under the pane, scroll or pinch to magnify it, and
/// double-click back to the whole disc: the lattice's three gestures on a flat
/// picture, which is where a reader will have learnt them.
///
/// What they move is the [`SpiralView`] and never the pitch range, and that seam
/// is worth stating outright because the Analyzer beside it answers the same
/// wheel with the RANGE (`panes::spectral::gestures::drag_zoom`). On a straight
/// pitch axis the range is the only zoom there is: the axis runs off both ends of
/// the pane, so what a hand reaches for is more of it. This disc holds its whole
/// range at once by construction — radius is the octave, hole to rim — so there
/// is nothing off the pane to fetch, and what a reader is short of is SIZE on the
/// inner turns. A magnifier is what gives them that. The range is still the
/// Analyzer section's bar, the two compose, and neither gesture can reach the
/// other's value.
///
/// The whole cost of magnifying is the rim: the names live outside it, so far
/// enough in they are off the pane and clipped away. The twelve rays are what
/// still says which direction is which, C's being the heavy one — which is the
/// job [`RAY_FADE`] weights them for.
///
/// `fit` is the UNFRAMED disc, so `fit.centre` is the pane's own centre and
/// `fit.bounds().1` the radius a `look` of 1 means: the gestures are in points
/// and the view is in disc radii, and this is the one place the two meet.
fn navigate(ui: &egui::Ui, response: &egui::Response, fit: &Spiral, view: &mut SpiralView) {
    // The whole disc again, centred. The lattice's own double-click, and the way
    // back from a framing dragged somewhere unreadable — which is what lets the
    // gestures be as free as they are.
    if response.double_clicked() {
        *view = SpiralView::default();
        return;
    }
    let out = fit.bounds().1;
    if response.dragged() {
        // Spent BEFORE the zoom below, and a frame can carry both — a trackpad
        // hands over two fingers sliding while they spread routinely. A drag is
        // spent in the radius the picture the hand moved OVER was drawn at, which
        // is the framing before this frame's zoom, so it goes first. The zoom then
        // anchors on the pointer, and `slide` leaves `zoom` alone, so the anchor
        // below is the same offset either way round and holds whatever the drag has
        // just brought under the cursor. Spending the drag after the zoom instead
        // spends it twice: the anchor is read from `pointer_hover_pos`, which is
        // where the pointer is once this frame's motion has been applied, so the
        // drag is already inside it.
        view.slide(response.drag_delta() / (out * view.zoom));
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    if let Some((scroll, pinch)) = super::zoom_gesture(ui, response) {
        // The Analyzer's own rate, quoted rather than picked afresh: one wheel
        // over one analyzer drawn two ways, so a notch closes each picture by
        // about the same third. WHAT it closes differs; how far a hand has to
        // spin does not.
        let factor = (scroll * super::spectral::ZOOM_PER_SCROLL_POINT).exp() * pinch;
        if (factor - 1.0).abs() > 1e-4 {
            // About the POINTER rather than the pane's centre: what is being
            // magnified is a turn already being looked at, and a zoom about the
            // middle walks it off the pane and leaves the drag to fetch it back.
            // The pitch wheel next door is anchored the same way and for the same
            // reason. With the pointer somewhere this cannot ask about — a pinch
            // arriving with no hover position — the centre is the honest
            // fallback, being the one point of the pane that is certainly on it.
            let anchor = ui
                .ctx()
                .pointer_hover_pos()
                .map_or(egui::Vec2::ZERO, |p| (p - fit.centre) / (out * view.zoom));
            view.zoom_about(factor, anchor);
        }
    }
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

    // The run of buckets one step covers, resampled by the heatmap's own
    // operator and not read by a MAX — the same read the Spectral pane's curve
    // takes, and for the same reason: the largest of N buckets grows with N, so
    // a max would lift this pane's noise floor as the pitch range was widened.
    let half_step = span / (2.0 * steps as f32);
    let level = |midi: f32| {
        let Some(levels) = levels else { return 0.0 };
        let bucket_x = |m: f32| (m - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32;
        footprint_mean(levels, bucket_x(midi - half_step), bucket_x(midi + half_step))
    };

    let mut mesh = egui::Mesh::default();
    for i in 0..=steps {
        let midi = spiral.min_midi + span * i as f32 / steps as f32;
        // Opaque, and untinted by anything of this pane's: the gradient's dark
        // end is black, so silence recedes into the disc rather than letting
        // the pane's own `well` through in rings between the turns.
        let color = cell_color(
            cfg.spectrogram_gradient,
            spectrogram_level_db(cfg, power_db(level(midi)), midi),
        );
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

/// The selected sparse candidate over the unchanged dense strip.
///
/// Both candidates take this exact geometry, pitch resampling, dB mapping,
/// gradient and alpha path. Their estimator is therefore the only variable in
/// the A/B. A max is intentional here where it is not in [`strip`]: each sparse
/// line is only two 3.125-cent buckets wide, so averaging a wide screen
/// footprint would make it disappear as the view zoomed out and compare the
/// renderer rather than the sharpening.
fn overlay_strip(
    spiral: &Spiral,
    levels: &crate::spectrum::SpectrumBuckets,
    cfg: &crate::SpectrumConfig,
) -> egui::Mesh {
    use harmonigraph_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    let span = spiral.max_midi - spiral.min_midi;
    let (r_in, r_out) = spiral.bounds();
    let cap = (span * BINS_PER_SEMITONE as f32).max(MIN_STEPS);
    let steps = (arc_len(r_in, r_out, span / 12.0) / SEGMENT_PT).clamp(MIN_STEPS, cap) as usize;
    let half_step = span / (2.0 * steps as f32);
    let level = |midi: f32| {
        let bucket_x = |m: f32| (m - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32;
        footprint_peak(levels, bucket_x(midi - half_step), bucket_x(midi + half_step))
    };

    let mut mesh = egui::Mesh::default();
    for i in 0..=steps {
        let midi = spiral.min_midi + span * i as f32 / steps as f32;
        let mapped = spectrogram_level_db(cfg, power_db(level(midi)), midi);
        let base = cell_color(cfg.spectrogram_gradient, mapped);
        let color = Color32::from_rgba_unmultiplied(
            base.r(),
            base.g(),
            base.b(),
            (255.0 * mapped).round() as u8,
        );
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

/// Read a sparse line over one screen segment. Across more than one bucket the
/// strongest deposit survives; inside one bucket the same dB-linear sampling
/// as [`footprint_mean`] keeps motion continuous between bucket centres.
fn footprint_peak(powers: &[f32], x0: f32, x1: f32) -> f32 {
    if powers.is_empty() {
        return 0.0;
    }
    let top = powers.len() as f32 - 1.0;
    let first = x0.floor().clamp(0.0, top) as usize;
    let last = x1.floor().clamp(0.0, top) as usize;
    if last > first {
        let lo = x0.clamp(0.0, powers.len() as f32);
        let hi = x1.clamp(0.0, powers.len() as f32);
        return powers[first..=last]
            .iter()
            .enumerate()
            .filter_map(|(offset, &power)| {
                let bucket = (first + offset) as f32;
                (hi.min(bucket + 1.0) > lo.max(bucket)).then_some(power)
            })
            .fold(0.0, f32::max);
    }
    let x = 0.5 * (x0 + x1) - 0.5;
    let bucket = x.floor().clamp(0.0, (powers.len() as f32 - 2.0).max(0.0)) as usize;
    let fraction = (x - bucket as f32).clamp(0.0, 1.0);
    let (a, b) = (power_db(powers[bucket]), power_db(powers[(bucket + 1).min(powers.len() - 1)]));
    10f32.powf(0.1 * (a + (b - a) * fraction))
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
    // Capped at the strip's own grain — one step per analyzer bucket — for the
    // case the magnifier makes: arc length is linear in radius, so a disc drawn
    // eight times over asks for eight times the seam, and the seam is the one
    // curve here whose step count nothing else bounds. The sagitta survives the
    // cap: at the extreme it binds hardest on — a 4K frame magnified to the hilt
    // over ten octaves, where a step is 121 points of arc at a radius of 7400 —
    // a straight segment departs its curve by a quarter of a point, and by
    // hundredths on a docked pane at any zoom.
    let cap = (span * harmonigraph_core::spectrum::BINS_PER_SEMITONE as f32).max(MIN_STEPS);
    let steps = (arc_len(r_a, r_b, span / 12.0) / SEAM_SEGMENT_PT).clamp(MIN_STEPS, cap) as usize;
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
///
/// Hands back the coloured discs it painted, for the halo the caller lays over
/// them ([`harmonigraph_render::glow_paint_callback`]). Returned from HERE
/// rather than derived a second time beside it, so the halo cannot grow from a
/// dot the picture does not have: one loop decides where a mark is, how big,
/// and what colour, and the light follows it by construction.
///
/// The BACKING is not in that list. It is black, and black is the one thing
/// that cannot bloom — handed over it would only take light out of the halo the
/// coloured disc does grow, which is the rule the roll's outline follows too.
fn dots(
    spiral: &Spiral,
    state: &SharedState,
    sounding: &[Sounding],
) -> Vec<harmonigraph_render::GlowDot> {
    let fill = spiral.dot();
    sounding
        .iter()
        .map(|voice| {
            let at = spiral.at(voice.pitch, 0.0);
            let color = note_color(state, voice.pitch, voice.strength);
            harmonigraph_render::GlowDot {
                center: [at.x, at.y],
                radius: fill,
                color: color.to_array(),
            }
        })
        .collect()
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
        let name =
            super::spectral::names::note_name(&state.view, &shown, &state.tuning, voice.pitch);
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
    use crate::tests::probe::{events_into, fresh, painted_into, press, themed};
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

    /// Each menu choice reads its own live product, and the default reads none.
    /// Distinct nonzero buckets make both non-default match arms necessary.
    #[test]
    fn comparison_modes_select_distinct_live_products() {
        let mut spectrum = crate::AudioSpectrum { last_samples: Some(1.0), ..Default::default() };
        spectrum.refined_peaks[41] = 0.25;
        spectrum.reassigned[73] = 0.5;

        assert_eq!(SpectrumComparison::default(), SpectrumComparison::Dense);
        assert!(SpectrumComparison::Dense.overlay(&spectrum, 1.0).is_none());
        let peaks = SpectrumComparison::RefinedPeaks.overlay(&spectrum, 1.0).unwrap();
        let reassigned = SpectrumComparison::Reassigned.overlay(&spectrum, 1.0).unwrap();
        assert_eq!((peaks[41], peaks[73]), (0.25, 0.0));
        assert_eq!((reassigned[41], reassigned[73]), (0.0, 0.5));
    }

    /// A two-bucket line must survive a screen segment spanning many buckets,
    /// or widening the pitch range would compare two invisible estimators.
    #[test]
    fn sparse_footprints_keep_the_strongest_line() {
        let mut powers = [0.0f32; 12];
        powers[6] = 0.8;
        assert_eq!(footprint_peak(&powers, 1.2, 10.7), 0.8);
    }

    /// The candidate mesh is transparent away from a line and visible on it,
    /// so it augments rather than replaces the dense strip underneath.
    #[test]
    fn a_sparse_overlay_leaves_the_dense_strip_visible() {
        use harmonigraph_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

        let cfg = SpectrumConfig { low_midi: 60.0, high_midi: 72.0, ..Default::default() };
        let spiral = Spiral::new(PANE, &cfg);
        let mut powers = [0.0; harmonigraph_core::spectrum::SPECTRUM_BINS];
        let bucket = ((66.0 - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32) as usize;
        powers[bucket] = 1.0;
        let mesh = overlay_strip(&spiral, &powers, &cfg);
        assert!(mesh.vertices.iter().any(|vertex| vertex.color.a() == 0));
        assert!(mesh.vertices.iter().any(|vertex| vertex.color.a() > 0));
    }

    /// A framing, as a gesture would leave it: through the type's own clamp, so
    /// no fixture can ask about a `look` the pane cannot be navigated to.
    fn view(zoom: f32, look: egui::Vec2) -> SpiralView {
        SpiralView { zoom, look: looking_at(glam::vec2(look.x, look.y)) }
    }

    /// Which point of the DISC a screen position is over, in the units
    /// [`SpiralView::look`] is in — the framing read backwards, so a fixture can
    /// ask what was under the pointer before a gesture and what is under it
    /// after.
    ///
    /// Written out from the fit rather than by inverting [`Spiral::framed`]
    /// through the drawn spiral: a transform compared with itself agrees at every
    /// sign, which is exactly what these tests are about.
    fn under(view: &SpiralView, at: egui::Pos2) -> egui::Vec2 {
        let fit = Spiral::new(PANE, &SpectrumConfig::default());
        egui::vec2(view.look.x, view.look.y) + (at - fit.centre) / (fit.bounds().1 * view.zoom)
    }

    /// One frame of the whole pane on a context of `frames`' own, with events
    /// delivered — what every gesture fixture below is built from.
    fn frame(ctx: &egui::Context, state: &mut SharedState, events: Vec<egui::Event>) {
        let _ = events_into(ctx, SCREEN, PANE, events, |ui| spiral_pane(ui, state, 100.0, 0));
    }

    /// The framing a drag from `from` by `delta` leaves behind, driven through
    /// the real pane.
    ///
    /// Four frames, exactly as the Analyzer's own drag fixture takes: egui
    /// resolves the widget under the pointer from the PREVIOUS pass, so the press
    /// needs a frame before it, and a drag registers only once the pointer has
    /// moved while held.
    fn dragged(start: SpiralView, from: egui::Pos2, delta: egui::Vec2) -> SpiralView {
        let mut state = fresh();
        state.spiral_view = start;
        let ctx = themed();
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from)]);
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from), press(from, true)]);
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from + delta)]);
        frame(&ctx, &mut state, vec![press(from + delta, false)]);
        state.spiral_view
    }

    /// The framing `points` of wheel with the pointer at `at` leaves behind.
    ///
    /// Several frames of it, because egui SMOOTHS a scroll: one notch arrives
    /// spread over the frames after it, so a fixture asks where the picture has
    /// got to rather than what one frame did.
    fn scrolled(start: SpiralView, at: egui::Pos2, points: f32) -> SpiralView {
        let mut state = fresh();
        state.spiral_view = start;
        let ctx = themed();
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(at)]);
        for _ in 0..6 {
            frame(
                &ctx,
                &mut state,
                vec![egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, points),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::default(),
                }],
            );
        }
        state.spiral_view
    }

    /// The framing ONE frame carrying both a drag and a pinch leaves behind: the
    /// press and the frame before it exactly as [`dragged`] takes them, then a
    /// single frame delivering the motion and the pinch factor together.
    ///
    /// `egui::Event::Zoom` rather than a ctrl-held wheel, because a wheel is
    /// smoothed over the frames after it and this fixture is about one frame: the
    /// event multiplies `zoom_delta` in the pass it arrives in, so the drag and
    /// the magnification land in the same call to [`navigate`].
    fn dragged_and_pinched(
        start: SpiralView,
        from: egui::Pos2,
        delta: egui::Vec2,
        pinch: f32,
    ) -> SpiralView {
        let mut state = fresh();
        state.spiral_view = start;
        let ctx = themed();
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from)]);
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from), press(from, true)]);
        frame(
            &ctx,
            &mut state,
            vec![egui::Event::PointerMoved(from + delta), egui::Event::Zoom(pinch)],
        );
        state.spiral_view
    }

    fn painted(state: &mut SharedState, now: f64) -> Vec<egui::Shape> {
        painted_into(SCREEN, PANE, |ui| spiral_pane(ui, state, now, 0))
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
            assert!(sin.abs() < 1e-5, "midi {midi} and {} are {sin} off one ray", midi + 12.0,);
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
    /// bucket-run footprint, `footprint_mean`, `loudness` and `cell_color` are reached
    /// by nothing at all, and the picture's whole colour path reads as covered
    /// while being untested.
    ///
    /// Two claims, because either alone passes on a broken pane: the peak is at
    /// the right PITCH (a strip built in the wrong order fails this) and it is
    /// in the right DIRECTION on screen (an angle map that is not
    /// `frac(midi/12)` fails this, and would otherwise draw a perfectly
    /// plausible spiral).
    ///
    /// What it does not pin is which operator the run is read by — a
    /// single-bucket read would still put the peak here. That choice is argued
    /// in `strip`'s own comment and is shared with the Spectral pane, whose
    /// `footprint_mean` tests hold it.
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
                let backed = s.dot();
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
            let fill = s.dot();
            assert!(
                fill <= s.half(),
                "a {side}pt pane draws a dot {} across on a {} track",
                2.0 * fill,
                s.dr,
            );
            assert!(fill > 0.0, "a {side}pt pane lost its sounding-note dot");
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
                assert!(!bounds.any_nan(), "{low}..{high} painted NaN geometry: {shape:?}",);
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
            let fill = Spiral::new(PANE, &state.spectrum_config).dot();
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

    /// The halo grows from the dots the picture HAS: every mark handed to the
    /// renderer is one of the coloured discs on the painter, at its centre, its
    /// radius and its colour, and the black backing is not among them.
    ///
    /// Asked of [`dots`] rather than of the callback, whose payload is the
    /// render crate's own type and opaque from here. That is where the pair is
    /// decided anyway, and this is the reason the marks are RETURNED from there
    /// rather than derived a second time beside the call: a second derivation
    /// is a second place for the picture and its light to disagree.
    #[test]
    fn the_halo_grows_from_the_dots_that_were_painted() {
        let mut state = fresh();
        for note in [55u8, 60, 67, 76] {
            state.tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
        }
        let spiral = Spiral::new(PANE, &state.spectrum_config);
        let lit = sounding(&spiral, &state, 0.1);
        let marks = dots(&spiral, &state, &lit);
        assert_eq!(marks.len(), 4, "the fixture's four notes are four coloured discs");
        assert!(marks.iter().all(|mark| mark.radius == spiral.dot()));
        assert!(marks.iter().all(|mark| mark.color[3] > 0));
    }

    /// Nothing to light asks for no halo: the pane adds the callback for a lit
    /// dot at a strength above zero and for nothing else.
    ///
    /// Counted as a DIFFERENCE rather than by picking the glow's callback out
    /// of the frame, because the names are a callback of their own and the two
    /// are the same shape from here. What the count is worth is that the
    /// callback carries GPU pipelines built on first sight of it, so a reader
    /// with the bloom off pays for none of them.
    #[test]
    fn nothing_to_light_asks_for_no_halo() {
        let callbacks = |bloom: f32, sounding: bool| {
            let mut state = fresh();
            state.view.bloom_strength = bloom;
            if sounding {
                state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
            }
            painted(&mut state, 0.1)
                .iter()
                .filter(|s| matches!(s, egui::Shape::Callback(_)))
                .count()
        };
        assert_eq!(
            callbacks(1.2, true),
            callbacks(0.0, true) + 1,
            "a lit dot at a strength above zero asks for exactly one more callback",
        );
        assert_eq!(
            callbacks(1.2, false),
            callbacks(0.0, false),
            "a strength above zero asked for a halo with nothing sounding",
        );
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

    /// The framing a fresh pane opens at is the plain fit, to the bit.
    ///
    /// Which is what every test above is about: they all ask [`Spiral::new`] and
    /// describe the pane a reader opens, and they only go on doing so while the
    /// framing at rest is inert. A default that magnified by a hair would leave
    /// them all passing about a picture nobody sees.
    #[test]
    fn the_fresh_framing_draws_exactly_the_fit() {
        let cfg = SpectrumConfig::default();
        let fit = Spiral::new(PANE, &cfg);
        let framed = fit.framed(&SpiralView::default());
        assert_eq!(
            (framed.centre, framed.r0, framed.dr, framed.band),
            (fit.centre, fit.r0, fit.dr, fit.band),
            "the fresh framing is not the fit",
        );
    }

    /// Magnifying draws the same picture larger: every radius scales, the pitch
    /// map does not move, and the type does not grow with it.
    ///
    /// Three claims because they are three different promises, and each fails on
    /// its own. A magnifier that also rotated or re-wound would still look like a
    /// spiral — which is the trap [`an_octave_is_one_turn_of_the_spiral`] exists
    /// for, one transform along. A band that scaled with the disc would set the
    /// rim names at eight times their dialled size, a word across the pane. And a
    /// track that did NOT scale would leave the dots and the seam drawn against a
    /// picture that had moved out from under them.
    #[test]
    fn magnifying_scales_the_picture_and_not_the_pitch_map_or_the_type() {
        let cfg = SpectrumConfig { low_midi: 36.0, high_midi: 96.0, ..Default::default() };
        let fit = Spiral::new(PANE, &cfg);
        for zoom in [1.5f32, 3.0, ZOOM.1] {
            let s = fit.framed(&view(zoom, egui::Vec2::ZERO));
            assert_eq!(s.centre, fit.centre, "zoom {zoom} moved the centre of a centred look");
            for midi in [36.0f32, 47.5, 60.0, 96.0] {
                assert!(
                    (s.radius(midi) - fit.radius(midi) * zoom).abs() < 1e-3,
                    "zoom {zoom}: {midi} draws at {} rather than {}",
                    s.radius(midi),
                    fit.radius(midi) * zoom,
                );
                assert_eq!(s.ray(midi), fit.ray(midi), "zoom {zoom}: {midi} changed direction");
            }
            assert!(
                (s.half() - fit.half() * zoom).abs() < 1e-3 && s.dot() > fit.dot(),
                "zoom {zoom}: the track and the dot on it must grow with the picture",
            );
            // The names: same band, same size, and the same air between the ink
            // and whatever rim the disc now has.
            assert_eq!(
                (s.band, s.name_scale()),
                (fit.band, fit.name_scale()),
                "zoom {zoom} resized the rim names",
            );
            let standoff = |s: &Spiral| (s.rim(60.0) - s.centre).length() - s.bounds().1;
            assert!(
                (standoff(&s) - standoff(&fit)).abs() < 1e-3,
                "zoom {zoom}: a name stands {} off the rim rather than {}",
                standoff(&s),
                standoff(&fit),
            );
        }
    }

    /// A `look` names the point of the disc the pane's CENTRE is at, which is
    /// what makes one saved framing mean one picture at any size it is drawn.
    ///
    /// Asked at the rim (`look` of length 1, the furthest [`LOOK_MAX`] allows)
    /// and half way out, on rays that are not the cardinals: the look is a
    /// vector and a transform that dropped or swapped a component still centres
    /// correctly on the axes.
    #[test]
    fn a_look_puts_its_own_point_of_the_disc_at_the_pane_centre() {
        let cfg = SpectrumConfig::default();
        let fit = Spiral::new(PANE, &cfg);
        let out = fit.bounds().1;
        for midi in [37.0f32, 60.5, 71.0, 94.25] {
            for share in [1.0f32, 0.5] {
                let ray = fit.ray(midi);
                let s = fit.framed(&view(2.0, ray * share));
                // The same point of the picture, named in the pane's own terms:
                // `share` of the way out along that ray from the spiral's centre.
                let drawn = s.centre + ray * (out * share * 2.0);
                assert!(
                    (drawn - fit.centre).length() < 1e-2,
                    "looking {share} out along {midi}'s ray put it at {drawn:?}, \
                     not the pane's centre {:?}",
                    fit.centre,
                );
            }
        }
    }

    /// The wheel magnifies about the POINTER: whatever is under it stays under
    /// it, so a reader zooms in on the turn they are already looking at instead
    /// of watching it slide off the pane.
    #[test]
    fn the_wheel_magnifies_about_the_pointer() {
        // Off centre, and off both axes, so a zoom about the middle — or about
        // one axis' worth of the pointer — moves what is under the pointer.
        let at = PANE.center() + egui::vec2(70.0, -40.0);
        let before = SpiralView::default();
        let after = scrolled(before, at, 30.0);
        assert!(
            after.zoom > before.zoom + 0.1,
            "scrolling up must magnify ({} -> {})",
            before.zoom,
            after.zoom,
        );
        let (was, now) = (under(&before, at), under(&after, at));
        assert!(
            (was - now).length() < 1e-3,
            "the disc point under the pointer moved from {was:?} to {now:?}",
        );
        // And scrolling back down opens the picture out again, so the gesture is
        // reversible rather than a one-way trip to the ceiling.
        assert!(
            scrolled(after, at, -30.0).zoom < after.zoom,
            "scrolling down must pull the picture back out",
        );
    }

    /// At the rim it is the ANCHOR that gives, not the look: a zoom there keeps
    /// the pane's centre on the picture and lets what is under the pointer slide.
    ///
    /// Which is [`the_wheel_magnifies_about_the_pointer`]'s promise held against
    /// [`LOOK_MAX`], and the case is reachable rather than theoretical — the rim
    /// centred and the pointer on the outward side of the pane's centre is where
    /// any pan that has run out of disc leaves a reader, one notch from a wheel.
    /// The bound is the one worth keeping: a pane whose middle has no spiral under
    /// it is a picture only the double-click can find. So this pins the cost rather
    /// than reporting it.
    #[test]
    fn a_zoom_at_the_rim_holds_the_look_and_lets_the_anchor_slide() {
        // The rim under the pane's centre, and the pointer further out still, so
        // the zoom asks for a look past the rim and cannot be given one.
        let before = view(2.0, egui::vec2(0.0, -1.0));
        assert!((before.look.length() - LOOK_MAX).abs() < 1e-6, "the fixture is not at the rim");
        let at = PANE.center() + egui::vec2(0.0, -60.0);
        let after = scrolled(before, at, 30.0);
        assert!(
            after.zoom > before.zoom + 0.1,
            "the wheel never magnified ({} -> {})",
            before.zoom,
            after.zoom,
        );
        assert!(
            after.look.length() <= LOOK_MAX + 1e-4,
            "a zoom at the rim looked {} radii off centre",
            after.look.length(),
        );
        // Pinned exactly, there being nowhere further out for it to go — so it is
        // the anchored point that moves. The pointer's offset from the centre is a
        // fixed number of POINTS, and a larger picture makes that fewer disc radii,
        // so it comes to sit nearer the rim point the pane's centre is held on.
        assert_eq!(after.look, before.look, "the clamp let the look off the rim");
        let (was, now) = (under(&before, at), under(&after, at));
        assert!(
            now.y > was.y + 1e-2,
            "the point under the pointer went from {was:?} to {now:?} rather than sliding",
        );
    }

    /// A drag slides the picture with the hand: the point of the disc under the
    /// press is the point under the pointer where the drag ends.
    ///
    /// Grab-style, which is the lattice's own promise for its pan, and EXACT: the
    /// drag is spent in the radius the picture is drawn at, so what the hand
    /// travels the picture travels.
    ///
    /// Exact to a rounding error because the fixture moves in one step. A drag
    /// slow enough to cross egui's click radius in stages loses those first few
    /// points, and that is not this arithmetic: it is the disambiguation any
    /// widget sensing clicks and drags alike has to do before it can know which
    /// it has, and the lattice's own pan spends the same points on it.
    #[test]
    fn a_drag_slides_the_picture_with_the_hand() {
        let from = PANE.center() + egui::vec2(-30.0, 20.0);
        let delta = egui::vec2(60.0, -45.0);
        let before = view(3.0, egui::Vec2::ZERO);
        let after = dragged(before, from, delta);
        // Both in disc radii, the units the framing itself is in.
        let (grabbed, landed) = (under(&before, from), under(&after, from + delta));
        assert!(
            (grabbed - landed).length() < 1e-4,
            "the point grabbed at {grabbed:?} was carried to {landed:?}",
        );
        // The right WAY, which the tolerance above would forgive on its own: a
        // drag up and to the right brings the picture up and to the right, so
        // the point being looked at travels down and to the left.
        let travel = egui::vec2(after.look.x - before.look.x, after.look.y - before.look.y);
        assert!(
            travel.x < 0.0 && travel.y > 0.0,
            "the look travelled {travel:?} for a drag of {delta:?}",
        );
    }

    /// A frame carrying both a drag and a pinch spends the drag ONCE: the point
    /// grabbed at the press is still under the pointer where the drag ends, and
    /// the magnification is the pinch's own.
    ///
    /// Both at once is what a trackpad hands over routinely — two fingers sliding
    /// while they spread — and it is the only case where the ORDER [`navigate`]
    /// reads the two gestures in is visible. The pinch is anchored on
    /// `pointer_hover_pos`, which is where the pointer is once this frame's motion
    /// has been applied, so the drag is already inside the anchor; a drag spent on
    /// top of a framing that anchor has already zoomed is spent twice, and the
    /// picture lands the drag times whatever the zoom changed the unit by past
    /// where the hand left it.
    ///
    /// Both directions of pinch, from a zoom with room to move either way: the
    /// error is the drag times one minus the zoom factor, so it changes sign
    /// across a pinch of 1 and a single direction would leave half of it unasked.
    #[test]
    fn a_frame_carrying_a_drag_and_a_pinch_spends_the_drag_once() {
        let from = PANE.center() + egui::vec2(-30.0, 20.0);
        let delta = egui::vec2(60.0, -45.0);
        for pinch in [0.5f32, 2.0] {
            let before = view(3.0, egui::Vec2::ZERO);
            let after = dragged_and_pinched(before, from, delta, pinch);
            // The pinch reached the pane at all, and neither end of [`ZOOM`]
            // clamped it — a refused zoom has no unit change to double-count and
            // would let the claim below pass on the bug.
            assert!(
                (after.zoom - before.zoom * pinch).abs() < 1e-3,
                "a pinch of {pinch} left the zoom at {} rather than {}",
                after.zoom,
                before.zoom * pinch,
            );
            let (grabbed, landed) = (under(&before, from), under(&after, from + delta));
            assert!(
                (grabbed - landed).length() < 1e-4,
                "pinch {pinch}: the point grabbed at {grabbed:?} was carried to {landed:?}, \
                 {} radii off",
                (grabbed - landed).length(),
            );
        }
    }

    /// A drag moves the framing and nothing else. In particular not the pitch
    /// range, which is what the same drag does on the Analyzer next door
    /// (`spectral::gestures::drag_zoom`) — and the range is shared, so a spiral
    /// that panned it would re-zoom the pane beside it.
    #[test]
    fn a_drag_leaves_the_analyzers_own_settings_alone() {
        let mut state = fresh();
        let before = state.spectrum_config;
        let ctx = themed();
        let from = PANE.center() + egui::vec2(-30.0, 20.0);
        let delta = egui::vec2(60.0, -45.0);
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from)]);
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from), press(from, true)]);
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(from + delta)]);
        frame(&ctx, &mut state, vec![press(from + delta, false)]);
        assert_ne!(
            state.spiral_view.look,
            SpiralView::default().look,
            "the drag never reached the pane",
        );
        assert_eq!(
            (before.low_midi, before.high_midi, before.roll_seconds, before.ceiling_db),
            (
                state.spectrum_config.low_midi,
                state.spectrum_config.high_midi,
                state.spectrum_config.roll_seconds,
                state.spectrum_config.ceiling_db,
            ),
            "a drag on the spiral moved an Analyzer setting",
        );
    }

    /// A double-click is the way back to the whole disc — the lattice's own
    /// reset, and what makes the freedom of the other two gestures safe.
    #[test]
    fn a_double_click_returns_to_the_whole_disc() {
        let mut state = fresh();
        state.spiral_view = view(5.0, egui::vec2(0.8, -0.4));
        let ctx = themed();
        let at = PANE.center() + egui::vec2(40.0, 40.0);
        frame(&ctx, &mut state, vec![egui::Event::PointerMoved(at)]);
        for _ in 0..2 {
            frame(&ctx, &mut state, vec![press(at, true)]);
            frame(&ctx, &mut state, vec![press(at, false)]);
        }
        let after = state.spiral_view;
        assert_eq!(
            (after.zoom, after.look),
            (SpiralView::default().zoom, SpiralView::default().look),
            "a double-click must land back on the fit",
        );
    }

    /// Neither gesture can take the picture off the pane or the magnifier out of
    /// its range, however far it is pushed.
    ///
    /// The drag is the one worth driving through the pane rather than through
    /// [`SpiralView::slide`] alone: a drag of ten pane-widths is what a hand
    /// actually does when it means "as far as this goes", and the clamp has to
    /// answer that rather than a single tidy delta.
    #[test]
    fn a_gesture_cannot_take_the_framing_off_the_picture() {
        let far = dragged(view(2.0, egui::Vec2::ZERO), PANE.center(), egui::vec2(4000.0, -3000.0));
        assert!(
            far.look.length() <= LOOK_MAX + 1e-4,
            "a long drag looked {} radii off centre",
            far.look.length(),
        );
        // And the look it lands on is the direction it was dragged, not a
        // component of it: a clamp per axis would put the corner of a square
        // where the rim is.
        assert!(far.look.x < 0.0 && far.look.y > 0.0, "the clamp turned the drag: {:?}", far.look);
        let at = PANE.center();
        assert_eq!(scrolled(SpiralView::default(), at, 400.0).zoom, ZOOM.1, "the ceiling holds");
        let out = scrolled(view(ZOOM.1, egui::Vec2::ZERO), at, -400.0);
        assert_eq!(out.zoom, ZOOM.0, "and the floor, which is the whole disc");
    }

    /// A wheel spun at either end of [`ZOOM`] leaves the look exactly where it is:
    /// where the clamp refuses the magnification, the picture does not travel at
    /// all.
    ///
    /// [`SpiralView::zoom_about`] divides by the CLAMPED zoom for this, so the
    /// share the magnification changes the unit by is nothing when it changes it by
    /// nothing. Dividing by the factor the gesture ASKED for instead agrees at
    /// every zoom the clamp lets through and moves the look on every notch the
    /// clamp eats — a reader holding the wheel at the ceiling would watch the disc
    /// walk sideways under a magnification that is not changing, and there is no
    /// way back but the double-click.
    ///
    /// Off-centre pointers, and that is the whole of why this is its own fixture:
    /// at the pane's own centre the anchor is exactly zero and the look cannot move
    /// under any formula at all, so
    /// [`a_gesture_cannot_take_the_framing_off_the_picture`]'s scrolls — which are
    /// the other two at the clamps — cannot ask it.
    #[test]
    fn a_wheel_spun_at_either_end_leaves_the_look_where_it_is() {
        // Off both axes, so a formula walking one component is caught too.
        let at = PANE.center() + egui::vec2(70.0, -40.0);
        // From the centred look and from one already off it: a travel term that
        // ZEROED the look rather than leaving it alone passes the first.
        for look in [egui::Vec2::ZERO, egui::vec2(0.3, -0.2)] {
            // The ceiling spun further in, then the floor spun further out.
            for (end, points) in [(ZOOM.1, 400.0f32), (ZOOM.0, -400.0)] {
                let before = view(end, look);
                let after = scrolled(before, at, points);
                assert_eq!(after.zoom, end, "a wheel at {end} magnified past it");
                assert_eq!(
                    after.look, before.look,
                    "{points} points of wheel refused at {end} walked the look",
                );
            }
        }
    }

    /// A framing a hand-edited blob can carry — a NaN, an infinity, a zoom out
    /// of range — loads as one the pane can draw.
    ///
    /// The same threat and the same door as [`SpectrumConfig::sanitize`]: both
    /// fields multiply the geometry this pane paints, and NaN geometry is a panic
    /// inside egui's tessellator rather than a wrong picture — which in the
    /// editor takes the host down with it.
    #[test]
    fn a_hand_edited_framing_loads_at_a_drawable_one() {
        let bad = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -3.0, 1e30];
        for zoom in bad {
            for look in bad {
                let mut framing = SpiralView { zoom, look: glam::vec2(look, -look) };
                framing.sanitize();
                assert!(
                    (ZOOM.0..=ZOOM.1).contains(&framing.zoom),
                    "zoom {zoom} loaded as {}",
                    framing.zoom,
                );
                assert!(
                    framing.look.is_finite() && framing.look.length() <= LOOK_MAX + 1e-4,
                    "a look of {look} loaded as {:?}",
                    framing.look,
                );
                // And the repaired framing paints finite geometry, which is the
                // claim the ranges above are only a proxy for.
                let mut state = fresh();
                state.spiral_view = framing;
                for shape in painted(&mut state, 0.1) {
                    assert!(
                        !shape.visual_bounding_rect().any_nan(),
                        "zoom {zoom}, look {look} painted NaN geometry",
                    );
                }
            }
        }
    }

    /// A magnified disc is still cut at the analyzer's own grain: arc length
    /// decides how many segments each curve is drawn from, and magnifying
    /// multiplies arc length, so both caps have to hold or the pane hands the
    /// tessellator eight times the geometry for a picture no smoother.
    ///
    /// The seam is the half worth pinning — the cap is the only thing bounding its
    /// step count, it being one hairline where the strip's colour changes along it
    /// — and the 1080p frame is where it binds: at the fit that frame asks for
    /// about two thirds of the cap, and magnified it asks for many times it.
    #[test]
    fn a_magnified_disc_is_cut_no_finer_than_the_analyzers_grain() {
        use harmonigraph_core::spectrum::BINS_PER_SEMITONE;
        let rect = FRAMES[1].1;
        let mut state = fresh();
        state.spiral_view = view(ZOOM.1, egui::Vec2::ZERO);
        let cfg = state.spectrum_config;
        let span = cfg.high_midi - cfg.low_midi;
        let shapes: Vec<egui::Shape> =
            painted_into(rect.size(), rect, |ui| spiral_pane(ui, &mut state, 0.1, 0))
                .shapes
                .into_iter()
                .map(|s| s.shape)
                .collect();

        // One step per bucket, two vertices a step, and the step count is
        // inclusive of both ends.
        let strip_cap = 2 * (span * BINS_PER_SEMITONE as f32) as usize + 4;
        let seam_cap = ((span - 12.0) * BINS_PER_SEMITONE as f32) as usize + 2;
        let (mut meshes, mut paths) = (0, 0);
        for shape in &shapes {
            match shape {
                egui::Shape::Mesh(mesh) => {
                    meshes += 1;
                    assert!(
                        mesh.vertices.len() <= strip_cap,
                        "the strip is {} vertices against a cap of {strip_cap}",
                        mesh.vertices.len(),
                    );
                }
                egui::Shape::Path(path) => {
                    paths += 1;
                    assert!(
                        path.points.len() <= seam_cap,
                        "the seam is {} points against a cap of {seam_cap}",
                        path.points.len(),
                    );
                }
                _ => {}
            }
        }
        assert_eq!((meshes, paths), (1, 1), "the pane draws one strip and one seam");
    }
}
