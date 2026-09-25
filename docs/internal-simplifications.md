# Internal simplifications

Status: implemented after independent design and implementation reviews.
Baseline: `f6287aa4`.
The goal is to retain every musical and visual feature while reducing duplicated behavior and internal contracts.
Window and dock management are excluded because another session owns that work.
The decisions and their limits are recorded below;
verification results are separate from the design claims.

## 1. One implementation of lattice melody and bass marks

### Before

`harmonigraph-scene/src/derive.rs` calculates melody/bass slots,
levels and colors from the tracker and its held-end stamps.
The production lattice then applies the audio ring,
`harmonigraph-ui/src/panes/node_motion.rs`,
and glow in that order.
NodeMotion replaces all six mark fields before the renderer consumes them.
The earlier marks only feed the initial glow value,
which the glow pass replaces when enabled and the renderer ignores when disabled.
Direct scene and renderer fixtures still exercise the earlier marks policy.

### Change

Move the pure NodeMotion state and progression into `harmonigraph-scene`,
keeping its existing per-surface storage and UI call order.
Remove the earlier melody/bass policy from scene derivation and initialize the six mark fields neutrally.
Keep the earlier activation,
departure,
octave and disc-color calculations:
the audio-ring floor,
resting-marker opacity and disc-color selection still use them.
Update mark fixtures to compose the same motion implementation production uses,
without retaining a second marks policy as a fixture fallback.
Keep renderer-only fixtures explicit about the settled scene they intend to draw.

### Contract and limits

Every current motion order,
starting pose,
mark delay,
fade,
late-event repair and live/offline path remains supported.
Production frames at finite times with valid configuration should remain byte-identical.
NodeMotion currently skips non-finite time;
neutral initialization would leave those frames without marks instead of exposing the earlier fallback.
Normalize and clamp mark delay in shared motion,
preserving the existing invalid-configuration tests.
This intentionally repairs invalid finite delays as well as non-finite ones.
Non-finite frame times explicitly produce no marks.
Keep derivation-only fixtures unchanged where they exercise earlier activation or octave values;
mark lifecycle fixtures use an explicit motion-aware helper that retains motion state across frames.
Do not remove tracker held-end metadata in this first change merely because its remaining consumers may become tests.
That is a separate follow-through decision after the production path is verified.

### Validation

Move existing NodeMotion behavior tests with the implementation.
Retain coverage for short notes,
same-time replacements,
late delivery,
reopening,
mark handoffs and different frame cadences.
Classify the older marks assertions individually:
retain feature assertions through the shared implementation,
and remove only assertions specific to the superseded policy.
Verify ordinary editor/preview/offline composition and both golden suites.
A changed fixture baseline must be explained and visually inspected;
do not bless changes merely to make the suite pass.

## 2. Explicit association between resting markers and nodes

### Before

The production `derive_pluses` constructor always associates each marker with a valid home-node index.
The renderer additionally recovers missing or stale indices by position,
resolves duplicate positions,
and inserts unassociated markers at the home-sheet boundary.
Repository consumers of that fallback are hand-built renderer fixtures.

### Change

Make `PlusInstance.node` a required index and remove its redundant lattice position.
Require a valid home node and at most one marker per node.
Retain the direct node-indexed marker table needed for painter order.
Remove positional lookup,
claim bookkeeping,
loose-marker collection and seam insertion.
Keep marker world position and styling independent.

Migrate isolated-marker fixtures to explicitly include a silent home node that establishes painter depth but draws no disc.
Use a small shared fixture helper where several tests need that setup.
Preserve their isolated-shadow and marker measurements;
do not substitute a visibly lit node or weaken the assertions.

### Contract and limits

Normal plugin and offline pictures should not change.
Hand-built Scene values must satisfy the documented association invariant instead of relying on recovery behavior.
This changes the internal Rust construction API,
not persisted state or user options.
The reviewed tradeoff is positive:
additional fixture setup remains local to tests,
while production loses the recovery paths and redundant marker identity.

### Validation

Retain depth-order,
culling,
marker-only shadow,
quantization and golden coverage.
Replace fallback-specific tests with direct association and invariant coverage only where needed.
Compare migrated isolated-marker fixtures against their existing pixels.

## 3. One owner of automatic-temperament judgement state

### Before

VisualRuntime duplicates ConfigReducer's remembered tuning judgements.
Each synchronous observation copies the runtime value into the reducer and then copies the result back.
Enabling Auto and restoring appearance reset the runtime copy.

### Change

Remove `VisualRuntime::temper_judged` and the judgement argument to `ConfigReducer::sync_display`.
Let the reducer retain its existing state.
Invalidate the display reducer at construction because its ordinary constructor already resolves default tuning.
Adopt synchronous policy edits together with raw parameters and modes at the next observation,
so a policy-only edit cannot consume a pending Auto recheck against old modes.
Provide narrow invalidation for one comma or all commas,
and use it at the existing Auto-enable and appearance-restore boundaries.
Do not replace the reducer on restore or change audio-owned configuration behavior.
The appearance-restore edit in `state.rs` is small but overlaps a file that window work may also touch;
coordinate integration rather than broadening the edit.

### Validation

Use the existing tests for manual release under Auto,
Auto re-enablement,
in-flight parameter writes and project restore.
Preserve their dependency-key behavior.
No persistence or musical-output change is intended.

## 4. Explicit render-progress messages

### Before

The plugin extracts total and completed frame counts from human-readable stderr.
The parser searches for ` frames`,
so warning and timing prose must avoid that token in certain positions.
The same byte-stream reader handles carriage-return terminal updates,
bounded segments and diagnostic retention.

### Change

Keep the percentage display,
cancellation,
encoded-frame accounting and warning/error status.
Use one shared codec beside `RenderProgress` in `harmonigraph-take` for an exact-prefixed record:
`progress: <done>/<total> frames (<percent>%)`.
Keep the existing bounded carriage-return/newline stderr reader and subprocess lifecycle.
Emit initial `0/planned`,
periodic `encoded/planned`,
and an unthrottled final `written/total` report (since #1061 the soundtrack no longer shortens the video).
The percentage is derived presentation,
not progress truth.
Ordinary warning and timing prose can contain frame counts without being mistaken for progress.
Malformed reserved-prefix records are ignored;
other output keeps the existing first-warning and final-diagnostic behavior.
There is no second stream,
reader thread,
command-line mode or negotiation.

### Contract and limits

Normal plugin behavior is unchanged.
Manual command-line progress gains an explicit prefix while retaining the percentage and rewritten-line presentation.
No progress is inferred from the human header or final filename message.
This removes a wording contract rather than a large amount of code.

### Validation

Exercise progress together with warnings,
render failure,
empty progress output and cancellation.
Ensure diagnostic output containing ordinary frame counts cannot alter progress.
Retain the existing encoded-frame progress semantics and successful-render warning visibility.
Check pipe draining and reader termination using bounded subprocess fixtures.

## Delivery

Implement the reviewed forms of all four changes.
Commit the reviewed changes with appropriate local checks,
then push and open a draft PR.
Build both plugin and offline renderer in this managed worktree while CI runs.
Do not merge or replace the shared DAW installation.
The handoff must name the load command,
read the build tag from the built binary,
and distinguish verified behavior from unperformed DAW acceptance.

The separately observed renderer-path override and redundant MapView geometry field are outside these four changes.

## Independent design review

Two fresh read-only reviewers checked the proposals against their consumers and tests before implementation.
Both recommended proceeding after the following revisions,
which are incorporated above:

- Keep invalid-delay normalization and use explicit motion fixtures rather than adding animation to every scene test.
- Silence every drawing contribution on marker-fixture anchors,
  preserve their former painter placement,
  and emit associated markers even when their owners are culled.
- Preserve first-observation Auto detection and pending rechecks across policy-only edits.
- Use an exact prefix on the existing stderr stream instead of introducing stdout reader ownership.
- Preserve final encoded-frame accounting when the output is shorter than planned.

The reviewers found no normal-runtime feature loss in the revised designs.
They did not establish pixel equivalence;
implementation verification owns that claim.
The unchanged baseline golden suites passed before code edits.

## Verification

The affected core, scene, UI, take, recording, renderer and offline suites passed locally.
Both golden suites pass against the unchanged committed images after the refactor.
The strict production Metal catalog also passes;
no shader asset regeneration is needed for these changes.
Independent implementation reviews found no remaining correctness blockers.

The mark fixtures now exercise carried motion rather than the removed prepass.
Two expectations intentionally changed to match existing production behavior:
short notes reverse their carried level at key-up,
and equal-strength slots favor the higher slot.
The transient chord-release fixture samples each handoff while it exists rather than expecting obsolete long tails.
Silent marker anchors retain the old painter placement in their fixtures;
the initial one-byte golden difference during fixture migration was corrected without blessing a baseline.

No performance benchmark or manual DAW acceptance is claimed.

## Follow-through

See [the follow-through decisions and deferred candidates](simplification-follow-through.md) for the next simplification batch.
