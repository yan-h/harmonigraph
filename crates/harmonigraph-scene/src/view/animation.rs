//! [`NoteAnimationConfig`] and the two enums it is built out of — how a
//! complete slice of the lattice arrives and in what order.

use super::*;

/// Easing of the existing lattice pieces, independent of their ordering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NoteAnimation {
    #[default]
    Fade,
    Pop,
}
impl NoteAnimation {
    /// Every easing, for the settings picker and the sweeps that compare them.
    ///
    /// Built through an exhaustive `match` rather than written out as a bare
    /// literal, so the list cannot fall behind the enum — the same guard every
    /// other `ALL` here uses ([`AnimationOrder::ALL`] below,
    /// `SpectralOrientation::ALL`, `DisplayPage::ALL`), and the reason a test
    /// that sweeps this list is a claim about the enum rather than about the
    /// names someone typed.
    pub const ALL: [Self; 2] = {
        // Exhaustive, and the compiler checks it. The arm is `()` because what
        // is wanted is the coverage error, not the value.
        const fn covered(animation: NoteAnimation) {
            match animation {
                NoteAnimation::Fade | NoteAnimation::Pop => (),
            }
        }
        covered(NoteAnimation::Fade);
        [Self::Fade, Self::Pop]
    };
    pub fn label(self) -> &'static str {
        match self {
            Self::Fade => "Fade",
            Self::Pop => "Pop",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AnimationOrder {
    #[default]
    Simultaneous,
    Circular,
    Bidirectional,
    RandomStagger,
    OddEvenStagger,
}
impl AnimationOrder {
    /// Every order, for the settings picker and the sweeps that compare them.
    /// Guarded the way [`NoteAnimation::ALL`] above is, and for its reason.
    pub const ALL: [Self; 5] = {
        const fn covered(order: AnimationOrder) {
            use AnimationOrder::*;
            match order {
                Simultaneous | Circular | Bidirectional | RandomStagger | OddEvenStagger => (),
            }
        }
        covered(AnimationOrder::Simultaneous);
        [
            Self::Simultaneous,
            Self::Circular,
            Self::Bidirectional,
            Self::RandomStagger,
            Self::OddEvenStagger,
        ]
    };
    pub fn label(self) -> &'static str {
        match self {
            Self::Simultaneous => "Simultaneous",
            Self::Circular => "Circular",
            Self::Bidirectional => "Bidirectional",
            Self::RandomStagger => "Random stagger",
            Self::OddEvenStagger => "Odd/even stagger",
        }
    }
}
/// Starting pose of complete slices. Radial -1 places each anchor at the node centre.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NoteAnimationConfig {
    pub animation: NoteAnimation,
    pub order: AnimationOrder,
    pub stagger_spread: f32,
    pub radial_start: f32,
    pub start_size: f32,
}
impl Default for NoteAnimationConfig {
    fn default() -> Self {
        Self {
            animation: NoteAnimation::Fade,
            order: AnimationOrder::Simultaneous,
            stagger_spread: 0.28,
            radial_start: 0.0,
            start_size: 1.0,
        }
    }
}
impl NoteAnimationConfig {
    pub fn staggers(self) -> bool {
        self.order != AnimationOrder::Simultaneous && self.stagger_spread > 0.0
    }
    /// The finite, bounded pose the renderer is handed, for the shells that
    /// never cross the persist door — `derive_scene` copies this config into
    /// the scene whole, so a NaN in it is a node the shader places nowhere.
    ///
    /// The fallback is the FRESH value rather than each range's low bound,
    /// which is the departure the rest of the picture's repairs do not make
    /// and is the same one [`GlowCurve::sanitized`] makes. These three are a
    /// POSE rather than a size: `radial_start`'s range is signed and
    /// `start_size`'s neutral is 1, so a low bound here would be one extreme
    /// of an animation rather than the least of one. Fresh is also the answer
    /// [`ViewConfig::sanitize`] gives each of them, so the door and the
    /// picture cannot disagree about what a broken pose looks like.
    pub fn sanitized(mut self) -> Self {
        let fresh = NoteAnimationConfig::default();
        self.stagger_spread = finite_or(self.stagger_spread, fresh.stagger_spread).clamp(0.0, 0.9);
        self.radial_start = finite_or(self.radial_start, fresh.radial_start).clamp(-1.0, 1.0);
        self.start_size = finite_or(self.start_size, fresh.start_size).clamp(0.0, 2.0);
        self
    }
    /// Fixed delays of complete displayed sectors; shared by live/export and
    /// renderer fixtures, including wheels with unequal outer sectors.
    ///
    /// A delay is a START OFFSET and nothing else: every sector still animates
    /// for the whole `duration`, so the spread widens the total to
    /// `duration * (1 + stagger_spread)` rather than dividing one fade time
    /// between waiting and moving. Compressing instead is what made a high
    /// spread read as two different animations -- the first sector fading over
    /// the full time because the level ramp under it was never staggered, the
    /// last one snapping in over what little time the spread had left it.
    pub fn delays(
        self,
        layout: &crate::OctaveLayout,
        cents: f32,
        seed: u32,
        duration: f32,
    ) -> [f32; 11] {
        let span = layout.span as usize;
        let ring = layout.ring(cents);
        let start = (((layout.center - layout.slot_pitch(ring.base, cents)) / 12.0 + 0.5).floor()
            as usize)
            .min(span - 1);
        let mut ranks = [0.0f32; 11];
        for (i, rank) in ranks.iter_mut().enumerate().take(span) {
            let clockwise = (i + span - start) % span;
            *rank = match self.order {
                AnimationOrder::Simultaneous => 0.0,
                AnimationOrder::Circular => clockwise as f32 / (span - 1).max(1) as f32,
                AnimationOrder::Bidirectional => {
                    let (a, b) = layout.sector(ring.base + i as i32, cents);
                    let angle = (std::f32::consts::FRAC_PI_2 - (a + b) * 0.5)
                        .rem_euclid(std::f32::consts::TAU);
                    if i == start {
                        0.0
                    } else {
                        angle.min(std::f32::consts::TAU - angle) / std::f32::consts::PI
                    }
                }
                AnimationOrder::OddEvenStagger => (clockwise % 2) as f32,
                AnimationOrder::RandomStagger => {
                    let mut x = seed.wrapping_add((clockwise as u32 + 1).wrapping_mul(0x9e3779b9));
                    x ^= x >> 16;
                    x = x.wrapping_mul(0x7feb352d);
                    x ^= x >> 15;
                    (x & 65535) as f32 / 65535.0
                }
            };
        }
        let min = ranks[..span].iter().copied().fold(f32::INFINITY, f32::min);
        let max = ranks[..span].iter().copied().fold(f32::NEG_INFINITY, f32::max);
        for rank in &mut ranks[..span] {
            *rank = if max > min {
                (*rank - min) / (max - min) * self.stagger_spread * duration
            } else {
                0.0
            };
        }
        ranks
    }
    pub fn moves(self) -> bool {
        self.animation == NoteAnimation::Pop || self.radial_start != 0.0 || self.start_size != 1.0
    }
    /// Fixed across animation frames: only settings change allocation bounds.
    pub fn reach(self, rim: f32) -> f32 {
        if !self.moves() {
            return rim;
        }
        let extent = |ease: f32| {
            let scale = self.start_size + (1.0 - self.start_size) * ease;
            scale.abs() + (1.0 + self.radial_start * (1.0 - ease) - scale).abs()
        };
        let pop = self.animation == NoteAnimation::Pop;
        // Coupled anchor/scale bounds keep Grow (-1,0) at the ordinary
        // radius, plus only the Pop curve's small actual overshoot.
        rim * extent(0.0).max(extent(if pop { 1.046 } else { 1.0 }))
            + if pop { rim * 0.09 * self.start_size } else { 0.0 }
    }
}
