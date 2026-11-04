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
//!
//! The material and each star layer travel in their OWN cells, each frame
//! adding the rate over that frame's cell. A travel held in cloud units or
//! star pixels would be divided by the cell as it is drawn, so minutes into a
//! session a nudge of `Material size`, `Patch size` or `Star size` would slide
//! the texture by that fraction of everything it had travelled. Counted in
//! cells, a size change scales the texture about its origin instead.

use crate::stars::STAR_SLICES;

/// Where a pane's moving textures are, or, as a rate, how far each goes in a
/// second at this frame's dials ([`Motion::rate`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Motion {
    /// The material's sampling offset in its own cells, less
    /// [`MaterialSettings::DRIFT_START`](harmonigraph_scene::MaterialSettings::DRIFT_START)
    /// in them.
    pub(crate) drift: [f64; 2],
    /// Each star layer's travel in its own cells, across and down.
    pub(crate) stars: [[f64; 2]; STAR_SLICES],
    /// The stars' life clock, in lives.
    pub(crate) life: f64,
    /// The lattice glow's texture clock, in seconds at a `Pattern speed` of 1.
    /// [`Motion::rate`] leaves it still; the lattice sets its rate.
    pub(crate) texture: f64,
    /// The lattice glow's breathing clock, in seconds at a `Breathing speed`
    /// of 1, which [`Breathing::at`](harmonigraph_scene::Breathing::at)
    /// reads. Still at [`Motion::rate`] too.
    pub(crate) breath: f64,
}

impl Motion {
    /// How far each texture goes in a second: the material at `speed` along
    /// `direction`, at `material_cells` of its cells to a cloud unit, each of
    /// the `stars`' layers at `pace` star pixels a second for a `Star speed`
    /// of 1 along the same direction, over its cell of `star_cells` star
    /// pixels, and the life clock once a `Star lifetime`.
    ///
    /// The cells are the ones this frame draws with — a star layer's is the
    /// layout's, which the atlas may have raised past the plan's.
    pub(crate) fn rate(
        speed: f32,
        direction: f32,
        material_cells: f64,
        stars: harmonigraph_scene::StarSettings,
        pace: f64,
        star_cells: [f32; STAR_SLICES],
    ) -> Self {
        let (sin, cos) = f64::from(direction).to_radians().sin_cos();
        let plan = stars.plan();
        Self {
            drift: harmonigraph_scene::MaterialSettings::drift_rate(speed, direction)
                .map(|v| v * material_cells),
            stars: std::array::from_fn(|k| {
                let along = pace * f64::from(plan.depths[k].speed) / f64::from(star_cells[k]);
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

    /// Where the textures were left, for a frame with no clock to advance by.
    pub(crate) fn held(&self) -> Motion {
        self.at
    }
}
