# S1–S3: UI, scene and lifecycle ownership

Investigator: state_scene, gpt-6-sol high; read-only at ad1c6e6b.
Status: stated questions deeply examined by code and fixture inspection; native host latency unverified.

## S1 — edit, automation and restore ordering

`crates/harmonigraph-plugin/src/editor/frame.rs:49–75` locks shared state, adopts pending restore before audio drain, and syncs host camera before take controls.
`crates/harmonigraph-plugin/src/editor/persist.rs:222–247` serializes live state under the lock and falls back after 20 ms.
`crates/harmonigraph-plugin/src/lib.rs:1366–1419` tests through actual Params serialization.
Closed restoration adopts before draining (`background.rs:224–237,285–310`), after rechecking open state under the lock.
#1331/#1338 already fixed stale open-editor save and pending-restore ownership.
#1412 fixed capture-script interpretation, not a missing runtime camera path.

Verdict: retain the current owner/order; no new mechanism justified.
Remaining uncertainty: frequency and effect of the 20 ms save fallback during real Bitwig saves under dense dock+preview load.
Measure without perturbing the active user session at a later scheduled host-validation opportunity.

## S2 — derived scene state

Appearance normalizes on load (`appearance.rs:45–57`), scene derivation sanitizes once and carries Scene.view (`scene/derive.rs:56–58,161`), lattice asserts normalization (`panes/lattice.rs:105–111`).
#1421–#1423 removed repeated downstream sanitization; copying the view once gives the downstream scene a complete owned value.
LUT keys include sanitized Gradient (`scene/color.rs:543–650`); four slots cover the measured workload.
Fold depends on display revision and normalized width (`ui/spectrum.rs:551–565`), excluding appearance and clock inputs that do not affect it.
Manual range tests remain, but #1424 already centralizes range values and bar inventories.
A field-schema framework would add concepts without demonstrated savings.

Verdict: reject another scene cache or schema abstraction.
Known dense full-refold issue #886 remains separate and consciously deferred pending a representative saved workload.

## S3 — hidden and resized surfaces

`ui/state.rs:105–195` gives each surface motion, glow and aggregation/GPU mirrors while sharing source history.
`ui/state.rs:776–785` clears context resources; `shell.rs:84–90` invokes it on context replacement.
GPU pipeline retention is intentional #698.
`scene/motion.rs:461–473` resets after a hidden gap and replays a bounded horizon; fixture at 903–917 reaches it.
`glow_fade.rs:170–260` owns time-based coefficients and row reclamation.

Verdict: retain separate per-surface owners; sharing their histories would merge different clocks/geometry.
Native reopen/resize latency remains unmeasured.
