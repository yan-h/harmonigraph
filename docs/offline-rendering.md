# Making a video without recording your screen

The visualization is a pure function of `(note events, parameters, view,
camera, now)`. Nothing in the draw path consults a wall clock or a random
number generator. So the picture does not have to be produced at the same
speed as the music — you can record the *inputs* once and render the
*output* later, as slowly as the GPU needs, at any frame rate and any
resolution.

That is the whole idea:

```
   arm "Record take" in the Video pane, play the piece once
                    │
                    ├──> take-<stamp>.take   what the plugin saw
   export audio ────┴──> piece.wav           the bounce, as always
                    │
                    ▼
        harmonigraph-offline take.take --audio piece.wav --out piece.mp4
                    │
                    ▼
               piece.mp4  (exact CFR, any size, audio muxed)
```

Both are stamped on the host transport, so they line up with no offset to
work out.

Nothing is captured from a screen, so nothing is limited by your monitor,
your refresh rate, or whether a window was in front.

## The short version

```sh
# 1. In Bitwig: arm "Record take" (Video pane), play the piece, disarm.
#    Export audio as usual for the soundtrack.
# 2. Render:
cargo build --release -p harmonigraph-offline
./target/release/harmonigraph-offline ~/Music/"Harmonigraph Takes"/take-1770000000.take \
    --audio ~/bounces/piece.wav \
    --out piece.mp4 \
    --size 3840x2160
```

## Pass 1 — recording a take

### From the DAW

**Turn on "Record take" in the Video pane, then play the piece.** That's
it. You do not have to export, and you do not have to do anything
differently from how you'd play it anyway.

Takes are written to `~/Music/Harmonigraph Takes/take-<unixtime>.take`
(`LATTICE_TAKE_DIR` overrides the directory). The status line under the
toggle tells you where the file is going and how many events have landed.

The plugin also writes a WAV beside the take — always, with nothing to
tick — so the render gets its spectrum and its soundtrack with no separate
bounce and nothing to point at. The catch is placement: the device has to
be somewhere audio actually reaches it — after the instrument, or on a
bus. On a pure note track there is nothing to record, and the take is
notes-only (which still renders; you just get no spectrum curve).

The WAV is 32-bit float, exactly as the input arrives, and it carries the
take time of its first sample — so arming part-way into a song still
lines the sound up with the picture, in both the spectrum and the muxed
track.

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

#### Does this work during an audio export?

Yes. Recording is not gated on the host's process mode — if Record take
is on and the transport is moving, the take is captured, offline render or
not. Set **When** to *Transport stops* and the video renders itself when
the export finishes.

One subtlety that was worth getting right: whether the transport counts as
rolling is the **union** of "the position advanced" and the host's
`playing` flag, not the flag alone. Some hosts report `playing = false`
throughout an offline render — nothing is being *played*, after all — and
trusting the flag would have silently recorded nothing for the whole
export.

#### Rendering automatically when the take ends

A finished take always renders: the plugin runs `harmonigraph-offline`
itself and writes the video next to the take. There is nothing to enable,
and no field for the renderer's path — it uses the copy `update-plugin.sh`
installs, and the audio it muxes and analyzes is the take's own recording.
What you do choose is **when** a take counts as finished, in the **Finish**
row below.

The video's size comes from **Aspect** and **Resolution** in the Frame
section, which the plugin passes as `--size`. There is no field for raw
renderer flags: there was one, defaulting to `--size 1920x1080`, and since
the renderer reads `--size` *after* resolving the take's frame it outranked
every Aspect but 16:9 — a 9:16 frame previewed tall and rendered wide. The
flags it could reach are all reachable by running the renderer on the take
by hand, which has completion, `--help`, and real error messages.

**Finish** has three settings:

| setting | what ends the take |
|---|---|
| On disarm | you switch Record take off — predictable, and it works however the transport behaves |
| On stop | the transport stops after something was recorded; recording disarms at the same moment |
| At loop end | one arranger-loop pass, ending the moment the loop wraps. Needs looping ON; with looping off it waits for a disarm |

*On stop* is what makes **exporting audio produce a video with nothing
further to click**: arm Record take, export, and both files land together.

ffmpeg is found automatically — on `PATH`, then in the usual install
locations. This matters more than it sounds: a macOS app launched from
Finder gets a `PATH` of `/usr/bin:/bin:/usr/sbin:/sbin`, with no Homebrew
in it, so a plugin-launched render would otherwise fail with "install
ffmpeg" on a machine that has ffmpeg. Override with `--ffmpeg /path` when
running the renderer by hand, or set `LATTICE_FFMPEG`.

The render runs on its own thread and never touches the audio thread or
the GUI, so a long one does not hold up the DAW. The status line reports
`rendering ...`, then either the finished path or the renderer's own error
— which is the only place you'll see it, since a plugin has no terminal.

`update-plugin.sh` builds the renderer alongside the plugin and installs
it to `~/Library/Application Support/Harmonigraph/harmonigraph-offline`, so
the two always match — they share the take format, so a mismatched pair
would fail at the version check.

Note the ordering: the render is launched by the *writer thread*, right
after it closes the file. That is the only point that knows the take is
actually complete on disk.

### From the standalone harness (no DAW)

```sh
LATTICE_TAKE=/tmp/piece.take cargo run --release -p harmonigraph-standalone
```

Records every note the harness sees — the mock progression, a MIDI
keyboard, or the DAW over an IAC bus — plus any parameter you move. The
harness's audio is a mock synth and is not recorded; pass the real bounce
to `--audio` at render time.

## Pass 2 — rendering

```sh
harmonigraph-offline <take> [options]
```

The flags worth knowing (`--help` lists them all):

| flag | what it does |
|---|---|
| `--out` | `.mp4`/`.mov`/`.mkv` → ffmpeg; `.png` → numbered stills; `.rgba` → raw |
| `--audio` | audio to use instead of the take's own: feeds the spectrum **and** is muxed in |
| `--align` | how `--audio` lines up: `auto` (default), `off`, or a start in seconds |
| `--layout` | preset name or a `.ron` file (see below) |
| `--size` | output pixels, e.g. `3840x2160`; default is the take's own aspect with its short edge at 1080 |
| `--scale` | pixels per point — the UI's *zoom*, not just its sharpness |
| `--fps` | default 60 |
| `--lead` | extra empty frame before the recording starts; default is the take's own **Lead-in** |
| `--start` / `--end` / `--tail` | trim; `--start` is an absolute song position, `--tail` the run-out after the last note |
| `--ui-state` | use a different look than the one in the take |
| `--playhead` | lay the whole take's spectrogram out at once and sweep a playhead through it |

### Where the video starts

Take times are the **host's transport position**, so time zero is the song's
start, not the record button's. Play a passage from a minute into the
arrangement and its first note lands at 60-odd seconds — so a render from
zero opens with a minute of empty lattice, however little silence you heard
before playing.

The render therefore begins **where the recording did**, wherever in the song
that falls. Nothing exists before that point — no events, no audio — so
everything it trims was guaranteed empty. The anchor is the take's first
event, and arming writes a full parameter snapshot whether or not anything is
played, so a take carrying sound but **no MIDI at all** anchors correctly too;
recorded audio contributes its own start as well, and the earliest wins.

The Video pane's `Lead-in` slider (0–5s, default 0) adds stillness *before*
that, and `--lead` overrides it per render. Default 0 because the run-up you
actually played is already in the video — the lead is a taste, not a
correction, and it is empty frame by construction.

`--start` still means an absolute song position and outranks the lead: use it
to skip *to* a passage, or `--start 0` to open at song zero.

A take played from song zero is unaffected either way — the recording already
begins at the start, so there is nothing to trim.

`--size` is a *size*, not a shape. Left off, it takes the aspect the take
was framed at and puts the short edge at 1080 — 16:9 renders 1920x1080,
9:16 renders 1080x1920. Given a different aspect it does not letterbox or
crop: the layout recomposes at whatever pixels it is handed, so the split
falls elsewhere and the lattice camera exposes a different amount of the
board. That is a legitimate thing to ask for, so it is allowed, but it
renders a different picture from the preview and the renderer says so.

`--scale` is the one that isn't obvious. Font sizes and paddings are in
*points*, so the scale decides how large the UI reads **relative to the
frame**. The default keeps the same apparent size at any output
resolution; raise it for chunkier text, lower it to fit more lattice in.

`--playhead` changes how time reads. Normally the spectrogram and roll
scroll past a fixed now-line, showing the last few seconds. With `--playhead`
the whole take is laid out at once — the entire spectrogram across the frame —
and a playhead sweeps through it, so the finished shape of the piece is visible
the whole way rather than arriving and scrolling off. It needs audio (the
spectrogram is audio-derived) and uses whatever spectrogram and roll look the
take already carries. The roll is laid out ahead too — the whole piece is
on screen from the first frame, not only the spectrogram.

Rendering is faster than realtime on an M-series Mac (roughly 19 s of
1080p60 in 17 s), so a five-minute piece is a coffee, not an afternoon.

## Replacing crackly audio with a clean bounce

Live playback can crackle, so the audio a take records is not always good
enough to ship. Bounce a clean WAV of the same performance and pass it as
`--audio`:

```sh
harmonigraph-offline take.take --audio clean-bounce.wav --out piece.mp4
```

The catch a naive swap hits is alignment. The clean bounce might start at
a different song position than where recording armed, and plugin-delay
compensation can shift it by a constant amount — either way the sound
drifts against the picture.

The fix uses the take's own recording as a timing reference. That
recording is crackly, but it is stamped to the same clock as the notes,
so it is *already* aligned to the visualization. `--audio` cross-correlates
the clean bounce against it — on onset envelopes, which shrug off the
crackle and the level difference — and places the bounce wherever it
actually belongs. It prints what it found:

```
aligned audio to the take's recording: soundtrack starts at 3.500s (confidence 0.97)
```

Confidence near 1 is a solid match; if it comes out low, the two files may
not be the same performance, and `--align <seconds>` sets the start by
hand. `--align off` skips correlation and assumes the bounce starts at
take zero.

This needs the take to have recorded its own audio, which it does unless
the device sat somewhere no audio reached it. Without a reference there is
nothing to correlate against, and the bounce is assumed to start at take
zero unless you say otherwise.

## Layouts

Offline rendering does **not** reproduce the plugin's dock. It composes
its own picture — no tab bars, no settings columns, and whatever
proportions suit the piece.

Five presets: `side-by-side` (lattice left, Spectral pane right — the
arrangement the plugin's own default dock uses), `stacked`, `lattice`,
`spectral`, `spiral` — the two arrangements the panes were designed around,
plus each pane alone. The spiral is a disc, so it centres itself in the frame
it is given, and a composition wanting it beside something else is a
hand-written `.ron`.

A preset places the panes and nothing else: the Spectral pane renders at
whichever orientation the take's own UI state carries, so a `side-by-side`
render wants Top or Bottom picked in the pane before the take is recorded —
its column is tall and narrow, and Left scrolls the spectrogram across the
short side. Nothing infers it from the aspect, deliberately: a picture that
changes with the size it is rendered at is not one you can dial in.

For anything else, start from a preset and edit:

```sh
harmonigraph-offline --layout side-by-side --dump-layout > mine.ron
harmonigraph-offline piece.take --layout mine.ron
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

**Captured alongside the notes:** the input bus as 32-bit float, aligned
to them.

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

- `harmonigraph-take` — the take format. Line-oriented RON, appendable,
  flushed per record so an interrupted export still renders everything up
  to the cut. Deliberately tiny: it is linked into the plugin.
- `harmonigraph-record` — the recording end. The audio thread's ring
  producers, the writer thread that turns them into a take, and the
  subprocess driver that runs `harmonigraph-offline` on a finished one.
  Sits beside the format crate rather than inside it because recording is a
  different job from reading: audio-thread rings, transport handling and a
  subprocess driver, none of which reading a take should have to link.
- `harmonigraph-offline` — the renderer. Headless wgpu device, egui driven by
  synthesized input, frames read back and piped to ffmpeg.

Determinism is tested, but the test is narrower than the property:
`render.rs` renders the same take twice and asserts the frames are
byte-identical, at 320x200, `side-by-side`, ten frames of one second. If it
ever fails, something time- or machine-dependent has entered the draw path.

The test is still narrower than the property, so treat it as a tripwire on
the pipeline rather than a guarantee about a real export. What it cannot see
is a tie: its take is three chords on one channel with no two voices sharing
a pitch class, so nothing in it exercises the case where two voices light one
node and something has to choose between them. That blind spot is what let
issue #135 — a hash map's per-map iteration order deciding a doubled node's
colour, and so its pixels — live in shipped renders while this stayed green.

#135 is fixed (the tracker's collections are ordered, not hashed), and what
guards it now is a set of unit tests on those collections' iteration order
rather than this render. That is deliberate: a hash map can always come back
sorted, so an end-to-end render is at best a probabilistic detector of one,
while asserting key order over a few hundred keys fails with probability 1.
