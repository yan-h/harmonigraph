//! Instance controls exercised through the production exported callbacks.
use super::*;
use harmonigraph_ui::params::InstanceEdit;

#[test]
fn retune_off_forgets_held_and_released_context_but_preserves_sounding_pitch() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    musical_tests::configure(&pair.hub, harmonigraph_core::Tuning::just());
    pair.step(vec![note(1, 0, 60, 0, true), note(2, 0, 64, 1, true)]);
    pair.idle();
    pair.step(vec![note(1, 0, 60, 0, false)]);
    pair.idle();
    let before = inspect_hub(&pair.hub, |hub| hub.test_voice(0, 0, 64)).unwrap();
    assert_ne!(before.frozen_offset_microcents, 0, "a correction must actually be preserved");
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_context()), 1);
    assert_eq!(
        inspect_hub(&pair.hub, |hub| hub.test_memory()),
        1,
        "the fixture must reach the toggle with both held and released pitches"
    );

    pair.tune.shared().set_retune(false);
    assert!(pair.idle().is_empty(), "a participation edit sends no note-off or pitch jump");
    let after = inspect_hub(&pair.hub, |hub| hub.test_voice(0, 0, 64)).unwrap();
    assert_eq!(before, after);
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_context()), 0);
    let context = inspect_hub(&pair.hub, |hub| hub.test_next_context());
    assert!(context.context.is_empty(), "released memory must also be gone");
    assert_eq!(context.reference, 0, "the removed source must not leave a moving reference");

    pair.step(vec![expression(2, 0.25, 0), note(3, 0, 67, 1, true), expression(3, 0.125, 1)]);
    let output = pair.idle();
    let old = output
        .iter()
        .find_map(|(_, event)| match event {
            Event::Expression { id: 2, value, .. } => Some(*value),
            _ => None,
        })
        .unwrap();
    assert!((old - (0.25 + before.frozen_offset_microcents as f64 / 100_000_000.0)).abs() < 1e-10);
    let new: Vec<_> = output
        .iter()
        .filter_map(|(_, event)| match event {
            Event::Expression { id: 3, value, .. } => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(new, vec![0.125], "a bypassed attack retains only the player's expression");
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_context()), 0);
    assert!(inspect_hub(&pair.hub, |hub| hub.test_next_context()).context.is_empty());
    assert_eq!(pair.misses(), 0, "intentional bypass is not a missed correction");

    pair.tune.shared().set_retune(true);
    pair.step(vec![note(4, 0, 69, 0, true)]);
    assert!(tuning_of(&pair.idle()).is_some());
    assert_eq!(
        inspect_hub(&pair.hub, |hub| hub.test_context()),
        1,
        "only the new attack joins; bypassed and previously removed held notes stay excluded"
    );
    pair.tune.shared().set_retune(false);
    pair.tune.shared().set_retune(true);
    pair.idle();
    assert_eq!(
        inspect_hub(&pair.hub, |hub| hub.test_context()),
        0,
        "an off/on between callbacks still removes the old context"
    );
    pair.tune.run(pair.raw, vec![note(5, 0, 71, 0, true)], None);
    pair.tune.shared().set_retune(false);
    pair.tune.shared().set_retune(true);
    pair.hub.run(pair.raw, vec![], None);
    pair.raw += 512;
    assert_eq!(
        inspect_hub(&pair.hub, |hub| hub.test_context()),
        0,
        "an already-queued capture cannot repopulate the cleared context"
    );
    assert_eq!(tuning_of(&pair.idle()), Some(0.0));
    assert_eq!(pair.misses(), 0);
}

#[test]
fn show_hides_and_restores_held_output_without_changing_tuning() {
    use harmonigraph_core::NoteTracker;
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.activate();
    let mut tune = Device::new(true);
    tune.activate();
    let mut pair = Pair { hub, tune, raw: 0 };
    let mut tracker = NoteTracker::new();
    pair.step(vec![note(1, 0, 60, 0, true)]);
    pair.idle();
    capture.display_into(&mut tracker, |_, _| {});
    assert_eq!(tracker.held_count(), 1);
    let voice = inspect_hub(&pair.hub, |hub| hub.test_voice(0, 0, 60)).unwrap();

    pair.tune.shared().set_show(false);
    // The source can be suspended: only Harmonigraph gets a callback here.
    pair.hub.run(pair.raw, vec![], None);
    pair.raw += 512;
    capture.display_into(&mut tracker, |_, _| {});
    assert_eq!(tracker.held_count(), 0);
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_context()), 1);
    pair.step(vec![note(2, 0, 64, 0, true)]);
    assert!(tuning_of(&pair.idle()).is_some(), "hidden attacks still retune");
    capture.display_into(&mut tracker, |_, _| {});
    assert_eq!(tracker.held_count(), 0, "subsequent deltas cannot unhide the source");

    pair.tune.shared().set_retune(false);
    pair.tune.shared().set_show(true);
    pair.idle();
    capture.display_into(&mut tracker, |_, _| {});
    assert_eq!(tracker.held_count(), 2, "Show restores held output even with Retune off");
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_voice(0, 0, 60)).unwrap(), voice);
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_context()), 0);
}

#[test]
fn central_edits_include_harmonigraph_and_use_each_tuners_host_latency() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.idle();
    let rows = instances::snapshots();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].is_hub);
    for row in &rows {
        instances::edit(row.id, InstanceEdit::Retune(false));
        instances::edit(row.id, InstanceEdit::Show(false));
    }
    assert!(instances::snapshots().iter().all(|row| !row.retune && !row.show));
    let output = pair.hub.run_format(pair.raw, vec![note(1, 0, 60, 0, true)], None, None, 512);
    assert!(output.values.is_empty());
    pair.raw += 512;
    let output = pair.hub.run_format(pair.raw, vec![], None, None, 512);
    assert_eq!(
        output.values.len(),
        1,
        "Harmonigraph's own input passes through without correction"
    );
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_context()), 0);

    let tuner = &rows[1];
    instances::edit(tuner.id, InstanceEdit::Name("Bass".to_owned()));
    instances::edit(tuner.id, InstanceEdit::Delay(3));
    pair.tune.main();
    assert_eq!(pair.tune.param_value(DELAY_PARAM).round(), 2.0);
    assert_eq!(pair.tune.latency(), 512, "the host still reads the running activation");
    assert!(instances::snapshots()[1].delay_text.contains("waiting for host reactivation"));
    pair.tune.reactivate_format(48000.0, 512);
    assert_eq!(pair.tune.latency(), 1536);
    assert_eq!(instances::snapshots()[1].name, "Bass");
    drop(pair.tune);
    instances::edit(tuner.id, InstanceEdit::Retune(true));
    assert_eq!(instances::snapshots().len(), 1, "a retired row cannot receive stale GUI edits");
}

#[test]
fn instance_settings_restore_independently_and_default_missing_fields() {
    use nice_plug::wrapper::clap::setup::Setup;
    let shared = setup::Shared::source();
    let adapter = setup::Adapter(shared.clone(), None);
    let mut state = nice_plug::plugin::PluginState {
        version: String::new(),
        params: Default::default(),
        fields: Default::default(),
    };
    shared.set_retune(false);
    shared.set_show(true);
    shared.set_name("Reference".to_owned());
    adapter.save(&mut state);
    let restored = setup::Shared::hub();
    setup::Adapter(restored.clone(), None).prepare(&state).unwrap().commit();
    assert_eq!(restored.retuning() & 1, 0);
    assert!(restored.show.load(Ordering::Acquire));
    assert_eq!(*restored.name.lock().unwrap(), "Reference");
    state.fields.insert("tuning-instance".to_owned(), r#"{"show":false}"#.to_owned());
    adapter.prepare(&state).unwrap().commit();
    assert_eq!(shared.retuning() & 1, 1, "a missing field falls back to its default");
    assert!(!shared.show.load(Ordering::Acquire));
    assert!(shared.name.lock().unwrap().is_empty());

    // Only an instance with nothing saved at all starts by role.
    restored.set_retune(true);
    state.fields.clear();
    setup::Adapter(restored.clone(), None).prepare(&state).unwrap().commit();
    assert_eq!(restored.retuning() & 1, 0, "the Hub's own input starts with Retune off");
}

#[test]
fn track_names_follow_the_host_until_overridden() {
    let _scope = crate::test_scope::enter();
    for tuner in [false, true] {
        for compat in [false, true] {
            let device = Device::with_host(
                tuner,
                Host {
                    track_info: true,
                    track_info_compat: compat,
                    track_name: std::sync::Mutex::new(Some("Bäss".to_owned())),
                    ..Default::default()
                },
            );
            let snapshot = || instances::snapshots().into_iter().next().unwrap();
            let row = snapshot();
            assert_eq!(row.display_name, "Bäss", "init reads the host before any editor opens");
            assert!(row.name.is_empty(), "automatic names are not saved overrides");
            let extension = unsafe {
                &*((*device.plugin).get_extension.unwrap()(
                    device.plugin,
                    if compat { CLAP_EXT_TRACK_INFO_COMPAT } else { CLAP_EXT_TRACK_INFO }.as_ptr(),
                )
                .cast::<clap_plugin_track_info>())
            };
            let rename = |name: Option<&str>| {
                *device._stats.track_name.lock().unwrap() = name.map(str::to_owned);
                unsafe {
                    extension.changed.unwrap()(device.plugin);
                }
            };
            rename(Some("Lead"));
            assert_eq!(snapshot().display_name, "Lead");
            instances::edit(row.id, harmonigraph_ui::params::InstanceEdit::Name("Solo".to_owned()));
            rename(Some("Keys"));
            assert_eq!(snapshot().display_name, "Solo");
            instances::edit(row.id, harmonigraph_ui::params::InstanceEdit::Name(String::new()));
            assert_eq!(snapshot().display_name, "Keys");
            let fallback =
                if tuner { format!("Tune {}", row.id) } else { "Harmonigraph input".to_owned() };
            rename(None);
            assert_eq!(
                snapshot().display_name,
                fallback,
                "unavailable metadata clears the old track name"
            );
            rename(Some(""));
            assert_eq!(snapshot().display_name, fallback);
        }
        let _device = Device::new(tuner);
        let row = instances::snapshots().remove(0);
        assert_eq!(
            row.display_name,
            if tuner { format!("Tune {}", row.id) } else { "Harmonigraph input".to_owned() }
        );
    }
}

#[test]
fn live_note_count_clears_when_the_host_stops_callbacks() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.step(vec![note(1, 0, 60, 0, true)]);
    pair.idle();
    let count =
        || instances::snapshots().into_iter().find(|row| !row.is_hub).unwrap().pitches.len();
    assert_eq!(count(), 1, "the note has reached the output");
    pair.tune.deactivate();
    assert_eq!(count(), 0, "the display clears without another process callback");
}

#[test]
fn source_dot_pitches_follow_emitted_tuning_bend_and_releases_even_when_hidden() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.tune.shared().set_show(false);
    pair.step(vec![note(1, 0, 60, 0, true), note(2, 0, 64, 1, true)]);
    pair.idle();
    let pitches = || instances::snapshots().into_iter().find(|row| !row.is_hub).unwrap().pitches;
    let expected: Vec<_> = [60, 64]
        .into_iter()
        .map(|key| {
            inspect_hub(&pair.hub, |hub| hub.test_voice(0, 0, key).unwrap().pitch_microcents) as f32
                / 100_000_000.0
        })
        .collect();
    assert_eq!(pitches(), expected, "dots include the correction emitted with each attack");
    pair.step(vec![expression(2, 0.5, 0), raw_midi([0xe0, 0, 96], 1)]);
    pair.idle();
    let bent = pitches();
    assert!((bent[0] - expected[0] - 1.0).abs() < 0.0001);
    assert!((bent[1] - expected[1] - 1.5).abs() < 0.0001);
    pair.step(vec![note(1, 0, 60, 0, false)]);
    pair.idle();
    assert_eq!(pitches(), vec![bent[1]]);
    pair.tune.deactivate();
    assert!(pitches().is_empty());
}
