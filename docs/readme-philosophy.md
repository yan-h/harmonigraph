# README philosophy

The README introduces Harmonigraph through the musical problem it solves,
then gives a new reader just enough context to see whether the project is for them and try it.
It is not the maintainer manual or the complete settings reference.

## Purpose and audience

Harmonigraph has two equal purposes:
helping me explore and understand microtonal harmony while I compose,
and making beautiful, musically informative videos of my pieces.
The writing should give beauty and understanding equal weight.

Assume the reader knows familiar chord names and intervals,
but not microtonal terminology.
Introduce pitch class, temperament, just intonation and Tonnetz only where the musical example needs them.
A reader should understand the main explanation without following a reference link.

This is the musical motivation in my own words:

> The lattice lets me identify intervals and chord types by shape, in a transposition invariant way, which lets me understand my harmony much faster on the fly. It also gives me a good sense of harmonic distance, so when composing in just intonation I can see instantly which wolf intervals are created and whether we have comma pumps.

Start from a familiar example such as C-E-G before introducing technical vocabulary.
Explain that transposition moves a chord without changing its shape,
so the eye can recognize its interval structure without reading every note.
Also explain that harmonic distance is not simply distance from low to high pitch:
two almost equal pitches can represent different harmonic paths through a just-intonation lattice.

Define wolf intervals and comma pumps briefly in the README.
Treat them as phenomena a composer may want to see and use,
not automatically as mistakes or unwanted dissonance.
Do not imply that Harmonigraph automatically detects or corrects either one.

## Project voice and boundaries

Write in focused, plain, warm first-person prose without marketing language.
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
Adaptive tuning belongs in a short, separately labelled experimental section after those features.
Describe its current CLAP-only Tune-and-Hub shape,
link to the setup guide,
and do not present it as finished or as the recommended first experience.

Prefer this narrative progression without freezing exact headings:

1. Musical purpose and project identity.
2. A demonstration and what to notice.
3. The lattice explanation through a concrete chord.
4. Established user-facing visualization and export features.
5. Experimental adaptive tuning.
6. The shortest verified way to try the project.
7. Further user guides, development documentation and licence scope.

## Media plan

Keep media sparse and purposeful.
Until final assets are chosen,
use visible editorial placeholders with meaningful captions rather than fake URLs or broken image links.

- A YouTube or other hosted video URL,
  optionally presented through a linked thumbnail at `docs/images/harmonigraph-demo-thumbnail.png`:
  place the primary demonstration immediately after the opening.
  Its caption should ask the viewer to notice how chord shapes remain recognizable as the harmony moves,
  and how the audio and MIDI views relate.
- `docs/images/harmonigraph-hero.png`:
  optional wide screenshot beside the demonstration,
  showing the lattice and spectral view together without obscuring the music with settings panels.
- `docs/images/lattice-c-major-shape.png`:
  place after the C-E-G explanation only if it makes the repeated chord shape clearer than prose alone.
- `docs/images/video-export-or-customization.png`:
  optional later image;
  use it only if it demonstrates either a materially different export layout or the range of visual customization.

For repository-hosted images,
use the existing `docs/images/` directory and paths relative to the README such as `docs/images/harmonigraph-hero.png`.
Do not commit the main video binary to Git;
use a hosted player or linked thumbnail.
The minimum useful set is one main demonstration and one transposition/chord-shape illustration.
The hero and export/customization images are optional.

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
