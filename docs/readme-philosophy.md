# README philosophy

The README introduces Harmonigraph through the musical problem it solves,
then gives a new reader just enough context to see whether the project is for them and try it.
It is not the maintainer manual or the complete settings reference.

## Purpose and audience

Harmonigraph has two equal purposes:
helping me explore and understand microtonal harmony while I compose,
and making beautiful, musically informative videos of my pieces.
The writing should give beauty and understanding equal weight.

Assume the reader knows familiar chord names and intervals.
Keep the introduction and feature descriptions concise,
and link the guides rather than teaching microtonal terminology in the README.

This is the musical motivation in my own words:

> The lattice lets me identify intervals and chord types by shape, in a transposition invariant way, which lets me understand my harmony much faster on the fly. It also gives me a good sense of harmonic distance, so when composing in just intonation I can see instantly which wolf intervals are created and whether we have comma pumps.

This motivation informs the product and its feature descriptions;
the README does not need to explain it in full.

## Project voice and boundaries

Write in focused, plain, concise first-person prose without marketing language.
Near the opening, say clearly that this is a personal, fast-moving project,
written primarily through LLM coding sessions under my direction and review,
and driven first by my own musical needs.
State the practical consequences directly:
expect rough edges and breaking changes,
and treat macOS with Bitwig Studio as the only tested environment rather than promising broader support.

Keep visualization separate from note processing.
The established core is the MIDI lattice,
the incoming-audio spectrum and spectrogram with MIDI comparison,
appearance and layout control,
and deterministic video export from recorded musical data rather than screen capture.
Adaptive tuning belongs in the same compact feature list,
clearly labelled experimental and CLAP-only,
with a link to its guide.

Prefer this narrative progression without freezing exact headings:

1. Musical purpose and project identity.
2. A screenshot and video example.
3. A compact feature list.
4. The shortest verified way to try the project.
5. Further user guides, development documentation and licence scope.

## Media plan

Keep media sparse and purposeful.
Omit unchosen assets rather than leaving editorial placeholders in the README.

- A YouTube or other hosted video URL:
  place a short text link directly below the hero screenshot.
- `docs/images/plugin-window-lattice-analyzer.webp`:
  a wide screenshot beside the demonstration,
  showing the lattice and spectral view together without obscuring the music with settings panels.
- `docs/images/video-export-or-customization.png`:
  optional later image;
  use it only if it demonstrates either a materially different export layout or the range of visual customization.

For repository-hosted images,
use the existing `docs/images/` directory and paths relative to the README such as `docs/images/plugin-window-lattice-analyzer.webp`.
Do not commit the main video binary to Git;
use a hosted player or linked thumbnail.
The export/customization image is optional.

## Maintenance rule

Add material to the README when it helps a new reader understand the project or begin using it.
Put detailed behaviour,
host-specific edge cases,
implementation design,
build mechanics and maintainer workflows in the relevant document under `docs/`,
then link it once from the README.
Do not keep the same procedure in both places.
Update claims when the shipped behaviour changes,
but do not turn this philosophy into a feature inventory or a fixed table of contents.
