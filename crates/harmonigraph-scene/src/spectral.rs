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
//! grounds. The analyzer's panes are bedded on BLACK — a spectrogram cell at
//! silence has to be black or the plane's edge shows — so there the light and
//! the colour are the same thing and the picture is the gradient itself. The
//! ring is bedded on the LATTICE, so it reads a copy of that gradient whose
//! silent end is anchored on the node's own ground ([`ring_gradient`]): a ramp
//! opening at black punches a hole through a grey lattice at every node, which
//! is a picture of a gap where the table means silence. That ground is the
//! neutral grey of [`ViewConfig::lattice_ground`] — one number the octave band
//! beside it stands on too, so the node's two rings read as empty in exactly
//! one colour, and one bar moves both.
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
//! A ring comes and goes on the note Fade rather than at the instant the gate's
//! answer changes ([`RingFade`]), and a node the KEYS have lit wears its ring
//! for as long as they light it, whatever the gate says
//! ([`Scene::wear_audio_rings`](crate::Scene)). Those are one rule read from
//! two ends: the ring is a layer of a node, and every other layer of a node
//! arrives and leaves on that one duration.
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
use harmonigraph_core::Envelope;

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
/// the test is a `>=` ([`RingGate::opens`]), so 0 admits every node including
/// one whose ring is silent through and through. That is the picture with no gate at all, and
/// it has to be reachable — a ring at the ramp's floor is a reading (nothing
/// sounds here), and whether it is worth the screen it takes is exactly what
/// this bar is for.
pub const SPECTRAL_GATE_MIN: f32 = 0.0;
/// See [`SPECTRAL_GATE_MIN`]. The ceiling is a full-scale reading — the top of
/// the Level window, where nothing short of the loudest thing the analyzer can
/// report opens a ring.
pub const SPECTRAL_GATE_MAX: f32 = 1.0;

/// The widest hysteresis band the gate offers: a quarter of the Level window.
///
/// The ceiling is what the band MEANS rather than arithmetic. It is the drop
/// from the gate to the threshold a bucket already open is held by, so at a
/// quarter of the window a ring that opened at the fresh 0.30 stays lit down to
/// 0.05 — nearly to the floor, and about as far as "this node is still
/// sounding" can be stretched before it reads as "this node once sounded".
///
/// The band is allowed to reach the gate and past it, and what happens there
/// is [`RingGate::opens`]: the held threshold stops one step of the grid above
/// silence rather than at it, so a band this wide holds a ring until its bucket
/// reads nothing at all. Only the gate's own floor gives back the ungated
/// picture.
pub const SPECTRAL_HYSTERESIS_MAX: f32 = 0.25;

/// The longest attack or release the ring's reading offers, in seconds.
///
/// Two seconds is well past useful and is a guard rail rather than a setting:
/// what the bar is for lives under a third of a second, and the room above that
/// is there so a slow smear is reachable deliberately rather than fenced off.
pub const SPECTRAL_BALLISTICS_MAX: f32 = 2.0;

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
    /// wedge is not empty, it is the ramp's silent end — the node's own ground,
    /// which the ring's ramp is anchored on (see [`ring_gradient`]). A stretch
    /// of spectrum with nothing in it is a reading, not a gap. Whether a node
    /// with nothing but that to show is worth its annulus is the gate's question
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
    /// screen at all — the ring costs its annulus at every node in the window
    /// whatever it reads, and the cost of carrying it at a node with nothing in
    /// it is the picture around it.
    ///
    /// A THRESHOLD, then, and its answer is still a yes or a no; what is smooth
    /// is how fast a ring follows it. [`RingFade`] carries each answer on the
    /// note Fade, so a level breathing across this bar is a ring dimming and
    /// coming back rather than one flickering, and a level that stays across is
    /// a ring that goes.
    ///
    /// [`SpectralPaint`] carries it so that the decision and the levels it is
    /// made against are read from one place; [`RingGate`] is where it is
    /// answered per node.
    pub gate: f32,
    /// How far [`gate`](Self::gate) drops for a bucket already open
    /// ([`ViewConfig::spectral_ring_hysteresis`]), already clamped. Carried
    /// beside the gate because the two are one decision and a threshold read
    /// from a different frame than its partner is a decision nobody made.
    pub hysteresis: f32,
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
    /// ramp's silent end — the neutral grey of [`ViewConfig::lattice_ground`],
    /// which that end is pinned onto — everywhere rather than as an empty
    /// annulus: the ring is a MEASUREMENT of a range, and a range with nothing
    /// in it is a reading, not a gap.
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
            hysteresis: 0.0,
            levels: Box::new([0; SPECTRUM_BINS]),
        }
    }

    /// The paint `view` asks for, its table baked from the analyzer's
    /// `gradient` anchored on the node's own ground, with no levels measured
    /// into it yet.
    ///
    /// The GRADIENT and not the analyzer's baked table, because the anchoring
    /// is a move of the ramp's two ENDS ([`ring_gradient`]) rather than a blend
    /// over the entries it produced: the knobs are what carries it, so this is
    /// the one place they are read for the ring and the one place its table is
    /// built. The ground the ends are moved onto comes off `view`, the same
    /// field the octave band's own ground does.
    ///
    /// Where the ring sits is [`ViewConfig::rings`]'s answer rather than a
    /// second reading of the same bars: it is the innermost layer of the stack,
    /// so its inner edge is wherever the stack starts
    /// ([`ViewConfig::ring_inner`]), and the same one answer is
    /// what places the band a gap outside it. That sum belongs in one place, and
    /// its clamps are there rather than in `ViewConfig::sanitize` for the
    /// reason every other geometry clamp is — the drawing code is reached by
    /// more routes than the persist door (a take replay, the offline
    /// renderer's layout, a standalone harness), and a hand-edited view must
    /// still come out as a node somebody can see.
    pub fn new(view: &ViewConfig, gradient: Gradient) -> SpectralPaint {
        let (inner, outer) = view.rings().audio;
        SpectralPaint {
            lut: pitch_ramp_lut(ring_gradient(gradient, view.lattice_ground_lightness())),
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
            // Repaired to 0 — one threshold — for the reason above it: a band
            // nobody can read falls back to the simpler rule.
            hysteresis: clamp_or(view.spectral_ring_hysteresis, 0.0, 0.0, SPECTRAL_HYSTERESIS_MAX),
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
    /// How far the gate drops for a bucket already open — the second threshold
    /// of a Schmitt trigger, carried with its pair for the same reason.
    hysteresis: f32,
}

impl RingGate {
    /// The gate `paint` asks for, with its grid reduced to what each wedge of
    /// the ring actually reaches.
    pub fn new(paint: &SpectralPaint) -> RingGate {
        // Half a wedge's window, in buckets: the fold reads ONE pitch per wedge
        // and so has no window at all, and the raw spectrum spreads `range`
        // cents across the arc, centred on the wedge's own pitch.
        let half = if paint.folded { 0 } else { window_half(paint.range) };
        RingGate {
            peaks: window_max(&paint.levels, half),
            gate: paint.gate,
            hysteresis: paint.hysteresis,
        }
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

    /// Whether the ring on a node of pitch class `cents` draws at all: at
    /// least one of its wedges reaching the gate.
    ///
    /// Asked through a SETTLED [`RingFade`] rather than of [`peak`](Self::peak)
    /// directly, because the gate is answered per BUCKET
    /// ([`opens`](Self::opens)) and a node then reads that grid at each of its
    /// wedges. A node-level `peak >= gate` is the same answer wherever a wedge
    /// falls between two buckets that disagree, and the picture's is not — see
    /// [`RingFade`].
    ///
    /// What it answers is the gate with NO history: a settled fade has nothing
    /// recorded in [`gated`](RingFade::gated), so every bucket is asked the
    /// strict threshold and the band never applies. That is the picture a first
    /// frame draws, and it is the ONLY frame it matches once the hysteresis is
    /// off its floor — a running lattice reads [`RingFade`] carried in
    /// `SharedState`, which is where the band lives.
    ///
    /// `>=` inside `opens`, so that a gate at [`SPECTRAL_GATE_MIN`] admits
    /// every node — the floor is the bar's off position and has to give back
    /// the ungated picture, silent rings and all.
    pub fn draws(&self, layout: &OctaveLayout, cents: f32) -> bool {
        RingFade::settled(self).level(layout, cents) > 0.0
    }

    /// Whether a wedge reading bucket `bucket` reaches the gate — the same
    /// question [`draws`](Self::draws) asks of a whole node, asked of one
    /// bucket of the grid so that [`RingFade`] can carry the answer across
    /// frames.
    ///
    /// Read at the bucket's own centre, which is where a wedge sitting on it
    /// reads: [`level_at`] interpolates between the two buckets a pitch falls
    /// between, and at a centre that blend is the bucket itself.
    ///
    /// `held` is whether this bucket is ALREADY open, which is what makes the
    /// pair of thresholds a Schmitt trigger rather than two unrelated gates: an
    /// open bucket is asked the lower one, so a level wobbling inside the band
    /// keeps whatever answer it already had, and only a move clear of the band
    /// changes it. It is this gate's OWN previous answer
    /// ([`RingFade::gated`]) and not how far the ring has faded — the band is a
    /// question about the measurement, and anything else in it makes the Fade
    /// decide when the band applies.
    fn opens(&self, bucket: usize, held: bool) -> bool {
        // The band may not reach silence while the GATE stands above it. A held
        // threshold of zero is met by an empty bucket, so a band as wide as the
        // gate would hold every bucket that ever opened for the life of the
        // session — a latch rather than a trigger, and a lattice that collects
        // rings instead of showing what is sounding. Floored instead at one
        // step of the grid, which is a `u8`: a held bucket keeps its ring while
        // it reads ANYTHING, and gives it back on silence.
        //
        // `self.gate.min(..)` and not the step alone, so the bar's own off
        // position is untouched: at a gate of `SPECTRAL_GATE_MIN` the floor is
        // zero, silence still opens every bucket, and that picture — the
        // analyzer's whole reading at once — is the one thing the floor is for.
        const GRID_STEP: f32 = 1.0 / 255.0;
        let gate = if held {
            (self.gate - self.hysteresis).max(self.gate.min(GRID_STEP))
        } else {
            self.gate
        };
        f32::from(self.peaks[bucket]) / 255.0 >= gate
    }
}

/// How far open the gate stands at every bucket of the analyzer's grid, carried
/// across frames so that a ring ARRIVES and LEAVES on the Note section's Fade
/// rather than switching on and off with the spectrum.
///
/// The gate is a threshold and a threshold flickers: what it is asked of is a
/// live measurement, so a partial breathing across the level the view names
/// takes its node's whole annulus with it every time it crosses. One Fade under
/// the decision makes that a ring dimming and coming back, which is the same
/// reading drawn at a speed an eye can follow — and it is the Fade rather than
/// a smoothing of its own because the ring is a LAYER OF A NODE: the node's
/// disc, its band and its marks all leave on that one duration, so a release
/// reads as a single gesture instead of the ring snapping off part way through
/// it.
///
/// Kept per BUCKET and not per node, for the reason the reading itself is one
/// grid: which nodes are in view is the camera's business and changes as it
/// pans, where the spectrum is the same measurement wherever the lattice is
/// looked at from. A node's own level is then a read of this grid at each of
/// its wedges, exactly as [`RingGate::peak`] is.
///
/// What that costs is stated rather than hidden: a wedge landing BETWEEN an
/// open bucket and a closed one wears a FRACTION of its ring, where the gate's
/// own answer is a yes or a no. It is a steady fraction and not a transition —
/// both buckets are at their targets, so [`advance`](Self::advance) skips them
/// and the level stands for as long as the spectrum does — and the band it
/// happens in is the whole 3.125¢ between the two centres, every pitch strictly
/// between them included.
///
/// So the gate's edge is soft across one bucket, and that is the picture worth
/// having: the alternative is [`open_at`] snapping to the nearer centre, which
/// buys a hard yes-or-no per node at the price of moving the edge by up to half
/// a bucket and putting the whole of a node's annulus on which side of a centre
/// its wedge fell. The ring's own reading is interpolated between buckets
/// ([`level_at`], the shader's `spectrum_at`), so a gate that stepped where the
/// reading ramps would take the ring off a node whose wedge is visibly still
/// showing the partial that opened it.
pub struct RingFade {
    /// How much of its ring a wedge reading each bucket wears, 0..=1 — the
    /// gate's own answer once it has settled, and part way between while a
    /// transition is running.
    open: Box<[f32; SPECTRUM_BINS]>,
    /// What the gate last ANSWERED at each bucket, which is what the band is
    /// asked against.
    ///
    /// Kept rather than read off the level above, and the two are not the same
    /// question: a level part way up says how long ago the gate turned over,
    /// where this says which way. Deriving one from the other puts the Fade's
    /// DURATION in charge of when the band engages — a bucket would have to
    /// stand above the strict threshold for a share of the Fade before the
    /// lower one applied at all, so the same gate and the same band would
    /// select different rings at different Fades
    /// (`the_band_holds_a_ring_at_any_fade`).
    gated: Box<[bool; SPECTRUM_BINS]>,
    /// The clock the levels above stand at, or `None` before the first step.
    ///
    /// A moment and not a duration, so that a pane drawn TWICE in one frame —
    /// the docked lattice and the Video tab's preview, off one clock — steps
    /// the fade once: the second call's `dt` is zero and every bucket answers
    /// where it already was.
    at: Option<f64>,
}

impl Default for RingFade {
    fn default() -> RingFade {
        RingFade {
            open: Box::new([0.0; SPECTRUM_BINS]),
            gated: Box::new([false; SPECTRUM_BINS]),
            at: None,
        }
    }
}

impl RingFade {
    /// `gate`'s answer with no transition anywhere in it — the first step of
    /// all, which settles rather than fading in.
    ///
    /// The envelope cannot reach it, which is what makes this the gate's own
    /// picture and not a fade of one: with no clock behind it every bucket
    /// takes its target outright.
    fn settled(gate: &RingGate) -> RingFade {
        let mut fade = RingFade::default();
        fade.advance(gate, &Envelope { attack_time: 0.0, fade_time: 0.0, shape: 0.0 }, 0.0);
        fade
    }

    /// Carry every bucket toward what `gate` says of it now, on `env`.
    ///
    /// The FIRST step of all settles rather than fading in, and so does one
    /// after a jump in the clock: there is no transition to draw when nothing
    /// was on screen to transition from, and a lattice whose ring layer has
    /// just been switched on should show what the analyzer is saying rather
    /// than a fade-in from a picture nobody saw. (A jump lands the same way
    /// through the arithmetic rather than through a case of its own — a `dt`
    /// past the Fade runs the transition out.)
    ///
    /// Buckets already at their target are skipped, which is nearly all of
    /// them nearly always: the grid is 3828 buckets and what is moving through
    /// a transition at any moment is the edge of whatever is sounding.
    ///
    /// The band is asked against [`gated`](Self::gated) — what the gate itself
    /// said last time — so a bucket mid-transition is on the same threshold it
    /// would be at rest. That answer is recorded for every bucket, including
    /// the ones whose level is already where it is going: the gate can turn
    /// over while nothing on screen moves, and a bucket skipped below still has
    /// to be asked.
    pub fn advance(&mut self, gate: &RingGate, env: &Envelope, now: f64) {
        let dt = self.at.map(|at| now - at);
        self.at = Some(now);
        for (bucket, (level, gated)) in self.open.iter_mut().zip(self.gated.iter_mut()).enumerate()
        {
            let open = gate.opens(bucket, *gated);
            *gated = open;
            let target = if open { 1.0 } else { 0.0 };
            if *level == target {
                continue;
            }
            *level = match dt {
                Some(dt) => env.carried(*level, dt, open),
                None => target,
            };
        }
    }

    /// How much of its ring a node of pitch class `cents` wears, 0..=1: the
    /// most open any of its wedges stands.
    ///
    /// The shape of [`RingGate::peak`] and for the same reasons — every slot
    /// the wheel DRAWS and no others, read through the same grid arithmetic the
    /// shader uses — so that what a node wears follows what its own wedges
    /// show.
    pub fn level(&self, layout: &OctaveLayout, cents: f32) -> f32 {
        let (low, high) = layout.slots(cents);
        (low..=high)
            .map(|slot| open_at(&self.open, layout.slot_pitch(slot, cents)))
            .fold(0.0, f32::max)
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
    let Some((bucket, next, across)) = grid_at(pitch) else {
        return 0.0;
    };
    let (low, high) = (f32::from(grid[bucket]) / 255.0, f32::from(grid[next]) / 255.0);
    low + (high - low) * across
}

/// [`RingFade`]'s reading of its own grid, which holds a level per bucket
/// already rather than a byte — the same walk as [`level_at`], so a node's ring
/// comes and goes at the pitch its wedges are gated at.
fn open_at(grid: &[f32; SPECTRUM_BINS], pitch: f32) -> f32 {
    let Some((bucket, next, across)) = grid_at(pitch) else {
        return 0.0;
    };
    grid[bucket] + (grid[next] - grid[bucket]) * across
}

/// The two buckets `pitch` falls between and how far across the pair it sits,
/// or `None` where the analyzer's axis does not reach it.
///
/// One definition of where a pitch lands on the grid, because three readers
/// have to agree on it — the gate's levels, the fade's own openness, and the
/// shader's `spectrum_at`, which is the one that draws. The half bucket is the
/// grid's own convention ([`bucket_pitch`]): a bucket stands for its CENTRE,
/// and dropping it puts every partial half a bucket flat of where it sounds.
fn grid_at(pitch: f32) -> Option<(usize, usize, f32)> {
    let x = (pitch - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32 - 0.5;
    if x < 0.0 || x > (SPECTRUM_BINS - 1) as f32 {
        return None;
    }
    let bucket = x as usize;
    Some((bucket, (bucket + 1).min(SPECTRUM_BINS - 1), x - bucket as f32))
}

/// The analyzer's gradient as the RING reads it: the same arc, with its silent
/// end moved onto the ground the ring is drawn on — `ground` as an `L*`, and no
/// chroma at all.
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
/// A move of the ramp's LIGHTNESS ends and not a blend over the baked table,
/// because the ground says where the ramp STARTS rather than anything about the
/// colours along it. Screening the analyzer's colours over the ground instead
/// lifts every channel by `ground * (1 - c)`, which washes the middle of the
/// ramp toward the ground's own grey and never lets go at the top: that lift
/// falls to nothing only at white, so a loud wedge and a loud spectrogram cell
/// would be two colours that nearly agree rather than one. Moving the end is
/// EXACT there instead — the `t` = 1 entry is the analyzer's own to the byte,
/// which `a_loud_reading_is_the_analyzers_own_colour_exactly` holds — and the
/// hue knob is untouched at every level.
///
/// **Both ends of the CHROMA ramp move too, and that is what makes the ground a
/// ground**: the silent end is pinned to chroma 0, so what a quiet wedge draws
/// is the neutral grey of [`ViewConfig::lattice_ground`] and not the analyzer
/// gradient's own dark hue at that brightness. The MIDI ring beside it stands
/// on that same grey, and two rings a gap apart whose empty state differed by a
/// tint would read as two measurements in different units when the only
/// difference is which one is empty. The cost is real and is stated rather than
/// hidden: the ramp opens neutral, so a wedge in the low mids carries less
/// colour than the spectrogram cell of the same loudness — most at chroma ramp
/// 0, where the analyzer's own table is flat in colour and the ring's is not.
/// The loud end is untouched at every setting, which is where the two pictures
/// are actually compared.
///
/// A PIN and not a floor, on both axes. The ground is where the ring starts
/// rather than merely what it must not sink under: a gradient whose own dark
/// end sits above it would otherwise draw a silent wedge in a colour that is
/// not the ground, and the two rings would agree only for some settings of a
/// gradient that knows nothing about either of them.
///
/// Each pair is stored as a middle plus a SIGNED ramp, so the ends are
/// recomposed rather than set, and `t` = 0 is the bright end wherever a ramp
/// runs downward — the pin lands on whichever end `t` = 0 carries, since that
/// is the end a silent wedge draws. Both signs are held:
/// `a_ramp_running_downward_is_pinned_at_its_bright_end` on the lightness pair,
/// and `a_chroma_ramp_that_runs_out_leaves_the_ring_grey` on the chroma pair,
/// where a fall steep enough to reach 0 at the loud end leaves the ring with no
/// colour anywhere — every drop of it was at the silent end the ground claims.
pub fn ring_gradient(gradient: Gradient, ground: f32) -> Gradient {
    // Sanitized first, so the ends are read off the gradient that will actually
    // be drawn — and so a gradient already standing on this ground comes back
    // as the very key `pitch_ramp_lut` would memoize it under.
    let gradient = gradient.sanitized();
    let ground = ground.clamp(0.0, 100.0);
    // Recomposed off the LOUD end: it is the end that must not move, and for a
    // top at or above the ground the subtraction is exact in f32 (Sterbenz), so
    // `lightness ± ramp/2` lands the `t` = 1 end back on exactly the lightness
    // it had and spends the rounding at the end being moved anyway.
    let top = gradient.lightness + gradient.lightness_ramp * 0.5;
    let lightness = (ground + top) * 0.5;
    let top_chroma = gradient.chroma + gradient.chroma_ramp * 0.5;
    let chroma = top_chroma * 0.5;
    Gradient {
        lightness,
        lightness_ramp: (top - lightness) * 2.0,
        chroma,
        // The full ramp its own middle leaves room for, which is exactly what a
        // silent end of 0 asks for — so this reaches `sanitized` at its bound
        // rather than past it, and comes back unflattened.
        chroma_ramp: top_chroma,
        ..gradient
    }
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
            // The stack seated on the node's own centre, so the ring's radii
            // are its width and nothing else: what these cases are about is the
            // width reaching the picture, and a start inherited from a dialled-
            // in look would put the widest of them off the quad and answer with
            // a refusal instead. Where the ring seats when the stack does not
            // start there is `the_ring_seats_on_the_stacks_start`.
            ring_inner: 0.0,
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
            let view = ringed(width, range);
            let paint = SpectralPaint::new(&view, Gradient::default());
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

    /// The ring sits where the STACK puts it: on the stack's own start, it
    /// being the innermost layer, and sized by its own width alone.
    ///
    /// The whole of what the width bars buy, checked at the one place a second
    /// copy of the sum would drift — the audio channel is built from
    /// `ViewConfig` on its own, without the scene the octave band comes out of.
    /// No gap is spent in front of it: there is no layer inside it to stand
    /// off, and a padding around nothing is a hole the picture cannot explain.
    #[test]
    fn the_ring_seats_on_the_stacks_start() {
        let view = ViewConfig { ring_inner: 0.2, ..ringed(0.25, 200.0) };
        let paint = SpectralPaint::new(&view, Gradient::default());
        assert_eq!((paint.inner, paint.outer), (0.2, 0.45));

        // And the gap it does not spend is the band's, a gap out from where
        // this ring ended.
        assert_eq!(view.rings().band.0, 0.45 + view.ring_gap);

        // A start of 0 is the stack seated on the node's own centre, where the
        // ring's wedges close into pie slices. Still no gap in front of it —
        // the rule is one rule, and the centre is not a special case of it.
        let view = ringed(0.25, 200.0);
        let paint = SpectralPaint::new(&view, Gradient::default());
        assert_eq!((paint.inner, paint.outer), (0.0, 0.25));
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
    /// black. Carries chroma at both ends, which is what makes the ring's
    /// silent end a question rather than a shade of grey by default.
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
    /// the ramp is authored in, and the only ones a claim about the ground
    /// means anything in.
    fn lightness(c: Vec4) -> f32 {
        crate::color::lightness_of_encoded(f64::from(c.x), f64::from(c.y), f64::from(c.z)) as f32
    }

    /// A silent wedge draws the GROUND: the neutral grey
    /// [`ViewConfig::lattice_ground`] names, to the byte.
    ///
    /// The whole claim of the pin, and it is stated against the octave band's
    /// own ground rather than against a number written here — the two rings sit
    /// a gap apart on one node and the ask is that their empty state is one
    /// colour, so what this holds is the two ends of that, not a shade.
    ///
    /// A silent wedge draws it deliberately — a reading, not a gap — so most of
    /// the ring is this entry most of the time. Unanchored (the analyzer's own
    /// table, whose bottom is black for the spectrogram's black plane) it is a
    /// hole punched through the lattice at every node in the window.
    #[test]
    fn a_silent_wedge_is_the_grey_the_octave_band_stands_on() {
        for ground in [0.0, 8.8, 20.0, 55.0, 100.0] {
            let view = ViewConfig { lattice_ground: ground, ..ringed(0.3, 200.0) };
            let silent = SpectralPaint::new(&view, analyzers()).lut[0];
            let band = crate::grey_of_lightness(view.lattice_ground_lightness());
            let step = (silent.truncate() - band.truncate()).abs().max_element();
            assert!(
                step * 255.0 < 0.5,
                "at Ground {ground} the audio ring's silence is {silent:?} \
                 and the octave band's is {band:?} — over half a byte apart",
            );
            assert_eq!(silent.w, 1.0, "the ring's ramp lost its alpha");
        }
    }

    /// The fresh Ground is the skin's own `surface_faint` rung, to under a fifth
    /// of an `L*`.
    ///
    /// The one thing tying the default to the chrome now that the ring reads a
    /// number instead of the skin: with the tie in code the two moved together
    /// and could not be dialled apart, and with no tie at all retuning the
    /// chrome's ladder would leave the lattice standing on the rung the ladder
    /// used to have. This is the tie a bar can still be dragged off.
    #[test]
    fn the_fresh_ground_is_the_skins_faint_surface() {
        let rung = lightness(crate::skin::surface_faint_color());
        let fresh = ViewConfig::default().lattice_ground;
        assert!(
            (fresh - rung).abs() < 0.2,
            "the fresh Ground is L* {fresh}, the skin's faint surface L* {rung}",
        );
    }

    /// A gradient whose own dark end ALREADY sits above the ground is still
    /// moved onto it.
    ///
    /// A pin and not a floor, and this is the difference. The ground is where
    /// the ring starts rather than what it must not sink under: left to open
    /// bright, a silent wedge would draw a colour that is not the ground, and
    /// the two rings would agree only for some settings of a gradient that
    /// knows about neither of them. A floor would pass every other test here.
    #[test]
    fn a_ramp_opening_above_the_ground_is_still_moved_onto_it() {
        let high = Gradient { lightness: 60.0, lightness_ramp: 20.0, ..analyzers() };
        let ground = ViewConfig::default().lattice_ground;
        assert!(
            lightness(pitch_ramp_lut(high)[0]) > ground,
            "the test's own gradient does not open above the ground",
        );
        let opened = lightness(pitch_ramp_lut(ring_gradient(high, ground))[0]);
        assert!(
            (opened - ground).abs() < 0.2,
            "a ramp opening bright was left at L* {opened} rather than the ground's {ground}",
        );
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
        partial_at(pitch, half, 255)
    }

    /// [`partial`] at a stated level rather than full scale — for claims about
    /// where a reading sits against the GATE, where full scale is on the wrong
    /// side of every threshold there is.
    fn partial_at(pitch: f32, half: f32, level: u8) -> Box<SpectralLevels> {
        let mut levels = Box::new([0u8; SPECTRUM_BINS]);
        for (bucket, bucket_level) in levels.iter_mut().enumerate() {
            if ((bucket_pitch(bucket) - pitch) * 100.0).abs() <= half {
                *bucket_level = level;
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
        ViewConfig { spectral_ring_gate: gate, spectral_reading: reading, ..ViewConfig::default() }
    }

    /// The band's whole purpose: a level between the two thresholds keeps
    /// whatever answer it already had, so a reading wobbling on the gate stops
    /// switching. A gate of 0.4 with a 0.1 band opens at 0.4 and closes under
    /// 0.3, and 0.35 is the level that reads both ways depending on where it
    /// came from.
    #[test]
    fn a_held_bucket_keeps_its_ring_until_the_level_clears_the_band() {
        let bucket = 1000usize;
        let mut levels = empty();
        // 0.35 of the window: over the closing threshold, under the opening one.
        levels[bucket] = 89;
        let view =
            ViewConfig { spectral_ring_hysteresis: 0.1, ..gated(0.4, SpectralReading::Fold) };
        let gate = gate_of(&view, levels);
        assert!(!gate.opens(bucket, false), "0.35 is under the gate and must not OPEN a ring");
        assert!(gate.opens(bucket, true), "0.35 is inside the band and must HOLD one");

        // Clear of the band below: shut either way.
        let mut levels = empty();
        levels[bucket] = 70; // 0.275
        let gate = gate_of(&view, levels);
        assert!(!gate.opens(bucket, true), "a level under the band closes a held ring");
    }

    /// 0 is one threshold, which is the bar's off position and has to give back
    /// the picture with no hysteresis in it — the same answer whether or not the
    /// bucket is already lit.
    #[test]
    fn a_band_of_nothing_is_one_threshold() {
        let view =
            ViewConfig { spectral_ring_hysteresis: 0.0, ..gated(0.4, SpectralReading::Fold) };
        for byte in [0u8, 70, 89, 102, 128, 255] {
            let mut levels = empty();
            levels[1000] = byte;
            let gate = gate_of(&view, levels);
            assert_eq!(
                gate.opens(1000, true),
                gate.opens(1000, false),
                "level {byte} answers differently held and not, at a band of 0",
            );
        }
    }

    /// The floor stays the off position however wide the band is: at a gate of
    /// [`SPECTRAL_GATE_MIN`] every bucket opens, and a band reaching under zero
    /// cannot make that more true — nor may it turn into a negative threshold
    /// that some arithmetic elsewhere reads as a level.
    #[test]
    fn the_band_never_reaches_under_the_gates_floor() {
        let view = ViewConfig {
            spectral_ring_hysteresis: SPECTRAL_HYSTERESIS_MAX,
            ..gated(SPECTRAL_GATE_MIN, SpectralReading::Fold)
        };
        let gate = gate_of(&view, empty());
        assert!(gate.opens(1000, true), "silence at the gate's floor still rings");
        assert!(gate.opens(1000, false), "and rings whether or not it is held");
    }

    /// A band that reaches the gate holds a ring until its bucket falls
    /// silent — it does not hold it for the life of the session.
    ///
    /// The trap this is against: subtracting the band from the gate and
    /// flooring the RESULT at zero makes the held threshold exactly the bar's
    /// off position whenever the band reaches the gate, and an empty bucket
    /// meets that. The Schmitt trigger becomes a LATCH — every bucket that
    /// ever opened stays open through silence — so the lattice fills with
    /// rings monotonically as a piece plays and never gives one back.
    ///
    /// The floor itself is a different question and stays as it is: at a gate
    /// of [`SPECTRAL_GATE_MIN`] silence rings deliberately, which is
    /// [`the_band_never_reaches_under_the_gates_floor`]. What must not happen
    /// is a gate ABOVE its floor behaving like one on it.
    ///
    /// Run at gates under the fresh band of 0.0964, which is the stretch of
    /// the Gate bar a person reaches by dragging it down to see more of the
    /// reading.
    #[test]
    fn a_band_reaching_the_gate_releases_on_silence() {
        for gate in [0.02, 0.05, 0.09] {
            let view = gated(gate, SpectralReading::Fold);
            assert!(
                view.spectral_ring_hysteresis >= gate,
                "the test's own band of {} does not reach the gate of {gate}",
                view.spectral_ring_hysteresis,
            );

            let mut loud = empty();
            loud[1000] = 255;
            assert!(
                gate_of(&view, loud).opens(1000, false),
                "a full-scale bucket does not open the gate at {gate}",
            );
            assert!(
                !gate_of(&view, empty()).opens(1000, true),
                "a bucket held open at a gate of {gate} never releases on silence",
            );
        }
    }

    /// The band is a question about the GATE and the measurement and nothing
    /// else, so the SAME reading holds the same ring at every Fade.
    ///
    /// The trap it is against: the fade's own level is the tempting stand-in
    /// for "this bucket was open", and it is not one. A level part way up says
    /// how long ago the gate opened, not what the gate said — so reading the
    /// band off it puts the Note section's Fade, a duration chosen for MIDI
    /// transients, in charge of whether the band works at all. On the curve a
    /// half-open level is 28.7% of the way through the Fade at the fresh shape,
    /// which at the Fade's top of 1 s is 287 ms of continuous above-gate
    /// reading before the band engages, and 43 ms at the fresh 0.15 s.
    ///
    /// Run here at two Fades an order of magnitude apart, on one gate, one
    /// band and one pair of readings: a level clear of the gate, then a level
    /// inside the band, which must hold the ring both times.
    #[test]
    fn the_band_holds_a_ring_at_any_fade() {
        let wheel = wheel();
        let view =
            ViewConfig { spectral_ring_hysteresis: 0.1, ..gated(0.4, SpectralReading::Fold) };
        let silent = gate_of(&view, empty());
        // 0.502 of the window, clear above the gate; then 0.349, inside the
        // band — over the closing threshold of 0.3, under the opening 0.4.
        let over = gate_of(&view, partial_at(60.0, 10.0, 128));
        let inside = gate_of(&view, partial_at(60.0, 10.0, 89));

        for seconds in [0.1, 1.0] {
            let env = fade_of(seconds);
            let mut fade = RingFade::default();
            fade.advance(&silent, &env, 0.0);
            fade.advance(&over, &env, 0.2);
            let opening = fade.level(&wheel, 0.0);
            fade.advance(&inside, &env, 0.4);
            let held = fade.level(&wheel, 0.0);
            assert!(
                held >= opening,
                "at a Fade of {seconds}s a reading inside the band dropped the ring \
                 from {opening} to {held}",
            );
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
    /// a silent ring is a reading (nothing sounds here), and asking for a
    /// lattice of them has to be one drag away. `>=` is what makes it true, and
    /// `>` is the version that passes every other test here while leaving the
    /// ungated picture unreachable from the bar.
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
    /// So a partial 60¢ off a node — well inside a 200¢ window and well
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
        // A window stated rather than the fresh view's own: the claim is
        // about a partial well inside SOME window, not about how wide the
        // fresh Zoom happens to be dialled.
        let wide =
            ViewConfig { spectral_ring_range: 200.0, ..gated(0.5, SpectralReading::Spectrum) };
        let spectrum = gate_of(&wide, off);
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
        let narrow =
            ViewConfig { spectral_ring_range: 40.0, ..gated(0.5, SpectralReading::Spectrum) };
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

    /// The gate's edge is soft across one bucket, and it STAYS where the
    /// spectrum does: a wedge between an open bucket and a closed one wears a
    /// fraction of its ring, and that fraction is a steady state rather than a
    /// transition part way through.
    ///
    /// Worth pinning because it is the one place the picture and a node-level
    /// `peak >= gate` part company, and reading the code cannot tell them
    /// apart: both buckets sit at their targets, so the fade skips them, and
    /// what looks like a ring on its way in never moves again. A future
    /// `open_at` that snapped to the nearer centre would pass every other test
    /// here.
    #[test]
    fn the_gates_edge_is_a_steady_ramp_one_bucket_wide() {
        // A fold peak's shoulder: two neighbours straddling the gate.
        let (lo, hi) = (1000usize, 1001usize);
        let mut levels = empty();
        levels[lo] = 128;
        levels[hi] = 100;
        let view = gated(0.4, SpectralReading::Fold);
        let gate = gate_of(&view, levels);
        // Neither bucket held, so both are asked the strict threshold: what is
        // being measured here is the ramp BETWEEN two buckets, not the band a
        // held one keeps.
        assert!(
            gate.opens(lo, false) && !gate.opens(hi, false),
            "the fixture's buckets do not straddle 0.4"
        );

        // Ten seconds on a half-second Fade: twenty durations, nothing moving.
        let env = fade_of(0.5);
        let mut fade = RingFade::default();
        for step in 0..=10 {
            fade.advance(&gate, &env, f64::from(step));
        }
        let mid = (bucket_pitch(lo) + bucket_pitch(hi)) * 0.5;
        let held = open_at(&fade.open, mid);
        assert!(
            (held - 0.5).abs() < 1e-3,
            "a wedge half way between the two wears {held} once everything has settled",
        );
        // The band is the whole gap between the centres, not a sliver of it.
        let (mut low, mut high) = (f32::MAX, f32::MIN);
        for i in 0..=2000 {
            let pitch =
                bucket_pitch(lo) + (bucket_pitch(hi) - bucket_pitch(lo)) * (i as f32 / 2000.0);
            let open = open_at(&fade.open, pitch);
            if open > 0.001 && open < 0.999 {
                low = low.min(pitch);
                high = high.max(pitch);
            }
        }
        let band = (high - low) * 100.0;
        assert!(
            (band - CENTS_PER_BUCKET).abs() < 0.05,
            "the ramp spans {band}¢, where the buckets are {CENTS_PER_BUCKET}¢ apart",
        );
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

    /// ...and the door a saved blob actually comes through repairs it the same
    /// way. [`ViewConfig::sanitize`] runs before anything reads the view, so
    /// [`SpectralPaint::new`]'s repair above only ever sees a value already
    /// mended — which is what makes this its own test rather than a second
    /// reading of that one.
    ///
    /// Pointed at the OFF position and not at the fresh value the two settings
    /// either side of it take, which is the whole claim: a level nobody can
    /// read is a reason to draw every ring, never to hide one. Repairing to
    /// the fresh 0.4 instead passes every other test in the tree.
    #[test]
    fn the_blobs_own_door_repairs_a_gate_toward_drawing() {
        for gate in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -2.0, 7.0] {
            let mut view = gated(gate, SpectralReading::Fold);
            view.sanitize();
            assert!(
                (SPECTRAL_GATE_MIN..=SPECTRAL_GATE_MAX).contains(&view.spectral_ring_gate),
                "a saved gate of {gate} loaded as {}",
                view.spectral_ring_gate,
            );
            if !gate.is_finite() {
                assert_eq!(
                    view.spectral_ring_gate, SPECTRAL_GATE_MIN,
                    "a saved gate of {gate} loaded holding rings back",
                );
            }
        }
        // A value the bar can produce is not a value to repair.
        let mut view = gated(0.4, SpectralReading::Fold);
        view.sanitize();
        assert_eq!(view.spectral_ring_gate, 0.4, "a settable gate was mended anyway");
    }

    /// A straight-line envelope of a stated length, so half a duration in
    /// reads half way along and a claim can be made in fractions.
    fn fade_of(seconds: f32) -> Envelope {
        Envelope { attack_time: seconds, fade_time: seconds, shape: 0.0 }
    }

    /// A ring COMES and GOES on the Fade: a class the gate has just opened
    /// wears part of its ring part way through the duration and all of it at
    /// the end, and the same in reverse once the gate shuts.
    ///
    /// The whole of what this type is for, and the failure it is against is the
    /// picture before it: a threshold over a live measurement, so a partial
    /// breathing across the bar takes a node's whole annulus with it every time
    /// it crosses, several times a second.
    ///
    /// The first step of all is the exception and is checked here too, because
    /// it is what every other test in this file rests on: with nothing on
    /// screen to transition FROM there is no transition to draw, so a fresh
    /// fade answers the gate exactly.
    #[test]
    fn a_rings_coming_and_going_runs_on_the_fade() {
        let wheel = wheel();
        let view = gated(0.5, SpectralReading::Fold);
        let env = fade_of(1.0);
        let sounding = gate_of(&view, partial(60.0, 10.0));
        let silent = gate_of(&view, empty());

        let mut fade = RingFade::default();
        fade.advance(&sounding, &env, 0.0);
        assert_eq!(fade.level(&wheel, 0.0), 1.0, "the first frame faded a ring in from nothing");

        // ...and out, on the gate shutting: a straight line, so a quarter of
        // the duration in is a quarter of the way down.
        fade.advance(&silent, &env, 0.25);
        let quarter = fade.level(&wheel, 0.0);
        assert!((quarter - 0.75).abs() < 1e-4, "a quarter of a Fade out reads {quarter}");
        fade.advance(&silent, &env, 0.75);
        let most = fade.level(&wheel, 0.0);
        assert!((most - 0.25).abs() < 1e-4, "three quarters of a Fade out reads {most}");

        // Turned around part way, the ring goes back up from where it had got
        // to rather than restarting from nothing — which is the whole reason
        // the level is carried and not the moment (`Envelope::carried`).
        fade.advance(&sounding, &env, 1.0);
        let back = fade.level(&wheel, 0.0);
        assert!(back > most, "a reopened gate dropped the ring from {most} to {back}");
        assert!(back < 1.0, "a reopened gate jumped the ring straight back to full");

        // And it LANDS: a fade running past its duration is over, exactly, so
        // an idle node stops being shipped at all rather than carrying a ring
        // nobody can see (`paints`, in harmonigraph-render).
        fade.advance(&silent, &env, 99.0);
        assert_eq!(fade.level(&wheel, 0.0), 0.0, "a ring long gone still wears something");
    }

    /// A Fade of 0 is the gate switching outright, which is the setting a
    /// player who wants the reading raw drags to — and the one setting at which
    /// a claim about the gate can be read straight off the picture, with no
    /// transition standing between the two.
    #[test]
    fn a_fade_of_nothing_is_the_gate_itself() {
        let wheel = wheel();
        let view = gated(0.5, SpectralReading::Fold);
        let env = fade_of(0.0);
        let mut fade = RingFade::default();
        fade.advance(&gate_of(&view, partial(60.0, 10.0)), &env, 0.0);
        assert_eq!(fade.level(&wheel, 0.0), 1.0);
        fade.advance(&gate_of(&view, empty()), &env, 0.001);
        assert_eq!(fade.level(&wheel, 0.0), 0.0, "a Fade of 0 left a ring behind");
    }

    /// One frame drawn twice steps the fade ONCE: the lattice is drawn by the
    /// docked pane and by the Video tab's preview off one clock, and a
    /// transition that ran per call would move at twice the speed in an editor
    /// and at once the speed in an export.
    ///
    /// Stated here rather than in the shell because it is this type's guarantee
    /// — the clock is what it steps against, and a caller cannot get it wrong.
    #[test]
    fn a_frame_drawn_twice_steps_the_fade_once() {
        let wheel = wheel();
        let view = gated(0.5, SpectralReading::Fold);
        // A CURVED envelope, where every other claim here is on a straight
        // line: the second call's step is zero, and a zero step is only exact
        // if it is answered as one — walking the curve out and back is a pair
        // of `powf`s that land a rounding apart, which on a straight line they
        // do not.
        let env = Envelope { attack_time: 1.0, fade_time: 1.0, shape: 0.6 };
        let sounding = gate_of(&view, partial(60.0, 10.0));
        let silent = gate_of(&view, empty());
        let twice = {
            let mut fade = RingFade::default();
            fade.advance(&sounding, &env, 0.0);
            fade.advance(&silent, &env, 0.5);
            fade.advance(&silent, &env, 0.5);
            fade.level(&wheel, 0.0)
        };
        let once = {
            let mut fade = RingFade::default();
            fade.advance(&sounding, &env, 0.0);
            fade.advance(&silent, &env, 0.5);
            fade.level(&wheel, 0.0)
        };
        assert_eq!(twice, once, "a pane drawn twice ran the fade twice");
    }

    /// A ramp running DOWNWARD is pinned at `t` = 0 like any other, and there
    /// `t` = 0 is the ramp's BRIGHT end.
    ///
    /// The pin is on the ends the analyzer's `t` axis names and not on the
    /// darker and lighter of the two: `t` = 0 is silence at either sign, so that
    /// is the end the ground claims, and `t` = 1 keeps the analyzer's own colour
    /// exactly whichever way the ramp runs. A signed ramp is a setting somebody
    /// can reach rather than a hand-edited blob's — the Spread bar writes one
    /// deliberately — so the sign the fixtures happen to carry is not the sign
    /// the function has to hold for.
    #[test]
    fn a_ramp_running_downward_is_pinned_at_its_bright_end() {
        let ground = ViewConfig::default().lattice_ground;
        for lightness_ramp in [-100.0, -44.0, -20.0] {
            let down = Gradient { lightness: 50.0, lightness_ramp, ..analyzers() };
            let analyzer = pitch_ramp_lut(down);
            assert!(
                lightness(analyzer[0]) > lightness(analyzer[PITCH_LUT_N - 1]),
                "the test's own ramp of {lightness_ramp} does not run downward",
            );
            let ring = pitch_ramp_lut(ring_gradient(down, ground));
            let silent = lightness(ring[0]);
            assert!(
                (silent - ground).abs() < 0.2,
                "a ramp of {lightness_ramp} opens at L* {silent}, not the ground's {ground}",
            );
            assert_eq!(
                ring[PITCH_LUT_N - 1],
                analyzer[PITCH_LUT_N - 1],
                "a ramp of {lightness_ramp} lost the analyzer's own colour at the loud end",
            );
        }
    }

    /// A chroma ramp that has run OUT by the loud end leaves the ring greyscale
    /// end to end, and that is the pin working rather than failing.
    ///
    /// The pin sets `t` = 0 to chroma 0 and holds `t` = 1 at whatever the
    /// analyzer carries there, so the ring's own ramp IS the analyzer's top
    /// chroma — never negative, whatever the sign it was written with. A
    /// gradient falling to nothing at the top therefore has all of its colour at
    /// its SILENT end, which is exactly the end the ground claims, and there is
    /// none left in between to spread. The analyzer's own panes keep drawing it:
    /// the ring reads a copy, and only the ring is bedded on the lattice.
    #[test]
    fn a_chroma_ramp_that_runs_out_leaves_the_ring_grey() {
        let ground = ViewConfig::default().lattice_ground;
        // Legal rather than hand-edited: `sanitized` allows a ramp of
        // `2 * min(chroma, 1 - chroma)`, which at a middle of 0.5 is the whole
        // axis, so the Spread bar reaches this at its own end stop.
        let drained = Gradient { chroma: 0.5, chroma_ramp: -1.0, ..analyzers() };
        assert_eq!(
            drained.sanitized().chroma_ramp,
            -1.0,
            "the test's own Spread was flattened before it ever reached the pin",
        );
        let ring = ring_gradient(drained, ground);
        assert_eq!((ring.chroma, ring.chroma_ramp), (0.0, 0.0));
        for (k, entry) in pitch_ramp_lut(ring).iter().enumerate() {
            assert!(
                entry.x == entry.y && entry.y == entry.z,
                "the ring's entry {k} is {entry:?}, which carries a tint",
            );
        }
        // The gradient itself is not grey — the mids of the analyzer's own table
        // are coloured, and it is the PIN that drains them, for the ring alone.
        let analyzer = pitch_ramp_lut(drained)[PITCH_LUT_N / 4];
        assert!(
            analyzer.x != analyzer.y || analyzer.y != analyzer.z,
            "the test's own gradient is already grey, so the pin drained nothing",
        );
        // A shallower fall keeps the loud end its colour: the collapse above is
        // this rule at its end stop rather than a case of its own.
        let shallow = Gradient { chroma_ramp: -0.5, ..drained };
        let shallow = ring_gradient(shallow, ground);
        assert_eq!((shallow.chroma, shallow.chroma_ramp), (0.125, 0.25));
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
