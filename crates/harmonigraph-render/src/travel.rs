//! How far a pane's texture has moved, built up frame by frame.
//!
//! The textures here move at a rate their dials set: a material drifts at
//! `Drift speed` along `Drift direction`, each star layer travels at its own
//! `Star speed` along the same direction, the stars' lives turn over once a
//! `Star lifetime`, and on the lattice the glow's texture turns at
//! `Pattern speed` and its light breathes at `Breathing speed`. Worked out as
//! `now` times the rate,
//! every nudge of one of those dials would carry the texture as far as it had
//! gone since the clock started, which minutes into a session scrambles the
//! whole field on every frame of a drag. So each pane holds one [`Travel`],
//! and every frame adds what the frame's own rates carry it in the time since
//! the last: a dial change alters the pace from where the texture is.
//!
//! One clock for all of them, held in each pane's own buffers, which outlive
//! the targets that draw them. So a rebuilt target (a resize, a Span zoom, a
//! change of style) goes on from where its texture was.

use crate::stars::STAR_SLICES;

/// Where a pane's moving textures are, or, as a rate, how far each goes in a
/// second at this frame's dials ([`Motion::rate`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Motion {
    /// The material's sampling offset in cloud units, less
    /// [`MaterialSettings::DRIFT_START`](harmonigraph_scene::MaterialSettings::DRIFT_START).
    pub(crate) drift: [f64; 2],
    /// Each star layer's travel in star pixels, across and down.
    pub(crate) stars: [[f64; 2]; STAR_SLICES],
    /// The stars' life clock, in lives.
    pub(crate) life: f64,
    /// The lattice glow's texture clock, in seconds at a `Pattern speed` of 1.
    /// [`Motion::rate`] leaves it still; the lattice sets its rate.
    pub(crate) texture: f64,
    /// The lattice glow's breathing clock, in seconds at a `Breathing speed`
    /// of 1, which [`AtmosphereSettings::breath`](harmonigraph_scene::AtmosphereSettings::breath)
    /// reads. Still at [`Motion::rate`] too.
    pub(crate) breath: f64,
}

impl Motion {
    /// How far each texture goes in a second: the material at `speed` along
    /// `direction`, each of the `stars`' layers at `pace` star pixels a second
    /// for a `Star speed` of 1 along the same direction, and the life clock
    /// once a `Star lifetime`.
    pub(crate) fn rate(
        speed: f32,
        direction: f32,
        stars: harmonigraph_scene::StarSettings,
        pace: f64,
    ) -> Self {
        let (sin, cos) = f64::from(direction).to_radians().sin_cos();
        Self {
            drift: harmonigraph_scene::MaterialSettings::drift_rate(speed, direction),
            stars: stars.plan().depths.map(|depth| {
                let along = pace * f64::from(depth.speed);
                [along * cos, along * sin]
            }),
            life: 1.0 / f64::from(stars.star_lifetime),
            texture: 0.0,
            breath: 0.0,
        }
    }

    /// This plus `rate` for `seconds`.
    fn after(self, rate: Self, seconds: f64) -> Self {
        let on = |at: f64, rate: f64| at + rate * seconds;
        Self {
            drift: std::array::from_fn(|a| on(self.drift[a], rate.drift[a])),
            stars: std::array::from_fn(|k| {
                std::array::from_fn(|a| on(self.stars[k][a], rate.stars[k][a]))
            }),
            life: on(self.life, rate.life),
            texture: on(self.texture, rate.texture),
            breath: on(self.breath, rate.breath),
        }
    }
}

/// A pane's [`Motion`], and the clock it was last advanced to.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Travel {
    clock: Option<f64>,
    at: Motion,
}

impl Travel {
    /// Where the textures are at `now`: where they were plus the time since
    /// the last frame at this frame's `rate`. A first frame, or a clock that
    /// ran backwards, as a render started over does, takes all of `now` at
    /// `rate` from the start, so a render is the same from its first frame
    /// however it was reached, and an export at fixed dials draws what `now`
    /// times its rates names.
    pub(crate) fn advance(&mut self, now: f64, rate: Motion) -> Motion {
        self.at = match self.clock {
            Some(clock) if now >= clock => self.at.after(rate, now - clock),
            _ => Motion::default().after(rate, now),
        };
        self.clock = Some(now);
        self.at
    }
}
