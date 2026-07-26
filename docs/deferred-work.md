# Deferred work

Items that were evaluated and consciously parked — not abandoned. Each entry
carries enough context (state, the actual work, the catch, and a value/effort
read) to pick it up cold later. Neither blocks anything today.

## Depth-buffer sorting

**State.** The lattice renders through an offscreen color + `Depth32Float`
pass (`crates/harmonigraph-render/src/lib.rs`). Depth is *written*, but the node
pipeline is created with `depth_compare: Always` (~`lib.rs:575`), so the
buffer is never used to reject fragments. Occlusion is still done the old
way: nodes are sorted **back-to-front on the CPU** and painted in that order
(painter's algorithm, ~`lib.rs:276`). The depth attachment exists purely as
infrastructure — the header comment flags it as "written but not yet read."

**The work.** Switch `depth_compare` to a real test (`Less` / `LessEqual`)
so overlapping nodes resolve per-pixel by true depth instead of draw order.

**The catch — why it isn't a one-line flag flip.** The nodes aren't opaque
spheres. They're soft, semi-transparent discs with glows; the field node
styles (Vortex / Spiral / Checker) and the envelope fades are
translucent. Depth testing + alpha blending is order-dependent:

- if transparent fragments *write* depth, a faint glow halo starts occluding
  nodes behind it → visible haloes / hard edges;
- if they *don't*, you still need the back-to-front sort for correct blending.

The correct approach is a **two-pass split**: opaque cores with depth
write + test (any order), then the transparent layers (glows, outer octave
glyphs, fades) drawn back-to-front with depth **read-only** (test but no
write). That's a real pipeline + shader change, not a toggle.

**Value / effort.** Medium-high effort (pipeline + `lattice.wgsl` + careful
visual verification in Bitwig). Payoff is *situational*: the CPU sort already
handles separated billboards well; per-pixel depth mainly helps when
billboards actually intersect or crowd at steep camera angles, and the
glow-based aesthetic doesn't obviously benefit. **Do it only if a specific
overlap artifact shows up in practice** — otherwise the infrastructure can
keep sitting there unused at zero cost.

## Render-style final trim

**State.** Most of the "prune the experiments" work is already done — see the
enum comments in `crates/harmonigraph-scene/src/lib.rs`:

- **`NodeStyle`**: trimmed from a 15-style set down to **4** — Steady,
  Vortex, Checker, Spiral. (Breathe, Sparks, Wire, Corona, Plasma, Aurora,
  Marble, Lava, Filament, Stripes, Tiles and Pinwheel were removed, kept only
  as `serde` aliases onto Steady so old projects still load.)
- **`OuterStyle`**: gone. The glyph shapes settled on slices and the layer
  became unconditional, so the enum and its persisted key were dropped; an
  old blob's `outer_style` is now an ignored unknown field.
- **`CoreStyle`**: replaced entirely by a `core_radius` + `core_solidity`
  slider pair; the enum is now legacy load-only (`migrate_legacy` folds old
  tokens into radius/solidity).

**The work.** A final aesthetic pass on the 4 surviving node styles: decide
whether Vortex, Checker and Spiral all earn their keep. Each cut
removes one shader branch (`lattice.wgsl`, indexed by
`NodeStyle::shader_index`) and one enum variant, keeping a `serde` alias onto
the survivor so persisted views still load (the established pattern).

**Value / effort.** Low effort — removal is mechanical. The gate is a human
aesthetic judgment on the *live, rendered* styles, which can't be automated.
The natural first step is an inventory of what each surviving style does (from
the shader) and where they visually overlap, then a pick of which to cut.

## Not deferred — closed

- **The piano roll's geometry** (was two entries here: baking settled notes
  into cached meshes, then superseding that with a wgpu callback). **Built.**
  The roll now draws as one instanced quad per note segment through
  `roll_paint_callback`, with a rounded-box SDF painting the core, the black
  outline and the white keyline as bands of one distance — the reasoning, the
  measurement it came from, and why the instance buffer is still rewritten
  every frame all live in `crates/harmonigraph-render/src/roll.rs`'s module doc,
  which is where they belong now that there is code to read them against.
  Mesh baking is dead rather than deferred: it addressed the 0.5 ms of
  tessellation and none of the 4-5 ms of upload, and with the notes off
  egui's vertex buffer entirely there is nothing left for it to cache. The
  `roll` row under `verts` in the performance overlay reports what the roll
  now costs (note count), since its geometry no longer passes through egui's
  vertex count.
- **Surface-format assumption** (`ASSUMED_SURFACE_FORMAT = Bgra8Unorm` in
  `harmonigraph-plugin/src/editor.rs`): the only clean fix needs `RenderState`
  access that lives upstream in egui-baseview, and upstreaming is off the
  table, so this stays as-is. The constant is the knob if a mismatch ever
  panics on an exotic host.
- **Alternate skins / live re-skinning**: parked by choice.
