use super::*;
use harmonigraph_take::{CanonicalRecord, NoteKind};

fn notes(on: bool, time: u32) -> Vec<Input> {
    (0..64).map(|key| note(key + 1, 2, key as i16, time, on)).collect()
}
fn tunings(time: u32) -> Vec<Input> {
    (0..64).map(|key| expression(key + 1, 0.234567890123, time)).collect()
}

#[test]
fn full_primary_publication_does_not_block_three_sources_actual_releases_and_credit_retirement() {
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut a = Device::new(true);
    a.configure(uuid, true);
    a.activate();
    let mut b = Device::new(true);
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

#[test]
fn off_rejoin_full_held_baseline_preserves_the_complete_original_take_lifetimes() {
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, false);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.arm();
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-full-rejoin-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("rejoin.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    assert_eq!(source.run(64, notes(true, 5), None).values.len(), 64);
    hub.run(64, vec![], None);
    writer.drain(&mut capture);
    assert_eq!(source.run(128, tunings(7), None).values.len(), 64);
    source.run(192, vec![source.participation(true, 0)], None);
    assert_eq!(source.run(256, notes(false, 9), None).values.len(), 64);
    assert_eq!(session.credits.load(Ordering::Acquire), 64);
    for block in 2..=7 {
        hub.run(block * 64, vec![], None);
        source.run((block + 3) * 64, vec![], None);
        writer.drain(&mut capture);
    }
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    capture.stop();
    writer.stop();
    hub.run(8 * 64, vec![], None);
    source.run(11 * 64, vec![], None);
    hub.run(9 * 64, vec![], None);
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert!(writer.finished.is_some());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_none());
    let baseline_position = take.events.iter().position(|record| matches!(record, CanonicalRecord::Baseline(frame) if frame.participating && frame.voices.len() == 64)).expect("complete 64-voice rejoin baseline");
    let CanonicalRecord::Baseline(frame) = &take.events[baseline_position] else { unreachable!() };
    let baseline = frame.baseline().unwrap();
    assert!(baseline
        .voices()
        .iter()
        .all(|voice| voice.player_tuning == 0.234567890123 && voice.onset.unwrap().sample == 69));
    let deltas: Vec<_> = take
        .events
        .iter()
        .enumerate()
        .filter_map(|(index, record)| match record {
            CanonicalRecord::Delta(delta) => Some((index, delta)),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 192);
    let mut tracker = harmonigraph_core::NoteTracker::default();
    for key in 0..64 {
        let voice: Vec<_> = deltas.iter().filter(|(_, delta)| delta.event.note == key).collect();
        assert_eq!(voice.len(), 3);
        assert!(matches!(voice[0].1.event.kind, NoteKind::On { .. }));
        assert!(matches!(voice[1].1.event.kind, NoteKind::Tuning { .. }));
        assert!(matches!(voice[2].1.event.kind, NoteKind::Off));
        assert_eq!(
            (voice[0].1.lifetime, voice[1].1.lifetime, voice[2].1.lifetime),
            (voice[0].1.lifetime, voice[0].1.lifetime, voice[0].1.lifetime)
        );
        assert!(voice[1].0 < baseline_position && baseline_position < voice[2].0);
        assert_eq!(
            (
                voice[0].1.timing.unwrap().sample,
                voice[1].1.timing.unwrap().sample,
                voice[2].1.timing.unwrap().sample
            ),
            (69, 135, 265)
        );
    }
    for record in take.events {
        record.apply(&mut tracker).unwrap();
    }
    assert_eq!(tracker.held_count(), 0);
    drop(writer);
    drop(source);
    drop(hub);
    std::fs::remove_dir_all(directory).unwrap();
}
