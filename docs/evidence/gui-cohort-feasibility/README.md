# Published GUI cohort probe for #968

This is the exact scratch harness used for the 2026-09-19 feasibility investigation,
plus a logging-only patch against the checksum-verified published `egui-baseview 0.7.2` archive.
Run on macOS with a graphical session and an already installed Rust 1.95+ toolchain.
It opens its own temporary native parent/child windows;
it does not load a plugin or interact with the DAW.
It requests window closure after 12 seconds and has a 15-second process watchdog.
A zero exit code is **not** a passing rendering assertion.

Copy this evidence directory to a disposable directory under `/tmp`,
then run from that copy:

```sh
curl --fail --location https://static.crates.io/crates/egui-baseview/egui-baseview-0.7.2.crate --output egui-baseview-0.7.2.crate
printf '%s  %s\n' f197eade3efd9bfe76b9cbf8357beb6a48e0a2ac262901b0f52c736e2710fdaf egui-baseview-0.7.2.crate | shasum -a 256 --check -
tar -xzf egui-baseview-0.7.2.crate
patch -p1 < instrumentation.diff
RUSTC_WRAPPER='' cargo +1.95.0 run -j2 --manifest-path probe/Cargo.toml > probe.log 2>&1
```

Substitute your installed toolchain selector if using a newer Rust release.
The recipe does not install or change any toolchain.
Keep this scratch directory outside the product checkout.
The patch was independently applied to a fresh archive extraction and its resulting renderer compared byte-for-byte with the measured instrumented source.

`probe/Cargo.toml` pins egui to `=0.36.2` and loads the exact archive by path;
the archive's transitive requirements remain unchanged.
A lockfile is intentionally not bundled.
The measured lockfile SHA-256 was:

```text
90d8e6984a743490c982c87c57313a02636804283ee2a65adf490bdd21ac555f
```

The measured resolved cohort was egui-baseview 0.7.2,
baseview 0.3.4,
egui/egui-wgpu 0.36.2,
and wgpu/wgpu-core/wgpu-hal/naga 30.0.1.
A fresh resolution can select newer compatible transitive releases;
record the generated lockfile if comparing results.
The original exact lock remains in `/tmp/gui-feasibility-968/probe/Cargo.lock` for this session,
not as a durable external prerequisite.

The harness checks these paths through the real published stack:

- A parented child opens with 1.5× egui zoom, requests a logical InnerSize resize, then receives an external physical resize.
- After initial painting becomes idle, a worker changes fonts and creates an image without waking the UI; two seconds later it explicitly requests repaint.
- A subsequent image change requests a repaint after 500 ms.
- Instrumentation prints real texture-update attempts, surface-acquire refusals and successful present calls.

In the measured run, native display scale was 2.0,
so total pixels per egui point was 3.0.
Initial 300×160 points produced 900×480 physical pixels;
InnerSize(320,180) produced 960×540;
external PhysicalSize(1080,600) produced 360×200 egui points on the next UI call.
The idle mutation left UI count unchanged at three for two seconds;
explicit wake delivered the pending image and rebuilt atlas to the renderer,
and delayed wake delivered the second image update.

**All eight surface acquisitions returned Occluded; no PRESENT occurred.** `GPU_UPLOAD` precedes the actual `update_texture` call;
a subsequent acquisition log proves the call returned,
not that queued GPU writes were submitted or displayed.
This supports the unit-conversion and pending-delta findings but does not establish visible glyph/image correctness or complete retirement of the texture patch.
The first Context::content_rect diagnostic also showed a transient oversized rectangle before later frames matched the requested size;
first-frame layout and the native loop's close behavior were not qualified.
No behavioral renderer fixes were applied to hide these limits.
