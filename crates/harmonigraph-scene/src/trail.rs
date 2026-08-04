//! Trails: a quiet mark on every node the music has already been to, so a
//! piece's harmonic territory accumulates on screen as it plays.
//!
//! The governing constraint is that a trail must never be mistaken for a
//! note. So it does not touch the active layers at all — not the core, not
//! the octave glyphs, not the grid. It changes the **idle marker**: the
//! small grey mark already sitting at every home-sheet node. A visited node
//! wears a slightly different version of that same mark ([`TrailMark`]),
//! which is as quiet as a difference can be while still being a difference.
//!
//! Two consequences worth stating, because they are what keep it subtle:
//! - Nothing on a visited node ever brightens or animates. The lattice at
//!   rest reads exactly as it did, with a little more information in it.
//! - Only the home sheet carries the marks. The whole history is remembered —
//!   every note played, wherever it landed — but an off-sheet idle node stays
//!   blank: a lone mark floating out in the sevens dimension reads as noise,
//!   not as territory. The memory shows on the home node of the same pitch
//!   class, which is where the eye is anyway.
//!
//! The other half of the feature is the label layer keeping note names and
//! cents on visited nodes, which needs nothing here beyond
//! [`NodeInstance::trail`].

use glam::Vec4;
use harmonigraph_core::{NoteHistory, PitchClass, Tuning};

use crate::color::channel_color;
use crate::view::{FrameParams, ViewConfig};
use crate::NodeInstance;

/// What a node the music has visited looks like while nothing is sounding
/// there. Each is a small change to the idle marker rather than a mark of
/// its own — except [`Ring`](TrailMark::Ring), which draws its own circle
/// so that it still reads with the idle marker turned off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum TrailMark {
    /// Remember nothing; the lattice shows only what is sounding.
    #[default]
    Off,
    /// The node's idle marker draws a lighter grey. The quietest of the
    /// three: no new shape, no color, just a little more presence.
    Lift,
    /// A pale circle around the node, where its sounding disc would be —
    /// a ghost of the note that was there.
    Ring,
    /// The idle marker keeps a hint of the color the note was played in, at
    /// idle brightness. Says not just that the music was here but what it
    /// was doing — a bass note and a melody note leave different marks.
    Tint,
}

impl TrailMark {
    /// Index the shader reads (uniform `misc6.x`).
    pub fn shader_index(self) -> u32 {
        match self {
            TrailMark::Off => 0,
            TrailMark::Lift => 1,
            TrailMark::Ring => 2,
            TrailMark::Tint => 3,
        }
    }

    /// Whether this mark works by changing the idle marker, and so needs
    /// one to be showing. The UI says so rather than leaving a setting that
    /// silently does nothing.
    pub fn needs_idle_marker(self) -> bool {
        matches!(self, TrailMark::Lift | TrailMark::Tint)
    }
}

/// The frame's memories, reduced to what a node needs: which pitch class,
/// how strongly, and in what color.
pub(crate) struct TrailField {
    marks: Vec<(PitchClass, f32, Vec4)>,
}

/// Below this a mark is invisible anyway, and dropping it here saves the
/// per-node matching it would cost.
const MIN_LEVEL: f32 = 0.01;

impl TrailField {
    /// Reduce `history` to this frame's marks, or `None` when the trail is
    /// off or has nothing to show.
    pub(crate) fn build(
        history: &NoteHistory,
        view: &ViewConfig,
        frame: &FrameParams,
        now: f64,
    ) -> Option<TrailField> {
        if view.trail_mark == TrailMark::Off || history.is_empty() {
            return None;
        }
        // 0 means never forget, which is the point of the feature: the
        // whole piece's territory. A positive span fades a pitch out over
        // that many seconds since it last sounded.
        let span = f64::from(view.trail_memory.max(0.0));
        let marks: Vec<_> = history
            .visits()
            .filter_map(|visit| {
                let level = if span <= 0.0 {
                    1.0
                } else {
                    let age = (now - visit.last_off).max(0.0);
                    (1.0 - age / span).clamp(0.0, 1.0) as f32
                };
                (level >= MIN_LEVEL).then(|| {
                    let color = channel_color(
                        visit.channel,
                        visit.pitch,
                        frame.darkest_pitch,
                        frame.brightest_pitch,
                        view.pitch_palette,
                    );
                    (visit.pitch_class, level, color)
                })
            })
            .collect();
        (!marks.is_empty()).then_some(TrailField { marks })
    }

    /// Mark the nodes the music has been to. `node_pcs` is each node's pitch
    /// class, parallel to `nodes`.
    ///
    /// Only `trail` is written, plus — on a node with nothing sounding —
    /// `color`, which the idle layer reads for [`TrailMark::Tint`] and which
    /// no active layer looks at while the node is silent. Every layer that
    /// means "is sounding" is left alone, which is what keeps a memory from
    /// reading as a note.
    pub(crate) fn apply(
        &self,
        nodes: &mut [NodeInstance],
        node_pcs: &[PitchClass],
        tuning: &Tuning,
    ) {
        for (node, &node_pc) in nodes.iter_mut().zip(node_pcs) {
            // Only the home sheet carries trails. The music's whole history is
            // remembered (every note reaches `self.marks`), but an off-sheet
            // node is deliberately blank at idle — a lone marker floating out
            // in the sevens dimension reads as noise, not as territory — so the
            // memory is shown on the home node of the same pitch class instead.
            if !node.on_home {
                continue;
            }
            for &(pitch_class, level, color) in &self.marks {
                // `max`, not a sum: two remembered pitches matching one node
                // under a wide tolerance are the same node visited twice,
                // not a stronger memory.
                if level > node.trail && tuning.matches(pitch_class, node_pc) {
                    node.trail = level;
                    if node.activation <= 0.0 {
                        node.color = color;
                    }
                }
            }
        }
    }
}
