# Lattice skin ideas

Brainstorm springboard for alternative lattice looks (BACKLOG `[lattice]`
"brainstorm some more skins"). **Ideas only — nothing here is implemented
or scheduled.** Pick whatever's appealing and it can be built later.

Two independent axes today:

- **Node body** — `NodeStyle` enum (`harmonigraph-scene`) → shader index →
  a branch in `lattice.wgsl` `fs_main`. Adding a look = enum variant +
  shader branch. Current set: Steady + three field styles
  (Vortex/Checker/Spiral).
- **Chrome** — grid lines and chord beams live in `fs_edge` /
  `derive_grid` / `derive_edges`; all colors (node idle, grid line, accent)
  live in one `Skin` struct (`harmonigraph-scene::skin`).

The disc is a camera-facing billboard with a signed-distance mask, so
new node shapes are cheap: change the coverage math, keep the compositing.

## Node bodies

- **Solid fill** — a matte filled disc, no glow, no gas. The literal
  "filled circle instead of empty"; calmest possible look, pitch/channel
  color at full. Good default candidate for people who find the field
  styles busy.
- **Outline-only (rings for everything)** — what channel-14 already does,
  promoted to a style: every note a hollow colored ring, stroke weight
  tracking activation. A clean "wireframe lattice".
- **Filled + hard rim** — filled interior with a bright crisp edge (the
  inverse of the outward glow); reads as a coin/token rather than a star.
- **Concentric rings** — 1–3 nested rings; ring count or spacing encodes
  octave or velocity without needing the separate octave glyphs.
- **Halftone fill** — disc filled with a dot screen whose density = level
  (print/comic feel); pairs well with a light skin.
- **Calm orb** — a smooth radial-gradient sphere with limb darkening but
  *without* the turbulent field, for a quieter 3-D read than the gas
  styles.
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
- **Accent-tinted grid** — grid in a desaturated skin accent instead of the
  neutral grey, so the whole field has a hue.

## Whole-look skins (coordinated palettes)

These mostly need a second `Skin` value (the color struct already
centralizes this) plus a matching node/line choice:

- **Blueprint** — deep blue field, cyan hairlines, hollow rings, mono
  labels.
- **Paper / ink** — light background, dark ink strokes, solid black dots
  (needs a light skin variant).
- **Neon** — black field, saturated glowing filled circles, bright beams.
- **Minimal mono** — greyscale, flat filled discs, thin solid lines, no
  glow.

## Cheapest first steps, if any of this gets picked up

1. **Solid fill** node style — one enum variant + a two-line shader branch;
   immediately gives the "filled circles" the backlog asked about.
2. **Filled-dot idle placeholder** — swap the placeholder ring mask for a
   small disc; tiny change, and it removes the disc/ring shape mismatch at
   the root of the fade issues.
3. **Dotted or solid grid** — a variant flag in `fs_edge`, mirroring the
   existing dashed-links path.
