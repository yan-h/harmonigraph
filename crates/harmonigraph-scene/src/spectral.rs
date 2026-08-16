//! The lattice's AUDIO channel: what the analyzer says, and the one colour
//! scheme everything lit by it is painted in.
//!
//! Two colour schemes reach the picture, and this module is the whole of the
//! second one:
//!
//! - **MIDI** — [`ViewConfig::pitch_gradient`](crate::ViewConfig) spread over
//!   the Color range, read through
//!   [`pitch_lut_color`](crate::pitch_lut_color). A note's disc, its octave
//!   wedges, its melody and bass marks and its ribbon on the piano roll all
//!   come off that one table, so a pitch is one colour wherever it is drawn.
//! - **FREQUENCY** — the analyzer's own ramp
//!   (`SpectrumConfig::spectrogram_gradient`) against its own Level window,
//!   read through [`gradient_color`](crate::gradient_color). A bucket of the
//!   spectrum curve, a cell of the spectrogram, a segment of the Spiral pane
//!   and every audio-lit element of the lattice come off THAT one table, so a
//!   loudness is one LIGHT wherever it is drawn — added over whatever ground
//!   the surface under it has.
//!
//! One light rather than one colour, because the two surfaces have different
//! beds. The analyzer's panes are bedded on BLACK — a spectrogram cell at
//! silence has to be black or the plane's edge shows — so there the light and
//! the colour are the same thing and the picture is the gradient itself. The
//! ring is bedded on the LATTICE, so it reads a copy of that gradient whose
//! lightness range is anchored on the lattice's own bed ([`ring_gradient`]): a
//! ramp opening at black punches a hole through a grey lattice at every node,
//! which is a picture of a gap where the table means silence. The bed is the
//! skin's `surface_faint` grey — a step ABOVE the lattice's panel ground and
//! level with the octave band's unlit ghost, so a quiet ring is a faintly
//! raised backdrop that is plainly still a reading, where the panel itself
//! would make it vanish and the well grey would read as black.
//!
//! The scheme is chosen by what the element MEASURES, never by which pane it
//! is on: the lattice carries both at once (a node's body held by the keys,
//! its audio ring measured from the spectrum), and the two are told apart by
//! their colour as much as by their radius.
//!
//! That the lattice carries both AT ONCE is the shape of the whole audio
//! channel here rather than a happy accident. The MIDI picture — the node
//! bodies, the octave band, the melody and bass marks — is never lit from the
//! analyzer; the measurement gets a ring of its own inside the band, and
//! [`SpectralReading`] picks which of two readings fills it. So neither
//! picture can be mistaken for the other, and neither has to be given up to
//! see the other.
//!
//! WHICH nodes wear that ring is the other half, and it is [`RingGate`]: the
//! reading is one grid the whole lattice shares, so without a gate every node
//! in view carries a ring of it whatever is sounding, and a lattice where
//! everything is marked says only where the nodes are. A node draws its ring
//! when one of its wedges reaches [`SpectralPaint::gate`], so where the rings
//! ARE is a reading in itself.
//!
//! Nothing in this crate reads audio, so [`SpectralPaint`] arrives already
//! measured — `harmonigraph-ui`'s `panes::spectral_fold` is what fills it, and
//! a scene derived without that pass carries [`SpectralPaint::silent`], which
//! paints nothing at all.

use std::collections::VecDeque;

use glam::Vec4;
use harmonigraph_core::spectrum::{
    BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI,
};

use crate::{pitch_ramp_lut, Gradient, OctaveLayout, ViewConfig, PITCH_LUT_N};

/// The narrowest and widest a wedge of the audio ring may be dialled to span
/// ([`ViewConfig::spectral_ring_range`]), in cents.
///
/// The ceiling is one octave, and it is the setting at which the ring stops
/// being a set of zoomed segments and becomes ONE continuous reading: a wedge
/// stands for exactly the octave it names, so neighbouring wedges meet at the
/// pitch they share and the whole ring is the spectrum bent round the wheel's
/// own pitch map. Past that the wedges would overlap in pitch — two arcs of
/// one node claiming the same frequency — which is a picture that cannot be
/// read as a position.
///
/// The floor is a sixth of an analyzer bucket (3.125¢). Any wedge narrower
/// than a bucket is one bucket stretched across its arc, saying nothing a
/// single number could not — so the floor is not where the picture sharpens,
/// only what keeps a hand-edited zero from collapsing every wedge to its
/// slot's own flat reading.
pub const SPECTRAL_RANGE_MIN: f32 = 0.5;
/// See [`SPECTRAL_RANGE_MIN`].
pub const SPECTRAL_RANGE_MAX: f32 = 1200.0;

/// The two ends of the audio ring's gate ([`ViewConfig::spectral_ring_gate`]):
/// how loud the loudest thing a node's ring shows has to read before that ring
/// is drawn at all.
///
/// A LEVEL and not a power or a dB, because it is the level the ring's own
/// colours are read at: the gate and the ramp index one axis, so the setting
/// says "dimmer than THIS colour and the ring goes away" rather than naming a
/// number that has to be converted before it means anything on screen. Where
/// that axis sits in dB is the analyzer's Level window, and moving that window
/// moves the gate with it — which is the same thing it does to every other
/// reading of the spectrum.
///
/// The floor is the gate's OFF position, and it is off rather than nearly off:
/// the test is `peak >= gate`, so 0 admits every node including one whose ring
/// is silent through and through. That is the picture with no gate at all, and
/// it has to be reachable — a ring at the ramp's floor is a reading (nothing
/// sounds here), and whether it is worth the screen it takes is exactly what
/// this bar is for.
pub const SPECTRAL_GATE_MIN: f32 = 0.0;
/// See [`SPECTRAL_GATE_MIN`]. The ceiling is a full-scale reading — the top of
/// the Level window, where nothing short of the loudest thing the analyzer can
/// report opens a ring.
pub const SPECTRAL_GATE_MAX: f32 = 1.0;

/// Buckets the analyzer's pitch grid holds, and how many of them a semitone
/// spans — `harmonigraph_core::spectrum`'s own numbers, named again here
/// because `harmonigraph-render` sizes its uniform row and walks the grid by
/// them and does not depend on that crate. Aliases, not second values.
pub const SPECTRAL_BUCKETS: usize = SPECTRUM_BINS;
/// See [`SPECTRAL_BUCKETS`].
pub const SPECTRAL_BUCKETS_PER_SEMITONE: usize = BINS_PER_SEMITONE;

/// Which reading of the analyzer the audio ring carries — the one control that
/// says what the lattice's spectrum indicator IS.
///
/// The ring is where a measurement of the sound goes, and the MIDI picture —
/// the node bodies, the octave band, the melody and bass marks — is never any
/// of it. So this is a question about the RING and not about the lattice: it
/// picks which of two readings fills the annulus, and the picture around it is
/// unchanged either way.
///
/// It carries no Off, and that is the point of it: whether the LAYER is drawn
/// is its WIDTH ([`ViewConfig::spectral_ring_width`]), the same off switch every
/// other layer of a node has and in the same place. An Off here would be a
/// second one for this layer alone, and every reader would then have to know
/// which of the two wins. (Which NODES wear the layer is a third question and a
/// different kind of one — see [`ViewConfig::spectral_ring_gate`], which asks
/// what the ring says rather than how big it is.)
///
/// The two are one measurement asked at two zooms, which is what makes them a
/// choice rather than a pair of features. Each wedge of the ring names one
/// octave of the node's pitch class, and:
///
/// - [`Fold`](Self::Fold) answers with ONE number for that octave — is this
///   pitch class sounding here — so a wedge is flat, and what the ring says is
///   read across the lattice rather than within a node.
/// - [`Spectrum`](Self::Spectrum) spreads a window of pitch across the wedge —
///   what is sounding NEAR that octave, and how far off it sits — so a wedge
///   reads as a detuning, and what the ring says is read within one node.
///
/// One choice and not two boxes, because they cannot both fill one annulus and
/// a second annulus is not what either is worth: they answer the same question
/// about the same stretch of sound, and which answer is wanted is a decision
/// about how closely the music is being looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SpectralReading {
    /// One level per wedge, measured at that octave's own pitch: the FOLD — a
    /// Gaussian kernel of [`ViewConfig::spectral_width`](crate::ViewConfig)
    /// cents over a local noise floor, so what survives is energy concentrated
    /// AT the node's pitch rather than energy near it.
    ///
    /// The lattice's native reading. A partial sits at an exact rational
    /// multiple of its fundamental, so folded to pitch class the first sixteen
    /// harmonics occupy six 7-limit nodes and a timbre draws as a
    /// CONSTELLATION anchored at its fundamental — a shape made of many nodes,
    /// which needs each of them to answer with one number per octave for the
    /// shape to be legible at all.
    #[default]
    Fold,
    /// A window of the RAW spectrum across each wedge, spanning
    /// [`ViewConfig::spectral_ring_range`](crate::ViewConfig) cents centred on
    /// that octave's own pitch: a segment of the spiral spectrogram, bent into
    /// the wedge's arc.
    ///
    /// No kernel and no floor, and that is the design rather than an omission:
    /// a wedge shows a window of PITCH, and every smoothing that helps the
    /// fold is a blur across the one axis the window exists to resolve. A
    /// partial dead on the node paints down the middle of its wedge and one a
    /// comma sharp paints to the clockwise side, in the direction pitch rises
    /// everywhere else on the wheel.
    ///
    /// What it costs is stated rather than hidden: with nothing sounding a
    /// wedge is not empty, it is the ramp's floor — the bed the ring's ramp is
    /// anchored on, a step above the lattice's own ground (see
    /// [`ring_gradient`]). A stretch of spectrum with nothing in it is a
    /// reading, not a gap. Whether a node with nothing but that to show is
    /// worth its annulus is the gate's question
    /// ([`ViewConfig::spectral_ring_gate`]), and it is asked of both readings —
    /// though it selects far less sharply here, a window of raw spectrum
    /// finding something loud near almost every pitch class in dense material.
    Spectrum,
}

/// The analyzer's loudness at every bucket of its pitch grid, quantized to a
/// byte apiece.
///
/// A byte because that is the grain the rest of the analyzer's picture is
/// already drawn at — the spectrogram stores its columns as dB bytes and the
/// octave packing sends a level as one — and because the whole grid then fits
/// in the lattice's uniform buffer beside the tables already there. Over a
/// 60 dB Level window a step is 0.24 dB, well under what a gradient shows.
pub type SpectralLevels = [u8; SPECTRUM_BINS];

/// What the analyzer says and how the lattice paints it — the audio channel,
/// whole.
///
/// One struct rather than a scatter of fields because the parts are only ever
/// right TOGETHER: a ramp with no levels behind it paints silence in the
/// analyzer's colours, and levels with no ramp paint a measurement in the
/// pitch ramp's, which is the exact confusion the two schemes exist to
/// prevent. [`silent`](Self::silent) is the one state where none of it is
/// read.
pub struct SpectralPaint {
    /// The FREQUENCY scheme's ramp — `SpectrumConfig::spectrogram_gradient`
    /// through [`pitch_ramp_lut`], the same gradient
    /// the spectrogram's cells, the spectrum curve and the Spiral pane's
    /// segments are read off — bedded on the LATTICE rather than on their
    /// black plane. Every audio-lit element on the lattice indexes it by its
    /// own LEVEL, exactly as those do; what differs is the ground each entry
    /// sits on, which [`ring_gradient`] anchors the ramp to before
    /// [`new`](Self::new) bakes the table.
    pub lut: [Vec4; PITCH_LUT_N],
    /// Each wedge of the ring is ONE reading taken at its own octave's pitch
    /// ([`SpectralReading::Fold`]) rather than a window of pitch spread across
    /// it ([`SpectralReading::Spectrum`]).
    ///
    /// A flag on the paint and not the reading itself, because it is the whole
    /// of what the SHADER has to know: [`levels`](Self::levels) carries a grid
    /// either way — folded here, raw there — and where in the wedge that grid
    /// is sampled is the entire difference between the two pictures.
    pub folded: bool,
    /// The audio ring's inner and outer radius in quad UV units, already
    /// clamped to a drawable span. **Both 0 when the ring is off** — an empty
    /// annulus is the one thing that says the ring is not drawn, so the toggle
    /// reaches the picture as geometry and nothing downstream needs the flag
    /// as well.
    pub inner: f32,
    pub outer: f32,
    /// How many cents of the spectrum one wedge of the ring spans, centred on
    /// that wedge's own octave ([`ViewConfig::spectral_ring_range`]). Already
    /// clamped into [`SPECTRAL_RANGE_MIN`]..=[`SPECTRAL_RANGE_MAX`].
    pub range: f32,
    /// The level a node's loudest wedge has to reach for that node to draw a
    /// ring at all ([`ViewConfig::spectral_ring_gate`], already clamped into
    /// [`SPECTRAL_GATE_MIN`]..=[`SPECTRAL_GATE_MAX`]).
    ///
    /// Per NODE and not per wedge, which is the whole shape of it: a ring is
    /// read as one object — a constellation's worth of wedges, or one node's
    /// spectrum bent round a wheel — so a node either has something to say or
    /// is backdrop, and hiding the quiet wedges of a ring that stays would put
    /// a gap in a reading rather than removing one.
    ///
    /// It is a GATE where everything else in this module is a weight, and the
    /// two are not in the same argument. What a wedge SAYS is a weight to the
    /// last byte: a partial drifting off a node dims smoothly, which is what
    /// makes vibrato breathe instead of flicker (`panes::spectral_fold`'s
    /// "weight, don't gate"). What this decides is whether the ring is on the
    /// screen, and there is no smooth version of that — the ring costs its
    /// annulus at every node in the window whatever it reads, and the cost of
    /// carrying it at a node with nothing in it is the picture around it.
    ///
    /// [`SpectralPaint`] carries it so that the decision and the levels it is
    /// made against are read from one place; [`RingGate`] is where it is
    /// answered per node.
    pub gate: f32,
    /// The reading the ring paints, bucket by bucket of the analyzer's own
    /// pitch grid — the raw spectrum or the fold over it, whichever
    /// [`folded`](Self::folded) says, both already through the analyzer's Level
    /// window.
    ///
    /// ONE grid for the whole lattice, which is what makes the ring cost the
    /// same whatever the extents are: a node's wedges are a window onto this
    /// table, and which part of it each of them reads is the shader's
    /// arithmetic off the node's own pitch class.
    ///
    /// All zeros where nothing has been measured, which the ring draws as the
    /// ramp's floor — the bed it is anchored on — everywhere rather than as an
    /// empty annulus: the ring is a MEASUREMENT of a range, and a range with
    /// nothing in it is a reading, not a gap.
    pub levels: Box<SpectralLevels>,
}

impl SpectralPaint {
    /// No analyzer: no ring, nothing measured into the grid it reads, and a
    /// ramp nothing indexes.
    ///
    /// Zeros and not a bedded ramp, unlike [`new`](Self::new)'s table, because
    /// the annulus here is empty and the shader never reaches it — building a
    /// table nothing samples would only make a scene with no analyzer look
    /// like one whose ring is at silence.
    ///
    /// What [`derive_scene`](crate::derive_scene) answers, and the right
    /// answer for every shell that draws a lattice without opening an
    /// analyzer.
    pub fn silent() -> SpectralPaint {
        SpectralPaint {
            lut: [Vec4::ZERO; PITCH_LUT_N],
            folded: false,
            inner: 0.0,
            outer: 0.0,
            range: SPECTRAL_RANGE_MAX,
            // Nothing held back, for the same reason the table above is zeros:
            // there is no ring here to decide about, and the one state a gate
            // must never be invented in is the one where nobody asked for a
            // reading at all.
            gate: SPECTRAL_GATE_MIN,
            levels: Box::new([0; SPECTRUM_BINS]),
        }
    }

    /// The paint `view` asks for, its table baked from the analyzer's
    /// `gradient` anchored on the lattice's own bed, with no levels measured
    /// into it yet.
    ///
    /// The GRADIENT and not the analyzer's baked table, because the anchoring
    /// is a move of the ramp's lightness range ([`ring_gradient`]) rather than
    /// a blend over the entries it produced: the knobs are what carries it, so
    /// this is the one place they are read for the ring and the one place its
    /// table is built.
    ///
    /// Where the ring sits is [`ViewConfig::rings`]'s answer rather than a
    /// second reading of the same bars, because the ring's inner edge is a sum
    /// over the layers INSIDE it: the core's radius, this ring's own gap, and
    /// whether either is dialled to nothing. That sum belongs in one place, and
    /// its clamps are there rather than in `ViewConfig::sanitize` for the
    /// reason every other geometry clamp is — the drawing code is reached by
    /// more routes than the persist door (a take replay, the offline
    /// renderer's layout, a standalone harness), and a hand-edited view must
    /// still come out as a node somebody can see.
    pub fn new(view: &ViewConfig, gradient: Gradient) -> SpectralPaint {
        let (inner, outer) = view.rings().audio;
        SpectralPaint {
            lut: pitch_ramp_lut(ring_gradient(gradient)),
            folded: view.spectral_reading == SpectralReading::Fold,
            inner,
            outer,
            range: clamp_or(
                view.spectral_ring_range,
                SPECTRAL_RANGE_MAX,
                SPECTRAL_RANGE_MIN,
                SPECTRAL_RANGE_MAX,
            ),
            // A hand-edited NaN falls back to the gate's OFF position, where
            // the range above falls back to a value that draws something: this
            // is the one setting the ring has that can empty the lattice, and a
            // blob nobody can read must not be able to do that silently.
            gate: clamp_or(
                view.spectral_ring_gate,
                SPECTRAL_GATE_MIN,
                SPECTRAL_GATE_MIN,
                SPECTRAL_GATE_MAX,
            ),
            levels: Box::new([0; SPECTRUM_BINS]),
        }
    }

    /// Whether the ring draws at all: an annulus with something in it.
    pub fn ring_draws(&self) -> bool {
        self.outer > self.inner
    }
}

/// Which nodes' rings have anything to say — [`SpectralPaint::gate`] answered
/// against the levels the ring would actually paint, per pitch class.
///
/// Built once for a frame and asked per node, because the expensive half is
/// shared: under [`SpectralReading::Spectrum`] a wedge shows a WINDOW of the
/// grid, so the level it reaches is the loudest bucket in that window rather
/// than the one at its own pitch, and walking the window per node per wedge is
/// the same few hundred reads over and over (1365 nodes by eleven wedges by 64
/// buckets is 960 000 of them at the fresh zoom). [`window_max`] does that
/// reduction once over the grid, and a node's question is then one read per
/// wedge whichever reading is on.
///
/// It answers for a PITCH CLASS rather than for a node, which is what makes it
/// testable in the units the claim is about: a ring is a function of the class,
/// the wheel and the spectrum, and nothing else about the node reaches it.
pub struct RingGate {
    /// The grid a wedge's level is read off: [`SpectralPaint::levels`] under
    /// the fold, where a wedge is one reading at its own pitch, and the loudest
    /// level within half a wedge's window either side of each bucket under the
    /// raw spectrum, where a wedge shows a stretch of the grid at once.
    peaks: Box<SpectralLevels>,
    /// [`SpectralPaint::gate`], carried so the pair cannot be read from two
    /// frames.
    gate: f32,
}

impl RingGate {
    /// The gate `paint` asks for, with its grid reduced to what each wedge of
    /// the ring actually reaches.
    pub fn new(paint: &SpectralPaint) -> RingGate {
        // Half a wedge's window, in buckets: the fold reads ONE pitch per wedge
        // and so has no window at all, and the raw spectrum spreads `range`
        // cents across the arc, centred on the wedge's own pitch.
        let half = if paint.folded { 0 } else { window_half(paint.range) };
        RingGate { peaks: window_max(&paint.levels, half), gate: paint.gate }
    }

    /// The loudest level any wedge of the ring reaches on a node whose pitch
    /// class is `cents`, 0..1 on the analyzer's Level window.
    ///
    /// Every slot the wheel DRAWS and no others, which is why the layout is
    /// asked rather than the octave slots being walked: a ring near the pitch
    /// limits names octaves no note can reach, and they are wedges on screen
    /// like any other — a partial the analyzer hears there is a reason to draw
    /// the ring, and a slot the wheel does not draw is not.
    pub fn peak(&self, layout: &OctaveLayout, cents: f32) -> f32 {
        let (low, high) = layout.slots(cents);
        (low..=high)
            .map(|slot| level_at(&self.peaks, layout.slot_pitch(slot, cents)))
            .fold(0.0, f32::max)
    }

    /// Whether the ring on a node of pitch class `cents` draws: at least one of
    /// its wedges reaching the gate.
    ///
    /// `>=`, so that a gate at [`SPECTRAL_GATE_MIN`] admits every node — the
    /// floor is the bar's off position and has to give back the ungated
    /// picture, silent rings and all.
    pub fn draws(&self, layout: &OctaveLayout, cents: f32) -> bool {
        self.peak(layout, cents) >= self.gate
    }
}

/// Cents one bucket of the analyzer's grid spans: 3.125.
const CENTS_PER_BUCKET: f32 = 1200.0 / (12 * BINS_PER_SEMITONE) as f32;

/// Buckets either side of a wedge's own pitch that wedge shows, at a window of
/// `range` cents across.
///
/// Rounded rather than floored so a window lands on the buckets it covers to
/// within half of one, and floored at 1: at the range bar's own floor (half a
/// cent) the window is a fraction of a bucket, and a zero here would read the
/// wedge's centre bucket alone while the wedge on screen still blends its two
/// neighbours.
fn window_half(range: f32) -> usize {
    ((range * 0.5 / CENTS_PER_BUCKET).round() as usize).max(1)
}

/// `levels` with every bucket replaced by the loudest one within `half` buckets
/// either side — what a wedge centred there reaches, rather than what it reads
/// at its own pitch.
///
/// A monotonic queue, so the whole grid costs one pass whatever the window is:
/// each bucket is pushed and popped once, and the front of the queue is the
/// window's maximum. The window is clamped at the axis ends rather than
/// shortened, which over-reports by nothing — a wedge hanging off the end of
/// the axis draws the floor there (the shader answers 0 past the last bucket),
/// and what is inside the axis is what it can reach.
fn window_max(levels: &SpectralLevels, half: usize) -> Box<SpectralLevels> {
    let mut peaks = Box::new([0u8; SPECTRUM_BINS]);
    if half == 0 {
        peaks.copy_from_slice(levels);
        return peaks;
    }
    // Indices, loudest first: a bucket no longer able to be the maximum of any
    // window still to come — one to its left and no louder — is dropped as it
    // is passed, so the queue holds a descending run and its front is the
    // answer.
    let mut queue: VecDeque<usize> = VecDeque::new();
    let mut next = 0usize;
    for (bucket, peak) in peaks.iter_mut().enumerate() {
        let high = (bucket + half).min(SPECTRUM_BINS - 1);
        while next <= high {
            while queue.back().is_some_and(|&b| levels[b] <= levels[next]) {
                queue.pop_back();
            }
            queue.push_back(next);
            next += 1;
        }
        let low = bucket.saturating_sub(half);
        while queue.front().is_some_and(|&b| b < low) {
            queue.pop_front();
        }
        *peak = queue.front().map_or(0, |&b| levels[b]);
    }
    peaks
}

/// The level `grid` holds at absolute MIDI `pitch`, interpolated between the
/// two buckets either side of it, or 0 where the analyzer's axis does not reach
/// that pitch.
///
/// The CPU's form of the shader's `spectrum_at`, down to the half-bucket offset
/// ([`bucket_pitch`]) and to answering zero rather than the nearest end past
/// the axis: what the gate is measuring is what the ring would paint, so the
/// two have to read the grid the same way or a node is hidden while showing a
/// partial the gate never saw.
fn level_at(grid: &SpectralLevels, pitch: f32) -> f32 {
    let x = (pitch - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32 - 0.5;
    if x < 0.0 || x > (SPECTRUM_BINS - 1) as f32 {
        return 0.0;
    }
    let bucket = x as usize;
    let (low, high) = (grid[bucket], grid[(bucket + 1).min(SPECTRUM_BINS - 1)]);
    let (low, high) = (f32::from(low) / 255.0, f32::from(high) / 255.0);
    low + (high - low) * (x - bucket as f32)
}

/// The analyzer's gradient as the RING reads it: the same arc, with the bottom
/// of its lightness range raised to the `L*` of the bed the ring is drawn on.
///
/// The FREQUENCY scheme's invariant is that a loudness is one LIGHT wherever it
/// is drawn, over whatever ground that surface has — not one colour, which is
/// only the same thing where the ground is black. The analyzer's own panes are
/// that case: the spectrogram's plane IS black (silence there has to be black
/// or the plane's edge shows), so the gradient reaches them untouched and their
/// picture is the gradient itself. The ring's ground is the LATTICE, so its
/// copy of the ramp is re-anchored to stand on it — a ramp opening at black
/// punches a hole through a grey lattice at every node in the window, which is
/// a picture of a gap where the table means silence.
///
/// A move of the LIGHTNESS and not a blend over the baked table, because the
/// lightness is the one axis the bed is a statement about. Screening the
/// analyzer's colours over the bed instead lifts every channel by
/// `bed * (1 - c)`, which washes the middle of the ramp toward the bed's own
/// grey and never quite lets go at the top. At any bed the re-anchor is the
/// closer of the two to the analyzer's mid colours, and it is EXACT at the loud
/// end where a screen is not: measured over the well grey — the darkest bed on
/// the ladder, and so the one a screen flatters most — the two miss the
/// analyzer's own colours by 10/7/0 and 13/11/10 per channel at `t`
/// 0.25/0.5/1.0. The hue and chroma knobs are untouched at every level, so a
/// loud wedge and a loud spectrogram cell are one colour rather than two that
/// nearly agree.
///
/// A FLOOR and not a pin: a gradient whose bottom already sits above the bed
/// comes back exactly as it stands. The bed is what the ring must not sink
/// under, not where it has to start.
///
/// The pair is stored as a middle plus a SIGNED ramp, so the ends are
/// recomposed rather than set, and `t` = 0 is the bright end wherever the ramp
/// runs downward — the floor is applied to whichever lightness the `t` = 0 end
/// carries, since that is the end a silent wedge draws.
///
/// The bed is [`skin::surface_faint_color`](crate::skin::surface_faint_color);
/// see there for why that rung of the chrome's ladder and not the lattice's own
/// ground.
pub fn ring_gradient(gradient: Gradient) -> Gradient {
    // Sanitized first, so the two ends are read off the gradient that will
    // actually be drawn — and so an untouched one comes back as the very key
    // `pitch_ramp_lut` would memoize it under.
    let gradient = gradient.sanitized();
    let bed = crate::skin::surface_faint_color();
    let bed_l = crate::color::lightness_of_encoded(
        f64::from(bed.x),
        f64::from(bed.y),
        f64::from(bed.z),
    ) as f32;
    let bottom = gradient.lightness - gradient.lightness_ramp * 0.5;
    let top = gradient.lightness + gradient.lightness_ramp * 0.5;
    if bottom >= bed_l {
        return gradient;
    }
    // Recomposed off the TOP end: for a top at or above the bed the subtraction
    // below is exact in f32 (Sterbenz), so `lightness ± ramp/2` lands the
    // `t` = 1 end back on exactly the lightness it had and spends the rounding
    // at the end being moved anyway.
    let lightness = (bed_l + top) * 0.5;
    Gradient { lightness, lightness_ramp: (top - lightness) * 2.0, ..gradient }
}

/// `value` held inside `low..=high`, or `fallback` where it is not a number at
/// all. `clamp` hands a NaN straight back — every comparison against one is
/// false — so a blob's NaN would otherwise walk through as a radius.
fn clamp_or(value: f32, fallback: f32, low: f32, high: f32) -> f32 {
    if value.is_finite() {
        value.clamp(low, high)
    } else {
        fallback
    }
}

/// The MIDI pitch bucket `bucket` of the analyzer's grid is centred on.
///
/// The grid is absolute log pitch at [`BINS_PER_SEMITONE`] buckets a semitone,
/// and a bucket stands for its own CENTRE rather than its low edge — the half
/// step is what keeps a partial's pitch and the bucket that carries it
/// agreeing to better than a bucket, which at 3.125¢ is the whole resolution
/// the ring has to place a partial with.
pub fn bucket_pitch(bucket: usize) -> f32 {
    SPECTRUM_MIN_MIDI + (bucket as f32 + 0.5) / BINS_PER_SEMITONE as f32
}

/// The pitch axis the analyzer reaches, as the ring reads it: 20 Hz to 20 kHz.
pub const SPECTRAL_AXIS: (f32, f32) = (SPECTRUM_MIN_MIDI, SPECTRUM_MAX_MIDI);

#[cfg(test)]
mod tests {
    use super::*;

    fn ringed(width: f32, range: f32) -> ViewConfig {
        ViewConfig {
            spectral_reading: SpectralReading::Spectrum,
            spectral_ring_width: width,
            spectral_ring_range: range,
            ..ViewConfig::default()
        }
    }

    /// The ring's width and its range reach the picture drawable, whatever a
    /// hand-edited blob holds — a NaN (which walks through a `clamp`
    /// untouched), an infinity, a negative width, a value off either end of
    /// its bar.
    ///
    /// A non-finite width is the one case that answers with NO ring, and
    /// deliberately: 0 is the width bar's own off position, so reading a NaN
    /// as it draws the picture some setting could have asked for. Every finite
    /// width above zero has to come out as an annulus somebody can see — the
    /// alternative is a ring that silently is not there while the bar reads out
    /// a number.
    #[test]
    fn a_hand_edited_audio_ring_still_draws_an_annulus() {
        for (width, range) in [
            (0.2, f32::NAN),
            (0.3, 200.0),
            (9.0, 1e9),
            (0.05, -80.0),
            (f32::INFINITY, 0.0),
            (f32::NAN, 200.0),
            (-3.0, 200.0),
        ] {
            let paint = SpectralPaint::new(&ringed(width, range), Gradient::default());
            assert_eq!(
                paint.ring_draws(),
                width.is_finite() && width > 0.0,
                "a width of {width} reached the picture as ({}, {})",
                paint.inner,
                paint.outer,
            );
            assert!(paint.outer <= 1.0, "the ring reaches past the node at {}", paint.outer);
            assert!(
                (SPECTRAL_RANGE_MIN..=SPECTRAL_RANGE_MAX).contains(&paint.range),
                "a range of {range} reached the picture as {}",
                paint.range,
            );
        }
    }

    /// A width of 0 is an empty annulus, which is how the ring's one off switch
    /// reaches the shader: the setting IS the geometry, so the two cannot
    /// disagree, and the reading beside it says nothing about whether there is
    /// a ring.
    #[test]
    fn a_ring_of_no_width_is_an_empty_annulus() {
        for reading in [SpectralReading::Fold, SpectralReading::Spectrum] {
            let view = ViewConfig { spectral_reading: reading, ..ringed(0.0, 200.0) };
            let paint = SpectralPaint::new(&view, Gradient::default());
            assert!(!paint.ring_draws(), "{reading:?} drew a ring with no width");
            assert_eq!((paint.inner, paint.outer), (0.0, 0.0));
        }
        assert!(!SpectralPaint::silent().ring_draws(), "a silent scene drew a ring");
    }

    /// The ring sits where the STACK puts it: a gap out from the core, and
    /// moved by every layer inside it rather than by a radius of its own.
    ///
    /// The whole of what the width bars buy, checked at the one place a second
    /// copy of the sum would drift — the audio channel is built from
    /// `ViewConfig` on its own, without the scene the octave band comes out of.
    #[test]
    fn the_ring_sits_a_gap_out_from_the_core() {
        let view = ringed(0.25, 200.0);
        let paint = SpectralPaint::new(&view, Gradient::default());
        assert_eq!(paint.inner, view.core_radius + view.ring_gap);
        assert_eq!(paint.outer, paint.inner + 0.25);

        // The core off, and the ring reaches the node's center: no layer to
        // stand off, so no gap is spent on one.
        let bare = SpectralPaint::new(
            &ViewConfig { core_radius: 0.0, ..view.clone() },
            Gradient::default(),
        );
        assert_eq!((bare.inner, bare.outer), (0.0, 0.25));
    }

    /// BOTH readings fill the same annulus, at the same radii, and are told
    /// apart by nothing but where in a wedge the grid is sampled.
    ///
    /// The whole shape of the selector, stated where the flag is set: a
    /// version that gave the fold its own geometry — a different ring, or the
    /// band relit — would pass every other test here and quietly be two
    /// features again rather than one control over one indicator.
    #[test]
    fn the_two_readings_share_one_annulus() {
        let raw = ringed(0.2, 200.0);
        let fold = ViewConfig { spectral_reading: SpectralReading::Fold, ..raw };
        let g = Gradient::default();
        let (fold, raw) = (SpectralPaint::new(&fold, g), SpectralPaint::new(&raw, g));
        assert!(fold.ring_draws() && raw.ring_draws(), "a reading drew no ring");
        assert_eq!(
            (fold.inner, fold.outer),
            (raw.inner, raw.outer),
            "the two readings drew at different radii",
        );
        assert!(fold.folded, "the fold is not read at its wedges' own pitches");
        assert!(!raw.folded, "the raw spectrum lost its window across the wedge");
    }

    /// The shape every analyzer preset has: a ramp whose bottom is BLACK,
    /// because the spectrogram's plane is black and silence there has to be
    /// black. Carries chroma at both ends, which is what makes the floor's
    /// colour a question rather than a shade of grey.
    fn analyzers() -> Gradient {
        Gradient {
            hue_start: 302.0,
            hue_span: -188.0,
            lightness: 50.0,
            lightness_ramp: 100.0,
            chroma: 0.615,
            chroma_ramp: 0.77,
        }
    }

    /// The `L*` a drawn entry carries, off the crate's own curve — the units
    /// the ramp is authored in, and the only ones a claim about the bed means
    /// anything in.
    fn lightness(c: Vec4) -> f32 {
        crate::color::lightness_of_encoded(f64::from(c.x), f64::from(c.y), f64::from(c.z)) as f32
    }

    /// A ramp opening at black comes out of [`SpectralPaint::new`] opening at
    /// the BED's lightness — and in the gradient's own bottom colour there,
    /// not in the bed's grey.
    ///
    /// A silent wedge draws the floor deliberately — a reading, not a gap — so
    /// most of the ring is this entry most of the time, and unanchored it is a
    /// black hole punched through the lattice at every node in the window.
    ///
    /// The two halves are the whole design. The lightness is the bed's, which
    /// is what puts the quiet ring level with the surface it sits on; the
    /// colour is the ANALYZER's, which is what keeps it part of one ramp
    /// rather than a grey cap stuck on the bottom of one. A version that
    /// simply painted the bed at level 0 would pass the first assertion and
    /// fail the second.
    #[test]
    fn the_rings_silence_sits_on_the_lattice_rather_than_under_it() {
        let paint = SpectralPaint::new(&ringed(0.3, 200.0), analyzers());
        let floor = paint.lut[0];
        let bed = lightness(crate::skin::surface_faint_color());
        let got = lightness(floor);
        // A fifth of an `L*`, which is about half a byte of the picture's own
        // quantization at this end of the axis: the recomposition rounds the
        // bottom end by an ulp of the middle, and nothing finer than a byte
        // reaches a pixel anyway.
        assert!(
            (got - bed).abs() < 0.2,
            "a silent wedge draws {floor:?} at L* {got}, not the bed's own L* {bed}",
        );
        let (hi, lo) = (floor.truncate().max_element(), floor.truncate().min_element());
        assert!(
            hi - lo > 4.0 / 255.0,
            "the floor entry {floor:?} is a grey, so the ramp's own bottom hue is gone",
        );
        assert_eq!(floor.w, 1.0, "the ring's ramp lost its alpha");
    }

    /// A gradient whose bottom ALREADY sits above the bed comes through
    /// untouched, entry for entry.
    ///
    /// The anchor is a floor and not a pin, and this is the difference: the bed
    /// is what a quiet ring must not sink under, never where the ramp has to
    /// start. A version that set the bottom instead of raising it would darken
    /// every gradient a person had deliberately opened bright, and would look
    /// perfectly correct at the analyzer's own presets.
    #[test]
    fn a_ramp_that_already_clears_the_bed_is_left_where_it_is() {
        let high = Gradient { lightness: 60.0, lightness_ramp: 20.0, ..analyzers() };
        assert!(
            lightness(pitch_ramp_lut(high)[0]) > lightness(crate::skin::surface_faint_color()),
            "the test's own gradient does not open above the bed",
        );
        assert_eq!(ring_gradient(high), high.sanitized(), "the floor moved a ramp above it");
        let paint = SpectralPaint::new(&ringed(0.3, 200.0), high);
        assert_eq!(paint.lut, pitch_ramp_lut(high), "the ring's table is not the analyzer's");
    }

    /// The top of the ring's ramp is the analyzer's own colour EXACTLY, so the
    /// ring and the spectrogram read as ONE measurement where a reading is
    /// actually being looked at.
    ///
    /// Where the re-anchor converges is what makes it a re-anchoring rather
    /// than a second colour scheme: it moves the bottom of the lightness range
    /// and leaves the top where it stands, so the deviation runs to nothing at
    /// the loud end. A screen over the bed never quite lets go — it moves every
    /// entry by `bed * (1 - c)` per channel, which is only zero at white — and
    /// a lerp toward the bed would miss by more the brighter the reading got.
    #[test]
    fn a_loud_reading_is_the_analyzers_own_colour_exactly() {
        let analyzer = pitch_ramp_lut(analyzers());
        let paint = SpectralPaint::new(&ringed(0.3, 200.0), analyzers());
        assert_eq!(
            paint.lut[PITCH_LUT_N - 1],
            analyzer[PITCH_LUT_N - 1],
            "the ramp's top is no longer the analyzer's own colour",
        );
        assert_ne!(
            paint.lut[0], analyzer[0],
            "the ring's floor is the analyzer's black, so nothing was anchored at all",
        );
    }

    /// The fresh wheel, which every gate claim below is read on: five octaves
    /// to the turn around middle C with a fringe either end, so a node's ring
    /// names slots 3..=9 and a partial can be put on one of them by name.
    fn wheel() -> OctaveLayout {
        let view = ViewConfig::default();
        crate::octave_layout(
            view.octave_count,
            view.octave_center,
            view.octave_extras,
            view.octave_extra_size,
            view.octave_extra_blend,
        )
    }

    /// A grid holding one partial: full level within `half` cents of absolute
    /// MIDI `pitch`, and nothing anywhere else.
    ///
    /// A BAND rather than a single bucket because that is the shape a partial
    /// has in the analyzer — one Hann main lobe spans several buckets — and
    /// because a claim about a window has to be made against something the
    /// window can miss by a stated distance.
    fn partial(pitch: f32, half: f32) -> Box<SpectralLevels> {
        let mut levels = Box::new([0u8; SPECTRUM_BINS]);
        for (bucket, level) in levels.iter_mut().enumerate() {
            if ((bucket_pitch(bucket) - pitch) * 100.0).abs() <= half {
                *level = 255;
            }
        }
        levels
    }

    /// The gate a view asks for, over a grid handed in — the pair
    /// [`RingGate::new`] reads, assembled the way the fold's pass assembles it.
    fn gate_of(view: &ViewConfig, levels: Box<SpectralLevels>) -> RingGate {
        let mut paint = SpectralPaint::new(view, Gradient::default());
        paint.levels = levels;
        RingGate::new(&paint)
    }

    /// The fresh view at a stated gate and reading.
    fn gated(gate: f32, reading: SpectralReading) -> ViewConfig {
        ViewConfig {
            spectral_ring_gate: gate,
            spectral_reading: reading,
            ..ViewConfig::default()
        }
    }

    /// A node rings where one of its wedges reaches the gate, and not where
    /// none of them does — the whole of what the setting says, read on a pitch
    /// class rather than on a node.
    ///
    /// A partial on middle C opens every C node on the lattice, because a wedge
    /// names an OCTAVE of the node's class and one of C's octaves is where the
    /// partial is. The tritone away is the node that must stay dark: it is the
    /// furthest any class can be from a partial, and if it rings then the gate
    /// is reading something other than this node's own wedges.
    #[test]
    fn a_node_rings_where_one_of_its_wedges_reaches_the_gate() {
        let wheel = wheel();
        let gate = gate_of(&gated(0.5, SpectralReading::Fold), partial(60.0, 10.0));
        assert!(gate.draws(&wheel, 0.0), "a C node stayed dark with a partial on middle C");
        assert!(!gate.draws(&wheel, 600.0), "the tritone from a lone partial rang");
        // ...and the level it answers is the level the ring paints there, not
        // merely something over the line: a full-scale partial reads full.
        let peak = gate.peak(&wheel, 0.0);
        assert!((peak - 1.0).abs() < 0.01, "a full-scale partial reads {peak} at its own class");
        assert_eq!(gate.peak(&wheel, 600.0), 0.0, "the tritone reads a level with nothing on it");
    }

    /// A gate ABOVE what a node's loudest wedge reaches closes it, and one
    /// below opens it — the bar doing the one thing it is for, on one class.
    ///
    /// Off a partial at half level, so both directions are a real setting
    /// rather than one of them being the empty grid.
    #[test]
    fn the_bar_decides_a_node_either_way() {
        let wheel = wheel();
        let mut levels = partial(60.0, 10.0);
        for level in levels.iter_mut() {
            *level /= 2;
        }
        let peak = gate_of(&gated(0.0, SpectralReading::Fold), levels.clone()).peak(&wheel, 0.0);
        assert!((0.1..0.9).contains(&peak), "the fixture's partial reads {peak}, not mid-scale");
        assert!(
            gate_of(&gated(peak - 0.05, SpectralReading::Fold), levels.clone()).draws(&wheel, 0.0),
            "a gate under the node's own peak closed it",
        );
        assert!(
            !gate_of(&gated(peak + 0.05, SpectralReading::Fold), levels).draws(&wheel, 0.0),
            "a gate over the node's own peak left it open",
        );
    }

    /// The gate at its floor is the UNGATED picture: every node rings, silence
    /// included.
    ///
    /// The claim that keeps the bar's low end a setting rather than a corner —
    /// a silent ring is a reading (nothing sounds here), and going back to a
    /// lattice of them has to be one drag away. `>=` is what makes it true, and
    /// `>` is the version that passes every other test here while leaving no
    /// way to ask for the old picture.
    #[test]
    fn the_gates_floor_rings_every_node() {
        let wheel = wheel();
        let silent = gate_of(&gated(SPECTRAL_GATE_MIN, SpectralReading::Fold), empty());
        for cents in [0.0, 100.0, 386.0, 600.0, 1100.0] {
            assert!(silent.draws(&wheel, cents), "a gate at its floor held back {cents}¢");
        }
    }

    /// A grid with nothing measured into it.
    fn empty() -> Box<SpectralLevels> {
        Box::new([0u8; SPECTRUM_BINS])
    }

    /// The two readings answer the gate the way they DRAW, which is the one
    /// place the reading reaches this at all: a fold wedge is one level at its
    /// octave's own pitch, and a spectrum wedge is the loudest thing in the
    /// window it spreads across its arc.
    ///
    /// So a partial 60¢ off a node — well inside the fresh 200¢ window and well
    /// outside anything the node's own pitch reads — opens that node under
    /// Spectrum and leaves it dark under Fold. Both are correct, because both
    /// are what is on the screen: the spectrum wedge is showing that partial,
    /// off-centre in its arc, and the fold wedge is not.
    ///
    /// A gate that read one grid for both would fail this in whichever
    /// direction it chose: hiding a node whose wedge is plainly lit, or ringing
    /// a node whose wedge is plainly empty.
    #[test]
    fn each_reading_is_gated_on_what_its_own_wedge_shows() {
        let wheel = wheel();
        let off = partial(60.6, 10.0);
        let fold = gate_of(&gated(0.5, SpectralReading::Fold), off.clone());
        let spectrum = gate_of(&gated(0.5, SpectralReading::Spectrum), off);
        assert!(
            !fold.draws(&wheel, 0.0),
            "the fold rang a node whose own pitch reads {}",
            fold.peak(&wheel, 0.0),
        );
        assert!(spectrum.draws(&wheel, 0.0), "the spectrum missed a partial inside its window");
        // ...and the window has an edge where the Zoom bar puts it: the same
        // partial, read at a window narrow enough not to reach it, closes the
        // node again. Otherwise this passes just as well for a gate that
        // searched the whole grid.
        let narrow = ViewConfig {
            spectral_ring_range: 40.0,
            ..gated(0.5, SpectralReading::Spectrum)
        };
        assert!(
            !gate_of(&narrow, partial(60.6, 10.0)).draws(&wheel, 0.0),
            "a partial 60¢ off opened a node whose wedge shows 20¢ either side",
        );
    }

    /// The window reduction reaches exactly as far as a wedge does and no
    /// further: an isolated bucket spreads `half` buckets either side of itself.
    ///
    /// The arithmetic under the claim above, pinned on its own because the
    /// failure is a quiet one — a window an octave wide answers "something is
    /// sounding" for nearly every node in dense material, and reads as a gate
    /// that does not bite rather than as a wrong window.
    #[test]
    fn the_window_reaches_exactly_as_far_as_a_wedge_shows() {
        let mut levels = Box::new([0u8; SPECTRUM_BINS]);
        let lit = 2000;
        levels[lit] = 255;
        for half in [1usize, 8, 32, 192] {
            let peaks = window_max(&levels, half);
            assert_eq!(peaks[lit], 255, "the lit bucket lost its own level at half {half}");
            assert_eq!(peaks[lit - half], 255, "the window falls short below at half {half}");
            assert_eq!(peaks[lit + half], 255, "the window falls short above at half {half}");
            assert_eq!(peaks[lit - half - 1], 0, "the window reaches too far below at half {half}");
            assert_eq!(peaks[lit + half + 1], 0, "the window reaches too far above at half {half}");
        }
        // The fold's window is no window at all — a wedge is one reading at one
        // pitch — so the grid comes through bucket for bucket.
        assert_eq!(*window_max(&levels, 0), *levels, "the fold's grid was widened");
    }

    /// A gate no bar can produce — a hand-edited NaN, an infinity, a level off
    /// either end — reaches the picture as a setting somebody can see, and a
    /// NaN reaches it as the gate OFF.
    ///
    /// The direction matters and is the reason this is its own test: every
    /// other hand-edited value in this module is repaired toward the fresh
    /// look, and this one is repaired toward drawing MORE. A gate is the one
    /// setting here that can empty the lattice, and a blob nobody can read must
    /// not be able to do that with nothing on screen to say why.
    #[test]
    fn a_hand_edited_gate_never_empties_the_lattice() {
        let wheel = wheel();
        for gate in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -2.0, 7.0] {
            let view = gated(gate, SpectralReading::Fold);
            let paint = SpectralPaint::new(&view, Gradient::default());
            assert!(
                (SPECTRAL_GATE_MIN..=SPECTRAL_GATE_MAX).contains(&paint.gate),
                "a gate of {gate} reached the picture as {}",
                paint.gate,
            );
            if !gate.is_finite() {
                assert!(
                    gate_of(&view, empty()).draws(&wheel, 0.0),
                    "a gate of {gate} held back a node",
                );
            }
        }
    }

    /// A bucket stands for its own centre, and the grid covers the axis the
    /// analyzer claims with no bucket falling outside it.
    #[test]
    fn the_grid_is_centred_on_the_axis_it_spans() {
        assert!(bucket_pitch(0) > SPECTRAL_AXIS.0, "the first bucket sits below the axis");
        assert!(
            bucket_pitch(SPECTRUM_BINS - 1) > SPECTRAL_AXIS.1 - 1.0,
            "the grid stops {} short of the top of the axis",
            SPECTRAL_AXIS.1 - bucket_pitch(SPECTRUM_BINS - 1),
        );
        // One semitone is exactly BINS_PER_SEMITONE buckets apart, which is
        // what makes a cents offset a bucket offset at any pitch.
        let step = bucket_pitch(BINS_PER_SEMITONE) - bucket_pitch(0);
        assert!((step - 1.0).abs() < 1e-4, "a semitone spans {step} of the grid, not 1");
    }
}
