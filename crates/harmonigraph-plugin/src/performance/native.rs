//! AppKit controls created only when the companion editor opens. Timer refresh
//! is cosmetic; it never admits a voice, acknowledges output or touches audio.
use super::{
    registry,
    routing::SavedUuid,
    setup::{self, Routing},
    tune::TuneParams,
};
use nice_plug::prelude::*;
use objc2::rc::Retained;
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSButton, NSPopUpButton, NSSlider, NSTextField, NSView};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSTimer,
};
use std::any::Any;
use std::cell::{Cell, OnceCell, RefCell};
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct NativeEditor {
    pub shared: Arc<setup::Shared>,
    pub params: Arc<TuneParams>,
}
struct Widgets {
    view: Retained<NSView>,
    participation: Retained<NSButton>,
    pairing: Retained<NSPopUpButton>,
    offset: Retained<NSTextField>,
    delay: Retained<NSSlider>,
    delay_label: Retained<NSTextField>,
    status: Retained<NSTextField>,
    choices: RefCell<Vec<SavedUuid>>,
    available: RefCell<Vec<SavedUuid>>,
    accepted_generation: Cell<Option<u64>>,
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
        #[unsafe(method(toggle:))]
        fn toggle(&self, _: &NSButton) {
            let vars = self.ivars();
            let setter = ParamSetter::new(&*vars.context);
            setter.begin_set_parameter(&vars.params.participating);
            setter.set_parameter(&vars.params.participating, !vars.params.participating.value());
            setter.end_set_parameter(&vars.params.participating);
        }
        #[unsafe(method(apply:))]
        fn apply(&self, _: &NSButton) { self.apply_value(true); }
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
            let shared = &self.ivars().shared;
            if let Err(error) = shared.apply(shared.value().routing, true) {
                self.ivars().widgets.get().unwrap().status.setStringValue(&NSString::from_str(error));
            } else {
                self.refresh_status();
            }
        }
        #[unsafe(method(refresh:))]
        fn refresh(&self, _: &NSTimer) { self.refresh_status(); }
    }
);
impl Actions {
    fn apply_value(&self, reset: bool) {
        // A host restore can arrive between cosmetic timer ticks and a click.
        // Rebind all draft controls before interpreting them as an edit.
        self.refresh_status();
        let vars = self.ivars();
        let w = vars.widgets.get().unwrap();
        let Routing::Source(mut value) = vars.shared.value().routing else {
            return;
        };
        let Ok(offset) = w.offset.stringValue().to_string().parse::<i64>() else {
            w.status.setStringValue(&NSString::from_str("Enter a signed sample offset."));
            return;
        };
        value.calibration = super::clock::Calibration { offset };
        let selection = w.pairing.indexOfSelectedItem();
        value.selected = if selection <= 0 {
            None
        } else {
            w.choices.borrow().get(selection as usize - 1).copied()
        };
        if let Err(error) = vars.shared.apply(Routing::Source(value), reset) {
            w.status.setStringValue(&NSString::from_str(error));
        }
    }
    fn refresh_status(&self) {
        let vars = self.ivars();
        let w = vars.widgets.get().unwrap();
        w.participation.setState(if vars.params.participating.value() { 1 } else { 0 });
        let pairing = vars.shared.source.as_ref().unwrap().status.load(Ordering::Acquire);
        let name = match pairing {
            registry::MISSING => "No matching hub",
            registry::AMBIGUOUS => "Ambiguous hub UUID",
            registry::OVERCAPACITY => "Session capacity reached",
            registry::OFFERED => "Waiting for audio enrollment",
            registry::ATTACHED => "Attached",
            _ => "Unavailable",
        };
        let status = vars.shared.status.load(Ordering::Acquire);
        let value = vars.shared.value();
        let diagnostics = setup::diagnostics_text(status);
        let pending = if value.generation > vars.shared.applied.load(Ordering::Acquire) {
            " · setup pending: old output must settle"
        } else {
            ""
        };
        let requested = vars.params.delay.value().max(1) as u32;
        let active = vars.shared.active_multiplier.load(Ordering::Acquire);
        w.delay.setIntegerValue(requested as isize);
        let adopted = vars.shared.adopted();
        w.delay_label.setStringValue(&NSString::from_str(&setup::delay_text(
            requested,
            active,
            adopted.map_or(0, |adopted| adopted.max_frames),
            adopted.map_or(0.0, |adopted| adopted.sample_rate),
        )));
        // Startup, a Hub this Tune has not attached to, and a setup still
        // settling are all reasons a note can be late that raising the delay
        // would not fix, so the deadline line names them instead.
        let starting = !pending.is_empty()
            || adopted.is_none_or(|adopted| !adopted.valid)
            || pairing != registry::ATTACHED;
        let deadline = setup::deadline_text(
            status,
            Some(pairing),
            starting,
            vars.shared.deadline_misses.load(Ordering::Relaxed),
            vars.shared.extra_delay.load(Ordering::Relaxed),
            adopted.map_or(0.0, |adopted| adopted.sample_rate),
        );
        if let Some(adopted) = adopted {
            let calibration = adopted.calibration;
            w.status.setStringValue(&NSString::from_str(&format!(
                "{name}{pending}\n{diagnostics}\n{deadline}\nAdopted {}: offset {} · Host: {} Hz · ≤{} frames\n{}",
                adopted.generation,
                calibration.offset,
                adopted.sample_rate,
                adopted.max_frames,
                if adopted.valid {
                    "Host clock ready"
                } else {
                    "Host clock unavailable; reinitialize after resolving the clock failure"
                }
            )));
        } else {
            w.status.setStringValue(&NSString::from_str(&format!(
                "{name}{pending}\n{diagnostics}\n{deadline}\nWaiting for the first audio clock boundary"
            )));
        }
        let available = registry::global().lock().unwrap().candidates();
        let Routing::Source(source) = value.routing else {
            return;
        };
        let restored = w.accepted_generation.get() != Some(value.generation);
        let selected = if restored {
            let calibration = source.calibration;
            w.offset.setStringValue(&NSString::from_str(&calibration.offset.to_string()));
            source.selected
        } else {
            let index = w.pairing.indexOfSelectedItem();
            (index > 0).then(|| w.choices.borrow().get(index as usize - 1).copied()).flatten()
        };
        let mut choices = available.clone();
        if let Some(uuid) = selected {
            if !choices.contains(&uuid) {
                choices.push(uuid);
            }
        }
        if restored || *w.available.borrow() != available || *w.choices.borrow() != choices {
            w.pairing.removeAllItems();
            w.pairing.addItemWithTitle(&NSString::from_str("Automatic: exactly one hub"));
            for uuid in &choices {
                let title = if available.contains(uuid) {
                    uuid.to_string()
                } else {
                    format!("{uuid} (unavailable)")
                };
                w.pairing.addItemWithTitle(&NSString::from_str(&title));
            }
            let selected =
                selected.and_then(|u| choices.iter().position(|v| *v == u)).map_or(0, |i| i + 1);
            w.pairing.selectItemAtIndex(selected as isize);
            *w.choices.borrow_mut() = choices;
            *w.available.borrow_mut() = available;
            w.accepted_generation.set(Some(value.generation));
        }
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
        let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 520.0, 540.0));
        let label = |text: &str, y: f64| {
            let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
            field.setFrame(rect(16.0, y, 485.0, 26.0));
            view.addSubview(&field);
            field
        };
        label("Harmonigraph Tune", 502.0);
        let delay_label =
            NSTextField::wrappingLabelWithString(&NSString::from_str("Tuning delay"), mtm);
        delay_label.setFrame(rect(16.0, 466.0, 485.0, 34.0));
        delay_label.setMaximumNumberOfLines(0);
        view.addSubview(&delay_label);
        // Stepped and discontinuous: the tick marks are the only values it can
        // take, and the action arrives once the drag is over.
        let delay = unsafe {
            NSSlider::sliderWithValue_minValue_maxValue_target_action(
                f64::from(self.params.delay.value()),
                1.0,
                f64::from(super::protocol::DELAY_MULTIPLIER_MAX),
                Some(&actions),
                Some(sel!(delay:)),
                mtm,
            )
        };
        delay.setFrame(rect(16.0, 438.0, 485.0, 24.0));
        delay.setNumberOfTickMarks(super::protocol::DELAY_MULTIPLIER_MAX as isize);
        delay.setAllowsTickMarkValuesOnly(true);
        delay.setContinuous(false);
        view.addSubview(&delay);
        let participation = unsafe {
            NSButton::checkboxWithTitle_target_action(
                &NSString::from_str("Participating"),
                Some(&actions),
                Some(sel!(toggle:)),
                mtm,
            )
        };
        participation.setFrame(rect(16.0, 390.0, 180.0, 26.0));
        view.addSubview(&participation);
        let pairing = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(mtm),
            rect(16.0, 354.0, 485.0, 28.0),
            false,
        );
        view.addSubview(&pairing);
        label("Signed sample offset", 326.0);
        label("Sample rate and buffer size follow the host automatically.", 292.0);
        let value = self.shared.value().routing.calibration();
        let field = |text: String, x: f64| {
            let field = NSTextField::textFieldWithString(&NSString::from_str(&text), mtm);
            field.setFrame(rect(x, 326.0, 145.0, 24.0));
            view.addSubview(&field);
            field
        };
        let offset = field(value.offset.to_string(), 220.0);
        label("Offset defaults to zero; adjust only for a known routing delay.", 264.0);
        for (title, action, x, width) in [
            ("Apply / Reinitialize", sel!(apply:), 16.0, 200.0),
            ("Reset voices", sel!(reset:), 236.0, 150.0),
        ] {
            let button = unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(title),
                    Some(&actions),
                    Some(action),
                    mtm,
                )
            };
            button.setFrame(rect(x, 228.0, width, 28.0));
            view.addSubview(&button);
        }
        let status = NSTextField::wrappingLabelWithString(
            &NSString::from_str("Waiting for audio state"),
            mtm,
        );
        status.setFrame(rect(16.0, 25.0, 485.0, 200.0));
        status.setMaximumNumberOfLines(0);
        view.addSubview(&status);
        label(harmonigraph_perf::BUILD_TAG, 5.0);
        actions
            .ivars()
            .widgets
            .set(Widgets {
                view,
                participation,
                pairing,
                offset,
                delay,
                delay_label,
                status,
                choices: RefCell::new(Vec::new()),
                available: RefCell::new(Vec::new()),
                accepted_generation: Cell::new(None),
            })
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
        (520, 540)
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
