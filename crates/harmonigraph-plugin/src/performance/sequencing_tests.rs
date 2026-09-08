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
fn production_missing_source_interval_retains_128_configuration_markers_then_contains_129() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    let mailbox = wrapper.configuration_handle().unwrap();
    source.run_format(1536, vec![note(7, 0, 60, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    source.run_format(2048, vec![], None, None, 512);
    hub.run_format(2048, vec![], None, None, 512);
    let original =
        inspect_source(&source, |source| source.state.voices().next().unwrap().assignment.unwrap());
    for block in 0..16 {
        let events =
            (0..8).map(|index| tuning_parameter(&hub, 690.0 + index as f32, index)).collect();
        hub.run_format(2560 + block * 512, events, None, None, 512);
        assert_eq!(mailbox.visible().0.status, 0);
        assert!(!mailbox.visible().1);
        let (len, pending, frontier) = wrapper.test_inspect_plugin(|plugin| {
            let timeline = &plugin.configuration.as_ref().unwrap().timeline;
            (timeline.len(), timeline.pending(), timeline.finalized_exclusive())
        });
        assert_eq!(len, (block + 1) as usize * 8);
        assert_eq!(
            pending, 0,
            "the actual required-marker store fills after command ingress drains"
        );
        assert_eq!(frontier, 2560, "missing Source input keeps later markers unretirable");
    }
    let retained = mailbox.visible().0;
    hub.run_format(10752, vec![tuning_parameter(&hub, 710.0, 0)], None, None, 512);
    assert_eq!(mailbox.visible().0.status & 2, 2);
    assert_eq!(mailbox.visible().0.raw, retained.raw);
    assert_eq!(mailbox.visible().0.revision, retained.revision);
    assert_eq!(
        inspect_source(&source, |source| source.state.voices().next().unwrap().assignment),
        Some(original)
    );
    let emergency = source.run_format(2560, vec![], None, None, 512);
    assert_eq!(emergency.values.iter().filter(|(_, event)| event.release()).count(), 1);
    assert_ne!(source.source_snapshot().faults & source::STORAGE_FAULT, 0);
    hub.run_format(11264, vec![], None, None, 512);
    source.run_format(3072, vec![], None, None, 512);
    assert_eq!(source.source_snapshot().held, 0);
    drop(source);
    drop(hub);
    assert_eq!(
        registry::global().lock().unwrap().test_counts(),
        (0, 0, 0),
        "completed Originals pinned by the terminal reader retire after callback join"
    );
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
            use harmonigraph_core::configuration::{
                timeline::ConfigTimeline, ConfigReducer, TuningModes,
            };
            plugin.configuration.as_mut().unwrap().timeline =
                ConfigTimeline::new(ConfigReducer::new(
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
                ));
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
    assert_eq!(attack_ids, [69, 70, 71, 72, 73], "rejoin adds no duplicate attacks");
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
        [-0.11731262, -0.11731262],
        "the held pitch survives Off/rejoin and ignores incoming bends"
    );
    assert_eq!(tuning(70).len(), 1);
    assert_ne!(tuning(70)[0], 0.0, "the pre-Off pending request finishes tuned");
    assert_eq!(tuning(71), [0.0], "only the newly received Off note deliberately uses zero");
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
