//! Actual endpoint ownership across the two return slots and a setup race.
use super::*;
use registry::SourceReturn;

pub(super) fn lease(source: &Device) -> Option<protocol::Lease> {
    let wrapper = unsafe {
        &*((*source.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    wrapper.test_inspect_plugin(|plugin| {
        plugin.source.as_ref().unwrap().offer.as_ref().map(|offer| offer.lease)
    })
}

#[test]
fn actual_adoption_and_detached_endpoint_fill_both_return_slots_without_main_service() {
    let _scope = crate::test_scope::enter();
    if std::env::var_os("HARMONIGRAPH_RETURN_SLOTS_CHILD").is_none() {
        // Any other factory's main-thread setup collects the process registry.
        // Isolate the explicit no-registry-drainer interval from other tests.
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "performance::tests::attachment_tests::actual_adoption_and_detached_endpoint_fill_both_return_slots_without_main_service", "--nocapture", "--test-threads=1"])
            .env("HARMONIGRAPH_RETURN_SLOTS_CHILD", "1")
            .status().unwrap().success());
        return;
    }
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    source.run(64, vec![], None);
    hub.run(64, vec![], None);
    let original = lease(&source).unwrap();
    let shared = source.shared();
    let bridge = shared.source.as_ref().unwrap();
    assert!(bridge.returns.reserve().is_some(), "the second real return slot is initially free");
    unsafe {
        (*hub.plugin).reset.unwrap()(hub.plugin);
    }
    for block in 2..=8 {
        source.run(block * 64, vec![], None);
        hub.run(block * 64, vec![], None);
    }
    assert_eq!(lease(&source), None);
    assert!(
        bridge.returns.reserve().is_none(),
        "the actual Adopted and Returned values occupy both slots"
    );
    let row = &session.rows[usize::from(original.slot - 1)];
    assert!(
        row.source_detached.load(Ordering::Acquire) && row.hub_detached.load(Ordering::Acquire)
    );
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    // Inspect and put back the same two whole values off audio. The second
    // contains the original owned rtrb endpoints, not a stand-in Arc.
    let adopted = bridge.returns.take().unwrap();
    let returned = bridge.returns.take().unwrap();
    assert!(matches!(&adopted, SourceReturn::Adopted { lease, .. } if *lease == original));
    assert!(matches!(&returned, SourceReturn::Returned(offer) if offer.lease == original));
    assert!(bridge.returns.publish(adopted).is_ok());
    assert!(bridge.returns.publish(returned).is_ok());
    source.run(9 * 64, vec![], None);
    hub.run(9 * 64, vec![], None);
    assert!(bridge.returns.reserve().is_none(), "audio cannot overwrite an unconsumed return");
    source.main();
    hub.main();
    assert!(bridge.returns.reserve().is_some());
}

#[test]
fn pairing_change_after_actual_offer_take_returns_the_whole_stale_bundle() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    let shared = source.shared();
    let bridge = shared.source.as_ref().unwrap();
    let generation = bridge.generation.load(Ordering::Acquire);
    let incarnation = session
        .rows
        .iter()
        .map(|row| row.expected_incarnation.load(Ordering::Acquire))
        .find(|value| *value != 0)
        .unwrap();
    shared.after_offer_take.enabled.store(true, Ordering::Release);
    std::thread::scope(|scope| {
        struct Resume<'a>(&'a setup::TestPause);
        impl Drop for Resume<'_> {
            fn drop(&mut self) {
                self.0.enabled.store(false, Ordering::Release);
            }
        }
        let _resume = Resume(&shared.after_offer_take);
        let address = (&source as *const Device) as usize;
        let callback = scope.spawn(move || {
            unsafe { &*(address as *const Device) }.run(
                0,
                vec![note(41, 2, 61, 5, true), note(41, 2, 61, 25, false)],
                None,
            )
        });
        wait_until(|| shared.after_offer_take.entered.load(Ordering::Acquire));
        let setup::Routing::Source(mut changed) = shared.value().routing else { unreachable!() };
        changed.selected = Some(SavedUuid::default());
        shared.apply(setup::Routing::Source(changed), false).unwrap();
        assert!(bridge.generation.load(Ordering::Acquire) > generation);
        shared.after_offer_take.enabled.store(false, Ordering::Release);
        assert!(callback.join().unwrap().values.is_empty());
    });
    assert_eq!(lease(&source), None);
    assert_eq!(source.source_snapshot().pending, 2);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    let returned = bridge.returns.take().unwrap();
    assert!(
        matches!(&returned, SourceReturn::Returned(offer) if offer.generation == generation && offer.lease.incarnation == incarnation)
    );
    assert!(bridge.returns.publish(returned).is_ok());
    source.main();
    hub.main();
    let setup::Routing::Source(mut changed) = shared.value().routing else { unreachable!() };
    changed.selected = Some(uuid);
    shared.apply(setup::Routing::Source(changed), false).unwrap();
    source.run(64, vec![], None);
    hub.run(64, vec![], None);
    let next = lease(&source).unwrap();
    assert_ne!(next.incarnation, incarnation);
    let played = source.run(128, vec![], None);
    assert_eq!(played.values.len(), 2);
    assert_eq!(
        played.values[1].0 - played.values[0].0,
        20,
        "the retained original duration survives the stale offer"
    );
    hub.run(128, vec![], None);
    source.run(192, vec![], None);
    hub.run(192, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
}
