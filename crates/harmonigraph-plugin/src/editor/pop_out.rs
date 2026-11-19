//! The editor's own window, so the picture stays on screen when the host is
//! not the frontmost app: egui-plugin-window's `plugin-own-window`, which
//! otonal shares. What the window does and why (its level and stacking, the
//! host's window concealed, moves deferred out of a frame, the close button
//! telling the host, where it reopens) is in that crate's documentation, and
//! its size, which it shares with the host's window, is that crate's
//! `EditorSize`, kept in [`EguiState`](super::EguiState). What is left here is
//! the window's name.
//!
//! The window is one per open editor, opened by `EditorSize::attach` in
//! `Editor::spawn` on the main thread its every method requires, and closed by
//! `EditorSize::close` in the editor handle's drop, before baseview closes the
//! view.

/// The window's title, and the name AppKit remembers its place under, in the
/// defaults of the process the plugin runs in: a new name would forget where
/// the player put it.
pub(super) const NAME: &str = "Harmonigraph";
