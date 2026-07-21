# lattice-core

The pitch-and-lattice math underneath [MIDI Lattice 3D](../../README.md),
with no dependencies at all — pure `std`.

- `tuning` — pitch classes in microcents, just-intonation ratios, meantone
  detection, and tuning inference from held chords.
- `coords` — Tonnetz coordinates as prime-count vectors (fifths / major
  thirds / harmonic sevenths), plus note spelling with comma marks.
- `notes` — MIDI voice tracking with release fades and channel roles.
- `history` — recently-played pitch memory.
- `spectrum` — a hand-rolled radix-2 FFT and peak picker.

## License

**`MIT OR Apache-2.0`**, at your option — *not* the GPL-3.0-or-later that
covers the rest of this workspace. See [`LICENSE-MIT`](LICENSE-MIT) and
[`LICENSE-APACHE`](LICENSE-APACHE).

Two reasons for the split. This crate is the only part of the workspace that
is a general-purpose library rather than application code — dependency-free
microtonal math is useful to anyone writing a tuning tool, a Scala-file
reader, or another plugin, and copyleft is exactly what would stop them.
And a meaningful share of the pitch math (the microcent `PitchClass`
representation, note spelling, tuning inference) descends from
[midi_lattice v1](https://github.com/yan-h/midi_lattice), which is
permissively licensed; relicensing that work under the GPL here would have
reversed an earlier decision without anyone actually making it.

The dependency-free property is what keeps the boundary honest, so `ci.sh`
enforces it: adding a dependency to this crate fails CI. If one is ever
genuinely needed it must be permissively licensed, and the guard must be
updated deliberately rather than by accident.
