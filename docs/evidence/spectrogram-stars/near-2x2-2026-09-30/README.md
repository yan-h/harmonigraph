# Near-two 2×2 Stars reads: research handoff (PR #1321)

State on 2026-09-30, written so a later session can pick this up cold.
PR #1321 is a research draft and is **not for merge as-is**:
the useful result gets ported to production in a separate, clean PR (see "Next steps").

## Why this exists

Stars cost is dominated by per-pixel neighbour gathering (see `docs/spectrogram-star-performance.md` and issue #1142).
Today the far three depth layers already draw a complete 2×2 response at reduced resolution,
while the near two draw a native one-read core plus a halo image filled by a 3×3 gather.
Yan asked to measure 2×2 on the near two layers alone, on top of today's renderer.
The earlier 2×2 rejection (#1170 round 1, Yan: "I do prefer A") was for ALL five layers with the halo faded at 0.6–0.7 cell,
so it did not settle this narrower case.

He then asked whether a star could keep its footprint while more of it becomes glow:
Yan, verbatim: *"I want the option of maintaining the star footprint, but decrease the core so more of it is glow."*
That is the `HARMONIGRAPH_STARS_NEAR_CORE` knob below.

## The geometry, in one paragraph

Each depth layer is a grid with one star per cell per life.
A star sits within its cell's jitter box, `0.5 ± 0.3·Position variation` cells from the corner,
so it can be as close as `0.5 − 0.3·J` cells to its cell's edge.
A pixel reads a fixed set of cells.
The radius within which EVERY star is guaranteed to be read, the support, is:

- 2×2 cells, starting at `floor(r − 0.5)`: `1 − 0.3·J` cells (0.85 at the default J 0.5, 0.70 at J 1);
- 3×3 cells: `1.5 − 0.3·J`, which production fixes at 1.2, the J 1 value.

Glow past the support is either cut by the shader's fade (production) or leaves seams along cell edges.
Glow reach is measured in cells, and near cells are large (the nearest is ~6.6 star px, a star px being pane height / 540),
so 0.85 cell is still a wide halo on screen.

## What the branch contains

Branch `worktree-stars-near-2x2`, stacked on the #1320 research plumbing.
#1320 is closed, and its P1/P1x/P2 prototypes are frozen as tag `archive/stars-research-pr-1320`.
Those prototypes are still in this tree but are not part of this study.

Research switch `HARMONIGRAPH_STARS_PROTO`, read once per renderer (`StarsProto::from_env` in `crates/harmonigraph-render/src/stars.rs`).
It reaches the shaders as the pipeline override `STAR_PROTO`, declared in `shaders/stars.wgsl`:

| Value | `STAR_PROTO` | What the near two layers draw |
|---|---|---|
| `off` (default) | 0 | Production, byte-identical: the three offline Stars goldens pass unblessed. |
| `n1` | 4 | Native core unchanged; the halo passes gather 2×2 (`star_near_gather(s, r, true)`) instead of 3×3. Residual = max(full − core, 0), support `1 − 0.3·J`, fading over the last 0.15 cell. The halo targets are the profile's usual sizes. |
| `n2` | 5 | No near halo passes, targets or samples. Each near layer draws its complete response from a 2×2 read (`star_near_gather(s, r, false)`, the far path's maths) at today's near resolution: native in High's final pass, the 75% near pass under Medium. |

The far three layers, colour memory and profile sizes are production's in both modes.

`HARMONIGRAPH_STARS_NEAR_CORE=<factor>` applies to n1 and n2 only; `off` is unaffected.
When set to any value, 1.0 included:

- the near core's sigma and cap are multiplied by the factor (CPU side, `proto_scale_near` in `stars.rs`, before defocus);
- the near fringe stops decaying in sigma units and is tied to the support instead:
  `fringe · exp(−1.5 · dist / (R · size))`, faded over R's last 0.15 cell (`star_near_texel` in `stars.wgsl`),
  where `size` is the star's own size draw;
- the size draw rides in the lowest mantissa byte of the baked centre's x (near slices, glow mode only),
  which moves the centre by under 3e-5 cell.

**Trap:** the first glow used `exp(−3 · dist / (R · size))`, which is SHORTER than today's fringe.
Today's fringe is `exp(−0.4 · d)` in sigma units, about 0.3 of a near cell.
Commit ce6124bc changed it to 1.5, which holds about a fifth of the centre at the support's edge, so the fade ends the glow.
The constant is hard-coded in `star_near_texel`, not a knob.

Research-only helpers:

- `HARMONIGRAPH_OFFLINE_SPECTRAL_ONLY=1` makes the offline renderer draw the spectral pane alone.
- Always set `HARMONIGRAPH_SHADER_ASSETS=source`: the Metal corpus was deliberately not regenerated on this branch.

## Measured results

Apple M1 Pro, Metal, source shaders.
The probe is `cloud_costs_by_style_and_dial` with `PROBE_CASE="stars-near,blur only"`, 240 frames.
All cases interleave in one process, with two runs per size.
"Stars" is the total minus `blur only`; compare within a run, because the absolute baseline drifts between sessions.
Raw logs are in `timing/`.

| | 1080p@2 stars | 4K@4 stars |
|---|---:|---:|
| Production Medium (fresh default) | 2.08 ms | 5.55 ms |
| n1 Medium | 1.92 (−8%) | 4.58 (−17%) |
| n2 Medium | 1.72 (−17%) | 4.03 (−27%) |
| Production High | 3.77 | 10.93 |
| n1 High | 2.92 (−23%) | 9.32 (−15%) |
| n2 High | 3.23 (−14%) | 9.85 (−10%) |

At Yan's live pane size (roughly the 1080p case), n2 saves about 0.36 ms of the whole spectrogram frame; at 4K, about 1.5 ms.
Under Medium, n2 is the faster and the simpler of the two, since it deletes the near halo passes.
Under High, n1 is faster.

## Look

- **Plain n1 and n2 are nearly indistinguishable from production:** a mean per-channel difference of 1.7/255, 99th percentile 13/255.
  Today's near fringe has mostly died out by 0.85 cell, so the 2×2 support barely cuts anything at default dials.
- **The glow knob changes the near stars modestly.** At core 1.0 their edges are a little softer. At core 0.5 the near stars become crisper points inside a soft glow.
- Mean brightness is unchanged across modes.
- **Yan's verdict on the glow variants is PENDING.** He has the videos but has not chosen.

Evidence in this directory:

- `long-take-medium-{off,n1,n2,n1-glow,n2-glow,n1-core05,n2-core05}.png`:
  Medium, take `take-2026-09-22_15-35-06.take`, last frame of a 28–40 s render.
  "glow" means core 1.0.
  All were re-rendered after ce6124bc, so every glow still uses the 1.5 falloff.
- `crops-2x.png` and `crops-2x-tight.png`:
  2× nearest-neighbour crops, top row off | n1 | n1 glow 1.0 | n1 glow 0.5, bottom row the same for n2.
- Videos were rendered to `/tmp/near2x2/` and are not committed (tens of MB).
  Re-create them with `timing/all.sh`, which runs the timing and then the videos:
  off | n2 | n2 glow 1.0 | n2 glow 0.5, plus a 2×2 comparison of 1:1 crops.

## How to reproduce

From the worktree, with `sccache` on PATH:

```sh
cargo build --release -p harmonigraph-offline
```

```sh
sh docs/evidence/spectrogram-stars/near-2x2-2026-09-30/timing.sh
```

```sh
sh docs/evidence/spectrogram-stars/near-2x2-2026-09-30/render.sh
```

```sh
sh docs/evidence/spectrogram-stars/near-2x2-2026-09-30/montage.sh 1000 800 /tmp/near2x2/crops-2x.png
```

`render.sh` writes to `/tmp/near2x2/`; `montage.sh` expects the `*-00011.png` frames there, and `off` needs one extra render (see `restills.sh`).
Timing needs a quiet GPU: no concurrent builds, captures or video encoding.

**Offline trap:** the renderer has no pre-roll, and the spectrogram history takes about 12 s of song to fill the pane.
Render from 12 s before the window you want, then trim with `ffmpeg -ss 12`;
a single-frame render comes out black.

## Next steps

Pick up from whichever of these Yan chooses.

1. **Port n2 to production, if the look is accepted.**
   Do this in a fresh branch off main, not on this research branch:
   - make the near two layers draw the complete 2×2 response;
   - delete the near halo passes, targets and the `star_halo_*` machinery they alone use;
   - the far three stay as they are.

   Check what this does to the Stars rendering profiles (Uniform / High / Medium / Low), since their near-halo resolutions become meaningless.
   Each profile's saved enum variant is persisted: read the `persistence-contract` skill before dropping any.
   The change owes a regenerated Metal corpus in the same commit (the `metal-corpus` skill) and re-blessed Stars goldens on Yan's command,
   and it must be timed against then-current main.
   The support now depends on Position variation (`1 − 0.3·J`) for the near layers too; production's near halo used a fixed 1.2.
2. **Iterate on the glow shape if Yan wants the core-size idea as a dial.**
   Likely dials are a core size (the knob's factor) and a glow length (the 1.5 constant).
   New persisted fields need the `persistence-contract` skill; the fresh default should reproduce whichever still or video he picks.
   Iterate on the look with the `look-prototype` skill's contact sheets or offline renders before a build.
3. **Close #1321 if neither is wanted.**
   Record the measurements on #1142, then freeze the branch as `archive/stars-research-pr-1321` (annotated tag), as was done for #1320.

## Related history

- #1142: Stars performance research, and the comment of 2026-09-30 on #1320.
- #1242: why three look choices force the gather.
- `docs/spectrogram-star-performance.md`: the decision index of everything tried.
- Tag `archive/stars-research-pr-1320`: the cores + bloom and pre-drawn-tile prototypes. Neither was adopted, and group twinkle was vetoed.
