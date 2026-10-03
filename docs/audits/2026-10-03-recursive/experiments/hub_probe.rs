#[cfg(test)]
#[test]
fn audit_epoch_rejection_does_not_spend_the_collection_budget() {
    fn collect(epoch: u64) -> (usize, usize) {
        let mut hub = Hub::new();
        hub.epoch = 1;
        let mut ends: Box<[Option<HubEnds>; TUNERS]> = Box::new(std::array::from_fn(|_| None));
        let mut producers = Vec::new();
        for slot in 0..3 {
            let (mut tx, rx) = rtrb::RingBuffer::new(super::CAPTURE_RING);
            let (reply_tx, _reply_rx) = rtrb::RingBuffer::new(super::REPLY_RING);
            for serial in 0..super::CAPTURE_RING {
                assert!(tx.push(session::Capture {
                    retune: 1, epoch, serial: serial as u64 + 1, sample: serial as i64,
                    event: Event::Midi { port: 0, data: [0xb0, 7, 1], flags: 0 },
                }).is_ok());
            }
            ends[slot] = Some(HubEnds { captures: rx, replies: reply_tx });
            producers.push(tx);
        }
        hub.ends = Some(ends);
        let initial: usize = hub.ends.as_ref().unwrap().iter().flatten().map(|end| end.captures.slots()).sum();
        assert_eq!(initial, 3 * super::CAPTURE_RING);
        hub.collect();
        let remaining: usize = hub.ends.as_ref().unwrap().iter().flatten().map(|end| end.captures.slots()).sum();
        let result = (initial - remaining, hub.batch.len());
        drop(producers);
        result
    }
    let matching = collect(1);
    assert_eq!(matching, (BATCH_EVENTS, BATCH_EVENTS));
    let mismatched = collect(2);
    assert_eq!(mismatched, (3 * super::CAPTURE_RING, 0));
    assert!(mismatched.0 > BATCH_EVENTS);
    eprintln!("matching: {} popped, {} retained; mismatched: {} popped, {} retained", matching.0, matching.1, mismatched.0, mismatched.1);
}
