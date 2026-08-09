//! Trails: a record of every node the music has already been to, so a
//! piece's harmonic territory accumulates on screen as it plays.
//!
//! The governing constraint is that a trail must never be mistaken for a
//! note, and the trail is drawn entirely in TYPE: the label layer keeps the
//! note name and cents on a visited node. Nothing here touches a drawn
//! layer — not the core, not the octave glyphs, not the grid — so a memory
//! and a sounding note are never even the same kind of thing on screen. The
//! whole of what this module produces is [`NodeInstance::trail`], which the
//! labels read.
//!
//! Two consequences worth stating, because they are what keep it subtle:
//! - Nothing on a visited node ever brightens or animates. The lattice at
//!   rest reads exactly as it did, with a little more information in it.
//! - Only the home sheet carries a memory. The whole history is remembered —
//!   every note played, wherever it landed — but an off-sheet idle node stays
//!   blank: a lone name floating out in the sevens dimension reads as noise,
//!   not as territory. The memory shows on the home node of the same pitch
//!   class, which is where the eye is anyway.

use harmonigraph_core::{NoteHistory, PitchClass, Tuning};

use crate::view::ViewConfig;
use crate::NodeInstance;

/// The frame's memories, reduced to what a node needs: which pitch class,
/// and how strongly.
pub(crate) struct TrailField {
    marks: Vec<(PitchClass, f32)>,
}

/// Below this a mark is invisible anyway, and dropping it here saves the
/// per-node matching it would cost.
const MIN_LEVEL: f32 = 0.01;

impl TrailField {
    /// Reduce `history` to this frame's marks, or `None` when the trail is
    /// off or has nothing to show.
    ///
    /// `trail_labels` IS the trail's on/off, the names being the whole of
    /// what a memory draws: with them off nothing reads `trail`, so filling
    /// it would be per-frame work for a field no layer looks at.
    pub(crate) fn build(history: &NoteHistory, view: &ViewConfig, now: f64) -> Option<TrailField> {
        if !view.trail_labels || history.is_empty() {
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
                (level >= MIN_LEVEL).then_some((visit.pitch_class, level))
            })
            .collect();
        (!marks.is_empty()).then_some(TrailField { marks })
    }

    /// Mark the nodes the music has been to. `node_pcs` is each node's pitch
    /// class, parallel to `nodes`.
    ///
    /// `trail` is the ONLY field written — not the color, not the activation,
    /// not an octave. Every field that means "is sounding" is left alone,
    /// which is what keeps a memory from reading as a note, and a memory
    /// reaches the screen through the label layer alone.
    pub(crate) fn apply(
        &self,
        nodes: &mut [NodeInstance],
        node_pcs: &[PitchClass],
        tuning: &Tuning,
    ) {
        for (node, &node_pc) in nodes.iter_mut().zip(node_pcs) {
            // Only the home sheet carries trails. The music's whole history is
            // remembered (every note reaches `self.marks`), but an off-sheet
            // node is deliberately blank at idle — a lone name floating out
            // in the sevens dimension reads as noise, not as territory — so the
            // memory is shown on the home node of the same pitch class instead.
            if !node.on_home {
                continue;
            }
            for &(pitch_class, level) in &self.marks {
                // `max`, not a sum: two remembered pitches matching one node
                // under a wide tolerance are the same node visited twice,
                // not a stronger memory.
                if level > node.trail && tuning.matches(pitch_class, node_pc) {
                    node.trail = level;
                }
            }
        }
    }
}
