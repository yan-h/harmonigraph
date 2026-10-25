//! The editor's own window, so the picture stays on screen when the host is
//! not the frontmost app.
//!
//! Bitwig takes every plugin window off screen while another app is in front:
//! the window it gives the editor sits at the floating level and is ordered
//! out on deactivation, while the plugin-host process stays un-hidden. A window
//! the plugin creates itself is one Bitwig does not know about, and it stays up
//! (checked in Bitwig: #1580). So on macOS the editor lives in one unless the
//! System tab docks it, and the host's window is then left open but invisible
//! and click-through. Hiding it outright would be undone: Bitwig orders it back
//! to the front whenever it is itself brought forward.
//!
//! The live view moves rather than a second one opening, which keeps the
//! renderer and the egui context; baseview already follows its view to a new
//! window (tracking area, first responder, occlusion, and the display link,
//! which follows the screen). A move is never made inside a frame: making a
//! window key sends baseview a focus event on the spot, and baseview's handler
//! is borrowed for the whole frame, so a move from a frame panics across AppKit
//! and takes the host process down. The one at open is made from
//! `Editor::spawn`; one the System tab asks for is performed on the run loop's
//! next turn ([`PopOut::want`]).
//!
//! What the host's window did for the editor, this one does in its place. It
//! floats above the host's windows while the host is in front, as Bitwig's
//! did, and drops to the normal level while another app is, which is where
//! Bitwig's would have vanished.
//!
//! Its close button does not close it: showing and hiding the editor is the
//! host's, and the host's show/hide control would otherwise still think it
//! open. The button asks the host to hide it ([`EditorHost`]), and the host's
//! `gui.hide`, which for an embedded editor does nothing, orders this window
//! out; `gui.show` orders it back in. Bitwig calls `gui.hide` during the
//! request (#1607). The ask waits for the run loop's next turn, since a host
//! may destroy the editor during it, and the window must not go while
//! AppKit is still closing it. A host that refuses gets the window minimized.
//!
//! The view's size is the size source's to set, as for a host resize: the
//! size source asks [`PopOut::take_size`] first, at the top of the frame it
//! will be laid out in. Every AppKit call here is on the main thread, which is
//! where egui-baseview runs its size source and its frames, and where the CLAP
//! host creates and destroys the editor.

use std::sync::Arc;

use nice_plug::prelude::ParentWindowHandle;
use nice_plug::wrapper::clap::EditorHost;

#[cfg(target_os = "macos")]
pub(super) use macos::PopOut;

#[cfg(not(target_os = "macos"))]
pub(super) use elsewhere::PopOut;

/// What the size source owes the view this frame, from [`PopOut::take_size`].
pub(super) enum Sized {
    /// The editor's own window was resized: the editor takes its size.
    Own((u32, u32)),
    /// The editor is back in the host's window, which should be brought to
    /// the editor's size.
    Docked,
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    use std::path::{Path, PathBuf};
    use std::sync::Weak;

    use baseview::WindowHandle;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2::{define_class, msg_send, sel, ClassType, DefinedClass, MainThreadOnly};
    use objc2_app_kit::{
        NSBackingStoreType, NSView, NSWindow, NSWindowDelegate, NSWindowLevel,
        NSWindowOrderingMode, NSWindowStyleMask, NSWorkspace,
        NSWorkspaceDidActivateApplicationNotification,
    };
    use objc2_foundation::{
        MainThreadMarker, NSArray, NSDictionary, NSNotification, NSNumber, NSObject,
        NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
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
        /// Hears which app comes to the front, and performs deferred moves.
        agent: Option<Retained<Agent>>,
        /// The editor's own window, while the view is in it.
        window: Option<Retained<NSWindow>>,
        /// The content size last given to the view in its own window, in
        /// logical points.
        size: (u32, u32),
        /// A move asked for and not yet performed: out if true.
        wanted: Option<bool>,
        /// Docked since the last size source.
        redocked: bool,
        /// An app that came forward and has not yet raised its windows, and
        /// how many more times to look ([`SETTLE_TRIES`]). Kept at zero once
        /// the looks run out with the window still raised over everything,
        /// so the next app to come forward tucks it instead.
        settling: Option<(i32, u8)>,
        /// Whom the close button asks, absent where there is no CLAP host.
        editor_host: Option<Arc<dyn EditorHost>>,
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
    ///
    /// `None` when it is the host, or else the frontmost app's process.
    fn other_in_front(exe: &Path) -> Option<i32> {
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        // SAFETY: `processIdentifier` is a `pid_t` property of every running
        // application, read by hand to spare the binding its `libc` feature.
        let pid: i32 = unsafe { msg_send![&*app, processIdentifier] };
        // This process by its pid, since a plugin host need not be a bundle.
        if pid as u32 == std::process::id() {
            return None;
        }
        let bundle = app.bundleURL().and_then(|url| url.path());
        if bundle.is_some_and(|bundle| exe.starts_with(PathBuf::from(bundle.to_string()))) {
            return None;
        }
        Some(pid)
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        /// Returns a +1 `CFArrayRef` of `CFDictionaryRef`, toll-free
        /// bridged to their Foundation counterparts.
        fn CGWindowListCopyWindowInfo(option: u32, relative_to_window: u32) -> *mut NSArray;
    }

    /// The window to put the editor's just below when `pid` comes to the
    /// front: the last of the run of its windows at the front of the normal
    /// level (this one's own excepted), so the app that came forward covers
    /// it and nothing that was behind it does. `None` while that app's windows
    /// are not at the front yet.
    ///
    /// Only the owner, layer and number of each window are read, which need
    /// no screen-recording permission.
    fn below_front_of(pid: i32, own: isize) -> Option<isize> {
        const ON_SCREEN_ONLY: u32 = 1 << 0;
        const EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
        // SAFETY: a Copy-rule CoreFoundation array, owned from here.
        let list = unsafe {
            Retained::from_raw(CGWindowListCopyWindowInfo(
                ON_SCREEN_ONLY | EXCLUDE_DESKTOP_ELEMENTS,
                0,
            ))
        }?;
        let value = |info: &AnyObject, key: &str| -> Option<Retained<NSNumber>> {
            let info = info.downcast_ref::<NSDictionary>()?;
            info.objectForKey(&*NSString::from_str(key))?.downcast::<NSNumber>().ok()
        };
        let number = |info: &AnyObject, key: &str| value(info, key).map(|n| n.as_isize());
        let mut front_run = None;
        for info in list.iter() {
            if number(&info, "kCGWindowLayer") != Some(NORMAL) {
                continue;
            }
            // An invisible window (an overlay, or Bitwig's concealed one) is
            // in front of nothing.
            if value(&info, "kCGWindowAlpha").is_some_and(|alpha| alpha.as_f64() == 0.0) {
                continue;
            }
            let Some(window) = number(&info, "kCGWindowNumber") else { continue };
            if window == own {
                continue;
            }
            if number(&info, "kCGWindowOwnerPID") != Some(pid as isize) {
                break;
            }
            front_run = Some(window);
        }
        front_run
    }

    /// Put the window just behind the windows `pid` brought forward, if they
    /// are at the front yet. Returns whether they were.
    fn tuck(window: &NSWindow, pid: i32) -> bool {
        let Some(other) = below_front_of(pid, window.windowNumber()) else { return false };
        window.orderWindow_relativeTo(NSWindowOrderingMode::Below, other);
        true
    }

    /// How often, and how many times, to look again for the windows of an
    /// app that came forward. The activation is announced before that app has
    /// raised them, at least when it was clicked in the Dock (seen in Bitwig,
    /// #1580): looked for at once, they are still wherever they were, and the
    /// editor tucked behind them would land behind whatever covered them.
    const SETTLE_DELAY: f64 = 0.05;
    const SETTLE_TRIES: u8 = 10;

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

    fn reveal(window: &NSWindow) {
        window.setIgnoresMouseEvents(false);
        window.setAlphaValue(1.0);
    }

    impl PopOut {
        pub(in crate::editor) fn new() -> Arc<Self> {
            Arc::new(Self(Mutex::new(State::default())))
        }

        /// Learn the editor's views once it has opened in the host's, and
        /// move it into its own window unless it is to stay `docked`. Called
        /// from `Editor::spawn`, which is outside any frame (see the module
        /// documentation).
        pub(in crate::editor) fn open(
            self: &Arc<Self>,
            parent: ParentWindowHandle,
            window: &WindowHandle,
            docked: bool,
            editor_host: Option<Arc<dyn EditorHost>>,
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
            let agent = Agent::alloc(mtm).set_ivars(Ivars { pop_out: Arc::downgrade(self), exe });
            // SAFETY: NSObject's init on a freshly allocated subclass instance.
            let agent: Retained<Agent> = unsafe { msg_send![super(agent), init] };
            // SAFETY: `activated:` takes the notification; the agent is
            // removed in `close`, before it can be freed.
            unsafe {
                NSWorkspace::sharedWorkspace()
                    .notificationCenter()
                    .addObserver_selector_name_object(
                        &agent,
                        sel!(activated:),
                        Some(NSWorkspaceDidActivateApplicationNotification),
                        None,
                    );
            }
            *self.0.lock() = State {
                host: Some(host),
                view: Some(view),
                agent: Some(agent),
                editor_host: editor_host.clone(),
                ..State::default()
            };
            if let Some(editor_host) = editor_host {
                let pop_out = Arc::downgrade(self);
                editor_host.on_show_hide(Some(Arc::new(move |visible| {
                    pop_out.upgrade().is_some_and(|pop_out| pop_out.show_hide(visible))
                })));
            }
            if !docked {
                self.pop(mtm);
            }
        }

        /// Whether the editor can have a window of its own here.
        pub(in crate::editor) fn offered(&self) -> bool {
            self.0.lock().agent.is_some()
        }

        /// Ask for the editor in its own window or in the host's, performed
        /// on the run loop's next turn if it is not there already: asked from
        /// a frame, which is no place to move it (see the module
        /// documentation).
        pub(in crate::editor) fn want(&self, out: bool) {
            let _ = main_thread();
            let agent = {
                let mut state = self.0.lock();
                if state.window.is_some() == out {
                    // There already, which also calls off a move the other
                    // way that has not happened yet.
                    state.wanted = None;
                    return;
                }
                if state.wanted.replace(out).is_some() {
                    return;
                }
                state.agent.clone()
            };
            let Some(agent) = agent else { return };
            // SAFETY: `apply:` is the agent's own method, taking an object it
            // ignores; the run loop retains the target until then.
            let () = unsafe {
                msg_send![
                    &*agent,
                    performSelector: sel!(apply:),
                    withObject: None::<&AnyObject>,
                    afterDelay: 0.0f64
                ]
            };
        }

        /// The move [`Self::want`] deferred, off the run loop.
        fn apply(&self) {
            let mtm = main_thread();
            let wanted = self.0.lock().wanted.take();
            match wanted {
                Some(true) => self.pop(mtm),
                Some(false) => self.dock(),
                None => {}
            }
        }

        /// The size of the editor's own window, while it is in one, which
        /// then has the say over the editor's size rather than the host.
        pub(in crate::editor) fn size(&self) -> Option<(u32, u32)> {
            let state = self.0.lock();
            state.window.as_ref().map(|_| state.size)
        }

        /// The size source's question, asked before the host's: what the
        /// view's size owes to where it now is.
        ///
        /// Also where the host's window is kept concealed while the editor is
        /// out of it, should the host have put it back, or have shown it only
        /// after the editor opened.
        pub(in crate::editor) fn take_size(&self) -> Option<Sized> {
            let _ = main_thread();
            let (window, host) = {
                let mut state = self.0.lock();
                if std::mem::take(&mut state.redocked) {
                    return Some(Sized::Docked);
                }
                (state.window.clone()?, state.host.clone())
            };
            if let Some(host_window) = host.and_then(|host| host.window()) {
                conceal(&host_window);
            }
            let size = logical(window.contentView()?.frame().size);
            let mut state = self.0.lock();
            (size != state.size).then(|| {
                state.size = size;
                Sized::Own(size)
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

        fn pop(&self, mtm: MainThreadMarker) {
            let (host, view, agent, closable) = {
                let state = self.0.lock();
                let (None, Some(host), Some(view), Some(agent)) =
                    (&state.window, &state.host, &state.view, &state.agent)
                else {
                    return;
                };
                (host.clone(), view.clone(), agent.clone(), state.editor_host.is_some())
            };
            let exe = agent.ivars().exe.clone();

            let content = view.frame().size;
            let mut style = NSWindowStyleMask::Titled
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable;
            if closable {
                style |= NSWindowStyleMask::Closable;
            }
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
            // The close button asks the agent, which refuses it and asks the
            // host to hide the editor. The window keeps the delegate weakly;
            // `State` owns both.
            own.setDelegate(Some(ProtocolObject::from_ref(&*agent)));
            own.setTitle(&NSString::from_str("Harmonigraph"));
            let min = super::super::MIN_SIZE;
            own.setContentMinSize(NSSize::new(f64::from(min.0), f64::from(min.1)));
            own.setLevel(if other_in_front(&exe).is_some() { NORMAL } else { FLOATING });

            // Where it was last time, at the editor's size. The first time,
            // where the host's window is.
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

            let mut state = self.0.lock();
            state.window = Some(own);
            state.size = logical(content);
        }

        /// Back into the host's window, shown again, and the editor's own
        /// closed. The view keeps its size; the size source asks the host for
        /// the same ([`Sized::Docked`]).
        fn dock(&self) {
            let _ = main_thread();
            let (window, host, view) = {
                let mut state = self.0.lock();
                let Some(window) = state.window.take() else { return };
                // Before the move, which can draw a frame on the spot: that
                // frame's size source must already find it docked.
                state.redocked = true;
                (window, state.host.clone(), state.view.clone())
            };
            if let (Some(host), Some(view)) = (host, view) {
                view.removeFromSuperview();
                host.addSubview(&view);
                view.setFrameOrigin(NSPoint::ZERO);
                if let Some(host_window) = host.window() {
                    reveal(&host_window);
                }
            }
            window.close();
        }

        /// The editor closing, called before baseview closes the view, which
        /// takes it out of whichever window it is in: show the host's window
        /// again, should the host reuse it, close the editor's own, and let go
        /// of everything. The last is not tidiness: the view owns the size
        /// source, which owns this, so held views would keep the whole editor
        /// alive after it closed.
        pub(in crate::editor) fn close(&self) {
            let _ = main_thread();
            let state = std::mem::take(&mut *self.0.lock());
            if let Some(editor_host) = &state.editor_host {
                editor_host.on_show_hide(None);
            }
            if let Some(agent) = &state.agent {
                // SAFETY: removing an observer this added, still alive.
                unsafe {
                    NSWorkspace::sharedWorkspace().notificationCenter().removeObserver(agent)
                };
            }
            if let Some(window) = &state.window {
                if let Some(host_window) = state.host.as_ref().and_then(|host| host.window()) {
                    reveal(&host_window);
                }
                window.setDelegate(None);
                window.close();
            }
        }

        /// The close button, deferred from the window's asking: the host is
        /// asked to hide the editor, which it does through [`Self::show_hide`].
        fn ask_host_close(&self) {
            let _ = main_thread();
            let (host, window) = {
                let state = self.0.lock();
                (state.editor_host.clone(), state.window.clone())
            };
            if host.is_some_and(|host| host.request_hide()) {
                return;
            }
            if let Some(window) = window {
                window.miniaturize(None);
            }
        }

        /// The host showing or hiding the editor, which while docked it does
        /// with its own window: the editor's own is ordered in or out, and
        /// out of sight it is occluded, which baseview treats as minimized.
        /// Docked, this leaves the host to it, as before the editor had a
        /// window of its own: Bitwig then destroys the editor and makes it
        /// anew.
        fn show_hide(&self, visible: bool) -> bool {
            let _ = main_thread();
            let Some(window) = self.0.lock().window.clone() else { return false };
            if visible {
                window.makeKeyAndOrderFront(None);
            } else {
                window.orderOut(None);
            }
            true
        }

        /// Another app came to the front, or the host did: float above the
        /// host's windows while the host is. Leaving the host, sit among the
        /// normal windows, just behind the app that came forward and above
        /// everything else; and from one other app to the next, stay put, as
        /// any window would, under whatever had already covered it.
        fn front_changed(&self, exe: &Path) {
            let (window, agent, host, unplaced) = {
                let mut state = self.0.lock();
                let unplaced = state.settling.take().is_some();
                (state.window.clone(), state.agent.clone(), state.host.clone(), unplaced)
            };
            let Some(window) = window else { return };
            let Some(pid) = other_in_front(exe) else {
                // Bitwig puts its window back in front as it comes forward:
                // hidden again now rather than at the next frame.
                if let Some(host_window) = host.and_then(|host| host.window()) {
                    conceal(&host_window);
                }
                window.setLevel(FLOATING);
                return;
            };
            // Hidden or minimized: nothing to stack, which ordering it would
            // undo.
            if !window.isVisible() {
                window.setLevel(NORMAL);
                return;
            }
            // Still raised from leaving the host, and never tucked: placed
            // behind this app instead, as it would have been behind that one.
            if window.level() != FLOATING && !unplaced {
                return;
            }
            window.setLevel(NORMAL);
            if tuck(&window, pid) {
                return;
            }
            // Above everything it covered, until that app's windows arrive in
            // front of it.
            window.orderFront(None);
            self.0.lock().settling = Some((pid, SETTLE_TRIES));
            if let Some(agent) = agent {
                agent.settle_later();
            }
        }

        /// Look again for the windows of the app that came forward.
        fn settle(&self) {
            let (window, agent, pid, tries) = {
                let mut state = self.0.lock();
                let (Some((pid, tries @ 1..)), Some(window)) =
                    (state.settling, state.window.clone())
                else {
                    return;
                };
                state.settling = None;
                (window, state.agent.clone(), pid, tries)
            };
            if !window.isVisible() {
                return;
            }
            if tuck(&window, pid) {
                return;
            }
            self.0.lock().settling = Some((pid, tries - 1));
            if tries == 1 {
                return;
            }
            if let Some(agent) = agent {
                agent.settle_later();
            }
        }
    }

    struct Ivars {
        pop_out: Weak<PopOut>,
        exe: PathBuf,
    }

    define_class!(
        // NSObject adds no subclass invariants; the agent is main-thread only,
        // as workspace notifications and run-loop performs are delivered there.
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = Ivars]
        struct Agent;

        unsafe impl NSObjectProtocol for Agent {}

        unsafe impl NSWindowDelegate for Agent {
            /// The close button: refused, and the host asked instead on the
            /// run loop's next turn.
            #[unsafe(method(windowShouldClose:))]
            fn window_should_close(&self, _: &NSWindow) -> bool {
                // SAFETY: `hostClose:` is the agent's own method, taking an
                // object it ignores; the run loop retains the target until
                // then, so the host destroying the editor cannot free it
                // mid-call.
                let () = unsafe {
                    msg_send![
                        self,
                        performSelector: sel!(hostClose:),
                        withObject: None::<&AnyObject>,
                        afterDelay: 0.0f64
                    ]
                };
                false
            }
        }

        impl Agent {
            /// Another app came to the front, or the host did.
            #[unsafe(method(activated:))]
            fn activated(&self, _: &NSNotification) {
                let ivars = self.ivars();
                if let Some(pop_out) = ivars.pop_out.upgrade() {
                    pop_out.front_changed(&ivars.exe);
                }
            }

            /// A move [`PopOut::want`] deferred to here.
            #[unsafe(method(apply:))]
            fn apply(&self, _: Option<&AnyObject>) {
                if let Some(pop_out) = self.ivars().pop_out.upgrade() {
                    pop_out.apply();
                }
            }

            #[unsafe(method(hostClose:))]
            fn host_close(&self, _: Option<&AnyObject>) {
                if let Some(pop_out) = self.ivars().pop_out.upgrade() {
                    pop_out.ask_host_close();
                }
            }

            #[unsafe(method(settle:))]
            fn settle(&self, _: Option<&AnyObject>) {
                if let Some(pop_out) = self.ivars().pop_out.upgrade() {
                    pop_out.settle();
                }
            }
        }
    );

    impl Agent {
        fn settle_later(&self) {
            // One chain of looks at a time: a switch mid-chain starts afresh.
            // SAFETY: cancelling this object's own pending `settle:`, then
            // scheduling it; the method takes an object it ignores, and the
            // run loop retains the target until then.
            let () = unsafe {
                msg_send![
                    NSObject::class(),
                    cancelPreviousPerformRequestsWithTarget: self,
                    selector: sel!(settle:),
                    object: None::<&AnyObject>
                ]
            };
            let () = unsafe {
                msg_send![
                    self,
                    performSelector: sel!(settle:),
                    withObject: None::<&AnyObject>,
                    afterDelay: SETTLE_DELAY
                ]
            };
        }
    }
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
            _: bool,
            _: Option<Arc<dyn EditorHost>>,
        ) {
        }
        pub(in crate::editor) fn offered(&self) -> bool {
            false
        }
        pub(in crate::editor) fn want(&self, _: bool) {}
        pub(in crate::editor) fn size(&self) -> Option<(u32, u32)> {
            None
        }
        pub(in crate::editor) fn take_size(&self) -> Option<Sized> {
            None
        }
        pub(in crate::editor) fn resize(&self, _: (u32, u32)) -> bool {
            false
        }
        pub(in crate::editor) fn close(&self) {}
    }
}
