# Deferred work

Items that were evaluated and consciously parked — not abandoned. Each entry
carries enough context (state, the actual work, the catch, and a value/effort
read) to pick it up cold later. Neither blocks anything today.

## Depth-buffer sorting

**State.** The lattice renders through an offscreen color + `Depth32Float`
pass (`crates/lattice-render/src/lib.rs`). Depth is *written*, but the node
pipeline is created with `depth_compare: Always` (~`lib.rs:575`), so the
buffer is never used to reject fragments. Occlusion is still done the old
way: nodes are sorted **back-to-front on the CPU** and painted in that order
(painter's algorithm, ~`lib.rs:276`). The depth attachment exists purely as
infrastructure — the header comment flags it as "written but not yet read."

**The work.** Switch `depth_compare` to a real test (`Less` / `LessEqual`)
so overlapping nodes resolve per-pixel by true depth instead of draw order.

**The catch — why it isn't a one-line flag flip.** The nodes aren't opaque
spheres. They're soft, semi-transparent discs with glows; the field node
styles (Vortex / Spiral / Checker / Pinwheel) and the envelope fades are
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
enum comments in `crates/lattice-scene/src/lib.rs`:

- **`NodeStyle`**: trimmed from a 15-style set down to **5** — Steady,
  Vortex, Checker, Spiral, Pinwheel. (Breathe, Sparks, Wire, Corona, Plasma,
  Aurora, Marble, Lava, Filament, Stripes, Tiles were removed, kept only as
  `serde` aliases onto Steady so old projects still load.)
- **`OuterStyle`**: down to **4** — Off, Dots, Slices, Rings (Petals / Flares
  removed, Bumps merged into Dots).
- **`CoreStyle`**: replaced entirely by a `core_radius` + `core_solidity`
  slider pair; the enum is now legacy load-only (`migrate_legacy` folds old
  tokens into radius/solidity).

**The work.** A final aesthetic pass on the 5 surviving node styles: decide
whether Vortex, Checker, Spiral, and Pinwheel all earn their keep. Each cut
removes one shader branch (`lattice.wgsl`, indexed by
`NodeStyle::shader_index`) and one enum variant, keeping a `serde` alias onto
the survivor so persisted views still load (the established pattern).

**Value / effort.** Low effort — removal is mechanical. The gate is a human
aesthetic judgment on the *live, rendered* styles, which can't be automated.
The natural first step is an inventory of what each surviving style does (from
the shader) and where they visually overlap, then a pick of which to cut.

## Not deferred — closed

- **Surface-format assumption** (`ASSUMED_SURFACE_FORMAT = Bgra8Unorm` in
  `midi_lattice_3d/src/editor.rs`): the only clean fix needs `RenderState`
  access that lives upstream in egui-baseview, and upstreaming is off the
  table, so this stays as-is. The constant is the knob if a mismatch ever
  panics on an exotic host.
- **Alternate skins / live re-skinning**: parked by choice.

## Baking settled piano-roll notes into cached meshes (SUPERSEDED — see below)

**State.** `draw_roll` (`crates/lattice-ui/src/panes/roll.rs`) re-derives every
visible note from scratch each frame. A note is drawn as up to **five stroked,
anti-aliased rounded rects** — two bloom bands (gated on
`view.bloom_strength`), a keyline pair, and the core (~`roll.rs:210-273`) —
and `alpha` is pinned at `1.0` by design (`roll.rs:175`), so a note eleven
seconds old costs exactly what a fresh one does. Cost is therefore linear in
notes-on-screen, which is `roll_seconds` wide (12 s default).

Measured with the overlay's `tess` row: ~4 ms of tessellation while spamming
notes, against a 6.94 ms refresh at 144 Hz, with `ui cpu` at ~2 ms and lattice
GPU at ~0.2 ms. Tessellation is the dominant frame cost, and it scales
one-for-one with roll note count (confirmed by sweeping the Span control) and
jumps again with Bloom on.

**The work.** The roll is a scrolling timeline of immutable content — the same
structure the spectrogram heatmap has, and the same fix applies (see
`SpectrogramRing`, `panes/spectrogram.rs`). Tessellate settled notes ONCE into
`Mesh`es chunked by absolute time, then per frame replay the chunks and
translate them. Held notes and anything near the window edge keep drawing live.

Two load-bearing assumptions, both verified:

- `Shape::Mesh(Arc<Mesh>)` — replaying a baked chunk is a refcount bump, not a
  copy of its vertices.
- `depth_of(t) = split + frac(t) * depth_span` is LINEAR in time
  (`panes/spectral.rs:614`), so scrolling a baked chunk is an exact
  screen-space translation along the depth axis, not an approximation that
  drifts. The translation vector is `axes.at(0.0, dd) - axes.at(0.0, 0.0)`,
  independent of pitch and depth because `Axes::at` is affine.

Chunk keys from absolute time (`floor(note.start / CHUNK)`), exactly as the
spectrogram keys slabs, so the offline renderer stays deterministic.

**The catch.** Two, both found by reading rather than by building:

1. **Notes are truncated at the far edge.** `let (t0, t1) = (t0.max(oldest),
   t1.max(oldest));` (`roll.rs:164`) rewrites a crossing note's geometry every
   frame while it exits, so it is NOT immutable there. A chunk must be retired
   before its notes reach that edge and its notes handed back to live drawing.
   This is the same trap that shipped as visible jitter in the spectrogram
   ring's first cut, where the sampler's ClampToEdge was quietly doing work the
   ring couldn't. Budget real care for the edge, and verify by watching notes
   leave the far end.
2. **`draw_roll` takes `&SharedState`, not `&mut`.** A cache needs mutable
   access, which ripples through `draw_pane` and into the offline renderer's
   path — mechanical, but it touches determinism-tested code.

Also: held notes' final segment ends at `now` (`note.segments(now)`), so the
live/baked split is by RELEASE time, not merely by window position.

**Open question to settle first.** Whether `egui::Context::tessellate` is safe
to call mid-frame, or whether to drive `epaint::Tessellator` directly. The roll
draws no text, so either way there is no font-atlas plumbing to arrange.

**Measured afterwards — read this before building it.** Tessellation is NOT
the binding cost. With the frame broken down (`tess`, `buf up`, `verts` in the
performance overlay) the roll turned out to cost ~0.5 ms of tessellation and
**4-5 ms of vertex upload**: 20k vertices idle, 100k+ with notes on screen,
arriving in only ~20 primitives — so the batching is fine and the volume is
the problem. egui re-uploads every vertex every frame, immediate-mode, and
`Shape::Mesh` is no exception.

**Baking into cached meshes therefore saves the 0.5 ms and none of the 4-5.**
It is the right idea in the wrong place. What would actually pay is owning the
buffers: draw the roll through a wgpu paint callback like the lattice does,
with persistent vertex buffers updated incrementally, so scrolled content is
uploaded once rather than every frame. The baking design below is the geometry
half of that job and is worth keeping for it — the chunking, the absolute-time
keys, the far-edge trap are all still correct — but on its own it fixes the
smaller number.

Cheaper first steps that attack the volume directly: skip the two bloom bands
on ribbons too small to show them (a note is 5 stroked rounded rects with
bloom on, 3 without, 1 with the keyline off too), and note that
`roll_rounding` at 0 removes the corner arcs, which are most of a rect's
vertices.

**Value / effort.** Medium-high effort, comparable to the spectrogram ring.
Payoff as originally scoped is now known to be small: roll tessellation
becomes O(notes arriving) instead of O(notes visible), `roll_seconds` stops being a
performance setting for tessellation — but not for upload, which is the cost
that matters. Strictly better than the geometric-LOD
alternative (skip bands on tiny ribbons), which trades a little appearance for
a bounded cost and would be subsumed by this. **There is a 5x lever available
without any code in the meantime:** Bloom off takes a note from five stroked
rects to three, Keyline off takes it to one.

## Drawing the piano roll through a wgpu callback

**Supersedes the mesh-baking entry above.** Same problem, and measurement moved
the answer.

**State.** The roll is the frame's dominant cost, and not where it looked. With
the overlay's breakdown on (Panel > Frame breakdown): tessellation ~0.5 ms,
vertex UPLOAD 4-5 ms, `verts` running 20k idle to 100k+ with notes on screen,
in only ~20 primitives. Batching is fine; the volume is the problem. egui is
immediate-mode and re-uploads every vertex every frame, so geometry that merely
scrolls is re-sent 144 times a second.

A note is currently three stroked, anti-aliased rounded rects (keyline pair
plus core — the two bloom bands are gone). Each is ~100-200 vertices once
corners and the AA ring are subdivided, so a note costs several hundred
vertices and a busy roll costs six figures.

**The work.** Draw the roll the way the lattice is drawn: a wgpu paint callback
with its own pipeline and a persistent instance buffer. The win is not caching
the tessellation — it is not tessellating at all.

**One instanced quad per note**, with a rounded-rect signed distance field in
the fragment shader. Core, black border and white keyline all fall out of the
distance as bands, so they cost nothing extra — no second and third shape, no
corner subdivision, no AA ring. Four vertices per note against several hundred:
a hundred thousand vertices becomes a few hundred, and the upload stops
mattering rather than getting cheaper.

Because the geometry is per-note and static once released, the buffer is
append-and-evict: new notes are written at the head, notes scrolling past the
window are dropped from the tail, and `now` moves through a uniform rather than
through the vertices. That is where the superseded entry's thinking still
applies — absolute-time keys, the far-edge truncation trap — but on a ring of
INSTANCES, which is far simpler than a ring of meshes.

Draw directly in `CallbackTrait::paint`, which is handed egui's render pass, so
the roll lands in egui's own draw order between the spectrogram beneath it and
the labels above. No offscreen target and no compositing — the lattice needs
those for its bloom chain; the roll needs neither.

**The catch.**

1. **Bent notes are not rects.** A glide draws as a quad following the pitch
   ramp (`roll.rs`, the `p0 != p1` arm). A rounded-rect SDF does not cover it.
   Either extend to a parallelogram SDF, or keep the rare bent case on the egui
   path — it is a small fraction of notes and correctness beats uniformity.
2. **`Axes` orientation.** The pane rotates and flips; the pipeline needs the
   same (pitch, depth) -> screen affine the egui path gets from `Axes::at`,
   passed as a uniform rather than baked into vertices.
3. **Clipping.** egui clips by scissor rect per primitive; a callback must
   respect the pane's clip rect itself.
4. **Offline determinism.** `lattice-offline` renders this pane, and the
   determinism test compares frames byte for byte. SDF coverage must be a pure
   function of the uniforms — no wall-clock, no frame counter.

**Value / effort.** High effort, high payoff, and it is the ONLY thing that
addresses the measured cost: it removes ~4-5 ms of per-frame upload rather
than the 0.5 ms of tessellation the earlier design targeted. Everything else
available is a constant factor on the same geometry (Keyline off takes a note
from three stroked rects to one; `roll_rounding` at 0 removes the corner arcs;
a shorter Span holds fewer notes).
