# Deferred work

Items that were evaluated and consciously parked —
not abandoned.
Each entry carries enough context (state, the actual work, the catch, and a value/effort read) to pick it up cold later.
Nothing here blocks anything today.

## Depth-buffer sorting

**State.** The lattice renders through an offscreen color + `Depth32Float` pass (`crates/harmonigraph-render/src/lib.rs`).
Depth is *written*, but the node pipeline is created with `depth_compare: Always` (~`lib.rs:575`), so the buffer is never used to reject fragments.
Occlusion is still done the old way:
nodes are sorted **back-to-front on the CPU** and painted in that order (painter's algorithm, ~`lib.rs:276`).
The depth attachment exists purely as infrastructure —
the header comment flags it as "written but not yet read."

**The work.** Switch `depth_compare` to a real test (`Less` / `LessEqual`) so overlapping nodes resolve per-pixel by true depth instead of draw order.

**The catch —
why it isn't a one-line flag flip.** The nodes aren't opaque spheres.
They're soft, semi-transparent discs with glows, and both the glow skirt and the envelope fades are translucent.
Depth testing + alpha blending is order-dependent:

- if transparent fragments *write* depth, a faint glow halo starts occluding
nodes behind it → visible haloes / hard edges;
- if they *don't*, you still need the back-to-front sort for correct blending.

The correct approach is a **two-pass split**:
opaque cores with depth write + test (any order), then the transparent layers (glows, outer octave glyphs, fades) drawn back-to-front with depth **read-only** (test but no write).
That's a real pipeline + shader change, not a toggle.

**Value / effort.** Medium-high effort (pipeline + `lattice.wgsl` + careful visual verification in Bitwig).
Payoff is *situational*:
the CPU sort already handles separated billboards well;
per-pixel depth mainly helps when billboards actually intersect or crowd at steep camera angles, and the glow-based aesthetic doesn't obviously benefit.
**Do it only if a specific overlap artifact shows up in practice** —
otherwise the infrastructure can keep sitting there unused at zero cost.

## Not deferred — closed

- **Render-style final trim.** **Done.** The aesthetic pass the entry asked
for was made, and none of the animated paints was kept:
Vortex, Checker and Spiral are gone along with the field machinery behind them (the noise, the sphere mapping, the per-node seed and the swirl gradient).
`NodeStyle` went with them rather than surviving as a one-variant enum, the way `OuterStyle` and `CoreStyle` did before it:
a blob's `node_style` is now an ignored unknown field, held by `a_persist_blob_naming_a_retired_node_style_still_loads`.
The disc they painted is gone too:
a node is its ring stack read out from an empty middle (`ring_inner`), and what lights that middle is the node glow, in the colours the node itself draws —
every layer contributing in proportion to the radial width it takes up, through the ink strip.
- **The piano roll's geometry** (was two entries here: baking settled notes
into cached meshes, then superseding that with a wgpu callback).
**Built.** The roll now draws as one instanced quad per note segment through `roll_paint_callback`, with a square-cornered box SDF painting the note's body and the two rim bands beside it as bands of one distance —
the reasoning, the measurement it came from, and why the instance buffer is still rewritten every frame all live in `crates/harmonigraph-render/src/roll.rs`'s module doc, which is where they belong now that there is code to read them against.
Mesh baking is dead rather than deferred:
it addressed the 0.5 ms of tessellation and none of the 4-5 ms of upload, and with the notes off egui's vertex buffer entirely there is nothing left for it to cache.
The `roll` row under `verts` in the performance overlay reports what the roll now costs (note count), since its geometry no longer passes through egui's vertex count.
- **Surface-format assumption** (`ASSUMED_SURFACE_FORMAT = Bgra8Unorm` in
`harmonigraph-plugin/src/editor.rs`):
the only clean fix needs `RenderState` access that lives upstream in egui-baseview, and upstreaming is off the table, so this stays as-is.
The constant is the knob if a mismatch ever panics on an exotic host.
- **Alternate skins / live re-skinning**: parked by choice.
