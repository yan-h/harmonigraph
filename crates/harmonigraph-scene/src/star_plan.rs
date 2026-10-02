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
    /// floor: the depth's star size times its `Star spacing`.
    pub cell: f32,
    /// The stars' outer radius in star pixels before each star's own size
    /// draw shrinks it: half the depth's `Star size`, always whole within
    /// [`Self::gather`], because `Star spacing` never runs below
    /// [`crate::STAR_SPACING_MIN`].
    pub radius: f32,
    /// The star's solid share ([`star_profile`]), between
    /// [`StarSettings::star_solid_far`] and [`StarSettings::star_solid_near`].
    pub solid: f32,
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
        let layers = star_layer_depths(self.star_layers);
        let drawn = |k: usize| layers[k].is_some();
        let solo = (0..STAR_DEPTHS).any(|k| drawn(k) && self.star_solo[k]);
        let place = |k: usize| layers[k].unwrap_or(k as f32 / (STAR_DEPTHS - 1) as f32);
        let depth = |k: usize, curve: f32| place(k).powf(curve);
        let along = |k, small: f32, big: f32, curve| small * (big / small).powf(depth(k, curve));
        let jitter = self.star_jitter;
        let depths = std::array::from_fn(|k| {
            let (spacing, size) = (self.star_spacing_ratio_curve, self.star_size_curve);
            let diameter = along(k, self.star_size_far, self.star_size_near, size);
            let ratio =
                along(k, self.star_spacing_ratio_far, self.star_spacing_ratio_near, spacing);
            let (cell, radius) = (ratio * diameter, 0.5 * diameter);
            let fits = |gather: StarGather| radius <= gather.bound(jitter) * cell;
            let gather = if !drawn(k) || solo && !self.star_solo[k] {
                StarGather::Off
            } else {
                StarGather::DRAWN.into_iter().find(|&g| fits(g)).unwrap_or(StarGather::Three)
            };
            let tier = match (uniform, k < STAR_FAR_DEPTHS) {
                (true, _) => 0,
                (false, true) => 2,
                (false, false) => (k - STAR_FAR_DEPTHS).min(1),
            };
            StarDepthPlan {
                gather,
                depth: place(k),
                speed: {
                    let (far, near) = (self.star_speed_far, self.star_speed_near);
                    far + (near - far) * depth(k, self.star_speed_curve)
                },
                cell,
                radius,
                solid: {
                    let (far, near) = (self.star_solid_far, self.star_solid_near);
                    far + (near - far) * depth(k, size)
                },
                tier,
            }
        });
        StarPlan { depths, halo_tiers, far, near }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each depth is read by the cheapest gather that holds its stars whole.
    /// At the fresh dials the far three fit 2x2 and the near two need 3x3,
    /// which is what the presets drew before sizes chose.
    #[test]
    fn each_depth_takes_the_cheapest_gather_that_holds_its_stars() {
        let fresh = StarSettings::default().plan();
        let gathers = fresh.depths.map(|depth| depth.gather);
        use StarGather::{Three, Two};
        assert_eq!(gathers, [Two, Two, Two, Three, Three]);

        let sparse = StarSettings {
            star_spacing_ratio_far: 2.0,
            star_spacing_ratio_near: 2.0,
            ..Default::default()
        };
        assert!(sparse.plan().depths.iter().all(|depth| depth.gather == StarGather::Core));
    }

    /// `Star size` is drawn as set at any size, `Star spacing` and `Position
    /// variation`: the closest spacing is a hair inside the widest read at
    /// full variation, never past it, and a cell is the size times the
    /// spacing.
    #[test]
    fn a_star_is_drawn_at_its_size_and_held_whole_by_its_spacing() {
        for size in [crate::STAR_SIZE_MIN, 15.8, crate::STAR_SIZE_MAX] {
            let tight = StarSettings {
                star_size_far: size,
                star_size_near: size,
                star_spacing_ratio_far: crate::STAR_SPACING_MIN,
                star_spacing_ratio_near: crate::STAR_SPACING_MIN,
                star_jitter: 1.0,
                ..Default::default()
            };
            for depth in tight.plan().depths {
                assert_eq!(depth.radius, 0.5 * size);
                assert!((depth.cell - crate::STAR_SPACING_MIN * size).abs() <= 1e-6 * size);
                let reach = StarGather::Three.bound(1.0) * depth.cell;
                assert!(depth.radius <= reach * (1.0 + 1e-6), "{} past {reach}", depth.radius);
                assert!(depth.radius > reach * 0.999, "the floor is looser than it needs");
            }
        }
    }

    /// Soloing draws only the soloed layers, each as it was, and a flag on a
    /// depth `Star layers` leaves out solos nothing.
    #[test]
    fn soloing_draws_only_the_soloed_layers() {
        let fresh = StarSettings::default();
        let mut solo = [false; STAR_DEPTHS];
        (solo[1], solo[3]) = (true, true);
        let plan = StarSettings { star_solo: solo, ..fresh }.plan();
        for (k, (got, all)) in plan.depths.iter().zip(fresh.plan().depths).enumerate() {
            if solo[k] {
                assert_eq!(*got, all, "depth {k}");
            } else {
                assert_eq!(got.gather, StarGather::Off, "depth {k}");
            }
        }
        // At three layers depths 1 and 3 are not drawn, so neither solos.
        let three = StarSettings { star_layers: 3, ..fresh };
        assert_eq!(StarSettings { star_solo: solo, ..three }.plan(), three.plan());
    }

    /// Every far-to-near pair runs either way: a reversed one survives the
    /// load boundary as it was set, and the far depth then gets the bigger
    /// end.
    #[test]
    fn a_reversed_pair_is_kept_and_drawn_reversed() {
        let fresh = StarSettings::default();
        let reversed = StarSettings {
            star_spacing_ratio_far: fresh.star_spacing_ratio_near,
            star_spacing_ratio_near: fresh.star_spacing_ratio_far,
            star_size_far: fresh.star_size_near,
            star_size_near: fresh.star_size_far,
            star_speed_far: fresh.star_speed_near,
            star_speed_near: fresh.star_speed_far,
            ..fresh
        };
        assert_eq!(reversed.sanitized(), reversed);
        let [far, .., near] = reversed.plan().depths;
        assert!(far.cell > near.cell && far.radius > near.radius);
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
            let cells =
                |far: f32, near: f32| (far * fresh.star_size_far, near * fresh.star_size_near);
            let want = cells(fresh.star_spacing_ratio_far, fresh.star_spacing_ratio_near);
            assert_eq!((far.cell, near.cell), want);
            assert_eq!((far.speed, near.speed), (fresh.star_speed_far, fresh.star_speed_near));
        }
        let middle = StarSettings { star_layers: 3, ..fresh }.plan().depths[2];
        let half = |far: f32, near: f32, curve: f32| far * (near / far).powf(0.5f32.powf(curve));
        let ratio = half(
            fresh.star_spacing_ratio_far,
            fresh.star_spacing_ratio_near,
            fresh.star_spacing_ratio_curve,
        );
        let want = ratio * half(fresh.star_size_far, fresh.star_size_near, fresh.star_size_curve);
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
}
