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
Fragment stages finish in order.

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

## Source map at `8b4edf4e`

- The two walks: `crates/harmonigraph-render/src/shaders/spectrogram.wgsl`, `dome_octave` / `cloud_domes` and `wash_glob` / `wash_scan`.
- What reads them: `scale_clouds` and `wash_clouds` / `wash_tone`, reached from `clouded`.
- Light field sizing and passes: `crates/harmonigraph-render/src/spectrogram/atmosphere.rs`, `source_size` and `Targets::blur`; scheduling in `SpectrogramCallback::prepare`.
- The backdrop and composite draws: `SpectrogramCallback::paint`.
