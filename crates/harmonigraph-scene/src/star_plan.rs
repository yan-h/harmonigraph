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
/// The farthest depths, which share the far image when there is one and take
/// `Distant gap fill`. The shader's `STAR_FAR_LAYERS`.
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

/// The band a star's centre is drawn from, in cells, at `Position variation`
/// `jitter`: the original 0.6 at 1.
pub fn star_jitter_width(jitter: f32) -> f32 {
    (0.6 * f64::from(jitter)) as f32
}

/// One depth as drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StarDepthPlan {
    pub gather: StarGather,
    /// The cell one star is hashed into, in star pixels, before the atlas's
    /// floor.
    pub cell: f32,
    /// The stars' outer radius in star pixels before each star's own size
    /// draw shrinks it: what the dials ask for, held to the gather's
    /// [`StarGather::bound`].
    pub radius: f32,
    /// What the dials ask for.
    pub wanted: f32,
    /// The star's shape: its core share between [`StarSettings::star_core_far`]
    /// and [`StarSettings::star_core_near`],
    /// [`StarSettings::star_glow`], [`StarSettings::star_falloff`].
    pub core: f32,
    pub glow: f32,
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
    pub core: Option<f32>,
    pub glow: Option<f32>,
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
            core: None,
            glow: None,
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
            depth.core = range(depth.core, crate::STAR_CORE_MIN, crate::STAR_CORE_MAX);
            depth.glow = range(depth.glow, 0.0, crate::STAR_GLOW_MAX);
            depth.falloff = range(depth.falloff, crate::STAR_FALLOFF_MIN, crate::STAR_FALLOFF_MAX);
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
        let packing = (self.star_density / 2.0).sqrt();
        let depth = |k: usize, curve: f32| (k as f32 / (STAR_DEPTHS - 1) as f32).powf(curve);
        let along = |k, small: f32, big: f32, curve| small * (big / small).powf(depth(k, curve));
        let depths = std::array::from_fn(|k| {
            let o = bed.depths[k];
            let (spacing, size) = (self.star_spacing_curve, self.star_size_curve);
            let cell =
                along(k, self.star_spacing_min, self.star_spacing_max, spacing) / packing * o.scale;
            let wanted =
                0.5 * along(k, self.star_size_min, self.star_size_max, size) * o.scale * o.size;
            let jitter = o.jitter.unwrap_or(self.star_jitter);
            let fits = |gather: StarGather| wanted <= gather.bound(jitter) * cell;
            let gather = if solo && !o.solo {
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
                cell,
                radius: wanted.min(gather.bound(jitter) * cell),
                wanted,
                core: o.core.unwrap_or_else(|| {
                    let (far, near) = (self.star_core_far, self.star_core_near);
                    far + (near - far) * depth(k, size)
                }),
                glow: o.glow.unwrap_or(self.star_glow),
                falloff: o.falloff.unwrap_or(self.star_falloff),
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

        let tiny = StarSettings { star_size_min: 0.5, star_size_max: 0.5, ..Default::default() };
        assert!(tiny.plan().depths.iter().all(|depth| depth.gather == StarGather::Core));

        let huge = StarSettings { star_size_max: 64.0, ..Default::default() }.plan();
        let nearest = huge.depths[STAR_DEPTHS - 1];
        assert_eq!(nearest.gather, Three);
        assert!(nearest.clamped());
        assert_eq!(nearest.radius, Three.bound(nearest.jitter) * nearest.cell);
    }

    /// An override replaces only what it names; everything else follows the
    /// settings live, and soloing a depth draws it alone.
    #[test]
    fn test_bed_overrides_follow_the_settings_where_unset() {
        let mut bed = StarTestBed::default();
        bed.depths[3].solo = true;
        bed.depths[3].glow = Some(0.1);
        bed.depths[3].gather = Some(StarGather::Two);
        let settings =
            StarSettings { star_falloff: 4.0, test_bed: Some(bed), ..Default::default() };
        let plan = settings.plan();
        for (k, depth) in plan.depths.iter().enumerate() {
            let want = if k == 3 { StarGather::Two } else { StarGather::Off };
            assert_eq!(depth.gather, want, "depth {k}");
        }
        assert_eq!(plan.depths[3].glow, 0.1);
        assert_eq!(plan.depths[3].falloff, 4.0);
        assert_eq!(plan.depths[3].core, StarSettings::default().plan().depths[3].core);
    }

    /// A test bed value off its range is drawn at the range's edge, and a
    /// non-finite one follows the settings, so the panel never shows a value
    /// the renderer is not drawing.
    #[test]
    fn a_test_bed_is_sanitized() {
        let mut bed = StarTestBed::default();
        bed.depths[0].scale = f32::NAN;
        bed.depths[1].glow = Some(9.0);
        bed.depths[2].gain = -1.0;
        bed.depths[3].tier = Some(7);
        bed.far = Some(0.0);
        let bed = bed.sanitized();
        assert_eq!(bed.depths[0].scale, 1.0);
        assert_eq!(bed.depths[1].glow, Some(crate::STAR_GLOW_MAX));
        assert_eq!(bed.depths[2].gain, 0.0);
        assert_eq!(bed.depths[3].tier, Some(STAR_HALO_TIERS - 1));
        assert_eq!(bed.far, Some(STAR_IMAGE_RESOLUTION_MIN));
    }
}
