//! Trails: a record of every node the music has already been to, so a
//! piece's harmonic territory accumulates on screen as it plays.
//!
//! The governing constraint is that a trail must never be mistaken for a
//! note, and the trail is drawn entirely in TYPE: the label layer keeps the
//! note name and cents on a visited node. Nothing here touches a drawn
//! layer — not the core, not the octave glyphs — so a memory and a sounding
//! note are never even the same kind of thing on screen. The whole of what
//! this module produces is [`NodeInstance::trail`], which the labels read.
//!
//! The resting DOT is the one drawn thing a memory reaches, and it reaches it
//! through the name rather than around it: a named position draws no dot (see
//! [`NodeInstance::is_named`]), and under
//! [`NoteNames::Past`] a remembered position is named.
//! So the rule above holds as written — this module still writes one field —
//! and what takes the dot away is the label layer, on the same terms it takes
//! one away from a node that is merely sounding. A memory is still type and
//! nothing else; what changed is that type now stands where the dot was
//! instead of over it.
//!
//! Two consequences worth stating, because they are what keep it subtle:
//! - Nothing on a visited node ever brightens or animates. The lattice at
//!   rest reads exactly as it did, one marker swapped for a more specific
//!   one.
//! - Only the home sheet carries a memory. The whole history is remembered —
//!   every note played, wherever it landed — but an off-sheet idle node stays
//!   blank: a lone name floating out in the sevens dimension reads as noise,
//!   not as territory. The memory shows on the home node of the same pitch
//!   class, which is where the eye is anyway.

use harmonigraph_core::{NoteHistory, PitchClass, Tuning};

use crate::style::NoteNames;
use crate::view::ViewConfig;
use crate::NodeInstance;

/// The frame's memories, reduced to what a node needs: which pitch classes
/// the music has been to.
pub(crate) struct TrailField {
    marks: Vec<PitchClass>,
}

impl TrailField {
    /// Reduce `history` to this frame's marks, or `None` when nothing is
    /// remembered or nothing reads a memory.
    ///
    /// [`NoteNames::Past`] IS the trail's on/off, the kept names being the
    /// whole of what a memory draws: under any other mode nothing reads
    /// `trail`, so filling it would be per-frame work for a field no layer
    /// looks at. [`NoteNames::All`] names every node without asking where the
    /// music has been, so it is a mode of the LABEL layer and takes no
    /// memory either.
    pub(crate) fn build(history: &NoteHistory, view: &ViewConfig) -> Option<TrailField> {
        if view.note_names != NoteNames::Past || history.is_empty() {
            return None;
        }
        // A memory never fades: the point of the feature is a whole piece's
        // territory rather than a rolling window, so every visit counts the
        // same however long ago it sounded.
        let marks: Vec<_> = history.visits().map(|visit| visit.pitch_class).collect();
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
            for &pitch_class in &self.marks {
                // Full strength or nothing: a memory has no level of its own
                // to carry, and two remembered pitches matching one node
                // under a wide tolerance are the same node visited twice
                // rather than a stronger memory.
                if tuning.matches(pitch_class, node_pc) {
                    node.trail = 1.0;
                    break;
                }
            }
        }
    }
}
