# Named lattice GPU transport

Implements [#646](https://github.com/yan-h/harmonigraph/issues/646), stacked on draft [#662](https://github.com/yan-h/harmonigraph/pull/662).
The immediate parent and the single Claude review base are `codex/lattice-gpu-batch` at `4c40d9ae2f1817fe5f5c1682182830559e029a57`, not `main`.
The independent CPU trail PR #661 is not part of this stack.

## Transport contract

`uniforms.rs` declares aligned GPU groups with named scalar settings and vector coordinates/colours.
Its local declaration macro emits the Rust structs and test metadata from those same field types and `offset_of!` values.
There is no separately maintained expected-field schema.
The metadata walker starts at the actual Naga uniform binding, then checks every nested field name and offset, scalar kind/width, vector and matrix dimensions, size and alignment, array count and stride, struct span and total binding size.
The alignment comparison uses uniform-address-space requirements (at least 16 bytes for structs and arrays), because Naga retains explicit `@align` in offsets/span while its Layouter reports the member types' natural maximum.
The shader's explicit group alignment matches Rust's `repr(C, align(16))`;
the Pod derive also rejects implicit Rust padding.

Lattice binding 0 starts with `CompositeParams`.
Blit binding 3 deliberately changes from a 128-byte view with bloom at byte 124 to that 16-byte group with bloom at byte 12. Both bound types are checked against the same Rust declaration, and the prefix's zero offset and bloom offset are asserted explicitly.
Roll/spiral bloom still uses the separate `AddUniforms` buffer at binding 4;
its shader declaration and transport remain unchanged.

The shader equations and public scene inputs are unchanged.
Unused clock/style/background lanes and the unread marker-cell sigma leave the upload.
Glow settings and row capacity still zero together when glow is disabled.
Marker world units and both shadow styles remain available without glow.
No saved-state shape, cache key, instance input, CPU row allocation or history ownership changes.

## Component coverage

Layout validation cannot detect assigning the wrong scene value to a correctly typed field.
The following maps every formerly anonymous live component to its executable consumer and existing picture coverage.
Test names below live under `crates/harmonigraph-render/src/lattice_tests/`.
No test is added per accessor.

| Former transport → named field | Consumer and behavior coverage |
|---|---|
| `view_proj`, `cam_right`, `cam_up` → `camera` | Node/marker vertex transforms and shimmer field axes; direct/offscreen comparison, all lattice goldens, projection/order and rectangular shadow fixtures |
| `misc.y` → `node.radius` | Billboard world size; layered-node shadow fixtures and zoomed/overlapping-sheet goldens |
| `misc2.xy` → `composite.darkest_pitch/brightest_pitch` | `pitch_lut_color`; octave hue and lattice goldens |
| `misc2.zw` → `composite.render_scale/bloom_strength` | `aa_width` and blit composite; existing scale-2 name golden plus `fractional_bloom_strength_is_independent_of_render_scale` |
| `misc3.yzw`, `misc4.y`, `misc5.zw` → `node.band_inner/band_outer/rings_outer/mark_inner/angular_gap/mark_thickness` | Node ink, rim and angular ink sampling; `a_mark_stands_off_the_outermost_ring_the_node_draws`, `a_mark_with_no_ring_under_it_reaches_the_nodes_centre`, distance-cell reference and octave seam fixtures |
| `misc5.xy`, `misc13.y` → `marker.half_width/taper_start/world_unit` | Marker ink, quad and shadow; `the_shader_spends_a_markers_proportions_on_the_shape_they_name`, taper/shadow fixtures and rectangular glow-off test |
| `misc6.w`, `misc8.xyzw` → `shimmer.pattern/slide/period/intensity/softness` | Pattern selector, travel and exposure; distinct-pattern, period, softness, speed/width and intensity tests in `shimmer.rs` |
| `misc7.xy`, `oct_bounds` → `octave.span/center/bounds` | Slice ownership/angles; `every_octave_in_the_range_is_drawn_and_they_close_the_ring`, `an_indicator_is_drawn_at_its_own_pitchs_angle` |
| `misc7.zw`, `misc9.xy` → `spectral.inner/outer/range_cents/folded` | Spectral annulus and sampling; `the_audio_ring_reads_the_spectrum_around_each_octave`, `the_folded_ring_reads_each_wedge_at_its_own_octave`, level-ramp fixture |
| `misc10.xyw`, `glow_curve.x`, `misc13.x` → `glow.reach/strength/blend/curve/wash` | Glow extent, amplitude, colour convolution, falloff and illumination; reach/curve/blend tests in `glow_reach.rs`, wash and partial-release tests in `glow_colour.rs` |
| `misc12.x` → `glow.row_capacity` | Ink strip row placement and allocation; per-node colour rows, culled-row reference, same-capacity carry and separate growth/reseed/reuse cases in the parent target fixture |
| `misc11.xyw`, `plus_shadow.xyw` → `geometry_shadow/marker_shadow.width/reach_sigmas/depth` | Caster quad extent and shadow multiplication; independent text/geometry styles, zero-width/depth gates, both kernels and maximum-reach fixtures |
| `misc14.xyzw` → `shadow_target.pane_points/atlas_texels` | Clip-to-point and cell-to-clip conversion; rectangular pane and actually unequal atlas dimensions with translated point-space shadow comparison |
| `plus_shadow_rect/cell`, `plus_shadow_terms.xzw` → `marker_cell.rect/cell/points_to_texels/aa_scale/arm_points` | Shared Gaussian marker raster and sampling; rectangular atlas fixture, taper and width/depth tests |
| `lattice_ground`, `pitch_lut`, `spectral_lut`, `spectrum_color` retain names | Silent ground, pitch/level ramps and packed analyzer bytes; silent-slice, colour identity, audio/fold/ramp tests and lattice goldens |

The fractional-bloom fixture uses strengths 0.25 and 0.75 at both scales 1 and 2, measuring unsaturated halo pixels outside the original ink.
The rectangular-shadow fixture uses panes 384×192 and 576×192 with off-centre node and Gaussian marker shadows, asserts the actual atlas has unequal axes, and compares shadow shape after the expected horizontal translation.
Both casters must contribute measurable darkening with glow disabled.
The existing parent tests retain painter order, label exclusion from bloom, bloom allocation toggles, same-capacity history carry and the distinct capacity-growth/reseed baseline.
The real hot-reload fixture rebuilds and draws both attachment variants;
the direct reference pipeline is exercised by the offscreen parity test.
Offline goldens cover the shared spectral picture;
they do not fill lattice component-coverage gaps.
Remaining limits:these fixtures do not exhaust every combination of camera, gradient bounds, shadow size and render scale.
Pitch-range bounds are covered through existing fixed scenes rather than a new independent bound sweep.
Padding has no observable behavior.
The layout contract and the named assignments reduce transport risk without claiming that static frames prove arbitrary semantics.

## Validation and handoff

Validation results and the single requested Claude review disposition are recorded here after execution.
Local Cargo operations use the coordinator's explicit machine lease, two build jobs and two test threads, this worktree's target and sccache.
No performance gain is claimed for this maintenance change;
no timing experiment or conditional GPU algorithm is added.
No golden is blessed, no shared DAW slot is swapped, and the PR remains draft and unmerged.