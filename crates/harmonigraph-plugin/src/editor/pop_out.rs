//! The editor's own window, so the picture stays on screen when the host is
//! not the frontmost app: egui-plugin-window's `plugin-own-window`, which
//! otonal shares. What the window does and why (its level and stacking, the
//! host's window concealed, moves deferred out of a frame, the close button
//! telling the host, where it reopens) is in that crate's documentation; this
//! is the adapter: the window's name and limits, whom its close button tells,
//! and what each move writes into [`EguiState`].
//!
//! The window is one per open editor, made in `Editor::spawn` on the main
//! thread its every method requires, owned by the editor handle and closed
//! there before baseview closes the view; the size source and the frame hold
//! it weakly.

use std::sync::Arc;

use std::ffi::c_void;

use egui_baseview::baseview::WindowHandle;
use harmonigraph_clap::prelude::EditorHost;
use plugin_own_window::{Limits, Move};
use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

pub(super) use plugin_own_window::{OwnSize, OwnWindow};

use super::persist::EguiState;
use super::{MAX_SIZE, MIN_SIZE};

/// The window's title, and the name AppKit remembers its place under, in the
/// defaults of the process the plugin runs in: a new name would forget where
/// the player put it.
pub(super) const NAME: &str = "Harmonigraph";

/// The sizes the player may give the own window, which are the editor's.
pub(super) const LIMITS: Limits = Limits { min: MIN_SIZE, max: MAX_SIZE };

/// Learn the editor's views once it has opened in the host's `NSView`
/// `host_view`, and move it into its own window unless the project keeps it
/// `docked`. Called from `Editor::spawn`, outside any frame, holding no lock
/// that `moving` or the unseen notice takes. Where baseview opened no AppKit
/// view, as off macOS, the window is never offered.
pub(super) fn open(
    own: &Arc<OwnWindow>,
    host_view: *mut c_void,
    window: &WindowHandle,
    editor: Arc<EguiState>,
    host: Arc<dyn EditorHost>,
) {
    let RawWindowHandle::AppKit(view) = window.raw_window_handle() else {
        return;
    };
    let out = !editor.docked();
    let closed = Some(Box::new(move || host.closed()) as Box<dyn Fn() -> bool>);
    // SAFETY: the host's view the editor opened in, and baseview's inside it,
    // both alive for the call.
    unsafe { own.open(host_view, view.ns_view, out, LIMITS, closed, Box::new(moving(editor))) };
}

/// What a move writes into the editor's saved state, before the view moves:
/// from a move out the editor's size is the own window's, which the project
/// saves, and a host size not yet taken is not the editor's; a host
/// `set_size` from then until the move back sizes the host's empty window
/// (`Editor::set_size`). Only `EguiState`'s atomics, and no lock, so it cannot
/// deadlock the main thread it runs on inside `open`, the deferred moves,
/// `close` and the last drop.
pub(super) fn moving(editor: Arc<EguiState>) -> impl Fn(Move) {
    move |moved| match moved {
        Move::Out(size) => {
            editor.host_resized.store(None);
            editor.size.store(size);
            editor.set_in_own_window(true);
        }
        Move::Back => editor.set_in_own_window(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A move out gives the editor its own window's size and drops a host
    /// size not yet taken; a move back hands `set_size` back to the host and
    /// keeps the size.
    #[test]
    fn a_move_out_saves_the_own_windows_size_and_a_move_back_returns_the_say_to_the_host() {
        let editor = EguiState::from_size(800, 600);
        editor.host_resized.store(Some((900, 650)));
        let moving = moving(editor.clone());

        moving(Move::Out((1000, 700)));
        assert_eq!(editor.host_resized.load(), None, "a host size outlived the move out");
        assert_eq!(editor.size(), (1000, 700), "the project does not save the own window's size");
        assert!(editor.in_own_window(), "a host resize would replace the own window's size");

        moving(Move::Back);
        assert!(!editor.in_own_window(), "docked, a host resize would be refused");
        assert_eq!(editor.size(), (1000, 700), "the move back changed the editor's size");
    }
}
