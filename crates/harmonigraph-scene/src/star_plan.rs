//! How each star depth is drawn: its gather, scale, size, opacity,
//! position variation and glow reach, and the images the far and near depths
//! and the halos are drawn into.
//!
//! The renderer draws every frame from a [`StarPlan`]. Production's comes from
//! the `Stars rendering` profile ([`StarPlan::production`]); the dev-only star
//! test bed edits one directly and is never saved
//! ([`crate::StarSettings::test_bed`]). With the test bed off the renderer
//! draws exactly what it drew before the plan existed.
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
/// The 3x3 halo's production reach in cells: the 3x3 window's guaranteed reach
/// at full `Position variation`, held there at every variation.
pub const STAR_HALO_REACH: f32 = 1.2;
/// The test bed's range for [`StarDepthPlan::reach`]. Past a gather's
/// [`StarGather::bound`] its window drops stars and the glow shows seams,
/// which is worth seeing rather than forbidding.
pub const STAR_REACH_MIN: f32 = 0.2;
pub const STAR_REACH_MAX: f32 = 2.0;
/// The test bed's range for [`StarDepthPlan::scale`] and
/// [`StarDepthPlan::size`], as multipliers.
pub const STAR_PLAN_SCALE_MIN: f32 = 0.25;
pub const STAR_PLAN_SCALE_MAX: f32 = 4.0;
/// The test bed's top for [`StarDepthPlan::gain`].
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
    /// Its own cell only: the whole star, fringe included, faded out where it
    /// would leave the cell, which is [`Self::bound`].
    Core,
    /// The four cells whose centres surround the pixel, drawing the whole
    /// star, glow included, in one pass.
    Two,
    /// The 1x1 core at the depth's own resolution plus a 3x3 halo drawn into
    /// a halo image at its tier's resolution.
    Three,
}

impl StarGather {
    /// The farthest a glow can reach in cells and never lose a star, at
    /// `Position variation` `jitter`: half the window's width less half the
    /// band a centre is drawn from. Computed as the shader computes the 2x2
    /// fade, so production's 2x2 reach is this exactly.
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
/// `jitter`: the original 0.6 at 1. In f64 and narrowed, as it always was.
pub fn star_jitter_width(jitter: f32) -> f32 {
    (0.6 * f64::from(jitter)) as f32
}

/// One depth's part of a [`StarPlan`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StarDepthPlan {
    pub gather: StarGather,
    /// Zooms the depth: its cell and its cores both, as a multiple of what
    /// `Star spacing`, `Star density` and `Star size` give them, so the depth
    /// keeps its proportions with fewer, larger stars.
    pub scale: f32,
    /// The core's size relative to the scaled one; the cap at a third of the
    /// cell still binds, and in 1x1 the cell's edge.
    pub size: f32,
    /// While any depth is soloed, only soloed depths are drawn; the rest count
    /// as [`StarGather::Off`] wherever the plan is read.
    pub solo: bool,
    /// A multiplier on every star's coverage.
    pub gain: f32,
    /// `Position variation` for this depth alone.
    pub jitter: f32,
    /// How far the glow reaches, in cells. [`StarGather::Core`] reaches its
    /// bound instead.
    pub reach: f32,
    /// Which of [`StarPlan::halo_tiers`] a [`StarGather::Three`] halo is
    /// drawn at.
    pub tier: usize,
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

impl StarPlan {
    /// What the `Stars rendering` profile draws. Uniform gives every depth a
    /// 3x3 halo at `Uniform halo resolution`; the presets draw the far three
    /// with 2x2 into a reduced far image and give the near two 3x3 halos at
    /// fixed resolutions.
    pub fn production(settings: StarSettings) -> Self {
        let jitter = settings.star_jitter;
        // The far tier is unused in production; it is where a far depth's halo
        // goes when the test bed switches one to 3x3, at the far image's own
        // resolution.
        let (halo_tiers, far, near) = match settings.star_halo_profile {
            StarHaloProfile::Uniform => ([settings.star_halo_resolution, 1.0, 1.0], 1.0, 1.0),
            StarHaloProfile::P3 => ([1.0, 0.6, 0.75], 0.75, 1.0),
            StarHaloProfile::Medium => ([0.75, 0.45, 0.5], 0.5, 0.75),
            StarHaloProfile::Low => ([0.5, 0.3, 1.0 / 3.0], 1.0 / 3.0, 0.5),
        };
        let uniform = settings.star_halo_profile == StarHaloProfile::Uniform;
        let depths = std::array::from_fn(|k| {
            let far_depth = k < STAR_FAR_DEPTHS;
            let gather = if far_depth && !uniform { StarGather::Two } else { StarGather::Three };
            let reach = match gather {
                StarGather::Two => gather.bound(jitter),
                _ => STAR_HALO_REACH,
            };
            let tier = match (uniform, far_depth, k) {
                (true, ..) => 0,
                (false, true, _) => 2,
                (false, false, k) => k - STAR_FAR_DEPTHS,
            };
            StarDepthPlan {
                gather,
                scale: 1.0,
                size: 1.0,
                gain: 1.0,
                jitter,
                reach,
                tier,
                solo: false,
            }
        });
        Self { depths, halo_tiers, far, near }
    }

    /// This plan with every value inside the test bed's ranges and every
    /// non-finite one back at `fallback`'s.
    pub fn sanitized(mut self, fallback: Self) -> Self {
        let clamp = |value: f32, fallback: f32, low: f32, high: f32| {
            if value.is_finite() {
                value.clamp(low, high)
            } else {
                fallback
            }
        };
        for (depth, fresh) in self.depths.iter_mut().zip(fallback.depths) {
            depth.scale = clamp(depth.scale, fresh.scale, STAR_PLAN_SCALE_MIN, STAR_PLAN_SCALE_MAX);
            depth.size = clamp(depth.size, fresh.size, STAR_PLAN_SCALE_MIN, STAR_PLAN_SCALE_MAX);
            depth.gain = clamp(depth.gain, fresh.gain, 0.0, STAR_GAIN_MAX);
            depth.jitter = clamp(depth.jitter, fresh.jitter, 0.0, 1.0);
            depth.reach = clamp(depth.reach, fresh.reach, STAR_REACH_MIN, STAR_REACH_MAX);
            depth.tier = depth.tier.min(STAR_HALO_TIERS - 1);
        }
        let image = |value: f32, fallback: f32| {
            clamp(value, fallback, STAR_IMAGE_RESOLUTION_MIN, STAR_IMAGE_RESOLUTION_MAX)
        };
        for (tier, fresh) in self.halo_tiers.iter_mut().zip(fallback.halo_tiers) {
            *tier = image(*tier, fresh);
        }
        self.far = image(self.far, fallback.far);
        self.near = image(self.near, fallback.near);
        self
    }

    /// This plan as drawn: with any depth soloed, every other one is Off.
    pub fn soloed(mut self) -> Self {
        if self.depths.iter().any(|depth| depth.solo) {
            for depth in self.depths.iter_mut().filter(|depth| !depth.solo) {
                depth.gather = StarGather::Off;
            }
        }
        self
    }
}

impl StarSettings {
    /// The plan the renderer draws: the test bed's while it is on, else the
    /// profile's.
    pub fn plan(self) -> StarPlan {
        self.test_bed.map_or_else(|| StarPlan::production(self), StarPlan::soloed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test bed value off its range is drawn at the range's edge, and a
    /// non-finite one at production's, so the panel never shows a value the
    /// renderer is not drawing.
    #[test]
    fn a_test_bed_plan_is_sanitized_against_production() {
        let production = StarPlan::production(StarSettings::default());
        let mut plan = production;
        plan.depths[0].scale = f32::NAN;
        plan.depths[1].reach = 9.0;
        plan.depths[2].gain = -1.0;
        plan.depths[3].tier = 7;
        plan.far = 0.0;
        let settings = StarSettings { test_bed: Some(plan), ..Default::default() }.sanitized();
        let plan = settings.plan();
        assert_eq!(plan.depths[0].scale, production.depths[0].scale);
        assert_eq!(plan.depths[1].reach, STAR_REACH_MAX);
        assert_eq!(plan.depths[2].gain, 0.0);
        assert_eq!(plan.depths[3].tier, STAR_HALO_TIERS - 1);
        assert_eq!(plan.far, STAR_IMAGE_RESOLUTION_MIN);
    }

    /// Soloing a depth draws it alone, and soloing none draws them all.
    #[test]
    fn a_soloed_depth_is_drawn_alone() {
        let mut plan = StarPlan::production(StarSettings::default());
        let gathers = plan.depths.map(|depth| depth.gather);
        let settings = |plan| StarSettings { test_bed: Some(plan), ..Default::default() };
        assert_eq!(settings(plan).plan().depths.map(|depth| depth.gather), gathers);
        plan.depths[3].solo = true;
        let drawn = settings(plan).plan().depths.map(|depth| depth.gather);
        for (k, gather) in drawn.into_iter().enumerate() {
            assert_eq!(gather, if k == 3 { gathers[3] } else { StarGather::Off }, "depth {k}");
        }
    }
}
