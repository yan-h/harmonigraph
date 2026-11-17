//! The platform off macOS: no window opens, and everything else compiles.
//!
//! This fork has a macOS platform only. The stub lets a Linux cloud session
//! build the workspace and run its headless tests; an editor opened there
//! gets a handle that reports itself closed, and nothing is drawn.

use std::convert::Infallible;
use std::marker::PhantomData;

use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle, XlibWindowHandle,
};

use crate::{MouseCursor, Size, WindowHandler, WindowOpenOptions};

pub struct WindowHandle;

impl WindowHandle {
    pub fn close(&mut self) {}

    pub fn is_open(&self) -> bool {
        false
    }
}

unsafe impl HasRawWindowHandle for WindowHandle {
    fn raw_window_handle(&self) -> RawWindowHandle {
        XlibWindowHandle::empty().into()
    }
}

/// Uninhabited: no window is ever built, so `build` is never called.
pub struct Window<'a> {
    never: Infallible,
    phantom: PhantomData<&'a ()>,
}

impl<'a> Window<'a> {
    pub fn open_parented<P, H, B>(
        _parent: &P, _options: WindowOpenOptions, _build: B,
    ) -> WindowHandle
    where
        P: HasRawWindowHandle,
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
        WindowHandle
    }

    pub fn open_blocking<H, B>(_options: WindowOpenOptions, _build: B)
    where
        H: WindowHandler + 'static,
        B: FnOnce(&mut crate::Window) -> H,
        B: Send + 'static,
    {
    }

    pub fn close(&mut self) {
        match self.never {}
    }

    pub fn has_focus(&mut self) -> bool {
        match self.never {}
    }

    pub fn focus(&mut self) {
        match self.never {}
    }

    pub fn resize(&mut self, _size: Size) {
        match self.never {}
    }

    pub fn set_mouse_cursor(&self, _cursor: MouseCursor) {
        match self.never {}
    }
}

unsafe impl<'a> HasRawWindowHandle for Window<'a> {
    fn raw_window_handle(&self) -> RawWindowHandle {
        match self.never {}
    }
}

unsafe impl<'a> HasRawDisplayHandle for Window<'a> {
    fn raw_display_handle(&self) -> RawDisplayHandle {
        match self.never {}
    }
}

pub fn copy_to_clipboard(_data: &str) {}
