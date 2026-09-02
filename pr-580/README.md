Renders for #580's clearance experiment, shot through the offline harness at
2 device pixels per point from Yan's captured DAW state (`New 2.bwproject`,
`glow_shadow_kernel: Distance`, `glow_shadow: 0.419`, `octave_gap: 0.084`).

`isolated-*` is that state with the mark strip and audio ring switched off, so
the octave gaps are the only pair standing across a gap and nothing with a zero
clearance can occupy the runner-up slot. `daw-*` is the state whole.

Three builds of one frame each: `1-nearest-field` is `union_distance` switched
off, `2-pr578` is the facing ramp with every clearance counted whole, and
`3-clearance` is this branch. The `diff-*` frames are that build minus the
nearest field, amplified 16x about mid-grey — green is a DARKER shadow than the
min-distance row draws.

Not part of the build; this branch exists only so a PR body can link them.
