# Simplification follow-through

Follow-through to [the first internal simplifications](internal-simplifications.md),
merged in #1057.
Baseline: `8f26f69c`.
The creative modes remain available;
this work removes obsolete state and simplifies ownership.
The oldest spectrogram slice intentionally changes its averaging rule.

## Design review

Two independent reviewers inspected the plan before implementation.
They supported removing the unused tracker metadata and moving upload history to the renderer.
The spectrogram reviewer required keeping the oldest admitted slab in the cache identity:
viewport clipping alone does not preserve the shader's sampling boundary,
so reusing a longer run would expose an additional preceding slab to filtering.
The core reviewer recommended leaving the optional held-map release rewrite out of this change.

## Implemented decisions

### Remove obsolete held-end metadata

`NodeMotion` owns the melody and bass timeline after #1057.
Remove the tracker's `HeldEnd`,
released-voice end stamps,
accessors and event-time restamping.
Keep note lifetimes,
source visibility,
release envelopes,
roll and history unchanged.
The removed metadata tests have no production consumer;
mixed source,
baseline and gap tests retain their independent behavior checks.
No musical or visual change is intended.

This removes two scans of held voices from each restamping site,
but ordinary chords are small and no frame-rate gain is claimed.

### Let the renderer own spectrogram upload history

The UI supplies an immutable folded snapshot with its absolute first key,
bin count and ring capacity.
The renderer compares it against the snapshot its buffer actually received.
An identical allocation at the same key and shape needs no comparison;
other snapshots compare slabs by absolute key and bytes.
New keys always need writes,
and a change to more than half of the run uses the existing bulk-upload policy.

Remove caller-supplied dirty keys,
generation and serial numbers,
upload acknowledgements and retry bookkeeping.
Discarded callbacks cannot advance the renderer's remembered state.
A new renderer context naturally has no previous buffer,
so the UI no longer invalidates its folded data on context recreation.
Font and label context resets remain independent.
The full-upload counter reports actual renderer writes and never decides correctness.

The renderer retains one previous immutable run while the UI produces the next.
That bounded memory overlap buys a single owner of upload truth;
this is primarily a maintenance improvement,
not a measured speedup.

### Keep complete averages at the oldest spectrogram edge

A completed time slab keeps the average of all columns folded into it,
even when some columns have just left the visible window.
Remove the special first-slab reaggregation,
propagation into gap-fill copies,
and parallel array marking those copies.
The newest active slab still accumulates normally.
One-slab jitter holds and substantial silent gaps keep their existing distinction.

The folded snapshot is reusable while source data,
slab width and the oldest admitted slab agree.
Advancing the raw oldest column within that slab does not invalidate it.
Advancing into another slab does:
the shader's filtering boundary is the run's first slab,
not merely the viewport edge.
Capacity growth can reveal older retained data;
capacity shrink can advance the admitted boundary.

The intended picture change is confined to the complete average of the oldest visible slab and its gap-fill copy,
with existing visual filters applied to those values.
This can change brightness near the far edge of the history.
Scrolling coordinates,
note alignment and creative controls are unchanged.

## Validation

The unchanged baseline passed both golden suites before implementation.
Independent implementation reviews found no actionable correctness defects.
The core and scene suites passed (117 and 203 tests),
as did the complete renderer and UI suites (310 and 721 tests).
The focused upload tests compare incremental and fresh-resource pixels in plain and softened rendering,
including discarded callbacks,
empty prepares,
context recreation,
ring wrap,
shape changes and the bulk-upload threshold.
Oldest-slab tests cover unequal source columns,
jitter holds,
retention trimming,
within-slab snapshot reuse and crossing the sampling boundary.
The strict production Metal catalog passed without regenerating assets.

The five offline golden frames changed by one color level at 27,
42,
36,
29 and 4 pixels respectively (short,
tall,
zoomed,
watercolour and mixed-shadow fixtures).
The comparison sheets were inspected before updating those baselines.
A temporary control restoring only the old first-slab average made all five original goldens pass byte-exactly,
confirming the intended edge change as the cause;
the control was then removed.
Existing softening can spread a small edge change farther across the frame.
Renderer goldens were unchanged.

## Deferred candidates

These are proposals for a future decision or focused change,
not claims of measured defects or completed design reviews.

### Make planned video duration authoritative

Done in #1061:
the planned frame count is authoritative,
and ffmpeg pads the soundtrack with silence (`apad`) and trims it to the video span instead of cutting the video with `-shortest`.
The proposal as it was written follows.

Today a soundtrack can end the video early through ffmpeg's `-shortest` behavior.
Using silence after audio ends would preserve the planned frame count,
including visual release tails and an explicit render end.
This could remove soundtrack-cut prediction and clean early-stop handling across the offline renderer,
but it changes output duration and needs a product decision.
Verify empty or exhausted audio,
seeks beyond audio end,
AAC priming and ordinary encoder failures before simplifying the finish path.
This is a maintainability candidate,
not a speed claim.
Relevant code: `harmonigraph-offline/src/sink.rs`,
`render.rs` and `main.rs`.

### Remove the hidden saved renderer-path override

Done in #1061:
the recorder always launches `default_renderer_path()`,
a saved `renderer_path` is ignored on load and not written again,
and fixtures inject their program through a test-only override.
The proposal as it was written follows.

`RenderConfig.renderer_path` can retain an absolute executable path in saved appearance and take data,
although the Video pane does not expose it.
The documented loader installs the paired renderer at a fixed path.
Removing the saved override would simplify that contract,
but tests using fake or missing executables would need explicit injection.
The savings are modest.
Read the persistence contract before changing the saved shape,
and state the loss of custom-path behavior.
Relevant code: `harmonigraph-take/src/render.rs` and `harmonigraph-record/src/recorder/render_job.rs`.

### Remove redundant map-view geometry

`MapView.working` copies a complete map,
while its UI consumers only ask whether it exists.
`playback.audition` already expresses that condition.
Replace the duplicate presence signal after checking edit and pending-adoption behavior.
The adjacent map-view construction also resolves offsets twice.
This is a small cleanup,
not a reason for a broad map refactor.
Relevant code: `harmonigraph-ui/src/lattice_maps.rs` and `harmonigraph-plugin/src/lattice_maps.rs`.

### Revisit the remaining scene-envelope prepass

Done in #1063 for markers and the audio ring's MIDI floor:
carried node motion now owns both,
the preliminary activation, departure and octave derivation is gone,
and so is `Scene::wear_audio_rings`.
The proposal as it was written follows.

Scene derivation computes activation,
departure and octave levels that carried motion later replaces.
Unlike the removed mark prepass,
these values still feed marker opacity,
the MIDI floor under audio rings and disc-color winner selection.
`NodeMotion` also reads the pre-motion activation and ring presence when deciding an audio-only node's starting pose.

A single carried envelope could make markers and audio floors follow the node more consistently,
with altered short-note timing,
but moving passes alone is insufficient.
Disc color still needs a policy for simultaneous voices,
and audio measurement must be separated from the MIDI floor before reordering.
Standalone scene callers and fixtures also need a clear construction contract.
Define those semantics and demonstrate real code reduction before undertaking this change.
Relevant code: `harmonigraph-scene/src/derive.rs`,
`motion.rs` and `Scene::wear_audio_rings`.

### Encapsulate map-document mutations

`MapDocument.slots` and `order` are public,
although changes must bump the revision used by the name memo and audio map bank.
Production external readers need only narrow length and name accessors.
Making the collections private would enforce revision ownership;
keep serde behavior and intentional mutation tests in view.
This is a small correctness-oriented API improvement.
Relevant code: `harmonigraph-ui/src/lattice_maps.rs` and its plugin callers.

### Remove the unused voice-octave cache and reconsider batch release

`Voice.octave` is maintained on bends but has no current production reader.
Removing it would eliminate a redundant invariant;
keep `pitch_class`,
which has real consumers.
Separately,
without end-stamp borrowing,
batch release could retain held-map survivors in place instead of taking and rebuilding the map.
Preserve deterministic release order and visibility-at-release,
and proceed only if the shared direct-off/batch helper remains simpler overall.
Neither cleanup promises a material performance gain for typical chord sizes.
Relevant code: `harmonigraph-core/src/notes.rs`.
