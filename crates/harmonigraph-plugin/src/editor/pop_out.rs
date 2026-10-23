//! The editor's own window, so the picture stays on screen when the host is
//! not the frontmost app.
//!
//! Bitwig takes every plugin window off screen while another app is in front:
//! the window it gives the editor sits at the floating level and is ordered
//! out on deactivation, while the plugin-host process stays un-hidden. A window
//! the plugin creates itself is one Bitwig does not know about, and it stays up
//! (checked in Bitwig: #1580). So on macOS the editor always lives in one,
//! and the host's window is left open but invisible and click-through. Hiding
//! it outright would be undone: Bitwig orders it back to the front whenever it
//! is itself brought forward.
//!
//! The live view moves rather than a second one opening, which keeps the
//! renderer and the egui context; baseview already follows its view to a new
//! window (tracking area, first responder, occlusion, and the display link,
//! which follows the screen). The move is made from `Editor::spawn`, never
//! from inside a frame: making a window key sends baseview a focus event on the
//! spot, and baseview's handler is borrowed for the whole frame, so a move from
//! a frame panics across AppKit and takes the host process down.
//!
//! What the host's window did for the editor, this one does in its place. It
//! floats above the host's windows while the host is in front, as Bitwig's
//! did, and drops to the normal level while another app is, which is where
//! Bitwig's would have vanished. It has no close button: a CLAP plugin cannot
//! ask its host to close the editor, so opening and closing stay the host's.
//!
//! The view's size is the size source's to set, as for a host resize: the
//! size source asks [`PopOut::take_size`] first, at the top of the frame it
//! will be laid out in. Every AppKit call here is on the main thread, which is
//! where egui-baseview runs its size source and its frames, and where the CLAP
//! host creates and destroys the editor.

use std::sync::Arc;

use nice_plug::prelude::ParentWindowHandle;

#[cfg(target_os = "macos")]
pub(super) use macos::PopOut;

#[cfg(not(target_os = "macos"))]
pub(super) use elsewhere::PopOut;

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    use std::path::{Path, PathBuf};
    use std::sync::Weak;

    use baseview::WindowHandle;
    use objc2::rc::Retained;
    use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
    use objc2_app_kit::{
        NSBackingStoreType, NSView, NSWindow, NSWindowLevel, NSWindowStyleMask, NSWorkspace,
        NSWorkspaceDidActivateApplicationNotification,
    };
    use objc2_foundation::{
        MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
        NSString,
    };
    use parking_lot::Mutex;
    use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

    /// The level Bitwig gives its plugin windows, measured (CG layer 3), and
    /// `kCGFloatingWindowLevel`: above the host's own windows.
    const FLOATING: NSWindowLevel = 3;
    /// `kCGNormalWindowLevel`.
    const NORMAL: NSWindowLevel = 0;

    /// Where AppKit keeps the window's frame between opens, in the defaults of
    /// the process the plugin runs in.
    const AUTOSAVE: &str = "Harmonigraph";

    /// See the module documentation. Shared between the size source and the
    /// editor handle, both of which egui-baseview or nice-plug require to be
    /// `Send`.
    pub(in crate::editor) struct PopOut(Mutex<State>);

    // SAFETY: the AppKit objects inside are only touched from the main thread:
    // every method takes a `MainThreadMarker` before reaching them, and panics
    // without one rather than racing.
    unsafe impl Send for PopOut {}
    unsafe impl Sync for PopOut {}

    #[derive(Default)]
    struct State {
        /// The host's view, which the editor's view lives in until it moves
        /// and goes back to at close.
        host: Option<Retained<NSView>>,
        /// baseview's view, the one that moves.
        view: Option<Retained<NSView>>,
        /// The editor's own window, once the view is in it.
        window: Option<Retained<NSWindow>>,
        /// Hears which app comes to the front, to set the window's level.
        observer: Option<Retained<Observer>>,
        /// The content size last given to the view, in logical points.
        size: (u32, u32),
    }

    fn main_thread() -> MainThreadMarker {
        MainThreadMarker::new().expect("the editor window is only reached from the main thread")
    }

    fn ns_view(handle: RawWindowHandle) -> Option<Retained<NSView>> {
        let RawWindowHandle::AppKit(handle) = handle else { return None };
        // SAFETY: an AppKit handle's `ns_view` is an NSView, alive for the
        // duration of this call; retaining it keeps it alive after.
        unsafe { Retained::retain(handle.ns_view.cast::<NSView>()) }
    }

    fn logical(size: NSSize) -> (u32, u32) {
        (size.width.round().max(1.0) as u32, size.height.round().max(1.0) as u32)
    }

    /// Whether the frontmost app is the host: the app whose bundle this
    /// process runs from. That is the DAW itself for a host that loads
    /// plugins in-process, and still the DAW for Bitwig's sandboxed plugin
    /// host, which runs from inside `Bitwig Studio.app`; the editor's own
    /// process counts too, since it is in front while its window is clicked.
    fn host_in_front(exe: &Path) -> bool {
        let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
            return true;
        };
        let bundle = app.bundleURL().and_then(|url| url.path());
        bundle.is_some_and(|bundle| exe.starts_with(PathBuf::from(bundle.to_string())))
    }

    /// The level for whichever app is now in front.
    fn level(exe: &Path) -> NSWindowLevel {
        if host_in_front(exe) {
            FLOATING
        } else {
            NORMAL
        }
    }

    /// Keep the host's window out of sight and out of the way, but open:
    /// closing or hiding it is the host's to undo, and Bitwig does.
    fn conceal(window: &NSWindow) {
        if window.alphaValue() != 0.0 {
            window.setAlphaValue(0.0);
        }
        if !window.ignoresMouseEvents() {
            window.setIgnoresMouseEvents(true);
        }
    }

    impl PopOut {
        pub(in crate::editor) fn new() -> Arc<Self> {
            Arc::new(Self(Mutex::new(State::default())))
        }

        /// Move the editor, just opened in the host's view, into its own
        /// window of `size`. Called from `Editor::spawn`, which is outside
        /// any frame (see the module documentation).
        pub(in crate::editor) fn open(
            self: &Arc<Self>,
            parent: ParentWindowHandle,
            window: &WindowHandle,
            size: (u32, u32),
        ) {
            let mtm = main_thread();
            let ParentWindowHandle::AppKitNsView(host) = parent else { return };
            let mut handle = raw_window_handle::AppKitWindowHandle::empty();
            handle.ns_view = host;
            let (Some(host), Some(view)) =
                (ns_view(RawWindowHandle::AppKit(handle)), ns_view(window.raw_window_handle()))
            else {
                return;
            };
            let exe = std::env::current_exe().unwrap_or_default();

            let content = NSSize::new(f64::from(size.0), f64::from(size.1));
            let style = NSWindowStyleMask::Titled
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable;
            // SAFETY: a plain window, created and owned on the main thread.
            let own = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    NSRect::new(NSPoint::ZERO, content),
                    style,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            // SAFETY: `State` owns the window, so AppKit must not release it
            // on close as well.
            unsafe { own.setReleasedWhenClosed(false) };
            // Already NO for a plain NSWindow; said anyway, since it is the
            // whole difference from the host's window.
            own.setHidesOnDeactivate(false);
            own.setTitle(&NSString::from_str("Harmonigraph"));
            let min = super::super::MIN_SIZE;
            own.setContentMinSize(NSSize::new(f64::from(min.0), f64::from(min.1)));
            own.setLevel(level(&exe));

            // Where it was last time, at the size the project saved. The
            // first time, where the host's window is.
            let name = NSString::from_str(AUTOSAVE);
            if own.setFrameUsingName(&name) {
                let frame = own.frame();
                let top_left = NSPoint::new(frame.origin.x, frame.origin.y + frame.size.height);
                own.setContentSize(content);
                own.setFrameTopLeftPoint(top_left);
            } else if let Some(host_window) = host.window() {
                let frame = host_window.frame();
                own.setFrameTopLeftPoint(NSPoint::new(
                    frame.origin.x,
                    frame.origin.y + frame.size.height,
                ));
            } else {
                own.center();
            }
            // Refused for a second open editor in the same process, which then
            // just goes unremembered.
            own.setFrameAutosaveName(&name);

            let observer =
                Observer::alloc(mtm).set_ivars(Watch { pop_out: Arc::downgrade(self), exe });
            // SAFETY: NSObject's init on a freshly allocated subclass instance.
            let observer: Retained<Observer> = unsafe { msg_send![super(observer), init] };
            // SAFETY: `activated:` takes the notification; the observer is
            // removed in `close`, before it can be freed.
            unsafe {
                NSWorkspace::sharedWorkspace()
                    .notificationCenter()
                    .addObserver_selector_name_object(
                        &observer,
                        sel!(activated:),
                        Some(NSWorkspaceDidActivateApplicationNotification),
                        None,
                    );
            }

            // The move. Made without the lock, as every move here is:
            // attaching a view can draw a frame on the spot, and its size
            // source takes the lock.
            let Some(own_content) = own.contentView() else { return };
            view.removeFromSuperview();
            own_content.addSubview(&view);
            view.setFrameOrigin(NSPoint::ZERO);
            if let Some(host_window) = host.window() {
                conceal(&host_window);
            }
            own.makeKeyAndOrderFront(None);

            *self.0.lock() = State {
                host: Some(host),
                view: Some(view),
                window: Some(own),
                observer: Some(observer),
                size: logical(content),
            };
        }

        /// The size of the editor's own window, while it is in one, which
        /// then has the say over the editor's size rather than the host.
        pub(in crate::editor) fn size(&self) -> Option<(u32, u32)> {
            let state = self.0.lock();
            state.window.as_ref().map(|_| state.size)
        }

        /// The size source's question, asked before the host's: the size the
        /// view has to take this frame, if its window has been resized.
        ///
        /// Also where the host's window is kept concealed, should the host
        /// have put it back, or have shown it only after the editor opened.
        pub(in crate::editor) fn take_size(&self) -> Option<(u32, u32)> {
            let _ = main_thread();
            let mut state = self.0.lock();
            if let Some(host_window) = state.host.as_ref().and_then(|host| host.window()) {
                conceal(&host_window);
            }
            let content = state.window.as_ref()?.contentView()?;
            let size = logical(content.frame().size);
            (size != state.size).then(|| {
                state.size = size;
                size
            })
        }

        /// A size the editor asked for itself (a pane folding sideways),
        /// given to its own window instead of to the host. Returns whether it
        /// was.
        pub(in crate::editor) fn resize(&self, size: (u32, u32)) -> bool {
            let _ = main_thread();
            let window = {
                let mut state = self.0.lock();
                let Some(window) = state.window.clone() else { return false };
                state.size = size;
                window
            };
            window.setContentSize(NSSize::new(f64::from(size.0), f64::from(size.1)));
            true
        }

        /// The editor closing, called before baseview closes the view: put the
        /// view back in the host's window, shown again, close the editor's
        /// own, and let go of everything. The last is not tidiness: the view
        /// owns the size source, which owns this, so held views would keep the
        /// whole editor alive after it closed.
        pub(in crate::editor) fn close(&self) {
            let _ = main_thread();
            let state = std::mem::take(&mut *self.0.lock());
            if let Some(observer) = &state.observer {
                // SAFETY: removing an observer this added, still alive.
                unsafe {
                    NSWorkspace::sharedWorkspace().notificationCenter().removeObserver(observer)
                };
            }
            if let (Some(host), Some(view)) = (&state.host, &state.view) {
                view.removeFromSuperview();
                host.addSubview(view);
                view.setFrameOrigin(NSPoint::ZERO);
                if let Some(host_window) = host.window() {
                    host_window.setIgnoresMouseEvents(false);
                    host_window.setAlphaValue(1.0);
                }
            }
            if let Some(window) = &state.window {
                window.close();
            }
        }

        fn front_changed(&self, exe: &Path) {
            let window = self.0.lock().window.clone();
            if let Some(window) = window {
                window.setLevel(level(exe));
            }
        }
    }

    struct Watch {
        pop_out: Weak<PopOut>,
        exe: PathBuf,
    }

    define_class!(
        // NSObject adds no subclass invariants; the observer is main-thread
        // only, as workspace notifications are delivered there.
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = Watch]
        struct Observer;

        unsafe impl NSObjectProtocol for Observer {}

        impl Observer {
            /// Another app came to the front, or the host did.
            #[unsafe(method(activated:))]
            fn activated(&self, _: &NSNotification) {
                let watch = self.ivars();
                if let Some(pop_out) = watch.pop_out.upgrade() {
                    pop_out.front_changed(&watch.exe);
                }
            }
        }
    );
}

#[cfg(not(target_os = "macos"))]
mod elsewhere {
    use super::*;

    /// Off macOS the editor stays in the host's window: the hosts there have
    /// not been seen hiding their plugin windows, and nothing here has been
    /// tried on them.
    pub(in crate::editor) struct PopOut;

    impl PopOut {
        pub(in crate::editor) fn new() -> Arc<Self> {
            Arc::new(Self)
        }
        pub(in crate::editor) fn open(
            self: &Arc<Self>,
            _: ParentWindowHandle,
            _: &baseview::WindowHandle,
            _: (u32, u32),
        ) {
        }
        pub(in crate::editor) fn size(&self) -> Option<(u32, u32)> {
            None
        }
        pub(in crate::editor) fn take_size(&self) -> Option<(u32, u32)> {
            None
        }
        pub(in crate::editor) fn resize(&self, _: (u32, u32)) -> bool {
            false
        }
        pub(in crate::editor) fn close(&self) {}
    }
}
