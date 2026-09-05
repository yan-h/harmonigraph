# Lattice history ownership (#645 B)

This bounded renderer change follows #662 → #646 / PR #664 → #645 B.
The immediate parent and review base is `codex/lattice-uniforms`, verified at `70743e4d37872ecb6be9113cca44588e30c7e6c3`.
The original #662 head was `4c40d9ae2f1817fe5f5c1682182830559e029a57`;
Yan separately merged it as `357e1325a1c6099587c5440f6ba17b76cfdabba7`.
The parent's normal ancestry merge reconciles those histories without changing renderer source.
This PR builds on that frozen parent, not a reimplementation of its earlier slices from main.

## Ownership and preserved behavior

`PaneBuffers::ink_history` owns the per-pane GPU `InkStrip` beside the viewport-sized `Offscreen`.
`GlowTarget` owns only the scene-resolution light field and its sampling binding.
No strip transfer or adoption remains, and creating a new viewport target no longer constructs a strip that will be discarded before use.

The history key is pane identity and `Scene::glow_rows` capacity while glow is enabled and target maintenance runs.
Native viewport size, render scale and bloom strength do not affect history allocation.
Capacity changes replace the strip and reset its parity;
the unchanged CPU `GlowFade::step` supplies `mix = 1` to reseed from current ink.
That baseline loses the color of a release that has no current ink on growth.
Same-capacity resize already preserved that release color in the parent;
this refactor removes its transfer mechanism while preserving that behavior.

Disabling glow discards history when drawable geometry lets target maintenance run.
Empty geometry continues to skip both glow/history maintenance and retains the existing parity advance before the scene-draw guard.
This change does not introduce hidden-view retirement or alter empty-frame behavior.
History remains on the GPU;
the shader computes ink exactly as before.

`Scene`, instance payloads, CPU row allocation, UI `SharedState`, source/event/configuration inputs and all musical semantics are unchanged.
The inherited source-aware fixtures keep their existing APIs.
#645 C remains explicitly deferred until B and coordinated integrated #617;
#645 remains open.
No caches, shader algorithm changes or general resource framework are added.

## Focused acceptance fixtures

- `viewport_changes_keep_history_without_allocating_a_strip` keeps a red inkless release through four native-size/render-scale transitions.
It compares each output byte-exact against a separate pane seeded at the final viewport, checks raw textures/bindings and blurred texture identity, checks parity and independent green preview history, and counts actual `InkStrip::new` calls including any throwaway allocations.
- `capacity_growth_reseeds_current_ink_and_row_reuse_keeps_identity` directly supplies renderer inputs for 64 → 128 rows,
including an old inkless release and a newly writable held row 64. It asserts two GPU instances, actual texture height, a replaced strip with reset parity, unchanged viewport targets, loss of red release color and visible green current ink.
Growth matches a fresh pane byte-exact;
row zero then seeds blue while row 64 carries green through reordered instances.
This exercises the renderer capacity-change branch;
it does not execute or change the CPU allocator.
- `glow_off_discards_history_when_target_maintenance_runs` verifies the unchanged empty-geometry guard and parity behavior,
then requires glow-off to discard history once geometry returns and prevents stale red from reappearing on glow-on with no ink.

The existing bloom on/off/on resource fixture, release-color suite, production/reference and hot-reload comparisons, 13 lattice goldens and offline golden set remain acceptance gates without blessing.
Only history accessors change in the existing glow-color and direct-reference fixtures.

## Validation status

Initial source is formatted;
local compilation and GPU checks await the coordinator's explicit machine lease.
This is a provisional implementation, not yet a validated or loadable handoff.
Exactly one authorized Claude Opus/xhigh review will use `--base codex/lattice-uniforms` after initial validation.
Local actual-adapter validation and fresh post-final-commit plugin plus offline release artifacts remain required.
No DAW slot is swapped.
