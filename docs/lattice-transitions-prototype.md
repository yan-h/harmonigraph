# Lattice transition prototypes

Display tab → Lattice page → Note layers → Transition selects the prototype.
Fade is the default and reference.
The adjacent Note fade bar sets the arrival and departure duration;
0 ms switches immediately.
For slower comparison,
try 300–600 ms.

| Choice | Arrival | Departure |
| --- | --- | --- |
| Fade | Existing opacity envelope | Existing opacity envelope |
| Pop and settle | Scale up with a small overshoot | Contract while fading |
| Staggered pop | Whole slices pop around their own centres in a fixed shuffled order | Whole slices contract in that order |
| Clockwise pop | Whole slices pop clockwise starting at twelve o'clock | Whole slices contract clockwise |
| Draw and retract | Draw the radial bands clockwise from twelve o'clock | Retract toward the same anchor |
| Ripple arrival | One expanding ring | Existing fade |
| Focus and dissolve | Resolve MIDI ink out of the existing halo | Dissolve MIDI ink back into the halo |
| Spark and trail | One pulse around the rim with an 18%-of-circumference tail | One departure pulse with the same bounded tail |

The lattice has no connecting edges,
so Draw traces its existing radial bands and Spark follows its rim.
The effects apply to MIDI geometry;
the independently gated audio ring continues reading audio at its original size and position.
Pitch centres and note-name labels stay fixed.
Focus uses the existing Glow on the Lighting page:
Reach and Strength must be above zero to see its halo.
No atmosphere or cloud setting is changed.

Motion uses linear progress on the same note timestamps and duration as the existing envelope,
while Fade curve continues to shape opacity.
A short note completes its arrival before departing.
An entrance belongs to the node's continuous visible presence.
Lighting another slice or octave,
retriggering a key,
or pruning the original voice does not replay it.
An existing audio ring also counts as an existing node.
Only disappearance rearms the entrance.
A pitch class stays whole while another octave remains held;
once all octaves depart,
the longest remaining release owns the contraction so pruning a newer voice cannot make the node grow back.
The scene builder supplies envelope progress;
transient per-surface history retains the entrance in the shared editor and offline drawing path.
Changing Note fade affects an unfinished entrance without winding it backward or replaying a settled one.

Staggered pop and Clockwise pop scale each complete slice and its matching mark extensions around the slice's centre.
Their starts occupy the first 28% of Note fade,
and every slice finishes within that duration.
Clockwise pop orders whole pieces spatially rather than wiping their angles.

The selector is persisted in the appearance as `note_transition`.
Missing keys default to Fade;
no existing persisted field or variant is removed.

The existing GPU scene and shadow passes draw the gestures.
Fixed per-mode headroom reserves their maximum extent;
animated sizes do not enter label glyph or shadow allocation keys.
The normal audio and glow history remain in place.

For a rendered comparison sheet,
run `HARMONIGRAPH_TRANSITION_FRAMES=/tmp/transitions cargo test -p harmonigraph-render transition_prototypes_draw_distinct_arrivals_and_settle`.
The optional PPM frames cover three arrival points,
held,
and three departure points for every mode.
The test requires distinct arrivals and identical held pictures with the fixed comparison fixture.
Existing Fade golden images remain unchanged.
The `slice_pops_move_complete_pieces_in_distinct_orders_and_settle` test emits additional `slices-` frames with six rotated slices,
matching mark extensions,
Gaussian and Distance shadows,
and near-settled continuity checks.
