use std::cell::Cell;
use std::rc::Rc;

use objc2::rc::{autoreleasepool, Weak};
use objc2::runtime::NSObjectProtocol;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSPasteboard, NSPasteboardTypeString,
};
use objc2_foundation::NSString;
use raw_window_handle::{
    AppKitDisplayHandle, AppKitWindowHandle, HasRawDisplayHandle, HasRawWindowHandle,
    RawDisplayHandle, RawWindowHandle,
};

use super::cursor::Cursor;
use crate::{MouseCursor, Size, WindowHandler, WindowInfo, WindowOpenOptions};

#[cfg(feature = "opengl")]
use crate::gl::GlContext;
use crate::platform::macos::view::{BaseviewView, ViewParentingType};
use crate::wrappers::appkit::{create_window, extract_raw_window_handle, View, ViewRef};

pub struct WindowHandle {
    view: Option<Weak<View<BaseviewView>>>,
    state: Rc<WindowSharedState>,
}

impl WindowHandle {
    pub fn close(&mut self) {
        let Some(view) = self.view.take().and_then(|w| w.load()) else {
            return;
        };

        BaseviewView::close(view.inner_ref());
    }

    pub fn is_open(&self) -> bool {
        self.state.closed.get()
    }
}

unsafe impl HasRawWindowHandle for WindowHandle {
    fn raw_window_handle(&self) -> RawWindowHandle {
        let Some(view) = self.view.as_ref().and_then(|w| w.load()) else {
            return AppKitWindowHandle::empty().into();
        };

        view.raw_window_handle()
    }
}

pub struct Window<'a> {
    view: &'a View<BaseviewView>,
    inner: &'a BaseviewView,
}

impl<'a> From<ViewRef<'a, BaseviewView>> for crate::Window<'a> {
    fn from(value: ViewRef<'a, BaseviewView>) -> Self {
        crate::Window::new(Window { view: value.view, inner: value.inner })
    }
}

impl<'a> Window<'a> {
    pub fn open_parented<P, H, B>(parent: &P, options: WindowOpenOptions, build: B) -> WindowHandle
    where
        P: HasRawWindowHandle,
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        autoreleasepool(|_| {
            let (_parent_window, parent_view) =
                extract_raw_window_handle(parent.raw_window_handle());

            let Some(parent_view) = parent_view else {
                panic!("Invalid window handle: ns_view is NULL");
            };

            let parenting =
                ViewParentingType::Parented { parent_view: Weak::from_retained(&parent_view) };

            let (ns_view, state) = BaseviewView::new(options, build, parenting);

            WindowHandle { view: Some(Weak::from_retained(&ns_view)), state }
        })
    }

    pub fn open_blocking<H, B>(options: WindowOpenOptions, build: B)
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        autoreleasepool(|_| {
            let Some(mtm) = MainThreadMarker::new() else {
                panic!("macOS: open_blocking can only be called on the main thread!")
            };

            // Creates the global NSApplication instance, if it doesn't exist yet
            let app = NSApplication::sharedApplication(mtm);

            let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

            let window = create_window(options.size, mtm);
            window.center();

            let title = NSString::from_str(&options.title);
            window.setTitle(&title);
            window.makeKeyAndOrderFront(None);

            let parenting = ViewParentingType::Windowed {
                running_app: Weak::from_retained(&app),
                owned_window: Weak::from_retained(&window),
            };

            let _ = BaseviewView::new(options, build, parenting);

            app.run();
        })
    }

    pub fn close(&mut self) {
        BaseviewView::close(self.view.inner_ref());
    }

    pub fn has_focus(&mut self) -> bool {
        let Some(window) = self.view.window() else {
            return false;
        };

        if !window.isKeyWindow() {
            return false;
        }

        let Some(first_responder) = window.firstResponder() else {
            return false;
        };

        self.view.isEqual(Some(&*first_responder))
    }

    pub fn focus(&mut self) {
        if let Some(window) = self.view.window() {
            window.makeFirstResponder(Some(self.view));
        }
    }

    pub fn resize(&mut self, size: Size) {
        if self.inner.state.closed.get() {
            return;
        }

        BaseviewView::resize(self.view.inner_ref(), size);
    }

    pub fn set_mouse_cursor(&self, cursor: MouseCursor) {
        // Cursor rects added outside resetCursorRects are discarded the
        // next time AppKit rebuilds them, after which whatever the host
        // window's underlying views declare wins (the plugin cursor then
        // appears to depend on what's BEHIND the plugin window). Remember
        // the cursor and force a rebuild; the resetCursorRects override
        // re-asserts it every time AppKit asks.
        self.inner.state.mouse_cursor.set(cursor);
        if let Some(window) = self.view.window() {
            window.invalidateCursorRectsForView(&self.view);
        }
        // Cursor rects only apply while the window is key; if the pointer
        // is over the view right now, apply the change directly so it also
        // works before the first click on a freshly opened window.
        if self.inner.state.mouse_inside.get() {
            Cursor::from(cursor).load().set();
        }
    }

    pub fn set_frame_interval(&self, interval: f64) {
        if self.inner.state.closed.get() {
            return;
        }
        BaseviewView::arm_frame_timer(self.view, interval);
    }

    pub fn display_max_fps(&self) -> Option<f64> {
        let screen = self.view.window()?.screen()?;
        // 0 would mean "no answer"; treat it as one rather than handing back
        // a rate nothing can be derived from.
        match screen.maximumFramesPerSecond() {
            fps if fps > 0 => Some(fps as f64),
            _ => None,
        }
    }

    #[cfg(feature = "opengl")]
    pub fn gl_context(&self) -> Option<&GlContext> {
        self.inner.gl_context.get()
    }
}

pub(crate) struct WindowSharedState {
    /// The last known window info for this window.
    pub window_info: Cell<WindowInfo>,
    pub closed: Cell<bool>,
    /// The cursor this view wants; re-asserted from resetCursorRects so it
    /// survives AppKit cursor-rect rebuilds (see set_mouse_cursor).
    pub mouse_cursor: Cell<MouseCursor>,
    /// Whether the pointer is currently over the view (tracking-area
    /// enter/exit); used to apply cursor changes immediately.
    pub mouse_inside: Cell<bool>,
}

impl WindowSharedState {
    pub fn new(options: &WindowOpenOptions) -> Self {
        Self {
            window_info: WindowInfo::from_logical_size(options.size, 1.0).into(),
            closed: false.into(),
            mouse_cursor: MouseCursor::default().into(),
            mouse_inside: false.into(),
        }
    }
}

unsafe impl<'a> HasRawWindowHandle for Window<'a> {
    fn raw_window_handle(&self) -> RawWindowHandle {
        self.view.raw_window_handle()
    }
}

unsafe impl<'a> HasRawDisplayHandle for Window<'a> {
    fn raw_display_handle(&self) -> RawDisplayHandle {
        RawDisplayHandle::AppKit(AppKitDisplayHandle::empty())
    }
}

pub fn copy_to_clipboard(string: &str) {
    let pb = NSPasteboard::generalPasteboard();
    let ns_str = NSString::from_str(string);

    pb.clearContents();
    pb.setString_forType(&ns_str, unsafe { NSPasteboardTypeString });
}
