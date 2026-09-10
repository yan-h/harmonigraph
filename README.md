# Harmonigraph

Harmonigraph is the harmony visualizer I use while composing microtonal music,
and the tool I use to turn my pieces into beautiful,
musically informative videos.
It runs as a CLAP or VST3 audio plugin and relates the MIDI I play to the audio I hear.

This is a personal,
fast-moving project driven primarily by my own musical needs.
Most of its code is written in LLM coding sessions under my direction and review.
Expect rough edges and breaking changes.
I have tested it only on macOS with Bitwig Studio;
I do not yet make broader host or platform support claims.

## See it in motion

> **Primary media placeholder:** put the chosen demonstration video here,
> with an optional wide screenshot showing the lattice and spectral view together.
> The exact paths and placement notes are in the [README philosophy](docs/readme-philosophy.md#media-plan).

For a current example,
watch [*slipstream* (5-limit just intonation)](https://www.youtube.com/watch?v=VuD9JOmi6_o).
In a demonstration,
look for chord shapes that stay recognizable as the harmony moves,
and for the relationship between MIDI shapes and the spectrum of the sounding result.
Those two readings — harmonic structure and sounding result — are the center of Harmonigraph.

## Reading harmony as shape

On a piano roll,
C-E-G and D-F♯-A occupy different vertical positions.
In Harmonigraph they draw the same triangle in different places.
The chord has been transposed,
but its intervals have not changed,
so I can recognize its type from the shape without reading every note name.

The board is a three-dimensional [Tonnetz](https://en.wikipedia.org/wiki/Tonnetz):
a lattice whose three directions represent a perfect fifth,
a major third and a harmonic seventh.
Each node is a pitch class — a lattice position that collects pitches related by octaves —
while circular slices distinguish individual voices and registers.
Each direction has its own tuning,
so the same picture can represent equal temperament,
just intonation or something between them.
Equal temperament divides the octave into equal steps;
just intonation instead tunes intervals from simple whole-number frequency ratios.

This makes harmonic distance visible as something different from low-to-high pitch distance.
For example,
an E reached as a pure major third above C and an E reached by following four pure fifths and bringing the result into the same octave differ by only a comma — a small pitch difference —
but they arrived by different harmonic paths and occupy different places in the lattice.
That distinction matters when a progression moves through just intonation.

It can expose a [wolf interval](https://en.wikipedia.org/wiki/Wolf_interval),
an expected interval that is noticeably wider or narrower than its just tuning,
or a [comma pump](https://en.wikipedia.org/wiki/Comma_pump),
a progression whose locally pure steps accumulate a small pitch shift when the harmony returns to its named starting point.
These are not automatically errors:
they can be problems to avoid or musical effects to use.
Harmonigraph makes the relationships visible;
it does not claim to identify or correct them automatically.

> **Chord illustration placeholder:** after choosing an image,
> place `docs/images/lattice-c-major-shape.png` here to compare C-E-G with the transposed D-F♯-A shape.

## How I use it

I play on [Latticeboard](https://github.com/yan-h/lattice-board),
an isomorphic MIDI keyboard I designed around the Wicki–Hayden layout.
Its fingering patterns stay consistent when transposed,
complementing the way Harmonigraph makes harmonic relationships recognizable by shape.
Harmonigraph also works with other MIDI controllers.

I usually start by improvising in a tuning close to [31-tone equal temperament](https://en.wikipedia.org/wiki/31_equal_temperament) or [quarter-comma meantone](https://en.wikipedia.org/wiki/Quarter-comma_meantone).
In 31-tone equal temperament,
the octave is divided into 31 equal steps.
The lattice helps me recognize chords and follow their relationships while I play.

Later,
I return to just intonation for the finished piece where possible.
Seeing the harmonic paths makes it easier to understand where that works and where wolf intervals or comma pumps complicate the move.

Once the music is finished,
I use Harmonigraph to create its video.

## What it does

- **Lattice MIDI visualization.** Incoming notes become interval shapes on a tunable Tonnetz,
  with voice register,
  note history and harmonic relationships visible at once.
- **Audio and MIDI comparison.** A live spectrum,
  scrolling spectrogram and related spectral views show incoming audio on the same pitch axis as the played notes.
- **A configurable picture.** Camera,
  lattice detail,
  frequency range,
  colors,
  lighting,
  pane visibility and layout can be shaped for analysis or presentation.

Visualization does not change the notes sent to an instrument.
The optional adaptive-tuning component below is a separate note-processing path.

See the [settings guide](docs/settings.md) for controls and units.

## Making videos

Arrange the panes and choose the appearance you want,
then arm **Record take** in the Video pane and play the piece or export its audio.
Harmonigraph records the performance data and the selected audio rather than capturing the screen.
The offline renderer can then draw every frame at the chosen resolution and frame rate,
without being limited by the monitor or realtime rendering speed.
You can re-render the same recorded performance later with a different appearance or layout.

See [making a video without recording the screen](docs/offline-rendering.md) for the complete workflow.

## Experimental: adaptive tuning

The CLAP bundle also contains an experimental **Harmonigraph Tune** note effect.
Placed before an instrument,
it can tune each new note from the harmonic context shared by a full Harmonigraph Hub.
This CLAP-only feature has rough edges and is not the recommended first way to meet the project;
the VST3 bundle remains the visualizer alone.
The [Bitwig setup and verification guide](docs/adaptive-tuning.md#setting-it-up-in-bitwig) explains the Tune/Hub layout and current limitations.

## Try it from source

Install Rust with [rustup](https://rustup.rs/) and clone the repository,
then install `sccache`,
which the workspace requires as its compiler wrapper:

```sh
git clone https://github.com/yan-h/harmonigraph.git
cd harmonigraph
brew install sccache
```

The quickest way to see the full interface is the standalone harness.
It needs no DAW and starts with a mock progression;
it can also listen to a connected MIDI port:

```sh
cargo run -p harmonigraph-standalone
```

For Bitwig,
create the bundle structure once from the main checkout:

```sh
cargo xtask bundle harmonigraph-plugin --release
```

Add `<checkout>/target/bundled/` under **Settings → Locations → Plug-in Locations** in Bitwig.
After that,
`./update-plugin.sh` builds the current checkout and loads its CLAP/VST3 plugin plus matching offline renderer.
After installing,
deactivate and reactivate Bitwig's audio engine.

Install `ffmpeg` with `brew install ffmpeg` when you want encoded video rather than a frame sequence.
See [developing Harmonigraph](docs/development.md) for test commands,
worktree-safe build loading,
architecture and dependency notes.

## Guides

- [Settings](docs/settings.md)
- [Making a video without recording the screen](docs/offline-rendering.md)
- [Adaptive tuning](docs/adaptive-tuning.md)
- [Development and architecture](docs/development.md)
- [README philosophy and media placement](docs/readme-philosophy.md)

Harmonigraph succeeds [midi_lattice](https://github.com/yan-h/midi_lattice),
the earlier project from which much of its pitch math descends.

## License

Copyright (C) 2026 Yan Han.

Harmonigraph is licensed under the [GNU General Public License v3.0 or later](LICENSE).
It comes without warranty;
see the licence for the full terms.

The reusable [`harmonigraph-core`](crates/harmonigraph-core) and [`harmonigraph-analysis`](crates/harmonigraph-analysis) crates are instead licensed under `MIT OR Apache-2.0`.
The vendored forks under [`vendor/`](vendor) retain their upstream `MIT OR Apache-2.0` terms and their own licence files;
[`PATCHES.md`](PATCHES.md) records the local changes.

VST is a trademark of Steinberg Media Technologies GmbH.
