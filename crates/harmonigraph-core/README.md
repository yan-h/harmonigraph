# harmonigraph-core

The pitch-and-lattice math underneath [Harmonigraph](../../README.md), with no dependencies at all —
pure `std`.

- `tuning` — pitch classes in microcents, just-intonation ratios, meantone detection, and tuning inference from held chords.
- `coords` — Tonnetz coordinates as prime-count vectors (fifths / major thirds / harmonic sevenths), plus note spelling with comma marks.
- `notes` — source-aware MIDI voice tracking, per-note tuning, release fades and explicit source/session resets.
- `history` and `roll` — recently played pitches, both as pitch memory and as a time-stamped note history.
- `configuration` and `policy` — resolved musical settings and the bounded adaptive-tuning scorer.
- `canonical` and `confirmed` — the source-aware publication boundary and its current-pitch state.
- `spectrum` — the shared pitch axis and Hz/MIDI conversions.
- `spectrogram` — quantized, age-tiered spectrum history.

## License

**`MIT OR Apache-2.0`**, at your option —
*not* the GPL-3.0-or-later that covers the application crates around it.
See [`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).

Two reasons for the split.
This crate is a general-purpose library rather than application code —
dependency-free microtonal math is useful to anyone writing a tuning tool, a Scala-file reader, or another plugin, and copyleft is exactly what would stop them.
And a meaningful share of the pitch math (the microcent `PitchClass` representation, note spelling, tuning inference) descends from [midi_lattice v1](https://github.com/yan-h/midi_lattice), which is permissively licensed;
relicensing that work under the GPL here would have reversed an earlier decision without anyone actually making it.

The dependency-free property is what keeps the boundary honest, so `ci.sh` enforces it:
adding a dependency to this crate fails CI.
Audio analysis lives in the separately permissive [`harmonigraph-analysis`](../harmonigraph-analysis), which depends on this axis contract and can use an FFT library without changing the guard.
