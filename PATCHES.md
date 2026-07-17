# Carried patches against upstream dependencies

Two dependencies carry local patches. Keep this file current when bumping
either; both diffs are small and deliberately minimal.

## baseview — vendored at `vendor/baseview/`

- **Upstream base**: `RustAudio/baseview` @ `237d323c729f3aa99476ba3efa50129c5e86cad3`
  (the revision egui-baseview pins; wired in via `[patch]` in the workspace
  `Cargo.toml`).
- **Patch** (3 lines, `src/macos/window.rs`): register the macOS frame timer
  in `kCFRunLoopCommonModes` instead of `kCFRunLoopDefaultMode`. Default-mode
  timers stop firing while the run loop is in an event-tracking mode (native
  drags/resizes, menu tracking), freezing plugin GUIs until mouse release.
- **Upgrade**: re-apply the three `kCFRunLoop*` lines onto the new revision.
- **Upstreaming**: good candidate; uncontroversial fix, helps every
  baseview-based plugin.

## nih-plug — fork at `yan-h/nih-plug`, branch `host-window-resize`

- **Upstream base**: `robbert-vdh/nih-plug` @ `f36931f7af4646065488a9845d8f8c2f95252c23`.
- **Consumed as**: git dependency on the fork (pinned rev in workspace
  `Cargo.toml`). `nih_plug_xtask` stays on upstream (no divergence).
- **Patch** (1 commit): implements the upstream `TODO: Host->Plugin resizing`.
  - `Editor::set_size(width, height) -> bool`, defaulted to reject (old
    behavior). Same unscaled-logical-pixel convention as `Editor::size()`.
  - Wrappers advertise resizability by probing `set_size` with the current
    size (a no-op resize that implementations must accept without side
    effects).
  - CLAP: `can_resize`, `adjust_size`, `set_size`; VST3: `canResize`,
    `onSize`, `checkSizeConstraint`.
- **Upgrade**: `git rebase` the branch onto the new upstream rev, re-pin. If
  upstream has implemented host→plugin resizing itself, drop the patch and
  migrate our editor to their API.
- **Upstreaming**: worth offering, but resize API design is opinionated
  territory; expect discussion. The probe-based resizability detection is
  the part most likely to get bikeshedded.
