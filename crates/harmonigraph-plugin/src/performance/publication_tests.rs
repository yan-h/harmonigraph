use super::*;
use harmonigraph_take::CanonicalRecord;

fn notes(on: bool, time: u32) -> Vec<Input> {
    (0..64).map(|key| note(key + 1, 2, key as i16, time, on)).collect()
}
fn tunings(time: u32) -> Vec<Input> {
    (0..64).map(|key| expression(key + 1, 0.234567890123, time)).collect()
}

#[test]
fn full_primary_publication_does_not_block_three_sources_actual_releases_and_credit_retirement() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_aggregation_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut a = Device::aggregation(true);
    a.configure(uuid, true);
    a.activate();
    let mut b = Device::aggregation(true);
    b.configure(uuid, false);
    b.activate();
    a.run(0, vec![], None);
    b.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.arm();
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-full-primary-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("primary.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    assert_eq!(a.run(64, notes(true, 5), None).values.len(), 64);
    assert_eq!(b.run(64, notes(true, 5), None).values.len(), 64);
    assert_eq!(hub.run(64, notes(true, 5), None).values.len(), 64);
    assert_eq!(session.credits.load(Ordering::Acquire), 192);
    // More than the actual 4096-cell primary publication ring, with no drain
    // by either the file or display consumer. All three actual owners progress.
    for block in 2..=33 {
        assert_eq!(a.run(block * 64, tunings(7), None).values.len(), 64);
        assert_eq!(b.run(block * 64, tunings(7), None).values.len(), 64);
        assert_eq!(hub.run(block * 64, tunings(7), None).values.len(), 64);
    }
    assert_eq!(a.run(34 * 64, notes(false, 9), None).values.len(), 64);
    assert_eq!(b.run(34 * 64, notes(false, 9), None).values.len(), 64);
    assert_eq!(hub.run(34 * 64, notes(false, 9), None).values.len(), 64);
    a.run(35 * 64, vec![], None);
    b.run(35 * 64, vec![], None);
    hub.run(35 * 64, vec![], None);
    assert_eq!(
        session.credits.load(Ordering::Acquire),
        0,
        "musical acknowledgement does not wait for publication capacity"
    );
    for source in [&a, &b] {
        let settled = source.source_snapshot();
        assert_eq!(
            (settled.held, settled.journal, settled.emergency, settled.faults),
            (0, 0, 0, 0)
        );
    }
    capture.stop();
    writer.stop();
    a.run(36 * 64, vec![], None);
    b.run(36 * 64, vec![], None);
    hub.run(36 * 64, vec![], None);
    let (actual_loss, actual_route) =
        capture.publication_loss().expect("actual primary loss descriptor");
    assert_eq!(actual_route.address, None, "the outage spans recording and disarmed closure");
    writer.drain(&mut capture);
    assert!(writer.failed());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let incomplete = take.incomplete.as_ref().unwrap();
    assert_eq!(incomplete.reason, harmonigraph_take::canonical::GapReasonRecord::PublicationFull);
    assert_eq!(
        (incomplete.first_publication, incomplete.last_publication),
        (actual_loss.first, actual_loss.last)
    );
    assert!(
        incomplete.first_publication > 0
            && incomplete.last_publication >= incomplete.first_publication
    );
    // Loss spans armed output and the later disarmed closure. The global loss
    // descriptor has no single pass route: each affected file gets the exact
    // incomplete range, and the display receives the canonical gap.
    // The stalled display ring can independently fill during this fanout and
    // report its own sequence namespace. Compare the file to its primary loss.
    assert!(writer.display_events().iter().any(|record| matches!(record, CanonicalRecord::Gap(gap) if gap.reason == harmonigraph_take::canonical::GapReasonRecord::PublicationFull)));
    drop(writer);
    drop(a);
    drop(b);
    drop(hub);
    std::fs::remove_dir_all(directory).unwrap();
}
