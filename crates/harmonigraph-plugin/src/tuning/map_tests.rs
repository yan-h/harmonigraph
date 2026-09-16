//! Map timing through real CLAP ingress, Hub ordering and Tune output.
use super::*;
use harmonigraph_core::lattice_map::LatticeMap;
use harmonigraph_core::{LatticePos, Tuning};

fn parameter(id: &str, value: f64, time: u32) -> Input {
    Input::Param(clap_event_param_value {
        header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, time),
        param_id: nice_plug::wrapper::hash_param_id(id),
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    })
}
fn install(hub: &Device) -> (LatticeMap, LatticeMap) {
    musical_tests::configure(hub, Tuning::just());
    hub.shared().set_retune(true);
    let first = LatticeMap::default();
    let mut second = first;
    second.replace(LatticePos::new(4, 0, 0));
    hub_wrapper(hub).test_inspect_plugin(|plugin| {
        assert_eq!(plugin.params.maps.write().capture(second, "Pythagorean E".into()), Some(1));
    });
    (first, second)
}
fn voice(hub: &Device, source: u8, key: u8) -> harmonigraph_core::canonical::VoiceBaseline {
    inspect_hub(hub, |hub| hub.test_voice(source, 0, key)).unwrap()
}

#[test]
fn lattice_map_automation_precedes_coincident_attacks_in_both_callback_orders() {
    let _scope = crate::test_scope::enter();
    for frames in [32, 128, 512] {
        for hub_first in [false, true] {
            let mut hub = Device::new(false);
            hub.activate_format(48000.0, frames);
            let (first, mut second) = install(&hub);
            // Rotation is zero, so MIDI E still reaches the substituted base E slot.
            second.position = LatticePos::new(50, -3, 1);
            let mut tune = Device::new(true);
            tune.activate_format(48000.0, frames);
            // 2x tolerates Hub-first callbacks without changing input timestamps.
            tune.run_format(0, vec![tune.param_event(DELAY_PARAM, 1.0, 0)], None, None, frames);
            tune.reactivate_format(48000.0, frames);
            hub.run_format(0, vec![parameter("tuning-engine", 2.0, 0)], None, None, frames);
            let start = i64::from(frames);
            let boundary = frames / 2;
            let input = vec![note(1, 0, 64, boundary - 1, true), note(2, 0, 76, boundary, true)];
            let automation = vec![
                parameter("map-fifths", 4096.0 + 50.0, boundary),
                parameter("lattice-map", 1.0, boundary),
                parameter("map-thirds", 4096.0 - 3.0, boundary),
                parameter("map-sevenths", 4096.0 + 1.0, boundary),
            ];
            if hub_first {
                hub.run_format(start, automation, None, None, frames);
                tune.run_format(start, input, None, None, frames);
            } else {
                tune.run_format(start, input, None, None, frames);
                hub.run_format(start, automation, None, None, frames);
            }
            hub.run_format(start * 2, vec![], None, None, frames);
            tune.run_format(start * 2, vec![], None, None, frames);
            let output = tune.run_format(start * 3, vec![], None, None, frames);
            assert_eq!(
                voice(&hub, 0, 64).frozen_offset_microcents,
                first.correction(64, Tuning::just())
            );
            assert_eq!(
                voice(&hub, 0, 76).frozen_offset_microcents,
                second.correction(76, Tuning::just())
            );
            assert_eq!(voice(&hub, 0, 76).attack_node, Some(second.node(76)));
            let corrections: Vec<_> = output
                .values
                .iter()
                .filter_map(|(_, event)| match event {
                    Event::Expression { kind: 2, value, .. } => Some(*value),
                    _ => None,
                })
                .collect();
            assert_eq!(corrections.len(), 2);
            assert!(
                (corrections[0] - first.correction(64, Tuning::just()) as f64 / 1e8).abs() < 1e-8
            );
            assert!(
                (corrections[1] - second.correction(76, Tuning::just()) as f64 / 1e8).abs() < 1e-8
            );
            assert_eq!(tune.shared().misses.load(Ordering::Relaxed), 0);
            // Following callbacks seed plain values without translating twice.
            hub.run_format(start * 4, vec![note(3, 0, 88, 0, true)], None, None, frames);
            assert_eq!(voice(&hub, crate::tuning::DIRECT, 88).attack_node, Some(second.node(88)));
            assert_eq!(
                voice(&hub, 0, 64).frozen_offset_microcents,
                first.correction(64, Tuning::just())
            );
        }
    }
}

#[test]
fn lattice_map_held_notes_keep_onset_while_chased_notes_and_off_use_current_state() {
    let _scope = crate::test_scope::enter();
    let mut hub = Device::new(false);
    hub.activate();
    let (first, second) = install(&hub);
    hub.run(0, vec![note(1, 0, 64, 0, true), parameter("tuning-engine", 2.0, 0)], None);
    hub.run(512, vec![parameter("lattice-map", 1.0, 0)], None);
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 64).frozen_offset_microcents,
        first.correction(64, Tuning::just())
    );
    // A chased attack needs no remembered original onset.
    hub.run(1024, vec![note(2, 0, 76, 0, true)], None);
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 76).frozen_offset_microcents,
        second.correction(76, Tuning::just())
    );
    hub.run(
        1536,
        vec![parameter("tuning-engine", 0.0, 0), note(3, 0, 88, 0, true), expression(3, 0.25, 0)],
        None,
    );
    assert_eq!(voice(&hub, crate::tuning::DIRECT, 88).frozen_offset_microcents, 0);
    assert_eq!(voice(&hub, crate::tuning::DIRECT, 88).player_tuning, 0.25);
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 76).frozen_offset_microcents,
        second.correction(76, Tuning::just())
    );
}

#[test]
fn lattice_map_shared_axes_and_audition_apply_to_new_attacks_only() {
    use nice_plug::prelude::Param;
    let _scope = crate::test_scope::enter();
    let mut hub = Device::new(false);
    hub.activate();
    let (first, second) = install(&hub);
    hub.run(0, vec![parameter("tuning-engine", 2.0, 0)], None);
    let axis_value = hub_wrapper(&hub)
        .test_inspect_plugin(|plugin| plugin.params.three.preview_normalized(699.0));
    hub.run(
        512,
        vec![
            note(1, 0, 67, 9, true),
            note(2, 0, 79, 10, true),
            parameter("tuning-three", f64::from(axis_value), 10),
        ],
        None,
    );
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 67).frozen_offset_microcents,
        first.correction(67, Tuning::just())
    );
    assert!(
        (voice(&hub, crate::tuning::DIRECT, 79).frozen_offset_microcents + 1_000_000).abs() < 100
    );
    hub_wrapper(&hub)
        .test_inspect_plugin(|plugin| plugin.params.map_editor.lock().working = Some(second));
    hub.run(
        1024,
        vec![
            parameter("lattice-map", 0.0, 0),
            parameter("map-fifths", 4092.0, 0),
            parameter("map-thirds", 4099.0, 0),
            parameter("map-sevenths", 4094.0, 0),
            note(3, 0, 64, 0, true),
        ],
        None,
    );
    // Keep label rotation zero so the auditioned shape differs at MIDI E.
    let offset = LatticePos::new(-4, 3, -2);
    let auditioned = voice(&hub, crate::tuning::DIRECT, 64);
    assert_eq!(auditioned.attack_node, Some(LatticeMap { position: offset, ..second }.node(64)));
    hub_wrapper(&hub).test_inspect_plugin(|plugin| {
        assert_eq!(plugin.params.maps.read().map(0), Some(first));
        plugin.params.map_editor.lock().working = None;
    });
    hub.run(1536, vec![note(4, 0, 76, 0, true)], None);
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 76).attack_node,
        Some(LatticeMap { position: offset, ..first }.node(76))
    );
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 64).frozen_offset_microcents,
        auditioned.frozen_offset_microcents
    );
    // Map is an automatable stepped state selector, never additive modulation.
    let params = hub.params();
    let count = unsafe { params.count.unwrap()(hub.plugin) };
    let ids = ["lattice-map", "map-fifths", "map-thirds", "map-sevenths"];
    let mut found = 0;
    for index in 0..count {
        let mut info: clap_sys::ext::params::clap_param_info = unsafe { std::mem::zeroed() };
        assert!(unsafe { params.get_info.unwrap()(hub.plugin, index, &mut info) });
        if let Some(id) = ids.iter().find(|id| info.id == nice_plug::wrapper::hash_param_id(id)) {
            use clap_sys::ext::params::*;
            assert_ne!(info.flags & CLAP_PARAM_IS_STEPPED, 0);
            assert_ne!(info.flags & CLAP_PARAM_IS_AUTOMATABLE, 0);
            assert_eq!(info.flags & CLAP_PARAM_IS_MODULATABLE, 0);
            if *id != "lattice-map" {
                assert_eq!(info.min_value, 0.0);
                assert_eq!(info.max_value, 8192.0);
                assert_eq!(info.default_value, 4096.0);
            }
            found += 1;
        }
    }
    assert_eq!(found, ids.len());
}
