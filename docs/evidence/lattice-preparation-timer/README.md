# Lattice preparation timer

Measured on the M1 Pro during the 2026-09-26 sweep for #1150 and #1182,
after the single-pass transmission change in #1213.
The scene has twelve held MIDI notes, 225 lit nodes, and 81 synthetic names,
using the live scrolled window.
Each run discards ten warmup frames and measures sixty frames.
Timer-off/on/on/off order keeps comparisons adjacent.
No overlay is drawn.

The production interval begins before preparation and ends on the scene pass without bloom,
or on the last dependent bloom blur with bloom.
The separate full interval ends on the composite pass that samples those results.
An independent tail pass cannot close preparation reliably on a tile-based GPU.

| Pane | Bloom | Full GPU, timer off (ms) | Full GPU, timer on (ms) | Production preparation (ms) |
| --- | --- | --- | --- | --- |
| 1536² | Off | 11.124 / 11.148 | 11.083 / 11.106 | 10.918 / 10.945 |
| 1536² | On | 11.515 / 11.481 | 11.464 / 11.467 | 11.289 / 11.304 |
| 2160² | Off | 20.697 / 20.751 | 20.647 / 20.672 | 20.396 / 20.402 |
| 2160² | On | 21.290 / 21.272 | 21.208 / 21.212 | 20.958 / 20.951 |

These are medians, not a portable performance threshold.
The preparation figure scales with pixels and remains slightly below the full interval,
which includes the host composite.
Arming the lattice timer shows no measurable added GPU cost in these runs;
the slightly lower armed figures are within run variation and are not a speedup claim.
The source fallback occurred during cold pipeline creation and is excluded by warmup.

Reproduce one cell with the existing ignored probe:

```sh
PROBE_TIMER=1 PROBE_SIZE=2160 PROBE_BLOOM=1 PROBE_NOTES=12 PROBE_CASE=all PROBE_FRAMES=60 \
  cargo test -p harmonigraph-render --lib lattice_tests::timing::atmosphere_costs_by_polyphony \
  -- --ignored --exact --nocapture
```

Set `PROBE_TIMER=0` for the adjacent control.
This exercises the production callback and asynchronous timer,
but it is not an in-DAW measurement and does not measure the separate host draw timer's overhead.
The plugin was not loaded into the shared DAW slot.
