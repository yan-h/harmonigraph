# Session notes

## Afternoon session — 2026-07-17 (~3h autonomous block)

Seven commits, oldest first; audit as before (`git log`, standalone,
Bitwig for plugin items). Rendering experiments are all switchable with
the previous look as default.

- **0385a6f — Shader hot-reload (standalone)**: edit `lattice.wgsl`, save,
  see it. Broken WGSL is rejected (naga-validated) and logged; the old
  pipeline keeps rendering. *Audit*: run the standalone from a terminal,
  touch the shader, watch stderr.
- **6cf8a21 — Held-note render styles**: Appearance → Node style:
  Steady / Breathe / Corona / Sparks / Wire. *Audit*: hold chords, cycle
  styles; idle nodes identical everywhere.
- **b62a3fe — Chord edges**: Appearance checkbox; held adjacent nodes get
  beams (a just triad = a lit triangle). Parallel beams under 12-TET are
  enharmonic duplicates, working as intended. *Audit*: hold C-E-G with
  Just tuning + tolerance up.
- **02e6d5b — Grid window center + camera pan**: View section center bars
  (v1 Grid X/Y/Z); shift-drag or middle-drag pans, double-click resets.
  *Audit*: pan center; Notes-pane `node` column and spectral ticks follow.
- **d4496d8 — Clock reconciliation**: fast runs keep their timing instead
  of quantizing to GUI frames. *Audit*: unit tests; play fast arpeggios.
- **93e129b — Hardware MIDI in the standalone**: floating "MIDI input"
  window, bottom-right; pick a CoreMIDI port. *Audit*: needs your
  controller — the one item not verifiable without hardware.
- **(this commit) — Upstream docs**: combined patch files + drafts for
  four upstream fixes in `docs/upstream/`; nothing submitted.

Deliberately deferred: second skin + live re-skinning, offscreen depth
pass, spectral audio FFT (unchanged from the overnight list).

---

# Overnight session notes — 2026-07-17

Eleven commits, one per feature, oldest first. Each entry: what changed and
a ~2-minute audit. Quick global audit: `cargo test --workspace` (18 tests),
`cargo run -p harmonigraph-standalone`, CI on GitHub Actions, and fresh bundles
in `target/bundled/`.

All items are correctness/mechanism work — no aesthetic decisions were made
without a toggle or default preserving the existing look.

## 3df3a4a — Param automation gestures
ValueBar drags now bracket begin/end so hosts record ONE automation
gesture per drag; typed values are single-value gestures.
**Audit**: in Bitwig, arm automation recording, drag a tuning bar; the
lane should show one clean gesture, not dozens.

## cf31a31 — Repaint throttling
Continuous rendering only while voices sound/decay; 20 Hz idle poll;
plugin repaints immediately when MIDI arrives.
**Audit**: open Activity Monitor with the plugin idle (no notes, editor
open) — GPU/CPU usage should be visibly lower than while notes play.

## 0cbf3cd — v1 channel semantics + pitch coloring
Channels (MIDI convention): 1-9 fixed colors, 10-14 pitch-height gradient
between new Darkest/Brightest Pitch params, 15 outline ring, 16 ignored.
**Audit**: send notes on those channels; `cargo test -p harmonigraph-scene`.

## 339d85f — Per-note tuning (PolyTuning/MPE)
Bent notes re-map pitch class, octave, and gradient color.
**Audit**: `cargo test -p harmonigraph-core` (tuning_bends_pitch_class_and_octave);
or play MPE slides in Bitwig and watch nodes hand off.

## ca9ca68 — Tuning-learn
"Learn" button (Tuning pane): hold a justly tuned chord, click — C offset
and 3/5/7 tunings snap to the held intervals (within 50¢/40¢, as v1).
**Audit**: hold C-E-G in the standalone's mock progression window... easier:
click Learn while mock chords play; console logs what was learned.

## c569a6f — UI persistence
Dock layout, camera, view settings survive editor close/reopen and project
save/load (plugin persist blob; eframe storage in the standalone). Format
is RON because JSON cannot round-trip the NaN rects in fresh dock layouts.
**Audit**: rearrange tabs + orbit the camera, close and reopen the editor.

## 038f8c0 — Spectral pane
MIDI-derived pitch-class meter on a 0-1200¢ axis: voice bars with lattice
colors, 12-TET gridlines, ticks where lattice nodes sit. Hover syncs BOTH
ways with the lattice (band here; node highlight there).
**Audit**: play notes, hover the Spectral pane and the lattice.

## 2469056 — Note-name labels
v1's note spelling (chain of fifths; syntonic comma marks like "E-1") on
hovered + sounding nodes, faded by envelope. "Note labels" checkbox in
View. Tooltip now leads with the name.
**Audit**: hover the origin node — "C"; one node up — "G".

## ca8b352 — Skin mechanism (no visual change)
`lattice_scene::skin::Skin` owns every color (scene + UI chrome); theme
constants became accessors. The single built-in skin reproduces the old
constants exactly.
**Audit**: the UI looks identical to last night; `git show ca8b352` for
the shape.

## 8377bf3 + 7b106cf — CI + clippy
GitHub Actions (macOS: clippy -D warnings + tests); workspace clippy-clean.
**Audit**: green check on GitHub next to HEAD.

## Upstream prep (this commit)
`docs/upstream/`: ready-to-submit PR text for the baseview run-loop fix
(branch pushed to yan-h/baseview) and the egui-baseview unit fix (patch
file; Codeberg), plus a design-discussion draft for nice-plug-egui.
**Nothing was submitted upstream** — that call is yours.

## Known follow-ups
- Bitwig checks marked above (gestures, MPE, persistence) need your hands.
- Adding a second Skin + live re-skinning; shader-side skin uniforms.
- Spectral pane audio FFT upgrade (needs audio-thread analysis).
- Standalone real MIDI input (midir) for testing without a DAW.
