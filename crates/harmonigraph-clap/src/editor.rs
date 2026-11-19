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
    fn spawn(&self, parent: *mut c_void) -> Box<dyn Any + Send>;

    /// The size the host is told, in logical points.
    fn size(&self) -> (u32, u32);

    /// Whether the host may resize the editor: freely, on both axes.
    fn resizable(&self) -> bool {
        false
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

    /// Set the parameter table's `row` to the plain value `plain` (an integer
    /// or a variant's index as a float), and tell the host.
    fn set_parameter(&self, row: usize, plain: f32);

    /// End the gesture [`Self::begin_set_parameter`] began on `row`.
    fn end_set_parameter(&self, row: usize);
}
