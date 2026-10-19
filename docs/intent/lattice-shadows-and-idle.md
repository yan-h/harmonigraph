# Intent: Lattice shadows and idle lattice

Yan’s decisions recorded for future drift audits.

## 2026-10-02 (issue sweep)

- Move Idle ring brightness to Note layers as Silent slice brightness. It controls silent MIDI octave slices of sounding/fading nodes and the audio ring’s quiet endpoint, not idle lattice positions. #1314 Shadows Q1

- Keep the Analyzer shadow curve in stops: its stronger tails separate ribbon and label ink from the spectrum. Its profile preview must show that curve; lattice shadows remain linear. #1314 Shadows Q2

## 2026-10-19

- Remove Contour (Distance) shadows entirely: Yan preferred Blur almost always. Every shadow group is a blur of its caster, with Width, Darkness and Spread. Keep the fixed glyph SDF sheet, which text Spread grows letters from. #1499
