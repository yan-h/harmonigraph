//! Full production factory defaults: central input ownership and D512 output.
use super::*;

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
    let calibration =
        Calibration { offset: 0, sample_rate: 44100.0, max_frames: 512, validated: true };
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
fn production_hub_recovery_replays_unsounded_originals_with_copied_configuration_and_duration() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    source.run_format(1536, vec![note(7, 0, 60, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    source.run_format(2048, vec![], None, None, 512);
    hub.run_format(2048, vec![], None, None, 512);
    let held = inspect_source(&source, |source| *source.state.voices().next().unwrap());
    assert_eq!(held.frozen_offset_microcents, 1_000_000);
    source.run_format(
        2560,
        vec![note(8, 1, 64, 0, true), note(8, 1, 64, 20, false)],
        None,
        None,
        512,
    );
    hub.run_format(2560, vec![], None, None, 512);
    let request = inspect_source(&source, |source| source.test_capture(2).unwrap());
    let prior = inspect_hub(&hub, |hub| hub.test_plan_binding(0, request.life).unwrap());
    assert_eq!(prior.decision, 2);
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    wrapper
        .test_with_plugin(|plugin| plugin.aggregation.as_mut().unwrap().test_request_recovery(2));
    let mut actual = Vec::new();
    let mut resumed_binding = None;
    let mut raw = 2560;
    for iteration in 0..160 {
        raw += 512;
        let input = if iteration == 1 { vec![note(9, 2, 67, 0, true)] } else { vec![] };
        let sink = source.run_format(raw, input, None, None, 512);
        for (time, event) in sink.values {
            if event.attack().is_some_and(|(id, ..)| id == 8) {
                resumed_binding =
                    inspect_source(&source, |source| source.test_assignment(request.life));
            }
            actual.push((raw + i64::from(time), event));
        }
        let input = if iteration == 0 { vec![tuning_parameter(&hub, 680.0, 0)] } else { vec![] };
        hub.run_format(raw, input, None, None, 512);
        if actual.iter().filter(|(_, event)| event.attack().is_some()).count() == 2
            && actual.iter().any(|(_, event)| {
                event.release() && matches!(event, event::Event::Note { id: 8, .. })
            })
        {
            break;
        }
    }
    let ons: Vec<_> = actual.iter().filter(|(_, event)| event.attack().is_some()).collect();
    assert_eq!(
        ons.len(),
        2,
        "{}; source {:?}",
        inspect_hub(&hub, |hub| hub.test_recovery_progress()),
        source.source_snapshot()
    );
    assert_eq!(ons.iter().map(|(_, event)| event.attack().unwrap().0).collect::<Vec<_>>(), [8, 9]);
    let on =
        actual.iter().find(|(_, event)| event.attack().is_some_and(|(id, ..)| id == 8)).unwrap().0;
    let off = actual
        .iter()
        .find(|(_, event)| event.release() && matches!(event, event::Event::Note { id: 8, .. }))
        .unwrap()
        .0;
    assert_eq!(off - on, 20, "retained original duration follows the accepted onset shift");
    let rebound = resumed_binding.unwrap();
    assert!(rebound.decision > prior.decision && rebound.emission > prior.emission);
    assert_eq!(rebound.configuration, prior.configuration);
    assert_eq!(rebound.correction, 2_000_000);
    let later = inspect_source(&source, |source| {
        source.state.voices().find(|voice| voice.lifetime == 3).unwrap().assignment.unwrap()
    });
    assert!(later.revision > prior.configuration.revision);
    assert_ne!(
        later.tuning, prior.configuration.tuning,
        "post-fence input binds the later configuration at its original input time"
    );
    assert_eq!(
        inspect_source(&source, |source| source
            .state
            .voices()
            .find(|voice| voice.lifetime == held.lifetime)
            .unwrap()
            .frozen_offset_microcents),
        held.frozen_offset_microcents
    );
    assert_eq!(source.source_snapshot().faults, 0);
    raw += 512;
    source.run_format(
        raw,
        vec![note(7, 0, 60, 0, false), note(9, 2, 67, 0, false)],
        None,
        None,
        512,
    );
    hub.run_format(raw, vec![], None, None, 512);
    for _ in 0..160 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        if source.source_snapshot().held == 0 && source.source_snapshot().captures == 0 {
            break;
        }
    }
    assert_eq!(source.source_snapshot().held, 0);
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn production_cancellation_survives_inventory_order_and_a_copied_replay_status() {
    let _scope = crate::test_scope::enter();
    // Unbound cancellation before/after the actual Retained inventory, then a
    // bound cancellation after its incomplete status has already been copied.
    for scenario in 0..3 {
        let (hub, source) = production_pair();
        let bound = scenario == 2;
        let first = if bound { note(40, 0, 60, 0, true) } else { raw_midi([0xf8, 0, 0], 0) };
        source.run_format(1536, vec![first], None, None, 512);
        hub.run_format(1536, vec![], None, None, 512);
        if bound {
            inspect_source(&source, |source| {
                source.offer.as_ref().unwrap().session.rows[0]
                    .emission_gate
                    .fetch_or(source::CLOSED, Ordering::AcqRel)
            });
        }
        let output = source.run_format(
            2048,
            if bound { vec![] } else { vec![note(40, 0, 60, 0, true)] },
            None,
            None,
            512,
        );
        assert!(!output.values.iter().any(|(_, event)| event.attack().is_some()));
        let serial = if bound { 1 } else { 2 };
        let capture = inspect_source(&source, |source| source.test_capture(serial).unwrap());
        let wrapper = unsafe {
            &*((*hub.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
        };
        if bound {
            hub.run_format(2048, vec![], None, None, 512);
            assert_ne!(
                inspect_source(&source, |source| source
                    .test_assignment(capture.life)
                    .unwrap()
                    .decision),
                0
            );
        }
        wrapper.test_with_plugin(|plugin| {
            plugin.aggregation.as_mut().unwrap().test_request_recovery(1)
        });
        if !bound {
            hub.run_format(2048, vec![], None, None, 512);
        }
        let source_wrapper = unsafe {
            &*((*source.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
        };
        let cancel = || {
            source_wrapper.test_with_plugin(|plugin| {
                plugin.source.as_mut().unwrap().test_cancel_before_receive()
            })
        };
        let mut raw = 2048;
        loop {
            raw += 512;
            assert!(!source
                .run_format(raw, vec![], None, None, 512)
                .values
                .iter()
                .any(|(_, event)| event.attack().is_some()));
            let published = inspect_source(&source, |source| source.test_recovery_state().2) == 1;
            if !bound && published {
                if scenario == 0 {
                    cancel();
                }
                hub.run_format(raw, vec![], None, None, 512);
                if scenario == 1 {
                    cancel();
                }
                break;
            }
            hub.run_format(raw, vec![], None, None, 512);
            if bound
                && inspect_hub(&hub, |hub| hub.test_recovery_progress()).contains("phase=Rebuild")
            {
                assert_eq!(
                    inspect_hub(&hub, |hub| hub.test_plan_state(0, capture.life)),
                    Some((false, true, true))
                );
                cancel();
                break;
            }
            assert!(
                raw < 65536,
                "scenario {scenario}: {}",
                inspect_hub(&hub, |hub| hub.test_recovery_progress())
            );
        }
        assert!(
            source.source_snapshot().manifest != 0,
            "the real original-On disposition is pending"
        );
        for _ in 0..160 {
            raw += 512;
            assert!(
                !source
                    .run_format(raw, vec![], None, None, 512)
                    .values
                    .iter()
                    .any(|(_, event)| event.attack().is_some()),
                "canceled On was resurrected in scenario {scenario}"
            );
            hub.run_format(raw, vec![], None, None, 512);
            if source.source_snapshot().captures == 0
                && source.source_snapshot().lives == 0
                && inspect_hub(&hub, |hub| hub.test_plan_state(0, capture.life)).is_none()
            {
                break;
            }
        }
        assert_eq!(
            (source.source_snapshot().captures, source.source_snapshot().lives),
            (0, 0),
            "scenario {scenario}: {}",
            inspect_hub(&hub, |hub| hub.test_recovery_progress())
        );
        assert!(inspect_hub(&hub, |hub| hub.test_plan_state(0, capture.life)).is_none());
        raw += 512;
        source.run_format(raw, vec![note(41, 0, 62, 0, true)], None, None, 512);
        assert_eq!(
            inspect_source(&source, |source| source.test_capture(serial + 1).unwrap().life),
            capture.life,
            "the actual Life index is reused after the canceled identity retires"
        );
        hub.run_format(raw, vec![], None, None, 512);
        let mut ons = 0;
        for _ in 0..8 {
            raw += 512;
            ons += source
                .run_format(raw, vec![], None, None, 512)
                .values
                .iter()
                .filter(|(_, event)| event.attack().is_some_and(|(id, ..)| id == 41))
                .count();
            hub.run_format(raw, vec![], None, None, 512);
        }
        assert_eq!(ons, 1, "reused Life can receive a new valid binding in scenario {scenario}");
        raw += 512;
        source.run_format(raw, vec![note(41, 0, 62, 0, false)], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        for _ in 0..8 {
            raw += 512;
            source.run_format(raw, vec![], None, None, 512);
            hub.run_format(raw, vec![], None, None, 512);
        }
        assert_eq!(source.source_snapshot().faults, 0);
        drop(source);
        drop(hub);
        assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
    }
}

#[test]
fn production_joined_owners_finish_an_active_chunked_recovery_without_new_output() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    let input = (0..65).map(|_| note(7, 0, 60, 0, true)).collect();
    assert!(source.run_format(1536, input, None, None, 512).values.is_empty());
    hub.run_format(1536, vec![], None, None, 512);
    assert_eq!(source.source_snapshot().lives, 65);
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    wrapper
        .test_with_plugin(|plugin| plugin.aggregation.as_mut().unwrap().test_request_recovery(1));
    hub.run_format(2048, vec![], None, None, 512);
    assert!(inspect_hub(&hub, |hub| hub.test_recovery_progress()).contains("active=true"));
    assert!(source.run_format(2048, vec![], None, None, 512).values.is_empty());
    let mut raw = 2048;
    while !inspect_source(&source, |source| source.test_recovery_state().0) {
        raw += 512;
        assert!(raw < 8192, "the crossed status response must yield the fence cell");
        assert!(source.run_format(raw, vec![], None, None, 512).values.is_empty());
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(source.source_snapshot().faults, 0);
    assert_eq!(
        inspect_source(&source, |source| {
            let (active, total, ..) = source.test_recovery_state();
            (active, total)
        }),
        (true, 65),
        "the fixed lifetime inventory actually requires two chunks before both callbacks join"
    );
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0),
        "joined pumps finish both inventory chunks, their acknowledgements, request pins and capture/plan retirements");
}

#[test]
fn production_recovery_rejects_a_stored_committed_off_binding_after_reopen() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    source.run_format(
        1536,
        vec![source.participation(false, 0), note(70, 0, 60, 1, true)],
        None,
        None,
        512,
    );
    hub.run_format(1536, vec![], None, None, 512);
    let capture = inspect_source(&source, |source| source.test_capture(1).unwrap());
    assert!(!capture.adaptive);
    let (lease, shared) = inspect_source(&source, |source| {
        let offer = source.offer.as_ref().unwrap();
        (offer.lease, offer.session.rows[0].clone())
    });
    let generation =
        shared.emission_gate.fetch_or(source::CLOSED, Ordering::AcqRel) & !source::GATE_FLAGS;
    assert!(source.run_format(2048, vec![], None, None, 512).values.is_empty());
    hub.run_format(2048, vec![], None, None, 512);
    let prior = inspect_source(&source, |source| source.test_assignment(capture.life).unwrap());
    assert_eq!((prior.decision, prior.emission, prior.correction), (1, generation, 0));
    let key = inspect_hub(&hub, |hub| {
        hub.test_capture_keys(1).into_iter().find(|key| key.serial == 1).unwrap()
    });
    let fence = protocol::Fence {
        lease,
        epoch: source.source_snapshot().epoch,
        transaction: 1,
        generation,
        from_decision: 1,
        terminal: false,
    };
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    let send = |reply| {
        wrapper.test_with_plugin(|plugin| {
            plugin.aggregation.as_mut().unwrap().offer.as_mut().unwrap().bank.rows[0]
                .replies
                .push(reply)
                .unwrap();
        })
    };
    send(protocol::Reply::Fence(fence));
    let mut raw = 2048;
    let mut acknowledged = false;
    let chunk = loop {
        raw += 512;
        assert!(source.run_format(raw, vec![], None, None, 512).values.is_empty());
        if let Some(protocol::Control::RevokeAck {
            fence: found,
            input_cut: 1,
            output_cut: 0,
            ..
        }) = shared
            .to_hub
            .take_repair_if(|control| matches!(control, protocol::Control::RevokeAck { .. }))
        {
            assert_eq!(found, fence);
            acknowledged = true;
        }
        hub.run_format(raw, vec![], None, None, 512);
        if let Some(chunk) = shared.inventory.read_owned() {
            break chunk;
        }
        assert!(raw < 32768);
    };
    assert!(acknowledged);
    assert_eq!((chunk.value().total, chunk.value().count), (1, 1));
    let record = chunk.value().records[0].unwrap();
    assert_eq!(record.outcome, protocol::RequestOutcome::Retained);
    assert_eq!(record.decision, prior.decision);
    drop(chunk);
    send(protocol::Reply::InventoryComplete { fence, input_cut: 1, total: 1, chunks: 1 });
    assert_eq!(
        shared.emission_gate.compare_exchange(
            generation | source::CLOSED,
            generation + 4,
            Ordering::AcqRel,
            Ordering::Acquire
        ),
        Ok(generation | source::CLOSED)
    );
    send(protocol::Reply::RecoveryComplete {
        fence,
        generation: generation + 4,
        boundary: raw + 512,
    });
    send(protocol::Reply::Assignment {
        key,
        life: capture.life,
        lifetime: capture.life_serial,
        binding: prior,
    });
    send(protocol::Reply::CohortCommitted { lease, epoch: fence.epoch, through: prior.decision });
    for _ in 0..6 {
        raw += 512;
        assert!(
            source.run_format(raw, vec![], None, None, 512).values.is_empty(),
            "neither the stored Off binding nor delayed old replies regain emission after reopen"
        );
    }
    assert!(!inspect_source(&source, |source| source.test_recovery_state().0));
    assert_eq!(
        inspect_source(&source, |source| source.test_assignment(capture.life).unwrap().decision),
        prior.decision
    );
    assert_eq!(
        (
            source.source_snapshot().held,
            source.source_snapshot().captures,
            source.source_snapshot().faults
        ),
        (0, 1, 0)
    );
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn production_revoke_inventory_keeps_a_lifetime_after_its_on_capture_and_plan_retire() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    source.run_format(1536, vec![note(7, 0, 60, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    let mut raw = 1536;
    for _ in 0..16 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        if source.source_snapshot().captures == 0 {
            break;
        }
    }
    assert_eq!((source.source_snapshot().held, source.source_snapshot().captures), (1, 0));
    let bound = inspect_source(&source, |source| *source.state.voices().next().unwrap());
    let (lease, shared) = inspect_source(&source, |source| {
        let offer = source.offer.as_ref().unwrap();
        (offer.lease, offer.session.rows[0].clone())
    });
    let generation =
        shared.emission_gate.fetch_or(source::CLOSED, Ordering::AcqRel) & !source::GATE_FLAGS;
    let fence = protocol::Fence {
        lease,
        epoch: source.source_snapshot().epoch,
        transaction: 1,
        generation,
        from_decision: 1,
        terminal: false,
    };
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    let send = |reply| {
        wrapper.test_with_plugin(|plugin| {
            plugin.aggregation.as_mut().unwrap().offer.as_mut().unwrap().bank.rows[0]
                .replies
                .push(reply)
                .unwrap();
        })
    };
    send(protocol::Reply::Fence(fence));
    raw += 512;
    source.run_format(raw, vec![note(7, 0, 60, 0, false)], None, None, 512);
    let ack = shared.to_hub.take_repair_if(|_| true).unwrap();
    assert!(
        matches!(ack, protocol::Control::RevokeAck { fence: found, input_cut: 1, output_cut: 2, .. } if found == fence)
    );
    hub.run_format(raw, vec![], None, None, 512);
    let mut releases = 0;
    let chunk = loop {
        if let Some(chunk) = shared.inventory.read_owned() {
            break chunk;
        }
        raw += 512;
        releases += source
            .run_format(raw, vec![], None, None, 512)
            .values
            .iter()
            .filter(|(_, event)| event.release())
            .count();
        hub.run_format(raw, vec![], None, None, 512);
        assert!(raw < 32768);
    };
    assert_eq!(releases, 1, "essential release runs while attack emission is fenced");
    assert_eq!(
        (
            source.source_snapshot().held,
            source.source_snapshot().captures,
            source.source_snapshot().lives
        ),
        (0, 0, 1)
    );
    assert_eq!((chunk.value().total, chunk.value().count, chunk.value().lifetime_cut), (1, 1, 1));
    let request = chunk.value().records[0].unwrap();
    assert_eq!(request.outcome, protocol::RequestOutcome::Accepted);
    assert_eq!(request.lifetime, bound.lifetime);
    assert_eq!(request.input, 1536);
    assert_ne!(request.decision, 0);
    assert_eq!(Some(request.configuration), bound.assignment);
    drop(chunk);
    send(protocol::Reply::InventoryComplete { fence, input_cut: 1, total: 1, chunks: 1 });
    assert_eq!(
        shared.emission_gate.compare_exchange(
            generation | source::CLOSED,
            generation + 4,
            Ordering::AcqRel,
            Ordering::Acquire
        ),
        Ok(generation | source::CLOSED)
    );
    send(protocol::Reply::RecoveryComplete { fence, generation: generation + 4, boundary: raw });
    for _ in 0..34 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        (
            source.source_snapshot().held,
            source.source_snapshot().lives,
            source.source_snapshot().faults
        ),
        (0, 0, 0)
    );
}

#[test]
fn production_revoke_inventory_chunks_sixty_five_requests_while_capture_window_is_full() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    wrapper.test_with_plugin(|plugin| plugin.aggregation.as_mut().unwrap().test_pause_captures());
    let events = (0..65)
        .map(|_| note(7, 0, 60, 0, true))
        .chain((0..1100).map(|_| expression(7, 0.5, 1)))
        .collect();
    source.run_format(1536, events, None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    let mut raw = 1536;
    for _ in 0..16 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        if inspect_hub(&hub, |hub| hub.test_capture_phases(1).0) == 1024 {
            break;
        }
    }
    assert_eq!(inspect_hub(&hub, |hub| hub.test_capture_phases(1)), (1024, 0));
    assert_eq!(source.source_snapshot().input_cut, 1165);
    let epoch = source.source_snapshot().epoch;
    let (lease, shared) = inspect_source(&source, |source| {
        let offer = source.offer.as_ref().unwrap();
        (offer.lease, offer.session.rows[0].clone())
    });
    let generation =
        shared.emission_gate.fetch_or(source::CLOSED, Ordering::AcqRel) & !source::GATE_FLAGS;
    let fence = protocol::Fence {
        lease,
        epoch,
        transaction: 1,
        generation,
        from_decision: 1,
        terminal: false,
    };
    let send = |reply| {
        wrapper.test_with_plugin(|plugin| {
            plugin.aggregation.as_mut().unwrap().offer.as_mut().unwrap().bank.rows[0]
                .replies
                .push(reply)
                .unwrap();
        })
    };
    send(protocol::Reply::Fence(fence));
    raw += 512;
    source.run_format(raw, vec![note(7, 0, 60, 0, true)], None, None, 512);
    let ack = shared
        .to_hub
        .take_repair_if(|_| true)
        .expect("completion boundary publishes its independent ACK");
    assert!(
        matches!(ack, protocol::Control::RevokeAck { fence: found, input_cut: 1165, output_cut: 0, .. } if found == fence)
    );
    let first = loop {
        if let Some(chunk) = shared.inventory.read_owned() {
            break chunk;
        }
        raw += 512;
        assert!(source.run_format(raw, vec![], None, None, 512).values.is_empty());
        assert!(inspect_source(&source, |source| source.test_recovery_state().3) <= 256);
        assert!(raw < 32768);
    };
    assert_eq!(
        (first.value().sequence, first.value().total, first.value().first, first.value().count),
        (0, 65, 0, 64)
    );
    assert_eq!(first.value().input_cut, 1165);
    assert_eq!(first.value().output_cut, 0);
    assert_eq!(first.value().fence, fence);
    assert_ne!(first.value().arena, 0);
    for (index, record) in first.value().records.iter().flatten().enumerate() {
        assert_eq!(record.lifetime, index as u64 + 1);
        assert_eq!(record.outcome, protocol::RequestOutcome::Retained);
        assert_eq!(record.input, 1536);
        assert_eq!(record.decision, 0);
    }
    for _ in 0..3 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        assert_eq!(inspect_source(&source, |source| source.test_recovery_state().2), 1);
        assert_eq!(
            first.value().records[63].unwrap().lifetime,
            64,
            "Reading retains the original chunk through callbacks"
        );
    }
    drop(first);
    raw += 512;
    source.run_format(raw, vec![], None, None, 512);
    let second =
        shared.inventory.read_owned().expect("releasing the first chunk permits its successor");
    assert_eq!((second.value().sequence, second.value().first, second.value().count), (1, 64, 1));
    assert_eq!(second.value().records[0].unwrap().lifetime, 65);
    assert!(second.value().records[1..].iter().all(Option::is_none));
    drop(second);
    send(protocol::Reply::RecoveryComplete { fence, generation: generation + 4, boundary: raw });
    raw += 512;
    source.run_format(raw, vec![], None, None, 512);
    assert!(
        inspect_source(&source, |source| source.test_recovery_state().0),
        "generation alone cannot acknowledge the inventory"
    );
    send(protocol::Reply::InventoryComplete { fence, input_cut: 1165, total: 65, chunks: 2 });
    assert_eq!(
        shared.emission_gate.compare_exchange(
            generation | source::CLOSED,
            generation + 4,
            Ordering::AcqRel,
            Ordering::Acquire
        ),
        Ok(generation | source::CLOSED)
    );
    send(protocol::Reply::RecoveryComplete { fence, generation: generation + 4, boundary: raw });
    raw += 512;
    source.run_format(raw, vec![], None, None, 512);
    assert!(!inspect_source(&source, |source| source.test_recovery_state().0));
    assert_eq!(source.source_snapshot().faults, 0);
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_capture_phases(1)),
        (1024, 0),
        "inventory never requires the blocked Capture suffix"
    );
    drop(shared);
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
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
    let uuid = SavedUuid::default();
    let calibration =
        Calibration { offset: 0, sample_rate: 44100.0, max_frames: 512, validated: true };
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, calibration);
    hub.activate_format(44100.0, 512);
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
    for source in &sources {
        source.run_format(
            1536,
            (0..16).map(|key| note(key, 0, key as i16 + 48, 0, true)).collect(),
            None,
            None,
            512,
        );
    }
    hub.run_format(1536, vec![], None, None, 512);
    let mut onsets = [0; 16];
    let mut actual = [None; 16];
    let mut raw = 1536;
    for _ in 0..256 {
        raw += 512;
        for (index, source) in sources.iter().enumerate() {
            let output = source.run_format(raw, vec![], None, None, 512);
            let count = output.values.iter().filter(|(_, event)| event.attack().is_some()).count();
            if count != 0 {
                assert!(
                    actual[index].replace(raw).is_none(),
                    "no Source repeats its accepted onsets"
                );
                println!(
                    "CAPACITY_DELIVERY source={index} actual={raw} extra={} marker_state={:?}",
                    raw - 2048,
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
                assert_eq!(keys, (48..64).collect::<Vec<_>>());
            }
            onsets[index] += count;
            assert_eq!(source.source_snapshot().faults, 0);
        }
        hub.run_format(raw, vec![], None, None, 512);
        if onsets == [16; 16] {
            break;
        }
    }
    assert_eq!(onsets, [16; 16]);
    assert_eq!(session.credits.load(Ordering::Acquire), 256);
    assert!(sources
        .iter()
        .all(|source| source.source_snapshot().held == 16 && source.latency() == 512));
    raw += 512;
    let last_release = raw + actual.into_iter().flatten().max().unwrap() - 1536;
    for source in &sources {
        source.run_format(
            raw,
            (0..16).map(|key| note(key, 0, key as i16 + 48, 0, false)).collect(),
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
    let wrapper = unsafe {
        &*((*source.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    let (context, param) = wrapper.test_gui_context(setup::PARTICIPATING);
    source.run_format(1536, vec![note(70, 0, 60, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    unsafe {
        context.raw_begin_set_parameter(param);
        context.raw_set_parameter_normalized(param, 0.0);
        context.raw_end_set_parameter(param);
    }
    source.run_format(
        2048,
        vec![note(71, 0, 62, 0, true), source.participation(true, 1), note(72, 0, 64, 2, true)],
        Some(CLAP_EVENT_PARAM_VALUE),
        None,
        512,
    );
    assert!(inspect_source(&source, |source| source.test_capture(1).unwrap().adaptive));
    assert!(!inspect_source(&source, |source| source.test_capture(2).unwrap().adaptive));
    assert!(inspect_source(&source, |source| source.test_capture(3).unwrap().adaptive));
    hub.run_format(2048, vec![], None, None, 512);
    // No host echo follows the native Set. Accepting its old notification now
    // cannot reinsert Off ahead of D or overwrite the newer host On value.
    source.run_format(2560, vec![note(73, 0, 65, 0, true)], None, None, 512);
    assert!(inspect_source(&source, |source| source.test_capture(4).unwrap().adaptive));
    assert!(inspect_source(&source, |source| source.participating));
    assert!(wrapper.test_inspect_plugin(|plugin| plugin.params.participating.value()));
    hub.run_format(2560, vec![], None, None, 512);
    for raw in [3072, 3584] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    source.run_format(
        4096,
        vec![
            note(70, 0, 60, 0, false),
            note(71, 0, 62, 0, false),
            note(72, 0, 64, 0, false),
            note(73, 0, 65, 0, false),
        ],
        None,
        None,
        512,
    );
    hub.run_format(4096, vec![], None, None, 512);
    for raw in (4608..8704).step_by(512) {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
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
    let calibration =
        Calibration { offset: 0, sample_rate: 44100.0, max_frames: 512, validated: true };
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
        assert_eq!(value, [0.01, 0.02, 0.04][index]);
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
    let calibration =
        Calibration { offset: 0, sample_rate: 44100.0, max_frames: 512, validated: true };
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
    let calibration =
        Calibration { offset: 0, sample_rate: 44100.0, max_frames: 512, validated: true };
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
                    let calibration = Calibration {
                        offset: 0,
                        sample_rate: 44100.0,
                        max_frames: 512,
                        validated: true,
                    };
                    let mut hub = Device::new(false);
                    hub.configure_format(uuid, true, calibration);
                    hub.activate_format(44100.0, 512);
                    let sources: [Device; 3] = std::array::from_fn(|_| {
                        let mut source = Device::new(true);
                        source.configure_format(uuid, true, Calibration { offset, ..calibration });
                        source.activate_format(44100.0, 512);
                        source
                    });
                    for raw in [0, 512, 1024] {
                        for source in &sources {
                            source.run_format(raw, vec![], None, None, 512);
                        }
                        hub.run_format(raw, vec![], None, None, 512);
                    }
                    for index in capture_order {
                        assert!(sources[index]
                            .run_format(
                                1536,
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
                    hub.run_format(1536, vec![], None, None, 512);
                    if hub_first {
                        hub.run_format(2048, vec![], None, None, 512);
                    }
                    let late = !hub_first && i64::from(onset_offset) + offset >= 512;
                    for index in output_order {
                        let output = sources[index].run_format(2048, vec![], None, None, 512);
                        if late {
                            assert!(output.values.is_empty());
                        } else {
                            assert_assignment_output(&output, index, onset_offset);
                        }
                    }
                    if !hub_first {
                        hub.run_format(2048, vec![], None, None, 512);
                    }
                    for index in output_order {
                        let output = sources[index].run_format(2560, vec![], None, None, 512);
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
                    hub.run_format(2560, vec![], None, None, 512);
                    for (index, source) in sources.iter().enumerate() {
                        source.run_format(
                            3072,
                            vec![note(index as i32, 0, 60 + index as i16 * 2, 0, false)],
                            None,
                            None,
                            512,
                        );
                    }
                    hub.run_format(3072, vec![], None, None, 512);
                    for raw in (3584..7680).step_by(512) {
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
    assert_eq!(value, [0.01, 0.02, 0.04][source]);
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
                    let calibration = Calibration {
                        offset: 0,
                        sample_rate: 44100.0,
                        max_frames: 512,
                        validated: true,
                    };
                    let mut hub = Device::new(false);
                    hub.configure_format(uuid, true, calibration);
                    hub.activate_format(44100.0, 512);
                    let sources: [Device; 3] = std::array::from_fn(|index| {
                        let mut source = Device::new(true);
                        source.configure_format(
                            uuid,
                            true,
                            Calibration { offset: if index == 1 { 64 } else { 0 }, ..calibration },
                        );
                        source.activate_format(44100.0, 512);
                        source
                    });
                    for raw in [0, 512, 1024] {
                        for source in &sources {
                            source.run_format(raw, vec![], None, None, 512);
                        }
                        hub.run_format(raw, vec![], None, None, 512);
                    }
                    // All three onsets name one mapped sample. At B448/511,
                    // A/C's original input belongs to their NEXT host callback.
                    let onsets = [1536 + b_offset + 64, 1536 + b_offset, 1536 + b_offset + 64];
                    for index in capture_order {
                        let events = if onsets[index] < 2048 {
                            vec![note(
                                index as i32,
                                0,
                                60 + index as i16 * 2,
                                (onsets[index] - 1536) as u32,
                                true,
                            )]
                        } else {
                            vec![]
                        };
                        assert!(sources[index]
                            .run_format(1536, events, None, None, 512)
                            .values
                            .is_empty());
                    }
                    hub.run_format(1536, vec![], None, None, 512);
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
                            hub.run_format(2048, vec![], None, None, 512);
                        }
                        let Some(index) = index else { continue };
                        let events = if onsets[index] >= 2048 {
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
                        let output = sources[index].run_format(2048, events, None, None, 512);
                        if !output.values.is_empty() {
                            let time = (onsets[index] + 512 - 2048) as u32;
                            assert_assignment_output(&output, index, time);
                            actual[index] = Some(2048 + i64::from(time));
                        }
                    }
                    hub.run_format(2560, vec![], None, None, 512);
                    for index in output_order {
                        let output = sources[index].run_format(2560, vec![], None, None, 512);
                        if !output.values.is_empty() {
                            assert!(actual[index].is_none(), "no duplicate assignment output");
                            let time = (onsets[index] + 512).max(2560) as u32 - 2560;
                            assert_assignment_output(&output, index, time);
                            actual[index] = Some(2560 + i64::from(time));
                        }
                        let expected =
                            if index == 1 && b_late { 2560 } else { onsets[index] + 512 };
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
                            3072,
                            vec![note(index as i32, 0, 60 + index as i16 * 2, 0, false)],
                            None,
                            None,
                            512,
                        );
                    }
                    hub.run_format(3072, vec![], None, None, 512);
                    for raw in (3584..7680).step_by(512) {
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
    let calibration =
        Calibration { offset: 0, sample_rate: 44100.0, max_frames: 512, validated: true };
    let (recorder, mut capture) = harmonigraph_record::testing::channel();
    crate::configuration::inject_recorder(recorder);
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
    capture.arm();
    let directory = std::env::temp_dir()
        .join(format!("harmonigraph-sequencing-partial-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("partial.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    source.run_format(1536, vec![note(7, 0, 60, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    let mut acceptance = vec![false; 1024];
    acceptance[0] = true;
    ACCEPTANCE_SCRIPT.with(|script| *script.borrow_mut() = acceptance);
    let output = source.run_format(2048, vec![], None, None, 512);
    assert_eq!(output.values.len(), 1);
    assert!(output.values[0].1.attack().is_some());
    assert!(matches!(output.rejected[0].1, Event::Expression { kind: 2, .. }));
    assert_eq!(source.source_snapshot().held, 1, "rejected emergency release keeps its credit");
    let local = inspect_source(&source, |source| *source.state.voices().next().unwrap());
    assert!(local.partial_output && local.release_pending);
    assert_eq!(local.pitch_microcents, 6_000_000_000);
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
    assert_eq!(partial[0].pitch_microcents, Some(6_000_000_000));
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
fn production_canceled_unsent_assignment_settles_with_a_full_reply_ring() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    source.run_format(1536, vec![note(99, 0, 60, 0, true)], None, None, 512);
    let capture = inspect_source(&source, |source| source.test_capture(1).unwrap());
    let hub_wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    let fill_replies = || {
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
        })
    };
    fill_replies();
    hub.run_format(1536, vec![], None, None, 512);
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_cohort_delivery().1),
        1,
        "real current-cohort assignment could not enqueue"
    );
    let source_wrapper = unsafe {
        &*((*source.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    source_wrapper
        .test_with_plugin(|plugin| plugin.source.as_mut().unwrap().test_cancel_before_receive());
    hub_wrapper
        .test_with_plugin(|plugin| plugin.aggregation.as_mut().unwrap().test_request_recovery(1));
    // Consume the crossed Hub status query without stealing its response;
    // cancellation cannot be received until that repair ACK cell is free.
    source.run_format(2048, vec![], None, None, 512);
    fill_replies();
    hub.run_format(2048, vec![], None, None, 512);
    assert_eq!(
        inspect_hub(&hub, |hub| hub.offer.as_ref().unwrap().bank.rows[0].replies.slots()),
        0
    );
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_cohort_delivery().1),
        0,
        "cancellation pays unsent exactly once despite identity reader and full reply ring"
    );
    assert!(inspect_hub(&hub, |hub| hub.test_plan_state(0, capture.life)).unwrap().0);
    for raw in (2560..100_352).step_by(512) {
        assert!(!source
            .run_format(raw, vec![], None, None, 512)
            .values
            .iter()
            .any(|(_, event)| event.attack().is_some()));
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!((source.source_snapshot().captures, source.source_snapshot().lives), (0, 0));
    assert!(inspect_hub(&hub, |hub| hub.test_plan_state(0, capture.life)).is_none());
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn production_terminal_partial_output_finishes_transaction_before_release_or_reset() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    source.run_format(1536, vec![note(7, 0, 60, 0, true)], None, None, 512);
    hub.run_format(1536, vec![], None, None, 512);
    let mut acceptance = vec![false; 640];
    acceptance[0] = true;
    ACCEPTANCE_SCRIPT.with(|script| *script.borrow_mut() = acceptance);
    let output = source.run_format(2048, vec![], None, None, 512);
    assert_eq!(output.values.len(), 1);
    assert!(output.values[0].1.attack().is_some());
    assert!(output.rejected.iter().any(|(_, event)| event.release()));
    hub.run_format(2048, vec![], None, None, 512);
    let mut raw = 2048;
    for _ in 0..160 {
        raw += 512;
        let output =
            source.run_format(raw, vec![note(8, 0, 62, 0, true)], Some(u16::MAX), None, 512);
        assert!(output.values.is_empty());
        hub.run_format(raw, vec![], None, None, 512);
        if !inspect_hub(&hub, |hub| hub.test_recovery_progress()).contains("active=true")
            && !inspect_source(&source, |source| source.test_recovery_state().0)
        {
            break;
        }
    }
    assert!(
        !inspect_hub(&hub, |hub| hub.test_recovery_progress()).contains("active=true"),
        "{}",
        inspect_hub(&hub, |hub| hub.test_recovery_progress())
    );
    assert!(!inspect_source(&source, |source| source.test_recovery_state().0));
    assert_eq!(source.source_snapshot().held, 1, "CLOSED is not accepted physical termination");
    assert_ne!(source.source_snapshot().faults & source::OUTPUT_FAULT, 0);
    let session = inspect_source(&source, |source| source.offer.as_ref().unwrap().session.clone());
    let gate = session.rows[0].emission_gate.load(Ordering::Acquire);
    assert_eq!(gate & source::GATE_FLAGS, source::CLOSED);
    assert_ne!(
        session.faults.load(Ordering::Acquire) & source::OUTPUT_FAULT,
        0,
        "enrolled contributor escalates"
    );
    for _ in 0..3 {
        raw += 512;
        source.run_format(raw, vec![], Some(u16::MAX), None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        assert_eq!(
            session.rows[0].emission_gate.load(Ordering::Acquire),
            gate,
            "rejected emergency retries do not restart the completed transaction"
        );
    }
    raw += 512;
    let output = source.run_format(raw, vec![], None, None, 512);
    assert_eq!(output.values.iter().filter(|(_, event)| event.release()).count(), 1);
    assert!(
        output.values.iter().all(|(offset, _)| *offset == 0),
        "essential repair ignores accumulated stream delay"
    );
    hub.run_format(raw, vec![], None, None, 512);
    for _ in 0..80 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        (
            source.source_snapshot().held,
            source.source_snapshot().captures,
            source.source_snapshot().lives
        ),
        (0, 0, 0)
    );
    assert_ne!(source.source_snapshot().faults & source::OUTPUT_FAULT, 0);
    let source_setup = source.shared();
    let hub_setup = hub.shared();
    source_setup.apply(source_setup.value().routing, true).unwrap();
    hub_setup.apply(hub_setup.value().routing, true).unwrap();
    assert_ne!(source.source_snapshot().faults, 0, "a Reset request is not fresh coverage");
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
        "{:?} source={} hub={}",
        source.source_snapshot(),
        inspect_source(&source, |s| s.test_reset_progress()),
        inspect_hub(&hub, |h| h.test_reset_progress())
    );
    assert_eq!(session.faults.load(Ordering::Acquire), 0);
    assert_eq!(inspect_hub(&hub, |hub| hub.test_terminal_scope()), (false, 0));
    raw += 512;
    let fresh = source.run_format(
        raw,
        vec![note(19, 0, 67, 0, true), expression(19, 0.25, 10), note(19, 0, 67, 20, false)],
        None,
        None,
        512,
    );
    let mut accepted = fresh.values;
    hub.run_format(raw, vec![], None, None, 512);
    for _ in 0..160 {
        raw += 512;
        accepted.extend(source.run_format(raw, vec![], None, None, 512).values);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(accepted.len(), 4, "new admission opens after explicit valid Reset");
    assert!(accepted[0].1.attack().is_some());
    assert!(accepted[3].1.release());
    assert_eq!(
        (
            source.source_snapshot().held,
            source.source_snapshot().pending,
            source.source_snapshot().faults
        ),
        (0, 0, 0)
    );
    drop(source);
    drop(hub);
    drop(session);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn production_fifteen_note_mixed_offsets_preserve_gestures_without_terminal_lateness() {
    let _scope = crate::test_scope::enter();
    for b_offset in [447i64, 448, 511] {
        for favorable in [true, false] {
            let uuid = SavedUuid::default();
            let calibration =
                Calibration { offset: 0, sample_rate: 44100.0, max_frames: 512, validated: true };
            let mut hub = Device::new(false);
            hub.configure_format(uuid, true, calibration);
            hub.activate_format(44100.0, 512);
            let sources: [Device; 3] = std::array::from_fn(|index| {
                let mut source = Device::new(true);
                source.configure_format(
                    uuid,
                    true,
                    Calibration { offset: if index == 1 { 64 } else { 0 }, ..calibration },
                );
                source.activate_format(44100.0, 512);
                source
            });
            let mut setup = [0; 3];
            for raw in [0, 512, 1024] {
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
            let onsets = [1536 + b_offset + 64, 1536 + b_offset, 1536 + b_offset + 64];
            let mut actual: [Vec<(i64, Event)>; 3] = std::array::from_fn(|_| Vec::new());
            let mut extra = [0; 3];
            for raw in (1536..7168).step_by(512) {
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
                    assert_eq!((events[2].0, expressed), (onset + 10, initial + 0.125));
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
            println!("FIFTEEN B{b_offset} favorable={favorable} extra={extra:?}, exact60 accepted events; ordinary prospective repair remains unwired");
            drop(sources);
            drop(hub);
            assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
        }
    }
}

#[test]
fn production_terminal_nonmember_fault_preserves_the_healthy_frozen_cohort() {
    let _scope = crate::test_scope::enter();
    for overlap_ordinary in [false, true] {
        let (hub, source) = production_pair();
        let uuid = match hub.shared().value().routing {
            setup::Routing::Hub(value) => value.uuid,
            _ => unreachable!(),
        };
        let calibration =
            Calibration { offset: 0, sample_rate: 44100.0, max_frames: 512, validated: true };
        source.run_format(
            1536,
            (0..64).map(|id| note(id, 0, id as i16, 0, true)).collect(),
            None,
            None,
            512,
        );
        let capture = inspect_source(&source, |source| source.test_capture(1).unwrap());
        let wrapper = unsafe {
            &*((*hub.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
        };
        let fill = || {
            wrapper.test_with_plugin(|plugin| {
                let replies =
                    &mut plugin.aggregation.as_mut().unwrap().offer.as_mut().unwrap().bank.rows[0]
                        .replies;
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
            })
        };
        fill();
        hub.run_format(1536, vec![], None, None, 512);
        let mut raw = 1536;
        while inspect_hub(&hub, |hub| hub.test_plan_state(0, capture.life)).is_none() {
            raw += 512;
            assert!(raw < 8192);
            assert!(source.run_format(raw, vec![], None, None, 512).values.is_empty());
            fill();
            hub.run_format(raw, vec![], None, None, 512);
        }
        assert!(inspect_hub(&hub, |hub| hub.test_plan_state(0, capture.life)).unwrap().1);
        assert!(inspect_hub(&hub, |hub| hub.test_input_sequence_progress().2
            || hub.test_cohort_delivery().3));
        assert_ne!(inspect_hub(&hub, |hub| hub.test_cohort_delivery().1), 0);
        if overlap_ordinary {
            wrapper.test_with_plugin(|plugin| {
                plugin.aggregation.as_mut().unwrap().test_request_recovery(1)
            });
            assert!(inspect_hub(&hub, |hub| hub.test_recovery_progress()).contains("active=true"));
        }
        let mut late = Device::new(true);
        late.configure_format(uuid, true, calibration);
        late.activate_format(44100.0, 512);
        // Actual wrapper input inspection failure on a newly adopted, not enrolled
        // Source. Its old initial baseline cannot fill the published join interval.
        late.run_status(
            raw,
            (0..2049).map(|_| raw_midi([0xf8, 0, 0], 0)).collect(),
            None,
            None,
            512,
            true,
        );
        let mut onsets = 0;
        let mut tunings = 0;
        for _ in 0..128 {
            raw += 512;
            let output = source.run_format(raw, vec![], None, None, 512);
            onsets += output.values.iter().filter(|(_, event)| event.attack().is_some()).count();
            tunings += output
                .values
                .iter()
                .filter(|(_, event)| matches!(event, Event::Expression { kind: 2, .. }))
                .count();
            late.run_format(raw, vec![], None, None, 512);
            hub.run_format(raw, vec![], None, None, 512);
            assert_eq!(inspect_hub(&hub, |hub| hub.test_terminal_scope()), (false, 2));
            assert!(!inspect_hub(&hub, |hub| hub.test_input_row(1).0));
            assert_eq!(source.source_snapshot().faults, 0);
            if onsets == 64 {
                break;
            }
        }
        assert_eq!((onsets, tunings), (64, 64), "local settlement retains the healthy cohort's cursor, configuration and delivery obligations");
        assert_ne!(late.source_snapshot().faults & source::INPUT_FAULT, 0);
        println!(
            "LOCAL completed64 raw={raw} extra={} snapshot={:?}",
            source.shared().extra_delay.load(Ordering::Acquire),
            source.source_snapshot()
        );
        raw += 512;
        source.run_format(
            raw,
            (0..64).map(|id| note(id, 0, id as i16, 0, false)).collect(),
            None,
            None,
            512,
        );
        late.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
        let drain_rounds = source.shared().extra_delay.load(Ordering::Acquire) / 512 + 160;
        for _ in 0..drain_rounds {
            raw += 512;
            source.run_format(raw, vec![], None, None, 512);
            late.run_format(raw, vec![], None, None, 512);
            hub.run_format(raw, vec![], None, None, 512);
        }
        assert_eq!(
            (
                source.source_snapshot().held,
                source.source_snapshot().captures,
                source.source_snapshot().lives
            ),
            (0, 0, 0),
            "raw={raw} {:?}; Hub {:?}",
            source.source_snapshot(),
            inspect_hub(&hub, |hub| hub.test_input_sequence_progress())
        );
        drop(late);
        drop(source);
        drop(hub);
        assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
    }
}

#[test]
fn production_replay_accepts_done_original_with_an_already_retired_plan() {
    let _scope = crate::test_scope::enter();
    let (hub, source) = production_pair();
    source.run_format(1536, vec![note(7, 0, 60, 0, true)], None, None, 512);
    let original = inspect_source(&source, |source| source.test_capture(1).unwrap());
    hub.run_format(1536, vec![], None, None, 512);
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    wrapper.test_with_plugin(|plugin| plugin.aggregation.as_mut().unwrap().test_pause_captures());
    assert_eq!(
        source.run_format(2048, vec![note(7, 0, 60, 20, false)], None, None, 512).values.len(),
        2
    );
    hub.run_format(2048, vec![], None, None, 512);
    let released = source.run_format(2560, vec![], None, None, 512);
    assert_eq!(released.values.iter().filter(|(_, event)| event.release()).count(), 1);
    hub.run_format(2560, vec![], None, None, 512);
    assert!(inspect_hub(&hub, |hub| hub.test_plan_state(0, original.life)).unwrap().0);
    // Isolate the legitimate independent retirement order: exact actual Off
    // makes the Plan terminal while a retained capture still owns its Original.
    wrapper.test_with_plugin(|plugin| {
        plugin.aggregation.as_mut().unwrap().test_service_terminal_plans()
    });
    assert!(inspect_hub(&hub, |hub| hub.test_plan_state(0, original.life)).is_none());
    assert!(inspect_source(&source, |source| source.test_capture(1)).is_some());
    wrapper
        .test_with_plugin(|plugin| plugin.aggregation.as_mut().unwrap().test_request_recovery(1));
    let mut raw = 2560;
    for _ in 0..160 {
        raw += 512;
        assert!(source.run_format(raw, vec![], None, None, 512).values.is_empty());
        hub.run_format(raw, vec![], None, None, 512);
        if !inspect_hub(&hub, |hub| hub.test_recovery_progress()).contains("active=true") {
            break;
        }
    }
    assert!(!inspect_hub(&hub, |hub| hub.test_recovery_progress()).contains("active=true"));
    assert!(inspect_hub(&hub, |hub| hub.test_plan_state(0, original.life)).is_none());
    wrapper.test_with_plugin(|plugin| {
        plugin.aggregation.as_mut().unwrap().test_resume_capture_collection()
    });
    for _ in 0..80 {
        raw += 512;
        assert!(source.run_format(raw, vec![], None, None, 512).values.is_empty());
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(
        (
            source.source_snapshot().lives,
            source.source_snapshot().captures,
            source.source_snapshot().faults
        ),
        (0, 0, 0)
    );
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn production_terminal_joined_pressure_advances_completed_capture_scans() {
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
    let mut raw = 1536;
    let mut accepted = 0;
    for block in 0..12 {
        let input =
            if block < 8 { (0..512).map(|_| raw_midi([0xf8, 0, 0], 0)).collect() } else { vec![] };
        accepted += source.run_format(raw, input, None, None, 512).values.len();
        hub.run_format(raw, vec![], None, None, 512);
        raw += 512;
    }
    let state = source.source_snapshot();
    assert_eq!(accepted, 4096);
    assert_eq!((state.pending, state.local_pending, state.intent_slots), (4096, 0, 0));
    assert_eq!(state.captures, 2047, "Hub and Source input windows are full");
    assert!(state.journal > 0, "accepted factual history also remains owned");
    assert!(inspect_source(&source, |s| s.test_capture(1025).unwrap().local_done));
    // The real cancellation cursor crosses completed retained originals. No
    // disposition, output ACK or queue length changes can supply this witness.
    let before = inspect_source(&source, |s| s.service_position());
    source_wrapper
        .test_with_plugin(|plugin| plugin.source.as_mut().unwrap().test_cancel_before_receive());
    assert_ne!(inspect_source(&source, |s| s.service_position()), before);
    for _ in 0..2 {
        source.run_format(
            raw,
            (0..1536).map(|_| note(7, 0, 60, 1, true)).collect(),
            None,
            None,
            512,
        );
        hub.run_format(raw, vec![], None, None, 512);
        raw += 512;
    }
    source.run_status(
        raw,
        (0..2049).map(|_| raw_midi([0xf8, 0, 0], 0)).collect(),
        None,
        None,
        512,
        true,
    );
    assert_ne!(source.source_snapshot().faults & source::INPUT_FAULT, 0);
    hub.run_format(raw, vec![], None, None, 512);
    for _ in 0..4 {
        raw += 512;
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    assert_eq!(source.source_snapshot().lives, 3072, "48 inventory chunks remain owned at join");
    assert!(inspect_source(&source, |s| s.test_recovery_state().0));
    drop(source);
    drop(hub);
    assert!(registry::test_service_rounds() > 2048, "the fixture exceeds the obsolete loop bound");
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
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
