//! Trails: what the piece has already played, left behind on the lattice.
//!
//! The live layers answer "what is sounding now". This one answers "where
//! has the music been" — so a piece's harmonic territory accumulates on
//! screen as it goes, and you can see the space it travelled rather than
//! only the instant it is in.
//!
//! Two independent devices, because they say different things and compose:
//! - The **marks** ([`TrailMode`]): a faint, steady presence left on each
//!   node the music has visited. *Where* it has been.
//! - The **path** ([`ViewConfig::trail_path`]): a line from each played
//!   note to the next. *In what order* it went there.
//!
//! Both are drawn through channels the renderer already has, which is why
//! neither needs a shader branch of its own. A mark is written into the
//! node's own `activation`/`octaves`/`color` (a trail is exactly a node
//! that is a little bit lit), and the path into [`EdgeInstance`]s. What
//! keeps the two tellable apart on screen is behavior, not machinery: a
//! live note decays and a trail does not.

use glam::Vec4;
use lattice_core::{NoteHistory, PitchClass, Tuning};

use crate::color::{channel_color, idle_color};
use crate::view::{FrameParams, ViewConfig};
use crate::{EdgeInstance, NodeInstance, OCTAVE_SLOTS};

/// How the lattice remembers a note once it has finished sounding. Every
/// mode leaves the same kind of mark — a dim, steady version of the note —
/// and differs only in how strongly each visited pitch is weighted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum TrailMode {
    /// Remember nothing; the lattice shows only what is sounding.
    #[default]
    Off,
    /// Every pitch ever played keeps the same faint presence, forever. The
    /// plain reading of "leave the notes visible": at the end of a piece
    /// the lit nodes ARE its harmonic territory, with no ranking implied.
    Ghost,
    /// Weighted by recency: a pitch is brightest as its live fade lets go
    /// and forgotten [`ViewConfig::trail_time`] seconds later. Shows where
    /// the music is *now* within its territory — a moving comet tail rather
    /// than a map.
    Fade,
    /// Weighted by dwell: brightest where the music has spent the most
    /// time. What a tonal center looks like — the notes a piece keeps
    /// returning to and sitting on stand out from the ones it passes
    /// through.
    Heat,
}

impl TrailMode {
    /// Whether [`ViewConfig::trail_time`] means anything in this mode.
    /// Ghost never forgets and Heat weighs totals, so only Fade reads it
    /// (the path reads it whatever the mode is).
    pub fn uses_time(self) -> bool {
        self == TrailMode::Fade
    }
}

/// One remembered pitch, reduced to what the nodes need: how strongly to
/// draw it, in what color, and in which octave slot.
struct TrailMark {
    pitch_class: PitchClass,
    /// Octave indicator slot, as `NodeInstance::octaves`.
    slot: usize,
    level: f32,
    color: Vec4,
}

/// The frame's trail marks, ready to be laid over the nodes.
pub(crate) struct TrailField {
    marks: Vec<TrailMark>,
}

/// Below this a mark is invisible anyway, and dropping it here saves the
/// per-node matching work it would cost.
const MIN_LEVEL: f32 = 0.004;

impl TrailField {
    /// Reduce `history` to this frame's marks, or `None` when the trail is
    /// off or would draw nothing.
    pub(crate) fn build(
        history: &NoteHistory,
        view: &ViewConfig,
        frame: &FrameParams,
        now: f64,
    ) -> Option<TrailField> {
        let strength = view.trail_strength.clamp(0.0, 1.0);
        if view.trail_mode == TrailMode::Off || strength <= 0.0 || history.is_empty() {
            return None;
        }
        let span = f64::from(view.trail_time.max(0.01));
        let peak = history.peak_heat();
        let idle = idle_color(view);
        let tint = view.trail_tint.clamp(0.0, 1.0);

        let mut marks = Vec::new();
        for visit in history.visits() {
            let weight = match view.trail_mode {
                TrailMode::Off => return None,
                TrailMode::Ghost => 1.0,
                TrailMode::Fade => {
                    let age = (now - visit.last_off).max(0.0);
                    (1.0 - age / span).clamp(0.0, 1.0)
                }
                // Square-rooted so one long pedal tone doesn't flatten
                // everything else to nothing: a note played a hundredth as
                // long still reads at a tenth the brightness, which is the
                // difference between a visible map and a single dot.
                TrailMode::Heat => {
                    if peak <= 0.0 {
                        0.0
                    } else {
                        (visit.heat_weight() / peak).clamp(0.0, 1.0).sqrt()
                    }
                }
            };
            let level = strength * weight as f32;
            if level < MIN_LEVEL {
                continue;
            }
            marks.push(TrailMark {
                pitch_class: visit.pitch_class,
                slot: visit.octave.clamp(0, OCTAVE_SLOTS as i8 - 1) as usize,
                level,
                // The note's own color, pulled toward the idle structure by
                // the tint: a memory should read as part of the lattice's
                // furniture, not as a note that is quietly still sounding.
                color: channel_color(
                    visit.channel,
                    visit.pitch,
                    frame.darkest_pitch,
                    frame.brightest_pitch,
                )
                .lerp(idle, tint),
            });
        }
        (!marks.is_empty()).then_some(TrailField { marks })
    }

    /// Lay the marks over already-derived nodes. `node_pcs` is each node's
    /// pitch class, parallel to `nodes`.
    ///
    /// Must run AFTER everything that reads `activation` as "is sounding" —
    /// the grid's sevens chains, above all. A trail is memory, not sound;
    /// it should not light the structure the way a played note does.
    pub(crate) fn apply(
        &self,
        nodes: &mut [NodeInstance],
        node_pcs: &[PitchClass],
        tuning: &Tuning,
        octaves: bool,
    ) {
        for (node, &node_pc) in nodes.iter_mut().zip(node_pcs) {
            for mark in &self.marks {
                if !tuning.matches(mark.pitch_class, node_pc) {
                    continue;
                }
                if octaves {
                    node.octaves[mark.slot] = node.octaves[mark.slot].max(mark.level);
                }
                // The strongest memory on this node is its trail. `max`
                // rather than a sum: two pitches matching one node under a
                // wide tolerance are the same node lit twice, not twice as
                // bright a memory.
                if mark.level > node.trail {
                    node.trail = mark.level;
                    // A live note always paints over its own memory — it is
                    // the same pitch, sounding now.
                    if mark.level > node.activation {
                        node.activation = mark.level;
                        node.color = mark.color;
                    }
                }
            }
        }
    }
}

/// The route: a segment from each recently played note to the next, so the
/// order the music moved through the lattice is readable and not just the
/// set of places it reached.
///
/// A pitch class can light several nodes at once (comma-equivalent
/// spellings, or a wide tuning tolerance), so "the note's position" is not
/// a given. The walk resolves it greedily: each step goes to whichever
/// matching node is nearest the previous one, which draws the shortest
/// route consistent with what was played — and short lattice moves are
/// exactly what close harmonic motion is.
pub(crate) fn derive_trail_path(
    history: &NoteHistory,
    tuning: &Tuning,
    view: &ViewConfig,
    frame: &FrameParams,
    nodes: &[NodeInstance],
    node_pcs: &[PitchClass],
    now: f64,
) -> Vec<EdgeInstance> {
    let strength = view.trail_strength.clamp(0.0, 1.0);
    if !view.trail_path || strength <= 0.0 || nodes.is_empty() {
        return Vec::new();
    }
    let span = f64::from(view.trail_time.max(0.01));
    let keep = view.trail_path_steps.clamp(2, NoteHistory::MAX_STEPS as u32) as usize;
    let idle = idle_color(view);
    let tint = view.trail_tint.clamp(0.0, 1.0);
    let center = view.center();

    // The last `keep` onsets, back in playing order.
    let recent: Vec<_> = history.steps().rev().take(keep).collect();
    let mut path = Vec::new();
    let mut prev: Option<(usize, f32, Vec4)> = None;
    for step in recent.into_iter().rev() {
        // Older than the trail time: nothing to draw, and since steps are
        // in playing order every one of these precedes the visible tail.
        let age = (now - step.on_time).max(0.0);
        let level = (1.0 - age / span).clamp(0.0, 1.0) as f32;
        if level <= 0.0 {
            continue;
        }
        let anchor = prev.map(|(i, _, _)| nodes[i].lattice_pos);
        let Some(index) = nearest_match(nodes, node_pcs, tuning, step.pitch_class, anchor, center)
        else {
            // Played outside the displayed window. Keep `prev` so the route
            // bridges the gap rather than breaking into pieces.
            continue;
        };
        let color = channel_color(
            step.channel,
            step.pitch,
            frame.darkest_pitch,
            frame.brightest_pitch,
        )
        .lerp(idle, tint);
        if let Some((from, from_level, from_color)) = prev {
            // Repeated notes land on the same node; a segment from a point
            // to itself is a degenerate quad, not a line.
            if from != index {
                path.push(EdgeInstance {
                    a: nodes[from].world_pos,
                    b: nodes[index].world_pos,
                    // Fades with whichever end is older, as a grid segment
                    // fades with whichever note releases first.
                    strength: strength * from_level.min(level),
                    color: (from_color + color) * 0.5,
                    dashed: view.trail_path_dashed,
                });
            }
        }
        prev = Some((index, level, color));
    }
    path
}

/// Index of the visible node nearest `anchor` whose pitch class matches
/// `pc` under the current tuning, or `None` when the pitch isn't on screen.
/// With no anchor (the first step of a walk) the window's center stands in,
/// which also breaks ties everywhere else — so a route through ambiguous
/// spellings stays as near the middle of the view as the music allows.
fn nearest_match(
    nodes: &[NodeInstance],
    node_pcs: &[PitchClass],
    tuning: &Tuning,
    pc: PitchClass,
    anchor: Option<lattice_core::LatticePos>,
    center: lattice_core::LatticePos,
) -> Option<usize> {
    let steps = |a: lattice_core::LatticePos, b: lattice_core::LatticePos| {
        (a.threes - b.threes).abs() + (a.fives - b.fives).abs() + (a.sevens - b.sevens).abs()
    };
    let from = anchor.unwrap_or(center);
    nodes
        .iter()
        .zip(node_pcs)
        .enumerate()
        .filter(|(_, (_, &node_pc))| tuning.matches(pc, node_pc))
        .min_by_key(|(_, (node, _))| {
            (steps(node.lattice_pos, from), steps(node.lattice_pos, center))
        })
        .map(|(i, _)| i)
}
