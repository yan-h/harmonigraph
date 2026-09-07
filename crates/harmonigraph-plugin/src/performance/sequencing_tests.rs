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
    for serial in 1..=4 {
        let original = inspect_source(&source, |source| source.test_capture(serial).unwrap());
        assert_eq!(original.work_count, 0);
        assert!(original.remote_pending, "every original was captured before retirement");
    }
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
    assert_eq!(inspect_hub(&hub, |hub| hub.test_capture_phases(1)), (0, 0));
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
fn production_capture_status_retires_a_full_window_behind_younger_actual_output() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    source.run_format(1536, vec![note(7, 0, 64, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    let onset = source.run_format(2048, vec![], None, None, 512);
    assert_eq!(onset.values.iter().filter(|(_, event)| event.attack().is_some()).count(), 1);
    hub.run_format(2048, vec![], None, None, 512);
    source.run_format(2560, vec![], None, None, 512);
    hub.run_format(2560, vec![], None, None, 512);
    let mut older = vec![note(11, 1, 60, 0, true)];
    older.extend((0..1023).map(|_| expression(11, 0.25, 0)));
    source.run_format(3072, older, None, None, 512);
    hub.run_format(3072, vec![], None, None, 512);
    let mut b_reports = 0;
    let mut a_onsets = 0;
    let mut a_sample = None;
    let mut raw = 3072;
    for block in 0..16 {
        raw += 512;
        let events = if block < 6 {
            (0..384).map(|offset| expression(7, 0.5, offset)).collect()
        } else {
            vec![]
        };
        let output = source.run_format(raw, events, None, None, 512);
        b_reports += output
            .values
            .iter()
            .filter(|(_, event)| matches!(event, Event::Expression { id: 7, .. }))
            .count();
        a_onsets += output
            .values
            .iter()
            .filter(|(_, event)| {
                matches!(event, Event::Note { id: 11, kind: CLAP_EVENT_NOTE_ON, .. })
            })
            .count();
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        b_reports, 2304,
        "younger actual output exceeds the entire 2048-cell owned output window"
    );
    assert_eq!(a_onsets, 0, "the older cohort still owns its bounded structural cursor");
    assert_eq!(inspect_hub(&hub, |hub| hub.test_capture_phases(1)), (1024, 0));
    assert!(source.source_snapshot().captures > 1024);
    assert!(
        inspect_hub(&hub, |hub| hub.service_position().1[0]) >= 2306,
        "canonical B publication proceeds while its own capture remains behind full A ingress"
    );
    for _ in 0..8192 {
        raw += 512;
        let output = source.run_format(raw, vec![], None, None, 512);
        a_onsets += output
            .values
            .iter()
            .filter(|(_, event)| {
                matches!(event, Event::Note { id: 11, kind: CLAP_EVENT_NOTE_ON, .. })
            })
            .count();
        if let Some((offset, _)) = output.values.iter().find(|(_, event)| {
            matches!(event, Event::Note { id: 11, kind: CLAP_EVENT_NOTE_ON, .. })
        }) {
            a_sample = Some(raw + i64::from(*offset));
        }
        assert!(
            !output.values.iter().any(|(_, event)| matches!(
                event,
                Event::Note { id: 7, kind: CLAP_EVENT_NOTE_ON, .. }
            )),
            "accepted B is never replayed because its capture arrived later"
        );
        hub.run_format(raw, vec![], None, None, 512);
        if source.source_snapshot().captures == 0 {
            break;
        }
    }
    assert_eq!(a_onsets, 1);
    assert_eq!(source.source_snapshot().captures, 0);
    assert_eq!(source.source_snapshot().faults, 0);
    raw += 512;
    let release_due = raw + a_sample.unwrap() - 3072;
    source.run_format(
        raw,
        vec![note(7, 0, 64, 0, false), note(11, 1, 60, 0, false)],
        None,
        None,
        512,
    );
    hub.run_format(raw, vec![], None, None, 512);
    while raw < release_due + 2048 {
        raw += 512;
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
    assert!(inspect_source(&source, |source| source.test_capture(2).unwrap().adaptive));
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
fn production_crossed_status_query_and_disposition_free_each_others_reply_lane() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    let hub_wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    let source_wrapper = unsafe {
        &*((*source.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    hub_wrapper
        .test_with_plugin(|plugin| plugin.aggregation.as_mut().unwrap().test_pause_captures());
    source.run_format(1536, vec![note(7, 0, 60, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    let shared =
        inspect_source(&source, |source| source.offer.as_ref().unwrap().session.rows[0].clone());
    source_wrapper
        .test_with_plugin(|plugin| plugin.source.as_mut().unwrap().test_cancel_before_receive());
    let mut query = None;
    shared.to_source.take_repair_if(|reply| {
        if let protocol::Reply::CaptureStatusQuery(key) = reply {
            query = Some(key);
        }
        false
    });
    let key = query.expect("Hub query owns the reserved return cell");
    let mut disposition = false;
    shared.to_hub.take_repair_if(|control| {
        disposition = matches!(control, protocol::Control::Disposition { .. });
        false
    });
    assert!(disposition, "Source cancellation simultaneously owns the reserved request cell");
    assert_eq!(source.source_snapshot().manifest, 1);
    source.run_format(2048, vec![], None, None, 512);
    assert!(
        shared.to_source.reserve_repair().is_some(),
        "Source retains only the key and frees the crossed ACK cell"
    );
    assert_eq!(
        source.source_snapshot().manifest,
        1,
        "a full response cell retains the exact pending query"
    );
    hub.run_format(2048, vec![], None, None, 512);
    source.run_format(2560, vec![], None, None, 512);
    assert_eq!(source.source_snapshot().manifest, 0);
    let mut completed = false;
    shared.to_hub.take_repair_if(|control| {
        completed = matches!(control, protocol::Control::CaptureStatus { key: found,
            status: Some(protocol::CaptureStatus { output_cut: 0, work_done: 0, inline_done: true }) } if found == key);
        false
    });
    assert!(completed, "the response snapshots completion after the exact cancellation ACK");
    hub.run_format(2560, vec![], None, None, 512);
    assert!(inspect_source(&source, |source| source.test_capture(1).unwrap().remote_pending));
    drop(shared);
    drop(source);
    drop(hub);
    assert_eq!(
        registry::global().lock().unwrap().test_counts(),
        (0, 0, 0),
        "real joined-owner pumps complete the retained capture"
    );
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
fn production_full_capture_window_consumes_following_completeness_without_eviction() {
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
    let events = (0..1024)
        .map(|index| {
            Input::Midi(clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                port_index: 0,
                data: [0xa0, (index % 128) as u8, 37],
            })
        })
        .collect();
    source.run_format(1536, events, None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    let mut full_before_proof = false;
    let mut committed = false;
    // The real 1024-node graph retains its structural-work cursor over many
    // callbacks; this fixture does not enlarge the 4096-unit grant.
    for block in 4..1028 {
        let raw = block * 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        let (counts, (coverage, _, active, finalized)) = inspect_hub(&hub, |hub| {
            (hub.test_capture_phases(1), hub.test_input_sequence_progress())
        });
        if counts.0 == 1024
            && coverage.is_none_or(|(coverage, cut)| coverage.through <= 1536 || cut < 1024)
        {
            assert!(!active, "no complete graph before its input proof");
        }
        full_before_proof |= inspect_hub(&hub, |hub| hub.full_input_control_seen);
        if finalized.is_some_and(|through| through > 1536) {
            assert!(
                full_before_proof,
                "fixture reached the real full owned window before consuming its following proof"
            );
            committed = true;
        }
        assert_eq!(source.source_snapshot().faults, 0);
        if committed && counts == (0, 0) && source.source_snapshot().captures == 0 {
            break;
        }
    }
    assert!(full_before_proof && committed);
    assert_eq!(inspect_hub(&hub, |hub| hub.test_capture_phases(1)), (0, 0));
    assert_eq!(source.source_snapshot().captures, 0);
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
        let replies = &mut hub.offer.as_mut().unwrap().bank.rows[0].replies;
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
