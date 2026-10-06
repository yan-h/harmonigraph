//! How each star depth is drawn: its cell, its stars' radius and shape, and
//! which cells a pixel reads for it.
//!
//! The renderer draws every frame from the [`StarPlan`] its settings give
//! ([`StarSettings::plan`]), worked out afresh each time from the dials, so
//! nothing in it is ever stale.
use crate::StarSettings;

/// How many depths the starfield draws, back (0) to front: each is drawn over
/// the ones before it, which is all a depth's number decides. The renderer's
/// `STAR_SLICES`.
pub const STAR_DEPTHS: usize = 5;
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
    /// The nine cells around the pixel's own, for stars that reach past
    /// the four.
    Three,
}

impl StarGather {
    /// The cheapest first: the order [`StarSettings::plan`] tries them in.
    pub const DRAWN: [Self; 3] = [Self::Core, Self::Two, Self::Three];

    /// How far from its cell's centre a star can reach in cells, on each
    /// axis, and still be seen whole by this read: the read sees a cell from
    /// pixels within this of the cell's centre. A star whose centre strays
    /// `(dx, dy)` from the cell's centre is whole while its radius is at most
    /// this less the larger stray, and the shader's `star_bake` caps every
    /// star's drawn radius there.
    pub fn half_width(self) -> f32 {
        match self {
            Self::Off | Self::Core => 0.5,
            Self::Two => 1.0,
            Self::Three => 1.5,
        }
    }

    /// The cheapest read that holds a star of radius `radius` star pixels
    /// whole at the centre of a cell `cell` wide; 3x3 where none does. The
    /// plan's choice for its stars, and the renderer's for the floor it
    /// widens them to, with the band a centre strays in added so the floor
    /// holds wherever it strays. A star its `Position variation` strays
    /// toward the cell's edge may overrun the read's window, and is drawn
    /// smaller to stay inside it ([`Self::half_width`]).
    pub fn holding(radius: f32, cell: f32) -> Self {
        Self::DRAWN.into_iter().find(|g| radius <= g.half_width() * cell).unwrap_or(Self::Three)
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

/// Where each depth sits from back (0) to front (1) at `Star layers` `layers`,
/// or `None` where it is not drawn. Layer `i` of `n` sits at `i / (n - 1)`,
/// in the depth whose own place at five layers is nearest: two layers are the
/// back and the front, three add the middle.
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
    /// Whether the depth is drawn, and the cheapest read that holds its
    /// largest star whole at its cell's centre; a star strayed toward the
    /// cell's edge is drawn smaller where it would overrun the read
    /// ([`StarGather::half_width`]). The renderer reads it again over the
    /// cell the atlas gives it, with the stars widened to the star image's
    /// texel (`star_slices`), so the read drawn may differ either way.
    pub gather: StarGather,
    /// How fast the depth drifts, as a multiple of the renderer's star pixels
    /// a second: its own `Star speed`.
    pub speed: f32,
    /// The cell one star is hashed into, in star pixels, before the atlas's
    /// floor: the depth's star size times its `Star spacing`.
    pub cell: f32,
    /// The stars' outer radius in star pixels before each star's own size
    /// draw shrinks it: half the depth's `Star size`. [`Self::gather`] holds
    /// it whole at the cell's centre, and the shader caps a star strayed
    /// toward the cell's edge at what the read still holds there; at 3x3 no
    /// star is ever capped, because `Star spacing` never runs below
    /// [`crate::STAR_SPACING_MIN`].
    pub radius: f32,
    /// The star's solid share ([`star_profile`]): its own `Solid`.
    pub solid: f32,
    /// How far its stars fade as one life gives way to the next: its own
    /// `Twinkle`. Below 1 a star keeps its place across its lives.
    pub twinkle: f32,
}

/// Everything the renderer needs to know about how to draw each depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StarPlan {
    pub depths: [StarDepthPlan; STAR_DEPTHS],
}

impl StarSettings {
    /// The plan the renderer draws: every depth's cell and star from the
    /// dials, each gathered by the cheapest read that holds its largest star
    /// whole at its cell's centre.
    pub fn plan(self) -> StarPlan {
        let layers = star_layer_depths(self.star_layers);
        let drawn = |k: usize| layers[k].is_some();
        let depths = std::array::from_fn(|k| {
            let diameter = self.star_size[k];
            let (cell, radius) = (self.star_spacing_ratio[k] * diameter, 0.5 * diameter);
            let gather =
                if !drawn(k) { StarGather::Off } else { StarGather::holding(radius, cell) };
            StarDepthPlan {
                gather,
                speed: self.star_speed[k],
                cell,
                radius,
                solid: self.star_solid[k],
                twinkle: self.star_twinkle[k],
            }
        });
        StarPlan { depths }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each depth is read by the cheapest gather that holds its largest star
    /// whole at its cell's centre, whatever `Position variation` is. At the
    /// fresh dials the far four fit 2x2 (the second-nearest at 0.93 of a
    /// cell) and the nearest, at 1.2, needs 3x3.
    #[test]
    fn each_depth_takes_the_cheapest_gather_that_holds_its_stars() {
        use StarGather::{Three, Two};
        for star_jitter in [0.0, 0.5, 1.0] {
            let fresh = StarSettings { star_jitter, ..Default::default() }.plan();
            let gathers = fresh.depths.map(|depth| depth.gather);
            assert_eq!(gathers, [Two, Two, Two, Two, Three], "jitter={star_jitter}");
        }

        let sparse = StarSettings { star_spacing_ratio: [2.0; STAR_DEPTHS], ..Default::default() };
        assert!(sparse.plan().depths.iter().all(|depth| depth.gather == StarGather::Core));
    }

    /// The closest `Star spacing` is a hair inside what the widest read holds
    /// uncapped at full `Position variation`, never past it, at any size: its
    /// half-width less the half-band a centre strays in, the reach of the
    /// worst-placed star. So a 3x3 depth draws every star at its size, and a
    /// cell is the size times the spacing.
    #[test]
    fn a_star_is_drawn_at_its_size_and_held_whole_by_its_spacing() {
        for size in [crate::STAR_SIZE_MIN, 15.8, crate::STAR_SIZE_MAX] {
            let tight = StarSettings {
                star_size: [size; STAR_DEPTHS],
                star_spacing_ratio: [crate::STAR_SPACING_MIN; STAR_DEPTHS],
                star_jitter: 1.0,
                ..Default::default()
            };
            for depth in tight.plan().depths {
                assert_eq!(depth.radius, 0.5 * size);
                assert!((depth.cell - crate::STAR_SPACING_MIN * size).abs() <= 1e-6 * size);
                assert_eq!(depth.gather, StarGather::Three);
                let stray = 0.5 * star_jitter_width(1.0);
                let reach = (StarGather::Three.half_width() - stray) * depth.cell;
                assert!(depth.radius <= reach * (1.0 + 1e-6), "{} past {reach}", depth.radius);
                assert!(depth.radius > reach * 0.999, "the floor is looser than it needs");
            }
        }
    }

    /// Every far-to-near control runs either way: a reversed one survives
    /// the load boundary as it was set, and the far depth then gets the bigger
    /// end.
    #[test]
    fn a_reversed_control_is_kept_and_drawn_reversed() {
        let fresh = StarSettings::default();
        let reversed = |mut values: [f32; STAR_DEPTHS]| {
            values.reverse();
            values
        };
        let reversed = StarSettings {
            star_spacing_ratio: reversed(fresh.star_spacing_ratio),
            star_size: reversed(fresh.star_size),
            star_speed: reversed(fresh.star_speed),
            ..fresh
        };
        assert_eq!(reversed.sanitized(), reversed);
        let [far, .., near] = reversed.plan().depths;
        assert!(far.cell > near.cell && far.radius > near.radius && far.speed > near.speed);
    }

    /// Fewer layers keep the farthest and the nearest and space the rest
    /// evenly between, each in the depth nearest its place, and each drawn
    /// one takes that depth's own size, spacing, speed, solid and twinkle.
    #[test]
    fn star_layers_keep_both_ends_and_spread_the_rest() {
        let fresh = StarSettings {
            star_solid: [0.1, 0.2, 0.3, 0.4, 0.5],
            star_twinkle: [0.9, 0.8, 0.7, 0.6, 0.5],
            ..Default::default()
        };
        for (layers, drawn) in [
            (2, vec![(0, 0.0), (4, 1.0)]),
            (3, vec![(0, 0.0), (2, 0.5), (4, 1.0)]),
            (4, vec![(0, 0.0), (1, 1.0 / 3.0), (3, 2.0 / 3.0), (4, 1.0)]),
            (5, vec![(0, 0.0), (1, 0.25), (2, 0.5), (3, 0.75), (4, 1.0)]),
        ] {
            let plan = StarSettings { star_layers: layers, ..fresh }.plan();
            let places = star_layer_depths(layers);
            let got: Vec<_> = (0..STAR_DEPTHS).filter_map(|k| places[k].map(|d| (k, d))).collect();
            assert_eq!(got, drawn, "{layers} layers");
            for (k, (depth, place)) in plan.depths.into_iter().zip(places).enumerate() {
                assert_eq!(depth.gather == StarGather::Off, place.is_none(), "{layers}: {k}");
                let size = fresh.star_size[k];
                assert_eq!(depth.cell, fresh.star_spacing_ratio[k] * size, "{layers} layers");
                assert_eq!((depth.radius, depth.speed), (0.5 * size, fresh.star_speed[k]));
                assert_eq!(
                    (depth.solid, depth.twinkle),
                    (fresh.star_solid[k], fresh.star_twinkle[k])
                );
            }
        }
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
