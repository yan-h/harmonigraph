# Recording the plugin window to video

An investigation, not an implementation. The question was whether the
plugin could record its own window to a video file, so making a video to
go with a track doesn't mean running OBS alongside the DAW.

**Short answer: yes, and nothing about the architecture forbids it — but
the version that literally records the plugin window is the *most*
expensive of the four ways to get the video, and the cheapest way to a
better-looking result records something else.** The findings below are
what that conclusion is built on; read §5 first if you only want the
recommendation.

> **Outcome: route 2 (offline replay) was built.** See
> [`offline-rendering.md`](offline-rendering.md) for the workflow. This
> document is kept for the analysis — in particular §2 and §3, which are
> the reasons live window capture is still not worth doing.

## 1. What the window actually is

On macOS the plugin editor is egui 0.35 → egui-wgpu 0.35 → wgpu 29 →
**Metal**. No OpenGL path is compiled in (`Cargo.toml` builds
egui-baseview with `default-features = false, features = ["wgpu"]`).

`EguiWindow::open_parented` (in `crates/harmonigraph-plugin/src/editor.rs`)
parents onto the host's `NSView`, and
`vendor/egui-baseview/src/renderer/wgpu/renderer.rs:59-83` creates a wgpu
surface on it — a `CAMetalLayer` swapchain, presented Fifo (vsync).

MSAA is off, so egui renders **straight into the swapchain texture**
(`renderer.rs:264-321`). The lattice is an `egui_wgpu` paint callback: its
`prepare()` (in `crates/harmonigraph-render/src/lib.rs`) runs the scene
and bloom passes into a pane-sized offscreen texture on egui's own
encoder, and its `paint()` blits a single quad inside
egui's render pass. Node labels and the learn overlay are laid out by
`draw_lattice` (`crates/harmonigraph-ui/src/panes/lattice.rs`), which puts the
names INTO the paint callback rather than painting them after, so a node in
front covers them.

**So the composited "what you see" image exists in exactly one place: the
swapchain texture.** That single fact drives everything below.

## 2. Why the obvious insertion point isn't reachable

Two things block a readback from where you'd want to put it:

- **The surface isn't copyable.**
  `vendor/egui-baseview/src/renderer/wgpu/renderer.rs:94` configures it
  `usage = RENDER_ATTACHMENT` — no `COPY_SRC`, so
  `copy_texture_to_buffer` on the swapchain texture is a validation error.
  Metal *permits* `COPY_SRC` on a surface
  (`wgpu-hal/src/metal/adapter.rs:428`); the cost is that
  `framebufferOnly` goes false on the layer
  (`wgpu-hal/src/metal/surface.rs:73`), which is why the flag isn't on by
  default.
- **There is no `Device`/`Queue`/`RenderState` reachable from the
  editor.** egui-baseview's `Renderer` keeps them private
  (`renderer.rs:48-56`) and hands the app only a small `Queue` of window
  commands (`vendor/egui-baseview/src/window.rs:76-119`).
  `editor.rs:24-31` already documents this gap — it's the same missing
  accessor that forces the `ASSUMED_SURFACE_FORMAT` hack.

A paint callback *can* see a `Device` and `Queue`
(`prepare()`'s arguments, `harmonigraph-render/src/lib.rs`) and they're cheap clonable handles, so a
callback could stash them. But a callback's `paint()` gets only a
`&mut RenderPass` — you can't copy inside a render pass, and the
`SurfaceTexture` is never handed to callbacks. **Full-window capture has
to live in `Renderer::render`, which means a vendored egui-baseview
patch.** (The project already carries three of those; see `PATCHES.md`.)

Two smaller blockers in the same area: `egui-wgpu`'s `capture` feature —
which contains a ready-made capture texture + readback buffer + blit
(`egui-wgpu/src/capture.rs`) — **is not enabled in the plugin build**
(it is in the standalone, via eframe), and egui-baseview drops
`ViewportCommand::Screenshot` on the floor (`window.rs:403-414`), so the
standalone's self-screenshot trick is a silent no-op in the plugin.

## 3. The problem nobody thinks about until the first clip looks wrong

Frame pacing. The plugin repaints **on demand**, not at a fixed rate:
continuously only while voices are sounding or decaying, otherwise a
50 ms poll (`IDLE_REPAINT_INTERVAL` in `crates/harmonigraph-ui/src/lib.rs`), on top of baseview's
15 ms macOS frame timer (`vendor/baseview/src/platform/macos/view.rs:105`)
and a Fifo swapchain. Presents are also skipped outright on
Occluded/Outdated/Lost (`renderer.rs:214-262`) — the editor already logs
these stalls (`note_frame` in `editor.rs`).

A recorder that counts frames will therefore produce video that drifts
against the music. Any live-capture route **must** force continuous
repaint while recording, stamp each frame with a real timestamp, and let
the encoder normalize to constant frame rate. Related: the window is
HiDPI and resizable mid-recording, and H.264 wants even dimensions — the
size has to be locked (the editor can even ask the host for an exact
physical size via `requested_size` + `request_resize()`, both in
`editor.rs`).

And audio: the mono ring feeding the spectrum analyzer **drops samples
under backpressure by design** (`AUDIO_RING_CAPACITY` and the `audio_producer`
writes in `crates/harmonigraph-plugin/src/lib.rs`).
It is not a recording source. Audio comes from a DAW bounce, muxed after.

## 4. What is already built that helps

- **A self-capture recorder, 50 lines from done.**
  `SelfShot` in `crates/harmonigraph-standalone/src/main.rs` already does
  `ViewportCommand::Screenshot` → `image::save_buffer`. Its own doc
  comment states the project's stance: the app captures its own swapchain
  "so macOS screen-recording permissions never get in the way."
- **An offline render-and-readback harness.**
  `crates/harmonigraph-render/src/gpu_harness.rs` already creates a headless
  device (`headless_device`), renders to a `COPY_SRC` texture and maps the
  result back (`parity_target` and the pass around it). That is most of an
  offline renderer. Named rather than cited by line: the helpers move
  whenever a test is added above them.
- **`derive_scene` is a pure function** of
  `(tracker, tuning, view, params, camera, now)`
  (`harmonigraph_scene::derive_scene`, called from `draw_lattice` in
  `crates/harmonigraph-ui/src/panes/lattice.rs`). Step `now` by exactly
  1/60 s and you get a perfect constant-frame-rate sequence with no vsync
  coupling at all.
- **Frameless mode** (`view.frameless`, the checkbox in
  `crates/harmonigraph-ui/src/panes/system.rs`) exists specifically to
  make adjacent panes record as one clean surface.
- **The host transport is available and unused.** `nice-plug-core` exposes
  `playing`/`pos_seconds`/`tempo`; `process()`
  (in `crates/harmonigraph-plugin/src/lib.rs`) never asks for it. That's
  the natural thing to gate and stamp a recording against.

Against that: **there is no video encoder anywhere in the tree**, and no
macOS media bindings (`objc2-video-toolbox`, `objc2-av-foundation`) either.
The dependency philosophy is explicit and deliberate — `harmonigraph-core` has
an empty `[dependencies]`, guarded by `ci.sh`; `image` is justified inline
as "png-only keeps the build slim". A fat encoder crate inside the plugin
cdylib would fight that. **Piping raw frames to an `ffmpeg` subprocess
adds zero dependencies** and is the natural first sink. (If it ever needs
to be in-process, `objc2-video-toolbox` + `objc2-av-foundation` is the only
sane macOS option — hardware H.264/HEVC, and the objc2 stack is already
pulled in by baseview.)

Raw frame dumps are not a serious sink: 2000×1400 RGBA at 60 fps is
~670 MB/s, ~40 GB/minute. PNG-per-frame trades that for tens of
milliseconds of CPU per frame. Both are fallbacks, not plans.

## 5. The four routes, and which one to take

| | What it records | Effort | Quality ceiling |
|---|---|---|---|
| **0. OBS / QuickTime** | the screen | none | screen res, compositor-paced |
| **1. Standalone recorder** | the standalone window | ~1 day | high, arbitrary size |
| **2. Offline replay** | a re-render of logged events | ~2-3 days | perfect CFR, 4K, no dropped frames |
| **3. Plugin window** | exactly what was asked | ~3-5 days + a vendor patch | high, but capped by live pacing |

**Do 0 first** — one evening with OBS on the plugin window tells you
whether anything else is worth building, and it's the baseline the rest
has to beat.

**Then 1, if it isn't.** The standalone already has the `capture` feature
compiled in, already screenshots itself, has free threads, no host, and a
window size you control. Turn `SelfShot` into
`LATTICE_RECORD=out.mp4`: screenshot per frame, hand frames to a worker
thread, pipe raw to `ffmpeg -f rawvideo`. Drive it from the DAW over an
IAC MIDI bus (the standalone already enumerates MIDI ports), and port the
look you dialed in with `./read-plugin-state.py --rust`. The catch is that
the standalone's audio is `MockSynth`, so the spectrum curve won't match
the real mix — if the video leans on the spectrum, this route needs the
standalone to accept real audio input first.

**Route 2 is the one that produces the best-looking video**, because it
sidesteps pacing entirely: log `NoteEvent`s plus tuning/camera/view during
a DAW pass, then replay them offline against a headless device at exactly
1/60 s per step, at any resolution. Everything needed is in
`gpu_harness.rs`'s `headless_device` / `parity_target` plus `derive_scene`. The
missing piece is egui itself —
you'd need an offscreen `egui_wgpu::Renderer` to get the labels and pane
chrome, or accept lattice-only frames.

**Route 3 is the literal request and should be last.** It needs an
egui-baseview patch that (a) adds `COPY_SRC` to the surface while
recording — cheap, since `configure_surface` already runs every frame —
or wires up `egui-wgpu`'s `CaptureState`, and (b) exposes `RenderState`
through the `Queue` so the editor can drive it. Frames go to a worker
thread and out to ffmpeg; pool 3-4 readback buffers and memcpy padded rows
rather than using egui's per-pixel `ColorImage` conversion, whose own
comment warns it isn't built for video. The redeeming feature: exposing
`RenderState` is exactly the accessor `editor.rs:24-31` already wants, so
this patch retires the `ASSUMED_SURFACE_FORMAT` hack at the same time —
one more entry in `PATCHES.md`, paying for two things.

**Route 4, ScreenCaptureKit/AVFoundation in-process, rejects itself.** The
TCC screen-recording prompt would be attributed to the *host* — precisely
what `SelfShot`'s own doc in `harmonigraph-standalone/src/main.rs` says the
project avoids — you'd
capture host chrome around a host-owned window, and you'd still be stuck
at screen resolution and compositor pacing. Strictly worse than OBS, with
code.
