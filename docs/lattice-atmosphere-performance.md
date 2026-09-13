# Lattice atmosphere performance audit

Audit of `53d55b97` (the atmosphere prototype, PR #871), on 2026-09-13.
This change adds a repeatable measurement and records optimization candidates;
it does not change the picture or the production renderer.
The first recommended prototype is an independently scaled soft-glow target, keeping sharp geometry at native resolution, followed by hierarchical candidate filtering for large halos.

## Measurement

The ignored `atmosphere_costs_by_polyphony` probe in `crates/harmonigraph-render/src/lattice_tests/timing.rs` sends 1, 6, 12, or 24 held chromatic MIDI notes through `NoteTracker` and `derive_scene` with current defaults.
The atmosphere clock advances at 60 Hz.
Twelve and twenty-four notes share pitch classes but fill different octave wedges;
the output reports lit lattice nodes separately from MIDI notes.

It compares all atmosphere, atmosphere off, each component disabled independently, half scene render scale, and all node glow disabled.
GPU timestamps bracket the rendering commands encoded by `prepare`, including shadows, ink history, glow, scene, and bloom.
They exclude the final egui composite, scene derivation, audio processing, and DAW contention.
CPU callback construction and CPU preparation are reported separately.
A wall-clock cross-check measures submission through final paint and device completion;
it includes host encoding, allocation of the final output texture each frame, readback scheduling, and waiting rather than isolating GPU execution.
Labels are controlled synthetic three-stroke names, not actual text shaping.
Held notes do not reproduce rapid turnover or a long sustain/release history.
The atmosphere-off comparison retains the current shadow and glow settings;
it is not a complete before/after comparison with PR #871's parent.
That PR also captured new appearance defaults, including Gaussian lattice shadows in place of Distance shadows and increased close-glow reach and strength.

Run serially, in release mode:

```sh
PROBE_FRAMES=120 cargo test --release -p harmonigraph-render atmosphere_costs_by_polyphony -- --ignored --nocapture --test-threads=1
PROBE_SIZE=1536 PROBE_NOTES=12 PROBE_FRAMES=120 cargo test --release -p harmonigraph-render atmosphere_costs_by_polyphony -- --ignored --nocapture --test-threads=1
```

`PROBE_CASE` can select one case from the matrix.
Ten warmup frames precede the measured frames;
the output includes median, p10, p90, adapter identity, target size, projected candidates, and candidates in the global list.
Cold pipeline creation is printed separately and excluded from steady-state timing.

## Observations on Apple M1 Pro / Metal

The 768 by 768 run used 120 measured frames per case.
The repeatable workload counts are:

| Held MIDI notes | Lit lattice positions | Candidates, atmosphere on / off | Global candidates, on / off |
| --- | --- | --- | --- |
| 1 | 96 | 29 / 15 | 17 / 10 |
| 6 | 519 | 169 / 109 | 105 / 57 |
| 12 | 1,025 | 337 / 221 | 213 / 109 |
| 24 | 1,025 | 337 / 221 | 213 / 109 |

At twelve notes, the global list alone requires about 125.6 million candidate visits per frame with atmosphere, versus 64.3 million with atmosphere disabled.
This is list traversal work, not 125.6 million nonzero contributions:
`glow_layer` rejects pixels outside each halo.
It nevertheless confirms that a faint wide component nearly doubles this unconditional part of the gather.

The GPU timestamp probe was highly variable on this machine during the audit.
For twelve notes, all-atmosphere measured 3.643 ms median with p10/p90 of 0.344/21.103 ms;
all-atmosphere-off measured 3.629 ms with 0.321/14.502 ms;
node-glow-off measured 0.415 ms with 0.226/1.366 ms.
Some component rankings reverse between runs, so these readings do not justify precise percentage savings or a ranking of cloud versus breathing costs.
Other compilation and GPU-test processes were observed on the host;
this audit does not determine how much of the variation comes from contention versus timestamp behavior.
The reproducible candidate counts and source analysis are the stronger evidence.

CPU callback construction remained around a tenth of a millisecond for twelve notes in this renderer-only fixture.
That does not measure the preceding node-by-voice scene derivation.
Raw output is retained in [the 768-pixel matrix](evidence/lattice-atmosphere/768-matrix.log).

At 1536 by 1536, the same twelve notes produce 285 global candidates with atmosphere and 193 without it.
The first completion-time run measured 58.188 ms for all atmosphere, 51.006 ms with the wide component off, 45.588 ms at half scene scale, and 14.522 ms with node glow off.
However, disabling nebula or breathing measured slower in that run, and the half-scale p90 reached 115.490 ms:
completion timing is also affected by host load and does not support precise component-level savings.
See [the full 1536-pixel run](evidence/lattice-atmosphere/1536-matrix.log).

A [repeat in a different order](evidence/lattice-atmosphere/1536-repeat.log) gave:

| Case, twelve notes at 1536 by 1536 | Submission through completion median | p10 / p90 |
| --- | --- | --- |
| All atmosphere | 57.764 ms | 51.158 / 72.634 ms |
| Half scene render scale | 16.676 ms | 14.813 / 33.554 ms |
| Node glow disabled | 9.416 ms | 7.962 / 27.171 ms |

This supports prioritizing the glow path and its resolution, but is not a DAW frame-rate measurement or a promised 3.5-times speedup for a glow-only target.
The half-scale case also reduces geometry and shadow work.
Both runs put the full-resolution all-atmosphere median near 58 ms, while lighter cases remain more variable.
The 768-pixel log predates addition of completion timing to the probe;
its candidate and timestamp measurements use the same fixture and shader paths.



## Why many notes are expensive

`lattice_node_glow::halo_pixels` multiplies the close halo's bounding radius by wide spread whenever wide strength is greater than zero.
At the prototype defaults, spread is 1.610383 and strength is only 0.052936:
the circular support area becomes about 2.59 times as large even though the added tail is faint.
Reducing a nonzero amount does not reduce that geometric support.

`lattice_node_glow::tiles::pack` puts a candidate in the global list when its bounding rectangle touches more than 64 tiles of 32 by 32 pixels.
`fs_glow_gather` checks every global candidate at every glow pixel, including pixels outside the halo.
The shader rejects those pixels after projection and radial work, but the list traversal is already paid.
The cutoff is expressed in physical pixels, so raising render resolution can both increase pixel count and move candidates from local lists to the global list.

The glow target has the same resolution as the scene's sharp geometry.
The cloud pattern runs once after the node contributions have been combined, not once per note.
Breathing is two sines per shipped glowing instance during CPU callback construction and a multiplier during projection;
it does not alter ink history.

On the CPU, `derive_scene` already matches lattice nodes against voices with an `O(nodes × voices)` loop.
That predates the atmosphere work and is outside this renderer probe's timings.
If the live overlay identifies scene derivation as the remaining bottleneck, grouping or indexing voices while preserving tuning tolerance and octave identity is the next independent audit target.
The atmosphere introduces no new cache key;
existing GPU buffer allocations remain keyed by capacity.

## Optimization candidates

These are implementation proposals, not measured speedups of completed alternatives.

| Priority | Change | Expected work reduction | Visual tradeoff |
| --- | --- | --- | --- |
| 1 | Give soft glow its own half-resolution target | One quarter as many glow fragments, plus potentially fewer global candidates | Slightly softer fine color detail; keep node ink and labels at full resolution |
| 1 | Add a coarser tile tier for large halos | Avoid checking large-but-local halos across the entire pane | Preserve analytic output and node order; validate pixel parity |
| 2 | Special-case the wide-only annulus | Skip angle evaluation and two directional strip reads where only mean color contributes | Same intended output; verify rounding with image comparisons |
| 2 | Precompute the circular ink-blur kernel | Remove repeated cosine/exponential evaluation from the 65 by 64 convolution per glowing row | Same intended kernel; verify numeric precision |
| 3 | Render cloud density at low resolution | Replace four interpolated noise evaluations, totaling 16 integer hashes, with a texture sample per glow pixel | Small loss of cloud detail; slow update rates need interpolation to avoid stepping |
| Optional redesign | Make broad light from low-resolution average-color splats and a shared blur | Avoid evaluating the broad analytic component against each candidate at each pixel | Changes radial falloff and overlap brightness; needs visual comparison |
| Optional quality setting | Reduce angular ink strips from 64 to 32 samples | Convolution falls from 4,160 to 1,056 taps per glowing row | Less angular color detail |

For a separate glow resolution, update the projected candidate coordinates, tile grid, nebula target dimensions, background composite, and the common shader's ink-wash texture reader together.
The current wash reader assumes one glow texel per scene pixel.
The probe's `half-scale` case scales the entire scene, so its timing is not a measurement of this proposed glow-only change.

For hierarchical tiles, preserve the current source-node accumulation order while merging candidates.
A radius or tile-tier change must remain conservative at viewport edges and under perspective.
A dim-light cutoff would be an approximation:
many individually faint tails can contribute visible light together, so per-note brightness alone is not a safe exact cull.

For an ink-blur kernel cache, the key is the sanitized blend setting and strip resolution.
It must not include note activity, camera, breathing time, or the entire view configuration.
The kernel is shared, but the ink itself and its carried attack/release state still change per node and frame.

## Source map

- Candidate projection and radius: `crates/harmonigraph-render/src/lattice_node_glow.rs`, `glow_nodes` and `halo_pixels`.
- Candidate indexing: `crates/harmonigraph-render/src/lattice_node_glow/tiles.rs`, `pack`.
- Target sizing and pass scheduling: `crates/harmonigraph-render/src/lattice_prepare.rs`, `prepare_targets` and `encode_node_glow`.
- Per-node convolution and per-pixel light: `crates/harmonigraph-render/src/shaders/lattice.wgsl`, `fs_ink_blur`, `glow_ink`, `glow_layer`, `nebula_light`, and `fs_glow_gather`.
- Breathing and frame uniforms: `crates/harmonigraph-render/src/lattice_frame.rs` and `crates/harmonigraph-scene/src/atmosphere.rs`.
