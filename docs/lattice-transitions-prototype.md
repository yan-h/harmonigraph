# Lattice animation controls

Display tab → Lattice page → Note layers has independent Animation and Order controls.
Animation is Fade or Pop;
Order is Simultaneous,
Circular,
Bidirectional,
Random stagger,
or Odd/even stagger.
The existing Note fade duration drives arrival and departure.
Fade curve continues shaping MIDI opacity.
Zero duration switches immediately.

Starting position moves each complete slice radially relative to its normal anchor:
-100% places its anchor at the node centre,
0% leaves it at its usual position,
and +100% starts at twice its normal radius.
Starting size ranges from zero to 200% of the settled size.
Both controls work with either animation.
Grow from centre sets Starting position to -100% and Starting size to zero;
it does not change Animation or Order.
Pop adds a small overshoot to the existing geometry.
No extra particles or render passes are added.

Circular starts with the complete slice containing twelve o'clock and proceeds clockwise.
Bidirectional sends two fronts from twelve toward six.
Random stagger uses a stable seed from the node and appearance onset;
interruptions preserve its ordering.
Odd/even stagger uses two alternating groups.
Stagger spread sets the time between the first and last slice starts as a percentage of Note fade,
from 0% to 90%,
with 28% as the default.
Zero starts every piece together;
Simultaneous ignores the control.
Each piece moves during the remaining duration,
so every piece finishes within Note fade.
All orders use the full selected spread,
including Random stagger and Bidirectional.
A normal departure from a settled node uses the same order.
Changing spread while held applies to the next normal departure;
interruptions retain their current movement speed and cancel pending waits.

A factual note-off immediately reverses an unfinished pose and its MIDI opacity.
Pending pieces cancel;
a quick repress reverses from the current state without restarting.
Another octave or a same-time key replacement does not replay the existing node's entrance.
The whole node departs after its last held contribution ends.
MIDI octave levels and delayed melody/bass marks carry their own reversible opacity.
An existing audio-only node acquires MIDI without a second entrance;
the audio ring keeps its independent level and fixed coordinates.

This is a lattice-only presentation clock.
Core envelopes,
tuning,
audio processing,
and other panes keep their existing sequencing.
The shared live/offline pane replays factual roll edges since a per-surface checkpoint,
including short presses between rendered frames and same-time tuning.
Current voices reconcile pitch and source state when history is incomplete;
an observation gap is not invented as a timestamped note-off.
The checkpoint does not replay old history after pruning.
The roll retains its existing bounded history,
so events never observed or already lost from that history cannot be reconstructed.

Persistence replaces the old `note_transition` key with `note_animation`.
Old choices reset to Fade + Simultaneous with zero offset and 100% size;
the rest of the saved appearance remains intact.
There are no compatibility aliases or retained old effect variants.

Each GPU node adds sixteen bytes containing eleven 10-bit poses and a settled flag.
Order computation happens on the CPU,
not per pixel.
Settled nodes take the reference drawing path.
Moving pieces use the existing scene and shadow passes;
allocation bounds depend on configuration rather than animation progress.
The Grow preset retains the usual bound for Fade and about 4.6% headroom for Pop.
Larger starting sizes and outward offsets can expand the area shaded and shadowed.

`HARMONIGRAPH_TRANSITION_FRAMES=/tmp/animations cargo test -p harmonigraph-render transitions` emits comparison frames with rotated unequal slices,
matching marks,
all five orders,
and Gaussian/Distance shadows.
`cargo test -p harmonigraph-render animation_costs_by_pose_and_density -- --ignored --nocapture` compares realistic 24-note and dense 225-node workloads.
One default Distance golden changed by a single channel level in three pixels after shader restructuring;
its expected/actual/difference sheet was inspected before updating the baseline.

Measured on an Apple M1 Pro at 768×768 over 120 warmed frames,
the 24-note fixture lights 1,025 lattice nodes and draws 81 names.
GPU timestamps cover the preparation encoder,
including shadows,
ink,
light,
scene,
and bloom;
they exclude the final egui composite.
These are medians from an interactive host with wide timing spreads,
not an isolated throughput guarantee.

| Starting pose / order | Gaussian | Distance |
| --- | --- | --- |
| Plain Fade | 1.21 ms | 2.44 ms |
| Pop at normal pose | 1.65 ms | 3.52 ms |
| Fade Grow from centre | 1.87 ms | 4.01 ms |
| Pop Grow + Circular | 2.04 ms | 3.75 ms |
| Pop at +100% offset / 200% size | 3.60 ms | 11.30 ms |

A separate 225-node fixture with every octave sector lit and no names measured 1.12–3.30 ms for Gaussian and 0.63–0.99 ms for Distance across these settings.
That workload suppresses whole-node activation to omit labels,
while nonzero octave levels still draw and cast shadows.
The per-surface CPU state alone measured 0.137 ms per frame for 225 nodes and 24 held notes,
and 0.160 ms with 4,096 additional retained completed notes.
The CPU measurement followed compilation and averaged 2,000 updates;
its scratch test was removed after recording the result.
Order lookup and state updates are small relative to drawing;
large starting poses with Distance shadows are the expensive case.

## Distance shadows during a fade

Distance previously admitted a complete layer into its shadow contour at 50% opacity.
That made a fading slice's shadow appear or disappear in one frame:
the measured plain Fade jump was 135/255 in a pixel and 9/255 averaged across the frame.
Staggered pieces crossed the threshold separately,
causing repeated jumps.

Node Distance cells now store the maximum of each layer's Distance profile weighted by its opacity.
The same representation continues through fully opaque frames,
so the endpoint cannot switch between coverage interpolation and distance interpolation.
Audio and marks keep independent opacity;
Gaussian shadows and text/marker Distance fields retain their existing paths.
No extra texture,
render pass,
or shader binding is added.

The temporal regression samples 1% steps through Fade,
Pop,
Grow,
28% and 90% Circular spread,
independent marks,
and audio-only fades with nondefault falloff.
Reversing these same frames checks departure continuity.
The final 0.99999-to-1 step changes by at most 1/255 in every case.
`HARMONIGRAPH_SHADOW_FRAMES=/tmp/shadow-frames cargo test -p harmonigraph-render distance_shadows_fade_continuously` writes the sequences and shadow-only measurements.

Three settled Distance baselines were inspected and updated:
live view averaged 0.083/255 difference with maximum 9/255,
the top view 0.038/255 with maximum 3/255,
and the zoomed-out view 0.293/255 with maximum 9/255.
Their contours and composition are unchanged.
The stable representation measured 2.19/3.51/3.14/3.27/8.64 ms for the realistic Distance Fade/Pop/Grow/Circular/Maximum cases above;
the dense Distance cases ranged from 0.43 to 1.01 ms.
Host timing varied across runs,
including the unchanged Gaussian control,
so these measurements establish no gross regression rather than a speedup.
