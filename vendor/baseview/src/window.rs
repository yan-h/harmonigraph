use std::marker::PhantomData;

use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
};

use crate::event::{Event, EventStatus};
use crate::window_open_options::WindowOpenOptions;
use crate::{platform, MouseCursor, Size};

pub struct WindowHandle {
    window_handle: platform::WindowHandle,
    // so that WindowHandle is !Send on all platforms
    phantom: PhantomData<*mut ()>,
}

impl WindowHandle {
    fn new(window_handle: platform::WindowHandle) -> Self {
        Self { window_handle, phantom: PhantomData }
    }

    /// Close the window
    pub fn close(&mut self) {
        self.window_handle.close();
    }

    /// Returns `true` if the window is still open, and returns `false`
    /// if the window was closed/dropped.
    pub fn is_open(&self) -> bool {
        self.window_handle.is_open()
    }
}

unsafe impl HasRawWindowHandle for WindowHandle {
    fn raw_window_handle(&self) -> RawWindowHandle {
        self.window_handle.raw_window_handle()
    }
}

pub trait WindowHandler {
    fn on_frame(&mut self, window: &mut Window);
    fn on_event(&mut self, window: &mut Window, event: Event) -> EventStatus;
}

pub struct Window<'a> {
    window: platform::Window<'a>,

    // so that Window is !Send on all platforms
    phantom: PhantomData<*mut ()>,
}

impl<'a> Window<'a> {
    #[cfg(target_os = "windows")]
    pub(crate) fn new(window: platform::Window<'a>) -> Window<'a> {
        Window { window, phantom: PhantomData }
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn new(window: platform::Window) -> Window {
        Window { window, phantom: PhantomData }
    }

    pub fn open_parented<P, H, B>(parent: &P, options: WindowOpenOptions, build: B) -> WindowHandle
    where
        P: HasRawWindowHandle,
        H: WindowHandler + 'static,
        B: FnOnce(&mut Window) -> H,
        B: Send + 'static,
    {
        let window_handle = platform::Window::open_parented::<P, H, B>(parent, options, build);
        WindowHandle::new(window_handle)
    }

    pub fn open_blocking<H, B>(options: WindowOpenOptions, build: B)
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut Window) -> H,
        B: Send + 'static,
    {
        platform::Window::open_blocking::<H, B>(options, build)
    }

    /// Close the window
    pub fn close(&mut self) {
        self.window.close();
    }

    /// Resize the window to the given size. The size is always in logical pixels. DPI scaling will
    /// automatically be accounted for.
    pub fn resize(&mut self, size: Size) {
        self.window.resize(size);
    }

    pub fn set_mouse_cursor(&mut self, cursor: MouseCursor) {
        self.window.set_mouse_cursor(cursor);
    }

    /// Re-arm the frame timer at `interval` seconds, changing how often
    /// `on_frame` runs from here on.
    ///
    /// This is the only hard bound on the frame rate: a handler that asks to
    /// be repainted more often simply isn't asked. The interval is clamped to
    /// [`MIN_FRAME_INTERVAL`]..=[`MAX_FRAME_INTERVAL`], so a nonsense value
    /// cannot spin the run loop or stall the window outright.
    ///
    /// macOS only; a no-op elsewhere, where frames are driven differently.
    pub fn set_frame_interval(&mut self, interval: f64) {
        #[cfg(target_os = "macos")]
        self.window.set_frame_interval(interval);
        #[cfg(not(target_os = "macos"))]
        let _ = interval;
    }

    /// The highest refresh rate, in Hz, that the display showing this window
    /// can present at — the rate above which painting is wasted work.
    ///
    /// `None` when the platform won't say (every non-macOS backend, or a
    /// window not on a screen yet).
    pub fn display_max_fps(&self) -> Option<f64> {
        #[cfg(target_os = "macos")]
        return self.window.display_max_fps();
        #[cfg(not(target_os = "macos"))]
        return None;
    }

    pub fn has_focus(&mut self) -> bool {
        self.window.has_focus()
    }

    pub fn focus(&mut self) {
        self.window.focus()
    }

    /// If provided, then an OpenGL context will be created for this window. You'll be able to
    /// access this context through [crate::Window::gl_context].
    #[cfg(feature = "opengl")]
    pub fn gl_context(&self) -> Option<&crate::gl::GlContext> {
        self.window.gl_context()
    }
}

unsafe impl<'a> HasRawWindowHandle for Window<'a> {
    fn raw_window_handle(&self) -> RawWindowHandle {
        self.window.raw_window_handle()
    }
}

unsafe impl<'a> HasRawDisplayHandle for Window<'a> {
    fn raw_display_handle(&self) -> RawDisplayHandle {
        self.window.raw_display_handle()
    }
}
