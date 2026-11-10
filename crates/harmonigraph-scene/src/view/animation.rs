//! [`NoteAnimationConfig`] — how complete lattice slices arrive.

use super::*;

/// Shared bar and load range for [`NoteAnimationConfig::stagger_spread`].
pub const STAGGER_SPREAD_RANGE: std::ops::RangeInclusive<f32> = 0.0..=0.9;

/// Shared bar and load range for [`NoteAnimationConfig::radial_start`].
pub const RADIAL_START_RANGE: std::ops::RangeInclusive<f32> = -1.0..=1.0;

/// Shared bar and load range for [`NoteAnimationConfig::fade_out_delay`], in
/// seconds. The two-second ceiling is a first guess, not a measured limit;
/// [`crate::NodeMotion`]'s replay horizon and the roll cursors it retains
/// follow it, so raising it needs nothing else.
pub const FADE_OUT_DELAY_RANGE: std::ops::RangeInclusive<f32> = 0.0..=2.0;

/// Starting pose of complete slices. Radial -1 places each anchor at the node centre.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct NoteAnimationConfig {
    pub stagger_spread: f32,
    pub radial_start: f32,
    /// Seconds a node whose last note is released waits before its slices
    /// start to retract and its presence to fall. Its colour does not wait,
    /// so the wheel stands uncoloured for this long. A note on the node again
    /// inside the wait takes it back from where it stands.
    pub fade_out_delay: f32,
}
impl Default for NoteAnimationConfig {
    fn default() -> Self {
        Self { stagger_spread: 0.602_636_7, radial_start: -0.034_186_72, fade_out_delay: 0.0 }
    }
}
impl NoteAnimationConfig {
    pub fn staggers(self) -> bool {
        self.stagger_spread > 0.0
    }
    /// Repair the pose at view normalization. Nonfinite values use the fresh
    /// pose: the signed offset's low bound would be an extreme, not neutral.
    pub fn sanitized(mut self) -> Self {
        let fresh = NoteAnimationConfig::default();
        self.stagger_spread = finite_or(self.stagger_spread, fresh.stagger_spread)
            .clamp(*STAGGER_SPREAD_RANGE.start(), *STAGGER_SPREAD_RANGE.end());
        self.radial_start = finite_or(self.radial_start, fresh.radial_start)
            .clamp(*RADIAL_START_RANGE.start(), *RADIAL_START_RANGE.end());
        // 0 rather than the fresh value: no wait is the inert setting.
        self.fade_out_delay = finite_or(self.fade_out_delay, 0.0)
            .clamp(*FADE_OUT_DELAY_RANGE.start(), *FADE_OUT_DELAY_RANGE.end());
        self
    }
    /// Fixed delays of complete displayed sectors; shared by live/export and
    /// renderer fixtures, including wheels with unequal outer sectors.
    ///
    /// Both ends start at the seam where the highest and lowest pitch meet and
    /// sweep toward each other across the ring.
    ///
    /// A delay is a START OFFSET and nothing else: every sector still animates
    /// for the whole `duration`, so the spread widens the total to
    /// `duration * (1 + stagger_spread)` rather than dividing one fade time
    /// between waiting and moving. Compressing instead is what made a high
    /// spread read as two different animations -- the first sector fading over
    /// the full time because the level ramp under it was never staggered, the
    /// last one snapping in over what little time the spread had left it.
    pub fn delays(self, layout: &crate::OctaveLayout, duration: f32) -> [f32; 11] {
        let span = layout.span as usize;
        let mut ranks = [0.0f32; 11];
        for (i, rank) in ranks.iter_mut().enumerate().take(span) {
            // Slice zero is the lowest displayed pitch and `i` walks upward.
            // The walk is measured from the seam, not from a fixed screen
            // angle. Mirror the index before measuring so each opposing pair
            // has exactly one rank; independently measuring both sides lets
            // f32 roundoff turn a two-slice tie into the whole normalized
            // spread.
            let from_seam = i.min(span - 1 - i);
            *rank = (layout.walk(from_seam as f32) + layout.walk(from_seam as f32 + 1.0)) * 0.5
                / std::f32::consts::PI;
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
    /// Seconds a node takes to leave once its last note has gone, for a Fade
    /// of `fade_time`: the fade-out delay, then the [`delays`](Self::delays)
    /// sweep, whose last slice starts `stagger_spread` of a fade after the
    /// first and then runs a whole fade.
    ///
    /// This summarizes what [`crate::NodeMotion`] does rather than deciding
    /// it: its departure edges and presence wait carry the per-slice delays,
    /// and the widest of those is `stagger_spread` of a fade on any wheel
    /// whose slices are staggered at all (an unstaggered one leaves sooner).
    /// The repaint guard keeps the frame loop awake this long, and the replay
    /// horizon is built on this at the spread's ceiling, so a setting that
    /// lengthens a departure joins the span here and both follow it.
    pub fn departure(self, fade_time: f32) -> f64 {
        f64::from(self.fade_out_delay)
            + f64::from(fade_time) * (1.0 + f64::from(self.stagger_spread))
    }
    pub fn moves(self) -> bool {
        self.radial_start != 0.0
    }
    /// Scale paired with the starting offset so a slice and the gaps around it
    /// keep the same proportions throughout the radial move.
    pub fn starting_scale(self) -> f32 {
        1.0 + self.radial_start
    }
    /// Fixed across animation frames: only settings change allocation bounds.
    pub fn reach(self, rim: f32) -> f32 {
        if !self.moves() {
            return rim;
        }
        rim * self.starting_scale().max(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_scale_tracks_offset_and_preserves_the_pose_proportion() {
        for offset in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            let config = NoteAnimationConfig { radial_start: offset, ..Default::default() };
            assert_eq!(config.starting_scale(), 1.0 + offset);
            assert_eq!(config.reach(1.0), (1.0 + offset).max(1.0));
        }
    }

    #[test]
    fn slices_start_at_the_low_high_seam_and_meet_across_the_ring() {
        for count in [2, 7, 6, 9] {
            let layout = crate::octave_layout(count, 64.5);
            let span = layout.span as usize;
            let both = NoteAnimationConfig { stagger_spread: 0.8, ..Default::default() }
                .delays(&layout, 2.0);
            assert!(both[0].abs() < 1e-6);
            assert!(both[span - 1].abs() < 1e-6);
            for i in 0..span / 2 {
                assert!(
                    (both[i] - both[span - 1 - i]).abs() < 1e-5,
                    "asymmetric seam walk for {count}: {both:?}",
                );
                if i + 1 < span / 2 {
                    assert!(both[i] < both[i + 1]);
                }
            }
        }
    }
}
