//! The plugin's editor and the host's side of it. The shell embeds an editor
//! only in an AppKit view (CLAP's `cocoa` window API), so a parent is an
//! `NSView` and a size is in logical points, which macOS scales itself.
//! Every call either way is on the main thread, where CLAP's `gui` calls
//! arrive; the shell owns no event loop, so this cannot be checked here.

use std::any::Any;
use std::ffi::c_void;

/// A plugin's editor ([`Plugin::editor`][crate::plugin::Plugin::editor]),
/// which the host may attach and destroy any number of times.
pub trait Editor: Send + Sync {
    /// Open the editor under the host's `NSView` `parent`. The handle
    /// returned closes it when dropped, which the host's destroy does; the
    /// shell drops one before it asks for the next.
    ///
    /// # Safety
    ///
    /// `parent` is a live `NSView`, and the call is on the main thread.
    unsafe fn spawn(&self, parent: *mut c_void) -> Box<dyn Any + Send>;

    /// The size the host is told, in logical points.
    fn size(&self) -> (u32, u32);

    /// Whether the host may resize the editor: freely, on both axes.
    fn resizable(&self) -> bool {
        false
    }

    /// The nearest size to `width` by `height` logical points the editor
    /// would take, for the host to size its window to as the player drags
    /// its border: CLAP's `gui.adjust_size`. A host asks only a resizable
    /// editor; by default every size is.
    fn adjust_size(&self, width: u32, height: u32) -> (u32, u32) {
        (width, height)
    }

    /// The host sized the editor's window to `width` by `height` logical
    /// points: whether the editor took it. A fixed editor refuses.
    fn set_size(&self, _width: u32, _height: u32) -> bool {
        false
    }
}

/// The host's side of the editor, given to the plugin before its editor
/// ([`Plugin::editor_host`][crate::plugin::Plugin::editor_host]). Main thread
/// only. It holds the shell weakly: once the plugin is destroyed every call
/// does nothing, and those that answer answer false.
pub trait EditorHost: Send + Sync {
    /// `clap_host_gui::closed(was_destroyed: true)`: tell the host the user
    /// closed the editor's window, for the host to destroy the editor, which
    /// it may do during the call. False where the host has no gui extension.
    fn closed(&self) -> bool;

    /// Ask the host to resize the editor's window to `size`, in logical
    /// points: whether it agreed. The host may call
    /// [`Editor::set_size`] and [`Editor::size`] during the call.
    fn request_resize(&self, size: (u32, u32)) -> bool;

    /// Begin a gesture on the parameter table's `row`: the host records the
    /// values set until [`Self::end_set_parameter`] as one edit.
    fn begin_set_parameter(&self, row: usize);

    /// Set the parameter table's `row` so it shows `shown` (an integer or a
    /// variant's index as a float), and tell the host. A modulated row is
    /// read with its modulation, so `shown` includes it and the value set is
    /// `shown` less the modulation's present shift.
    fn set_parameter(&self, row: usize, shown: f32);

    /// End the gesture [`Self::begin_set_parameter`] began on `row`.
    fn end_set_parameter(&self, row: usize);
}
