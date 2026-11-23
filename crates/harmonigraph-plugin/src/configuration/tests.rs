//! The exported Harmonigraph CLAP factory, real host state and params vtables,
//! and real structured UI bridge. No editor is opened and no probe is enabled.
use super::*;
use crate::test_device::{header, Device, Host, Input};
use clap_sys::events::*;
use clap_sys::ext::gui::{clap_plugin_gui, CLAP_EXT_GUI};
use clap_sys::ext::params::{clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::*;
use harmonigraph_clap::plugin::ParamValue;
use harmonigraph_core::configuration::TuningAxis;
use harmonigraph_core::tuning::microcents;
use harmonigraph_core::SourceId;
use harmonigraph_ui::params::ParamKey;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::Ordering;

mod host_surface;
use super::recording::tests::{stop, transport};

/// A Hub on the configuration suites' host, which answers params, state and
/// gui, activated at 64-frame callbacks.
pub(super) fn hub() -> Device {
    Device::hub(Host::new().params().state().gui()).with_max_frames(64)
}

impl Device {
    /// Called explicitly after a fixture's original assertions. These fixtures
    /// study configuration and recording, so finish their gestures before
    /// destruction: a Hub that still holds a voice marks its take incomplete.
    pub(super) fn finish_notes(&mut self, mut raw: i64, notes: &[(i32, i16)]) {
        let state = |device: &Self| {
            device.wrapper().test_inspect_plugin(|plugin| {
                let hub = plugin.aggregation.as_ref().unwrap();
                (hub.tune.held(), hub.tune.pending())
            })
        };
        self.run(
            raw,
            notes.iter().map(|&(id, key)| note(id, key, 0, CLAP_EVENT_NOTE_OFF)).collect(),
            None,
        );
        for _ in 0..16 {
            raw += 64;
            let mut stopped = transport(0.0, 0);
            stopped.flags &= !CLAP_TRANSPORT_IS_PLAYING;
            self.run_transport(raw, vec![], false, None, Some(stopped));
            if state(self) == (0, 0) {
                return;
            }
        }
        panic!("the delay line must drain after its releases: {:?}", state(self));
    }
    pub(super) fn param(&self, key: ParamKey, plain: f32, time: u32) -> Input {
        let values = crate::HarmonigraphParams::default();
        Input::Param(clap_event_param_value {
            header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, time),
            param_id: self.id(key),
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: values.table.spec(crate::Row::camera(key).index()).to_host(plain),
        })
    }
}
/// A saved state whose musical settings hold `fifth` and no links.
pub(super) fn restored(device: &Device, fifth: f32) -> PluginState {
    let mut state = device.wrapper().get_state_object();
    let mut settings =
        MusicalSettings { meantone: false, marvel: false, learning: false, ..Default::default() };
    settings.axes[1] = microcents(fifth);
    state.fields.insert(MUSICAL_SETTINGS.into(), serde_json::to_string(&settings).unwrap());
    state
}
/// The musical settings a saved state holds.
/// What the editor shows, with the raw axes a save stores.
struct Shown {
    raw: Tuning,
    view: ConfigurationView,
}
fn shown(settings: &Settings) -> Shown {
    Shown { raw: settings.read(ConfigReducer::raw), view: settings.view() }
}
/// The configuration a saved record restores.
fn reduced(settings: MusicalSettings) -> ResolvedConfig {
    let mut reducer = ConfigReducer::default();
    reducer.apply(settings.mutation());
    reducer.resolved()
}
/// The configuration the audio owner copied at its last callback.
fn sounding(device: &Device) -> ResolvedConfig {
    device.wrapper().test_inspect_plugin(|plugin| plugin.configuration.as_ref().unwrap().current())
}
pub(in crate::configuration) fn saved(state: &PluginState) -> MusicalSettings {
    serde_json::from_str(&state.fields[MUSICAL_SETTINGS]).unwrap()
}
pub(super) fn plain(state: &PluginState, key: ParamKey) -> f32 {
    match state.params[key.id()] {
        ParamValue::F32(value) => value,
        _ => panic!("float parameter"),
    }
}

/// An edit and a restore each show and save at once, the newest winning,
/// and audio adopts the document at its next callback. An edit marks the
/// project when it is made; a restore marks nothing.
#[test]
fn edits_and_restores_show_and_save_at_once_and_audio_adopts_them() {
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    let settings = device.settings();
    let fresh = device.service();
    device.load(restored(&device, 690.0));
    assert_eq!(device.service(), fresh, "a restore marks nothing");
    settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(695.0)));
    assert_eq!(shown(&settings).raw.three, microcents(695.0), "the edit shows at once");
    device.load(restored(&device, 705.0));
    let now = shown(&settings);
    assert!(now.view.unadopted.is_some());
    assert_eq!(now.raw.three, microcents(705.0), "the newest restore is what shows");
    assert_eq!(saved(&device.save()).axes[1], microcents(705.0));
    assert_eq!(saved(&device.wrapper().get_state_object()).axes[1], microcents(705.0));
    device.run(1000, vec![], None);
    assert!(shown(&settings).view.unadopted.is_none());
    assert_eq!(sounding(&device).tuning.three, microcents(705.0), "audio adopted it");
    let marked = device.service();
    settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(700.0)));
    assert!(device.service() > marked, "an edit marks the project modified when it is made");
    assert_eq!(saved(&device.save()).axes[1], microcents(700.0), "and saves before audio runs");
    device.run(1064, vec![], None);
    assert_eq!(sounding(&device).tuning.three, microcents(700.0));
}

/// Audio takes no lock: while a writer holds the document at a callback's
/// start, the callback still reads the last configuration a write published.
#[test]
fn a_held_document_does_not_hold_audio() {
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    let settings = device.settings();
    device.run(0, vec![], None);
    settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(695.0)));
    let (held, release) = (std::sync::Barrier::new(2), std::sync::Barrier::new(2));
    let read = std::thread::scope(|scope| {
        scope.spawn(|| {
            settings.document.write(|_| {
                held.wait();
                release.wait();
            })
        });
        held.wait();
        device.run(64, vec![], None);
        let read = sounding(&device).tuning.three;
        release.wait();
        read
    });
    assert_eq!(read, microcents(695.0), "read while the document was held");
    assert!(shown(&settings).view.unadopted.is_none());
}

/// With no audio running, an edit shows and saves at once (Yan, #1938), and
/// the view it returns is drawn as a frame draws it: under Pass through with
/// Decode fifths on, in the board's tuning, so the rest of a drag's frame
/// draws the Decoded fifth it set rather than the saved one.
#[test]
fn an_edit_with_no_audio_shows_and_saves_in_the_tuning_it_is_drawn_in() {
    let _scope = crate::test_scope::enter();
    let device = hub();
    let settings = device.settings();
    let host = device.editor_host();
    let mut policy = settings.view().resolved.policy;
    (policy.decode_fifths, policy.decoded_fifth) = (true, microcents(695.0));
    let view = device.wrapper().test_inspect_plugin(|plugin| {
        let gestures = std::cell::Cell::default();
        let backend = crate::PluginParamBackend {
            params: &plugin.params,
            host: &*host,
            configuration: plugin.params.configuration_view(),
            gestures: &gestures,
        };
        let edit = ConfigEdit { policy: Some(policy), ..Default::default() };
        harmonigraph_ui::params::ParamBackend::submit_tuning(&backend, edit).unwrap()
    });
    assert!(view.board);
    assert_eq!(view.resolved.tuning.three, microcents(695.0), "drawn in the board's tuning");
    assert!(view.unadopted.is_some(), "shown and saved, not yet sounding");
    let saved = saved(&device.save());
    assert_eq!(PolicyConfig::from(saved.adaptive).decoded_fifth, microcents(695.0));
    assert_ne!(saved.axes[TuningAxis::Three.index()], microcents(695.0), "the saved fifth stays");
}

/// A host may save from inside a call the editor makes into it
/// (`editor/lock.rs`). No document door is held across one, so the save
/// returns, for a map edit's parameter set and a camera set made beside a
/// tuning edit.
#[test]
fn a_save_from_inside_a_host_call_returns() {
    struct Saving(Arc<crate::HarmonigraphParams>, std::sync::atomic::AtomicUsize);
    impl Saving {
        fn save(&self) {
            let fields = Default::default();
            let mut state =
                PluginState { version: String::new(), params: Default::default(), fields };
            self.0.settings.save(&mut state);
            self.1.fetch_add(1, Ordering::Relaxed);
        }
    }
    impl harmonigraph_clap::prelude::EditorHost for Saving {
        fn closed(&self) -> bool {
            false
        }
        fn request_resize(&self, _: (u32, u32)) -> bool {
            false
        }
        fn begin_set_parameter(&self, _: usize) {
            self.save();
        }
        fn set_parameter(&self, _: usize, _: f32) {
            self.save();
        }
        fn end_set_parameter(&self, _: usize) {
            self.save();
        }
    }
    let params = Arc::new(crate::HarmonigraphParams::default());
    let host = Saving(params.clone(), Default::default());
    crate::lattice_maps::edit(&params, &host, harmonigraph_ui::lattice_maps::MapEdit::Select(0));
    let gestures = std::cell::Cell::default();
    let backend = crate::PluginParamBackend {
        params: &params,
        host: &host,
        configuration: params.configuration_view(),
        gestures: &gestures,
    };
    use harmonigraph_ui::params::ParamBackend;
    backend.submit_tuning(ConfigEdit::axis(TuningAxis::Three, microcents(696.0)));
    backend.set(ParamKey::CameraDistance, 6.0);
    assert_eq!(host.1.load(Ordering::Relaxed), 6, "every host call saved, and returned");
}

/// The host may destroy the plugin while the input worker is inside a host
/// wake, here Learn's, holding the wrapper's other reference: destruction
/// stops and joins the worker first, so the plugin drops on the host's
/// thread rather than on the worker, which would join itself (#1938).
#[test]
fn destruction_waits_out_a_worker_wake_and_drops_on_the_hosts_thread() {
    let _scope = crate::test_scope::enter();
    let mut device = hub();
    device.activate();
    let settings = device.settings();
    settings.submit(ConfigEdit::axis(TuningAxis::Three, 690_000_000));
    settings.submit(ConfigEdit { learning: Some(true), ..Default::default() });
    let (hold, holding) = (device.host().hold.clone(), device.host().holding.clone());
    *device.host().hold_thread.lock().unwrap() = Some("harmonigraph-input-worker");
    hold.store(true, Ordering::Release);
    device.run(
        0,
        vec![note(60, 60, 0, CLAP_EVENT_NOTE_ON), note(67, 67, 0, CLAP_EVENT_NOTE_ON)],
        None,
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut raw = 64;
    while !holding.load(Ordering::Acquire) {
        assert!(std::time::Instant::now() < deadline, "Learn never woke the host");
        device.run(raw, vec![], None);
        raw += 64;
        std::thread::sleep(crate::background::POLL);
    }
    let dropped_on = device.wrapper().test_inspect_plugin(|plugin| plugin.dropped_on.clone());
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        hold.store(false, Ordering::Release);
    });
    drop(device);
    let dropped = dropped_on.lock().unwrap().clone();
    release.join().unwrap();
    let host = Some(std::thread::current().name().map(str::to_owned));
    assert_eq!(dropped, host, "the plugin dropped by the end of destroy, on its thread");
}

/// Learn's gate, inference and result are one write: it infers armed only
/// where it acts under the engine read inside that write, so a restore that
/// switched Pass through to Lattice Map since the last step leaves the axes
/// alone.
#[test]
fn learn_applies_only_while_the_document_lets_it_act() {
    let settings = Settings::default();
    let learned =
        harmonigraph_core::tuning::LearnedTuning { three: Some(696.0), ..Default::default() };
    let learn = |engine: TuningEngine| {
        let mut asked = None;
        let changed = settings.learn(
            || engine,
            |armed| {
                asked = Some(armed);
                armed.then_some(learned)
            },
        );
        (asked, changed)
    };
    assert_eq!(learn(TuningEngine::Off), (Some(false), false), "disarmed");
    settings.submit(ConfigEdit { learning: Some(true), ..Default::default() });
    assert_eq!(learn(TuningEngine::LatticeMap), (Some(false), false), "suspended in Lattice Map");
    assert_eq!(learn(TuningEngine::Adaptive), (Some(false), false), "and under Adaptive");
    assert_ne!(shown(&settings).raw.three, microcents(696.0), "the axes are left alone");
    assert_eq!(learn(TuningEngine::Off), (Some(true), true), "it acts in Pass through");
    assert_eq!(shown(&settings).raw.three, microcents(696.0));
}

/// A processing stop ends what was held, though no callback follows to say
/// so (#1948): the chord leaves the lattice and the board's keys at once, and
/// Learn, armed while stopped, learns nothing from it. A chord struck after a
/// quick restart stays held and is learned. The whole path: `Plugin::stop`
/// publishes the end, and the input worker drains it in order.
#[test]
fn a_processing_stop_ends_what_was_held_and_a_restart_plays_on() {
    let _scope = crate::test_scope::enter();
    let mut device = hub();
    device.activate();
    let settings = device.settings();
    settings.submit(ConfigEdit::axis(TuningAxis::Three, 690_000_000));
    device.run(
        0,
        vec![note(60, 60, 0, CLAP_EVENT_NOTE_ON), note(67, 67, 0, CLAP_EVENT_NOTE_ON)],
        None,
    );
    let lock = device.editor_lock();
    let held = || lock.lock().ui.picture.runtime.tracker.held_count();
    let lit = || crate::board::sounding(&lock.lock().ui.picture.runtime.tracker).len();
    let rounds = |device: &Device| {
        device.wrapper().test_inspect_plugin(|plugin| plugin.input_worker.completed_rounds())
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut raw = 64;
    while held() < 2 {
        assert!(std::time::Instant::now() < deadline, "the chord never reached the editor");
        device.run(raw, vec![], None);
        raw += 64;
        std::thread::sleep(crate::background::POLL);
    }
    assert_eq!(lit(), 2, "the board lights the chord");
    device.deactivate();
    settings.submit(ConfigEdit { learning: Some(true), ..Default::default() });
    while held() > 0 {
        assert!(std::time::Instant::now() < deadline, "the stop never ended the chord");
        std::thread::sleep(crate::background::POLL);
    }
    assert_eq!(lit(), 0, "the board's keys go out");
    let from = rounds(&device);
    while rounds(&device) < from + 4 {
        assert!(std::time::Instant::now() < deadline, "the worker stopped stepping");
        std::thread::sleep(crate::background::POLL);
    }
    assert_eq!(shown(&settings).raw.three, 690_000_000, "Learn did not hear it");
    device.activate();
    let mut raw = 0;
    device.run(
        raw,
        vec![note(62, 62, 0, CLAP_EVENT_NOTE_ON), note(69, 69, 0, CLAP_EVENT_NOTE_ON)],
        None,
    );
    raw += 64;
    until(&mut device, &mut raw, |now| now.raw.three == 700_000_000);
    assert_eq!(held(), 2, "the new chord stays held");
}

/// Activation reads the document again, so an edit made while the plugin
/// was inactive is adopted before the first callback, and the pending label
/// clears.
#[test]
fn activation_adopts_an_edit_made_while_inactive() {
    let _scope = crate::test_scope::enter();
    let mut device = hub();
    let settings = device.settings();
    settings.submit(ConfigEdit::axis(TuningAxis::Three, 695_000_000));
    assert!(shown(&settings).view.unadopted.is_some(), "not yet sounding");
    device.activate();
    assert!(shown(&settings).view.unadopted.is_none(), "adopted at activation");
    assert_eq!(sounding(&device).tuning.three, 695_000_000);
}

/// Run callbacks until the input worker's Learn leaves the shown
/// configuration as `learned` says, from callback time `raw` on.
fn until(device: &mut Device, raw: &mut i64, learned: impl Fn(&Shown) -> bool) {
    let settings = device.settings();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !learned(&shown(&settings)) {
        assert!(
            std::time::Instant::now() < deadline,
            "Learn never got there: {:?}",
            shown(&settings).raw
        );
        device.run(*raw, vec![], None);
        *raw += 64;
        std::thread::sleep(crate::background::POLL);
    }
}

/// Learn runs in the input worker (#1938 C), with no editor open: armed under
/// Pass through, it hears the Hub's notes with the per-note tuning they were
/// played with, as an MPE board plays them, and the document takes what it
/// found, marked and saved.
#[test]
fn learn_hears_the_played_tuning_with_no_editor_open() {
    let _scope = crate::test_scope::enter();
    let mut device = hub();
    device.activate();
    let settings = device.settings();
    settings.submit(ConfigEdit { learning: Some(true), ..Default::default() });
    let marked = device.service();
    let mut events = Vec::new();
    for key in [60, 64, 67] {
        events.push(note(i32::from(key), key, 0, CLAP_EVENT_NOTE_ON));
        // A just triad, played 10¢ sharp throughout, so the C offset Learn
        // finds is not 0 and the third is not 12-TET's.
        let bend = 10.0
            + match key {
                64 => harmonigraph_core::tuning::FIVE_JUST - 400.0,
                67 => harmonigraph_core::tuning::THREE_JUST - 700.0,
                _ => 0.0,
            };
        events.push(id_tuning(i32::from(key), f64::from(bend) / 100.0));
    }
    device.run(0, events, None);
    let mut raw = 64;
    until(&mut device, &mut raw, |now| (now.raw.c_offset - 10_000_000).abs() < 1_000);
    let now = shown(&settings);
    let third = microcents(harmonigraph_core::tuning::FIVE_JUST);
    assert!((now.raw.five - third).abs() < 1_000, "the played third: {}", now.raw.five);
    assert!(!now.view.resolved.modes.tempered.syntonic);
    let keyboard = now.view.resolved.policy.keyboard;
    assert!((keyboard[0] - microcents(harmonigraph_core::tuning::THREE_JUST)).abs() < 1_000);
    // The worker marks the project once its editor lock is let go.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while device.service() <= marked {
        assert!(std::time::Instant::now() < deadline, "Learn never marked the project");
        std::thread::sleep(crate::background::POLL);
    }
    assert_eq!(saved(&device.save()).axes, now.raw.axes(), "a save stores what Learn found");
    device.finish_notes(raw, &[(60, 60), (64, 64), (67, 67)]);
}

/// The tuning pending label waits on the document's latest revision, not on
/// the configuration audio last copied: with audio stopped, a second edit
/// restarts the wait the first began (#1814 Q2).
#[test]
fn the_tuning_label_waits_on_the_latest_revision() {
    use harmonigraph_ui::{PendingLabel, PENDING_AFTER};
    let settings = Settings::default();
    let start = std::time::Instant::now();
    let at = |ms: u64| start + std::time::Duration::from_millis(ms);
    let mut label = PendingLabel::default();
    settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(700.0)));
    assert!(!label.shown(settings.view().unadopted, at(0)));
    let later = PENDING_AFTER.as_millis() as u64 / 2;
    settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(701.0)));
    assert!(!label.shown(settings.view().unadopted, at(later)));
    let first_wait = PENDING_AFTER.as_millis() as u64;
    assert!(!label.shown(settings.view().unadopted, at(first_wait)), "the wait restarted");
    assert!(label.shown(settings.view().unadopted, at(later + first_wait)));
}

#[test]
fn interval_links_keep_their_value_on_release_and_recognize_new_entries() {
    let _scope = crate::test_scope::enter();
    for fifth in [696.58, *TuningAxis::Three.range().start(), *TuningAxis::Three.range().end()] {
        let mut device = hub().with_retune();
        device.activate();
        let settings = device.settings();
        settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(fifth)));
        device.run(0, vec![], None);
        let linked = shown(&settings).view.resolved;
        assert!(linked.modes.tempered.syntonic && linked.modes.tempered.septimal_kleisma);
        assert!((linked.tuning.five_cents() - (4.0 * fifth - 2400.0)).abs() < 0.001);
        for comma in harmonigraph_core::Comma::ALL.into_iter().rev() {
            settings.submit(ConfigEdit::temper(comma, false, linked.tuning));
        }
        assert!(
            shown(&settings).view.unadopted.is_some(),
            "the switches are one pending audio adoption"
        );
        device.run(64, vec![], None);
        let released = shown(&settings);
        assert_eq!(released.view.resolved.modes.tempered, Tempered::default());
        assert_eq!(released.raw.five, linked.tuning.five, "a released link keeps its value");
        assert_eq!(released.raw.seven, linked.tuning.seven);
        assert_eq!(saved(&device.save()).axes[2..4], [released.raw.five, released.raw.seven]);
        settings.submit(ConfigEdit::axis(TuningAxis::Five, released.raw.five));
        device.run(128, vec![], None);
        assert!(
            shown(&settings).view.resolved.modes.tempered.syntonic,
            "a typed same-value entry recognizes meantone"
        );
        settings.submit(ConfigEdit::axis(TuningAxis::Five, 390_000_000));
        device.run(192, vec![], None);
        let edited = shown(&settings).view.resolved;
        assert!(!edited.modes.tempered.syntonic);
        assert_eq!(edited.tuning.five_cents(), 390.0);
        assert!(edited.modes.tempered.septimal_kleisma);
    }
}

#[test]
fn destroying_pending_or_faulted_configuration_owners_does_not_panic() {
    let _scope = crate::test_scope::enter();
    for commands in [false, true] {
        let mut device = hub().with_retune();
        if commands {
            let settings = device.settings();
            device.load(restored(&device, 690.0));
            device.activate();
            device.run(0, vec![], None);
            device.load(restored(&device, 705.0));
            assert!(shown(&settings).view.unadopted.is_some(), "a restore audio has not adopted");
        } else {
            device.activate();
            device.run(0, vec![device.param(ParamKey::CameraDistance, 6.0, 0)], None);
            // Timed past the block's end: the list is invalid, its input lost.
            let lost = vec![device.param(ParamKey::CameraDistance, 7.0, 64)];
            let (status, _) = device.run_transport(64, lost, false, None, None);
            assert_eq!(status, CLAP_PROCESS_ERROR);
            assert_eq!(shown(&device.settings()).view.status & 2, 2);
        }
        drop(device);
    }
}

#[test]
fn construction_outside_the_clap_wrapper_has_no_configuration_owner() {
    let plugin = crate::Harmonigraph::default();
    assert!(
        plugin.configuration.is_none(),
        "construction outside the CLAP wrapper has no CLAP-only owner or callback work"
    );
}

/// The restored modes are the saved ones, read back as themselves: two
/// patterns that differ in every bit and neither of which is the default, so a
/// readback, a save or an adoption that dropped, swapped or defaulted one
/// cannot agree with the others by accident.
#[test]
fn a_restore_reads_back_and_saves_its_modes_before_and_after_adoption() {
    let _scope = crate::test_scope::enter();
    for (meantone, marvel, learning) in [(true, false, true), (false, true, false)] {
        let mut device = hub().with_retune();
        device.activate();
        let settings = device.settings();
        let mut state = restored(&device, 700.0);
        let mut restoring = saved(&state);
        (restoring.meantone, restoring.marvel, restoring.learning) = (meantone, marvel, learning);
        state.fields.insert(MUSICAL_SETTINGS.into(), serde_json::to_string(&restoring).unwrap());
        device.load(state);
        let readback = shown(&settings).view;
        assert!(readback.unadopted.is_some());
        let readback = readback.resolved;
        let restored_modes = TuningModes {
            tempered: Tempered { syntonic: meantone, septimal_kleisma: marvel },
            learning,
        };
        assert_eq!(readback.modes, restored_modes);
        assert_eq!(readback.tuning.three, 700_000_000, "the readback's tuning is the restored one");
        let saved_host = saved(&device.save());
        assert_eq!(
            (saved_host.meantone, saved_host.marvel, saved_host.learning),
            (meantone, marvel, learning)
        );
        assert_eq!(reduced(saved_host).modes, readback.modes);
        assert_eq!(reduced(saved(&device.wrapper().get_state_object())).modes, readback.modes);
        device.run(0, vec![], None);
        assert_eq!(shown(&settings).view.resolved.modes, readback.modes);
        // And the configuration audio copied holds them too.
        assert_eq!(sounding(&device).modes, restored_modes);
    }
}

/// A saved field that does not parse costs that field alone, with a Console
/// line: the rest of the project restores, and the instance keeps what it
/// held for the field.
#[test]
fn a_field_that_does_not_parse_is_skipped_with_a_console_line() {
    let _scope = crate::test_scope::enter();
    let device = hub().with_retune();
    let maps = |device: &Device| {
        device.wrapper().test_inspect_plugin(|plugin| {
            serde_json::to_string(&*plugin.params.maps.read()).unwrap()
        })
    };
    let before = maps(&device);
    let mut state = device.wrapper().get_state_object();
    state.fields.insert("lattice-maps".into(), "{not json".into());
    state.fields.insert("editor-state".into(), r#"{"size":[901,702]}"#.into());
    state.params.insert("map-fifths".into(), ParamValue::I32(3));
    device.load(state);
    assert_eq!(maps(&device), before, "the maps the instance held");
    let (size, fifths) = device.wrapper().test_inspect_plugin(|plugin| {
        (plugin.params.editor_state.size(), plugin.params.int(crate::Row::MapFifths))
    });
    assert_eq!((size, fifths), ((901, 702), 3), "the rest of the project");
    let Some(Notice::Restore(line)) = device.settings().next_notice() else {
        panic!("no restore line in the Console");
    };
    assert!(line.starts_with("This project's saved lattice-maps could not be read"), "{line}");
}

/// Analyzer Audio Input saves as its index, as the other choices do, and a
/// project saved with Sidechain reopens at Sidechain.
#[test]
fn the_analysis_input_saves_as_its_index_and_reopens() {
    let _scope = crate::test_scope::enter();
    let device = hub();
    device
        .wrapper()
        .test_inspect_plugin(|plugin| plugin.params.set(crate::Row::AnalysisInput, 1.0));
    let saved = device.save();
    assert!(matches!(saved.params["analysis-input"], ParamValue::I32(1)));
    let reopened = hub();
    reopened.load(saved);
    let input = reopened.wrapper().test_inspect_plugin(|plugin| plugin.params.analysis_input());
    assert_eq!(input, harmonigraph_ui::params::AnalysisInput::Sidechain);
}

/// A restore line that reports a loss opens the Console in the open editor,
/// as the editor door's own losses do (Yan, 2026-11-22, in
/// `docs/intent/editor-shell.md`), and opening it is no edit the host is told
/// of. A line about the host only appends, and both reach the scrollback.
#[test]
fn a_restore_that_reports_a_loss_opens_the_console() {
    use harmonigraph_ui::testing;
    let _scope = crate::test_scope::enter();
    let device = hub();
    let (settings, lock) = (device.settings(), device.editor_lock());
    // Saved with Settings folded on a tab that is not the Console.
    testing::fold_settings_on_tuning(&mut lock.lock().ui);
    let mut state = device.wrapper().get_state_object();
    // One id no row has, so the restore owes one line.
    state.params.insert("a-retired-parameter".into(), ParamValue::F32(1.0));
    device.load(state);
    settings.notify(Notice::Host("a line about the host".into()));
    let before = device.service();
    // As an editor frame takes them: the lock installs the restore, then
    // its Console lines arrive over it.
    let mut shared = lock.lock();
    assert!(testing::settings_folded(&shared.ui), "the fixture opens folded");
    settings.deliver_notices(&mut shared.ui);
    testing::end_frame(&mut shared.ui);
    assert!(testing::console_revealed(&shared.ui), "the restore named a retired parameter");
    let lines: Vec<_> = shared.ui.picture.runtime.console.lines().map(str::to_owned).collect();
    drop(shared);
    assert_eq!(
        lines,
        [
            "This project saved parameters this build no longer has, which were not read: \
             a-retired-parameter.",
            "a line about the host",
        ],
    );
    assert_eq!(device.service(), before, "the reveal is no edit");
}

/// A newer restore's look wins over the Console an older one's line owed: the
/// line stays in the scrollback, and the layout is the newer restore's.
#[test]
fn a_newer_restore_keeps_an_older_ones_line_but_not_its_reveal() {
    use harmonigraph_ui::testing;
    let _scope = crate::test_scope::enter();
    let device = hub();
    let (settings, lock) = (device.settings(), device.editor_lock());
    testing::fold_settings_on_tuning(&mut lock.lock().ui);
    let clean = device.wrapper().get_state_object();
    let mut lossy = clean.clone();
    lossy.params.insert("a-retired-parameter".into(), ParamValue::F32(1.0));
    device.load(lossy);
    device.load(clean);
    let mut shared = lock.lock();
    settings.deliver_notices(&mut shared.ui);
    assert!(testing::settings_folded(&shared.ui), "the newer restore's layout");
    assert_eq!(shared.ui.picture.runtime.console.lines().count(), 1, "the older restore's line");
}

/// A project saved while the axes were host parameters (#1756) holds them
/// there, beside musical settings without axes. It opens at fresh axes with
/// one Console notice, keeps its links, and saves no tuning parameter back.
#[test]
fn a_project_saved_with_tuning_parameters_opens_at_fresh_axes() {
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    let settings = device.settings();
    let mut state = device.wrapper().get_state_object();
    device.load(state.clone());
    assert_eq!(settings.next_notice(), None, "a project saved since opens quietly");
    state.params.insert("tuning-three".into(), ParamValue::F32(696.0));
    state.params.insert("tuning-tolerance".into(), ParamValue::F32(3.0));
    state.fields.insert(MUSICAL_SETTINGS.into(), r#"{"meantone":false,"marvel":true}"#.into());
    device.load(state);
    assert_eq!(
        settings.next_notice(),
        Some(Notice::Restore(
            "This project saved parameters this build no longer has, which were not read: \
             tuning-three, tuning-tolerance."
                .into()
        ))
    );
    assert_eq!(settings.next_notice(), Some(Notice::Restore(LEGACY_AXES.into())));
    assert_eq!(settings.next_notice(), None, "the notice is given once");
    device.activate();
    device.run(0, vec![], None);
    let opened = shown(&device.settings());
    assert_eq!(opened.raw.axes(), MusicalSettings::default().axes);
    let modes = opened.view.resolved.modes.tempered;
    assert_eq!((modes.syntonic, modes.septimal_kleisma), (false, true));
    let resaved = device.save();
    assert!(LEGACY_AXIS_IDS.iter().all(|id| !resaved.params.contains_key(*id)));
    assert_eq!(saved(&resaved).axes, MusicalSettings::default().axes);
}

/// A host that saves right after restoring, before it has run the plugin
/// once, stores the restored record rather than the fresh one.
#[test]
fn a_save_before_any_audio_stores_the_restore() {
    let _scope = crate::test_scope::enter();
    let device = hub().with_retune();
    device.load(restored(&device, 690.0));
    let saved = saved(&device.save());
    assert_eq!(saved.axes[TuningAxis::Three.index()], microcents(690.0));
    assert!(!saved.meantone && !saved.marvel, "the restore's links, not the fresh ones");
}

/// An axis edit changes no host parameter, so the host marks the project
/// modified without rescanning; a map-document edit rescans the parameter
/// text, since the Lattice Map's label names its map.
#[test]
fn an_axis_edit_marks_the_project_without_a_rescan() {
    use clap_sys::ext::params::{CLAP_PARAM_RESCAN_TEXT, CLAP_PARAM_RESCAN_VALUES};
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    let settings = device.settings();
    device.run(0, vec![], None);
    let marked = device.service();
    let rescans = device.host().rescans.load(Ordering::Relaxed);
    settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(696.0)));
    device.run(64, vec![], None);
    assert!(device.service() > marked, "the edit marks the project modified");
    assert_eq!(device.host().rescans.load(Ordering::Relaxed), rescans, "and rescans nothing");
    settings.maps_changed();
    device.service();
    assert_eq!(device.host().rescans.load(Ordering::Relaxed), rescans + 1);
    let flags = device.host().rescan_flags.load(Ordering::Relaxed);
    assert_eq!(flags & CLAP_PARAM_RESCAN_TEXT, CLAP_PARAM_RESCAN_TEXT);
    assert_eq!(flags & CLAP_PARAM_RESCAN_VALUES, 0, "the values did not change");
}

/// Opening a project, and Reset all voices, change nothing the project saves,
/// so neither has the host mark it modified: the musical settings and the
/// instance's own Retune, Show and name alike.
#[test]
fn a_restore_and_a_reset_leave_the_project_unmodified() {
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    device.run(1000, vec![], None);
    let before = device.service();
    device.load(restored(&device, 700.0));
    device.run(1064, vec![], None);
    assert_eq!(device.service(), before, "a restore is not an edit");
    crate::tuning::instances::edit(0, harmonigraph_ui::params::InstanceEdit::Reset);
    device.run(1128, vec![], None);
    assert_eq!(device.service(), before, "Reset all voices saves nothing");
}

/// An edit to what `ui-state` saves has the host mark the project modified
/// once, as a musical setting's does; restoring it marks nothing (#1813).
/// Ended by a close, which reports a gesture it cut off.
#[test]
fn a_ui_state_edit_marks_the_project_once_and_its_restore_does_not() {
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    device.run(0, vec![], None);
    let lock = device.wrapper().test_inspect_plugin(|plugin| plugin.editor_shared.clone());
    let before = device.service();
    let mut project = harmonigraph_ui::SharedState::new(crate::editor::ASSUMED_SURFACE_FORMAT);
    let restored = project.picture.appearance.view.max_sevens + 1;
    project.picture.appearance.view.max_sevens = restored;
    let mut state = device.wrapper().get_state_object();
    let blob = harmonigraph_ui::shell::close(&project);
    state.fields.insert("ui-state".into(), serde_json::to_string(&blob).unwrap());
    device.load(state);
    let installed = lock.lock().ui.picture.appearance.view.max_sevens;
    assert_eq!(installed, restored, "the restore installed");
    let _ = lock.end_window_session();
    assert_eq!(device.service(), before, "a restore is not an edit");

    lock.lock().ui.picture.appearance.view.max_sevens = restored + 1;
    let _ = lock.end_window_session();
    assert_eq!(device.service(), before + 1, "the edit marks the project modified");
    let _ = lock.end_window_session();
    assert_eq!(device.service(), before + 1, "once");
}

/// A resize or an Own window switch, what `editor-state` saves, has the host
/// mark the project modified, once per change; a host's resize while the
/// editor opens marks nothing, nor does a restore, whose resize lands at the
/// next frame (#1824). `frame` is what each editor frame does, the window's
/// size source and then the frame's end, and the editor opens docked, where a
/// host's `set_size` is the editor's size.
/// A notice only sets a flag the host services, so the device is serviced
/// after every frame: notices from two frames would otherwise read as one.
#[test]
fn a_window_edit_marks_the_project_and_an_opening_resize_or_a_restore_does_not() {
    let _scope = crate::test_scope::enter();
    let device = hub().with_retune();
    device.host().grants_resize.store(true, Ordering::Relaxed);
    let gui = unsafe {
        &*(((*device.plugin).get_extension.unwrap())(device.plugin, CLAP_EXT_GUI.as_ptr())
            .cast::<clap_plugin_gui>())
    };
    let (mut api, mut floating) = (ptr::null(), false);
    unsafe {
        assert!((gui.get_preferred_api.unwrap())(device.plugin, &mut api, &mut floating));
        assert!((gui.create.unwrap())(device.plugin, api, false));
    }
    let state = device.wrapper().test_inspect_plugin(|plugin| plugin.params.editor_state.clone());
    let settings = device.settings();
    let (host, lock) = (device.editor_host(), device.editor_lock());
    let opened = std::cell::Cell::new(None);
    let frame = |own: Option<bool>| {
        if let Some(opened) = opened.get() {
            crate::editor::next_size(&state, opened, host.as_ref(), &lock);
        }
        state.end_frame(own, None, false, || settings.changed());
    };
    let resize = |grow: u32| {
        let (width, height) = state.size();
        let size = (width + grow, height + grow);
        assert!(unsafe { (gui.set_size.unwrap())(device.plugin, size.0, size.1) });
        assert_eq!(state.size(), size, "the host sized the editor");
    };
    let before = device.service();
    state.set_docked(true);
    opened.set(Some(state.open()));
    state.settle();
    resize(40);
    frame(None);
    assert_eq!(device.service(), before, "a resize while the editor opens");
    resize(30);
    frame(None);
    assert_eq!(device.service(), before + 1, "a resize");
    frame(None);
    assert_eq!(device.service(), before + 1, "once");
    frame(Some(true));
    assert_eq!(device.service(), before + 2, "an Own window switch");
    frame(None);
    assert_eq!(device.service(), before + 2, "once");

    let saved = crate::editor::SavedWindow { size: (900, 700), docked: true };
    let mut project = device.wrapper().get_state_object();
    project.fields.insert("editor-state".into(), serde_json::to_string(&saved).unwrap());
    device.load(project);
    frame(None);
    assert_eq!(state.size(), (900, 700), "the restore installed");
    assert_eq!(device.service(), before + 2, "a restore");

    // Closed, then reopened: a frame the own window draws as the view moves
    // into it comes before the opening is settled.
    state.close();
    opened.set(Some(state.open()));
    resize(20);
    frame(None);
    assert_eq!(device.service(), before + 2, "a resize while the view moves in");
    state.settle();
    resize(20);
    frame(None);
    assert_eq!(device.service(), before + 2, "a resize while the editor opens again");
}

/// Bitwig's border drag stops at the editor's limits, the answer to
/// `gui.adjust_size`, and a host size outside them is refused, the editor
/// keeping its own (Yan, #1814). Docked and open, where a host size inside
/// them is the editor's.
#[test]
fn a_border_drag_stops_at_the_editors_limits_and_a_size_outside_them_is_refused() {
    let _scope = crate::test_scope::enter();
    let device = hub().with_retune();
    let gui = unsafe {
        &*(((*device.plugin).get_extension.unwrap())(device.plugin, CLAP_EXT_GUI.as_ptr())
            .cast::<clap_plugin_gui>())
    };
    let (mut api, mut floating) = (ptr::null(), false);
    unsafe {
        assert!((gui.get_preferred_api.unwrap())(device.plugin, &mut api, &mut floating));
        assert!((gui.create.unwrap())(device.plugin, api, false));
    }
    let state = device.wrapper().test_inspect_plugin(|plugin| plugin.params.editor_state.clone());
    state.set_docked(true);
    state.set_open(true);
    let adjust = |(mut width, mut height): (u32, u32)| {
        assert!(unsafe { (gui.adjust_size.unwrap())(device.plugin, &mut width, &mut height) });
        (width, height)
    };
    let set_size = |(width, height): (u32, u32)| unsafe {
        (gui.set_size.unwrap())(device.plugin, width, height)
    };
    let limits = crate::editor::LIMITS;
    assert_eq!(adjust((1, 1)), limits.min, "a drag below the minimum");
    assert_eq!(adjust((u32::MAX, u32::MAX)), limits.max, "a drag past the maximum");
    let inside = (limits.min.0 + 1, limits.max.1 - 1);
    assert_eq!(adjust(inside), inside, "a drag inside them");

    let kept = state.size();
    for outside in [(limits.min.0 - 1, limits.min.1), (limits.max.0, limits.max.1 + 1)] {
        assert!(!set_size(outside), "{outside:?} was taken");
        assert_eq!(state.size(), kept, "a refused size replaced the editor's");
    }
    assert!(set_size(limits.min), "the minimum itself");
    assert_eq!(state.size(), limits.min);
}

/// The window a Console reveal asks for, after a restore that reported a loss
/// unfolded a folded Settings section, is the editor's own write: applying it
/// marks the project modified no more than the reveal does (#1813). The same
/// ask made as an edit, a fold click's, marks it once.
#[test]
fn a_reveals_window_marks_nothing_where_a_folds_marks_once() {
    let _scope = crate::test_scope::enter();
    let device = hub().with_retune();
    device.host().grants_resize.store(true, Ordering::Relaxed);
    let state = device.wrapper().test_inspect_plugin(|plugin| plugin.params.editor_state.clone());
    let settings = device.settings();
    let (host, lock) = (device.editor_host(), device.editor_lock());
    state.set_docked(true);
    let opened = state.open();
    let frame = |ask: Option<(u32, u32)>, own: bool| {
        crate::editor::next_size(&state, opened, host.as_ref(), &lock);
        state.end_frame(None, ask, own, || settings.changed());
    };
    frame(None, false);
    for own in [true, false] {
        let marks = device.service();
        let (width, height) = state.size();
        frame(Some((width + 300, height)), own);
        frame(None, false);
        frame(None, false);
        assert_eq!(state.size(), (width + 300, height), "the window took the ask");
        assert_eq!(device.service(), marks + usize::from(!own), "own: {own}");
    }
}

/// A restore while the editor is open (a preset, Bitwig's undo) resizes its
/// window to the restored size at the next frame, through the host, and
/// marks nothing (Yan, #1814); until then the editor keeps the size its
/// window has. `frame` is the window's size source and the frame's end.
#[test]
fn a_restore_while_the_editor_is_open_resizes_its_window_at_the_next_frame() {
    let _scope = crate::test_scope::enter();
    let device = hub().with_retune();
    device.host().grants_resize.store(true, Ordering::Relaxed);
    let state = device.wrapper().test_inspect_plugin(|plugin| plugin.params.editor_state.clone());
    let settings = device.settings();
    let (host, lock) = (device.editor_host(), device.editor_lock());
    state.set_docked(true);
    let opened = state.open();
    let frame = || {
        let laid_out = crate::editor::next_size(&state, opened, host.as_ref(), &lock);
        state.end_frame(None, None, false, || settings.changed());
        laid_out
    };
    frame();
    let kept = state.size();
    let restored = (kept.0 + 120, kept.1 + 80);
    let saved = crate::editor::SavedWindow { size: restored, docked: true };
    let mut project = device.wrapper().get_state_object();
    project.fields.insert("editor-state".into(), serde_json::to_string(&saved).unwrap());
    let marks = device.service();

    device.load(project);
    assert_eq!(state.size(), kept, "resized before the window's next frame");
    assert!(device.host().resizes.lock().unwrap().is_empty(), "the host asked before the frame");
    assert_eq!(frame(), Some(restored), "the frame was not laid out at the restored size");
    assert_eq!(*device.host().resizes.lock().unwrap(), [restored], "the host asked");
    assert_eq!(state.size(), restored, "the window took the restored size");
    assert_eq!(device.service(), marks, "a restore marked the project");
}

pub(super) fn note(id: i32, key: i16, time: u32, kind: u16) -> Input {
    Input::Note(clap_event_note {
        header: header::<clap_event_note>(kind, time),
        note_id: id,
        port_index: 0,
        channel: 0,
        key,
        velocity: 0.8,
    })
}
fn id_tuning(id: i32, value: f64) -> Input {
    Input::Expression(clap_event_note_expression {
        header: header::<clap_event_note_expression>(CLAP_EVENT_NOTE_EXPRESSION, 0),
        expression_id: CLAP_NOTE_EXPRESSION_TUNING,
        note_id: id,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    })
}
#[test]
fn note_id_only_expression_and_release_match_only_the_addressed_held_note() {
    let _scope = crate::test_scope::enter();
    let mut device = hub();
    device.activate();
    let settings = device.settings();
    // Seeded away from the 12-TET fifth, so the wait below is Learn's.
    settings.submit(ConfigEdit::axis(TuningAxis::Three, 690_000_000));
    settings.submit(ConfigEdit { learning: Some(true), ..Default::default() });
    device.run(
        0,
        vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON), note(20, 67, 0, CLAP_EVENT_NOTE_ON)],
        None,
    );
    // C and G are learned as a fifth; then C moves, and what Learn does with
    // the interval left is read at the end.
    let mut raw = 64;
    until(&mut device, &mut raw, |now| now.raw.three == 700_000_000);
    device.run(raw, vec![id_tuning(10, 1.0)], None);
    settings.submit(ConfigEdit::axis(TuningAxis::Three, 690_000_000));
    let mut release = match note(10, -1, 0, CLAP_EVENT_NOTE_OFF) {
        Input::Note(event) => event,
        _ => unreachable!(),
    };
    release.channel = -1;
    release.port_index = -1;
    device.run(raw + 64, vec![Input::Note(release)], None);
    settings.submit(ConfigEdit::axis(TuningAxis::Five, 390_000_000));
    device.run(raw + 128, vec![id_tuning(20, 5.0), note(30, 64, 0, CLAP_EVENT_NOTE_ON)], None);
    raw += 192;
    // ID20 survived the ID10 release and now supplies C beside E.
    until(&mut device, &mut raw, |now| now.raw.five == 400_000_000);
    assert_eq!(
        shown(&settings).raw.three,
        690_000_000,
        "only C moved: the remaining interval supplied no fifth evidence"
    );
    device.finish_notes(raw, &[(20, 67), (30, 64)]);
}

#[test]
fn canonical_publication_slots_and_loss_are_allocation_free() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_core::canonical::*;
    use harmonigraph_record::publication::*;
    let (mut publisher, mut consumer) = channel();
    let voices: [_; 64] = std::array::from_fn(|note| VoiceBaseline {
        note: note as u8,
        lifetime: note as u64 + 1,
        pitch_microcents: note as i64 * 100_000_000,
        velocity: 0.8,
        ..Default::default()
    });
    let baseline = SourceBaseline::new(SourceId::DIRECT, 1.0, 0, true, &voices).unwrap();
    let start = std::time::Instant::now();
    nice_assert_no_alloc::assert_no_alloc(|| {
        for _ in 0..SNAPSHOT_SLOTS {
            publisher.baseline(&baseline).unwrap();
        }
        assert_eq!(publisher.baseline(&baseline), Err(PublishError::Busy));
        for _ in SNAPSHOT_SLOTS..PUBLICATION_RING - 1 {
            publisher
                .note(harmonigraph_core::NoteEvent::on(1.0, SourceId::DIRECT, 0, 60, 0.8).into())
                .unwrap();
        }
        assert_eq!(
            publisher.note(harmonigraph_core::NoteEvent::off(2.0, SourceId::DIRECT, 0, 60).into()),
            Err(PublishError::Lost)
        );
    });
    let duration = start.elapsed();
    // Consume outside the audio guard, then exercise payload reuse under it.
    consumer.drain(|_| true);
    nice_assert_no_alloc::assert_no_alloc(|| publisher.baseline(&baseline).unwrap());
    consumer.drain(|_| true);
    eprintln!("canonical guarded fill: {SNAPSHOT_SLOTS} complete 64-voice payloads + {} notes + Busy/Lost = {duration:?}; no allocation/deallocation", PUBLICATION_RING - 1 - SNAPSHOT_SLOTS);
}

#[test]
fn adaptive_settings_restore_readback_save_and_audio_adoption_agree() {
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    let policy = PolicyConfig {
        pitch_flexibility: 37,
        axes: 3,
        silence_ms: 4500,
        reset_loop: true,
        reset_stop: true,
        decode_fifths: true,
        decoded_fifth: 694_786_840,
        ..Default::default()
    };
    let mut state = restored(&device, 700.0);
    state.fields.insert(
        MUSICAL_SETTINGS.into(),
        serde_json::to_string(&MusicalSettings { adaptive: policy.into(), ..Default::default() })
            .unwrap(),
    );
    device.load(state);
    let settings = device.settings();
    assert_eq!(shown(&settings).view.resolved.policy, policy);
    let saved: MusicalSettings =
        serde_json::from_str(&device.save().fields[MUSICAL_SETTINGS]).unwrap();
    assert_eq!(PolicyConfig::from(saved.adaptive), policy);
    device.run(0, vec![], None);
    assert_eq!(shown(&settings).view.resolved.policy, policy);
    let defaults: MusicalSettings = serde_json::from_str("{\"adaptive\":{}}").unwrap();
    assert_eq!(PolicyConfig::from(defaults.adaptive), PolicyConfig::default());
    let old: MusicalSettings =
        serde_json::from_str(r#"{"adaptive":{"version":2,"harmonic":20000,"pitch_scale":20}}"#)
            .unwrap();
    assert_eq!(PolicyConfig::from(old.adaptive), PolicyConfig::default());
    let clamped: MusicalSettings =
        serde_json::from_str(r#"{"adaptive":{"pitch_flexibility":65535}}"#).unwrap();
    assert_eq!(PolicyConfig::from(clamped.adaptive).pitch_flexibility, 100);
    let fifth: MusicalSettings =
        serde_json::from_str(r#"{"adaptive":{"decoded_fifth":800000000}}"#).unwrap();
    assert_eq!(
        PolicyConfig::from(fifth.adaptive).decoded_fifth,
        TuningAxis::Three.clamp(i32::MAX),
        "a decoded fifth past the Perfect fifth's range opens at its edge"
    );
}

/// The presentation clock, which dates the live audio ring and the take's
/// published clock, runs one block's length per process call at the host's
/// rate.
#[test]
fn the_presentation_clock_advances_by_each_blocks_length() {
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    for block in 0..3 {
        device.run(block * 64, vec![], None);
    }
    let seconds = device.wrapper().test_inspect_plugin(|plugin| plugin.clock.end());
    assert!((seconds - 3.0 * 64.0 / 48_000.0).abs() < 1e-12, "{seconds} s after three blocks");
}

/// A fresh Hub leaves player pitch alone until someone picks a retuning
/// engine: the saved param and the engine the audio thread adopted both say
/// so.
#[test]
fn a_fresh_hub_passes_note_pitch_through() {
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    device.run(0, vec![], None);
    let (param, engine) = device.wrapper().test_inspect_plugin(|plugin| {
        (plugin.params.int(crate::Row::TuningEngine), plugin.params.map_playback.lock().engine)
    });
    assert_eq!(param, 0);
    assert_eq!(engine, harmonigraph_core::lattice_map::TuningEngine::Off);
}

#[test]
fn lattice_maps_restore_without_editor_preserves_geometry_and_shared_tuning() {
    use harmonigraph_core::lattice_map::{LatticeMap, TuningEngine};
    use harmonigraph_ui::lattice_maps::{MapDocument, MapEdit, MapOffsetLane};
    use std::sync::Arc;
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    let original_names = device
        .wrapper()
        .test_inspect_plugin(|plugin| crate::lattice_maps::view(&plugin.params).names);
    let mut map =
        LatticeMap { position: harmonigraph_core::LatticePos::new(50, 0, 0), ..Default::default() };
    map.replace(harmonigraph_core::LatticePos::new(54, 0, 0));
    let mut document = MapDocument::default();
    document.capture(map, "Distant passage".into());
    map.position = harmonigraph_core::LatticePos::new(50, -2, 1);
    let mut state = restored(&device, 696.5);
    state.fields.insert("lattice-maps".into(), serde_json::to_string(&document).unwrap());
    state.params.insert("lattice-map".into(), harmonigraph_clap::plugin::ParamValue::I32(1));
    for (id, value) in [
        ("map-fifths", 0),
        ("map-thirds", 8),
        ("map-sevenths", -9),
        ("map-fifths-extension", 5),
        ("map-thirds-extension", -1),
        ("map-sevenths-extension", 1),
    ] {
        state.params.insert(id.into(), harmonigraph_clap::plugin::ParamValue::I32(value));
    }
    state.params.insert("tuning-engine".into(), harmonigraph_clap::plugin::ParamValue::I32(2));
    device.load(state);
    device.wrapper().test_inspect_plugin(|plugin| {
        let preview = crate::lattice_maps::view(&plugin.params);
        assert_eq!(preview.playback.engine, TuningEngine::LatticeMap);
        assert_eq!(preview.playback.map, Some(map));
        assert_eq!(preview.playback.offset, preview.offsets.total());
        assert!(!Arc::ptr_eq(&original_names, &preview.names));
        assert_eq!(&preview.names[1], &(1, "Distant passage".into()));
        assert!(
            preview.unadopted.is_some(),
            "restored intent must be visible before audio adoption"
        );
    });
    let saved = device.save();
    let recalled: MapDocument = serde_json::from_str(&saved.fields["lattice-maps"]).unwrap();
    assert_eq!(
        recalled.map(1),
        Some(LatticeMap { position: harmonigraph_core::LatticePos::ORIGIN, ..map })
    );
    for (id, value) in [
        ("map-fifths", 0),
        ("map-thirds", 8),
        ("map-sevenths", -9),
        ("map-fifths-extension", 5),
        ("map-thirds-extension", -1),
        ("map-sevenths-extension", 1),
    ] {
        assert!(
            matches!(saved.params[id], harmonigraph_clap::plugin::ParamValue::I32(actual) if actual == value)
        );
    }
    device.run(0, vec![], None);
    device.wrapper().test_inspect_plugin(|plugin| {
        let playback = *plugin.params.map_playback.lock();
        assert_eq!(playback.engine, TuningEngine::LatticeMap);
        assert_eq!(playback.selected, 1);
        assert_eq!(playback.map, Some(map));
        assert_eq!(plugin.configuration.as_ref().unwrap().current().tuning.three, 696_500_000);
    });
    let context = device.editor_host();
    let destination = map.position + harmonigraph_core::LatticePos::new(-3, 0, 0);
    let mut changed = map;
    assert!(changed.replace(destination), "the edit must change the captured geometry");
    device.wrapper().test_inspect_plugin(|plugin| {
        assert!(!crate::lattice_maps::view(&plugin.params).unadopted.is_some());
        let edit = |action| crate::lattice_maps::edit(&plugin.params, context.as_ref(), action);
        let dirty = &plugin.params.settings.modified;
        dirty.store(false, std::sync::atomic::Ordering::Release);
        edit(MapEdit::EditShape(true));
        edit(MapEdit::Replace(map.node(64)));
        edit(MapEdit::Undo);
        assert!(
            !dirty.load(std::sync::atomic::Ordering::Acquire),
            "an edit that changes nothing must not mark the project modified"
        );
        edit(MapEdit::Replace(destination));
        assert!(dirty.load(std::sync::atomic::Ordering::Acquire));
        let preview = crate::lattice_maps::view(&plugin.params);
        assert!(preview.editing() && preview.can_undo);
        assert_eq!(preview.playback.map, Some(changed));
        assert_eq!(
            plugin.params.maps.read().map(1),
            Some(LatticeMap { position: harmonigraph_core::LatticePos::ORIGIN, ..changed }),
            "an edit is saved to the selected map itself"
        );
        assert!(
            preview.unadopted.is_some(),
            "editing follows intent before audio adopts the shape"
        );
        edit(MapEdit::Undo);
        let undone = crate::lattice_maps::view(&plugin.params);
        assert_eq!(undone.playback.map, Some(map));
        assert!(!undone.can_undo);
        edit(MapEdit::Replace(destination));
        edit(MapEdit::Duplicate);
        let copied = crate::lattice_maps::view(&plugin.params);
        assert_eq!(&copied.names[2], &(2, "Distant passage copy".into()));
        // Thirds stand at fine 8, coarse −1: two arrow presses carry into
        // Coarse rather than stopping at the end of Fine, and the second,
        // pressed before the host has applied the first, still counts — with
        // a frame drawn between them, as the editor draws one.
        let right = harmonigraph_core::LatticePos::new(0, 1, 0);
        edit(MapEdit::Translate(right, MapOffsetLane::Fine));
        crate::lattice_maps::view(&plugin.params);
        edit(MapEdit::Translate(right, MapOffsetLane::Fine));
    });
    device.run(64, vec![], None);
    device.wrapper().test_inspect_plugin(|plugin| {
        let playback = *plugin.params.map_playback.lock();
        let moved = crate::lattice_maps::offsets(&plugin.params);
        assert_eq!((moved.fine.fives, moved.extension.fives), (0, 0), "−2 + 2 is exactly zero");
        assert_eq!(playback.selected, 2, "a duplicate is selected for further edits");
        let translated = LatticeMap { position: moved.total(), ..changed };
        assert_eq!(playback.map, Some(translated), "audio adopts the translated shape");
        assert!(
            !crate::lattice_maps::view(&plugin.params).can_undo,
            "undo never reaches past the selected map to slot 1's edits"
        );
        // Each row of the editor's list deletes its own map, so a delete can
        // name a map that is not selected.
        crate::lattice_maps::edit(&plugin.params, context.as_ref(), MapEdit::Delete(0));
    });
    device.run(80, vec![], None);
    device.wrapper().test_inspect_plugin(|plugin| {
        let playback = *plugin.params.map_playback.lock();
        assert_eq!(playback.selected, 2, "deleting another map leaves the selection alone");
        let moved = crate::lattice_maps::offsets(&plugin.params).total();
        let kept = LatticeMap { position: moved, ..changed };
        assert_eq!(playback.map, Some(kept), "new attacks keep the selected map");
        let names = crate::lattice_maps::view(&plugin.params).names;
        assert!(names.iter().all(|(id, _)| *id != 0), "the deleted map leaves the list");
        crate::lattice_maps::edit(&plugin.params, context.as_ref(), MapEdit::Delete(2));
    });
    device.run(96, vec![], None);
    device.wrapper().test_inspect_plugin(|plugin| {
        let playback = *plugin.params.map_playback.lock();
        assert_eq!(playback.selected, 1, "deleting the last map in order selects the previous");
        let moved = crate::lattice_maps::offsets(&plugin.params).total();
        let neighbour = LatticeMap { position: moved, ..changed };
        assert_eq!(playback.map, Some(neighbour), "new attacks are tuned by the neighbour");
    });
    device.load(saved);
    device.wrapper().test_inspect_plugin(|plugin| {
        let preview = crate::lattice_maps::view(&plugin.params);
        assert!(!preview.editing() && !preview.can_undo, "undo never reaches a restored project");
        assert_eq!(preview.playback.map, Some(map));
        assert!(
            preview.unadopted.is_some(),
            "audio still holds the edited shape until its next callback"
        );
    });
    device.run(128, vec![], None);
    device.wrapper().test_inspect_plugin(|plugin| {
        assert_eq!(plugin.params.map_playback.lock().map, Some(map));
    });
    // Old or partial presets must not retain lanes from the current preset,
    // and repaired plain values must agree with host and editor readback.
    let mut partial = device.save();
    partial.params.insert("map-fifths".into(), harmonigraph_clap::plugin::ParamValue::I32(50));
    partial.params.insert("map-thirds".into(), harmonigraph_clap::plugin::ParamValue::I32(-50));
    partial.params.insert("map-follow".into(), harmonigraph_clap::plugin::ParamValue::I32(1));
    for id in [
        "map-fifths-extension",
        "map-thirds-extension",
        "map-sevenths-extension",
        "tuning-engine",
        "lattice-map",
    ] {
        partial.params.remove(id);
    }
    device.load(partial);
    device.run(192, vec![], None);
    device.wrapper().test_inspect_plugin(|plugin| {
        let preview = crate::lattice_maps::view(&plugin.params);
        // The instance was in Lattice Map on slot 1: neither leaks in.
        assert_eq!(preview.playback.engine, TuningEngine::Off);
        assert_eq!(preview.playback.selected, 0);
        assert_eq!(preview.offsets.extension, harmonigraph_core::LatticePos::ORIGIN);
        assert_eq!(preview.playback.offset, harmonigraph_core::LatticePos::new(9, -9, -9));
        assert_eq!(plugin.params.map_playback.lock().offset, preview.playback.offset);
        assert_eq!(preview.playback.follow, harmonigraph_core::lattice_map::Follow::Thirds);
    });
    let repaired = device.save();
    assert!(matches!(repaired.params["map-follow"], harmonigraph_clap::plugin::ParamValue::I32(1)));
    assert!(matches!(repaired.params["map-fifths"], harmonigraph_clap::plugin::ParamValue::I32(9)));
    assert!(matches!(
        repaired.params["map-thirds"],
        harmonigraph_clap::plugin::ParamValue::I32(-9)
    ));
}

/// A restore once stored an integer parameter's value unclamped, while the
/// host's label clamps it. An engine or map index out of range at either end —
/// hand-edited or foreign state, since the plugin never saves one — must load
/// as the value the host shows, in the editor and in the audio alike, and save
/// back as that value. On `main` before the repair, the high side's engines,
/// map label and saved values failed and the low side's saved values failed;
/// the selected slots and the low side's engines already agreed, and stay
/// here as the agreement this pins.
#[test]
fn an_out_of_range_engine_or_map_restores_as_the_value_the_host_shows() {
    use harmonigraph_core::lattice_map::TuningEngine;
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    for (round, (engine, map, shown, decoded, slot)) in [
        (3, 200, "Lattice Map", TuningEngine::LatticeMap, 127),
        (-1, -5, "Pass through", TuningEngine::Off, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let mut state = device.save();
        state
            .params
            .insert("tuning-engine".into(), harmonigraph_clap::plugin::ParamValue::I32(engine));
        state.params.insert("lattice-map".into(), harmonigraph_clap::plugin::ParamValue::I32(map));
        device.load(state);
        device.run(64 * round as i64, vec![], None);
        device.wrapper().test_inspect_plugin(|plugin| {
            let preview = crate::lattice_maps::view(&plugin.params);
            let audio = *plugin.params.map_playback.lock();
            assert_eq!(plugin.params.label(crate::Row::TuningEngine), shown);
            assert_eq!(preview.playback.engine, decoded, "the editor disagrees");
            assert_eq!(audio.engine, decoded, "the audio disagrees");
            let label = plugin.params.label(crate::Row::LatticeMap);
            assert!(label.starts_with(&format!("{} · ", slot + 1)), "the host shows {label}");
            assert_eq!((preview.playback.selected, audio.selected), (slot, slot));
        });
        let repaired = device.save();
        assert!(matches!(
            (&repaired.params["tuning-engine"], &repaired.params["lattice-map"]),
            (harmonigraph_clap::plugin::ParamValue::I32(e), harmonigraph_clap::plugin::ParamValue::I32(m))
                if (*e, *m as usize) == (decoded.index(), slot)
        ));
    }
}

/// The editor draws the configuration under the engine the parameters
/// select, as a take records it: the Decoded fifth under Pass through with
/// Decode fifths on, the saved fifth under Lattice Map.
#[test]
fn the_editor_draws_the_board_s_tuning_under_pass_through() {
    use harmonigraph_core::lattice_map::TuningEngine;
    let _scope = crate::test_scope::enter();
    let mut device = hub();
    device.activate();
    let policy =
        PolicyConfig { decode_fifths: true, decoded_fifth: 696_000_000, ..PolicyConfig::default() };
    let saved_fifth = harmonigraph_core::tuning::microcents(701.0);
    device.settings().submit(ConfigEdit {
        axes: [None, Some(saved_fifth), None, None, None],
        policy: Some(policy),
        ..Default::default()
    });
    device.run(0, vec![], None);
    let params = device.wrapper().test_inspect_plugin(|plugin| plugin.params.clone());
    let drawn = || {
        let view = params.configuration_view();
        (view.resolved.tuning.three, view.board)
    };
    assert_eq!(drawn(), (696_000_000, true));
    params.set(crate::Row::TuningEngine, TuningEngine::LatticeMap.index() as f32);
    assert_eq!(drawn(), (saved_fifth, false));
}

#[test]
fn camera_orbit_gestures_reach_the_host_once_per_axis() {
    use harmonigraph_ui::params::ParamBackend;
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    let context = device.editor_host();
    let params = device.wrapper().test_inspect_plugin(|plugin| plugin.params.clone());
    let gestures = std::cell::Cell::default();
    let backend = crate::PluginParamBackend {
        params: &params,
        host: context.as_ref(),
        configuration: params.settings.view(),
        gestures: &gestures,
    };
    let mut events = Vec::new();
    for frame in 0..3 {
        for key in [ParamKey::CameraYaw, ParamKey::CameraPitch] {
            backend.begin_set(key);
            backend.set(key, 0.1 * (frame + 1) as f32);
        }
        events.extend(device.run(frame * 64, vec![], None).attempts);
    }
    for _ in 0..2 {
        for key in ParamKey::CAMERA {
            backend.end_set(key);
        }
    }
    events.extend(device.run(192, vec![], None).attempts);
    for key in [ParamKey::CameraYaw, ParamKey::CameraPitch] {
        let id = device.id(key);
        for kind in [CLAP_EVENT_PARAM_GESTURE_BEGIN, CLAP_EVENT_PARAM_GESTURE_END] {
            assert_eq!(events.iter().filter(|e| e.0 == kind && e.2 == id && e.3).count(), 1);
        }
        assert_eq!(
            events.iter().filter(|e| e.0 == CLAP_EVENT_PARAM_VALUE && e.2 == id && e.3).count(),
            3
        );
    }
    assert_eq!(gestures.get().open().count(), 0);
}

/// Every host gesture the editor holds open, a map-offset lane's as well as a
/// camera axis's, is ended once when the editor stops being seen, and the
/// close after that ends nothing more; a lane's gesture alone is ended by the
/// close. Through the plug-in's CLAP events, so what is counted is what the
/// host receives.
#[test]
fn the_editors_gestures_end_once_when_it_stops_being_seen_or_closes() {
    use harmonigraph_ui::lattice_maps::{MapAxis, MapEdit, MapOffsetLane};
    use harmonigraph_ui::params::ParamBackend;
    let _scope = crate::test_scope::enter();
    let mut device = hub().with_retune();
    device.activate();
    let context = device.editor_host();
    let (params, lock) = device
        .wrapper()
        .test_inspect_plugin(|plugin| (plugin.params.clone(), plugin.editor_shared.clone()));
    let (axis, lane) = (MapAxis::Thirds, MapOffsetLane::Extension);
    let with_backend = |act: &dyn Fn(&crate::PluginParamBackend)| {
        let shared = lock.lock();
        act(&crate::PluginParamBackend {
            params: &params,
            host: context.as_ref(),
            configuration: params.settings.view(),
            gestures: &shared.gestures,
        });
    };
    let lane_id =
        device.id_named(params.table.spec(crate::lattice_maps::lane_row(axis, lane).index()).name);
    let yaw_id = device.id(ParamKey::CameraYaw);
    let count = |events: &[_], kind, id| {
        events.iter().filter(|e: &&(u16, _, u32, bool)| e.0 == kind && e.2 == id && e.3).count()
    };
    let (begins, ends) = (CLAP_EVENT_PARAM_GESTURE_BEGIN, CLAP_EVENT_PARAM_GESTURE_END);

    // A camera drag and a lane drag, then the editor unseen, then closed.
    with_backend(&|backend| {
        backend.begin_set(ParamKey::CameraYaw);
        backend.set(ParamKey::CameraYaw, 0.2);
        backend.edit_lattice_map(MapEdit::BeginOffset(axis, lane));
        backend.edit_lattice_map(MapEdit::Offset(axis, lane, 1));
    });
    let mut events = device.run(0, vec![], None).attempts;
    let open = lock.end_unseen().expect("the editor lock is free");
    crate::editor::gestures::end(open, context.as_ref());
    events.extend(device.run(64, vec![], None).attempts);
    for id in [yaw_id, lane_id] {
        assert_eq!(count(&events, begins, id), 1, "param {id}");
        assert_eq!(count(&events, ends, id), 1, "unseen left {id} open");
    }
    let open = lock.end_window_session();
    crate::editor::gestures::end(open, context.as_ref());
    events.extend(device.run(128, vec![], None).attempts);
    for id in [yaw_id, lane_id] {
        assert_eq!(count(&events, ends, id), 1, "the close ended {id} again");
    }

    // The lane's drag still held at the reveal: its next value begins a new
    // gesture, which the drag's end ends.
    with_backend(&|backend| backend.edit_lattice_map(MapEdit::Offset(axis, lane, 2)));
    with_backend(&|backend| backend.edit_lattice_map(MapEdit::EndOffset(axis, lane)));
    events.extend(device.run(192, vec![], None).attempts);
    assert_eq!(count(&events, begins, lane_id), 2, "a value set with no gesture open");
    assert_eq!(count(&events, ends, lane_id), 2);

    // A lane's gesture alone, begun and never moved, ended by the close.
    with_backend(&|backend| backend.edit_lattice_map(MapEdit::BeginOffset(axis, lane)));
    let open = lock.end_window_session();
    crate::editor::gestures::end(open, context.as_ref());
    events.extend(device.run(256, vec![], None).attempts);
    assert_eq!(count(&events, begins, lane_id), 3, "a lane's begin bypassed the registry");
    assert_eq!(count(&events, ends, lane_id), 3, "the close left a lane open");
}

/// The own window's close button tells the host through `EditorHost::closed`,
/// which must ask for the editor's destruction: Bitwig's show/hide control
/// shows the editor closed after `closed(true)`, and still open after a hide
/// or `closed(false)` (#1609).
#[test]
fn the_close_button_tells_the_host_to_destroy_the_editor() {
    let _scope = crate::test_scope::enter();
    let device = hub().with_retune();
    let host = device.editor_host();
    assert!(host.closed(), "a host with a gui extension is told");
    assert_eq!(device.host().closed.load(Ordering::Relaxed), 1);
    assert!(device.host().closed_destroyed.load(Ordering::Relaxed));
}

/// A size the host gives its plugin window before it attaches the editor
/// (CLAP allows `set_size` before `set_parent`) becomes the editor's only
/// where the editor will open in the host's window. Opening in its own, it
/// opens at the size its project saved, which a save keeps
/// (yan-h/egui-plugin-window#12); a platform that offers no own window takes
/// the host's size either way.
#[test]
fn a_host_size_before_attach_is_the_editors_only_where_it_opens_docked() {
    let _scope = crate::test_scope::enter();
    let device = hub().with_retune();
    let gui = unsafe {
        &*(((*device.plugin).get_extension.unwrap())(device.plugin, CLAP_EXT_GUI.as_ptr())
            .cast::<clap_plugin_gui>())
    };
    let state = device.wrapper().test_inspect_plugin(|plugin| plugin.params.editor_state.clone());
    let (mut api, mut floating) = (ptr::null(), false);
    unsafe {
        assert!((gui.get_preferred_api.unwrap())(device.plugin, &mut api, &mut floating));
        assert!((gui.create.unwrap())(device.plugin, api, false));
    }
    let set_size = |(width, height): (u32, u32)| {
        assert!(unsafe { (gui.set_size.unwrap())(device.plugin, width, height) });
    };
    let size = || {
        let (mut width, mut height) = (0, 0);
        assert!(unsafe { (gui.get_size.unwrap())(device.plugin, &mut width, &mut height) });
        (width, height)
    };
    let saved = size();
    let host = (saved.0 + 160, saved.1 + 90);

    state.set_docked(false);
    set_size(host);
    let own = cfg!(target_os = "macos");
    assert_eq!(size(), if own { saved } else { host }, "the project's saved size stays");

    state.set_docked(true);
    let host = (host.0 + 40, host.1 + 30);
    set_size(host);
    assert_eq!(size(), host, "docked, the host's size is the editor's");
}
