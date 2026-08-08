#![allow(deprecated)] // Allow use of NSFilenamesPboardType for now

use super::cursor::Cursor;
use super::keyboard::{make_modifiers, KeyboardState};
use super::window::WindowSharedState;
use crate::wrappers::appkit::*;
use crate::MouseEvent::{ButtonPressed, ButtonReleased};
use crate::{
    DropData, DropEffect, Event, EventStatus, MouseButton, MouseEvent, Point, ScrollDelta, Size,
    WindowEvent, WindowHandler, WindowInfo, WindowOpenOptions,
};
use objc2::__framework_prelude::Retained;
use objc2::rc::Weak;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{msg_send, AllocAnyThread};
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingInfo, NSEvent, NSFilenamesPboardType, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSWindow, NSWindowOcclusionState,
};
use objc2_foundation::{NSArray, NSNotification, NSPoint, NSRect, NSSize, NSString};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

pub enum ViewParentingType {
    Parented { parent_view: Weak<NSView> },
    Windowed { owned_window: Weak<NSWindow>, running_app: Weak<NSApplication> },
}

pub(crate) struct BaseviewView {
    pub(crate) state: Rc<WindowSharedState>,
    window_handler: RefCell<Option<Box<dyn WindowHandler>>>,

    /// Events that will be triggered at the end of `window_handler`'s borrow.
    deferred_events: RefCell<VecDeque<Event>>,

    /// A `CursorLeft` held back because the pointer left with a button down
    /// (see `mouse_exited`), owed to the handler once the button comes up.
    exit_withheld: Cell<bool>,

    /// Buttons this view has reported pressed and not yet reported released,
    /// in `NSEvent::pressedMouseButtons`' bit order (see
    /// `buttons_owed_release`). One bit is one release owed, and whoever pays
    /// it — AppKit's `mouseUp:`, or `settle_stuck_buttons` — the other does not
    /// pay it again.
    ///
    /// A bit, which is not the same as a press: every button past the first two
    /// shares bit 2, so two of them held at once are one bit between them. The
    /// second release is swallowed as a duplicate, and a button still held
    /// after the first has been released is no longer recorded to be repaired.
    /// Neither reaches the consumer, because a button is a bool at that end too
    /// (egui's `down[Middle]`): the first release already puts it up, so nothing
    /// is left held and no scroll area stays shut. Counting presses per bit
    /// would buy a truer number that nothing reads.
    buttons_reported_down: Cell<u8>,

    /// Of those, the ones the OS already showed up at the previous SETTLE — a
    /// release is synthesised only for a button stuck across two of them. A
    /// `mouseUp:` sitting in the queue while the hardware is already up is an
    /// ordinary fast click, and it deserves the gap it takes to arrive.
    ///
    /// A settle rather than a frame interval, because a settle is what this
    /// counts: `trigger_frame` runs it, and the frame timer is not
    /// `trigger_frame`'s only caller — `handle_occlusion_notification` paints
    /// from inside a notification. Two settles can land microseconds apart
    /// there, leaving a queued release almost no gap at all. What that costs is
    /// a release sent early, carrying `last_mods()` rather than the event's
    /// modifiers, on a window coming back from occlusion.
    buttons_seen_up: Cell<u8>,

    frame_timer: Cell<Option<TimerHandle>>,
    notification_center_observer: Cell<Option<NotificationCenterObserver>>,
    occlusion_observer: Cell<Option<NotificationCenterObserver>>,

    /// Occlusion as this view last saw it, so a notification repeating the
    /// current state is dropped rather than acted on. Starts visible, which
    /// is what a window a view is being added to almost always is, and costs
    /// only a missed no-op event if it isn't.
    window_visible: Cell<bool>,

    keyboard_state: KeyboardState,

    parenting: ViewParentingType,

    #[cfg(feature = "opengl")]
    pub(crate) gl_context: std::cell::OnceCell<crate::gl::GlContext>,
}

impl BaseviewView {
    pub fn new<H: WindowHandler + 'static>(
        options: WindowOpenOptions, builder: impl FnOnce(&mut crate::Window) -> H,
        parenting: ViewParentingType,
    ) -> (Retained<View<Self>>, Rc<WindowSharedState>) {
        let view_rect =
            NSRect::new(NSPoint::ZERO, NSSize::new(options.size.width, options.size.height));

        let state = Rc::new(WindowSharedState::new(&options));

        let inner = BaseviewView {
            state: state.clone(),

            deferred_events: RefCell::default(),
            exit_withheld: false.into(),
            buttons_reported_down: 0.into(),
            buttons_seen_up: 0.into(),
            keyboard_state: KeyboardState::new(),
            frame_timer: None.into(),
            window_handler: None.into(),
            notification_center_observer: None.into(),
            occlusion_observer: None.into(),
            window_visible: true.into(),
            parenting,

            #[cfg(feature = "opengl")]
            gl_context: std::cell::OnceCell::new(),
        };

        let view = View::new(view_rect, inner, |view| {
            // Set up parenting before handler setup
            match &view.parenting {
                ViewParentingType::Parented { parent_view } => {
                    let parent_view = parent_view.load().unwrap();
                    parent_view.addSubview(view.view);
                }
                ViewParentingType::Windowed { owned_window, .. } => {
                    let owned_window = owned_window.load().unwrap();
                    owned_window.setContentView(Some(view.view));
                    set_delegate(&owned_window, view.view);
                }
            }

            #[cfg(feature = "opengl")]
            if let Some(gl_config) = options.gl_config {
                let gl_context = super::gl::GlContext::create(view.view, gl_config).unwrap();
                let gl_context = crate::gl::GlContext::new(gl_context);
                let Ok(()) = view.gl_context.set(gl_context) else { unreachable!() };
            }

            // Initialize handler
            view.window_handler.replace(Some(Box::new(builder(&mut view.into()))));

            // Set up anything that might trigger events to the handler

            // SAFETY: This static is a read-only constant
            let ns_filenames_pboard_type = unsafe { NSFilenamesPboardType };
            view.view.registerForDraggedTypes(&NSArray::from_slice(&[ns_filenames_pboard_type]));

            Self::arm_frame_timer(view.view, options.frame_interval);

            let notifier_view = Weak::new(view.view);
            let observer = NotificationCenterObserver::register_window_key_change(move |n| {
                if let Some(view) = notifier_view.load() {
                    BaseviewView::handle_notification(view.inner_ref(), n);
                }
            });
            view.notification_center_observer.set(Some(observer));

            let occlusion_view = Weak::new(view.view);
            let observer = NotificationCenterObserver::register_window_occlusion_change(move |n| {
                if let Some(view) = occlusion_view.load() {
                    BaseviewView::handle_occlusion_notification(view.inner_ref(), n);
                }
            });
            view.occlusion_observer.set(Some(observer));

            // Send an initial Resized event so users get the correct scale factor and physical size.
            Self::trigger_deferrable_event(
                view,
                Event::Window(WindowEvent::Resized(Self::fetch_view_size(view.view))),
            );
        });

        (view, state)
    }

    pub fn close(this: ViewRef<Self>) {
        this.state.closed.set(true);
        this.view.removeFromSuperview();

        if let ViewParentingType::Windowed { owned_window: parent_window, running_app } =
            &this.parenting
        {
            if let Some(parent_window) = parent_window.load() {
                parent_window.close();
            }

            if let Some(app) = running_app.load() {
                app.stop(Some(&app));
            }
        }
    }

    pub fn resize(this: ViewRef<Self>, size: Size) {
        // NOTE: macOS gives you a personal rave if you pass in fractional pixels here. Even
        // though the size is in fractional pixels.
        let size = NSSize::new(size.width.round(), size.height.round());

        this.view.setFrameSize(size);
        this.view.setNeedsDisplay(true);

        // When using OpenGL the `NSOpenGLView` needs to be resized separately? Why? Because
        // macOS.
        #[cfg(feature = "opengl")]
        if let Some(gl_context) = this.gl_context.get() {
            gl_context.inner.resize(size);
        }

        // If this is a standalone window then we'll also need to resize the window itself
        if let ViewParentingType::Windowed { owned_window, .. } = &this.parenting {
            if let Some(owned_window) = owned_window.load() {
                owned_window.setContentSize(size);
            }
        }

        Self::view_did_change_backing_properties(this);
    }

    /// Trigger the event immediately and return the event status.
    /// Will panic if `window_handler` is already borrowed (see `trigger_deferrable_event`).
    fn trigger_event(this: ViewRef<Self>, event: Event) -> EventStatus {
        let mut handler = this.window_handler.borrow_mut();
        let Some(handler) = handler.as_mut() else {
            return EventStatus::Ignored;
        };

        let status = handler.on_event(&mut this.into(), event);
        Self::send_deferred_events(this, handler.as_mut());
        status
    }

    /// Trigger the event immediately if `window_handler` can be borrowed mutably,
    /// otherwise add the event to a queue that will be cleared once `window_handler`'s mutable borrow ends.
    /// As this method might result in the event triggering asynchronously, it can't reliably return the event status.
    fn trigger_deferrable_event(this: ViewRef<Self>, event: Event) {
        let Ok(mut handler) = this.window_handler.try_borrow_mut() else {
            this.deferred_events.borrow_mut().push_back(event);
            return;
        };

        let Some(handler) = handler.as_mut() else { return };

        handler.on_event(&mut this.into(), event);
        Self::send_deferred_events(this, handler.as_mut());
    }

    /// (Re)arm the frame timer at `interval` seconds.
    ///
    /// Storing the new handle drops the old one, whose `Drop` removes it from
    /// the run loop — so this replaces the cadence rather than stacking a
    /// second timer on top of it, and is safe to call from inside a frame.
    pub(crate) fn arm_frame_timer(view: &View<Self>, interval: f64) {
        let interval = interval.clamp(crate::MIN_FRAME_INTERVAL, crate::MAX_FRAME_INTERVAL);
        let timer_view = Weak::new(view);
        view.inner_ref().frame_timer.set(TimerHandle::new(interval, move || {
            if let Some(view) = timer_view.load() {
                Self::trigger_frame(view.inner_ref());
            }
        }));
    }

    /// Run one frame, unless one is already running.
    ///
    /// The timer is not the only caller — `handle_occlusion_notification`
    /// paints from inside a notification, and a notification can in principle
    /// be delivered while a frame holds the handler. Taking the borrow
    /// fallibly rather than with `borrow_mut` is what makes that safe: a
    /// panicking borrow would turn a re-entrant delivery into a crash.
    ///
    /// What the dropped frame costs in that case is a tick, not more: the
    /// frame in flight paints, and the event queued behind it is drained when
    /// it finishes. It is only this direction that is covered — `trigger_event`
    /// still borrows infallibly, and says so.
    fn trigger_frame(this: ViewRef<Self>) {
        // Before the frame, so a pointer that has quietly stopped being ours is
        // gone by the time anything is drawn from where it was. The button
        // first only to order the two within the settle: a release belongs
        // ahead of the exit it precedes, the same way `report_release` sends
        // one before paying a withheld exit. Neither reads the other's work —
        // `settle_pointer_exit` asks the OS, not `buttons_reported_down` — so
        // the other order would cost no time, just deliver `CursorLeft` first.
        Self::settle_stuck_buttons(this);
        Self::settle_pointer_exit(this);

        let Ok(mut handler) = this.window_handler.try_borrow_mut() else { return };
        let Some(handler) = handler.as_mut() else { return };

        handler.on_frame(&mut this.into());
        Self::send_deferred_events(this, handler.as_mut());
    }

    /// Pay an exit `mouse_exited` held back, once no button is down.
    ///
    /// Called from every mouse-up, which is the ordinary way a drag off the
    /// window ends, and from the frame tick, which is the only way the ones
    /// that never reach us do. A release goes missing whenever the press was
    /// not ours to begin with — the pointer crossing the view mid-drag gets
    /// enter and exit from the tracking area but no buttons at all — and
    /// whenever the host takes the release out of our hands. Either way the
    /// button is up and the OS says so, so the tick catches it within a frame.
    fn settle_pointer_exit(this: ViewRef<Self>) {
        if !this.exit_withheld.get() || NSEvent::pressedMouseButtons() != 0 {
            return;
        }
        this.exit_withheld.set(false);
        Self::trigger_deferrable_event(this, Event::Mouse(MouseEvent::CursorLeft));
    }

    /// Report the release of a button the OS says is no longer held.
    ///
    /// A press this view reported is owed a release, and AppKit does not always
    /// pay: a `mouseUp:` handed to a host popup, a modal that opens under the
    /// finger, a gesture the host takes out of our hands — the press is ours,
    /// the release is delivered somewhere else, and nothing ever tells the view.
    /// The handler is then left believing a button is down forever.
    ///
    /// What that costs is out of all proportion to how obscure it sounds,
    /// because a consumer's drag state is global: egui gates EVERY `ScrollArea`
    /// on `dragged_id().is_none()`, so one unreleased press silently stops the
    /// wheel in every scrollable pane at once, with the window focused and the
    /// pointer sitting right over the pane that will not move.
    ///
    /// The two guards already here do not reach it. `settle_pointer_exit` waits
    /// on an exit that never happens if the pointer never leaves, and a consumer
    /// that ends its drags on lost focus never sees focus go. Both are about the
    /// POINTER; this is about the BUTTON, and a stuck button is invisible in
    /// terms of either.
    ///
    /// So ask the OS, which is the same authority `settle_pointer_exit` already
    /// trusts over any count of the presses this view has seen. Ordinary
    /// gestures are untouched: through a real drag, however far it wanders, the
    /// button reads down and nothing is synthesised.
    fn settle_stuck_buttons(this: ViewRef<Self>) {
        let owed =
            buttons_owed_release(this.buttons_reported_down.get(), NSEvent::pressedMouseButtons());
        // Stuck at this settle AND the one before it. A single one is not
        // evidence: the hardware goes up before the `mouseUp:` queued behind it
        // is dispatched, so a fast enough click reads exactly like a stuck
        // button once.
        let confirmed = owed & this.buttons_seen_up.replace(owed);
        if confirmed == 0 {
            return;
        }

        this.buttons_reported_down.set(this.buttons_reported_down.get() & !confirmed);
        let modifiers = make_modifiers(this.keyboard_state.last_mods());
        for (bit, button) in
            [(0, MouseButton::Left), (1, MouseButton::Right), (2, MouseButton::Middle)]
        {
            if confirmed & (1 << bit) != 0 {
                Self::trigger_deferrable_event(
                    this,
                    Event::Mouse(ButtonReleased { button, modifiers }),
                );
            }
        }
    }

    /// Report a press, and record the release it now owes the handler.
    fn report_press(this: ViewRef<Self>, button: MouseButton, event: &NSEvent) {
        this.buttons_reported_down.set(this.buttons_reported_down.get() | button_bit(button));
        Self::trigger_event(
            this,
            Event::Mouse(ButtonPressed {
                button,
                modifiers: make_modifiers(event.modifierFlags()),
            }),
        );
    }

    /// Report a release, unless the debt is already paid.
    ///
    /// A `mouseUp:` for a bit that is no longer set is dropped rather than
    /// passed on — the release `settle_stuck_buttons` gave up on and sent
    /// itself, or the second of two buttons sharing bit 2. Sending it twice
    /// would end the NEXT gesture as well: a second release against a fresh
    /// press is how a click lands on whatever the pointer moved to since.
    fn report_release(this: ViewRef<Self>, button: MouseButton, event: &NSEvent) {
        let owed = this.buttons_reported_down.get();
        if owed & button_bit(button) != 0 {
            this.buttons_reported_down.set(owed & !button_bit(button));
            Self::trigger_event(
                this,
                Event::Mouse(ButtonReleased {
                    button,
                    modifiers: make_modifiers(event.modifierFlags()),
                }),
            );
        }
        // After the release, never before it: the gesture ends where the
        // pointer still is, and only then is the pointer gone.
        Self::settle_pointer_exit(this);
    }

    fn send_deferred_events(this: ViewRef<Self>, window_handler: &mut dyn WindowHandler) {
        let mut window = this.into();
        loop {
            let next_event = { this.deferred_events.borrow_mut().pop_front() };
            if let Some(event) = next_event {
                window_handler.on_event(&mut window, event);
            } else {
                break;
            }
        }
    }

    fn handle_occlusion_notification(this: ViewRef<Self>, notification: &NSNotification) {
        let Some(window) = this.view.window() else { return };
        let Some(notification_object) = notification.object().and_then(|o| o.downcast().ok())
        else {
            return;
        };

        // Unlike focus, occlusion is a property of the whole window: no
        // first-responder check — every embedded view wants to know.
        if window != notification_object {
            return;
        }

        let visible = window.occlusionState().contains(NSWindowOcclusionState::Visible);
        // A notification that reports the state we are already in costs a
        // whole off-cadence frame below, so it stops here. macOS posts this on
        // change, but "change" is the WINDOW's, and the notification is
        // delivered to every observer of it — nothing promises the state read
        // back differs from the last one this view saw.
        if this.window_visible.replace(visible) == visible {
            return;
        }
        Self::trigger_deferrable_event(this, Event::Window(WindowEvent::Occluded(!visible)));

        if visible {
            // Re-exposed, and what is on screen is the drawable from before
            // the window was hidden — the compositor keeps showing it until
            // something presents over it, and nothing can present while the
            // window is occluded (a drawable is refused, so the whole time it
            // was hidden every frame was skipped).
            //
            // So the ghost lives exactly as long as it takes to get one frame
            // out, and this notification is the earliest that can happen: the
            // occlusion state it reports is the same state the drawable
            // request checks, so no earlier signal — activation, key window —
            // buys a frame that would be allowed to present.
            //
            // Painting HERE rather than leaving it to the next timer tick is
            // therefore the whole of the fix. The event above marks the
            // handler for repaint; the tick that would act on it is up to a
            // frame interval away, and that interval IS the ghost: measured
            // at 11-14 ms against a ~15 ms timer, and it scales with the
            // interval, so a frame-rate cap makes it worse.
            //
            // The dirty mark is for whatever AppKit draws itself, which for a
            // surface-backed window is nothing; the frame is what puts new
            // pixels up.
            this.view.setNeedsDisplay(true);
            Self::trigger_frame(this);
        }
    }

    fn fetch_view_size(view: &NSView) -> WindowInfo {
        let ns_window = view.window();

        let scale_factor: f64 = ns_window.map(|w| w.backingScaleFactor()).unwrap_or(1.0);

        let bounds = view.bounds();

        WindowInfo::from_logical_size(
            Size::new(bounds.size.width, bounds.size.height),
            scale_factor,
        )
    }
}

impl Drop for BaseviewView {
    fn drop(&mut self) {
        self.state.closed.set(true);
    }
}

impl ViewImpl for BaseviewView {
    fn become_first_responder(this: ViewRef<Self>) -> bool {
        let Some(window) = this.view.window() else {
            return true;
        };

        if window.isKeyWindow() {
            Self::trigger_deferrable_event(this, Event::Window(WindowEvent::Focused));
        }

        true
    }

    fn resign_first_responder(this: ViewRef<Self>) -> bool {
        Self::trigger_deferrable_event(this, Event::Window(WindowEvent::Unfocused));
        true
    }

    fn window_should_close(this: ViewRef<Self>) -> bool {
        Self::trigger_event(this, Event::Window(WindowEvent::WillClose));
        Self::close(this);

        true
    }

    fn view_did_change_backing_properties(this: ViewRef<Self>) {
        let new_window_info = Self::fetch_view_size(this.view);
        let window_info = this.state.window_info.get();

        // Only send the event when the window's size has actually changed to be in line with the
        // other platform implementations
        if new_window_info.physical_size() != window_info.physical_size() {
            this.state.window_info.set(new_window_info);
            Self::trigger_deferrable_event(
                this,
                Event::Window(WindowEvent::Resized(new_window_info)),
            );
        }
    }

    /// `hitTest:` override that collapses hits on baseview's internal
    /// OpenGL render subview to this NSView.
    ///
    /// `src/gl/gl` attaches an `NSOpenGLView` as a subview of this
    /// view so the GL context is isolated from event handling. The side
    /// effect is that `[NSView hitTest:]` returns the GL subview for
    /// every click inside our frame — `NSOpenGLView` inherits the
    /// default `acceptsFirstMouse:` which returns `NO`, so AppKit treats
    /// the first click in a non-key window as an activation click and
    /// never dispatches `mouseDown:`. That's the "first click dead zone"
    /// symptom reported in baseview#129 / #202 / #169.
    ///
    /// Fix: if the hit lands on our own GL render subview (pointer
    /// equality against the `NSOpenGLView` stored in `GlContext`),
    /// collapse the result to `self`. AppKit then asks US about
    /// `acceptsFirstMouse:` (we return `YES`), and `mouseDown:` is
    /// dispatched on the first click. Hits on any other subview pass
    /// through unchanged — we only redirect our own render child, not
    /// anything the consumer may add.
    ///
    /// No-op without the `opengl` feature: there's no GL subview to
    /// collapse, so the override pass-through is equivalent to the
    /// default implementation.
    fn hit_test(this: ViewRef<'_, Self>, point: NSPoint) -> Option<&NSView> {
        let superclass = this.view.class().superclass().unwrap();

        // SAFETY: Our superclass is NSView
        let super_result: Option<&NSView> =
            unsafe { msg_send![super(this.view, superclass), hitTest: point] };
        let super_result = super_result?;

        #[cfg(feature = "opengl")]
        {
            if let Some(gl_context) = this.gl_context.get() {
                if *super_result == **gl_context.inner.0.view {
                    return Some(this.view);
                }
            }
        }

        Some(super_result)
    }

    fn view_will_move_to_window(this: ViewRef<Self>, new_window: Option<&NSWindow>) {
        let tracking_areas = this.view.trackingAreas();

        match new_window {
            None => {
                if tracking_areas.count() > 0 {
                    let tracking_area = tracking_areas.objectAtIndex(0);
                    this.view.removeTrackingArea(&tracking_area);
                }
            }
            Some(new_window) => {
                if tracking_areas.is_empty() {
                    let tracking_area = new_tracking_area(this.view);
                    this.view.addTrackingArea(&tracking_area);
                }

                new_window.setAcceptsMouseMovedEvents(true);
                new_window.makeFirstResponder(Some(this.view));
            }
        }

        unsafe {
            let superclass = msg_send![this.view, superclass];

            let () = msg_send![super(this.view, superclass), viewWillMoveToWindow: new_window];
        }
    }

    fn update_tracking_areas(this: ViewRef<Self>) {
        let tracking_areas = this.view.trackingAreas();
        if tracking_areas.count() > 0 {
            let tracking_area = tracking_areas.objectAtIndex(0);
            this.view.removeTrackingArea(&tracking_area);
        }

        let tracking_area = new_tracking_area(this.view);

        this.view.addTrackingArea(&tracking_area);
    }

    fn cursor_update(this: ViewRef<Self>, _event: &NSEvent) {
        // Sent via the tracking area (CursorUpdate | ActiveInActiveApp),
        // which works even while the window is NOT key — cursor RECTS only
        // activate once the window becomes key (i.e. after a click), so
        // this path covers the freshly-opened, not-yet-clicked window.
        Cursor::from(this.state.mouse_cursor.get()).load().set();
    }

    fn reset_cursor_rects(this: ViewRef<Self>) {
        // Re-assert the desired cursor over the whole view whenever AppKit
        // rebuilds cursor rects; without this, the host window's own rects
        // take over after any invalidation.
        let cursor = Cursor::from(this.state.mouse_cursor.get());
        this.view.addCursorRect_cursor(this.view.bounds(), &cursor.load());
    }

    fn mouse_moved(this: ViewRef<Self>, event: &NSEvent) {
        let point = this.view.convertPoint_fromView(event.locationInWindow(), None);

        let position = Point { x: point.x, y: point.y };

        Self::trigger_event(
            this,
            Event::Mouse(MouseEvent::CursorMoved {
                position,
                modifiers: make_modifiers(event.modifierFlags()),
            }),
        );
    }

    fn scroll_wheel(this: ViewRef<Self>, event: &NSEvent) {
        // Report where the wheel happened before reporting the wheel itself.
        //
        // `scrollWheel:` carries a location but no CursorMoved, and consumers
        // route a wheel by where they last saw the pointer — egui hands it to
        // whatever its stored pointer position is over. That position can be
        // absent or stale, because `mouseMoved:` only arrives through the
        // tracking area (`ActiveInActiveApp`) and `mouseExited:` clears it: a
        // window opened with the cursor already inside it, or one whose app was
        // not frontmost while the cursor moved in, has never been told where the
        // pointer is. The wheel then did nothing at all, or landed on whatever
        // was under the LAST known position — scrolling one pane while the
        // cursor sat over another.
        //
        // The event's own location is always right, so send it first and the
        // wheel is applied where the cursor actually is. Gated on the location
        // being inside the view so inertial scrolling that outlives the pointer
        // leaving cannot resurrect it.
        let point = this.view.convertPoint_fromView(event.locationInWindow(), None);
        let bounds = this.view.bounds();
        let inside = point.x >= bounds.origin.x
            && point.y >= bounds.origin.y
            && point.x <= bounds.origin.x + bounds.size.width
            && point.y <= bounds.origin.y + bounds.size.height;
        if inside {
            Self::trigger_event(
                this,
                Event::Mouse(MouseEvent::CursorMoved {
                    position: Point { x: point.x, y: point.y },
                    modifiers: make_modifiers(event.modifierFlags()),
                }),
            );
        }

        let x = event.scrollingDeltaX() as f32;
        let y = event.scrollingDeltaY() as f32;

        let delta = if event.hasPreciseScrollingDeltas() {
            ScrollDelta::Pixels { x, y }
        } else {
            ScrollDelta::Lines { x, y }
        };

        Self::trigger_event(
            this,
            Event::Mouse(MouseEvent::WheelScrolled {
                delta,
                modifiers: make_modifiers(event.modifierFlags()),
            }),
        );
    }

    fn dragging_entered(
        this: ViewRef<Self>, sender: Option<&ProtocolObject<dyn NSDraggingInfo>>,
    ) -> NSDragOperation {
        let modifiers = this.keyboard_state.last_mods();
        let drop_data = get_drop_data(sender);

        let event = MouseEvent::DragEntered {
            position: get_drag_position(sender),
            modifiers: make_modifiers(modifiers),
            data: drop_data,
        };

        on_event(this, event)
    }

    fn dragging_updated(
        this: ViewRef<Self>, sender: Option<&ProtocolObject<dyn NSDraggingInfo>>,
    ) -> NSDragOperation {
        let modifiers = this.keyboard_state.last_mods();
        let drop_data = get_drop_data(sender);

        let event = MouseEvent::DragMoved {
            position: get_drag_position(sender),
            modifiers: make_modifiers(modifiers),
            data: drop_data,
        };

        on_event(this, event)
    }

    fn prepare_for_drag_operation(
        _this: ViewRef<Self>, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>,
    ) -> bool {
        // Always accept drag operation if we get this far
        // This function won't be called unless dragging_entered/updated
        // has returned an acceptable operation
        true
    }

    fn perform_drag_operation(
        this: ViewRef<Self>, sender: Option<&ProtocolObject<dyn NSDraggingInfo>>,
    ) -> bool {
        let modifiers = this.keyboard_state.last_mods();
        let drop_data = get_drop_data(sender);

        let event = MouseEvent::DragDropped {
            position: get_drag_position(sender),
            modifiers: make_modifiers(modifiers),
            data: drop_data,
        };

        let event_status = Self::trigger_event(this, Event::Mouse(event));

        matches!(event_status, EventStatus::AcceptDrop(_))
    }

    fn dragging_exited(this: ViewRef<Self>, _sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) {
        on_event(this, MouseEvent::DragLeft);
    }

    fn handle_notification(this: ViewRef<Self>, notification: &NSNotification) {
        let Some(window) = this.view.window() else { return };
        // The subject of the notification, in this case an NSWindow object.
        let Some(notification_object) = notification.object().and_then(|o| o.downcast().ok())
        else {
            return;
        };

        // Only trigger focus events if the NSWindow that's being notified about is our window,
        // and if the window's first responder is our NSView.
        if window != notification_object {
            return;
        }

        let Some(first_responder) = window.firstResponder() else { return };

        // If the first responder isn't our NSView, the focus events will instead be triggered
        // by the becomeFirstResponder and resignFirstResponder methods on the NSView itself.
        if !this.view.isEqual(Some(&first_responder)) {
            return;
        }

        Self::trigger_event(
            this,
            Event::Window(if window.isKeyWindow() {
                WindowEvent::Focused
            } else {
                WindowEvent::Unfocused
            }),
        );
    }

    fn mouse_down(this: ViewRef<Self>, event: &NSEvent) {
        Self::report_press(this, MouseButton::Left, event);
    }

    fn mouse_up(this: ViewRef<Self>, event: &NSEvent) {
        Self::report_release(this, MouseButton::Left, event);
    }

    fn right_mouse_down(this: ViewRef<Self>, event: &NSEvent) {
        Self::report_press(this, MouseButton::Right, event);
    }

    fn right_mouse_up(this: ViewRef<Self>, event: &NSEvent) {
        Self::report_release(this, MouseButton::Right, event);
    }

    fn other_mouse_down(this: ViewRef<Self>, event: &NSEvent) {
        Self::report_press(this, MouseButton::Middle, event);
    }

    fn other_mouse_up(this: ViewRef<Self>, event: &NSEvent) {
        Self::report_release(this, MouseButton::Middle, event);
    }

    fn mouse_entered(this: ViewRef<Self>) {
        this.state.mouse_inside.set(true);
        // Back inside before the exit was ever reported: as far as the handler
        // knows the pointer never left, so there is no entry to announce
        // either. The pair stays balanced (see `mouse_exited`).
        if this.exit_withheld.replace(false) {
            return;
        }
        Self::trigger_event(this, Event::Mouse(MouseEvent::CursorEntered));
    }

    fn mouse_exited(this: ViewRef<Self>) {
        this.state.mouse_inside.set(false);
        // A button down means the drag is still ours, so the exit waits.
        //
        // The tracking area is `EnabledDuringMouseDrag`, so `mouseExited:`
        // arrives mid-drag — while AppKit goes on sending every `mouseDragged:`
        // and the closing `mouseUp:` to the view the press landed in, wherever
        // the pointer has got to. Reporting the exit is what breaks the drag:
        // a consumer told the pointer is gone stops following it (egui's
        // `PointerGone`), and a slider dragged a pixel past the window edge
        // lets go under the hand still holding it.
        //
        // Held back rather than dropped, because the exit is real once the
        // button is up — `settle_pointer_exit` is where it is paid, and it
        // reads the button state from the OS rather than from a count of the
        // presses this view has seen, which a release delivered elsewhere
        // would leave wrong forever.
        if NSEvent::pressedMouseButtons() != 0 {
            this.exit_withheld.set(true);
            return;
        }
        Self::trigger_event(this, Event::Mouse(MouseEvent::CursorLeft));
    }

    fn key_down(this: ViewRef<Self>, event: &NSEvent) {
        if let Some(key_event) = this.keyboard_state.process_native_event(event) {
            let status = Self::trigger_event(this, Event::Keyboard(key_event));

            if let EventStatus::Ignored = status {
                unsafe {
                    let superclass = msg_send![this.view, superclass];

                    let () = msg_send![super(this.view, superclass), keyDown:event];
                }
            }
        }
    }

    fn key_up(this: ViewRef<Self>, event: &NSEvent) {
        if let Some(key_event) = this.keyboard_state.process_native_event(event) {
            let status = Self::trigger_event(this, Event::Keyboard(key_event));

            if let EventStatus::Ignored = status {
                unsafe {
                    let superclass = msg_send![this.view, superclass];

                    let () = msg_send![super(this.view, superclass), keyUp:event];
                }
            }
        }
    }

    fn flags_changed(this: ViewRef<Self>, event: &NSEvent) {
        if let Some(key_event) = this.keyboard_state.process_native_event(event) {
            let status = Self::trigger_event(this, Event::Keyboard(key_event));

            if let EventStatus::Ignored = status {
                unsafe {
                    let superclass = msg_send![this.view, superclass];

                    let () = msg_send![super(this.view, superclass), flagsChanged:event];
                }
            }
        }
    }
}

/// The bit standing for a button this view reports, in
/// `NSEvent::pressedMouseButtons`' order.
///
/// Every button but the first two shares one bit, because that is how they
/// reach the handler: `otherMouseDown:` reports the third, fourth and fifth
/// alike as `MouseButton::Middle`, so this side of the comparison cannot tell
/// them apart either (see `buttons_owed_release`).
fn button_bit(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0b001,
        MouseButton::Right => 0b010,
        _ => 0b100,
    }
}

/// Which of the buttons a view has reported pressed are no longer held,
/// according to `NSEvent::pressedMouseButtons`.
///
/// Both sides are bitmasks in that method's order — bit 0 the left button, bit
/// 1 the right — and they part company above that. AppKit gives every remaining
/// button a bit of its own (bit 2 the third button, bit 3 the fourth, on up a
/// mouse with side buttons), while `button_bit` has only the one to give them.
/// So the tail is folded to match: ANY button above the first two being down
/// counts as that bit held. Comparing the masks unfolded would read a thumb
/// button held on bit 3 as bit 2 standing empty, and end a gesture the hand is
/// still making.
fn buttons_owed_release(reported_down: u8, os_pressed: usize) -> u8 {
    let os_down = (os_pressed as u8 & 0b11) | u8::from(os_pressed & !0b11 != 0) << 2;
    reported_down & !os_down
}

#[cfg(test)]
mod tests {
    use super::{button_bit, buttons_owed_release};
    use crate::MouseButton;

    /// A button held is a button owed nothing, however long the drag runs.
    #[test]
    fn a_button_the_os_still_holds_is_never_released() {
        for (button, os) in
            [(MouseButton::Left, 0b001), (MouseButton::Right, 0b010), (MouseButton::Middle, 0b100)]
        {
            let bit = button_bit(button);
            assert_eq!(
                buttons_owed_release(bit, os),
                0,
                "{button:?} was let go of while the OS still had it down",
            );
        }
    }

    /// The case the whole repair exists for: the press was reported, the OS
    /// says nothing is down, and no `mouseUp:` ever came.
    #[test]
    fn a_press_the_os_has_forgotten_is_owed_its_release() {
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            let bit = button_bit(button);
            assert_eq!(
                buttons_owed_release(bit, 0),
                bit,
                "{button:?} stayed down forever after its release went missing",
            );
        }
    }

    /// One button going up does not take another down with it.
    #[test]
    fn only_the_button_the_os_let_go_of_is_released() {
        let (left, right) = (button_bit(MouseButton::Left), button_bit(MouseButton::Right));
        assert_eq!(buttons_owed_release(left | right, 0b010), left);
        assert_eq!(buttons_owed_release(left | right, 0b001), right);
    }

    /// A side button holds the one bit every button past the first two shares,
    /// so a mouse with more than three of them keeps its drag.
    ///
    /// `otherMouseDown:` reports the fourth button as `Middle` — bit 2 here —
    /// while the OS has it on bit 3. Comparing the masks as they come would
    /// find bit 2 empty and let go mid-gesture.
    #[test]
    fn a_button_past_the_third_holds_the_middle_bit() {
        let middle = button_bit(MouseButton::Middle);
        for os in [0b100, 0b1000, 0b10000, 0b1 << 20] {
            assert_eq!(
                buttons_owed_release(middle, os),
                0,
                "a button held on the OS mask {os:b} read as let go",
            );
        }
        assert_eq!(buttons_owed_release(middle, 0b011), middle, "no other button was down");
    }

    /// Nothing pressed here is nothing to settle, whatever the OS reports —
    /// a drag begun in the host's own window is not this view's to end.
    #[test]
    fn a_press_this_view_never_saw_is_not_its_to_release() {
        for os in [0, 0b001, 0b010, 0b100, 0b111] {
            assert_eq!(buttons_owed_release(0, os), 0);
        }
    }
}

/// Info:
/// https://developer.apple.com/documentation/appkit/nstrackingarea
/// https://developer.apple.com/documentation/appkit/nstrackingarea/options
/// https://developer.apple.com/documentation/appkit/nstrackingareaoptions
fn new_tracking_area(this: &NSView) -> Retained<NSTrackingArea> {
    let options = NSTrackingAreaOptions::MouseEnteredAndExited
        | NSTrackingAreaOptions::MouseMoved
        | NSTrackingAreaOptions::CursorUpdate
        | NSTrackingAreaOptions::ActiveInActiveApp
        | NSTrackingAreaOptions::InVisibleRect
        | NSTrackingAreaOptions::EnabledDuringMouseDrag;

    // SAFETY: `this` is of the correct type (NSView)
    unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            this.bounds(),
            options,
            Some(this),
            None,
        )
    }
}

fn get_drag_position(sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) -> Point {
    let point = match sender {
        Some(sender) => sender.draggingLocation(),
        None => NSPoint::ZERO,
    };

    Point::new(point.x, point.y)
}

fn get_drop_data(sender: Option<&ProtocolObject<dyn NSDraggingInfo>>) -> DropData {
    let Some(sender) = sender else {
        return DropData::None;
    };

    let pasteboard = sender.draggingPasteboard();
    let Some(file_list) = pasteboard.propertyListForType(unsafe { NSFilenamesPboardType }) else {
        return DropData::None;
    };

    let Ok(file_list) = file_list.downcast::<NSArray>() else {
        return DropData::None;
    };

    let files = file_list
        .into_iter()
        .filter_map(|s| s.downcast::<NSString>().ok())
        .map(|s| s.to_string().into())
        .collect();

    DropData::Files(files)
}

fn on_event(this: ViewRef<BaseviewView>, event: MouseEvent) -> NSDragOperation {
    let event_status = BaseviewView::trigger_event(this, Event::Mouse(event));
    match event_status {
        EventStatus::AcceptDrop(DropEffect::Copy) => NSDragOperation::Copy,
        EventStatus::AcceptDrop(DropEffect::Move) => NSDragOperation::Move,
        EventStatus::AcceptDrop(DropEffect::Link) => NSDragOperation::Link,
        EventStatus::AcceptDrop(DropEffect::Scroll) => NSDragOperation::Generic,
        _ => NSDragOperation::None,
    }
}
