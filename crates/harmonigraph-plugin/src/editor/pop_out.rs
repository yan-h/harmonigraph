//! The editor moved out of the host's window into one of its own, so it stays
//! on screen when the host is not the frontmost app.
//!
//! Bitwig takes every plugin window off screen while it is in the background:
//! the window it gives the editor sits at the floating level and is ordered
//! out on deactivation, while the plugin-host process stays un-hidden. A
//! window the plugin creates itself is one Bitwig does not know about, so it
//! should stay up. Moving the live view, rather than opening a second one,
//! keeps the renderer, the egui context and everything they hold; baseview
//! already follows its view to a new window (tracking area, first responder,
//! occlusion, and the display link, which follows the screen).
//!
//! The move itself is never made inside a frame. Making the new window key
//! sends baseview a focus event on the spot, and baseview's handler is
//! borrowed for the whole frame, so a move from the frame that asked for it
//! panics across AppKit and takes the host process down. A request is
//! performed instead on the run loop's next turn ([`PopOut::request`]).
//!
//! The view's size is the size source's to set, as for a host resize: the
//! size source asks [`PopOut::take_size`] first, at the top of the frame it
//! will be laid out in. Every AppKit call here is on the main thread, which is
//! where egui-baseview runs its size source and its frames, and where the
//! CLAP host destroys the editor.

use std::sync::Arc;

use nice_plug::prelude::ParentWindowHandle;

#[cfg(target_os = "macos")]
pub(super) use macos::PopOut;

#[cfg(not(target_os = "macos"))]
pub(super) use elsewhere::PopOut;

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    use baseview::WindowHandle;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSBackingStoreType, NSTextField, NSView, NSWindow,
        NSWindowDelegate, NSWindowStyleMask,
    };
    use objc2_foundation::{
        MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize,
        NSString,
    };
    use parking_lot::Mutex;
    use raw_window_handle::{HasRawWindowHandle, RawWindowHandle};

    /// See the module documentation. Shared between the size source, the
    /// frame and the editor handle, all of which egui-baseview or nice-plug
    /// require to be `Send`.
    pub(in crate::editor) struct PopOut(Mutex<State>);

    // SAFETY: the AppKit objects inside are only touched from the main thread:
    // every method takes a `MainThreadMarker` before reaching them, and panics
    // without one rather than racing.
    unsafe impl Send for PopOut {}
    unsafe impl Sync for PopOut {}

    #[derive(Default)]
    struct State {
        /// The host's view, which the editor's view lives in while docked.
        host: Option<Retained<NSView>>,
        /// baseview's view, the one that moves.
        view: Option<Retained<NSView>>,
        /// The pop-out window's delegate, and where a requested move is
        /// performed from, outside the frame that asked for it.
        delegate: Option<Retained<Delegate>>,
        out: Option<Out>,
        /// What the UI asked for and the run loop has not yet done.
        wanted: Option<bool>,
        /// Docked since the last size source, which owes the view the host's
        /// size back.
        redocked: bool,
    }

    struct Out {
        window: Retained<NSWindow>,
        /// Left in the host's window, saying where the editor went.
        note: Retained<NSTextField>,
        /// The content size last given to the view, in logical points.
        size: (u32, u32),
    }

    fn main_thread() -> MainThreadMarker {
        MainThreadMarker::new().expect("the pop-out is only reached from the main thread")
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

    impl PopOut {
        pub(in crate::editor) fn new() -> Arc<Self> {
            Arc::new(Self(Mutex::new(State::default())))
        }

        /// Learn the two views once the editor is open: the host's, and the
        /// one baseview put inside it.
        pub(in crate::editor) fn attach(
            self: &Arc<Self>,
            parent: ParentWindowHandle,
            window: &WindowHandle,
        ) {
            let mtm = main_thread();
            let ParentWindowHandle::AppKitNsView(host) = parent else { return };
            let mut handle = raw_window_handle::AppKitWindowHandle::empty();
            handle.ns_view = host;
            let delegate = Delegate::alloc(mtm).set_ivars(Arc::downgrade(self));
            // SAFETY: NSObject's init on a freshly allocated subclass instance.
            let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
            let mut state = self.0.lock();
            state.host = ns_view(RawWindowHandle::AppKit(handle));
            state.view = ns_view(window.raw_window_handle());
            state.delegate = Some(delegate);
        }

        /// Whether the editor is out, or `None` where it cannot be.
        pub(in crate::editor) fn is_out(&self) -> Option<bool> {
            let state = self.0.lock();
            (state.host.is_some() && state.view.is_some())
                .then(|| state.wanted.unwrap_or(state.out.is_some()))
        }

        /// Ask for the editor in or out, done on the run loop's next turn:
        /// asked from a frame, which is no place to move it (see the module
        /// documentation).
        pub(in crate::editor) fn request(&self, out: bool) {
            let _ = main_thread();
            let delegate = {
                let mut state = self.0.lock();
                state.wanted = Some(out);
                state.delegate.clone()
            };
            if let Some(delegate) = delegate {
                // SAFETY: `apply:` is the delegate's own method, taking an
                // object it ignores; the run loop retains the target until
                // then.
                let () = unsafe {
                    msg_send![
                        &*delegate,
                        performSelector: sel!(apply:),
                        withObject: None::<&AnyObject>,
                        afterDelay: 0.0f64
                    ]
                };
            }
        }

        /// The requested move, off the run loop.
        fn apply(&self) {
            let mtm = main_thread();
            let wanted = self.0.lock().wanted.take();
            match wanted {
                Some(true) => self.pop(mtm),
                Some(false) => self.take_back(true),
                None => {}
            }
        }

        /// The window's size while it is out, which is then the editor's.
        pub(in crate::editor) fn size(&self) -> Option<(u32, u32)> {
            self.0.lock().out.as_ref().map(|out| out.size)
        }

        /// The size source's question, asked before the host's: the size
        /// the view has to take this frame, if this decides it. `docked` is
        /// the host's size, which the view goes back to.
        pub(in crate::editor) fn take_size(&self, docked: (u32, u32)) -> Option<(u32, u32)> {
            let _ = main_thread();
            let mut state = self.0.lock();
            if std::mem::take(&mut state.redocked) {
                return Some(docked);
            }
            let out = state.out.as_mut()?;
            let content = out.window.contentView()?;
            let size = logical(content.frame().size);
            (size != out.size).then(|| {
                out.size = size;
                size
            })
        }

        /// A size the editor asked for itself (a pane folding sideways),
        /// given to the window while it is out instead of to the host.
        /// Returns whether it was.
        pub(in crate::editor) fn resize(&self, size: (u32, u32)) -> bool {
            let _ = main_thread();
            let window = {
                let mut state = self.0.lock();
                let Some(out) = state.out.as_mut() else { return false };
                out.size = size;
                out.window.clone()
            };
            window.setContentSize(NSSize::new(f64::from(size.0), f64::from(size.1)));
            true
        }

        fn pop(&self, mtm: MainThreadMarker) {
            let (host, view, delegate) = {
                let state = self.0.lock();
                let (None, Some(host), Some(view), Some(delegate)) =
                    (&state.out, &state.host, &state.view, &state.delegate)
                else {
                    return;
                };
                (host.clone(), view.clone(), delegate.clone())
            };
            let size = view.frame().size;
            let style = NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable;
            // SAFETY: a plain window, created and owned on the main thread.
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm),
                    NSRect::new(NSPoint::ZERO, size),
                    style,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            // SAFETY: `Out` owns the window, so AppKit must not release it on
            // close as well.
            unsafe { window.setReleasedWhenClosed(false) };
            // The point of the window. Already NO for a plain NSWindow; said
            // anyway, since it is exactly what Bitwig's floating window is not.
            window.setHidesOnDeactivate(false);
            window.setTitle(&NSString::from_str("Harmonigraph"));
            let min = super::super::MIN_SIZE;
            window.setContentMinSize(NSSize::new(f64::from(min.0), f64::from(min.1)));
            match host.window() {
                // Just inside the host window's top-left, so it opens where
                // the editor was rather than wherever the screen's centre is.
                Some(host_window) => {
                    let frame = host_window.frame();
                    window.setFrameTopLeftPoint(NSPoint::new(
                        frame.origin.x + 32.0,
                        frame.origin.y + frame.size.height - 32.0,
                    ));
                }
                None => window.center(),
            }

            window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

            let note = NSTextField::wrappingLabelWithString(
                &NSString::from_str(
                    "Harmonigraph is in its own window. Close that window, or untick \
                     Pop out in System, to bring it back here.",
                ),
                mtm,
            );
            let bounds = host.bounds();
            note.setFrame(NSRect::new(
                NSPoint::new(24.0, (bounds.size.height - 64.0).max(0.0)),
                NSSize::new((bounds.size.width - 48.0).max(1.0), 40.0),
            ));
            note.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewMinYMargin,
            );

            // The move: baseview moves its tracking area and first responder
            // with the view, and its display link follows it to the new screen.
            // Made without the lock, as every move here is: attaching a view
            // can draw a frame on the spot, and its size source takes the lock
            // (here, inside a frame already, baseview declines that one).
            let Some(content) = window.contentView() else { return };
            view.removeFromSuperview();
            content.addSubview(&view);
            view.setFrameOrigin(NSPoint::ZERO);
            host.addSubview(&note);
            window.makeKeyAndOrderFront(None);
            self.0.lock().out = Some(Out { window, note, size: logical(size) });
        }

        /// The editor closing, called before baseview closes the view: put
        /// the view back in the host's window, close the pop-out so it never
        /// outlives the editor, and let go of both views. The last is not
        /// tidiness: the view owns the size source, which owns this, so held
        /// views would keep the whole editor alive after it closed.
        pub(in crate::editor) fn close(&self) {
            self.take_back(true);
            let held = {
                let mut state = self.0.lock();
                (state.host.take(), state.view.take(), state.delegate.take())
            };
            drop(held);
        }

        /// The docking itself; `close` is false when the window is closing
        /// already, from its own close button.
        fn take_back(&self, close: bool) {
            let _ = main_thread();
            let (out, views) = {
                let mut state = self.0.lock();
                let Some(out) = state.out.take() else { return };
                // Before the move, which can draw a frame on the spot (from a
                // close button or the editor's teardown, outside any frame):
                // that frame's size source is where the view gets the host's
                // size back, so it must already find it owed.
                state.redocked = true;
                (out, state.host.clone().zip(state.view.clone()))
            };
            if let Some((host, view)) = views {
                view.removeFromSuperview();
                host.addSubview(&view);
                view.setFrameOrigin(NSPoint::ZERO);
            }
            out.note.removeFromSuperview();
            // Closing notifies the delegate, which would dock again; nothing is
            // out any more, but it is cleared rather than relied on.
            out.window.setDelegate(None);
            if close {
                out.window.close();
            }
        }
    }

    define_class!(
        // NSObject adds no subclass invariants; the delegate is main-thread
        // only, as every window delegate is.
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = std::sync::Weak<PopOut>]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl NSWindowDelegate for Delegate {
            /// The window's close button: the editor goes back to the host.
            #[unsafe(method(windowWillClose:))]
            fn window_will_close(&self, _: &NSNotification) {
                if let Some(pop_out) = self.ivars().upgrade() {
                    pop_out.take_back(false);
                }
            }
        }

        impl Delegate {
            /// A move [`PopOut::request`] deferred to here.
            #[unsafe(method(apply:))]
            fn apply(&self, _: Option<&AnyObject>) {
                if let Some(pop_out) = self.ivars().upgrade() {
                    pop_out.apply();
                }
            }
        }
    );
}

#[cfg(not(target_os = "macos"))]
mod elsewhere {
    use super::*;

    /// Off macOS there is no pop-out: the hosts there have not been seen
    /// hiding their plugin windows, and nothing here has been tried on them.
    pub(in crate::editor) struct PopOut;

    impl PopOut {
        pub(in crate::editor) fn new() -> Arc<Self> {
            Arc::new(Self)
        }
        pub(in crate::editor) fn attach(
            self: &Arc<Self>,
            _: ParentWindowHandle,
            _: &baseview::WindowHandle,
        ) {
        }
        pub(in crate::editor) fn is_out(&self) -> Option<bool> {
            None
        }
        pub(in crate::editor) fn request(&self, _: bool) {}
        pub(in crate::editor) fn size(&self) -> Option<(u32, u32)> {
            None
        }
        pub(in crate::editor) fn take_size(&self, _: (u32, u32)) -> Option<(u32, u32)> {
            None
        }
        pub(in crate::editor) fn resize(&self, _: (u32, u32)) -> bool {
            false
        }
        pub(in crate::editor) fn close(&self) {}
    }
}
