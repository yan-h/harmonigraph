//! AppKit controls created only when the companion editor opens. Timer refresh
//! is cosmetic; it never admits a voice, acknowledges output or touches audio.
//!
//! Two controls survive the fault cut: the delay slider and Reset. The pairing
//! popup went with saved-UUID pairing and the offset field went with routing
//! calibration, and neither had a value left to carry.
use super::{plugin::TuneParams, session, setup, DELAY_MULTIPLIER_MAX};
use nice_plug::prelude::*;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSButton, NSSlider, NSTextField, NSView};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer,
};
use std::any::Any;
use std::cell::OnceCell;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct NativeEditor {
    pub shared: Arc<setup::Shared>,
    pub params: Arc<TuneParams>,
}
struct Widgets {
    view: Retained<NSView>,
    delay: Retained<NSSlider>,
    delay_label: Retained<NSTextField>,
    status: Retained<NSTextField>,
}
struct Ivars {
    shared: Arc<setup::Shared>,
    params: Arc<TuneParams>,
    context: Arc<dyn GuiContext>,
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
        // The slider is not continuous, so AppKit sends this once, when the
        // drag ends. One finished gesture is one latency change, and one
        // reactivation request; the intermediate values never leave the knob.
        #[unsafe(method(delay:))]
        fn delay(&self, slider: &NSSlider) {
            let vars = self.ivars();
            let value = slider.integerValue() as i32;
            let setter = ParamSetter::new(&*vars.context);
            setter.begin_set_parameter(&vars.params.delay);
            setter.set_parameter(&vars.params.delay, value);
            setter.end_set_parameter(&vars.params.delay);
            self.refresh_status();
        }
        #[unsafe(method(reset:))]
        fn reset(&self, _: &NSButton) {
            self.ivars().shared.request_reset();
            self.refresh_status();
        }
        #[unsafe(method(refresh:))]
        fn refresh(&self, _: &NSTimer) { self.refresh_status(); }
    }
);
impl Actions {
    fn refresh_status(&self) {
        let vars = self.ivars();
        let w = vars.widgets.get().unwrap();
        let status = vars.shared.status();
        let requested = vars.params.delay.value().max(1) as u32;
        let active = vars.shared.active_multiplier.load(Ordering::Acquire);
        let (rate, frames) = vars.shared.format();
        w.delay.setIntegerValue(requested as isize);
        w.delay_label.setStringValue(&NSString::from_str(&setup::delay_text(
            requested, active, frames, rate,
        )));
        let session = session::session();
        let pairing = if status & session::NO_HUB != 0 {
            "No Harmonigraph in this process"
        } else if status & session::NO_ROW != 0 {
            "No free row: sixteen Tunes are already paired"
        } else {
            "Paired"
        };
        let deadline = setup::deadline_text(status, vars.shared.misses.load(Ordering::Relaxed));
        w.status.setStringValue(&NSString::from_str(&format!(
            "{pairing}\n{}\n{deadline}\nSession epoch {} · {} Hz · ≤{frames} frames",
            session::status_text(status),
            session.epoch(),
            rate,
        )));
    }
}
fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}
impl Editor for NativeEditor {
    fn spawn(
        &self,
        parent: ParentWindowHandle,
        context: Arc<dyn GuiContext>,
    ) -> Box<dyn Any + Send> {
        let mtm = MainThreadMarker::new().expect("CLAP editor main thread");
        let ParentWindowHandle::AppKitNsView(parent) = parent else {
            return Box::new(());
        };
        let actions = Actions::alloc(mtm).set_ivars(Ivars {
            shared: self.shared.clone(),
            params: self.params.clone(),
            context,
            widgets: OnceCell::new(),
        });
        let actions: Retained<Actions> = unsafe { msg_send![super(actions), init] };
        let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 520.0, 360.0));
        let label = |text: &str, y: f64| {
            let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
            field.setFrame(rect(16.0, y, 485.0, 26.0));
            view.addSubview(&field);
            field
        };
        label("Harmonigraph Tune", 322.0);
        let delay_label =
            NSTextField::wrappingLabelWithString(&NSString::from_str("Tuning delay"), mtm);
        delay_label.setFrame(rect(16.0, 286.0, 485.0, 34.0));
        delay_label.setMaximumNumberOfLines(0);
        view.addSubview(&delay_label);
        // Stepped and discontinuous: the tick marks are the only values it can
        // take, and the action arrives once the drag is over.
        let delay = unsafe {
            NSSlider::sliderWithValue_minValue_maxValue_target_action(
                f64::from(self.params.delay.value()),
                1.0,
                f64::from(DELAY_MULTIPLIER_MAX),
                Some(&actions),
                Some(sel!(delay:)),
                mtm,
            )
        };
        delay.setFrame(rect(16.0, 258.0, 485.0, 24.0));
        delay.setNumberOfTickMarks(DELAY_MULTIPLIER_MAX as isize);
        delay.setAllowsTickMarkValuesOnly(true);
        delay.setContinuous(false);
        view.addSubview(&delay);
        label("Pairing is automatic: one Harmonigraph per process, no choice to make.", 226.0);
        let reset = unsafe {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Reset voices"),
                Some(&actions),
                Some(sel!(reset:)),
                mtm,
            )
        };
        reset.setFrame(rect(16.0, 194.0, 150.0, 28.0));
        view.addSubview(&reset);
        let status = NSTextField::wrappingLabelWithString(
            &NSString::from_str("Waiting for audio state"),
            mtm,
        );
        status.setFrame(rect(16.0, 25.0, 485.0, 160.0));
        status.setMaximumNumberOfLines(0);
        view.addSubview(&status);
        label(harmonigraph_perf::BUILD_TAG, 5.0);
        actions
            .ivars()
            .widgets
            .set(Widgets { view, delay, delay_label, status })
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
        (520, 360)
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
