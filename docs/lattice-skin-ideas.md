# Lattice skin ideas

Brainstorm springboard for alternative lattice looks (BACKLOG `[lattice]`
"brainstorm some more skins"). **Ideas only — nothing here is implemented
or scheduled.** Pick whatever's appealing and it can be built later.

Two places a skin reaches, and only one of them is an axis today:

- **Node body** — a node is its RING STACK and nothing else: an empty middle
  the stack is read out from (`ring_inner`), the audio ring seated on that, the
  octave band around it, the melody/bass marks past that, each sized on the
  Layers bar. What lights the middle of one is the node glow
  (`lattice.wgsl`'s `glow_layer`), colored by the node's own INK — each layer
  it draws weighted by the radial width it occupies, laid out by angle in the
  ink strip (`fs_ink_strip`, read back by `glow_ink`). There is no style enum to add a
  variant to — a second paint means a branch there and a setting to pick it,
  which is what the retired `NodeStyle` was.
- **Chrome** — grid lines and chord beams live in `fs_edge` /
  `derive_grid` / `derive_edges`. The `Skin` struct
  (`harmonigraph-scene::skin`) owns every color the CHROME draws — panel,
  well, hairline, accent — but not what the LATTICE draws at rest. The grid
  lines, the audio ring where it reads silence, the octave band's unsounding
  slices and an unplayed node all stand on one view setting instead,
  `ViewConfig::lattice_ground`: a neutral `L*` resolved per frame off the
  Ground bar, so it is dialable while a picture is being read. The skin's
  part in it is `surface_faint`, the rung a fresh view opens on.

A node is a camera-facing billboard with signed-distance masks, so new node
shapes are cheap: change the coverage math, keep the compositing.

## Node bodies

- **Solid fill** — a matte filled disc at the node's centre, no glow at all.
  The literal "filled circle instead of empty"; calmest possible look, pitch
  color at full.
- **Outline-only (rings for everything)** — every note a hollow colored
  ring, stroke weight tracking activation. A clean "wireframe lattice", and
  a view setting rather than anything a note carries in with it.
- **Filled + hard rim** — filled interior with a bright crisp edge (the
  inverse of the outward glow); reads as a coin/token rather than a star.
- **Concentric rings** — 1–3 nested rings; ring count or spacing encodes
  octave or velocity without needing the separate octave glyphs.
- **Halftone fill** — disc filled with a dot screen whose density = level
  (print/comic feel); pairs well with a light skin.
- **Calm orb** — a smooth radial-gradient sphere with limb darkening, for a
  3-D read the flat disc has no way to give.
- **Polygon nodes** — hex/triangle fills instead of circles, leaning into
  the lattice-as-tiling geometry (hex especially suits the triangular
  just-intonation grid).

## Idle placeholder (currently a faint grey ring on the home sheet)

- **Filled dot** — a small solid dot at each position instead of a ring;
  denser, more graph-like, and sidesteps the ring/disc shape mismatch that
  the fade fix works around.
- **Cross tick** — a tiny `+` at each node (blueprint look).
- **Grid-only** — drop the placeholder entirely and let the grid gaps mark
  positions (relies on a slightly stronger grid).

## Lines (grid + chord beams)

- **Solid hairlines** — thin continuous grid lines with a faint glow,
  instead of the current soft band; sharper, more technical.
- **Dotted grid** — render each grid segment as a row of dots rather than a
  line; lighter, less cage-like.
- **Comet beams** — chord beams brighter at the root end, fading toward the
  other notes, so chord direction reads.
- **Weight by consonance** — simpler intervals get thicker/brighter beams
  (octave/fifth heavy, high-limb intervals thin).
- **Flow pulse** — a bright pulse travelling along a chord edge from the
  root, looping while the chord is held.
- **Accent-tinted grid** — a desaturated hue over the field instead of the
  neutral grey. Still an idea, but no longer a skin swap: the grid's color
  is `ViewConfig::lattice_ground`, an `L*` with no hue axis at all, so it
  means giving that setting a chroma and a hue of its own. It also reaches
  further than the grid — the audio ring's silent end and the band's
  unsounding slices stand on that same number, and
  `the_grid_the_ring_and_an_idle_node_are_one_grey` says they must — so what
  is really on offer is "tint the lattice AT REST", all three surfaces
  together, rather than the lines alone.

## Whole-look skins (coordinated palettes)

These mostly need a second `Skin` value (the color struct already
centralizes the chrome) plus a `lattice_ground` to match it — the field is
what the resting lattice is drawn in, and a light skin under a dark ground
is two looks at once — plus a matching node/line choice:

- **Blueprint** — deep blue field, cyan hairlines, hollow rings, mono
  labels.
- **Paper / ink** — light background, dark ink strokes, solid black dots
  (needs a light skin variant).
- **Neon** — black field, saturated glowing filled circles, bright beams.
- **Minimal mono** — greyscale, flat filled discs, thin solid lines, no
  glow.

## Cheapest first steps, if any of this gets picked up

1. **Solid fill** node body — a two-line branch in the node's ink and a
   setting to reach it; immediately gives the "filled circles" the backlog
   asked about.
2. **Filled-dot idle placeholder** — swap the placeholder ring mask for a
   small disc; tiny change, and it removes the disc/ring shape mismatch at
   the root of the fade issues.
3. **Dotted or solid grid** — a variant flag in `fs_edge`, mirroring the
   existing dashed-links path.
