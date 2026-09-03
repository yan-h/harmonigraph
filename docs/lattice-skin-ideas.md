# Lattice skin ideas

Brainstorm springboard for alternative lattice looks (BACKLOG `[lattice]` "brainstorm some more skins").
**Ideas only —
nothing here is implemented or scheduled.** Pick whatever's appealing and it can be built later.

Two places a skin reaches, and only one of them is an axis today:

- **Node body** — a node is its RING STACK and nothing else: an empty middle
the stack is read out from (`ring_inner`), the audio ring seated on that, the octave band around it, the melody/bass marks past that, each sized on the Layers bar.
What lights the middle of one is the node glow (`lattice.wgsl`'s `glow_layer`), colored by the node's own INK —
each layer it draws weighted by the radial width it occupies, laid out by angle in the ink strip (`fs_ink_strip`, read back by `glow_ink`).
There is no style enum to add a variant to —
a second paint means a branch there and a setting to pick it, which is what the retired `NodeStyle` was.
- **Chrome** — the resting cross at each home-sheet position lives in
`fs_plus` / `derive_pluses`;
nothing is drawn between two positions.
The `Skin` struct (`harmonigraph-scene::skin`) owns every color the CHROME draws —
panel, well, hairline, accent —
but not what the LATTICE draws at rest.
The audio ring where it reads silence, the octave band's unsounding slices and an unplayed node all stand on one view setting instead, `ViewConfig::lattice_ground`:
a neutral `L*` resolved per frame off the Ground bar, so it is dialable while a picture is being read.
The markers stand on `ViewConfig::marker_ink`, an `L*` of their own on the same axis, so the resting field is free of the node's unlit rings.
The skin's part in it is `surface_faint`, the rung a fresh view opens on.

A node is a camera-facing billboard with signed-distance masks, so new node shapes are cheap:
change the coverage math, keep the compositing.

## Node bodies

- **Solid fill** — a matte filled disc at the node's centre, no glow at all.
The literal "filled circle instead of empty";
calmest possible look, pitch color at full.
- **Outline-only (rings for everything)** — every note a hollow colored
ring, stroke weight tracking activation.
A clean "wireframe lattice", and a view setting rather than anything a note carries in with it.
- **Filled + hard rim** — filled interior with a bright crisp edge (the
inverse of the outward glow);
reads as a coin/token rather than a star.
- **Concentric rings** — 1–3 nested rings; ring count or spacing encodes
octave or velocity without needing the separate octave glyphs.
- **Halftone fill** — disc filled with a dot screen whose density = level
(print/comic feel);
pairs well with a light skin.
- **Calm orb** — a smooth radial-gradient sphere with limb darkening, for a
3-D read the flat disc has no way to give.
- **Polygon nodes** — hex/triangle fills instead of circles, leaning into
the lattice-as-tiling geometry (hex especially suits the triangular just-intonation grid).

## Resting marker (a `+` at each home-sheet position)

**Cross tick is the shipped design** (PR #429), so this section is what is left to want rather than a menu.
Three bars set it —
an arm's length, the taper on its four ends, and the thickness across it —
and its edge is a ring's edge, the one screen-constant band every layer here is cut with.

- **Filled dot** — a small solid disc instead of the cross. It was a Shape
row for two commits and came out again:
one field of marks reads as a ground for the music to arrive on, and a choice between two of them is a setting nobody has a reason to move twice.
Rebuilding it is a variant in `plus_coverage` and a row to pick it.
- **Open ring** — a hollow circle rather than ink, so a position reads as a
socket a note lands in.
The one shape here that a note arriving could grow OUT of rather than cover.
- **Nothing at all** — the arm bar at 0, which is already reachable, leaving
the lattice at rest as the node rings alone.

## Lines between positions

**Nothing is drawn between two positions** —
PR #429 removed the grid pipeline whole, and the chord beams that shared its buffer went with it.
What the eye reads the rows and columns off is the regularity of the field itself, and a cross draws exactly what a pair of lines would draw where they meet.

So each of these is a NEW pipeline rather than a flag on an existing one, which is most of what they cost:

- **Solid hairlines** — thin continuous lines between neighbours with a faint
glow;
sharper, more technical, and the thing the field was traded for.
- **Comet beams** — a chord's notes joined, brighter at the root end, so
chord direction reads.
- **Weight by consonance** — simpler intervals thicker or brighter
(octave/fifth heavy, high-limit intervals thin).
- **Flow pulse** — a bright pulse travelling along a chord edge from the
root, looping while the chord is held.
- **The sevens tether** — the one line that ever LIT: a dashed chain hanging
an off-sheet note down to the home sheet, so it had something visible to hang from.
It went with the rest, and an off-sheet note now floats over the field with only its draw SIZE saying how far off it has gone.
The narrowest of these to bring back, and the only one that was answering a question.
- **Accent-tinted ground** — a desaturated hue over the field instead of the
neutral grey.
Not a skin swap:
both numbers the resting picture stands on are an `L*` with no hue axis at all, so it means giving them a chroma and a hue of their own.
It reaches two settings rather than one —
the markers take `ViewConfig::marker_ink`, while the audio ring's silent end and the band's unsounding slices stand on `lattice_ground` (`the_ring_and_an_idle_node_are_one_grey`), and the two open equal (`a_fresh_lattice_rests_in_one_grey`) without being held equal.
So "tint the lattice AT REST" is two edits, and keeping the surfaces together through them is part of what the idea costs.

## Whole-look skins (coordinated palettes)

These mostly need a second `Skin` value (the color struct already centralizes the chrome) plus a `lattice_ground` to match it —
the field is what the resting lattice is drawn in, and a light skin under a dark ground is two looks at once —
plus a matching node/line choice:

- **Blueprint** — deep blue field, cyan hairlines, hollow rings, mono
labels.
The closest of these to what already draws:
the resting cross is the blueprint tick.
- **Paper / ink** — light background, dark ink strokes, solid black dots
(needs a light skin variant).
- **Neon** — black field, saturated glowing filled circles, bright beams.
- **Minimal mono** — greyscale, flat filled discs, thin solid lines, no
glow.

## Cheapest first steps, if any of this gets picked up

1. **Solid fill** node body — a two-line branch in the node's ink and a
setting to reach it;
immediately gives the "filled circles" the backlog asked about.
2. **A second marker shape** — a variant in `plus_coverage` (which is one
distance field and an edge) plus a row to pick it.
The cheapest of these by a wide margin, and the only one that adds no pipeline.
3. **The sevens tether** — a pipeline of its own now, but the smallest one:
one instanced segment per off-sheet note, hanging to the home sheet.
