//! A track endpoint: identity, participation, visibility and connection status.
//! Ensemble controls and timing live in Harmonigraph's Tuning pane.
use super::{session, setup};
use nice_plug::prelude::*;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSButton, NSButtonType, NSTextField, NSView};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer,
};
use std::any::Any;
use std::cell::OnceCell;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct NativeEditor {
    pub shared: Arc<setup::Shared>,
}
struct Widgets {
    view: Retained<NSView>,
    name: Retained<NSTextField>,
    retune: Retained<NSButton>,
    show: Retained<NSButton>,
    status: Retained<NSTextField>,
}
struct Ivars {
    shared: Arc<setup::Shared>,
    widgets: OnceCell<Widgets>,
}
define_class!(
    // NSObject adds no subclass invariants. All controls and actions are main-thread only.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct Actions;
    unsafe impl NSObjectProtocol for Actions {}
    impl Actions {
        #[unsafe(method(retune:))]
        fn retune(&self, button: &NSButton) {
            self.ivars().shared.set_retune(button.state() != 0);
            self.refresh_status();
        }
        #[unsafe(method(show:))]
        fn show(&self, button: &NSButton) {
            self.ivars().shared.set_show(button.state() != 0);
            self.refresh_status();
        }
        #[unsafe(method(refresh:))]
        fn refresh(&self, _: &NSTimer) { self.refresh_status(); }
    }
);
impl Actions {
    fn refresh_status(&self) {
        let shared = &self.ivars().shared;
        let w = self.ivars().widgets.get().unwrap();
        w.name.setStringValue(&NSString::from_str(&shared.display_name()));
        w.retune.setState(isize::from(shared.retuning() & 1 != 0));
        w.show.setState(isize::from(shared.show.load(Ordering::Acquire)));
        let status = shared.status();
        let connection = if shared.active_multiplier.load(Ordering::Acquire) == 0 {
            "Waiting for host activation"
        } else if status & session::NO_HUB != 0 {
            "No Harmonigraph connected in this process"
        } else if status & session::NO_ROW != 0 {
            "Not connected: all 16 tuner slots are occupied"
        } else {
            "Connected to Harmonigraph"
        };
        let misses = shared.misses.load(Ordering::Relaxed);
        let detail = if status != 0 {
            session::status_text(status)
        } else if misses != 0 {
            format!("{misses} missed corrections — check tuning delay in Harmonigraph")
        } else {
            format!("{} notes held", shared.held.load(Ordering::Relaxed))
        };
        w.status.setStringValue(&NSString::from_str(&format!("{connection}\n{detail}")));
    }
}
fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}
impl Editor for NativeEditor {
    fn spawn(&self, parent: ParentWindowHandle, _: Arc<dyn GuiContext>) -> Box<dyn Any + Send> {
        let mtm = MainThreadMarker::new().expect("CLAP editor main thread");
        let ParentWindowHandle::AppKitNsView(parent) = parent else {
            return Box::new(());
        };
        let actions = Actions::alloc(mtm)
            .set_ivars(Ivars { shared: self.shared.clone(), widgets: OnceCell::new() });
        let actions: Retained<Actions> = unsafe { msg_send![super(actions), init] };
        let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 460.0, 230.0));
        let label = |text: &str, y: f64, height: f64| {
            let field = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
            field.setFrame(rect(16.0, y, 428.0, height));
            field.setMaximumNumberOfLines(0);
            view.addSubview(&field);
            field
        };
        let name = label("Harmonigraph Tune", 196.0, 26.0);
        let button = |title: &str, x: f64, action| {
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(title),
                    Some(&actions),
                    Some(action),
                    mtm,
                )
            };
            button.setButtonType(NSButtonType::Switch);
            button.setFrame(rect(x, 155.0, 190.0, 30.0));
            view.addSubview(&button);
            button
        };
        let retune = button("Retune", 16.0, sel!(retune:));
        let show = button("Show in Harmonigraph", 206.0, sel!(show:));
        label("Retune off passes notes through without influencing tuning.", 119.0, 32.0);
        let status = label("Waiting for audio state", 54.0, 60.0);
        label("Manage instances and timing in Harmonigraph → Tuning.", 16.0, 32.0);
        actions
            .ivars()
            .widgets
            .set(Widgets { view, name, retune, show, status })
            .unwrap_or_else(|_| unreachable!());
        actions.refresh_status();
        let parent = unsafe { &*parent.cast::<NSView>() };
        parent.addSubview(&actions.ivars().widgets.get().unwrap().view);
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                0.25,
                &actions,
                sel!(refresh:),
                None,
                true,
            )
        };
        Box::new(Handle { actions, timer })
    }
    fn size(&self) -> (u32, u32) {
        (460, 230)
    }
    fn set_scale_factor(&self, _: f32) -> bool {
        true
    }
    fn param_value_changed(&self, _: &str, _: f32) {}
    fn param_modulation_changed(&self, _: &str, _: f32) {}
    fn param_values_changed(&self) {}
}
struct Handle {
    actions: Retained<Actions>,
    timer: Retained<NSTimer>,
}
// The CLAP wrapper creates/destroys this editor handle only on its GUI thread.
// Send carries ownership through nice-plug's erased handle, not AppKit access.
unsafe impl Send for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        self.timer.invalidate();
        self.actions.ivars().widgets.get().unwrap().view.removeFromSuperview();
    }
}
