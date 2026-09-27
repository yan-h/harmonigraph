# Stars optimization: decisions and findings

This records the discussion and experiments through 2026-09-27 for [issue #1142](https://github.com/yan-h/harmonigraph/issues/1142) and [draft PR #1234](https://github.com/yan-h/harmonigraph/pull/1234).
The aim is to make Stars cheaper while preserving the layered dust,
irregular placement and broad glow Yan likes.
The current choice is **five layers,
one native core neighbor per layer,
and separate analytic halos with adjustable resolution**.
Yan preferred keeping those halos after comparing per-depth image blur.
The branch remains a plugin trial;
that choice does not mean the PR has been merged.

## Current controls and rendering

- **Jitter: 0–100%, default 50%.** Controls positional irregularity within each cell. At 100%, the original full jitter works with the full halo reach.
- **Halo resolution: 25–100%, default 50%.** Scales each halo image's width and height. Lower values reduce work and soften the sampled glow; compact star centers stay at native resolution.
- **Five depth layers remain.** Large panes still use the automatic native 2+3 pass split. This changes where the layers are drawn, not how many exist.
- **Fringe and Near star softness remain independent controls.** No per-depth blur selector or combined Star glow control is shipped in this branch.

Each depth gets one compact native-resolution core lookup and a separate nine-neighbor halo image.
The halo is the nonnegative remainder of the full star shape after subtracting that core.
Its weighted palette color and coverage are added to the core before color normalization and ordinary far-to-near composition.
This preserves the depth treatment rather than adding a white bloom over the finished image.
The outer response fades between 0.84 and 1.2 cells at every Jitter setting.

Changing Jitter changes star locations and the light they sample,
so it resets Stars' retained color history.
Changing Halo resolution only changes the sampling grid and preserves that history through reallocation.
The allocation key includes the actual halo dimensions;
resolution does not invalidate star material/history keys.
The shader receives the actual rounded dimensions so odd pane sizes and fractional display scales agree with allocation.

Both controls are saved with the appearance.
Saved appearances missing either field use 50%;
finite out-of-range halo resolutions are clamped to 25–100%,
and nonfinite values fall back to 50%.
There is no compatibility shim.
The slider's 50% default leaves the preceding halo build's golden images unchanged.
See the [separate-halo implementation and evidence](evidence/spectrogram-stars/separate-halos/README.md).

## Where the performance investigation started

Before this trial,
each output pixel considered five depths times nine nearby stars:
45 candidate evaluations,
each with atlas reads,
distance/coverage calculations and color accumulation.
The full-resolution 2+3 split preserved all 45 evaluations.
Several optimizations had already landed:

| Change | Evidence at that stage |
| --- | --- |
| Bake each star's properties once per frame | Original 4K benchmark fell from about 38.8 to 14.2 ms |
| Unroll neighbor rows and precompute inverse size | About 20% less GPU time at 4K and 16% at 1080p in the historical probe |
| Draw two far layers in a separate native-resolution pass | About 14–17% less GPU time from dense 1440p through 4K; smaller panes retain the unsplit path |

These are successive experiments with different settings and timing methods,
not numbers to add together or compare as one series.
The [historical performance log](spectrogram-star-performance.md) and [validated split measurements](evidence/spectrogram-stars/split/README.md) retain their scope.
Issue [#1203](https://github.com/yan-h/harmonigraph/issues/1203) documents the old independent opening-pass timestamp's undercount/reversal problem.
Later paired tests measure from the actual first source pass through the final composite.

Earlier probes implicated the repeated neighborhood evaluation more than star preparation.
Removing falloff exponentials alone did not remove most of the cost;
a lookup-table replacement was 18–41% slower,
and tighter atlas packing also failed to help.
Extra rejection branches and unrolling both neighbors and depths failed to earn their complexity.
Splitting identical work across passes helped,
which makes compiler scheduling or register pressure plausible explanations,
but neither was established as the hardware limit.
Color memory and star baking still repeat some life,
position and lighting work;
combining them has not been measured.

## Fewer layers: what 2+2 would mean

The shipped **2+3** is two far layers in one pass and three near layers in another.
Changing to **2+2** would remove one of the five layers,
nominally reducing the old nine-neighbor walk from 45 to 36 candidates per pixel,
or 20%.
The discussion's **10–20% less total Stars GPU time at 4K** was a planning estimate,
not a benchmark result.
The initial suggestion was to remove the middle layer while retaining the other layers' positions and parallax speeds.
It would thin the dust and reduce depth overlap.

An older three-layer prototype kept layers 0,
2 and 4 and measured about 38% less GPU time at 4K;
that does not establish a 2+2 saving,
especially after changing the halo architecture.
Four neighbors plus four layers would nominally be 16 candidates rather than the old 45,
but that combination was not implemented or timed here.
We kept all five layers throughout the plugin trials.

## Four neighbors and the halo/jitter tradeoff

Four neighbors select the nearest 2×2 cells instead of a 3×3 neighborhood.
With five layers,
that reduces native candidate evaluations from 45 to 20.
The initial historical prototype measured 46.4% less GPU time at 4K and about 38% at 1080p.
After defaults and the native split changed,
[current paired measurements](evidence/spectrogram-stars/four-neighbor-current/README.md) found **39.6–39.8% less at 4K and 35.5–36.3% less at 1080p**.
Those are different baselines.

The price is halo reach.
At full jitter,
star centers can move 0.3 cells from their nominal center;
excluded neighbors can then contribute beyond 0.7 cells.
To avoid missing contributions,
the four-neighbor version must fade to zero by that radius.
It keeps the centers,
colors,
lifetimes and five layers,
but clips the broad faint skirts.
Visually this means crisper isolated pinpoints,
darker gaps,
less connected haze and firmer edges on large soft stars.
Bright band edges and enlarged crops expose it more readily than ordinary playback.

Reducing positional jitter permits wider support without more neighbors:

| Jitter | Safe outer reach with four neighbors | Difference from full-jitter reach |
| --- | ---: | ---: |
| 100% | 0.70 cells | Reference |
| 50% | 0.85 cells | About 21% farther |
| 0% | 1.00 cells | About 43% farther |

Half jitter makes placement more regular while recovering some glow.
The [half-jitter measurements](evidence/spectrogram-stars/half-jitter/README.md) cost about 1.1–2.2% more at 4K than full jitter with four neighbors;
the 1080p difference fit inside control variation.
Making [Jitter configurable](evidence/spectrogram-stars/configurable-jitter/README.md) established no further GPU penalty.
Randomness remains the separate size/brightness variation control.
The earlier four-neighbor slider automatically coupled jitter and halo radius;
the current separate halos remove that coupling.

Four is not a universal minimum.
A single nominal-cell lookup works when the compact core is confined to that cell.
Here its radius ends at `0.5 - 0.3 * jitter` cells:
0.5 at zero jitter,
0.35 at half jitter and 0.2 at full jitter.
Anything outside that core must be gathered separately.
Keeping a broad star in one lookup without that separate work would miss neighboring contributions.

## Why one neighbor is only modestly cheaper than four

The one-neighbor trial changed both the core lookup and halo construction.
At the default 50% halo resolution,
the images have one quarter as many pixels as the final image.
Per depth,
the nominal work is therefore:

```text
Previous four-neighbor renderer:       4
Current native core + reduced halos:  1 + 9 × 0.5² = 3.25
```

Across five depths that is 16.25 rather than 20 candidate evaluations per output pixel,
about 19% fewer,
not a fourfold saving.
It also adds five halo draws,
texture writes and filtered reads,
and restores the wide response that the previous four-neighbor version discarded.
Candidate counts are an intuition for shape work,
not a timing model.

[Paired measurements](evidence/spectrogram-stars/separate-halos/README.md) found **8.3–8.5% less total GPU time at 4K but 3.8–4.0% more at 1080p** versus configurable four-neighbor rendering at 50% jitter.
The current appearance has a wider,
smoother glow and softer fine dust,
with compact centers retained at native resolution.
Yan liked this version.

## What blur, softness, Fringe and halos mean

There are three main spatial-softening mechanisms with five appearance controls,
plus sampling controls and temporal memory:

| Mechanism or control | What it does | Relation to halo rendering |
| --- | --- | --- |
| Pitch softness and Time softness | Blur the underlying spectrogram before stars select their color and brightness | Change the source light, not star outlines |
| Wide blur mix | Mix in a broader version of that source light | Also operates before star shape evaluation |
| Near star softness, internally `star_defocus` | Broaden nearer stars progressively; width scales by `1 + softness × depth²` | Widens both Gaussian and Fringe because both use the same normalized distance; adds no separate pass |
| Fringe | Increase the slowly fading skirt around each star | One term inside the existing analytic halo calculation |
| Separate halo images | Render the full star response minus its compact native core | An implementation of the outer star shape, not an additional independent glow effect |
| Halo resolution | Set how finely those halo images are sampled | Lower settings introduce extra sampling softness and reduce evaluations |
| Color memory | Smooth color changes across time | Temporal persistence, not spatial blur |
| Contour edge softness / Blur time step | Soften underlying terraces / control source blur sampling | Separate from the Stars halo shape |

Using the internal name “defocus” earlier obscured the actual UI label,
**Near star softness**.
It means making an individual star look out of focus,
with nearer layers affected more than the farthest one.
Fringe can preserve a sharper center while adding a longer skirt;
softness broadens the whole shape.
Their looks overlap,
but they are not identical controls.

For distance `d` in units of a star's width,
the full response is approximately:

```text
min(exp(-0.5 × d²) + Fringe × exp(-0.4 × d), 1)
halo = max(full response - compact core, 0)
```

The implementation also applies lifetime and outer-support fades.
Fringe's exponential falls off more slowly than the Gaussian.
**Fringe zero still leaves Gaussian tails and all five halo draws.** The halo images evaluate analytic shapes;
they are not a convolution blur,
although filtering them back to native resolution adds softness.
The lattice's general bloom pipeline is separate from this Stars construction.

Calling Fringe “cheap” means its incremental math is cheap after paying for neighborhood evaluation.
It cannot replace the halo machinery for free:
neighboring stars still need to contribute their skirts to the pixel.
Using only a Fringe tail could simplify some math and change the shape,
but would retain neighbor gathering and image traffic.
Consolidating Fringe and Near star softness into one appearance control was discussed;
it could simplify the UI while sacrificing independent width/tail choices,
not remove the major GPU work.
No such consolidation was selected.

## Current bottleneck and the blur experiment

A [paired halo-freeze probe](evidence/spectrogram-stars/halo-cost/README.md) kept warmed halo images and skipped only their five update draws.
It reduced total GPU time by **about 30% at 4K and 33% at 1080p**;
the 4K difference was 4.4–4.6 ms in that run.
Native cores,
light preparation,
color memory,
star baking,
halo sampling and composition still ran.
This measures the marginal saving from freezing,
not an exclusive pass budget or an achievable no-change optimization.
The other costs have not been independently separated.
Fringe zero gave no repeatable saving:
changes ranged from about 1.1% faster to 0.4% slower.

Image blur was initially a plausible way to replace repeated analytic shape evaluation.
One shared blur could be simpler,
but would mix colors and depth layers;
per-depth blur could retain their ordering and give nearer depths wider glow.
Both would still need a star image to blur,
filtering passes and composition.
Tiny stars must contribute energy even when smaller than a low-resolution pixel.
The early expectation of savings was a hypothesis,
not a measured result.

We then [implemented and compared per-depth blur](evidence/spectrogram-stars/depth-blur/README.md).
It retained native cores and deposited each star into a low-resolution seed image with energy-preserving quads,
then applied horizontal and vertical Gaussian filters independently for each depth.
Calibration kept average brightness close to the analytic version.
The images were quite similar;
blur made the glow somewhat more uniform and the grain smoother,
while losing some individual halo-width variation and filling the center of the residual differently.
Real-take contact sheets and a motion comparison informed the discussion.

**That implementation cost 91–92% more total GPU time at 1080p and about 34% more at 4K.** It drew roughly 1.19 million seed stars in a default 16:9 field,
added filtering passes and scratch images,
and always ran the final vertical filter at half resolution.
A seed-draw bypass diagnostic implicated seed production as substantial work,
but was not an equivalent-picture alternative or an exclusive cost breakdown.
The report preserves sampling/edge limitations;
filtering entirely at each depth's logical resolution before one upsample was tested in the follow-up below.
It does not establish that every possible blur design is slower.

Yan chose **“Let's keep the halos.”** The blur code and experimental selector were removed;
the implementation patch,
raw timings,
images and limitations remain in the evidence folder.

## Follow-up: directly gathered seeds and smaller per-depth filters

The [second blur experiment](evidence/spectrogram-stars/gathered-blur/README.md) removes per-star seed quads and runs both filter directions at each depth's smaller logical resolution,
then upsamples once during native composition.
It gathers energy-preserving tent seeds directly from actual jittered centers,
including every star within the seed footprint.
The comparison uses **100% jitter against analytic halos at exactly one-third resolution**.

The first version cost **13–14% more total GPU time at 1080p and was roughly tied at 4K**,
with means about 1% slower.
Two attempts to trim nearly zero-weight filter taps also failed to beat the baseline;
the final specialized filters cost **16–19% more at 1080p and 3–5% more at 4K**.
Load varied between series,
so their absolute times do not isolate the effect of kernel tuning.
The report retains the paired controls and all three series.

An independent GPU-versus-CPU seed check passed through full-jitter motion,
including many tiny stars per output pixel and an atlas-row boundary.
Thus the measured implementation preserves the tested seeds' energy;
it does not obtain speed by dropping them.
Real-take images remain close,
with somewhat smoother grain and more uniform glow.
Shared per-depth kernels still approximate the analytic halo shapes.

**The experiment was reverted.** Removing tiny seed triangles and reducing vertical-filter work was insufficient to make this separate seed-plus-two-filter design cheaper than the one-third analytic baseline.
The scratch implementation also needs bounded atlas padding and cached border planning before any future adoption.
The patch,
raw samples,
images,
verification and limitations are preserved in the report.
Fused seed/filter generation and shared-data analytic compute remain unmeasured ideas.

## Reducing halo work without reducing jitter or reach

The selected option is to reduce halo image resolution while leaving all five depths,
nine halo neighbors and full reach intact.
For linear scale `s`,
the approximate per-depth candidate count is `1 + 9s²`.
Actual dimensions round up independently on each axis.

| Halo resolution | Halo pixels relative to native | Candidates per native pixel per depth | Five RGBA16F halo layers at 4K |
| --- | ---: | ---: | ---: |
| 100% | 100% | 10 | 316.4 MiB |
| 50%, default | 25% | 3.25 | 79.1 MiB |
| One third | About 11.1% | 2 | About 35.2 MiB |
| 25% | 6.25% | 1.5625 | 19.8 MiB |

The native two-layer split image remains about 63.3 MiB at 4K;
other resources are additional.
The table is allocation/evaluation arithmetic,
not proportional total frame-time savings.
Lower resolution can soften or undersample fine halo detail,
especially on small panes;
it does not move stars or reduce their intended outer radius.

[Repeated paired measurements](evidence/spectrogram-stars/halo-resolution/README.md) of **50% versus exactly one third** found **17.6–17.9% less total GPU time at 4K and 7.2–8.7% less at 1080p**.
One third evaluates about 56% fewer halo pixels than 50%.
The UI rounds its displayed percentage;
a displayed 33% need not be exactly the measured `1 / 3` value.
The 25% endpoint has not been separately benchmarked.
The default stays at 50% to preserve the current sampled look.

Other ideas remain unmeasured:
use four halo neighbors with shorter support,
use different resolutions for different analytic-halo depths,
try different native pass partitions,
or combine color-memory preparation and star baking.
Four halo neighbors at half resolution would nominally give two candidates per depth,
but full jitter would again force a 0.7-cell reach rather than 1.2.
It does not meet the same visual constraint as lowering resolution.
Adaptive neighbor/radius paths or caching moving halos would add complexity and need evidence before adoption.

## How to read the measurements and try the result

Current paired tests use M1 Pro / Metal,
source-compiled shaders,
a synthetic full-pane 1024-slab / 3828-bucket light field,
a ten-second history and two device pixels per point.
Controls and candidates are interleaved with repeated order permutations;
identical A/A controls show noise.
They are offscreen GPU intervals,
not Bitwig frame times or guaranteed FPS improvements on other devices.
Compare each candidate with its controls,
not absolute milliseconds across separately loaded runs.
Do not sum overlapping Metal pass timestamps into an exclusive budget.
The older evidence documents where its fixtures or timing brackets differ.

The current draft build can be loaded from the repository with:

```sh
./load-plugin.sh codex/stars-four-neighbor
```

Then deactivate and reactivate Bitwig's audio engine.
The handoff names the verified overlay tag;
System → Performance → Performance overlay lets it be checked in the plugin.
Try Halo resolution around one third against the 50% default while leaving the other controls fixed,
and inspect bright band edges,
fine dust and motion.
The plugin and offline renderer are built together so exports use the same construction.
