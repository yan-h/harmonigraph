# R4 — shadow and glow support, sizing, and fixture reach

Investigator: render; source baseline ad1c6e6b.
Status: bounded source/history/fixture inspection complete; no new implementation recommendation.
No builds, benchmarks, image mutation trials, or host interactions were performed for this leaf.
The coordinator's current test results must be attached separately; an inspected assertion is not a fresh passing result.
Paths below are relative to the repository root.

## Current parameter and sampling contracts

Lattice shadow width is a share of the home node radius in pane points.
`crates/harmonigraph-render/src/lattice_frame.rs:249–268` derives that radius using the camera's points-per-world scale,
computes one sigma per shadow group,
and includes Gaussian spread in the visible support.
The home radius is intentional: nodes on smaller/deeper sheets retain a common width in the picture.
`shadow.rs:91` converts the width to sigma as half the resolved point width.
The atlas packer applies device pixels-per-point/render scale once,
limits Gaussian sigma to three cell texels,
and pads the cell by the kernel support plus one sampling texel and source spread (`shadow.rs:400–484`).
Distance cells have a separate sampling-density floor;
direct analytic distance casters need no cell.

The Gaussian shader uses support 3 sigma and maximum radius nine,
so its separable pass visits at most nineteen taps per axis (`shaders/shadow.wgsl:95–164`).
It subtracts the Gaussian's value at the support edge before normalization,
making the compact kernel reach zero rather than terminate at a nonzero skirt.
Out-of-cell taps contribute zero without reducing the denominator;
one cell therefore cannot normalize a neighbor's content into its own blur.
The RG cell format has two actual consumers:
blurred coverage and the source-opacity ceiling needed when lattice slices fade (#1429).
Removing the second channel would restore a recently fixed behavior rather than simplify unused state.

Spectral width has a different unit contract: a fixed four-point width unit,
multiplied by the composed preview's point scale after clamping the saved style (`shadow.rs:50–71`).
`roll.rs:338–346,995` and `text.rs:1458` consume that same conversion.
The shared reach helper includes the selected kernel's support and Gaussian spread,
and returns zero for a style that cannot cast.
Clamping the already scaled preview width back to the saved dial maximum would break enlarged previews (#1420).
Lattice and Analyzer transfer curves deliberately differ:
the recorded owner decision retains Analyzer shadows in stops and lattice shadows linear (`docs/intent/lattice-shadows-and-idle.md`).
Unifying their transfer function is not an authorized simplification.

There is still a deliberate conservative lattice quad margin:
`shaders/lattice.wgsl:238–277` floors the quad's reach to three sigma even for Distance's two-sigma profile.
Sigma is fixed in focus-plane points while node UV expansion also depends on sheet/projection scale.
The source records six lattice goldens moving by up to 7/255 when that margin was removed (#1310).
This historical result is a reason to preserve the margin,
not a current measured cost or proof of minimal padding at every camera angle.

Glow has a distinct spatial contract.
`lattice_frame.rs:505` supplies view-level reach/strength/curve;
`shaders/lattice.wgsl:3341` evaluates a finite circular domain using a fixed configured rim plus Reach.
Marks affect directional color but no longer create a per-node halo-size envelope.
Color becomes directional at the ordinary ring rim independently of the larger halo domain.
`lib.rs:1977–2025` sizes the glow target and three statistics textures at ceil(scene dimension / 2),
and `lattice_node_glow.rs:64–129,186–236` rasterizes node quads into blended statistics before resolving the field.
The angular strip remains the owner of directional color/history;
the resolve reconstructs overlap rather than reintroducing per-pixel spatial candidate lists.
Ordinary materials read the filtered half-resolution field;
Stars can expose their native-resolution output through `GlowTarget::binding`.

## Fixtures that reach the claimed cases

| Behavior | Evidence and actual reach | Boundary of the evidence |
|---|---|---|
| CPU/shader kernel support agreement | `shadow.rs:1356` checks shader support against `REACH_SIGMAS` and radius against the packer's cap; the following pedestal test checks its exponential endpoint. `a_cells_sigma_is_at_most_three_texels_at_every_shadow_width` sweeps 0.05–5000 pixels and checks padding. | Structural/numeric contracts; not visual parity across all devices. |
| No adjacent-cell bleed and normalized blur | `shadow.rs:1574`, `a_cells_blur_stays_inside_its_own_cell_and_keeps_its_mass`, packs two touching cells on one shelf at sigma 3; asserts the arrangement and >100 source texels, then checks the empty neighbor, mass, and half-plane response after production GPU blur. | Exercises the coverage channel at maximum Gaussian radius; it does not alone test the newer opacity-ceiling channel. |
| Complete support at the widest lattice dial | `lattice_tests/shadows.rs:1232`, `the_grown_quad_holds_the_whole_blur_at_the_top_of_the_shadow_bar`, tests node/cross × Gaussian/Distance with the camera pulled back to retain tails. It requires visible attenuation, reach exceeding 65% of mathematical support, monotonic samples, and a final step no greater than one output code. | A horizontal ray on these fixtures, not every view angle/corner. |
| Huge projected geometry and Distance floor | `shadows.rs:1366`, `a_node_close_to_the_eye_packs_a_cell_the_atlas_can_hold`, first asserts a node projects >10 pane widths and that the wide reading crosses the Distance density floor; checks both kernels and two widths for present cells and bounded atlas dimensions. | CPU packing test of this deliberately extreme projection; not a GPU stress timing. |
| Different depths retain point-width shadows | `shadows.rs:868`, `a_distance_markers_shadow_width_is_screen_constant_under_perspective`, places crosses at z=4 and z=-8, requires each tail >3 pixels, and allows <2 pixels difference past the projected ink tip. `compose.rs:51` separately asserts multiple sheet depths before testing order under Cabinet/Perspective/Orthographic. | Direct Distance marker width plus CPU painter order; not a universal multi-depth Gaussian image oracle. |
| Cross opacity, shared cell, and actual shape | `shadows.rs:735` measures a nonzero cross shadow and half/full scaling, requires three differently opaque crosses to share one cell, then requires measurable halo light and stable attenuation at two light levels. The fixture uses `standalone_marker` (`fixtures.rs:67`), whose anchor draws no node ink. Crucially, `golden.rs:96,601` carries the actual cross-shadow silhouette in `resting-markers-in-one-light`. | The quantitative attenuation tests alone would still not reject every disc-for-cross mutation. The golden is the shape oracle; its documented #450 mutation result was not rerun here. |
| Fading source and self-shadow | `shadows.rs:964` requires ordered nontrivial shadow loss over release levels 1, 0.6, 0.3, 0 and checks instance/caster disappearance. `shadows.rs:1028` checks both kernels at 0.3, 0.7, 1, requires >30 fully covered ring pixels, inside loss <=3 brightness codes and outside loss >20. | These current GPU fixtures reach fading ink; they do not quantify every Gaussian ceiling interaction between different casters. |
| Independent mixed shadow groups | `shadows.rs:1803`, `a_frame_whose_groups_disagree_draws_both_renderers`, tests both geometry/text assignments and requires each mixed frame to differ from uniform kernels. Exclusive-region masks exclude the union of both kernels' support and require >=200 probe pixels. | The specifically separated node/label fixture, not arbitrary overlap. |
| Enlarged preview support | `ui/panes/render.rs:1210` checks portrait/landscape, 1080/2160 short edges, both kernels and preview ratios 0.25–2; actual UI draw must emit separated pane callbacks and preserve saved styles. `render/roll.rs:1938` and `render/text.rs:2186` additionally sample production GPU pixels outside maximum unscaled support but inside scale-2 support. | The UI helper is not image parity; the GPU fixtures fill that gap for roll/text. Whole export/live equivalence still belongs to offline/native validation. |
| Glow extent and overlap rather than empty fixtures | `glow_reach.rs:19,75` checks reach and curve/endpoint behavior. `glow_overlap.rs:891` requires >500 pixels receiving both colored halos and an actual luminance lift, then tests reversed order within one byte. The dense-chord fixture at :929 requires the peak to be approached and a visible gamut-repair case. `glow_markers.rs:583` requires >300 ink pixels and >half lifted by a nontrivial wash. | Tests read the production half-resolution field through the production linear reconstruction (`glow_overlap.rs:42–81`), not just scene metadata. These are finite selected overlap scenes. |

GPU fixtures may return early when no headless device is available (`lattice_tests.rs:1–4`).
A green generic test invocation without adapter evidence is insufficient to claim these GPU paths ran.
The coordinator's Metal/headless run can establish execution;
this leaf does not substitute source inspection for it.

## History, maintenance tradeoffs, and conclusions

The history is specific rather than a generic claim that shaders are complicated:
#450 exposed attenuation tests unable to recognize the wrong silhouette;
the resting-marker golden now supplies that shape oracle.
#877 replaced spatial candidate gathering with rasterized overlap statistics;
#879 moved it to half resolution and cached angular weights;
#1404 made geometry pane-relative;
#1420 corrected clamp/scale order;
#1429 removed preview machinery while carrying fading source opacity through the blur.
`docs/directional-glow-simplification.md` records the accepted visual result and the older timings,
but those timings are historical hypotheses for this audit, not measurements of today's tree.

The best maintenance choice supported here is to retain the shared packer/conversion helpers and existing direct-distance bypass,
while preserving the small destination-specific geometry and transfer rules that have actual consumers.
There is no demonstrated user cost that pays for a new unified shadow/glow resource owner,
removing the conservative quad floor,
or replacing the accepted half-float overlap design.
Such changes add rebind/lifetime or composition contracts,
and would need a concrete visible/performance problem plus before/after evidence.
The deferred depth-buffer proposal likewise requires a reproduced overlap artifact (`docs/deferred-work.md:64–81`).

One known limit must remain explicit:
`glow_overlap.rs:1102` is an ignored diagnostic for order precision with 128/1024/4096 coincident faint halos,
linked to #878.
It deliberately reports drift rather than enforcing exact parity.
The ordinary one-byte image comparisons therefore do not establish a universal one-byte bound.
This is an existing documented limitation, not a new audit defect or a reason to reopen the accepted design.

Verdict: **leave current design; no new pursue item**.
The leaf's strongest clean conclusion is that relevant present fixtures have explicit non-vacuity guards and complementary shape/image checks;
it is not that all shadows/glows are proven correct or fast.
No separate benchmark campaign is justified by this inspection.
If the current release suite includes these tests on Metal, stop here.
If a future change touches support, scaling, or blend format,
run the existing `shadow::tests`, `lattice_tests::shadows`, `lattice_tests::glow_overlap`, `lattice_tests::golden`,
and `enlarged_preview_` filters through the repository lifecycle wrapper;
add a new fixture only for a newly demonstrated missing behavior.
