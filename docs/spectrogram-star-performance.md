# Stars shader performance

This is the current starting point for Stars optimization.
The product cost is visual difference plus ongoing maintenance burden.
[Issue #1142](https://github.com/yan-h/harmonigraph/issues/1142) tracks the research;
[#1242](https://github.com/yan-h/harmonigraph/issues/1242) covers broader changes of look.
The selected implementation is [#1248](https://github.com/yan-h/harmonigraph/pull/1248).
Historical percentages below have different baselines and must not be added together.

## Current architecture and selected tradeoff

The atlas bakes each star's position,
life,
size,
and color once per frame.
All five depth layers remain.
High (formerly Optimized) renders the complete farthest three responses together into an RGBA16Float image at 75% width and height,
then samples it bilinearly.
Those layers use four neighboring cells and shorter glow;
the nearest two retain native cores and their existing 100% and 60% halo images.
The three unused far-layer halo images and passes are absent.

Medium renders the far three at 50% width and height,
then the nearest two over that image at 75% in a second RGBA16Float target.
Its foreground halo targets use 75% and 45% dimensions.
The final pass samples that composite bilinearly,
keeping Texture mix and the underlying spectrogram at native resolution.
Both presets retain all five layers,
star geometry,
and color history.
High remains the default and keeps its existing saved `P3` value;
Medium is a new saved enum variant,
and Uniform retains its manual halo-resolution override.
Switching presets changes sampling without resetting retained colors.

The four-cell gather starts at `floor(r - 0.5)`.
Its radial support is `1 - jitter_span / 2` cell widths,
with a fade over the final 0.15 cells.
The actual jitter span is `0.6 * Jitter`,
so support is 0.85 cells at the default dial and 0.7 at maximum jitter.
Jitter itself is unchanged.
The previous wide support is 1.2 cells:
four neighbors cannot reproduce it simply by dropping five reads.

The Stars **Far fill** slider reduces background leakage through the farthest three layers without adding stars or texture reads.
It works in every rendering profile:
0% preserves the original coverage,
50% gives gentle filling,
and 100% gives stronger filling,
with continuous interpolation between them.
New and older appearances default to 0%.
Changing fill preserves star color history.
It strengthens partial tails and lifetime fades but cannot fill a pixel with zero star contribution.
The [bounded prototype report](https://github.com/yan-h/harmonigraph/tree/e90cca23/docs/evidence/spectrogram-stars/crack-fill-2026-09-28) records the appearance and cost comparison.
The [slider validation](evidence/spectrogram-stars/far-fill-2026-09-28/README.md) records production parity and measured overhead.

Uniform retains all five wide responses and its adjustable halo resolution.
At drawn coverage of at least 2560×1440 device pixels in area,
it composites the farthest three into a native-resolution target and draws the nearest two over an exact texel read.
Smaller Uniform regions retain the unsplit path.
High and Medium use their reduced far-three images at every pane size.
Both policies retain layer order,
palette mixing,
motion,
and color history.

Yan selected the 75% variant after viewing the higher-resolution motion comparison.
No new persisted shape or quality control was added;
saved Optimized appearances adopt the selected distant softness and shorter glow.
The [production evidence](evidence/spectrogram-stars/far-three-75-2026-09-28/README.md) records the implementation and verification in detail.

## Measurements supporting the decision

All measurements here are Apple M1 Pro / Metal offscreen GPU intervals,
not whole-DAW frame times or claims about other hardware.

| Comparison | 1080p reduction | 4K reduction | Interpretation |
| --- | ---: | ---: | --- |
| Native 2+3 split versus pre-split renderer | No dependable gain | About 14–17% | Led to the large-region cutoff; later dependency-ordered production measurements confirm the gain |
| Native 3+2 versus previous P3 | About 3–4% at dense defaults | About 3–4% at dense defaults | Small same-look change shipped in #1245; baseline at 1080p was unsplit |
| Short-glow far three at 75% versus pre-#1245 P3 | 32.9–33.9% | 25.9–26.3% | Research screens plus independent repeat and workload checks |
| Selected production port versus shipped native 3+2 shader | 31.4% | 22.5% | Bounded A/B/B/A synthetic confirmation; full method and limitations in production evidence |

The short75 research confirmations also cover synthetic input,
sparse stars,
and a 926×720 live-sized pane.
The latter saved about 20.9% in that fixture.
The final production confirmation retains a disturbed initial 4K sequence but uses its stable repeat for the reported percentage.

The accepted 4K video versus the previous look had whole-frame mean absolute channel error about 0.238/255 across 144 frames.
The production port versus the accepted prototype is a different comparison:
99.9943% of pixels match exactly across the warmed 768×432 movie,
and 99.9944% match in the saved 4K frame;
every remaining channel differs by at most 1/255.
Pixel averages describe these captures and do not certify perceptual equivalence across settings.

A [follow-up against the current profile](evidence/spectrogram-stars/resolution-followup-2026-09-28/README.md) measured back-three 50% and foreground 75% variants.
Back-only 50% saved about 10.5% at 4K with no dependable 1080p gain;
scaling foreground cores and existing halo dimensions to 75% saved about 34.8%/20.2% at 4K/1080p in the synthetic probe.
Recording-derived captures quantify the accompanying softness.
The cores-and-halos variant is now available as Medium;
High remains the default.
The [production confirmation](evidence/spectrogram-stars/medium-preset-2026-09-28/README.md) records the port and its interleaved timing checks.

## What was tried and what remains worth considering

This table is a decision index,
not a list of supported modes.
The frozen reports below preserve exact implementations,
run-level results,
and visual comparisons.

| Family | Finding and disposition |
| --- | --- |
| Neighbor-row unrolling and baked inverse sigma | Shipped early; keep the depth loop. Historical measurements suggested roughly 16%/20% at 1080p/4K, but used the older timestamp bracket. Reciprocal packing changed half-float rounding by at most one channel level in tested frames. |
| Native partitions | 3+2 was the strongest simple follow-up and shipped. 1+4 had no dependable 1080p benefit and was slower at 4K. |
| Aligned native complete response | About 2.6–2.9% at 4K P3, no dependable 1080p benefit; about 8–12%/10% for Uniform Full. Not shipped. Eligibility/alignment fallback adds maintenance cost, and those baselines predate the selected port. |
| Four-neighbor direct or residual response on all layers | Substantial savings, but a larger glow change than the selected far-three policy. Direct and residual paths have different sampling grids and are not visually identical. |
| Fixed depth masks | Wide-glow masks 1, 2, 3, 4, 8, 12, 16, 24 and 28 were screened. Mask 24 keeps the nearest two wide; the native far-three short-glow group closely matched its movie with a simpler production structure. |
| Far-three short glow at native dimensions | One balanced run saved 26.1%/16.9% against old P3. Simpler coordinates and retained sharpness, but less saving than the visually accepted 75% target. |
| Far-three short glow at 50%, or wide glow at 50% | Faster alternatives with more distant softness. Both were measured; neither was selected. The wide50 family also received workload confirmations. |
| All-five reduced-resolution groups | Wide glow at 75% saved roughly 37–40%/31%; at 50%, roughly 53–55%/56.5% against old P3. Softer foreground detail made the visual tradeoff worse. |
| Omit impossible neighboring core work | Small same-look gains around 2% in confirmation; environment-sensitive and not enough to justify additional machinery in this port. |
| Fused seed/horizontal filtering | Roughly 2.1× existing blur cost at 1080p and 20–22% slower at 4K. Rejected. Matching a shared blurred image does not reconstruct each analytic halo. |
| Seed quads, sprites and shared bloom | Previously investigated. Additional draws/filtering or changed halo structure did not establish a better visual-plus-maintenance tradeoff. Consult the archived evidence and #1242 before retrying. |
| Falloff lookup, packed atlas, early/squared rejection, tail truncation, full depth unrolling | No useful demonstrated gain; several were slower. The lookup was 18–41% slower in paired dense-default tests. |
| f16, wider native cores, RGBA8 halos, boundary tweaks | No repeatable useful win or an unfavorable appearance tradeoff in the tested screens. |
| Skip unused immediate-color work during memory | Only 0.2–3.3%, nearly tied in the longer 4K repeat. Not selected. |
| Fewer layers or reduced jitter | Measured appearance changes, not equivalent-look optimizations; the selected design preserves both. |
| Temporal reuse or caching the star bake | No valid moving-picture cache was established. Drift, life, and sampled light evolve every frame; a frozen diagnostic is not an implementable cache. |

Uniform Full complete-response work is the clearest narrowly scoped remaining measured lead if that override matters in actual use.
Otherwise profile the selected implementation before reviving older candidates:
its cost distribution has changed.
Adaptive neighbors,
silent-tile masks,
and temporal reuse remain hypotheses rather than demonstrated cost-effective designs.
A small static or stale-field speedup is insufficient evidence for their complexity.

## Measuring the next change

The maintained probe is `spectrogram::tests::timing::cloud_costs_by_style_and_dial`.
A synthetic baseline can be run without the private recording:

```sh
HARMONIGRAPH_SHADER_ASSETS=source PROBE_CASE=stars PROBE_FRAMES=240 PROBE_FILLS=1 \
  PROBE_SIZE=3840x2160 PROBE_PPP=4 \
  cargo test --release -p harmonigraph-render cloud_costs_by_style_and_dial \
  -- --ignored --nocapture --test-threads=1
```

This selects the Stars default and memory cases.
Use 1920×1080 at 2 px/point to preserve the same logical pane size.
A single process is a baseline or smoke check,
not the full paired experiment protocol.
Read the probe's module documentation for all controls.

Use the real source-BEGIN through dependent-composite-END interval (`source/full`).
[Issue #1203](https://github.com/yan-h/harmonigraph/issues/1203) demonstrated that independent opening-pass `end/full` timestamps can reverse or undercount on this tile-based GPU.
Earlier reports retain that limitation;
later dependency-ordered split measurements are linked separately.
Do not compare unlike intervals or add percentages across baselines.

For a claimed gain,
use warmup,
byte-identical A/A controls,
and balanced or interleaved candidate ordering;
retain raw measurements and exclude disturbed runs as whole runs with a stated reason.
Avoid simultaneous builds,
captures,
video encoding,
or other graphics-heavy work.
Descriptive bootstrap ranges do not eliminate background-load uncertainty.
Cover dense/sparse settings,
small and large panes,
and synthetic plus recording-derived input when available.

Keep captures warmed long enough to fill visible history and color memory.
Inspect motion and foreground grain,
not just averaged still-image error.
Check jitter extremes,
life transitions,
silence,
fractional scale/origin,
partial-region edges,
and profile transitions.
Sampling/layout changes must preserve color history when its identity is unchanged.
Future shader changes still owe reviewed goldens and a runner-generated Metal corpus.

## Frozen research archives

These annotated tags preserve the exact final snapshots of the closed research PRs.
Their reports are historical:
recommendations or statements that appearance is undecided are superseded by the selection above.
They are not extra production modes or maintained benchmark implementations.

| Research PR | Archive tag / pinned report | Contents |
| --- | --- | --- |
| [#1170](https://github.com/yan-h/harmonigraph/pull/1170) | `archive/stars-research-pr-1170`; [report](https://github.com/yan-h/harmonigraph/blob/a8e1166b695ed9c95eb330b3b4fd516704dc2ddd/docs/spectrogram-star-performance.md), [paired-run recipe](https://github.com/yan-h/harmonigraph/blob/a8e1166b695ed9c95eb330b3b4fd516704dc2ddd/docs/evidence/spectrogram-stars/round3/confidence/README.md) | Initial alternatives, layer split, image metrics, raw samples, analysis and scratch patches |
| [#1244](https://github.com/yan-h/harmonigraph/pull/1244) | `archive/stars-research-pr-1244`; [report](https://github.com/yan-h/harmonigraph/blob/b548e62f3cc0d6ddcdac757c7a04dbc997efccc3/docs/evidence/spectrogram-stars/next-experiments-2026-09-27/REPORT.md), [replay recipe](https://github.com/yan-h/harmonigraph/blob/b548e62f3cc0d6ddcdac757c7a04dbc997efccc3/docs/evidence/spectrogram-stars/next-experiments-2026-09-27/README.md) | Complete response, native partitions, fused filtering, controls and limitations |
| [#1246](https://github.com/yan-h/harmonigraph/pull/1246) | `archive/stars-research-pr-1246`; [report](https://github.com/yan-h/harmonigraph/blob/fcdf11f600a4b4e5895325df00cd643a0fdccff9/docs/evidence/spectrogram-stars/architecture-trials-2026-09-28/REPORT.md), [all results](https://github.com/yan-h/harmonigraph/blob/fcdf11f600a4b4e5895325df00cd643a0fdccff9/docs/evidence/spectrogram-stars/architecture-trials-2026-09-28/RESULTS.md), [replay recipe](https://github.com/yan-h/harmonigraph/blob/fcdf11f600a4b4e5895325df00cd643a0fdccff9/docs/evidence/spectrogram-stars/architecture-trials-2026-09-28/README.md) | Architecture masks/groups, exact generated shaders, raw runs, coverage checks, comparison sheets and 4K provenance |

Fetch an archive tag explicitly if it is absent locally,
then read or export its files with `git show` or `git archive`.
Copy the bundle outside an owner-managed experiment worktree and apply its scratch patch only at the recipe's pinned production base.
The archive tag is the evidence snapshot,
not necessarily the base the patch applies to.
Never load a plugin built with research hooks as if it were production.

Synthetic replay is self-contained where the archived recipe says so.
Private recordings,
large movies,
and raw frame arrays were not committed;
the archives preserve hashes and provenance but cannot recreate missing private inputs.

Keep this current report,
the reusable probe,
regression tests,
and small implementation-validation records on main.
Keep complete one-off trial collections in these frozen archives.
Do not rewrite old merged history to reclaim small evidence files or continually port old scratch patches to current production.

## Picture-changing candidates, measured but not shipped

This historical link target is retained for the [early prototype bundle](evidence/spectrogram-stars/README.md).
Its individual A–E geometry and old measurements remain available in the [pre-consolidation report](https://github.com/yan-h/harmonigraph/blob/c5c0cb77596b899db818459eb4b521e0f7fa75d0/docs/spectrogram-star-performance.md#picture-changing-candidates-measured-but-not-shipped).
The current selected architecture is described above;
old statements that reduced-resolution RGB remains unmeasured no longer apply.
