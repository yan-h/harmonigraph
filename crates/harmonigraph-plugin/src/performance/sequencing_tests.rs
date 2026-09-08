//! Full production factory defaults: central input ownership and D512 output.
use super::*;
#[path = "musical_tests.rs"]
mod musical_tests;

fn inspect_hub<R>(device: &Device, f: impl FnOnce(&hub::Hub) -> R) -> R {
    let wrapper = unsafe {
        &*((*device.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    wrapper.test_inspect_plugin(|plugin| f(plugin.aggregation.as_ref().unwrap()))
}

fn inspect_source<R>(device: &Device, f: impl FnOnce(&source::Source) -> R) -> R {
    let wrapper = unsafe {
        &*((*device.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    wrapper.test_inspect_plugin(|plugin| f(plugin.source.as_ref().unwrap()))
}

fn production_pair() -> (Device, Device) {
    let uuid = SavedUuid::default();
    let calibration = Calibration { offset: 0 };
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, calibration);
    hub.activate_format(44100.0, 512);
    let mut source = Device::new(true);
    source.configure_format(uuid, true, calibration);
    source.activate_format(44100.0, 512);
    for raw in [0, 512, 1024] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    (hub, source)
}

fn raw_midi(data: [u8; 3], time: u32) -> Input {
    Input::Midi(clap_event_midi {
        header: header::<clap_event_midi>(CLAP_EVENT_MIDI, time),
        port_index: 0,
        data,
    })
}

fn tuning_parameter(hub: &Device, cents: f32, time: u32) -> Input {
    use harmonigraph_ui::params::ParamKey;
    use nice_plug::prelude::Param;
    let mut id = None;
    for index in 0..unsafe { hub.params().count.unwrap()(hub.plugin) } {
        let mut info = unsafe { std::mem::zeroed() };
        assert!(unsafe { hub.params().get_info.unwrap()(hub.plugin, index, &mut info) });
        if unsafe { CStr::from_ptr(info.name.as_ptr()) }.to_bytes()
            == ParamKey::Three.host_name().as_bytes()
        {
            id = Some(info.id);
        }
    }
    let params = crate::HarmonigraphParams::default();
    Input::Param(clap_event_param_value {
        header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, time),
        param_id: id.unwrap(),
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value: f64::from(params.param_for(ParamKey::Three).preview_normalized(cents)),
    })
}

#[test]
fn production_sixteen_sources_complete_a_256_onset_cohort_and_hold_exact_credit() {
    let _scope = crate::test_scope::enter();
    let synthetic = std::env::var_os("HARMONIGRAPH_MUSICAL_MAX").is_some();
    let address = |index: i32| {
        if synthetic {
            ((index / 4) as i16, 48 + (index % 4) as i16 * 12)
        } else {
            (0, index as i16 + 48)
        }
    };
    let mut source_max = 0;
    let mut hub_max = 0;
    let mut callback_sum = 0;
    let uuid = SavedUuid::default();
    let calibration = Calibration { offset: 0 };
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, calibration);
    hub.activate_format(44100.0, 512);
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let sources: [Device; 16] = std::array::from_fn(|_| {
        let mut source = Device::new(true);
        source.configure_format(uuid, true, calibration);
        source.activate_format(44100.0, 512);
        source
    });
    let session = registry::global().lock().unwrap().test_session(uuid);
    for raw in [0, 512, 1024] {
        for source in &sources {
            source.run_format(raw, vec![], None, None, 512);
        }
        hub.run_format(raw, vec![], None, None, 512);
    }
    if synthetic {
        // Existing test-only serialized inspection installs one complete owned
        // config before any musical request. This all-zero arithmetic ceiling
        // is outside UI-supported tuning, never a production setting/path.
        let wrapper = unsafe {
            &*((*hub.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
        };
        wrapper.test_with_plugin(|plugin| {
            use harmonigraph_core::configuration::{ConfigReducer, TuningModes};
            plugin.configuration.as_mut().unwrap().reducer = ConfigReducer::new(
                harmonigraph_core::Tuning {
                    c_offset: 0,
                    three: 0,
                    five: 0,
                    seven: 0,
                    tolerance: 0,
                },
                TuningModes {
                    tempered: harmonigraph_core::Tempered::default(),
                    auto: [false; 2],
                    learning: false,
                },
            );
        });
    }
    for source in &sources {
        let sink = source.run_format(
            1536,
            (0..16)
                .map(|key| {
                    let (channel, note_key) = address(key);
                    note(key, channel, note_key, 0, true)
                })
                .collect(),
            None,
            None,
            512,
        );
        source_max = source_max.max(sink.callback_nanos);
        callback_sum += sink.callback_nanos;
    }
    let sink = hub.run_format(1536, vec![], None, None, 512);
    hub_max = hub_max.max(sink.callback_nanos);
    callback_sum += sink.callback_nanos;
    let mut onsets = [0; 16];
    let mut actual = [None; 16];
    let mut raw = 1536;
    for _ in 0..256 {
        raw += 512;
        for (index, source) in sources.iter().enumerate() {
            let output = source.run_format(raw, vec![], None, None, 512);
            source_max = source_max.max(output.callback_nanos);
            callback_sum += output.callback_nanos;
            let count = output.values.iter().filter(|(_, event)| event.attack().is_some()).count();
            if count != 0 {
                let offsets: Vec<_> = output
                    .values
                    .iter()
                    .filter_map(|(offset, event)| event.attack().map(|_| *offset))
                    .collect();
                let accepted_sample = raw + i64::from(*offsets.iter().max().unwrap());
                assert!(
                    actual[index].replace(accepted_sample).is_none(),
                    "no Source repeats its accepted onsets"
                );
                println!(
                    "CAPACITY_DELIVERY source={index} callback={raw} offsets={offsets:?} last_accepted={accepted_sample} extra={} marker_state={:?}",
                    accepted_sample - 2048,
                    inspect_hub(&hub, |hub| hub.test_cohort_delivery())
                );
                assert_eq!(count, 16);
                assert_eq!(
                    output
                        .values
                        .iter()
                        .filter(|(_, event)| matches!(event, Event::Expression { kind: 2, .. }))
                        .count(),
                    16
                );
                let mut keys: Vec<_> = output
                    .values
                    .iter()
                    .filter_map(|(_, event)| event.attack().map(|(_, _, key, _)| key))
                    .collect();
                keys.sort_unstable();
                let mut expected: Vec<_> = (0..16).map(|index| address(index).1 as u8).collect();
                expected.sort_unstable();
                assert_eq!(keys, expected);
            }
            onsets[index] += count;
            assert_eq!(source.source_snapshot().faults, 0);
        }
        let sink = hub.run_format(raw, vec![], None, None, 512);
        hub_max = hub_max.max(sink.callback_nanos);
        callback_sum += sink.callback_nanos;
        if onsets == [16; 16] {
            break;
        }
    }
    assert_eq!(onsets, [16; 16]);
    let counts = inspect_hub(&hub, |hub| hub.test_policy_counts());
    // Late-cohort replay can reevaluate unaccepted requests. Report that cost
    // too, while proving the first 256 sequential selections reach 255 voices.
    assert!(counts[0] >= 256 && counts[1] >= 32640);
    assert_eq!(counts[2], 255);
    let config = inspect_source(&sources[0], |source| {
        source.state.voices().next().unwrap().assignment.unwrap()
    });
    let candidates: Vec<_> = (0..16)
        .map(|index| {
            let target = harmonigraph_core::PitchClass::from_midi_note(address(index).1 as u8);
            harmonigraph_core::positions_within(-6..=6, -2..=2, 0..=0)
                .filter(|node| {
                    config.tuning.pitch_class(*node).signed_microcents_from(target).unsigned_abs()
                        <= harmonigraph_core::policy::CANDIDATE_RADIUS
                })
                .count()
        })
        .collect();
    if synthetic {
        assert!(candidates.iter().all(|count| *count == 65));
    }
    println!("MUSICAL max synthetic={synthetic} domain=65 candidates={candidates:?} policy[calls,sum_context,max_context]={counts:?} source_max_ns={source_max} hub_max_ns={hub_max} total_callback_ns={callback_sum} completed_raw={raw}");
    assert_eq!(session.credits.load(Ordering::Acquire), 256);
    assert!(sources
        .iter()
        .all(|source| source.source_snapshot().held == 16 && source.latency() == 512));
    raw += 512;
    let last_release = raw + actual.into_iter().flatten().max().unwrap() - 1536;
    for source in &sources {
        source.run_format(
            raw,
            (0..16)
                .map(|key| {
                    let (channel, note_key) = address(key);
                    note(key, channel, note_key, 0, false)
                })
                .collect(),
            None,
            None,
            512,
        );
    }
    hub.run_format(raw, vec![], None, None, 512);
    while raw < last_release + 4096 {
        raw += 512;
        for source in &sources {
            source.run_format(raw, vec![], None, None, 512);
        }
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert!(sources.iter().all(|source| source.source_snapshot().held == 0));
}

#[test]
fn production_unmatched_originals_settle_without_hiding_independent_clock() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    let mut output = source
        .run_format(
            1536,
            vec![
                expression(999, 0.5, 0),
                raw_midi([0xa0, 60, 37], 1),
                note(999, 0, 60, 2, false),
                raw_midi([0xf8, 0, 0], 3),
            ],
            None,
            None,
            512,
        )
        .values;
    assert_eq!(source.source_snapshot().input_cut, 4);
    // All four were copied to the Hub. The three that matched no note own no
    // work and owe no output, so their envelopes settle inside this callback;
    // only the forwarded clock still waits for its own output at input+D.
    for serial in 1..=3 {
        assert!(inspect_source(&source, |source| source.test_capture(serial)).is_none());
    }
    let clock = inspect_source(&source, |source| source.test_capture(4).unwrap());
    assert_eq!(clock.work_count, 0);
    assert!(clock.published, "every original was copied to the Hub");
    assert_eq!(source.source_snapshot().pending, 1);
    hub.run_format(1536, vec![], None, None, 512);
    for raw in (2048..8192).step_by(512) {
        output.extend(source.run_format(raw, vec![], None, None, 512).values);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(output.len(), 1);
    assert!(matches!(output[0].1, Event::Midi { data: [0xf8, 0, 0], .. }));
    assert_eq!(source.source_snapshot().captures, 0);
    assert_eq!(source.source_snapshot().obligations, 0);
    assert_eq!(source.source_snapshot().faults, 0);
    assert!(inspect_hub(&hub, |hub| hub.test_inputs(1).is_empty()));
}

#[test]
fn production_channel_headers_complete_all_sixty_four_captured_targets() {
    let _scope = crate::test_scope::enter();
    for terminal in [120, 123] {
        let (hub, source) = production_pair();
        let mut onsets = 0;
        let mut addresses = Vec::new();
        let mut raw = 1536;
        let events =
            (0..64).map(|index| note(index, 0, index as i16, index as u32, true)).collect();
        source.run_format(raw, events, None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        for _ in 0..128 {
            raw += 512;
            let output = source.run_format(raw, vec![], None, None, 512);
            onsets += output.values.iter().filter(|(_, event)| event.attack().is_some()).count();
            addresses.extend(output.values.iter().filter_map(|(_, event)| event.attack()));
            hub.run_format(raw, vec![], None, None, 512);
            if onsets == 64 {
                break;
            }
        }
        assert_eq!(
            onsets,
            64,
            "the fixture reaches all 64 sounding consumers: {addresses:?}; {:?}; Hub {:?}",
            source.source_snapshot(),
            inspect_hub(&hub, |hub| hub.test_input_sequence_progress())
        );
        assert_eq!(source.source_snapshot().held, 64);
        for controller in [7, terminal] {
            raw += 512;
            let mut output = source
                .run_format(raw, vec![raw_midi([0xb0, controller, 0], 0)], None, None, 512)
                .values;
            let serial = source.source_snapshot().input_cut;
            let original = inspect_source(&source, |source| source.test_capture(serial).unwrap());
            assert_eq!(
                original.work_count, 64,
                "CC{controller} owns the full immutable target group"
            );
            hub.run_format(raw, vec![], None, None, 512);
            for _ in 0..64 {
                raw += 512;
                output.extend(source.run_format(raw, vec![], None, None, 512).values);
                hub.run_format(raw, vec![], None, None, 512);
                if inspect_source(&source, |source| source.test_capture(serial).is_none()) {
                    break;
                }
            }
            assert_eq!(output.len(), 1, "CC{controller} has one physical wire effect: {output:?}");
            assert!(
                matches!(output[0].1, Event::Midi { data: [0xb0, cc, 0], .. } if cc == controller)
            );
            assert!(inspect_source(&source, |source| source.test_capture(serial).is_none()));
        }
        assert!(
            (1..=64).all(|lifetime| inspect_hub(&hub, |hub| hub.test_voice(0, lifetime).is_none()))
        );
        assert_eq!(source.source_snapshot().note_off_owed, if terminal == 120 { 64 } else { 0 });
        raw += 512;
        let events = (0..64).map(|index| note(index, 0, index as i16, 0, false)).collect();
        let mut releases = source.run_format(raw, events, None, None, 512).values;
        hub.run_format(raw, vec![], None, None, 512);
        for _ in 0..64 {
            raw += 512;
            releases.extend(source.run_format(raw, vec![], None, None, 512).values);
            hub.run_format(raw, vec![], None, None, 512);
            if source.source_snapshot().captures == 0 {
                break;
            }
        }
        assert_eq!(
            releases.iter().filter(|(_, event)| event.release()).count(),
            if terminal == 120 { 64 } else { 0 }
        );
        assert_eq!(source.source_snapshot().note_off_owed, 0);
        assert_eq!(source.source_snapshot().held, 0);
        assert_eq!(source.source_snapshot().faults, 0);
        assert_eq!(source.source_snapshot().captures, 0);
    }
}

#[test]
fn production_dense_sixty_four_onsets_publish_one_complete_cohort() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    source.run_format(
        1536,
        (0..64).map(|index| note(index, 0, index as i16, 0, true)).collect(),
        None,
        None,
        512,
    );
    hub.run_format(1536, vec![], None, None, 512);
    let mut onsets = Vec::new();
    let mut tuning = 0;
    for raw in (2048..8192).step_by(512) {
        let output = source.run_format(raw, vec![], None, None, 512);
        onsets.extend(output.values.iter().filter_map(|(offset, event)| {
            event.attack().map(|address| (raw + i64::from(*offset), address.0))
        }));
        tuning += output
            .values
            .iter()
            .filter(|(_, event)| matches!(event, Event::Expression { kind: 2, .. }))
            .count();
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        onsets.len(),
        64,
        "all original requests progress while their predecessors stay held"
    );
    assert_eq!(tuning, 64);
    assert!(onsets.iter().all(|(actual, _)| *actual == onsets[0].0));
    assert_eq!(onsets.iter().map(|(_, id)| *id).collect::<Vec<_>>(), (0..64).collect::<Vec<_>>());
    assert_eq!(source.latency(), 512);
    assert_eq!(source.shared().extra_delay.load(Ordering::Acquire), (onsets[0].0 - 2048) as u64);
    assert_eq!(source.source_snapshot().faults, 0);
    source.run_format(
        8192,
        (0..64).map(|index| note(index, 0, index as i16, 0, false)).collect(),
        None,
        None,
        512,
    );
    hub.run_format(8192, vec![], None, None, 512);
    for raw in (8704..12288).step_by(512) {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(source.source_snapshot().held, 0);
}

#[test]
fn production_native_gui_off_classifies_birth_without_host_echo_or_retry_reapplication() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let wrapper = unsafe {
        &*((*source.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    let (context, param) = wrapper.test_gui_context(setup::PARTICIPATING);
    let mut actual = Vec::new();
    let mut run = |raw, inputs, reject| {
        let sink = source.run_format(raw, inputs, reject, None, 512);
        actual.extend(
            sink.values.into_iter().map(|(offset, event)| (raw + i64::from(offset), event)),
        );
    };
    run(1536, vec![note(69, 0, 59, 0, true)], None);
    hub.run_format(1536, vec![], None, None, 512);
    run(2048, vec![note(70, 0, 61, 0, true)], None);
    // Read each classification in its own capturing callback: an envelope is
    // released as soon as its copy is sent and its own work has settled.
    assert!(inspect_source(&source, |source| source.test_capture(2).unwrap().adaptive));
    hub.run_format(2048, vec![], None, None, 512);
    unsafe {
        context.raw_begin_set_parameter(param);
        context.raw_set_parameter_normalized(param, 0.0);
        context.raw_end_set_parameter(param);
    }
    run(
        2560,
        vec![note(71, 0, 62, 0, true), source.participation(true, 1), note(72, 0, 64, 2, true)],
        Some(CLAP_EVENT_PARAM_VALUE),
    );
    assert!(!inspect_source(&source, |source| source.test_capture(4).unwrap().adaptive));
    assert!(inspect_source(&source, |source| source.test_capture(6).unwrap().adaptive));
    hub.run_format(2560, vec![], None, None, 512);
    // No host echo follows the native Set. Accepting its old notification now
    // cannot reinsert Off ahead of D or overwrite the newer host On value.
    run(3072, vec![note(73, 0, 65, 0, true)], None);
    assert!(inspect_source(&source, |source| source.test_capture(7).unwrap().adaptive));
    assert!(inspect_source(&source, |source| source.participating));
    assert!(wrapper.test_inspect_plugin(|plugin| plugin.params.participating.value()));
    hub.run_format(3072, vec![], None, None, 512);
    for raw in [3584, 4096] {
        run(raw, if raw == 3584 { vec![expression(69, 0.25, 7)] } else { vec![] }, None);
        hub.run_format(raw, vec![], None, None, 512);
    }
    run(
        4608,
        vec![
            note(69, 0, 59, 0, false),
            note(70, 0, 61, 0, false),
            note(71, 0, 62, 0, false),
            note(72, 0, 64, 0, false),
            note(73, 0, 65, 0, false),
        ],
        None,
    );
    hub.run_format(4608, vec![], None, None, 512);
    for raw in (5120..9216).step_by(512) {
        run(raw, vec![], None);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let attack_ids: Vec<_> =
        actual.iter().filter_map(|(_, event)| event.attack().map(|(id, ..)| id)).collect();
    assert_eq!(
        attack_ids,
        [69, 72, 73],
        "either toggle direction cancels the attacks standing before it: 70 was pending and \
         71 was captured into the Off mode the second marker left"
    );
    let tuning = |id| {
        actual
            .iter()
            .filter_map(|(_, event)| match event {
                Event::Expression { kind: 2, id: found, value, .. } if *found == id => Some(*value),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        tuning(69),
        [-0.11731262],
        "the note that sounded before the toggle is tuned once, and the toggle ends it rather \
         than carrying it into the new mode"
    );
    assert!(
        actual.iter().any(|(sample, event)| *sample == 2560
            && matches!(event, Event::Note { kind: 2, id: 69, .. })),
        "the wire termination goes out at the marker, before the ownership is forgotten"
    );
    assert!(
        tuning(70).is_empty() && tuning(71).is_empty(),
        "a cancelled attack is never tuned, and an Off onset states no tuning of its own"
    );
    assert!(tuning(72)[0] != 0.0 && tuning(73)[0] != 0.0);
    assert_eq!(source.source_snapshot().held, 0);
    assert_eq!(source.source_snapshot().faults, 0);
    drop(context);
}

#[test]
fn production_three_sources_form_one_sequential_assignment_chain() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let calibration = Calibration { offset: 0 };
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, calibration);
    hub.activate_format(44100.0, 512);
    let mut sources: [Device; 3] = std::array::from_fn(|_| {
        let mut source = Device::new(true);
        source.configure_format(uuid, true, calibration);
        source.activate_format(44100.0, 512);
        assert_eq!(source.latency(), 512);
        assert_eq!(source._stats.restarts.load(Ordering::Relaxed), 0);
        assert_eq!(source._stats.latency_changes.load(Ordering::Relaxed), 1);
        source
    });
    for raw in [0, 512, 1024] {
        for source in &sources {
            source.run_format(raw, vec![], None, None, 512);
        }
        hub.run_format(raw, vec![], None, None, 512);
    }
    for (index, source) in sources.iter().enumerate().rev() {
        assert!(source
            .run_format(
                1536,
                vec![note(100 + index as i32, 0, 60 + index as i16 * 2, 0, true)],
                None,
                None,
                512
            )
            .values
            .is_empty());
    }
    hub.run_format(1536, vec![], None, None, 512);
    for (index, source) in sources.iter().enumerate() {
        let output = source.run_format(2048, vec![], None, None, 512);
        assert_eq!(
            output.values.len(),
            2,
            "source {index}: {:?}, {:?}, {:?}",
            output.values,
            inspect_hub(&hub, |hub| hub.test_input_row(index)),
            inspect_hub(&hub, |hub| hub.test_input_sequence_progress())
        );
        assert!(output.values[0].1.attack().is_some());
        assert_eq!(output.values[0].0, 0);
        assert_eq!(output.values[1].0, 0);
        let Event::Expression { value, id, channel, key, .. } = output.values[1].1 else {
            panic!("missing tuning")
        };
        assert_eq!((id, channel, key), (100 + index as i32, 0, 60 + index as i16 * 2));
        assert_eq!(value, [0.0; 3][index]);
    }
    hub.run_format(2048, vec![], None, None, 512);
    // Explicit physical release keeps the retirement fixture independent of
    // an unobserved host termination guarantee during destruction.
    for (index, source) in sources.iter_mut().enumerate() {
        source.run_format(
            2560,
            vec![note(100 + index as i32, 0, 60 + index as i16 * 2, 0, false)],
            None,
            None,
            512,
        );
    }
    hub.run_format(2560, vec![], None, None, 512);
    for source in &sources {
        source.run_format(3072, vec![], None, None, 512);
    }
    hub.run_format(3072, vec![], None, None, 512);
}

#[test]
fn production_missing_assignment_retains_one_late_onset_and_fixed_latency() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let calibration = Calibration { offset: 0 };
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, calibration);
    hub.activate_format(44100.0, 512);
    let mut source = Device::new(true);
    source.configure_format(uuid, true, calibration);
    source.activate_format(44100.0, 512);
    for raw in [0, 512, 1024] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert!(source
        .run_format(1536, vec![note(7, 0, 60, 0, true)], None, None, 512)
        .values
        .is_empty());
    // The real Hub is not called until the entire target callback has passed.
    // Original input and its output obligation must survive this missing reply.
    assert!(source.run_format(2048, vec![], None, None, 512).values.is_empty());
    assert_eq!(source.source_snapshot().faults, 0);
    assert_ne!(source.shared().status.load(Ordering::Acquire) & source::TIMING_FAILURE, 0);
    hub.run_format(1536, vec![], None, None, 512);
    let late = source.run_format(2560, vec![], None, None, 512);
    assert_eq!(late.values.len(), 2);
    assert!(late.values[0].1.attack().is_some());
    assert_eq!(late.values[0].0, 0);
    assert_eq!(source.shared().extra_delay.load(Ordering::Acquire), 512);
    assert_eq!(source.latency(), 512);
    assert_eq!(source._stats.restarts.load(Ordering::Relaxed), 0);
    assert_eq!(source._stats.latency_changes.load(Ordering::Relaxed), 1);
    hub.run_format(2048, vec![], None, None, 512);
    assert_ne!(hub.shared().status.load(Ordering::Acquire) & source::TIMING_FAILURE, 0);
    assert!(source.run_format(3072, vec![], None, None, 512).values.is_empty());
    assert_ne!(source.shared().status.load(Ordering::Acquire) & source::TIMING_FAILURE, 0);
    hub.run_format(2560, vec![], None, None, 512);
    source.run_format(3584, vec![note(7, 0, 60, 0, false)], None, None, 512);
    hub.run_format(3072, vec![], None, None, 512);
    source.run_format(4096, vec![], None, None, 512);
    hub.run_format(3584, vec![], None, None, 512);
    source.run_format(4608, vec![], None, None, 512);
    hub.run_format(4096, vec![], None, None, 512);
}

#[test]
fn the_deadline_counter_counts_each_note_once_with_worst_lateness() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    // Two onsets in one callback, 256 samples apart. The Hub runs a callback
    // behind, so both wait past input+D and both emit in the same later
    // callback: the first one 512 samples late, the second only 256.
    assert!(source
        .run_format(1536, vec![note(7, 0, 60, 0, true), note(8, 0, 62, 256, true)], None, None, 512)
        .values
        .is_empty());
    // Both deadlines pass here, unassigned, and each note is seen late a
    // second time when it finally emits below. Counting arrivals rather than
    // notes would double this.
    assert!(source.run_format(2048, vec![], None, None, 512).values.is_empty());
    hub.run_format(1536, vec![], None, None, 512);
    let late = source.run_format(2560, vec![], None, None, 512);
    assert_eq!(late.values.iter().filter(|(_, event)| event.attack().is_some()).count(), 2);
    assert_eq!(source.shared().deadline_misses.load(Ordering::Acquire), 2);
    assert_eq!(
        source.shared().extra_delay.load(Ordering::Acquire),
        512,
        "the worst of 512 and 256, not the last measured and not their mean"
    );
    hub.run_format(2048, vec![], None, None, 512);
    assert_eq!(hub.shared().deadline_misses.load(Ordering::Acquire), 2);
    assert_eq!(hub.shared().extra_delay.load(Ordering::Acquire), 512);
}

#[test]
fn production_late_onset_keeps_its_duration_and_shifts_only_its_own_release() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    // The Hub runs a callback behind, so this onset cannot emit at input+D and
    // waits with rule one. It lands 512 samples late.
    assert!(source
        .run_format(1536, vec![note(7, 0, 60, 0, true)], None, None, 512)
        .values
        .is_empty());
    assert!(source.run_format(2048, vec![], None, None, 512).values.is_empty());
    hub.run_format(1536, vec![], None, None, 512);
    let late = source.run_format(2560, vec![], None, None, 512);
    assert!(late.values[0].1.attack().is_some());
    assert_eq!(late.values[0].0, 0, "one whole callback past input+D");
    hub.run_format(2048, vec![], None, None, 512);
    assert!(source.run_format(3072, vec![], None, None, 512).values.is_empty());
    hub.run_format(2560, vec![], None, None, 512);
    // The note's own release carries the same extra shift, so the sounding
    // duration is the played duration. Two unrelated events sit behind it in
    // the same callback and neither inherits its lateness: a shared channel
    // control, which leaves through `schedule_channels`, and a raw MIDI clock,
    // which has no channel and only `schedule_pending` can carry.
    assert!(source
        .run_format(
            3584,
            vec![note(7, 0, 60, 0, false), raw_midi([0xb0, 11, 90], 0), raw_midi([0xf8, 0, 0], 1)],
            None,
            None,
            512
        )
        .values
        .is_empty());
    hub.run_format(3072, vec![], None, None, 512);
    let control = source.run_format(4096, vec![], None, None, 512);
    assert_eq!(
        control.values.len(),
        2,
        "neither is held to the note's shift: {:?}",
        control.values
    );
    assert!(control.values.iter().any(
        |(time, event)| *time == 0 && matches!(event, Event::Midi { data: [0xb0, 11, 90], .. })
    ));
    assert!(
        control
            .values
            .iter()
            .any(|(time, event)| *time == 1
                && matches!(event, Event::Midi { data: [0xf8, 0, 0], .. }))
    );
    hub.run_format(3584, vec![], None, None, 512);
    let release = source.run_format(4608, vec![], None, None, 512);
    assert_eq!(release.values.len(), 1, "{:?}", release.values);
    assert_eq!(release.values[0].0, 0);
    assert!(release.values[0].1.release());
    // Input 1536..3584 played; output 2560..4608 sounded.
    assert_eq!(4608 - 2560, 3584 - 1536);
    assert_eq!(source.source_snapshot().faults, 0);
    hub.run_format(4096, vec![], None, None, 512);
}

#[test]
fn an_unpaired_tune_retires_its_own_accepted_output_rather_than_filling_its_journal() {
    let _scope = crate::test_scope::enter();
    let mut source = Device::new(true);
    source.activate_format(44100.0, 512);
    // No Hub exists at all, so nothing can ever send `OutputRetained` and
    // `transfer` never looks at the journal. Forward more raw controls than
    // the 4,096-entry journal holds: with #718 open this latches
    // STORAGE_FAULT and the journal never empties.
    let mut emitted = 0;
    for block in 0..12i64 {
        let controls =
            (0..400u32).map(|index| raw_midi([0xb0, 1, 64], index % 512)).collect::<Vec<_>>();
        emitted += source.run_format(block * 512, controls, None, None, 512).values.len();
    }
    assert!(
        emitted > super::protocol::OUTCOME_JOURNAL,
        "the fixture reaches past one journal: {emitted}"
    );
    let settled = source.source_snapshot();
    assert_eq!(settled.faults, 0, "an unpaired Tune's own output is not an overflow");
    assert_eq!(settled.journal, 0, "it acknowledges what only it can acknowledge");
}

#[test]
fn an_unpaired_hub_allocates_nothing_for_tuning_and_pairing_allocates_one_row() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let calibration = Calibration { offset: 0 };
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, calibration);
    hub.activate_format(44100.0, 512);
    for raw in [0, 512] {
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_plan_ledger_bytes()),
        0,
        "a Harmonigraph with no paired Tune allocates no plan ledger"
    );
    assert!(
        inspect_hub(&hub, |hub| hub.offer.as_ref().unwrap().bank.is_none()),
        "nor any of the sixteen ring triples"
    );
    let mut source = Device::new(true);
    source.configure_format(uuid, true, calibration);
    source.activate_format(44100.0, 512);
    for raw in [1024, 1536] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_plan_ledger_bytes()),
        hub::Hub::test_plan_row_bytes(),
        "pairing one Tune allocates exactly that Tune's row"
    );
    assert!(inspect_hub(&hub, |hub| hub.offer.as_ref().unwrap().bank.is_some()));
}

#[test]
fn production_same_key_retrigger_chokes_its_predecessor_at_the_moment_of_emission() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    // Rule two: a replacement onset on the same channel and key force-releases
    // the note it displaces at the moment the replacement is EMITTED -- not at
    // its input, not on the predecessor's schedule, and not left to the
    // receiving instrument. Both halves of that are exercised here.
    source.run_format(1536, vec![note(1, 0, 60, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    let sounding = source.run_format(2048, vec![], None, None, 512);
    assert!(matches!(sounding.values[0].1, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 1, .. }));
    hub.run_format(2048, vec![], None, None, 512);
    assert!(inspect_hub(&hub, |hub| hub.test_voice(0, 1)).is_some());
    // First ordering: the Hub has not answered the replacement by input+D. The
    // key keeps sounding rather than going silent between the two.
    source.run_format(2560, vec![note(2, 0, 60, 0, true)], None, None, 512);
    assert!(
        source.run_format(3072, vec![], None, None, 512).values.is_empty(),
        "an unanswered replacement does not get to choke its predecessor"
    );
    hub.run_format(2560, vec![], None, None, 512);
    let retrigger = source.run_format(3584, vec![], None, None, 512);
    assert_eq!(retrigger.values.len(), 3, "{:?}", retrigger.values);
    assert!(retrigger.values.iter().all(|(time, _)| *time == 0));
    assert!(
        matches!(
            retrigger.values[0].1,
            Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 1, key: 60, channel: 0, .. }
        ),
        "the predecessor's choke precedes its replacement: {:?}",
        retrigger.values
    );
    assert!(matches!(
        retrigger.values[1].1,
        Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 2, key: 60, channel: 0, .. }
    ));
    hub.run_format(3072, vec![], None, None, 512);
    hub.run_format(3584, vec![], None, None, 512);
    assert!(inspect_hub(&hub, |hub| hub.test_voice(0, 1)).is_none(), "the displaced note ended");
    assert!(inspect_hub(&hub, |hub| hub.test_voice(0, 2)).is_some());
    // Second ordering: note 2 sounded a whole callback late and carries that
    // shift, and its own replacement is answered in time. The choke rides the
    // replacement's schedule, so an otherwise ready replacement is not delayed
    // by how late the note it displaces was.
    source.run_format(4096, vec![note(3, 0, 60, 0, true)], None, None, 512);
    hub.run_format(4096, vec![], None, None, 512);
    let replaced = source.run_format(4608, vec![], None, None, 512);
    assert_eq!(replaced.values.len(), 3, "at the replacement's input+D: {:?}", replaced.values);
    assert!(replaced.values.iter().all(|(time, _)| *time == 0));
    assert!(matches!(
        replaced.values[0].1,
        Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 2, key: 60, .. }
    ));
    assert!(matches!(
        replaced.values[1].1,
        Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 3, key: 60, .. }
    ));
    hub.run_format(4608, vec![], None, None, 512);
    source.run_format(5120, vec![note(3, 0, 60, 0, false)], None, None, 512);
    hub.run_format(5120, vec![], None, None, 512);
    assert!(source.run_format(5632, vec![], None, None, 512).values[0].1.release());
    for raw in [6144, 6656] {
        hub.run_format(raw - 512, vec![], None, None, 512);
        source.run_format(raw, vec![], None, None, 512);
    }
    let settled = source.source_snapshot();
    assert_eq!((settled.held, settled.lives, settled.faults), (0, 0, 0), "one key, three lives");
}

#[test]
fn production_a_lifetime_born_and_ended_at_one_sample_leaves_no_tuning_context() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    // Release-first ordering applies a lifetime's terminal before its own
    // onset whenever both fall on one sample, so no later record can remove
    // the voice that onset would insert. Two gestures reach that: a key struck
    // and lifted at one sample, and a same-key onset repeated at one sample,
    // whose predecessor's choke shares the replacement's sample.
    let mut input = vec![note(1, 0, 60, 0, true), note(1, 0, 60, 0, false)];
    for id in 2..10 {
        input.push(note(id, 0, 64, 0, true));
    }
    source.run_format(1536, input, None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context()),
        vec![(1, 9)],
        "only the one note still standing at the end of the sample"
    );
    // The survivor's own release clears the last of it.
    source.run_format(2048, vec![note(9, 0, 64, 0, false)], None, None, 512);
    hub.run_format(2048, vec![], None, None, 512);
    assert_eq!(inspect_hub(&hub, |hub| hub.test_context()), vec![], "nothing outlives the phrase");
    for raw in [2560, 3072, 3584] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let settled = source.source_snapshot();
    assert_eq!((settled.held, settled.lives, settled.faults), (0, 0, 0), "{settled:?}");
    // A transport Stop is the third such ending, and the only one that is not
    // addressed to a lifetime. It sorts into the release-first half like any
    // other, so the onset it ends is still ahead of it in the pass.
    let stopped = {
        let Input::Transport(mut value) = transport(0, 120.0) else { unreachable!() };
        value.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        Input::Transport(value)
    };
    source.run_format(4096, vec![], None, None, 512);
    hub.run_format(
        4096,
        vec![transport(0, 120.0), note(1, 0, 60, 0, true), stopped],
        None,
        None,
        512,
    );
    for raw in [4608, 5120] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context()),
        vec![],
        "the Stop's own sample carries the onset it ends"
    );
}

#[test]
fn production_stop_cancels_the_pending_attack_and_releases_the_forwarded_voice() {
    let _scope = crate::test_scope::enter();
    let stopped = || {
        let Input::Transport(mut value) = transport(0, 120.0) else { unreachable!() };
        value.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        Input::Transport(value)
    };
    let (hub, source) = production_pair();
    // Four notes, each ending by a different route: note 1 is assigned and
    // forwarded downstream, note 3 is assigned but one callback short of
    // sounding, note 2 is still waiting for its assignment, and note 4 is held
    // on the Hub's own DIRECT input, which is assigned nothing and forwarded
    // whole.
    source.run_format(1536, vec![transport(0, 120.0), note(1, 0, 60, 0, true)], None, None, 512);
    hub.run_format(1536, vec![transport(0, 120.0), note(4, 0, 72, 0, true)], None, None, 512);
    let sounding = source.run_format(2048, vec![note(3, 0, 67, 0, true)], None, None, 512);
    assert!(matches!(sounding.values[0].1, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 1, .. }));
    hub.run_format(2048, vec![], None, None, 512);
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context()),
        vec![(1, 1), (0, 1), (1, 2)],
        "note 3 is assigned before the Stop, and note 2 has not been played yet"
    );
    let stop = source.run_format(2560, vec![note(2, 0, 64, 0, true), stopped()], None, None, 512);
    assert_eq!(source.source_snapshot().local_pending, 2, "neither note 2 nor note 3 sounded");
    assert!(
        stop.values.iter().any(|(time, event)| *time == 0
            && matches!(event, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 1, key: 60, .. })),
        "the forwarded voice is terminated, not merely forgotten: {:?}",
        stop.values
    );
    for controller in [64, 66, 69] {
        assert!(stop.values.iter().any(|(_, event)| matches!(
            event,
            Event::Midi { data: [0xb0, value, 0], .. } if *value == controller
        )));
    }
    assert!(!stop.values.iter().any(|(_, event)| event.attack().is_some()));
    // The same Stop reaches the Hub's own transport, and note 4 leaves by the
    // one route DIRECT has: its emergency release goes straight to the host,
    // never becoming an `OutputDelta`, and it has no plan to cancel.
    let direct = hub.run_format(2560, vec![stopped()], None, None, 512);
    assert!(
        direct.values.iter().any(|(_, event)| matches!(
            event,
            Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 4, key: 72, .. }
        )),
        "the DIRECT voice is terminated too: {:?}",
        direct.values
    );
    // The Hub answers note 2 only after the Stop. Neither that assignment nor
    // note 3's, which arrived before it, may resurrect a canceled attack.
    for raw in [3072, 3584, 4096, 4608] {
        assert!(source.run_format(raw, vec![], None, None, 512).values.is_empty());
        hub.run_format(raw, vec![], None, None, 512);
    }
    let settled = source.source_snapshot();
    assert_eq!((settled.held, settled.lives, settled.faults), (0, 0, 0), "{settled:?}");
    for lifetime in 1..=3 {
        assert!(inspect_hub(&hub, |hub| hub.test_voice(0, lifetime)).is_none());
    }
    // The factual row is not the whole of it: the emergency release reaches the
    // Hub as accepted output rather than as an addressed Terminal record, and
    // the cancellation only marks a plan, so both notes can survive in the
    // table the policy actually scores against. DIRECT reaches neither of those
    // two routes at all, so its Stop capture is what has to end note 4 here.
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context()),
        vec![],
        "neither the released voice nor the canceled attack stays in tuning context"
    );
}

#[test]
fn production_a_congested_callback_chokes_a_predecessor_only_with_its_replacement() {
    let _scope = crate::test_scope::enter();
    let clocks = || (0..510).map(|_| raw_midi([0xf8, 0, 0], 0)).collect::<Vec<_>>();
    let musical = |sink: &Sink| {
        sink.values
            .iter()
            .filter(|(_, event)| !matches!(event, Event::Midi { data: [0xf8, ..], .. }))
            .copied()
            .collect::<Vec<_>>()
    };
    let (hub, source) = production_pair();
    // Note 1 sounds first, so the retrigger below owes it a forced release.
    source.run_format(1536, vec![note(1, 0, 60, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    assert_eq!(source.run_format(2048, vec![], None, None, 512).values.len(), 2);
    hub.run_format(2048, vec![], None, None, 512);
    let mut input = vec![note(2, 0, 60, 0, true)];
    input.extend(clocks());
    source.run_format(2560, input, None, None, 512);
    hub.run_format(2560, vec![], None, None, 512);
    // Every callback from here carries another 510 raw MIDI clocks, so
    // whichever one the Hub's answer lets the retrigger take is saturated:
    // its choke, onset and tuning are three events against the two those 510
    // leave of the callback's 512. Which callback that is depends on how long
    // the Hub takes with the copied input, and is not what this measures.
    let mut raw = 3072;
    let congested = loop {
        assert!(raw < 8192, "the retrigger never emitted");
        let out = source.run_format(raw, clocks(), None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        raw += 512;
        if !musical(&out).is_empty() {
            break out;
        }
    };
    // The choke stages first, out of the indexed release scan; the
    // replacement's own onset stages a host round trip later, out of
    // `complete`, with the whole raw stream having had its turn at the same
    // credits in between. Rule two: without room for the onset the predecessor
    // is not choked at all, so the two either ride together or neither goes.
    assert!(
        matches!(
            musical(&congested)[..],
            [
                (0, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 1, key: 60, .. }),
                (0, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 2, key: 60, .. }),
                (0, Event::Expression { kind: 2, id: 2, .. }),
            ]
        ),
        "the choke rides out with the replacement it is for: {:?}",
        musical(&congested)
    );
    assert_eq!(congested.values.len(), 512, "the callback's credits are spent, not exceeded");
    // The Hub's own DIRECT input reaches the same staging with no round trip in
    // the way and a one-event onset group, so 511 clocks are what saturate its
    // callback rather than 510. The rule and the reservation are the same.
    source.run_format(raw, vec![], None, None, 512);
    hub.run_format(raw, vec![note(3, 0, 72, 0, true)], None, None, 512);
    raw += 512;
    source.run_format(raw, vec![], None, None, 512);
    let mut retrigger = vec![note(4, 0, 72, 0, true)];
    retrigger.extend((0..511).map(|_| raw_midi([0xf8, 0, 0], 0)));
    let saturated = hub.run_format(raw, retrigger, None, None, 512);
    assert!(
        matches!(
            musical(&saturated)[..],
            [
                (0, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 3, key: 72, .. }),
                (0, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 4, key: 72, .. }),
            ]
        ),
        "DIRECT pairs them the same way: {:?}",
        musical(&saturated)
    );
    assert_eq!(saturated.values.len(), 512, "and spends its credits the same way");
    raw += 512;
    source.run_format(raw, vec![note(2, 0, 60, 0, false)], None, None, 512);
    hub.run_format(raw, vec![note(4, 0, 72, 0, false)], None, None, 512);
    for _ in 0..8 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let settled = source.source_snapshot();
    assert_eq!((settled.held, settled.lives, settled.faults), (0, 0, 0), "{settled:?}");
}

#[test]
fn production_two_replacements_in_one_callback_each_keep_the_output_their_own_onset_needs() {
    let _scope = crate::test_scope::enter();
    let clocks = |count| (0..count).map(|_| raw_midi([0xf8, 0, 0], 0)).collect::<Vec<_>>();
    let musical = |sink: &Sink| {
        sink.values
            .iter()
            .filter(|(_, event)| !matches!(event, Event::Midi { data: [0xf8, ..], .. }))
            .copied()
            .collect::<Vec<_>>()
    };
    let (hub, source) = production_pair();
    // Two sounding notes, so the two retriggers below each owe a forced release.
    source.run_format(
        1536,
        vec![note(1, 0, 60, 0, true), note(2, 0, 72, 0, true)],
        None,
        None,
        512,
    );
    hub.run_format(1536, vec![], None, None, 512);
    assert_eq!(source.run_format(2048, vec![], None, None, 512).values.len(), 4);
    hub.run_format(2048, vec![], None, None, 512);
    let mut input = vec![note(3, 0, 60, 0, true), note(4, 0, 72, 0, true)];
    input.extend(clocks(510));
    source.run_format(2560, input, None, None, 512);
    hub.run_format(2560, vec![], None, None, 512);
    // Both retriggers are answered together, so both land in one callback that
    // another 510 raw MIDI clocks are already claiming. Six musical events
    // against 512 credits is the whole point: two chokes, two onsets and two
    // tunings, and nothing may come between a choke and the onset it is for.
    // Which callback the Hub's answer lets them take is not what this measures.
    let mut raw = 3072;
    let congested = loop {
        assert!(raw < 8192, "the retriggers never emitted");
        let out = source.run_format(raw, clocks(510), None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        raw += 512;
        if !musical(&out).is_empty() {
            break out;
        }
    };
    // Rule two is not divisible, and it is not divisible per replacement
    // either: one pair's admission cannot spend what the other pair's onset
    // still needs, so the credits that give way are the ordinary stream's.
    assert!(
        matches!(
            musical(&congested)[..],
            [
                (0, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 1, key: 60, .. }),
                (0, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 3, key: 60, .. }),
                (0, Event::Expression { kind: 2, id: 3, .. }),
                (0, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 2, key: 72, .. }),
                (0, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 4, key: 72, .. }),
                (0, Event::Expression { kind: 2, id: 4, .. }),
            ]
        ),
        "each choke rides out with its own replacement: {:?}",
        musical(&congested)
    );
    assert_eq!(congested.values.len(), 512, "the callback's credits are spent, not exceeded");
    // The Hub's own DIRECT input reaches the same staging with one-event onset
    // groups, so its two pairs are four events rather than six.
    source.run_format(raw, vec![], None, None, 512);
    hub.run_format(raw, vec![note(5, 0, 48, 0, true), note(6, 0, 55, 0, true)], None, None, 512);
    raw += 512;
    source.run_format(raw, vec![], None, None, 512);
    let mut retrigger = vec![note(7, 0, 48, 0, true), note(8, 0, 55, 0, true)];
    retrigger.extend(clocks(510));
    let saturated = hub.run_format(raw, retrigger, None, None, 512);
    assert!(
        matches!(
            musical(&saturated)[..],
            [
                (0, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 5, key: 48, .. }),
                (0, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 7, key: 48, .. }),
                (0, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 6, key: 55, .. }),
                (0, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 8, key: 55, .. }),
            ]
        ),
        "DIRECT keeps both pairs whole the same way: {:?}",
        musical(&saturated)
    );
    assert_eq!(saturated.values.len(), 512, "and spends its credits the same way");
    raw += 512;
    source.run_format(
        raw,
        vec![note(3, 0, 60, 0, false), note(4, 0, 72, 0, false)],
        None,
        None,
        512,
    );
    hub.run_format(raw, vec![note(7, 0, 48, 0, false), note(8, 0, 55, 0, false)], None, None, 512);
    for _ in 0..8 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let settled = source.source_snapshot();
    assert_eq!((settled.held, settled.lives, settled.faults), (0, 0, 0), "{settled:?}");
}

#[test]
fn production_all_sound_off_owes_each_voice_a_physical_note_off_and_all_notes_off_does_not() {
    let _scope = crate::test_scope::enter();
    for cc in [120u8, 123] {
        let (hub, source) = production_pair();
        source.run_format(
            1536,
            vec![note(1, 0, 60, 0, true), note(2, 0, 64, 0, true)],
            None,
            None,
            512,
        );
        hub.run_format(1536, vec![], None, None, 512);
        assert_eq!(source.run_format(2048, vec![], None, None, 512).values.len(), 4);
        hub.run_format(2048, vec![], None, None, 512);
        source.run_format(2560, vec![raw_midi([0xb0, cc, 0], 0)], None, None, 512);
        hub.run_format(2560, vec![], None, None, 512);
        // Both terminations end both notes for the Hub. Only All Sound Off
        // leaves each voice a physical Note-Off still owed downstream.
        let terminal = source.run_format(3072, vec![], None, None, 512);
        assert_eq!(terminal.values.len(), 1, "CC{cc} has one wire effect: {:?}", terminal.values);
        assert!(
            matches!(terminal.values[0].1, Event::Midi { data: [0xb0, value, 0], .. } if value == cc)
        );
        hub.run_format(3072, vec![], None, None, 512);
        assert_eq!(source.source_snapshot().note_off_owed, if cc == 120 { 2 } else { 0 });
        assert!((1..=2).all(|life| inspect_hub(&hub, |hub| hub.test_voice(0, life)).is_none()));
        // A replacement on one of those keys pays that voice's owed Off first,
        // as a Note-Off rather than the choke an ordinary retrigger sends.
        source.run_format(3584, vec![note(3, 0, 60, 0, true)], None, None, 512);
        hub.run_format(3584, vec![], None, None, 512);
        let retrigger = source.run_format(4096, vec![], None, None, 512);
        assert_eq!(retrigger.values.len(), 2 + usize::from(cc == 120), "{:?}", retrigger.values);
        if cc == 120 {
            assert!(matches!(
                retrigger.values[0].1,
                Event::Note { kind: CLAP_EVENT_NOTE_OFF, id: 1, key: 60, channel: 0, .. }
            ));
        }
        assert!(retrigger.values[usize::from(cc == 120)].1.attack().is_some());
        hub.run_format(4096, vec![], None, None, 512);
        // The untouched key still owes its Off, and its own release pays it.
        source.run_format(
            4608,
            vec![note(3, 0, 60, 0, false), note(2, 0, 64, 0, false)],
            None,
            None,
            512,
        );
        hub.run_format(4608, vec![], None, None, 512);
        let last = source.run_format(5120, vec![], None, None, 512);
        assert_eq!(
            last.values.iter().filter(|(_, event)| event.release()).count(),
            1 + usize::from(cc == 120),
            "{:?}",
            last.values
        );
        hub.run_format(5120, vec![], None, None, 512);
        for raw in [5632, 6144] {
            source.run_format(raw, vec![], None, None, 512);
            hub.run_format(raw, vec![], None, None, 512);
        }
        let settled = source.source_snapshot();
        assert_eq!((settled.held, settled.note_off_owed, settled.faults), (0, 0, 0), "{settled:?}");
    }
}

#[test]
fn production_unaddressed_control_behind_a_late_attack_keeps_its_own_schedule() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    // One callback carries an onset and, behind it, an unaddressed raw MIDI
    // clock. The Hub is not run, so no assignment can come back and the onset
    // waits — rule one holds it with its own note. The clock is addressed to
    // no note, so it keeps its own input+D schedule rather than waiting
    // behind the oldest pending attack.
    assert!(source
        .run_format(1536, vec![note(7, 0, 60, 0, true), raw_midi([0xf8, 0, 0], 1)], None, None, 512)
        .values
        .is_empty());
    let late = source.run_format(2048, vec![], None, None, 512);
    assert_eq!(late.values.len(), 1, "only the clock, and it is not held back");
    assert_eq!(late.values[0].0, 1, "at its own input+D, not the attack's");
    assert!(matches!(late.values[0].1, Event::Midi { data: [0xf8, 0, 0], .. }));
    assert_eq!(source.source_snapshot().faults, 0);
    // The attack keeps its place in its own order once its assignment lands.
    hub.run_format(1536, vec![], None, None, 512);
    let attack = source.run_format(2560, vec![], None, None, 512);
    assert!(attack.values[0].1.attack().is_some());
}

#[test]
fn production_a_broadcast_release_waiting_on_one_target_still_lets_the_clock_through() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    // Note 1 sounds. Note 2 is captured a callback later and left unanswered,
    // so it is still unsounded when one release addresses them both.
    source.run_format(1536, vec![note(1, 0, 60, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    assert!(source.run_format(2048, vec![note(2, 0, 64, 0, true)], None, None, 512).values[0]
        .1
        .attack()
        .is_some());
    // A release addressed to every note on the channel owns a child per target
    // and no life of its own, so the refusal that holds it belongs to one child
    // and not to the envelope.
    assert!(source
        .run_format(2560, vec![note(-1, 0, -1, 0, false),], None, None, 512)
        .values
        .is_empty());
    // Note 1's share leaves here. Note 2's is still owed, and a raw MIDI clock
    // captured now falls behind an envelope that can only refuse from here on.
    let first = source.run_format(3072, vec![raw_midi([0xf8, 0, 0], 1)], None, None, 512);
    assert_eq!(first.values.len(), 1, "{:?}", first.values);
    assert!(matches!(first.values[0], (0, Event::Note { kind: CLAP_EVENT_NOTE_OFF, key: 60, .. })));
    let clock = source.run_format(3584, vec![], None, None, 512);
    assert_eq!(
        clock.values.len(),
        1,
        "the clock does not inherit note 2's wait: {:?}",
        clock.values
    );
    assert_eq!(clock.values[0].0, 1, "at its own input+D");
    assert!(matches!(clock.values[0].1, Event::Midi { data: [0xf8, 0, 0], .. }));
    // Note 2's share of that release is still owed, and settles once the Hub
    // answers it.
    for raw in [4096, 4608, 5120, 5632, 6144, 6656] {
        hub.run_format(raw - 2048, vec![], None, None, 512);
        source.run_format(raw, vec![], None, None, 512);
    }
    let settled = source.source_snapshot();
    assert_eq!((settled.held, settled.lives, settled.faults), (0, 0, 0), "{settled:?}");
}

#[test]
fn production_d512_boundaries_keep_canonical_pitch_across_callback_permutations() {
    let _scope = crate::test_scope::enter();
    let orders = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    for (offset, onset_offset) in [(0, 0), (0, 511), (64, 447), (64, 448), (64, 511)] {
        for capture_order in orders {
            for output_order in orders {
                for hub_first in [false, true] {
                    let uuid = SavedUuid::default();
                    let calibration = Calibration { offset: 0 };
                    let mut hub = Device::new(false);
                    hub.configure_format(uuid, true, calibration);
                    hub.activate_format(44100.0, 512);
                    let sources: [Device; 3] = std::array::from_fn(|_| {
                        let mut source = Device::new(true);
                        source.configure_format(uuid, true, Calibration { offset });
                        source.activate_format(44100.0, 512);
                        source
                    });
                    // Initial enrollment waits for the Hub's first real audio progress.
                    for raw in [0, 512, 1024, 1536] {
                        for source in &sources {
                            source.run_format(raw, vec![], None, None, 512);
                        }
                        hub.run_format(raw, vec![], None, None, 512);
                    }
                    for index in capture_order {
                        assert!(sources[index]
                            .run_format(
                                2048,
                                vec![note(
                                    index as i32,
                                    0,
                                    60 + index as i16 * 2,
                                    onset_offset,
                                    true
                                )],
                                None,
                                None,
                                512
                            )
                            .values
                            .is_empty());
                    }
                    hub.run_format(2048, vec![], None, None, 512);
                    if hub_first {
                        hub.run_format(2560, vec![], None, None, 512);
                    }
                    let late = !hub_first && i64::from(onset_offset) + offset >= 512;
                    for index in output_order {
                        let output = sources[index].run_format(2560, vec![], None, None, 512);
                        if late {
                            assert!(output.values.is_empty());
                        } else {
                            assert_assignment_output(&output, index, onset_offset);
                        }
                    }
                    if !hub_first {
                        hub.run_format(2560, vec![], None, None, 512);
                    }
                    for index in output_order {
                        let output = sources[index].run_format(3072, vec![], None, None, 512);
                        if late {
                            assert_assignment_output(&output, index, 0);
                        } else {
                            assert!(output.values.is_empty());
                        }
                        assert_eq!(sources[index].latency(), 512);
                        assert_eq!(sources[index].source_snapshot().faults, 0);
                        assert_eq!(
                            sources[index].shared().extra_delay.load(Ordering::Acquire),
                            if late { 512 - u64::from(onset_offset) } else { 0 }
                        );
                    }
                    hub.run_format(3072, vec![], None, None, 512);
                    for (index, source) in sources.iter().enumerate() {
                        source.run_format(
                            3584,
                            vec![note(index as i32, 0, 60 + index as i16 * 2, 0, false)],
                            None,
                            None,
                            512,
                        );
                    }
                    hub.run_format(3584, vec![], None, None, 512);
                    for raw in (4096..8192).step_by(512) {
                        for source in &sources {
                            source.run_format(raw, vec![], None, None, 512);
                        }
                        hub.run_format(raw, vec![], None, None, 512);
                    }
                }
            }
        }
    }
}

fn assert_assignment_output(output: &Sink, source: usize, time: u32) {
    assert_eq!(output.values.len(), 2, "source {source}: {:?}", output.values);
    assert!(output.values[0].1.attack().is_some());
    assert_eq!(output.values[0].0, time);
    assert_eq!(output.values[1].0, time);
    let Event::Expression { value, id, channel, key, .. } = output.values[1].1 else {
        panic!("missing tuning")
    };
    assert_eq!((id, channel, key), (source as i32, 0, 60 + source as i16 * 2));
    assert_eq!(value, [0.0; 3][source]);
}

#[test]
fn production_mixed_calibration_requires_every_sources_next_interval() {
    let _scope = crate::test_scope::enter();
    let orders = [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]];
    for b_offset in [447, 448, 511] {
        for capture_order in orders {
            for output_order in orders {
                for hub_position in 0..4 {
                    let uuid = SavedUuid::default();
                    let calibration = Calibration { offset: 0 };
                    let mut hub = Device::new(false);
                    hub.configure_format(uuid, true, calibration);
                    hub.activate_format(44100.0, 512);
                    let sources: [Device; 3] = std::array::from_fn(|index| {
                        let mut source = Device::new(true);
                        source.configure_format(
                            uuid,
                            true,
                            Calibration { offset: if index == 1 { 64 } else { 0 } },
                        );
                        source.activate_format(44100.0, 512);
                        source
                    });
                    // Initial enrollment waits for the Hub's first real audio progress.
                    for raw in [0, 512, 1024, 1536] {
                        for source in &sources {
                            source.run_format(raw, vec![], None, None, 512);
                        }
                        hub.run_format(raw, vec![], None, None, 512);
                    }
                    // All three onsets name one mapped sample. At B448/511,
                    // A/C's original input belongs to their NEXT host callback.
                    let onsets = [2048 + b_offset + 64, 2048 + b_offset, 2048 + b_offset + 64];
                    for index in capture_order {
                        let events = if onsets[index] < 2560 {
                            vec![note(
                                index as i32,
                                0,
                                60 + index as i16 * 2,
                                (onsets[index] - 2048) as u32,
                                true,
                            )]
                        } else {
                            vec![]
                        };
                        assert!(sources[index]
                            .run_format(2048, events, None, None, 512)
                            .values
                            .is_empty());
                    }
                    hub.run_format(2048, vec![], None, None, 512);
                    let before_hub = &output_order[..hub_position.min(3)];
                    let b_late = b_offset != 447
                        && !(before_hub.contains(&0)
                            && before_hub.contains(&2)
                            && !before_hub.contains(&1));
                    let mut actual = [None; 3];
                    for (position, index) in
                        output_order.into_iter().map(Some).chain(std::iter::once(None)).enumerate()
                    {
                        if position == hub_position {
                            hub.run_format(2560, vec![], None, None, 512);
                        }
                        let Some(index) = index else { continue };
                        let events = if onsets[index] >= 2560 {
                            vec![note(
                                index as i32,
                                0,
                                60 + index as i16 * 2,
                                (onsets[index] - 2560) as u32,
                                true,
                            )]
                        } else {
                            vec![]
                        };
                        let output = sources[index].run_format(2560, events, None, None, 512);
                        if !output.values.is_empty() {
                            let time = (onsets[index] + 512 - 2560) as u32;
                            assert_assignment_output(&output, index, time);
                            actual[index] = Some(2560 + i64::from(time));
                        }
                    }
                    hub.run_format(3072, vec![], None, None, 512);
                    for index in output_order {
                        let output = sources[index].run_format(3072, vec![], None, None, 512);
                        if !output.values.is_empty() {
                            assert!(actual[index].is_none(), "no duplicate assignment output");
                            let time = (onsets[index] + 512).max(3072) as u32 - 3072;
                            assert_assignment_output(&output, index, time);
                            actual[index] = Some(3072 + i64::from(time));
                        }
                        let expected =
                            if index == 1 && b_late { 3072 } else { onsets[index] + 512 };
                        assert_eq!(actual[index], Some(expected), "B{b_offset} capture={capture_order:?} output={output_order:?} Hub={hub_position}, source={index}");
                        assert_eq!(sources[index].latency(), 512);
                        assert_eq!(
                            sources[index].shared().extra_delay.load(Ordering::Acquire),
                            (expected - onsets[index] - 512) as u64
                        );
                        assert_eq!(sources[index].source_snapshot().faults, 0);
                    }
                    for (index, source) in sources.iter().enumerate() {
                        source.run_format(
                            3584,
                            vec![note(index as i32, 0, 60 + index as i16 * 2, 0, false)],
                            None,
                            None,
                            512,
                        );
                    }
                    hub.run_format(3584, vec![], None, None, 512);
                    for raw in (4096..8192).step_by(512) {
                        for source in &sources {
                            source.run_format(raw, vec![], None, None, 512);
                        }
                        hub.run_format(raw, vec![], None, None, 512);
                    }
                }
            }
        }
    }
}

#[test]
fn production_partial_onset_preserves_actual_pitch_debt_and_take_fault() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let calibration = Calibration { offset: 0 };
    let (recorder, mut capture) = harmonigraph_record::testing::channel();
    crate::configuration::inject_recorder(recorder);
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, calibration);
    hub.activate_format(44100.0, 512);
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let mut source = Device::new(true);
    source.configure_format(uuid, true, calibration);
    source.activate_format(44100.0, 512);
    for raw in [0, 512, 1024] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    capture.arm();
    let directory = std::env::temp_dir()
        .join(format!("harmonigraph-sequencing-partial-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("partial.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    source.run_format(1536, vec![note(7, 0, 59, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    let mut acceptance = vec![false; 1024];
    acceptance[0] = true;
    ACCEPTANCE_SCRIPT.with(|script| *script.borrow_mut() = acceptance);
    let output = source.run_format(2048, vec![], None, None, 512);
    assert_eq!(output.values.len(), 1);
    assert!(output.values[0].1.attack().is_some());
    assert!(matches!(output.rejected[0].1, Event::Expression { kind: 2, .. }));
    assert!(
        matches!(output.rejected[0].1, Event::Expression { value, .. } if (value + 0.11731262).abs() < 1e-9)
    );
    assert_eq!(source.source_snapshot().held, 1, "rejected emergency release keeps its credit");
    let local = inspect_source(&source, |source| *source.state.voices().next().unwrap());
    assert!(local.partial_output && local.release_pending);
    assert_eq!(local.pitch_microcents, 5_900_000_000);
    assert_eq!(local.frozen_offset_microcents, 0);
    assert_eq!(local.player_tuning, 0.0);
    assert_eq!(local.assignment, None, "failed intended tuning is not accepted metadata");
    hub.run_format(2048, vec![], None, None, 512);
    let observed = inspect_hub(&hub, |hub| hub.test_voice(0, local.lifetime).unwrap());
    assert!(observed.partial_output && observed.release_pending);
    assert_eq!(observed.assignment, None);
    writer.drain(&mut capture);
    let released = source.run_format(2560, vec![], None, None, 512);
    assert_eq!(released.values.iter().filter(|(_, event)| event.release()).count(), 1);
    assert!(!released.values.iter().any(|(_, event)| matches!(event, Event::Expression { .. })));
    hub.run_format(2560, vec![], None, None, 512);
    for raw in (3072..5632).step_by(512) {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        writer.drain(&mut capture);
    }
    capture.stop();
    writer.stop();
    source.run_format(5632, vec![], None, None, 512);
    hub.run_format(5632, vec![], None, None, 512);
    writer.drain(&mut capture);
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let partial: Vec<_> = take
        .events
        .iter()
        .filter_map(|record| match record {
            harmonigraph_take::CanonicalRecord::Delta(delta) if delta.partial_output => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(partial.len(), 1);
    assert!(matches!(partial[0].event.kind, harmonigraph_take::NoteKind::On { .. }));
    assert_eq!(partial[0].timing.unwrap().planned, Some(2048));
    assert_eq!(partial[0].pitch_microcents, Some(5_900_000_000));
    assert_eq!(partial[0].assignment, None);
    assert_eq!(source.source_snapshot().held, 0);
    assert_ne!(source.shared().status.load(Ordering::Acquire) & source::OUTPUT_FAULT, 0);
    drop(writer);
    drop(source);
    drop(hub);
    std::fs::remove_dir_all(directory).unwrap();
    assert_eq!(
        registry::global().lock().unwrap().test_counts(),
        (0, 0, 0),
        "a completed cohort label cannot retain the partial onset's capture after callback join"
    );
}

#[test]
fn production_unpayable_cohort_debt_faults_instead_of_panicking() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    source.run_format(1536, vec![note(99, 0, 60, 0, true)], None, None, 512);
    let hub_wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    // A full reply ring is the production route to an unsent in-cohort plan:
    // the assignment is minted and owed but cannot enqueue.
    hub_wrapper.test_with_plugin(|plugin| {
        let hub = plugin.aggregation.as_mut().unwrap();
        let replies = &mut hub.offer.as_mut().unwrap().bank.as_mut().unwrap().rows[0].replies;
        while replies.slots() != 0 {
            replies
                .push(protocol::Reply::PlanRetired {
                    incarnation: 0,
                    epoch: 0,
                    life: 0,
                    lifetime: 0,
                    decision: 0,
                })
                .unwrap();
        }
    });
    hub.run_format(1536, vec![], None, None, 512);
    let session = inspect_hub(&hub, |hub| hub.offer.as_ref().unwrap().session.clone());
    assert_eq!(session.faults.load(Ordering::Acquire), 0);
    // Only the counter is corrupted; the plan keeps the binding, decision and
    // unsent flag production gave it, so the branch is entered on real state.
    assert_eq!(
        hub_wrapper.test_with_plugin(|plugin| plugin
            .aggregation
            .as_mut()
            .unwrap()
            .test_clear_cohort_unsent()),
        1,
        "the fixture starts from a real outstanding cohort delivery"
    );
    hub.run_format(2048, vec![], None, None, 512);
    assert_ne!(
        session.faults.load(Ordering::Acquire) & source::STORAGE_FAULT,
        0,
        "the unpayable debt latches instead of panicking the audio callback"
    );
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_cohort_delivery().1),
        0,
        "the counter is abandoned at zero rather than wrapped"
    );
    drop(source);
    drop(hub);
}

#[test]
fn production_fifteen_note_mixed_offsets_preserve_gestures_without_terminal_lateness() {
    let _scope = crate::test_scope::enter();
    for b_offset in [447i64, 448, 511] {
        for favorable in [true, false] {
            let uuid = SavedUuid::default();
            let calibration = Calibration { offset: 0 };
            let mut hub = Device::new(false);
            hub.configure_format(uuid, true, calibration);
            hub.activate_format(44100.0, 512);
            let sources: [Device; 3] = std::array::from_fn(|index| {
                let mut source = Device::new(true);
                source.configure_format(
                    uuid,
                    true,
                    Calibration { offset: if index == 1 { 64 } else { 0 } },
                );
                source.activate_format(44100.0, 512);
                source
            });
            let mut setup = [0; 3];
            // Initial enrollment waits for the Hub's first real audio progress.
            for raw in [0, 512, 1024, 1536] {
                for (index, source) in sources.iter().enumerate() {
                    let input = if raw == 0 {
                        [64, 66, 69].into_iter().map(|cc| raw_midi([0xb0, cc, 0], 0)).collect()
                    } else {
                        vec![]
                    };
                    setup[index] += source.run_format(raw, input, None, None, 512).values.len();
                }
                hub.run_format(raw, vec![], None, None, 512);
            }
            assert_eq!(setup, [3; 3], "accepted neutral controller precondition");
            let onsets = [2048 + b_offset + 64, 2048 + b_offset, 2048 + b_offset + 64];
            let mut actual: [Vec<(i64, Event)>; 3] = std::array::from_fn(|_| Vec::new());
            let mut extra = [0; 3];
            for raw in (2048..7680).step_by(512) {
                // At the decisive callback, favorable is A,C,Hub,B; adverse is
                // B,A,C,Hub. Events are sorted across all five notes and split
                // at their actual enclosing boundaries, including +10/+20.
                let order = if favorable { [0, 2, 3, 1] } else { [1, 0, 2, 3] };
                for index in order {
                    if index == 3 {
                        hub.run_format(raw, vec![], None, None, 512);
                        continue;
                    }
                    let mut input = Vec::new();
                    for note_index in 0..5 {
                        let id = (index * 5 + note_index) as i32;
                        let key = 48 + id as i16;
                        for delta in [0, 10, 20] {
                            let sample = onsets[index] + delta;
                            if !(raw..raw + 512).contains(&sample) {
                                continue;
                            }
                            let time = (sample - raw) as u32;
                            input.push(match delta {
                                0 => note(id, 0, key, time, true),
                                10 => expression(id, 0.125, time),
                                _ => note(id, 0, key, time, false),
                            });
                        }
                    }
                    input.sort_by_key(|event| event.header().time);
                    let output = sources[index].run_format(raw, input, None, None, 512);
                    actual[index].extend(
                        output
                            .values
                            .into_iter()
                            .map(|(offset, event)| (raw + i64::from(offset), event)),
                    );
                    extra[index] = extra[index]
                        .max(sources[index].shared().extra_delay.load(Ordering::Acquire));
                    assert_eq!(
                        sources[index].source_snapshot().faults,
                        0,
                        "B{b_offset} favorable={favorable}, Source{index}"
                    );
                }
                assert_eq!(inspect_hub(&hub, |hub| hub.test_terminal_scope()), (false, 0));
            }
            assert_eq!(actual.iter().map(Vec::len).sum::<usize>(), 60);
            // The real finite recovery reader outlives these short gestures.
            // Its32-callback inventory scan keeps Originals pinned until ACK.
            for raw in (7680..7680 + 128 * 512).step_by(512) {
                for source in &sources {
                    assert!(source.run_format(raw, vec![], None, None, 512).values.is_empty());
                    assert_eq!(source.source_snapshot().faults, 0);
                }
                hub.run_format(raw, vec![], None, None, 512);
                if sources.iter().all(|source| source.source_snapshot().captures == 0) {
                    break;
                }
            }
            for index in 0..3 {
                let onset =
                    actual[index].iter().find(|(_, event)| event.attack().is_some()).unwrap().0;
                assert!(onset >= onsets[index] + 512);
                assert_eq!(extra[index], (onset - onsets[index] - 512) as u64);
                for note_index in 0..5 {
                    let id = (index * 5 + note_index) as i32;
                    let events: Vec<_> = actual[index]
                        .iter()
                        .filter(|(_, event)| match event {
                            Event::Note { id: found, .. } | Event::Expression { id: found, .. } => {
                                *found == id
                            }
                            _ => false,
                        })
                        .collect();
                    assert_eq!(events.len(), 4);
                    assert!(events[0].1.attack().is_some());
                    assert_eq!(events[0].0, onset);
                    let Event::Expression { value: initial, .. } = events[1].1 else {
                        panic!("initial tuning")
                    };
                    assert_eq!(events[1].0, onset);
                    let Event::Expression { value: expressed, .. } = events[2].1 else {
                        panic!("player expression")
                    };
                    assert_eq!((events[2].0, expressed), (onset + 10, initial));
                    assert!(events[3].1.release());
                    assert_eq!(events[3].0, onset + 20);
                }
                assert_eq!(
                    (
                        sources[index].source_snapshot().held,
                        sources[index].source_snapshot().captures
                    ),
                    (0, 0)
                );
            }
            println!("FIFTEEN B{b_offset} favorable={favorable} extra={extra:?}, exact60 accepted events; automatic recovery ownership settled");
            drop(sources);
            drop(hub);
            assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
        }
    }
}

#[test]
fn production_reset_requires_fresh_complete_input_after_a_same_class_fault() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    let mut raw = 1536;
    let lost_input = || (0..2049).map(|_| raw_midi([0xf8, 0, 0], 0)).collect();
    source.run_status(raw, lost_input(), None, None, 512, true);
    hub.run_format(raw, vec![], None, None, 512);
    for _ in 0..160 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let source_setup = source.shared();
    let hub_setup = hub.shared();
    source_setup.apply(source_setup.value().routing, true).unwrap();
    hub_setup.apply(hub_setup.value().routing, true).unwrap();
    for _ in 0..256 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        source.main();
        hub.main();
        if inspect_source(&source, |s| s.test_reset_armed()) {
            break;
        }
    }
    assert!(
        inspect_source(&source, |s| s.test_reset_armed()),
        "{}",
        inspect_source(&source, |s| s.test_reset_progress())
    );
    assert_eq!(source_setup.applied.load(Ordering::Acquire), source_setup.value().generation);
    assert_ne!(
        source.source_snapshot().faults & source::INPUT_FAULT,
        0,
        "applied Reset still awaits fresh coverage"
    );
    raw += 512;
    source.run_status(raw, lost_input(), None, None, 512, true);
    hub.run_format(raw, vec![], None, None, 512);
    assert!(!inspect_source(&source, |s| s.test_reset_armed()));
    for _ in 0..160 {
        raw += 512;
        assert!(source
            .run_format(raw, vec![note(20, 0, 60, 0, true)], None, None, 512)
            .values
            .is_empty());
        hub.run_format(raw, vec![], None, None, 512);
        assert_ne!(source.source_snapshot().faults & source::INPUT_FAULT, 0);
    }
    source_setup.apply(source_setup.value().routing, true).unwrap();
    hub_setup.apply(hub_setup.value().routing, true).unwrap();
    for _ in 0..256 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        source.main();
        hub.main();
        if source.source_snapshot().faults == 0 {
            break;
        }
    }
    assert_eq!(
        source.source_snapshot().faults,
        0,
        "{}",
        inspect_source(&source, |s| s.test_reset_progress())
    );
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn production_same_sample_group_past_one_queue_latches_instead_of_stalling() {
    let _scope = crate::test_scope::enter();
    // Both input owners stage into a CAPTURES_PER_SOURCE queue and both stop
    // collecting when it fills: a Tune's row through the intent ring, and the
    // Hub's own MIDI without one. The group is oversized for either.
    for slot in [1, 0] {
        let (hub, source) = production_pair();
        let (played, other) = if slot == 0 { (&hub, &source) } else { (&source, &hub) };
        let mut raw = 1536;
        let held = (0..64).map(|index| note(index, 0, index as i16, index as u32, true)).collect();
        played.run_format(raw, held, None, None, 512);
        other.run_format(raw, vec![], None, None, 512);
        for _ in 0..128 {
            raw += 512;
            played.run_format(raw, vec![], None, None, 512);
            other.run_format(raw, vec![], None, None, 512);
        }
        // Seventeen wildcard tuning expressions at one sample copy one record
        // per addressed note: 1,088, past the 1,024 either queue stages. The
        // 2,048-record batch overflow never sees them; collection stops first.
        raw += 512;
        let wildcards =
            (0..17).map(|index| expression(-1, 0.01 + f64::from(index) * 0.001, 0)).collect();
        played.run_format(raw, wildcards, None, None, 512);
        other.run_format(raw, vec![], None, None, 512);
        let session = inspect_hub(&hub, |hub| hub.offer.as_ref().unwrap().session.clone());
        let mut group = None;
        let mut latched = false;
        for _ in 0..24 {
            raw += 512;
            played.run_format(raw, vec![], None, None, 512);
            other.run_format(raw, vec![], None, None, 512);
            let staged = inspect_hub(&hub, |hub| hub.test_inputs(slot));
            if group.is_none() && staged.len() == protocol::CAPTURES_PER_SOURCE {
                group = Some(staged.iter().map(|record| record.sample).collect::<Vec<_>>());
            }
            latched |= session.faults.load(Ordering::Acquire) & source::STORAGE_FAULT != 0;
        }
        let group = group.unwrap_or_else(|| panic!("slot {slot} fills with the oversized group"));
        assert!(
            group.iter().all(|sample| *sample == group[0]),
            "slot {slot}: every staged record stands at one sample, which is what makes the group unconsumable"
        );
        assert!(
            latched,
            "slot {slot}: an unconsumable same-sample group is a bounded storage failure, not a silent stall"
        );
        assert_eq!(
            inspect_hub(&hub, |hub| hub.test_inputs(slot).len()),
            0,
            "slot {slot}: the latch settles the stream and drops copies it can never sequence"
        );
        drop(source);
        drop(hub);
    }
}

#[test]
fn production_a_sample_too_big_for_the_rest_of_a_callback_waits_rather_than_vanishing() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let calibration = Calibration { offset: 0 };
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, calibration);
    hub.activate_format(44100.0, 512);
    let sources: [Device; 16] = std::array::from_fn(|_| {
        let mut source = Device::new(true);
        source.configure_format(uuid, true, calibration);
        source.activate_format(44100.0, 512);
        source
    });
    let mut raw = 0;
    for _ in 0..3 {
        for source in &sources {
            source.run_format(raw, vec![], None, None, 512);
        }
        hub.run_format(raw, vec![], None, None, 512);
        raw += 512;
    }
    raw = 1536;
    for (index, source) in sources.iter().enumerate() {
        let held = (0..16)
            .map(|key| note(key, index as i16 % 16, 48 + key as i16, 0, true))
            .collect::<Vec<_>>();
        source.run_format(raw, held, None, None, 512);
    }
    hub.run_format(raw, vec![], None, None, 512);
    for _ in 0..64 {
        raw += 512;
        for source in &sources {
            source.run_format(raw, vec![], None, None, 512);
        }
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context().len()),
        256,
        "the crowded sample needs every source's sixteen notes sounding to address"
    );
    // One crowded sample per source: six wildcard tuning expressions over the
    // sixteen held notes (96 records) plus sixteen same-key retriggers, each
    // copying its predecessor's choke beside its own onset (32). 128 a source,
    // 2,048 across the session, every one of them at that sample. Collecting
    // them costs 2,064 of the callback's 4,096, which leaves less than the
    // 2,048 assembly then needs.
    raw += 512;
    for (index, source) in sources.iter().enumerate() {
        let mut crowd = (0..6).map(|_| expression(-1, 0.25, 0)).collect::<Vec<_>>();
        crowd.extend(
            (0..16).map(|key| note(100 + key, index as i16 % 16, 48 + key as i16, 0, true)),
        );
        source.run_format(raw, crowd, None, None, 512);
    }
    hub.run_format(raw, vec![], None, None, 512);
    let mut sounded = 0;
    for _ in 0..64 {
        raw += 512;
        for source in &sources {
            sounded += source
                .run_format(raw, vec![], None, None, 512)
                .values
                .iter()
                .filter(|(_, event)| event.attack().is_some())
                .count();
        }
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        sounded, 256,
        "a deferred sample is late, not lost: every retrigger still gets its assignment"
    );
    assert_eq!(
        inspect_hub(&hub, |hub| (1..=16).map(|s| hub.test_inputs(s).len()).sum::<usize>()),
        0,
        "and the deferral leaves nothing behind once the next callback takes it"
    );
    for source in &sources {
        raw += 512;
        source.run_format(raw, vec![note(-1, -1, -1, 0, false)], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    for _ in 0..64 {
        raw += 512;
        for source in &sources {
            source.run_format(raw, vec![], None, None, 512);
        }
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert!(sources.iter().all(|source| source.source_snapshot().held == 0));
    drop(sources);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn production_reset_retires_a_cohort_that_never_published_its_marker() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    let hub_wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    // A full reply ring is the production route to a cohort that cannot
    // publish its marker: the assignment is minted and owed but cannot enqueue.
    let fill = || {
        hub_wrapper.test_with_plugin(|plugin| {
            let hub = plugin.aggregation.as_mut().unwrap();
            let replies = &mut hub.offer.as_mut().unwrap().bank.as_mut().unwrap().rows[0].replies;
            while replies.slots() != 0 {
                replies
                    .push(protocol::Reply::PlanRetired {
                        incarnation: 0,
                        epoch: 0,
                        life: 0,
                        lifetime: 0,
                        decision: 0,
                    })
                    .unwrap();
            }
        });
    };
    let mut raw = 1536;
    source.run_format(raw, vec![note(1, 0, 60, 0, true)], None, None, 512);
    fill();
    hub.run_format(raw, vec![], None, None, 512);
    for _ in 0..8 {
        if inspect_hub(&hub, |hub| hub.test_cohort_delivery()).3 {
            break;
        }
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        fill();
        hub.run_format(raw, vec![], None, None, 512);
    }
    let pending = inspect_hub(&hub, |hub| hub.test_cohort_delivery());
    assert!(
        pending.3 && pending.2 != 0,
        "the fixture starts from a cohort holding an unpublished marker: {pending:?}"
    );
    // A latched terminal fault from the Source's own malformed input.
    raw += 512;
    let malformed = Input::Midi(clap_event_midi {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_header>() as u32,
            ..header::<clap_event_midi>(CLAP_EVENT_MIDI, 0)
        },
        port_index: 0,
        data: [0xf8, 0, 0],
    });
    source.run_status(raw, vec![malformed], None, None, 512, true);
    hub.run_format(raw, vec![], None, None, 512);
    for _ in 0..16 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert!(
        inspect_hub(&hub, |hub| hub.test_terminal_scope()).0,
        "the fault latches the shared reset, which is what bypasses publish_cohort"
    );
    assert!(
        inspect_hub(&hub, |hub| hub.test_cohort_delivery()).3,
        "and leaves the marker unpublished across it"
    );
    let source_setup = source.shared();
    let hub_setup = hub.shared();
    source_setup.apply(source_setup.value().routing, true).unwrap();
    hub_setup.apply(hub_setup.value().routing, true).unwrap();
    for _ in 0..256 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        source.main();
        hub.main();
        if source.source_snapshot().faults == 0 {
            break;
        }
    }
    assert_eq!(source.source_snapshot().faults, 0, "the explicit reset settles");
    let settled = inspect_hub(&hub, |hub| hub.test_cohort_delivery());
    assert!(
        !settled.3 && settled.2 == 0 && settled.1 == 0,
        "the old session's commit barrier goes with it: {settled:?}"
    );
    let mut sounded = 0;
    for step in 0..64 {
        raw += 512;
        let events = if step == 0 { vec![note(2, 0, 62, 0, true)] } else { vec![] };
        sounded += source
            .run_format(raw, events, None, None, 512)
            .values
            .iter()
            .filter(|(_, event)| event.attack().is_some())
            .count();
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        sounded, 1,
        "a re-paired session sequences fresh input rather than returning on a dead barrier"
    );
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context()),
        vec![(1, 2)],
        "and the Hub believes the note it assigned is the one that is sounding"
    );
    raw += 512;
    source.run_format(raw, vec![note(-1, -1, -1, 0, false)], None, None, 512);
    hub.run_format(raw, vec![], None, None, 512);
    for _ in 0..64 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn production_a_full_sixty_four_voices_still_replace_one_of_their_own_in_one_callback() {
    let _scope = crate::test_scope::enter();
    let musical = |sink: &Sink| {
        sink.values
            .iter()
            .filter(|(_, event)| !matches!(event, Event::Midi { data: [0xf8, ..], .. }))
            .copied()
            .collect::<Vec<_>>()
    };
    let (hub, source) = production_pair();
    let mut raw = 1536;
    let sounding =
        (0..64).map(|index| note(index + 1, 0, 24 + index as i16, 0, true)).collect::<Vec<_>>();
    source.run_format(raw, sounding, None, None, 512);
    hub.run_format(raw, vec![], None, None, 512);
    loop {
        raw += 512;
        assert!(raw < 8192, "the sixty-four never sounded");
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        if source.source_snapshot().held == 64 {
            break;
        }
    }
    // Every one of the source's 64 reservations is spoken for, and the one this
    // retrigger displaces stays charged until the Hub acknowledges its release
    // — a whole round trip after the choke goes out. A replacement that had to
    // find a 65th would emit its choke here and its onset in a later callback,
    // leaving the key dead in between.
    raw += 512;
    source.run_format(raw, vec![note(200, 0, 24, 0, true)], None, None, 512);
    hub.run_format(raw, vec![], None, None, 512);
    let replaced = loop {
        raw += 512;
        assert!(raw < 16384, "the retrigger never emitted");
        let out = source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        if !musical(&out).is_empty() {
            break out;
        }
    };
    assert!(
        matches!(
            musical(&replaced)[..],
            [
                (0, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 1, key: 24, .. }),
                (0, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 200, key: 24, .. }),
                (0, Event::Expression { kind: 2, id: 200, .. }),
            ]
        ),
        "a full source replaces one of its own voices whole: {:?}",
        musical(&replaced)
    );
    assert_eq!(
        source.source_snapshot().held,
        64,
        "and the pair shares one reservation rather than needing a sixty-fifth"
    );
    // DIRECT reaches the same preparation with no round trip in the way, so its
    // predecessor's release is unacknowledged for certain: choke and onset are
    // in the callback the retrigger arrives in.
    raw += 512;
    let sounding =
        (0..64).map(|index| note(index + 1, 0, 24 + index as i16, 0, true)).collect::<Vec<_>>();
    source.run_format(raw, vec![], None, None, 512);
    assert_eq!(hub.run_format(raw, sounding, None, None, 512).values.len(), 64);
    raw += 512;
    source.run_format(raw, vec![], None, None, 512);
    let direct = hub.run_format(raw, vec![note(200, 0, 24, 0, true)], None, None, 512);
    assert!(
        matches!(
            musical(&direct)[..],
            [
                (0, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 1, key: 24, .. }),
                (0, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 200, key: 24, .. }),
            ]
        ),
        "DIRECT replaces its own voice the same way: {:?}",
        musical(&direct)
    );
    let session = inspect_hub(&hub, |hub| hub.offer.as_ref().unwrap().session.clone());
    let mut off = vec![note(200, 0, 24, 0, false)];
    off.extend((1..64).map(|index| note(index + 1, 0, 24 + index as i16, 0, false)));
    raw += 512;
    source.run_format(raw, off.clone(), None, None, 512);
    hub.run_format(raw, off, None, None, 512);
    for _ in 0..16 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let settled = source.source_snapshot();
    assert_eq!((settled.held, settled.lives, settled.faults), (0, 0, 0), "{settled:?}");
    // An inherited reservation is still exactly one credit, carrying the
    // predecessor's acknowledgement debt as well as the successor's, so the
    // session comes back to nothing owed rather than one short.
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
}

/// A tuning edit is effective at the NEXT Hub block boundary, and the group
/// assigned before it keeps the configuration it started with -- through the
/// emission that lands a whole callback after that boundary.
#[test]
fn production_a_tuning_edit_waits_for_the_next_boundary_and_a_group_keeps_its_snapshot() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    let bends = |output: &Sink| {
        output
            .values
            .iter()
            .filter_map(|(_, event)| match event {
                Event::Expression { kind: 2, id, value, .. } => Some((*id, *value)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // A fifth of 710c, adopted at the 2048 boundary. Neither configuration in
    // this fixture is the default, so a lost snapshot cannot read as a kept one.
    hub.run_format(1536, vec![tuning_parameter(&hub, 710.0, 0)], None, None, 512);
    source.run_format(1536, vec![], None, None, 512);
    // D and A, one group at raw2148, while THIS Hub block also carries 690c at
    // its own offset0. Two fifths and three fifths from the origin, so the
    // value the group is assigned under is visible in the bend.
    source.run_format(
        2048,
        vec![note(1, 0, 62, 100, true), note(2, 0, 69, 100, true)],
        None,
        None,
        512,
    );
    hub.run_format(2048, vec![tuning_parameter(&hub, 690.0, 0)], None, None, 512);
    // Input2148 + D512 = raw2660, so the group completes a whole callback after
    // the boundary that adopted 690c.
    let group = bends(&source.run_format(2560, vec![], None, None, 512));
    hub.run_format(2560, vec![], None, None, 512);
    assert_eq!(
        group,
        [(1, 0.2), (2, 0.3)],
        "the group keeps the 710c it was assigned under, across the boundary that adopted 690c"
    );
    // The same D, assigned by a Hub block that has adopted the edit.
    let mut raw = 3072;
    source.run_format(
        raw,
        vec![note(1, 0, 62, 0, false), note(2, 0, 69, 0, false)],
        None,
        None,
        512,
    );
    hub.run_format(raw, vec![], None, None, 512);
    raw += 512;
    source.run_format(raw, vec![note(3, 0, 62, 0, true)], None, None, 512);
    hub.run_format(raw, vec![], None, None, 512);
    raw += 512;
    let after = bends(&source.run_format(raw, vec![], None, None, 512));
    hub.run_format(raw, vec![], None, None, 512);
    assert_eq!(after, [(3, -0.2)], "two fifths of 690c is 20 cents flat of the tempered second");
    raw += 512;
    source.run_format(raw, vec![note(3, 0, 62, 0, false)], None, None, 512);
    hub.run_format(raw, vec![], None, None, 512);
    for _ in 0..4 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let settled = source.source_snapshot();
    assert_eq!((settled.held, settled.lives, settled.faults), (0, 0, 0), "{settled:?}");
}

#[test]
fn production_a_host_format_reactivation_reestablishes_the_paired_session() {
    let _scope = crate::test_scope::enter();
    let (mut hub, mut source) = production_pair();
    let phrase = |source: &Device, hub: &Device, raw: &mut i64, id: i32, key: i16, frames: u32| {
        let mut output =
            source.run_format(*raw, vec![note(id, 0, key, 0, true)], None, None, frames).values;
        hub.run_format(*raw, vec![], None, None, frames);
        for _ in 0..16 {
            *raw += i64::from(frames);
            output.extend(source.run_format(*raw, vec![], None, None, frames).values);
            hub.run_format(*raw, vec![], None, None, frames);
        }
        *raw += i64::from(frames);
        output.extend(
            source.run_format(*raw, vec![note(id, 0, key, 0, false)], None, None, frames).values,
        );
        hub.run_format(*raw, vec![], None, None, frames);
        for _ in 0..16 {
            *raw += i64::from(frames);
            output.extend(source.run_format(*raw, vec![], None, None, frames).values);
            hub.run_format(*raw, vec![], None, None, frames);
        }
        output
    };
    let mut raw = 1536;
    let before = phrase(&source, &hub, &mut raw, 1, 64, 512);
    assert_eq!(before.iter().filter(|(_, event)| event.attack().is_some()).count(), 1);
    assert_eq!(source.source_snapshot().held, 0);

    // The host changes its processing format: stop, deactivate, activate,
    // start. Nothing is outstanding, so this is a boundary and not a failure.
    hub.reactivate_format(48000.0, 256);
    source.reactivate_format(48000.0, 256);
    for _ in 0..8 {
        raw += 256;
        source.main();
        hub.main();
        source.run_format(raw, vec![], None, None, 256);
        hub.run_format(raw, vec![], None, None, 256);
    }
    for adopted in [hub.shared().adopted().unwrap(), source.shared().adopted().unwrap()] {
        assert_eq!((adopted.sample_rate, adopted.max_frames), (48000.0, 256));
        assert!(adopted.valid, "the reactivated format is adopted, not latched: {adopted:?}");
    }
    assert_eq!(source.source_snapshot().faults, 0, "{:?}", source.source_snapshot());

    raw += 256;
    let after = phrase(&source, &hub, &mut raw, 2, 67, 256);
    assert_eq!(
        after.iter().filter(|(_, event)| event.attack().is_some()).count(),
        1,
        "a phrase played after the reactivation sounds: {:?}",
        source.source_snapshot()
    );
    assert_eq!(after.iter().filter(|(_, event)| event.release()).count(), 1);
    assert_eq!(source.source_snapshot().held, 0);
}

#[test]
fn a_delay_edit_leaves_participation_where_the_host_put_it() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    let wrapper = unsafe {
        &*((*source.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    let (context, delay) = wrapper.test_gui_context("tuning_delay");
    // A stepped parameter carries its step index, so the multiplier's ends are
    // a zero for 1x buffer and fifteen for 16x -- the same two numbers as Off
    // and On, which is what made reading this as participation invisible.
    let steps = protocol::DELAY_MULTIPLIER_MAX - 1;
    let mut raw = 1536;
    // One delay edit made each of the two ways a Tune receives them: a
    // finished slider gesture, admitted at the next capture, and plain host
    // automation. Both are read back from both sides afterwards, because the
    // defect moved one of them and left the other saying the opposite.
    let edit = |raw: &mut i64, value: i32, gesture: bool, participating: bool| {
        let automation = if gesture {
            unsafe {
                context.raw_begin_set_parameter(delay);
                context.raw_set_parameter_normalized(delay, value as f32 / steps as f32);
                context.raw_end_set_parameter(delay);
            }
            vec![]
        } else {
            vec![source.param_event(DELAY_PARAM, f64::from(value), 1)]
        };
        source.run_format(*raw, automation, None, None, 512);
        hub.run_format(*raw, vec![], None, None, 512);
        source.main();
        *raw += 512;
        assert_eq!(source.param_value(DELAY_PARAM), f64::from(value), "the edit was delivered");
        assert_eq!(
            inspect_source(&source, |source| source.participating),
            participating,
            "a {value}-step delay edit moved the Source's participation"
        );
        assert_eq!(
            source.param_value(PARTICIPATING_PARAM),
            f64::from(u8::from(participating)),
            "and the host's own readback disagrees with it"
        );
    };
    for gesture in [true, false] {
        edit(&mut raw, 0, gesture, true);
        edit(&mut raw, steps, gesture, true);
    }
    source.run_format(raw, vec![source.participation(false, 1)], None, None, 512);
    hub.run_format(raw, vec![], None, None, 512);
    raw += 512;
    assert!(!inspect_source(&source, |source| source.participating));
    // Off is the state a delay edit used to switch back on.
    for gesture in [true, false] {
        edit(&mut raw, 0, gesture, false);
        edit(&mut raw, steps, gesture, false);
    }
    assert_eq!(source.source_snapshot().faults, 0);
    drop(context);
}

#[test]
fn a_pending_delay_change_keeps_the_reported_latency_at_the_active_one() {
    let _scope = crate::test_scope::enter();
    let (mut hub, mut source) = production_pair();
    assert_eq!(source.latency(), 512);
    // 2x buffer, requested while this activation runs 1x, alongside a note.
    source.run_format(
        1536,
        vec![note(7, 0, 60, 0, true), source.param_event(DELAY_PARAM, 1.0, 1)],
        None,
        None,
        512,
    );
    hub.run_format(1536, vec![], None, None, 512);
    let emitted = source.run_format(2048, vec![], None, None, 512);
    hub.run_format(2048, vec![], None, None, 512);
    source.main();
    assert_eq!(emitted.values[0].0, 0, "the note still emits at input + the active 512");
    assert_eq!(source.latency(), 512, "which is the latency the host is still told to expect");
    assert_eq!(source._stats.restarts.load(Ordering::Relaxed), 1, "one restart is requested");
    assert_eq!(
        source._stats.latency_changes.load(Ordering::Relaxed),
        1,
        "and nothing is published"
    );
    assert_eq!(
        source._stats.observed_latency.load(Ordering::Relaxed),
        512,
        "the only value this host has ever read is the one it is compensating for"
    );
    source.run_format(2560, vec![note(7, 0, 60, 0, false)], None, None, 512);
    hub.run_format(2560, vec![], None, None, 512);
    for raw in [3072, 3584] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(source.source_snapshot().held, 0);
    // The host answers the request. The multiplier is adopted whole at that
    // boundary and the reported latency moves with it, together.
    hub.reactivate_format(44100.0, 512);
    source.reactivate_format(44100.0, 512);
    assert_eq!(source.latency(), 1024);
    assert_eq!(source._stats.latency_changes.load(Ordering::Relaxed), 2, "published exactly once");
    assert_eq!(
        source._stats.observed_latency.load(Ordering::Relaxed),
        1024,
        "and reading it the way a host does, inside the notification, finds the new delay"
    );
    let mut raw = 3584;
    for _ in 0..8 {
        raw += 512;
        source.main();
        hub.main();
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    raw += 512;
    source.run_format(raw, vec![note(8, 0, 62, 0, true)], None, None, 512);
    hub.run_format(raw, vec![], None, None, 512);
    assert!(
        source.run_format(raw + 512, vec![], None, None, 512).values.is_empty(),
        "input + the old 512 is now a callback short of the delay"
    );
    hub.run_format(raw + 512, vec![], None, None, 512);
    let after = source.run_format(raw + 1024, vec![], None, None, 512);
    assert_eq!(after.values[0].0, 0, "and the note emits at input + the adopted 1024");
    assert!(after.values[0].1.attack().is_some());
    hub.run_format(raw + 1024, vec![], None, None, 512);
    // End the phrase rather than abandoning a held stream in the registry.
    source.run_format(raw + 1536, vec![note(8, 0, 62, 0, false)], None, None, 512);
    hub.run_format(raw + 1536, vec![], None, None, 512);
    for step in 1..6 {
        let raw = raw + 1536 + 512 * step;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(source.source_snapshot().held, 0);
}

/// What the host has been told about the latency, and what it read when it was
/// told. A host compensates with the value it takes inside `changed()`, so a
/// published number nobody was told about is a number nobody is using.
fn announced(device: &Device) -> (usize, usize) {
    (
        device._stats.latency_changes.load(Ordering::Relaxed),
        device._stats.observed_latency.load(Ordering::Relaxed),
    )
}
fn restarts(device: &Device) -> usize {
    device._stats.restarts.load(Ordering::Relaxed)
}

/// Notifications that reached the host outside `activate`. CLAP allows the
/// latency to change only there, so one delivered anywhere else is one a
/// conforming host may discard -- and a plugin that spends its notification
/// out of phase has told nobody anything.
fn out_of_phase(device: &Device) -> usize {
    device._stats.out_of_phase.load(Ordering::Relaxed)
}

#[test]
fn an_activation_announces_a_delay_whose_task_ran_while_deactivated() {
    // Two requests, the first serviced while the activation it cannot move is
    // still running and the second serviced in the gap between deactivation
    // and reactivation. Neither delivery is a moment the host may be told at,
    // so the activation that adopts the second one is left holding the only
    // notification the host will accept.
    let _scope = crate::test_scope::enter();
    let (mut hub, mut source) = production_pair();
    assert_eq!(announced(&source), (1, 512), "the first activation published its own delay");
    // 2x, requested and delivered while active: a restart and nothing else.
    source.run_format(1536, vec![source.param_event(DELAY_PARAM, 1.0, 1)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    source.run_format(2048, vec![], None, None, 512);
    hub.run_format(2048, vec![], None, None, 512);
    source.main();
    hub.main();
    assert_eq!(restarts(&source), 1, "the active request asks for the restart that adopts it");
    assert_eq!(announced(&source), (1, 512));
    // 3x, and this time the host services the task after deactivating.
    source.run_format(2560, vec![source.param_event(DELAY_PARAM, 2.0, 1)], None, None, 512);
    hub.run_format(2560, vec![], None, None, 512);
    source.run_format(3072, vec![], None, None, 512);
    hub.run_format(3072, vec![], None, None, 512);
    source.deactivate();
    source.main();
    assert_eq!(out_of_phase(&source), 0, "a task between activations tells the host nothing");
    assert_eq!(announced(&source), (1, 512), "so the host is still on the delay it read");
    assert_eq!(source.latency(), 512, "and still reads the one it is compensating for");
    assert_eq!(restarts(&source), 1, "a deactivated plugin asks for no restart");
    hub.reactivate_format(44100.0, 512);
    source.activate_format(44100.0, 512);
    assert_eq!(source.latency(), 1536, "the activation adopts the last request");
    assert_eq!(
        announced(&source),
        (2, 1536),
        "and is the one that tells the host, at a moment the host may be told"
    );
}

#[test]
fn an_activation_that_adopts_a_pending_delay_announces_it() {
    // The host reactivates before it services `on_main_thread`, so the
    // activation adopts the request while the task that would have announced
    // it is still queued. Adopting it silently leaves the host compensating
    // for the delay it last read, with the notification it is waiting for
    // deduplicated away by the request the activation already carries.
    let _scope = crate::test_scope::enter();
    let (mut hub, mut source) = production_pair();
    assert_eq!(announced(&source), (1, 512), "the first activation published its own delay");
    assert_eq!(restarts(&source), 0);
    source.run_format(1536, vec![source.param_event(DELAY_PARAM, 1.0, 1)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    source.run_format(2048, vec![], None, None, 512);
    hub.run_format(2048, vec![], None, None, 512);
    assert_eq!(announced(&source), (1, 512), "the request alone tells the host nothing");
    assert_eq!(restarts(&source), 0, "and the task carrying it has not been delivered");
    hub.reactivate_format(44100.0, 512);
    source.reactivate_format(44100.0, 512);
    assert_eq!(source.latency(), 1024, "the activation adopted the request");
    assert_eq!(
        announced(&source),
        (2, 1024),
        "and told the host, which read the adopted delay rather than the one it had"
    );
    // The queued task arrives after the change it was carrying was adopted.
    source.main();
    hub.main();
    assert_eq!(restarts(&source), 0, "a request already adopted asks for no further restart");
    assert_eq!(announced(&source), (2, 1024), "and is announced once, not twice");
}

#[test]
fn a_delay_request_delivered_while_deactivated_waits_for_the_activation() {
    // The host services `on_main_thread` between the deactivation and the
    // activation. That is not a moment the latency may change at, so the
    // request stays pending and the activation ahead of it is the one that
    // adopts and announces it -- there is still no restart to ask for.
    let _scope = crate::test_scope::enter();
    let (mut hub, mut source) = production_pair();
    source.run_format(1536, vec![source.param_event(DELAY_PARAM, 1.0, 1)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    source.run_format(2048, vec![], None, None, 512);
    hub.run_format(2048, vec![], None, None, 512);
    source.deactivate();
    source.main();
    assert_eq!(source.latency(), 512, "a deactivated plugin publishes nothing");
    assert_eq!(announced(&source), (1, 512), "and announces nothing");
    assert_eq!(restarts(&source), 0, "and asks for no restart it is already between");
    hub.reactivate_format(44100.0, 512);
    source.activate_format(44100.0, 512);
    assert_eq!(source.latency(), 1024);
    assert_eq!(
        announced(&source),
        (2, 1024),
        "the activation that follows is the one that moves the value and tells the host"
    );
}

#[test]
fn the_last_delay_request_is_the_one_the_activation_announces() {
    // A second edit while the restart answering the first is still outstanding.
    let _scope = crate::test_scope::enter();
    let (mut hub, mut source) = production_pair();
    for (raw, value) in [(1536, 1.0), (2560, 2.0)] {
        source.run_format(raw, vec![source.param_event(DELAY_PARAM, value, 1)], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        source.run_format(raw + 512, vec![], None, None, 512);
        hub.run_format(raw + 512, vec![], None, None, 512);
        source.main();
        hub.main();
    }
    assert_eq!(restarts(&source), 2, "each request asks the host for the restart that adopts it");
    assert_eq!(announced(&source), (1, 512), "and neither moves what the host is compensating for");
    hub.reactivate_format(44100.0, 512);
    source.reactivate_format(44100.0, 512);
    assert_eq!(source.latency(), 1536, "the activation adopts the last request, not the first");
    assert_eq!(announced(&source), (2, 1536), "announced once, at the value that was adopted");
    // Down to 1x and back to 3x inside one activation. The second edit leaves
    // the published number where it already is, so the restart it would have
    // asked for has nothing left to adopt.
    for (raw, value) in [(3584, 0.0), (4096, 2.0)] {
        source.run_format(raw, vec![source.param_event(DELAY_PARAM, value, 1)], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        source.main();
        hub.main();
    }
    assert_eq!(
        restarts(&source),
        3,
        "the edit away asked for a restart; the edit back asks for none"
    );
    assert_eq!(announced(&source), (2, 1536), "with nothing to adopt, nothing is announced");
    hub.reactivate_format(44100.0, 512);
    source.reactivate_format(44100.0, 512);
    assert_eq!(source.latency(), 1536);
    assert_eq!(announced(&source), (2, 1536), "including across the activation that answers it");
}

/// Release what a fixture left sounding and run the pair until the Tune owns
/// nothing, so the process registry it shares with every other fixture is
/// empty when both are dropped.
fn settle(hub: &Device, source: &Device, mut raw: i64, release: Vec<Input>) {
    source.run_format(raw, release, None, None, 512);
    hub.run_format(raw, vec![], None, None, 512);
    for _ in 0..32 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    source.main();
    hub.main();
    assert_eq!(source.source_snapshot().held, 0);
}

/// While Participating the Tune owns pitch and overrides what the player
/// sends. Off owns nothing: the bend and the per-note tuning are the
/// player's and reach the instrument as they were written.
#[test]
fn production_off_forwards_the_players_bend_and_per_note_tuning_unchanged() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let mut actual = Vec::new();
    let mut run = |raw, inputs| {
        let sink = source.run_format(raw, inputs, None, None, 512);
        actual.extend(
            sink.values.into_iter().map(|(offset, event)| (raw + i64::from(offset), event)),
        );
    };
    // A quarter-tone-up bend and a quarter-semitone per-note tuning, played
    // twice against the same key: once Participating, once Off.
    let bend = |time| raw_midi([0xe0, 0x00, 0x60], time);
    run(1536, vec![bend(0), note(1, 0, 64, 1, true), expression(1, 0.25, 2)]);
    hub.run_format(1536, vec![], None, None, 512);
    for raw in [2048, 2560] {
        run(raw, vec![]);
        hub.run_format(raw, vec![], None, None, 512);
    }
    run(3072, vec![note(1, 0, 64, 0, false), source.participation(false, 1)]);
    hub.run_format(3072, vec![], None, None, 512);
    for raw in [3584, 4096] {
        run(raw, vec![]);
        hub.run_format(raw, vec![], None, None, 512);
    }
    run(4608, vec![bend(0), note(2, 0, 64, 1, true), expression(2, 0.25, 2)]);
    hub.run_format(4608, vec![], None, None, 512);
    for raw in [5120, 5632] {
        run(raw, vec![]);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let bends: Vec<_> = actual
        .iter()
        .filter_map(|(_, event)| match event {
            Event::Midi { data: [status, lsb, msb], .. } if status & 0xf0 == 0xe0 => {
                Some(u16::from(*lsb) | (u16::from(*msb) << 7))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        bends,
        [0x2000, 0x3000],
        "Participating centres the player's bend; Off passes the same bend through"
    );
    let tuning = |id| {
        actual
            .iter()
            .filter_map(|(_, event)| match event {
                Event::Expression { kind: 2, id: found, value, .. } if *found == id => Some(*value),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let participating = tuning(1);
    assert_eq!(participating.len(), 2, "an adaptive onset states its tuning, then the override");
    assert_ne!(participating[0], 0.0, "the assignment the Tune chose, not the player's 0.25");
    assert_eq!(
        participating[1], participating[0],
        "the player's per-note tuning is zeroed and carries only the assignment"
    );
    assert_eq!(
        tuning(2),
        [0.25],
        "Off states no tuning of its own and forwards the player's exactly"
    );
    assert_eq!(source.source_snapshot().faults, 0);
    settle(&hub, &source, 6144, vec![note(2, 0, 64, 0, false)]);
}

/// The control is not host bypass. Both modes play at the delay this
/// activation adopted, and toggling asks the host for nothing.
#[test]
fn production_a_participation_toggle_keeps_the_delay_the_activation_adopted() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let mut onsets = Vec::new();
    let mut run = |raw: i64, inputs| {
        let sink = source.run_format(raw, inputs, None, None, 512);
        onsets.extend(
            sink.values
                .into_iter()
                .filter(|(_, event)| event.attack().is_some())
                .map(|(offset, _)| raw + i64::from(offset)),
        );
    };
    let before = (source.latency(), restarts(&source), announced(&source));
    run(1536, vec![note(1, 0, 64, 0, true)]);
    hub.run_format(1536, vec![], None, None, 512);
    for raw in [2048, 2560] {
        run(raw, vec![]);
        hub.run_format(raw, vec![], None, None, 512);
    }
    run(3072, vec![note(1, 0, 64, 0, false), source.participation(false, 1)]);
    hub.run_format(3072, vec![], None, None, 512);
    for raw in [3584, 4096] {
        run(raw, vec![]);
        hub.run_format(raw, vec![], None, None, 512);
    }
    run(4608, vec![note(2, 0, 64, 0, true)]);
    hub.run_format(4608, vec![], None, None, 512);
    for raw in [5120, 5632] {
        run(raw, vec![]);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        onsets,
        [1536 + 512, 4608 + 512],
        "each mode plays its note at its own input plus the one adopted D"
    );
    assert_eq!(
        (source.latency(), restarts(&source), announced(&source)),
        before,
        "a toggle is not a latency change and asks the host for no reactivation"
    );
    assert_eq!(source.source_snapshot().faults, 0);
    settle(&hub, &source, 6144, vec![note(2, 0, 64, 0, false)]);
}

/// A toggle owes the termination of what it has already forwarded, and it
/// keeps owning those voices until the host has actually taken the release.
/// A rejected release is retried; it is never a note the instrument keeps
/// sounding after the Tune has forgotten it.
#[test]
fn production_a_toggle_owns_its_forwarded_note_until_the_release_is_accepted() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    source.run_format(1536, vec![note(1, 0, 64, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    for raw in [2048, 2560] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(source.source_snapshot().held, 1, "the fixture must reach a forwarded note");
    // The toggle's termination is refused by the host for one whole callback.
    let refused =
        source.run_format(3072, vec![source.participation(false, 0)], Some(u16::MAX), None, 512);
    hub.run_format(3072, vec![], None, None, 512);
    assert!(
        refused.rejected.iter().any(|(_, event)| matches!(event, Event::Note { kind: 2, .. })),
        "the toggle reached the termination"
    );
    assert!(refused.values.is_empty());
    assert_eq!(
        source.source_snapshot().held,
        1,
        "a refused release does not release the ownership it was owed for"
    );
    let retried = source.run_format(3584, vec![], None, None, 512);
    hub.run_format(3584, vec![], None, None, 512);
    assert!(retried.values.iter().any(|(_, event)| matches!(event, Event::Note { kind: 2, .. })));
    for raw in (4096..6144).step_by(512) {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(source.source_snapshot().held, 0, "and it is given up once the host has taken it");
    assert_eq!(
        source.source_snapshot().faults,
        source::OUTPUT_FAULT,
        "a host that refuses a release the Tune owes is a latched output failure, not a note \
         quietly written off"
    );
    settle(&hub, &source, 6144, vec![]);
}

/// Off leaves the player's bend on the wire. Participating owns pitch again,
/// so returning to it recentres the channels Off actually bent -- otherwise
/// every note the Tune tuned afterwards would sound at that offset.
#[test]
fn production_returning_to_participating_recentres_what_off_bent() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    source.run_format(1536, vec![source.participation(false, 0)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    // The bend is an ordinary shared controller and reaches the wire at its
    // own input plus D, so it has to be a whole D clear of the next toggle.
    source.run_format(2048, vec![raw_midi([0xe0, 0x00, 0x60], 0)], None, None, 512);
    hub.run_format(2048, vec![], None, None, 512);
    let bent = source.run_format(2560, vec![], None, None, 512);
    hub.run_format(2560, vec![], None, None, 512);
    assert!(
        bent.values
            .iter()
            .any(|(_, event)| matches!(event, Event::Midi { data: [0xe0, 0x00, 0x60], .. })),
        "the fixture must actually leave a bend on the wire"
    );
    let back = source.run_format(3072, vec![source.participation(true, 0)], None, None, 512);
    hub.run_format(3072, vec![], None, None, 512);
    assert!(
        back.values
            .iter()
            .any(|(_, event)| matches!(event, Event::Midi { data: [0xe0, 0x00, 0x40], .. })),
        "the boundary recentres it"
    );
    // A channel this Tune never bent owes nothing, so the recentre is not a
    // fixed cost every toggle pays.
    let again = source.run_format(3584, vec![source.participation(false, 0)], None, None, 512);
    hub.run_format(3584, vec![], None, None, 512);
    assert!(
        !again
            .values
            .iter()
            .any(|(_, event)| matches!(event, Event::Midi { data: [0xe0, ..], .. })),
        "a centred channel is already in the new mode's state"
    );
    assert_eq!(source.source_snapshot().faults, 0);
    settle(&hub, &source, 4096, vec![]);
}

/// A restore can move participation and routing together, and the routing half
/// is a reset whose cancel cut covers the participation marker standing in the
/// same queue. That marker never reaches output -- and it carries the only
/// thing that arms the recentre. Losing it leaves the wire holding the bend the
/// Off phrase passed through, under a Tune that owns pitch again, so every note
/// it tunes afterwards sounds at that offset.
#[test]
fn production_a_reset_that_cancels_a_toggle_still_recentres_what_off_bent() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, Calibration { offset: 0 });
    hub.activate_format(44100.0, 512);
    let mut source = Device::new(true);
    source.configure_format(uuid, true, Calibration { offset: 0 });
    source.activate_format(44100.0, 512);
    for raw in [0, 512, 1024] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    source.run_format(1536, vec![source.participation(false, 0)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    source.run_format(2048, vec![raw_midi([0xe0, 0x00, 0x60], 0)], None, None, 512);
    hub.run_format(2048, vec![], None, None, 512);
    let bent = source.run_format(2560, vec![], None, None, 512);
    hub.run_format(2560, vec![], None, None, 512);
    assert!(
        bent.values
            .iter()
            .any(|(_, event)| matches!(event, Event::Midi { data: [0xe0, 0x00, 0x60], .. })),
        "the fixture must actually leave an Off bend on the wire"
    );
    let before = attachment_tests::lease(&source).expect("the fixture must reach a paired Tune");
    // Participation and calibration in one restored state: `Adapter::prepare`
    // makes the routing change a reset, so `apply_setup` captures the toggle's
    // marker and cancels it again inside the same call.
    source.configure_format(uuid, true, Calibration { offset: 64 });
    let mut recentres = 0;
    let mut raw = 3072;
    for _ in 0..16 {
        let sink = source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        source.main();
        hub.main();
        recentres += sink
            .values
            .iter()
            .filter(|(_, event)| matches!(event, Event::Midi { data: [0xe0, 0x00, 0x40], .. }))
            .count();
        raw += 512;
    }
    assert_eq!(
        source.param_value(PARTICIPATING_PARAM),
        1.0,
        "the restore must actually reach the Participating mode that owns pitch"
    );
    assert_ne!(
        attachment_tests::lease(&source).map(|lease| lease.incarnation),
        Some(before.incarnation),
        "and it must actually reach the reset: nothing armed this at the marker, so it \
         has to survive the whole withdraw and re-adopt the routing change puts between \
         the toggle and the next moment output is allowed"
    );
    assert_eq!(recentres, 1, "the toggle still owes the wire its centre, exactly once");
    assert_eq!(source.source_snapshot().faults, 0);
    settle(&hub, &source, raw, vec![]);
}

/// Lateness is not a mode change. A note whose assignment misses its deadline
/// takes the late-playback path and nothing else: the phrase already sounding
/// keeps sounding, no release or controller cleanup goes out, and the note
/// itself arrives tuned rather than untuned or cancelled.
#[test]
fn production_a_missed_deadline_is_not_a_participation_toggle() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let mut actual = Vec::new();
    {
        let mut run = |raw: i64, inputs| {
            let sink = source.run_format(raw, inputs, None, None, 512);
            actual.extend(
                sink.values.into_iter().map(|(offset, event)| (raw + i64::from(offset), event)),
            );
        };
        // The Hub runs one whole callback behind, so neither onset can be
        // assigned by its own input+D and both take the late path.
        run(1536, vec![note(1, 0, 64, 0, true)]);
        run(2048, vec![]);
        hub.run_format(1536, vec![], None, None, 512);
        run(2560, vec![]);
        hub.run_format(2048, vec![], None, None, 512);
        run(3072, vec![note(2, 0, 67, 0, true)]);
        hub.run_format(2560, vec![], None, None, 512);
        run(3584, vec![]);
        hub.run_format(3072, vec![], None, None, 512);
        run(4096, vec![]);
        hub.run_format(3584, vec![], None, None, 512);
    }
    assert_eq!(
        source.shared().deadline_misses.load(Ordering::Acquire),
        2,
        "the fixture must actually miss both deadlines"
    );
    let snapshot = source.source_snapshot();
    assert_eq!(
        (snapshot.held, snapshot.emergency, snapshot.faults),
        (2, 0, 0),
        "a deadline miss cancels nothing, terminates nothing and latches nothing"
    );
    assert!(
        !actual.iter().any(|(_, event)| event.release()
            || matches!(event, Event::Note { kind: 2, .. })
            || matches!(event, Event::Midi { data: [0xb0..=0xbf, 64 | 66 | 69, _], .. })
            || matches!(event, Event::Midi { data: [0xe0..=0xef, ..], .. })),
        "no release, pedal neutralization or recentre is owed for lateness"
    );
    assert!(inspect_source(&source, |source| source.participating));
    let onsets: Vec<_> = actual
        .iter()
        .filter_map(|(sample, event)| event.attack().map(|(id, ..)| (*sample, id)))
        .collect();
    assert_eq!(
        onsets,
        [(2560, 1), (4096, 2)],
        "each retained attack plays late on its own schedule rather than being dropped"
    );
    for id in [1, 2] {
        assert!(
            actual.iter().any(|(_, event)| matches!(
                event,
                Event::Expression { kind: 2, id: found, value, .. }
                    if *found == id && *value != 0.0
            )),
            "and it arrives tuned"
        );
    }
    settle(&hub, &source, 4608, vec![note(1, 0, 64, 0, false), note(2, 0, 67, 0, false)]);
}

/// A `Control` carries the lease incarnation and session epoch it was minted
/// under, and a reset ends both. #712 makes "old replies must not revive
/// notes in the new session" a hard constraint, and this is the one test
/// every arm of the Hub's control lane is written in.
///
/// `Detach` is the arm with nothing else in its guard, so what reaches the
/// row is exactly what the identity test let through.
#[test]
fn production_an_obsolete_control_cannot_act_on_the_row_that_replaced_it() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let calibration = Calibration { offset: 0 };
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, calibration);
    hub.activate_format(44100.0, 512);
    let mut source = Device::new(true);
    source.configure_format(uuid, true, calibration);
    source.activate_format(44100.0, 512);
    for raw in (0..3072).step_by(512) {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let session = registry::global().lock().unwrap().test_session(uuid);
    let lease = attachment_tests::lease(&source).expect("the fixture must reach a paired row");
    let slot = usize::from(lease.slot) - 1;
    let held = inspect_hub(&hub, |hub| hub.test_row_identity(slot));
    assert_eq!(held.lease, Some(lease));
    assert_eq!(held.detach, None);
    // The Tune's own Progress owns the normal cell between callbacks. Take it
    // the way the Hub would, so each injected message is the only one there.
    let deliver = |control| {
        session.rows[slot].to_hub.take();
        assert!(session.rows[slot].to_hub.publish(control).is_ok());
        hub.run_format(3072, vec![], None, None, 512);
        inspect_hub(&hub, |hub| hub.test_row_identity(slot)).detach
    };
    assert_eq!(
        deliver(protocol::Control::Detach {
            incarnation: lease.incarnation - 1,
            epoch: held.epoch,
            cut: 7,
        }),
        None,
        "a message from the lease this row replaced is not this row's"
    );
    assert_eq!(
        deliver(protocol::Control::Detach {
            incarnation: lease.incarnation,
            epoch: held.epoch + 1,
            cut: 8,
        }),
        None,
        "nor is one minted under a session epoch this row does not hold"
    );
    assert_eq!(
        deliver(protocol::Control::Detach {
            incarnation: lease.incarnation,
            epoch: held.epoch,
            cut: 9,
        }),
        Some(9),
        "the fixture must reach the arm at all, or neither refusal above means anything"
    );
}

/// A row's slot outlives the lease that used it, and the Hub sequences by
/// slot. What the previous Tune left there -- its participation, its serial,
/// its key preferences -- belongs to that lease, so a fresh one must not
/// inherit it: a Participating Tune that took an Off Tune's slot would be
/// scored as Off, contributing nothing to anyone's tuning context including
/// its own, with nothing on screen saying so.
#[test]
fn production_a_fresh_lease_is_not_sequenced_as_the_off_tune_whose_slot_it_took() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let calibration = Calibration { offset: 0 };
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, calibration);
    hub.activate_format(44100.0, 512);
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let mut raw = 0;
    let idle = |hub: &Device, tune: Option<&Device>, raw: &mut i64, blocks: usize| {
        for _ in 0..blocks {
            if let Some(tune) = tune {
                tune.run_format(*raw, vec![], None, None, 512);
            }
            hub.run_format(*raw, vec![], None, None, 512);
            *raw += 512;
        }
    };
    let slot = {
        let mut off = Device::new(true);
        off.configure_format(uuid, true, calibration);
        off.activate_format(44100.0, 512);
        idle(&hub, Some(&off), &mut raw, 3);
        let lease = attachment_tests::lease(&off).expect("the first Tune must pair");
        off.run_format(raw, vec![off.participation(false, 0)], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        raw += 512;
        idle(&hub, Some(&off), &mut raw, 3);
        let slot = usize::from(lease.slot) - 1;
        let identity = inspect_hub(&hub, |hub| hub.test_row_identity(slot));
        assert!(
            !identity.participating && identity.participation_serial != 0,
            "the fixture must reach an Off row the Hub has actually sequenced"
        );
        slot
    };
    // The Off Tune is gone. Its row settles and its slot goes back.
    for _ in 0..16 {
        hub.main();
        idle(&hub, None, &mut raw, 1);
    }
    hub.main();
    let mut back = Device::new(true);
    back.configure_format(uuid, true, calibration);
    back.activate_format(44100.0, 512);
    idle(&hub, Some(&back), &mut raw, 4);
    let lease = attachment_tests::lease(&back).expect("the second Tune must pair");
    assert_eq!(usize::from(lease.slot) - 1, slot, "the fixture must reach slot reuse");
    let identity = inspect_hub(&hub, |hub| hub.test_row_identity(slot));
    assert_eq!(
        (identity.participating, identity.participation_serial),
        (true, 0),
        "the fresh lease is sequenced as what it is, not as what the slot last held"
    );
    // And the musical consequence: this Tune's own held D is context for its
    // own E. Sequenced as Off it would score against nothing and pick 5/4.
    back.run_format(raw, vec![note(1, 0, 50, 0, true)], None, None, 512);
    hub.run_format(raw, vec![], None, None, 512);
    raw += 512;
    idle(&hub, Some(&back), &mut raw, 3);
    back.run_format(raw, vec![note(2, 0, 52, 0, true)], None, None, 512);
    hub.run_format(raw, vec![], None, None, 512);
    raw += 512;
    idle(&hub, Some(&back), &mut raw, 3);
    assert_eq!(
        inspect_source(&back, |source| source
            .state
            .voices()
            .find(|voice| voice.note == 52)
            .unwrap()
            .attack_node),
        Some(harmonigraph_core::LatticePos::new(4, 0, 0)),
        "the held D is in the context this E was scored against"
    );
    assert_eq!(back.source_snapshot().faults, 0);
    settle(&hub, &back, raw, vec![note(1, 0, 50, 0, false), note(2, 0, 52, 0, false)]);
}

/// Hold the Tune one callback ahead of its Hub for the whole run, so no reply
/// can reach an onset before that onset's own input plus D. Everything the
/// Off path is supposed to do without the Hub is measured under this lag.
fn lead(
    hub: &Device,
    source: &Device,
    raw: &mut i64,
    behind: &mut i64,
    inputs: Vec<Input>,
) -> Vec<(i64, Event)> {
    let sink = source.run_format(*raw, inputs, None, None, 512);
    let emitted =
        sink.values.into_iter().map(|(offset, event)| (*raw + i64::from(offset), event)).collect();
    hub.run_format(*behind, vec![], None, None, 512);
    *raw += 512;
    *behind += 512;
    emitted
}

/// Go Off, settle that toggle, then take the lead. Returns the Tune's next
/// input sample and the Hub's, which trails it by one callback.
fn off_and_leading(hub: &Device, source: &Device, mut raw: i64) -> (i64, i64) {
    source.run_format(raw, vec![source.participation(false, 0)], None, None, 512);
    hub.run_format(raw, vec![], None, None, 512);
    raw += 512;
    for _ in 0..4 {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        raw += 512;
    }
    source.run_format(raw, vec![], None, None, 512);
    (raw + 512, raw)
}

/// #712: "Off note processing should not require Hub assignment replies or
/// adaptive scheduling." So the lag that makes a Participating onset late
/// leaves an Off onset exactly on time — it was never waiting for anything —
/// while D itself is untouched, because this control is not host bypass.
#[test]
fn production_an_off_note_emits_at_input_plus_d_without_an_assignment_reply() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let (mut raw, mut behind) = off_and_leading(&hub, &source, 1536);
    let mut onsets = Vec::new();
    let collect = |emitted: Vec<(i64, Event)>, onsets: &mut Vec<i64>| {
        onsets.extend(
            emitted
                .into_iter()
                .filter(|(_, event)| event.attack().is_some())
                .map(|(sample, _)| sample),
        );
    };
    let off = raw;
    let emitted = lead(&hub, &source, &mut raw, &mut behind, vec![note(1, 0, 64, 0, true)]);
    collect(emitted, &mut onsets);
    let emitted = lead(&hub, &source, &mut raw, &mut behind, vec![note(1, 0, 64, 0, false)]);
    collect(emitted, &mut onsets);
    assert_eq!(onsets, [off + 512], "an Off onset plays at its own input plus D and no later");
    assert_eq!(
        source.shared().deadline_misses.load(Ordering::Acquire),
        0,
        "and misses no deadline, because it had none to wait for"
    );
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_cohort_delivery().0),
        0,
        "the Hub spent no decision on it: it was never asked"
    );
    // The same lag, the other mode. A Participating onset does wait.
    let emitted = lead(&hub, &source, &mut raw, &mut behind, vec![source.participation(true, 0)]);
    collect(emitted, &mut onsets);
    for _ in 0..3 {
        let emitted = lead(&hub, &source, &mut raw, &mut behind, vec![]);
        collect(emitted, &mut onsets);
    }
    let participating = raw;
    let emitted = lead(&hub, &source, &mut raw, &mut behind, vec![note(2, 0, 64, 0, true)]);
    collect(emitted, &mut onsets);
    for _ in 0..2 {
        let emitted = lead(&hub, &source, &mut raw, &mut behind, vec![]);
        collect(emitted, &mut onsets);
    }
    assert_eq!(
        onsets,
        [off + 512, participating + 1024],
        "the fixture must actually deny the reply, or the Off onset's punctuality means nothing"
    );
    assert_eq!(source.shared().deadline_misses.load(Ordering::Acquire), 1);
    assert_eq!(source.source_snapshot().faults, 0);
    hub.run_format(behind, vec![], None, None, 512);
    settle(&hub, &source, raw, vec![note(2, 0, 64, 0, false)]);
}

/// The other half of the same decision, and the one that goes quietly wrong:
/// an onset that emits without an assignment reports decision zero, which
/// matches no plan, so a plan minted for it is never retired. One `LIFETIMES`
/// slot per Off note — and the Tune's free list hands the same request slot
/// straight back, so the SECOND note finds a stranger's plan in its own slot,
/// which is `configuration_exhausted` and a latched STORAGE_FAULT.
///
/// The run below is twelve times longer than the leak needs to latch, and the
/// chord holds eight distinct request slots open at once.
#[test]
fn production_off_notes_take_no_plan_slot_and_never_exhaust_the_ledger() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let (mut raw, mut behind) = off_and_leading(&hub, &source, 1536);
    let mut attacks = 0;
    let ledger = std::cell::Cell::new(0);
    let slots = std::cell::Cell::new(0);
    let play = |inputs, raw: &mut i64, behind: &mut i64| {
        let emitted = lead(&hub, &source, raw, behind, inputs);
        ledger.set(ledger.get().max(inspect_hub(&hub, |hub| hub.test_plan_count())));
        slots.set(slots.get().max(source.source_snapshot().lives));
        emitted.iter().filter(|(_, event)| event.attack().is_some()).count()
    };
    for index in 0..24 {
        let id = index + 1;
        attacks += play(vec![note(id, 0, 64, 0, true)], &mut raw, &mut behind);
        attacks += play(vec![note(id, 0, 64, 0, false)], &mut raw, &mut behind);
        attacks += play(vec![], &mut raw, &mut behind);
        attacks += play(vec![], &mut raw, &mut behind);
    }
    assert_eq!(ledger.get(), 0, "not one of the 24 took a plan slot");
    assert_eq!(slots.get(), 1, "and one slot served them all, so the Hub saw that one 24 times");
    let voices =
        |on| (0..8).map(|voice| note(100 + voice, 0, 60 + voice as i16, 0, on)).collect::<Vec<_>>();
    attacks += play(voices(true), &mut raw, &mut behind);
    attacks += play(vec![], &mut raw, &mut behind);
    attacks += play(voices(false), &mut raw, &mut behind);
    for _ in 0..4 {
        attacks += play(vec![], &mut raw, &mut behind);
    }
    assert_eq!(slots.get(), 8, "and the chord held eight distinct ones at once");
    assert_eq!(attacks, 32, "every note reached the wire");
    // The other way a slot could be taken: a cancelled attack. The Hub holds a
    // placeholder plan for one until the onset's own record retires it, and an
    // Off onset's record retires nothing — so an Off attack must not report
    // itself cancelled either.
    let stopped = || {
        let Input::Transport(mut value) = transport(0, 120.0) else { unreachable!() };
        value.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        Input::Transport(value)
    };
    play(vec![transport(0, 120.0)], &mut raw, &mut behind);
    attacks += play(vec![note(200, 0, 64, 0, true), stopped()], &mut raw, &mut behind);
    for _ in 0..4 {
        attacks += play(vec![], &mut raw, &mut behind);
    }
    assert_eq!(attacks, 32, "the Stop must actually cancel that attack before it sounds");
    assert_eq!(ledger.get(), 0, "and not one of them took a plan slot");
    assert_eq!(
        (source.source_snapshot().faults, inspect_hub(&hub, |hub| hub.test_cohort_delivery().0)),
        (0, 0),
        "nothing was exhausted and no decision was spent"
    );
    hub.run_format(behind, vec![], None, None, 512);
    settle(&hub, &source, raw, vec![]);
}

/// A paired Hub and Tune, then a healthy DIRECT reanchor with a key held down
/// through it. Returns the raw time to play from and how far the run got.
fn reanchored(hub: &Device, source: &Device, uuid: SavedUuid, held: Vec<Input>) -> i64 {
    musical_tests::configure(hub, harmonigraph_core::Tuning::just());
    source.run_format(1536, vec![], None, None, 512);
    hub.run_format(1536, held, None, None, 512);
    let mut raw = 2048;
    for _ in 0..2 {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        raw += 512;
    }
    hub.shared()
        .apply(
            setup::Routing::Hub(HubSetup { uuid, calibration: Calibration { offset: 64 } }),
            false,
        )
        .unwrap();
    // The transition, then the Tune's own withdraw and re-adopt behind it.
    for _ in 0..96 {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        source.main();
        hub.main();
        raw += 512;
    }
    assert_eq!(
        inspect_hub(hub, |hub| hub.direct.test_snapshot().epoch),
        2,
        "the fixture must reach a committed healthy boundary"
    );
    raw
}

/// The observation runs a whole callback ahead of the merge: the wrapper walks
/// every input through `clap_configuration_observe` before performance sees any
/// of it. Reconciling the cells a boundary carried against that state answers
/// with the end of the callback, so a release at its tail retires the voice for
/// onsets at its head -- and an assignment, once made, is frozen.
#[test]
fn production_a_carried_direct_voice_is_context_for_the_onset_before_its_release() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, Calibration { offset: 0 });
    hub.activate_format(44100.0, 512);
    let mut source = Device::new(true);
    source.configure_format(uuid, true, Calibration { offset: 0 });
    source.activate_format(44100.0, 512);
    for raw in [0, 512, 1024] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let mut raw = reanchored(&hub, &source, uuid, vec![note(31, 0, 50, 1, true)]);
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context()),
        [(0, 1)],
        "the fixture must reach a carried DIRECT voice: forwarding released this D at \
         the boundary, but the player is still holding the key"
    );
    let before = inspect_hub(&hub, |hub| hub.test_policy_counts());
    // One callback: the Tune's E at its head, the carried D's own release 400
    // samples later.
    source.run_format(raw, vec![note(7, 0, 52, 0, true)], None, None, 512);
    hub.run_format(raw, vec![note(31, 0, 50, 400, false)], None, None, 512);
    raw += 512;
    let after = inspect_hub(&hub, |hub| hub.test_policy_counts());
    assert_eq!(
        (after[0] - before[0], after[1] - before[1]),
        (1, 1),
        "exactly one assignment, and the D was still sounding at the sample it was made"
    );
    for _ in 0..6 {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        raw += 512;
    }
    assert_eq!(
        inspect_source(&source, |s| s
            .state
            .voices()
            .find(|voice| voice.note == 52)
            .and_then(|voice| voice.attack_node)),
        Some(harmonigraph_core::LatticePos::new(-4, -1, 0)),
        "and the musical consequence: scored against the held D. Read against a context \
         the release had already emptied, this E is the plain 5/4 (0, 1, 0)"
    );
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context()),
        [(1, 1)],
        "the release still lands: the observation retires its own cell, one sample later \
         rather than one callback earlier"
    );
    assert_eq!(source.source_snapshot().faults, 0);
    settle(&hub, &source, raw, vec![note(7, 0, 52, 0, false)]);
}

/// The other half of the merge rule the replay has to obey: inside one sample
/// every release and controller lands before any onset. The carried voice's own
/// release is a release, so an onset at exactly its sample is scored without it.
#[test]
fn production_a_carried_direct_release_applies_before_the_onset_at_its_own_sample() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, Calibration { offset: 0 });
    hub.activate_format(44100.0, 512);
    let mut source = Device::new(true);
    source.configure_format(uuid, true, Calibration { offset: 0 });
    source.activate_format(44100.0, 512);
    for raw in [0, 512, 1024] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let mut raw = reanchored(&hub, &source, uuid, vec![note(31, 0, 50, 1, true)]);
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context()),
        [(0, 1)],
        "the fixture must reach a carried DIRECT voice"
    );
    let before = inspect_hub(&hub, |hub| hub.test_policy_counts());
    // The boundary moved the Hub's calibration to +64, so a DIRECT event at
    // this callback's head and a Tune event 64 frames in are the same sample.
    source.run_format(raw, vec![note(7, 0, 52, 64, true)], None, None, 512);
    hub.run_format(raw, vec![note(31, 0, 50, 0, false)], None, None, 512);
    raw += 512;
    let after = inspect_hub(&hub, |hub| hub.test_policy_counts());
    assert_eq!(
        (after[0] - before[0], after[1] - before[1]),
        (1, 0),
        "one assignment, scored against nothing: the release is at its sample too"
    );
    for _ in 0..6 {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        raw += 512;
    }
    assert_eq!(
        inspect_source(&source, |s| s
            .state
            .voices()
            .find(|voice| voice.note == 52)
            .and_then(|voice| voice.attack_node)),
        Some(harmonigraph_core::LatticePos::new(0, 1, 0)),
        "the plain 5/4, not the (-4, -1, 0) the same E takes when the D outlives it"
    );
    assert_eq!(source.source_snapshot().faults, 0);
    settle(&hub, &source, raw, vec![note(7, 0, 52, 0, false)]);
}

/// A healthy transition waits across callbacks for the paired rows to settle,
/// and the player can strike a key inside that window. The fence never ended
/// that note: its onset record is retained across the closing and arrives under
/// the new session carrying its own identity. Carrying the observation of it as
/// well would give one physical note two owners -- two context slots, and two
/// votes in every assignment after it.
#[test]
fn production_a_direct_key_struck_during_a_transition_takes_one_context_cell() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, Calibration { offset: 0 });
    hub.activate_format(44100.0, 512);
    let mut source = Device::new(true);
    source.configure_format(uuid, true, Calibration { offset: 0 });
    source.activate_format(44100.0, 512);
    for raw in [0, 512, 1024] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let mut raw = 1536;
    hub.shared()
        .apply(
            setup::Routing::Hub(HubSetup { uuid, calibration: Calibration { offset: 64 } }),
            false,
        )
        .unwrap();
    for _ in 0..2 {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        source.main();
        hub.main();
        raw += 512;
    }
    assert_eq!(
        inspect_hub(&hub, |hub| hub.direct.test_snapshot().epoch),
        1,
        "the fixture must strike the key while the transition is still waiting"
    );
    source.run_format(raw, vec![], None, None, 512);
    hub.run_format(raw, vec![note(41, 0, 67, 3, true)], None, None, 512);
    source.main();
    hub.main();
    raw += 512;
    let mut retained = false;
    for _ in 0..96 {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        source.main();
        hub.main();
        raw += 512;
        retained |= inspect_hub(&hub, |hub| {
            hub.test_inputs(0).iter().any(|record| record.onset() && record.lifetime == 1)
        });
        assert!(
            inspect_hub(&hub, |hub| hub.test_context().len()) <= 1,
            "one physical note, one context cell, at every point in the transition"
        );
    }
    assert_eq!(
        inspect_hub(&hub, |hub| hub.direct.test_snapshot().epoch),
        2,
        "the fixture must reach the committed boundary the carry runs at"
    );
    assert!(
        retained,
        "and it must reach the case that makes the two owners possible: the capture \
         stream still held this note's onset record across the closing"
    );
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context()),
        [(0, 1)],
        "the capture stream owns it, because the fence never took it away from there"
    );
    assert_eq!(source.source_snapshot().faults, 0);
    source.run_format(raw, vec![], None, None, 512);
    hub.run_format(raw, vec![note(41, 0, 67, 0, false)], None, None, 512);
    raw += 512;
    settle(&hub, &source, raw, vec![]);
    assert!(
        inspect_hub(&hub, |hub| hub.test_context()).is_empty(),
        "and its own release is what takes it out again"
    );
}

/// Striking a carried key again is a change to a fenced voice that the event's
/// own target set does not name: `State::apply` overwrites the same
/// channel/key slot, and the attack's lifetime is above the fence. The
/// displaced lifetime is retired, so the replay owes the merge that retirement
/// at this sample like any other change the observation makes.
#[test]
fn production_a_restruck_direct_key_retires_the_carried_cell_it_replaced() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, Calibration { offset: 0 });
    hub.activate_format(44100.0, 512);
    let mut source = Device::new(true);
    source.configure_format(uuid, true, Calibration { offset: 0 });
    source.activate_format(44100.0, 512);
    for raw in [0, 512, 1024] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let mut raw = reanchored(&hub, &source, uuid, vec![note(31, 0, 50, 1, true)]);
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context()),
        [(0, 1)],
        "the fixture must reach a carried DIRECT voice"
    );
    // The player strikes the same key again without having let go of it.
    source.run_format(raw, vec![], None, None, 512);
    hub.run_format(raw, vec![note(32, 0, 50, 8, true)], None, None, 512);
    raw += 512;
    for _ in 0..6 {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        raw += 512;
    }
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context()).len(),
        1,
        "one physical key, one context cell: the capture stream owns the replacement, \
         and the observation's cell for the lifetime it displaced is gone"
    );
    source.run_format(raw, vec![], None, None, 512);
    hub.run_format(raw, vec![note(32, 0, 50, 0, false)], None, None, 512);
    raw += 512;
    settle(&hub, &source, raw, vec![]);
    assert!(
        inspect_hub(&hub, |hub| hub.test_context()).is_empty(),
        "and the release of the replacement empties the context: a cell the replacement \
         never owned would outlive it and score every assignment afterwards"
    );
    assert_eq!(source.source_snapshot().faults, 0);
}

/// Sixty-four carried keys and a burst of wildcard tuning expressions: each
/// expression addresses every one of them, so seventeen of them are 1,088
/// changes owed to a 1,024-cell replay. Forwarding retired those lifetimes at
/// the boundary, so the same seventeen events are seventeen unaddressed
/// captures -- the capture queue of the same length is nowhere near full, and
/// nothing else stands between this and losing the replay.
#[test]
fn production_a_lost_direct_replay_is_a_fault_at_the_sample_it_was_lost_at() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, Calibration { offset: 0 });
    hub.activate_format(44100.0, 512);
    let mut source = Device::new(true);
    source.configure_format(uuid, true, Calibration { offset: 0 });
    source.activate_format(44100.0, 512);
    for raw in [0, 512, 1024] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    let held = (0..64)
        .map(|index| note(100 + index, 0, 24 + index as i16, 1 + index as u32, true))
        .collect();
    let raw = reanchored(&hub, &source, uuid, held);
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_context().len()),
        64,
        "the fixture must carry a full source of DIRECT voices"
    );
    let before = inspect_hub(&hub, |hub| hub.test_policy_counts());
    // The Tune's onset at the callback's head; the burst 400 frames later, in
    // the Hub's own numbering, which the boundary moved to +64.
    source.run_format(raw, vec![note(7, 0, 52, 64, true)], None, None, 512);
    hub.run_format(raw, (0..17).map(|_| expression(-1, 0.02, 400)).collect(), None, None, 512);
    let after = inspect_hub(&hub, |hub| hub.test_policy_counts());
    assert_eq!(
        (after[0] - before[0], after[1] - before[1]),
        (1, 64),
        "one assignment, scored against every carried key: it stands 400 samples before          the burst, and a surrender applied at the front this pass reached instead of at          the sample it happened would take them all out from under it"
    );
    assert!(
        inspect_hub(&hub, |hub| hub.test_context().iter().all(|(source, _)| *source != 0)),
        "and past that sample they are gone: the observation cannot say when a carried          cell changes any more, so it stops owning one"
    );
    assert_ne!(
        inspect_hub(&hub, |hub| hub.direct.test_snapshot().faults) & source::STORAGE_FAULT,
        0,
        "audibly: a bounded store that could not hold what it was given latches, rather          than every assignment after this scoring against a context missing the keys the          player is still holding"
    );
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}
