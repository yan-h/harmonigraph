# Making a video without recording your screen

The visualization is a pure function of `(note events, parameters, view,
camera, now)`. Nothing in the draw path consults a wall clock or a random
number generator. So the picture does not have to be produced at the same
speed as the music — you can record the *inputs* once and render the
*output* later, as slowly as the GPU needs, at any frame rate and any
resolution.

That is the whole idea:

```
   ┌── Bitwig: Export Audio (offline, faster than realtime) ──┐
   │                                                          │
   ├──> piece.wav          (the bounce, as always)            │
   └──> take-<stamp>.take  (what the plugin saw)              │
                    │
                    ▼
        lattice-offline piece.take --audio piece.wav --out piece.mp4
                    │
                    ▼
               piece.mp4  (exact CFR, any size, audio muxed)
```

Nothing is captured from a screen, so nothing is limited by your monitor,
your refresh rate, or whether a window was in front.

## The short version

```sh
# 1. In Bitwig: export audio as usual. A .take lands next to it (see below).
# 2. Render:
cargo build --release -p lattice-offline
./target/release/lattice-offline ~/Music/"MIDI Lattice 3D Takes"/take-1770000000.take \
    --audio ~/bounces/piece.wav \
    --out piece.mp4 \
    --size 3840x2160
```

## Pass 1 — recording a take

### From the DAW

**Turn on "Record take" in the View pane, then play the piece.** That's
it. You do not have to export, and you do not have to do anything
differently from how you'd play it anyway.

Takes are written to `~/Music/MIDI Lattice 3D Takes/take-<unixtime>.take`
(`LATTICE_TAKE_DIR` overrides the directory). The status line under the
toggle tells you where the file is going and how many events have landed.

Three things to know:

- **Nothing is recorded until the transport rolls.** Events are stamped
  with *transport position*, not a plugin-local clock, so a take lines up
  with a bounce of the same song automatically — no offset to work out.
  While the transport is stopped the status line says so.
- **The look is captured when you arm**, from what is on screen at that
  moment. (Unlike the project's saved `ui-state` blob, which only updates
  when the editor window closes — the trap `read-plugin-state.py`
  documents. You can also override it at render time with `--ui-state`.)
- **The device has to be in the note path**, exactly as it is live. What it
  no longer needs is *audio* — the spectrum is fed from the bounced WAV at
  render time, so the device's position only has to be right for MIDI.

> **Why a button and not automatic.** The first version armed itself when
> nice-plug reported `ProcessMode::Offline`, on the theory that exporting
> audio should also export a take. Bitwig disproved it: an export produced
> a take with parameter values, 37 `AllOff`s and *no notes*, while the
> lattice visibly lit up the whole time. Both can only be true if the pass
> carrying the notes wasn't the pass flagged offline — the host runs a
> short offline probe and then renders in realtime mode. Recording is
> explicit now, and works in any process mode.

### From the standalone harness (no DAW)

```sh
LATTICE_TAKE=/tmp/piece.take cargo run --release -p lattice-standalone
```

Records every note the harness sees — the mock progression, a MIDI
keyboard, or the DAW over an IAC bus — plus any parameter you move. The
harness's audio is a mock synth and is not recorded; pass the real bounce
to `--audio` at render time.

## Pass 2 — rendering

```sh
lattice-offline <take> [options]
```

The flags worth knowing (`--help` lists them all):

| flag | what it does |
|---|---|
| `--out` | `.mp4`/`.mov`/`.mkv` → ffmpeg; `.png` → numbered stills; `.rgba` → raw |
| `--audio` | the bounce: feeds the spectrum **and** is muxed into the video |
| `--layout` | preset name or a `.ron` file (see below) |
| `--size` | output pixels, e.g. `3840x2160` |
| `--scale` | pixels per point — the UI's *zoom*, not just its sharpness |
| `--fps` | default 60 |
| `--start` / `--end` / `--tail` | trim; `--tail` is the run-out after the last note |
| `--ui-state` | use a different look than the one in the take |

`--scale` is the one that isn't obvious. Font sizes and paddings are in
*points*, so the scale decides how large the UI reads **relative to the
frame**. The default keeps the same apparent size at any output
resolution; raise it for chunkier text, lower it to fit more lattice in.

Rendering is faster than realtime on an M-series Mac (roughly 19 s of
1080p60 in 17 s), so a five-minute piece is a coffee, not an afternoon.

## Layouts

Offline rendering does **not** reproduce the plugin's dock. It composes
its own picture — no tab bars, no settings columns, and whatever
proportions suit the piece.

Four presets: `side-by-side` (lattice left, upright roll right — the
Spectral pane's Auto orientation turns itself upright at that aspect),
`stacked` (the plugin's default arrangement), `lattice`, `spectral`.

For anything else, start from a preset and edit:

```sh
lattice-offline --layout side-by-side --dump-layout > mine.ron
lattice-offline piece.take --layout mine.ron
```

```ron
(
    background: (14, 14, 18),
    margin: 24.0,
    gap: 16.0,
    panes: [
        (pane: Lattice,  rect: (0.0, 0.0, 0.68, 1.0)),
        (pane: Spectral, rect: (0.68, 0.0, 1.0, 1.0)),
    ],
)
```

`rect` is `(x0, y0, x1, y1)` as **fractions of the frame**, origin
top-left — so one layout means the same picture at 1080p and at 4K. Panes
draw in order, so a later one overlaps an earlier one if you want a roll
inset over a full-bleed lattice.

## What is and isn't captured

**Captured:** every note event with its own sub-frame timestamp (so a
60 fps render is not quantized to 60 fps — envelopes start where they
actually started), per-note tuning and MPE bends, all eight automatable
parameters over time, and the view/camera/spectrum settings.

**Not captured:** camera *movement* (the camera is a UI control, not an
automatable parameter, so a take holds one fixed angle), anything you
click mid-piece, and the audio (which comes from the bounce).

## Why not just record the plugin window?

That was investigated first; see
[`docs/window-recording.md`](window-recording.md). Short version: it needs
a vendored `egui-baseview` patch to get a copyable surface and a reachable
device, and even then it is capped by live frame pacing — the plugin
repaints on demand, not at a fixed rate, so a frame-counting recorder
drifts against the music. Offline replay sidesteps all of it, and this
route needs no patch at all: the fonts, the panes and the lattice's paint
callback all work the same off a plain `egui_wgpu::Renderer` as they do in
the DAW.

## Crates

- `lattice-take` — the take format. Line-oriented RON, appendable,
  flushed per record so an interrupted export still renders everything up
  to the cut. Deliberately tiny: it is linked into the plugin.
- `lattice-offline` — the renderer. Headless wgpu device, egui driven by
  synthesized input, frames read back and piped to ffmpeg.

Determinism is a tested property, not an aspiration: `render.rs` renders
the same take twice and asserts the frames are byte-identical. If that
test ever fails, something time- or machine-dependent has entered the
draw path and renders have quietly stopped being reproducible.
