//! How each star depth is drawn: its cell, its stars' radius and shape, which
//! cells a pixel reads for it, and the images the far and near depths and the
//! halos are drawn into.
//!
//! The renderer draws every frame from the [`StarPlan`] its settings give
//! ([`StarSettings::plan`]), worked out afresh each time from the dials, so
//! nothing in it is ever stale. The dev-only star test bed is a set of
//! overrides on top ([`StarTestBed`], [`StarSettings::test_bed`]), never saved.
use crate::{StarHaloProfile, StarSettings};

/// How many depths the starfield draws, far (0) to near. The renderer's
/// `STAR_SLICES`.
pub const STAR_DEPTHS: usize = 5;
/// The farthest depths, which share the far image when there is one. The
/// shader's `STAR_FAR_LAYERS`.
pub const STAR_FAR_DEPTHS: usize = 3;
/// How many halo resolutions a plan may use at once: the renderer binds one
/// halo image array per resolution, and the spectrogram's shader has no room
/// for more.
pub const STAR_HALO_TIERS: usize = 3;
/// The test bed's range for [`StarDepthOverride::scale`] and
/// [`StarDepthOverride::size`], as multipliers.
pub const STAR_PLAN_SCALE_MIN: f32 = 0.25;
pub const STAR_PLAN_SCALE_MAX: f32 = 4.0;
/// The test bed's top for [`StarDepthOverride::gain`].
pub const STAR_GAIN_MAX: f32 = 4.0;
/// The test bed's range for image resolutions, as a fraction of the pane's
/// device pixels on each axis.
pub const STAR_IMAGE_RESOLUTION_MIN: f32 = 0.25;
pub const STAR_IMAGE_RESOLUTION_MAX: f32 = 1.0;

/// Which cells a pixel reads to draw one depth's stars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StarGather {
    /// The depth is not drawn, baked or allocated.
    Off,
    /// Its own cell only: one read, for stars small enough to stay inside it.
    Core,
    /// The four cells whose centres surround the pixel, drawing the whole
    /// star in one pass.
    Two,
    /// The star's part inside its own cell at the depth's resolution, plus
    /// the rest gathered from nine cells into a halo image at its tier's
    /// resolution.
    Three,
}

impl StarGather {
    /// The cheapest first: the order [`StarSettings::plan`] tries them in.
    pub const DRAWN: [Self; 3] = [Self::Core, Self::Two, Self::Three];

    /// The farthest a star can reach in cells and never lose a pixel, at
    /// `Position variation` `jitter`: half the window's width less half the
    /// band a centre is drawn from.
    pub fn bound(self, jitter: f32) -> f32 {
        let half_band = star_jitter_width(jitter) * 0.5;
        match self {
            Self::Off | Self::Core => 0.5 - half_band,
            Self::Two => 1.0 - half_band,
            Self::Three => 1.5 - half_band,
        }
    }
}

/// One star's coverage at `t`, its distance over its own outer radius: full
/// out to `solid`, then the glow, easing out of the solid edge and into the
/// star's edge with no corner at either and bent by `bend`
/// ([`star_falloff_bend`]). The shader's `star_profile`, which takes `ramp`
/// as `1 / (1 - solid)`.
pub fn star_profile(t: f32, solid: f32, bend: f32) -> f32 {
    if t >= 1.0 {
        return 0.0;
    }
    let u = ((t - solid) / (1.0 - solid)).clamp(0.0, 1.0);
    let x = u * u * (3.0 - 2.0 * u);
    (1.0 - x) / (1.0 + bend * x)
}

/// The glow's bend at `Glow falloff` `falloff`: `(1 - x) / (1 + bend x)` over
/// the eased glow, so 0.5 falls evenly (bend 0), 0 bows it out to stay bright
/// almost to the edge (bend -15/16) and 1 drops at once into a long faint
/// tail (bend 15).
pub fn star_falloff_bend(falloff: f32) -> f32 {
    16f32.powf(2.0 * falloff - 1.0) - 1.0
}

/// Where each depth sits from far (0) to near (1) at `Star layers` `layers`,
/// or `None` where it is not drawn. Layer `i` of `n` sits at `i / (n - 1)`,
/// in the depth whose own place at five layers is nearest, so it keeps that
/// depth's image and halo tier: two layers are the farthest and the nearest,
/// three add the middle.
pub fn star_layer_depths(layers: u32) -> [Option<f32>; STAR_DEPTHS] {
    let n = (layers as usize).clamp(2, STAR_DEPTHS);
    let mut depths = [None; STAR_DEPTHS];
    for i in 0..n {
        let d = i as f32 / (n - 1) as f32;
        depths[(d * (STAR_DEPTHS - 1) as f32).round() as usize] = Some(d);
    }
    depths
}

/// The band a star's centre is drawn from, in cells, at `Position variation`
/// `jitter`: the original 0.6 at 1.
pub fn star_jitter_width(jitter: f32) -> f32 {
    (0.6 * f64::from(jitter)) as f32
}

/// One depth as drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StarDepthPlan {
    pub gather: StarGather,
    /// Where the depth sits from far (0) to near (1): its layer's place among
    /// `Star layers` ([`star_layer_depths`]), or its own place at five layers
    /// where it is not drawn. Every far-to-near control is read here.
    pub depth: f32,
    /// How fast the depth drifts, as a multiple of the renderer's star pixels
    /// a second: `far + (near - far) d^curve` over `Star speed`.
    pub speed: f32,
    /// The cell one star is hashed into, in star pixels, before the atlas's
    /// floor.
    pub cell: f32,
    /// The stars' outer radius in star pixels before each star's own size
    /// draw shrinks it: what the dials ask for, held to the gather's
    /// [`StarGather::bound`].
    pub radius: f32,
    /// What the dials ask for.
    pub wanted: f32,
    /// The star's shape ([`star_profile`]): its solid share between
    /// [`StarSettings::star_solid_far`] and [`StarSettings::star_solid_near`],
    /// and [`StarSettings::star_glow_falloff`].
    pub solid: f32,
    pub falloff: f32,
    /// A multiplier on every star's coverage.
    pub gain: f32,
    /// `Position variation` at this depth.
    pub jitter: f32,
    /// Which of [`StarPlan::halo_tiers`] a [`StarGather::Three`] halo is
    /// drawn at.
    pub tier: usize,
}

impl StarDepthPlan {
    /// Whether the depth's stars are drawn smaller than the dials ask, to fit
    /// the widest read its spacing allows.
    pub fn clamped(&self) -> bool {
        self.gather != StarGather::Off && self.radius < self.wanted
    }
}

/// Everything the renderer needs to know about how to draw each depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StarPlan {
    pub depths: [StarDepthPlan; STAR_DEPTHS],
    /// Halo image resolutions, as a fraction of the pane on each axis.
    pub halo_tiers: [f32; STAR_HALO_TIERS],
    /// The far image's resolution: the far depths drawn once into their own
    /// image and sampled filtered. 1 draws them at the pane's resolution,
    /// split off only on a big pane.
    pub far: f32,
    /// The near image's resolution: the near depths drawn over the far image
    /// at this fraction and filtered up. 1 draws them straight into the pane,
    /// and so does any value while [`Self::far`] is 1.
    pub near: f32,
}

/// The dev-only star test bed: overrides on the plan the settings give. Every
/// `None` follows the settings live.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StarTestBed {
    pub depths: [StarDepthOverride; STAR_DEPTHS],
    pub halo_tiers: [Option<f32>; STAR_HALO_TIERS],
    pub far: Option<f32>,
    pub near: Option<f32>,
}

/// One depth's overrides. The multipliers are 1 when untouched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StarDepthOverride {
    /// `None` picks the cheapest gather that holds the stars whole.
    pub gather: Option<StarGather>,
    /// Zooms the depth: its cell and its stars together.
    pub scale: f32,
    /// The stars' size relative to the scaled depth.
    pub size: f32,
    pub gain: f32,
    pub jitter: Option<f32>,
    pub solid: Option<f32>,
    pub falloff: Option<f32>,
    pub tier: Option<usize>,
    /// While any depth is soloed, only soloed depths are drawn.
    pub solo: bool,
}

impl Default for StarDepthOverride {
    fn default() -> Self {
        Self {
            gather: None,
            scale: 1.0,
            size: 1.0,
            gain: 1.0,
            jitter: None,
            solid: None,
            falloff: None,
            tier: None,
            solo: false,
        }
    }
}

impl StarTestBed {
    /// Every value inside its range; a non-finite one is dropped back to
    /// following the settings, or to 1 for a multiplier.
    pub fn sanitized(mut self) -> Self {
        let clamp =
            |value: f32, low: f32, high: f32| value.is_finite().then(|| value.clamp(low, high));
        let range = |value: Option<f32>, low, high| value.and_then(|value| clamp(value, low, high));
        for depth in &mut self.depths {
            let scale =
                |value| clamp(value, STAR_PLAN_SCALE_MIN, STAR_PLAN_SCALE_MAX).unwrap_or(1.0);
            depth.scale = scale(depth.scale);
            depth.size = scale(depth.size);
            depth.gain = clamp(depth.gain, 0.0, STAR_GAIN_MAX).unwrap_or(1.0);
            depth.jitter = range(depth.jitter, 0.0, 1.0);
            depth.solid = range(depth.solid, 0.0, crate::STAR_SOLID_MAX);
            depth.falloff = range(depth.falloff, 0.0, 1.0);
            depth.tier = depth.tier.map(|tier| tier.min(STAR_HALO_TIERS - 1));
        }
        let image = |value| range(value, STAR_IMAGE_RESOLUTION_MIN, STAR_IMAGE_RESOLUTION_MAX);
        self.halo_tiers = self.halo_tiers.map(image);
        self.far = image(self.far);
        self.near = image(self.near);
        self
    }
}

impl StarSettings {
    /// The plan the renderer draws: the `Stars rendering` profile's images and
    /// halo tiers, every depth's cell and star from the dials, each gathered
    /// by the cheapest read that holds its stars whole, and the test bed's
    /// overrides on top. The test bed is sanitized here, and only here, so
    /// every reader of the plan draws the same values.
    pub fn plan(self) -> StarPlan {
        let bed = self.test_bed.map(StarTestBed::sanitized).unwrap_or_default();
        let ([nearer, nearest], far, near) = match self.star_halo_profile {
            StarHaloProfile::Uniform => ([self.star_halo_resolution, 1.0], 1.0, 1.0),
            StarHaloProfile::P3 => ([1.0, 0.6], 0.75, 1.0),
            StarHaloProfile::Medium => ([0.75, 0.45], 0.5, 0.75),
            StarHaloProfile::Low => ([0.5, 0.3], 1.0 / 3.0, 0.5),
        };
        // The third tier is where a far depth's halo goes when it is gathered
        // 3x3: at the far image's own resolution.
        let halo_tiers = [nearer, nearest, far];
        let uniform = self.star_halo_profile == StarHaloProfile::Uniform;
        let solo = bed.depths.iter().any(|depth| depth.solo);
        let layers = star_layer_depths(self.star_layers);
        let place = |k: usize| layers[k].unwrap_or(k as f32 / (STAR_DEPTHS - 1) as f32);
        let depth = |k: usize, curve: f32| place(k).powf(curve);
        let along = |k, small: f32, big: f32, curve| small * (big / small).powf(depth(k, curve));
        let depths = std::array::from_fn(|k| {
            let o = bed.depths[k];
            let (spacing, size) = (self.star_spacing_curve, self.star_size_curve);
            let cell = along(k, self.star_spacing_far, self.star_spacing_near, spacing) * o.scale;
            let wanted =
                0.5 * along(k, self.star_size_far, self.star_size_near, size) * o.scale * o.size;
            let jitter = o.jitter.unwrap_or(self.star_jitter);
            let fits = |gather: StarGather| wanted <= gather.bound(jitter) * cell;
            let gather = if layers[k].is_none() || solo && !o.solo {
                StarGather::Off
            } else {
                o.gather.unwrap_or_else(|| {
                    StarGather::DRAWN.into_iter().find(|&g| fits(g)).unwrap_or(StarGather::Three)
                })
            };
            let tier = o.tier.unwrap_or(match (uniform, k < STAR_FAR_DEPTHS) {
                (true, _) => 0,
                (false, true) => 2,
                (false, false) => (k - STAR_FAR_DEPTHS).min(1),
            });
            StarDepthPlan {
                gather,
                depth: place(k),
                speed: {
                    let (far, near) = (self.star_speed_far, self.star_speed_near);
                    far + (near - far) * depth(k, self.star_speed_curve)
                },
                cell,
                radius: wanted.min(gather.bound(jitter) * cell),
                wanted,
                solid: o.solid.unwrap_or_else(|| {
                    let (far, near) = (self.star_solid_far, self.star_solid_near);
                    far + (near - far) * depth(k, size)
                }),
                falloff: o.falloff.unwrap_or(self.star_glow_falloff),
                gain: o.gain,
                jitter,
                tier,
            }
        });
        let pick = |over: Option<f32>, base: f32| over.unwrap_or(base);
        StarPlan {
            depths,
            halo_tiers: std::array::from_fn(|t| pick(bed.halo_tiers[t], halo_tiers[t])),
            far: pick(bed.far, far),
            near: pick(bed.near, near),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each depth is read by the cheapest gather that holds its stars whole,
    /// and one too big for the widest is drawn at the widest that fits and
    /// says so. At the fresh dials the far three fit 2x2 and the near two
    /// need 3x3, which is what the presets drew before sizes chose.
    #[test]
    fn each_depth_takes_the_cheapest_gather_that_holds_its_stars() {
        let fresh = StarSettings::default().plan();
        let gathers = fresh.depths.map(|depth| depth.gather);
        use StarGather::{Three, Two};
        assert_eq!(gathers, [Two, Two, Two, Three, Three]);
        assert!(fresh.depths.iter().all(|depth| !depth.clamped()));

        let tiny = StarSettings { star_size_far: 0.5, star_size_near: 0.5, ..Default::default() };
        assert!(tiny.plan().depths.iter().all(|depth| depth.gather == StarGather::Core));

        let huge = StarSettings { star_size_near: 64.0, ..Default::default() }.plan();
        let nearest = huge.depths[STAR_DEPTHS - 1];
        assert_eq!(nearest.gather, Three);
        assert!(nearest.clamped());
        assert_eq!(nearest.radius, Three.bound(nearest.jitter) * nearest.cell);
    }

    /// Every far-to-near pair runs either way: a reversed one survives the
    /// load boundary as it was set, and the far depth then gets the bigger
    /// end.
    #[test]
    fn a_reversed_pair_is_kept_and_drawn_reversed() {
        let fresh = StarSettings::default();
        let reversed = StarSettings {
            star_spacing_far: fresh.star_spacing_near,
            star_spacing_near: fresh.star_spacing_far,
            star_size_far: fresh.star_size_near,
            star_size_near: fresh.star_size_far,
            star_speed_far: fresh.star_speed_near,
            star_speed_near: fresh.star_speed_far,
            ..fresh
        };
        assert_eq!(reversed.sanitized(), reversed);
        let [far, .., near] = reversed.plan().depths;
        assert!(far.cell > near.cell && far.wanted > near.wanted);
    }

    /// Fewer layers keep the farthest and the nearest and space the rest
    /// evenly between, each in the depth nearest its place, and every
    /// far-to-near control spreads over just the drawn ones.
    #[test]
    fn star_layers_keep_both_ends_and_spread_the_rest() {
        let fresh = StarSettings::default();
        for (layers, drawn) in [
            (2, vec![(0, 0.0), (4, 1.0)]),
            (3, vec![(0, 0.0), (2, 0.5), (4, 1.0)]),
            (4, vec![(0, 0.0), (1, 1.0 / 3.0), (3, 2.0 / 3.0), (4, 1.0)]),
            (5, vec![(0, 0.0), (1, 0.25), (2, 0.5), (3, 0.75), (4, 1.0)]),
        ] {
            let plan = StarSettings { star_layers: layers, ..fresh }.plan();
            let got: Vec<_> = (0..STAR_DEPTHS)
                .filter(|&k| plan.depths[k].gather != StarGather::Off)
                .map(|k| (k, plan.depths[k].depth))
                .collect();
            assert_eq!(got, drawn, "{layers} layers");
            let [far, .., near] = plan.depths;
            assert_eq!((far.cell, near.cell), (fresh.star_spacing_far, fresh.star_spacing_near));
            assert_eq!((far.speed, near.speed), (fresh.star_speed_far, fresh.star_speed_near));
        }
        let middle = StarSettings { star_layers: 3, ..fresh }.plan().depths[2];
        let half =
            |far: f32, near: f32| far * (near / far).powf(0.5f32.powf(fresh.star_spacing_curve));
        let want = half(fresh.star_spacing_far, fresh.star_spacing_near);
        assert!((middle.cell - want).abs() < 1e-5 * want, "{} vs {want}", middle.cell);
    }

    /// A star is full out to its solid share and nothing at its edge, its glow
    /// falls the whole way between, and a higher falloff sits lower all along
    /// it: 50% is the even fall, through half coverage halfway.
    #[test]
    fn the_profile_is_solid_then_a_glow_bent_by_its_falloff() {
        let solid = 0.3;
        let glow = |t, falloff| star_profile(t, solid, star_falloff_bend(falloff));
        for falloff in [0.0, 0.5, 1.0] {
            assert_eq!(glow(0.0, falloff), 1.0);
            assert_eq!(glow(solid, falloff), 1.0);
            assert_eq!(glow(1.0, falloff), 0.0);
            let along: Vec<f32> =
                (0..=20).map(|i| glow(solid + 0.035 * i as f32, falloff)).collect();
            assert!(along.windows(2).all(|w| w[1] < w[0]), "falloff {falloff}: {along:?}");
        }
        let mid = solid + 0.5 * (1.0 - solid);
        assert!((glow(mid, 0.5) - 0.5).abs() < 1e-6);
        assert!(glow(mid, 0.0) > glow(mid, 0.5) && glow(mid, 0.5) > glow(mid, 1.0));
    }

    /// An override replaces only what it names; everything else follows the
    /// settings live, and soloing a depth draws it alone.
    #[test]
    fn test_bed_overrides_follow_the_settings_where_unset() {
        let mut bed = StarTestBed::default();
        bed.depths[3].solo = true;
        bed.depths[3].solid = Some(0.1);
        bed.depths[3].gather = Some(StarGather::Two);
        let settings =
            StarSettings { star_glow_falloff: 0.8, test_bed: Some(bed), ..Default::default() };
        let plan = settings.plan();
        for (k, depth) in plan.depths.iter().enumerate() {
            let want = if k == 3 { StarGather::Two } else { StarGather::Off };
            assert_eq!(depth.gather, want, "depth {k}");
        }
        assert_eq!(plan.depths[3].solid, 0.1);
        assert_eq!(plan.depths[3].falloff, 0.8);
        assert_eq!(plan.depths[3].jitter, StarSettings::default().plan().depths[3].jitter);
    }

    /// A test bed value off its range is drawn at the range's edge, and a
    /// non-finite one follows the settings, so the panel never shows a value
    /// the renderer is not drawing.
    #[test]
    fn a_test_bed_is_sanitized() {
        let mut bed = StarTestBed::default();
        bed.depths[0].scale = f32::NAN;
        bed.depths[1].solid = Some(9.0);
        bed.depths[2].gain = -1.0;
        bed.depths[3].tier = Some(7);
        bed.far = Some(0.0);
        let bed = bed.sanitized();
        assert_eq!(bed.depths[0].scale, 1.0);
        assert_eq!(bed.depths[1].solid, Some(crate::STAR_SOLID_MAX));
        assert_eq!(bed.depths[2].gain, 0.0);
        assert_eq!(bed.depths[3].tier, Some(STAR_HALO_TIERS - 1));
        assert_eq!(bed.far, Some(STAR_IMAGE_RESOLUTION_MIN));
    }
}
