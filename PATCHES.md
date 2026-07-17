# Carried patches against upstream dependencies

One dependency carries a local patch. Keep this file current when bumping it.

## baseview — vendored at `vendor/baseview/`

- **Upstream base**: `baseview 0.1.4` from crates.io (the version
  egui-baseview 0.3 pins; wired in via `[patch.crates-io]` in the workspace
  `Cargo.toml`, which covers both our direct dependency and egui-baseview's).
- **Patch** (3 lines, `src/wrappers/appkit/timer.rs`): register the macOS
  frame timer in `kCFRunLoopCommonModes` instead of `kCFRunLoopDefaultMode`.
  Default-mode timers stop firing while the run loop is in an event-tracking
  mode (native drags/resizes, menu tracking), freezing plugin GUIs until
  mouse release.
- **Upgrade**: download the new crates.io tarball into `vendor/baseview`,
  re-apply the `kCFRunLoop*` lines.
- **Upstreaming**: good candidate; uncontroversial fix, helps every
  baseview-based plugin. baseview and nice-plug are both RustAudio projects,
  so the fix would land in exactly the stack this plugin uses.

## Historical: nih-plug fork (retired)

Before migrating to nice-plug (which supports host→plugin window resizing
natively via `ResizeHint`/`Editor::set_size`), this project carried a
nih-plug fork implementing that feature: `yan-h/nih-plug`, branch
`host-window-resize`. The branch is kept for reference but is no longer a
dependency.
