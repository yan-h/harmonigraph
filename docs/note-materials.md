# Lattice note materials

The Note material section of the Lattice settings offers three surfaces.
Smooth retains the original octave slices and remains the default for new and saved appearances.
Quiet pigment adds ragged radial edges and stable pigment texture,
with full-width neutral guides on unplayed octaves.
Broken traces keeps the same sounding slices,
with thinner grainy guides on unplayed octaves.
The two pigment surfaces preserve each octave sector and its dark seams.
At Gap 0%, Smooth sectors join;
pigment surfaces retain a fine seam to keep octaves distinct.

| Control | Meaning | Selected A/B setting |
| --- | --- | --- |
| Surface | Sounding pigment and quiet-guide shape | Quiet pigment (A), Broken traces (B) |
| Roughness | Radial displacement and feathered pigment fringe | 100% |
| Quiet guides | Opacity of unplayed octave guides | 100% |
| Guide width | Thickness of Broken traces guides | 100% |

The texture follows each slice through note animation without moving over its surface.
Thickness mappings change the sounding pigment's annulus;
releasing it continuously reveals its quiet guide.
Audio rings and melody/bass marks retain their own drawing.
Pigment stops before their occupied strips,
so those readings remain clear.
The existing Idle ring brightness setting supplies the neutral guide colour.
Thin traces cover less area than Quiet pigment.
Raise Idle ring brightness when brighter guides are wanted;
the captured appearance uses a dim value of 6.

The new settings are saved under `view.note_material`.
Missing settings use their defaults;
no existing appearance changes unless a pigment surface is selected.

The renderer timing probe compares all three styles with both shadow kernels:

```sh
cargo test -p harmonigraph-render a_frame_of_note_materials_costs_this_much -- --ignored --nocapture
```
