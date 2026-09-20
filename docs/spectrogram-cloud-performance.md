# Spectrogram cloud performance audit

Audit of `8b4edf4e` on 2026-09-19, prompted by the Mosaic and Watercolor textures dropping a 4K pane from 144 fps to 60-100 while the plain spectrogram holds 144.
It adds a repeatable probe and ranks the levers; it changes no shader.

The short version: **the cell walks are the whole cost, and nothing they compute depends on the sound.** Mosaic spends about 11 of its 12.5 ms walking 18 domes per pixel, Watercolor about 34 of its 37 ms walking 50 globs per pixel, and everything else either texture does —
the blur, the light taps, the shading, the terraces, the palette —
fits in 1.5 to 3 ms.
A frame at 144 Hz is 6.9 ms, shared with the lattice and egui.

## Measurement

The ignored `cloud_costs_by_style_and_dial` probe in `crates/harmonigraph-render/src/spectrogram/tests/timing.rs` draws the shipping `prepare` and `paint` into a held 3840 by 2160 target at 2 pixels per point,
over a 4096-slot ring of 3828-bucket slabs with 1024 on screen, eight octaves visible and ten seconds of history across the pane.
Settings are `SpectralAtmosphere::default()` with one dial turned per case.

```sh
cargo test --release -p harmonigraph-render cloud_costs_by_style_and_dial -- --ignored --nocapture --test-threads=1
PROBE_SIZE=1920x1080 PROBE_PPP=1 PROBE_CASE=watercolor cargo test --release -p harmonigraph-render cloud_costs_by_style_and_dial -- --ignored --nocapture --test-threads=1
```

Two things about the probe are load-bearing, and the first version of it had neither.

**Cases are interleaved, one frame of each per round.** Run back to back, `Variety` 0 read 50% SLOWER than `Variety` 0.5, which is a GPU changing its clock under a run rather than anything the dial does;
interleaved, the two agree to 1%.
Bitwig was drawing the live plugin on the same GPU throughout, so medians are inflated by some common amount and the ratios are the evidence.
Timestamp minimums are often zero and mean nothing.

**Every stamp is the END of a pass.** A tile-based GPU runs a later pass's vertex stage ahead of an earlier pass's fragments,
so a beginning-of-pass stamp after `prepare` landed before the light field's fragment work and billed it to the paint pass.
End stamps are better and still not a clean split:
the stamp between `prepare` and `paint` is an independent one-texel pass that nothing orders against its neighbours,
and the tone pass below — encoded in `prepare` — reads out under `paint`.
**The sum of the two columns is the figure to trust**, and the split is a hint.

## Readings on Apple M1 Pro (14-core) / Metal

Median GPU time per frame, history covering the whole pane.
`light` is the density source, four blur passes and the bake; `paint` is the backdrop and the composite.

| Case | light ms | paint ms at 3840x2160 | paint ms at 1920x1080 |
| --- | --- | --- | --- |
| plain | — | under 2 | under 1 |
| blur only | 0.6 | 0.8 | 0.4 |
| blur + terraces | 0.6 | 0.8 | 0.5 |
| Mosaic, defaults | 0.6 | 13.0 | 3.2 |
| Mosaic, no terraces / `Variety` 0 / `Rock` 1 | 0.6 | 13.2 / 13.1 / 12.6 | 3.2 / 3.1 / 3.1 |
| Mosaic, zero softness | 4.8 | 23.5 | 3.7 |
| Watercolor, defaults | 0.7 | 37.3 | 9.7 |
| Watercolor, `Layers` 0 | 0.6 | 22.2 | 5.0 |
| Watercolor, `Lobe shape` 0 | 0.6 | 35.5 | 9.5 |
| Watercolor, `Ragged` 0 | 0.7 | 36.1 | 9.5 |
| Watercolor, `Lobe shape`, `Ragged` and `Layers` all 0 | 0.6 | 19.9 | 4.3 |

What the table says:

- **Cost is linear in pane pixels.**
A quarter of the pixels is 3.9 to 4.1 times faster in every cloud row.
- **The blurred light field is free.**
At the default softness the field is reduced to a few hundred texels a side and all six passes cost 0.6 ms.
At ZERO softness it is full resolution and costs 4.8 ms, and the composite's ten displaced reads of two 4K `R16Float` textures add another 10 ms —
so turning the blur off to save time does the opposite.
- **`Layers` is the only Watercolor dial that buys anything**, 15 ms, because it removes one of the two walks.
`Lobe shape` and `Ragged` are 1 to 2 ms each.
- **The uniform branches around `Variety`'s `exp2` and `Rock`'s sine pair save nothing measurable.**
They cost nothing either, so they stay.

### Where the time goes inside the two shaders

Throwaway edits to `spectrogram.wgsl`, measured with the same probe and reverted; none is in this PR.

| Edit | paint ms | Reading |
| --- | --- | --- |
| Mosaic, `cloud_domes` replaced by a trivial face | 1.35 | the two 3x3 dome rings are about 11 of 12.5 ms |
| Mosaic, the four gradient taps removed | 11.7 | 8 of the 10 light reads cost 0.8 ms |
| Watercolor, `wash_scan` replaced by a trivial glob | 3.15 | the two 5x5 rings are about 34 of 37 ms; the `Lobe shape` noise is 1.4 of the rest |
| Watercolor, `WASH_RING` 1 (3x3, 18 visits for 50) | 19.7 | about 0.55 ms per visited cell at 4K, and the sparse fine octave's early return does not make its cells cheaper |
| Watercolor, the hash's two avalanche rounds removed | 32.6 | hashing is about an eighth; the rest is the per-cell geometry and top-two bookkeeping |

The walk cannot be culled exactly.
At `Fuzz` 1 the bleed window reaches 0.9 of a radius past a rim, so a glob contributes until its rim coordinate passes 1.9,
which for the mean radius is 2.5 cells out — about 20 of the 25 cells visited.
The ring is the look's own definition rather than slack around it.

### A hypothesis the probe refuted

`paint` draws the backdrop over the whole region and then the measured mesh over the same pixels, both through `EGUI_BLEND`,
which reads as the cloud shader running twice per covered pixel.
It does not cost that here:
a pane 100% covered by history and one 2% covered time the same, and skipping the backdrop draw outright left a fully covered pane at 12.5 ms.
The mechanism was not established —
both entry points return a constant alpha of 1, which may be what lets the M1's hidden-surface removal treat the blended draw as opaque —
only the result.
An immediate-mode GPU would likely pay the second draw;
nothing this plugin runs on is one.

## Levers

The cost is `pixels x cell visits x frames`, the walk is arithmetic-bound, and no factor is wasted inside it —
so each lever removes one of the three.
These are proposals; only the first has a measured bound.

**Lever 1 is built**, as the `Cloud pixel size` dial:
it runs 0.5 to 4 points per sample, is native at the fresh 0.5 on a Retina pane, and swallows lever 3 by being a dial rather than one fixed reduction.
`PROBE_CLOUD_PIXEL` re-reads the table above at any of its settings.
Measured at 3840x2160 and 2 px/pt, light plus paint, median ms per frame:

| `Cloud pixel size` | Mosaic | Watercolor |
| --- | --- | --- |
| 0.5 pt (native) | 13.6 | 38.0 |
| 1 pt | 4.9 | 12.9 |
| 1.5 pt | 2.7 | 5.0 |
| 2 pt | 2.2 | 3.5 |
| 4 pt | 1.7 | 2.1 |

Mosaic is inside the 6.9 ms frame from 1 pt and Watercolor from 1.5 pt.
Past 2 pt there is little left to buy: the floor is the 1.4 to 3 ms that is not the walk.

| Priority | Change | Expected | What it costs |
| --- | --- | --- | --- |
| 1 | Draw the cloud's scalar TONE into a half-resolution `R16Float` target (one sample per point at 2 px/pt), and have the full-resolution composite read it, look the palette up and mix the base | Measured bound: Mosaic 13.0 to about 3.2 + 1, Watercolor 37.3 to about 9.7 + 1 | One pass and one target per pane. Rims soften by half a point; at `Fuzz` 1 the rims are already tens of points wide. Terraces and the measured core stay full resolution because the base is still composited there. Needs Yan's eye at `Fuzz` 0 and on Mosaic's creases, so it belongs on a dial first |
| 2 | Bake what the walks produce and redraw it only when it moves | Per-frame cost falls to the 1.4 / 3.2 ms residue; the walk is paid once per whole-pixel step of the drift, which at `Cloud speed` 1x on a 2160-pixel pane is 10 to 30 times a second rather than 144 | See below — this is the cache-key lever and carries that risk |
| 3 | Watercolor: a third-resolution tone target | About 37.3 / 9 + 1 | As 1, at 1.5 points per sample; only worth trying if 1 lands and is still short |
| 4 | Watercolor: per-cell glob parameters from a small texture instead of two hashes | At most the 12% the hash measures | Exactness rests on CPU and GPU agreeing on `f32(n & 0x3ff) / 1023.0` |

Lever 1 alone puts Mosaic inside the 144 Hz budget and leaves Watercolor near 11 ms, about 90 fps;
Watercolor needs 1 with either 2 or 3 to hold 144 on a full 4K pane.

### What lever 2 is, and what its key must be

Neither walk reads the sound.
`dome_octave` and `wash_scan` are functions of the pane position plus `drift` and of the texture's own dials;
the light enters afterwards, through the offsets they return.
So the walk's OUTPUT can live in a texture —
Mosaic's `face` and `to_centre` in one `Rgba16Float`, Watercolor's two look offsets, two pigments and the fine cover in two —
and the per-frame shader becomes those reads plus the light taps it already does.

The drift moves about a tenth of a pixel per frame, so an exact bake needs it snapped to whole target pixels, and the walk then reruns once per step.
That is where the design gets its edges:

- A 35 ms rebake in one frame is a hitch five frames long, so the rebake has to be spread over the frames before the step it is for, into a second target, and swapped.
- The key is the snapped drift, the target size, `ppp`, the style and that style's own dials — and NOT `time`, the light, the palette or the softness.
`Rock` above zero puts `time` into the walk's output and defeats the cache outright; it ships at zero.
- `Cloud speed` reaches 20x, where a step arrives every few frames and the lever has nothing left to give.
- Whether one-pixel steps in a field this soft are visible is a question for the eye, not the probe.

It stacks with lever 1, and lever 1 is the smaller change with the measured number, so it goes first.

## Similar, not identical (2026-09-19)

Everything above holds the picture EXACT: lever 1 defaults to native so the goldens stay byte-identical, lever 2 snaps the drift so a bake reproduces the live walk, and ring culling was refuted as an exact cull.
Yan then asked the other question — what opens up if the texture only has to look SIMILAR — and the answer is that lever 2's three edges were all exactness, not the bake.

**The walk's output is a fixed field that slides.** `cloud.drift` enters both styles only as `q = ... + cloud.drift`, a translation, and `cloud.time` appeared only inside `Rock`, which shipped at 0 and is now retired (below).
So the drift does not belong in a cache key at all: it is a UV offset into a field that never changes.
Snapping it, rebaking per step, spreading a 35 ms rebake over frames and losing the lever at `Cloud speed` 20x were the price of reproducing the live walk texel for texel.
Sampled bilinearly at the fractional offset instead, the motion stays smooth and nothing is ever rebaked because of time.

### The periodic tile (built: the `Cloud tile` dial, PR #991)

Wrap the cell hash every `P` cells and the field is periodic, so ONE tile of `P` by `P` cells, baked once and read through a repeat sampler, is the whole plane.
`Cloud tile` is `P` in cells: 0 is off — the live walk, and the fresh value, so the goldens do not move — and 20 and 40 are the two periods on offer.

Measured with `PROBE_CLOUD_TILE` at 3840x2160 and 2 px/pt, light plus paint, median ms per frame, Bitwig on the same GPU:

| Case | Mosaic | Watercolor |
| --- | --- | --- |
| tile off, native `Cloud pixel size` | 11.65 | 33.14 |
| `Cloud tile` 20 | 2.72 | 2.35 |
| `Cloud tile` 40 | 2.67 | 2.39 |
| `Cloud tile` 20 with `Cloud pixel size` 1.5 pt | 1.88 | 2.00 |
| tile off, repeated at the end of the run | 11.20 | 31.84 |

The repeated row is 4% under the first, which is the run's noise; the ratios are 4x and 14x.
The period costs nothing per frame, since the walk is paid once either way.
Tiled, Watercolor is CHEAPER than Mosaic: what is left per pixel is two light taps for the wash and ten for the mosaic's sun.
Both held 144 Hz in that original run on a full 4K pane at NATIVE resolution, which `Cloud pixel size` could only buy with softness — so that dial, its target, its pass and its branch are the next retirement candidate once the tile has been judged.

The exact rotation was measured again on 2026-09-20 in the same 3840x2160 probe, with `P = 40` and forty frames per case.
Against the same run's live walk, Mosaic paint fell from 14.23 to 2.66 ms and Watercolor from 37.54 to 3.65 ms: 5.3x and 10.3x.
Yan then kept the rotation only on Watercolor because turning the Mosaic scale pile changed its look.
Mosaic returns to the original square lookup measured at 2.67 ms above; Watercolor's extra inverse rotation and two vector rotations preserve its order-of-magnitude benefit.
The pure rotation also returns P20's 1080-high wash bake from the stagger's 768² target to 512².

- **What is baked** is the walk's output and never the tone, because the tone carries the sound.
Mosaic: `face` and `to_centre`, one `Rgba16Float`.
Watercolor: per octave the finished look offset (`look - r`, after feather and bleed) and the pigment, plus the fine octave's `cover` — seven channels, two `Rgba16Float`.
The per-frame shader keeps the light taps, the lean, the shading and the palette, which is the measured 1.35 ms (Mosaic) and 3.15 ms (Watercolor) residue from the throwaway-edit table above, plus one or two coherent texture reads.
- **Why `P` is a multiple of 10.**
Both styles run a second octave at a lacunarity of 2.1, so the fine octave tiles when `2.1 * P` is an integer; the wash's warp noise lattice sits at 0.9 of a cell, which wants `0.9 * P` whole as well.
`P = 20` gives 42 and 18.
A ragged noise at 2.8 was the third constraint until `Ragged` was retired (below).
The noise's own second octave is at 2.07, which no `P` makes whole, so the tiled path runs it at 2.0 — the one constant the tile changes, and only when the tile is on.
- **The key** is the style, `P`, the tile's texel size, the Watercolor pane orientation and the dials the walk reads: `Variety` for Mosaic; `Lobe shape`, `Fuzz` and `Pool` for Watercolor.
NOT the drift, the clock, the light, the palette, the softness, `Refraction`, `Relief`, `Cloud depth` or `Layers` — none of them reaches the baked channels, and the bake always walks the fine octave so `Layers` is a mix over channels already held.
The size dials and the pane reach it only through the texel size, which is as fine as the pane draws a cell, rounded up to a multiple of 256 and capped at 2048, so a resize drag does not rebake on every frame.
The tile is also carried across a rebuild of the light field's targets, which a zoom or a Span drag forces.
- **`Rock` and `Grain` were retired with it, on Yan's call.**
`Rock` was the only reader of the clock in either walk and could not be baked — each dome turns at its own hashed rate, and interpolating a RATE across a bisector spins the phase without bound — so keeping it meant keeping a live-walk fallback for a sway worth at most 5% of tone over a 12 to 31 s cycle, and about 1% wherever the sound's light is flat.
`Grain` shipped at 0 and still ran a smoothstep per visited cell to count the pile.
Removing `Rock`'s accumulator moved `spectrogram-zoomed-in` by 1/255 on three pixels: the Metal compiler scheduling the dome ring differently, isolated by putting a dead accumulator back, which restores the frame.
- **Inside the first period the tile IS the live field**, since a wrapped hash equals the unwrapped one for cells in `[0, P)`.
That is what makes "tiled matches walked" testable, up to bilinear resampling, half-float storage and the 2.07 to 2.0 change.
`a_tiled_cloud_draws_the_live_walk_inside_its_first_period` holds it: mean difference 0.02/255 for the mosaic and 0.37/255 for the wash, worst channel 1 and 21 — the 21 is where the wash's stored offset steps because the glob UNDER the visible one changes, a discontinuity the live walk has too and one texel of bilinear smooths.

What it spends of "similar": up to half a texel of bilinear softening that varies with the drift's phase; half-float offsets, under a tenth of a pixel of lookup error; and visible REPETITION, which is the open question.
At the fresh sizes a 4K pane is about 52 by 93 wash cells and 27 by 48 mosaic cells, so `P = 20` repeats the glob outlines 2.6 by 4.6 times (wash) and 1.4 by 2.4 times (mosaic), each repeat refracting different sound.

The original square repetition lined those copies up on the same pitch rows, which made them easiest to find against long horizontal harmonics.
Issue #992 first tried B10's ten-cell stagger, then replaced it with an exact 3-4-5 rotation of Watercolor's complete baked field.
Mosaic keeps the original square recurrence because rotating its scale pile did not preserve the look.
Watercolor's stored texture and every hash lattice inside it remain square and seamless; the read applies the inverse 36.87-degree rotation to its coordinates and turns the stored direction vectors forward with the picture.
Its two repeat vectors are `(4P/5, 3P/5)` and `(-3P/5, 4P/5)` in `(time, pitch)`, so neither recurrence axis follows a pane axis.
At `P = 20` the same geometry returns to a pitch row after 100 time cells; at `P = 40` it takes 200.
The Watercolor rotation follows the pane's time and pitch axes when its orientation changes, and as an isometry it needs none of the extra tile resolution the stagger's shear did.
Whether the eye finds that is Yan's call, and the dial exists so he can compare `P` against the live walk in the DAW.

### The alternative if repetition shows: a scrolling window

A pane-sized bake with toroidal addressing, walking only the strip the drift newly exposes each frame — a column every ten frames or so at `Cloud speed` 1x, two columns a frame at 20x.
It never repeats.
What it costs over the tile: about 130 MB of `Rgba16Float` at native 4K for Watercolor (it stacks with a coarser bake, as `Cloud pixel size` does); a full 35 ms walk on every frame of a dial drag or a pane resize, which is no worse than today; and incremental validity state — which strips are current — that is exactly the carry-forward cache this repo's `CLAUDE.md` warns about.
Unbuilt and unmeasured.
Try it only if the tile's repetition is rejected at every `P` that fits in memory.

### Thinning the walk itself: dominated, and why

Recorded so they are not re-derived.
Neither is built or measured beyond the arithmetic here, and the bake removes the walk they would only shorten.

- **Watercolor, a half-cell-centred 4x4 ring** (`base = floor(r - 0.5)`, visit `0..3`): 16 visits for 25, about 37 ms to about 25.
The nearest unvisited centre is then `2.0 - (JITTER / 2) * sqrt(2) = 1.717` away, which is the whole of what `RADIUS_MAX` may be now that `Ragged` is retired and the reach bound no longer carries `(1 + RAGGED)` — so `RADIUS_MAX` falls from 1.91 to under 1.717 and the radius band from 1.63:1 to about 1.47:1.
(Derived at the shipped `RAGGED` it was worse still: 1.66 to 1.32, a band of about 1.3:1.) That band is the "different sized globs" the wash was asked to be, so this is a look regression for a third of the walk.
A plain `WASH_RING = 1` measured 19.7 ms, but it breaks the reach proof and draws steps on the cell grid — not a similar picture, a broken one.
- **Mosaic, the fine octave replaced by a cheap noise gradient.**
The fine octave is half the walk, about 5.5 ms, and contributes only slope at a gain of 0.22.
Four hashes for nine hash-sqrt-exp visits would save perhaps 4 of 13 ms.
- **Updating the tone at a lower rate, checkerboarding, temporal reprojection.**
The tone reads the sound, so a stale tone lags the music; the walk is the only part safe to hold, and holding it is the bake.

### Open questions for whoever continues

1. Does repetition read at `P = 20` and `P = 40`, on both styles, over real music and at `Cloud speed` 1x and 20x? (Yan's eye; the `Cloud tile` dial.)
2. Does the drift-phase softening shimmer at `Fuzz` 0 and on Mosaic's creases? If it does, bake at 1.5x texel density before reaching for anything cleverer.
3. Which `Cloud tile` becomes the default, and whether it stays a dial at all once a period is picked — a default change moves the goldens and owes a regenerated Metal corpus.
4. If the tile becomes the default: retire `Cloud pixel size`, and retire the live walk with it? The live walk is then only the reference the first-period test compares against.
5. ~~`Ragged`~~ RETIRED in the PR stacked on #991. It shipped at 1.0 and its wobble was one-sided, so the radius band was scaled by 1.15 (`WASH_RADIUS_MIN` 1.02 → 1.17, `WASH_RADIUS_MAX` 1.66 → 1.91) to keep the default globs their size. The reach bound stopped carrying `(1 + RAGGED)` and `RADIUS_MAX` now sits 1.91 against a bound of 2.217, so a band WIDER than 1.63:1 is available and deliberately untaken — a look change for Yan's eye (#992).
6. The scrolling window above, only if 1 fails.

## Source map at `8b4edf4e`

- The two walks: `crates/harmonigraph-render/src/shaders/spectrogram.wgsl`, `dome_octave` / `cloud_domes` and `wash_glob` / `wash_scan`.
- What reads them: `scale_clouds` and `wash_clouds` / `wash_tone`, reached from `clouded`.
- Light field sizing and passes: `crates/harmonigraph-render/src/spectrogram/atmosphere.rs`, `source_size` and `Targets::blur`; scheduling in `SpectrogramCallback::prepare`.
- The backdrop and composite draws: `SpectrogramCallback::paint`.
