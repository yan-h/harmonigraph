//! How each star depth is drawn: its cell, its stars' radius and shape, which
//! cells a pixel reads for it, and the images the far and near depths and the
//! halos are drawn into.
//!
//! The renderer draws every frame from the [`StarPlan`] its settings give
//! ([`StarSettings::plan`]), worked out afresh each time from the dials, so
//! nothing in it is ever stale.
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
/// Which cells a pixel reads to draw one depth's stars.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StarGather {
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
    pub const CHEAPEST_FIRST: [Self; 3] = [Self::Core, Self::Two, Self::Three];

    /// The farthest a star can reach in cells and never lose a pixel, at
    /// `Position variation` `jitter`: half the window's width less half the
    /// band a centre is drawn from.
    pub fn bound(self, jitter: f32) -> f32 {
        let half_band = star_jitter_width(jitter) * 0.5;
        match self {
            Self::Core => 0.5 - half_band,
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
    /// Which of [`StarPlan::halo_tiers`] a [`StarGather::Three`] halo is
    /// drawn at.
    pub tier: usize,
}

impl StarDepthPlan {
    /// Whether the depth's stars are drawn smaller than the dials ask, to fit
    /// the widest read its spacing allows.
    pub fn clamped(&self) -> bool {
        self.radius < self.wanted
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

impl StarSettings {
    /// The plan the renderer draws: the `Stars rendering` profile's images and
    /// halo tiers, every depth's cell and star from the dials, each gathered
    /// by the cheapest read that holds its stars whole.
    pub fn plan(self) -> StarPlan {
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
        let depth = |k: usize, curve: f32| (k as f32 / (STAR_DEPTHS - 1) as f32).powf(curve);
        let along = |k, small: f32, big: f32, curve| small * (big / small).powf(depth(k, curve));
        let jitter = self.star_jitter;
        let depths = std::array::from_fn(|k| {
            let (spacing, size) = (self.star_spacing_curve, self.star_size_curve);
            let cell = along(k, self.star_spacing_far, self.star_spacing_near, spacing);
            let wanted = 0.5 * along(k, self.star_size_min, self.star_size_max, size);
            let fits = |gather: StarGather| wanted <= gather.bound(jitter) * cell;
            let gather = StarGather::CHEAPEST_FIRST
                .into_iter()
                .find(|&g| fits(g))
                .unwrap_or(StarGather::Three);
            let tier = match (uniform, k < STAR_FAR_DEPTHS) {
                (true, _) => 0,
                (false, true) => 2,
                (false, false) => (k - STAR_FAR_DEPTHS).min(1),
            };
            let (far, near) = (self.star_core_far, self.star_core_near);
            StarDepthPlan {
                gather,
                cell,
                radius: wanted.min(gather.bound(jitter) * cell),
                wanted,
                core: far + (near - far) * depth(k, size),
                glow: self.star_glow,
                falloff: self.star_falloff,
                tier,
            }
        });
        StarPlan { depths, halo_tiers, far, near }
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
        assert_eq!(nearest.radius, Three.bound(StarSettings::default().star_jitter) * nearest.cell);
    }
}
