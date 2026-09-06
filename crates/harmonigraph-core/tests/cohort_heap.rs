//! Separate binary: the policy unit-test binary already owns a global allocator.
use harmonigraph_core::cohort::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::mem::{align_of, size_of};

struct Allocator;
thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
}
fn count() {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        let _ = CALLS.try_with(|calls| calls.set(calls.get() + 1));
    }
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        count();
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn without_heap<T>(run: impl FnOnce() -> T) -> T {
    struct Stop;
    impl Drop for Stop {
        fn drop(&mut self) {
            ACTIVE.set(false);
        }
    }
    CALLS.set(0);
    ACTIVE.set(true);
    let stop = Stop;
    let result = run();
    drop(stop);
    assert_eq!(CALLS.get(), 0, "cohort allocated/reallocated/deallocated");
    result
}

struct Capture(Vec<TargetLink>);
impl TargetAccess for Capture {
    fn get(&self, _: u8, handle: TargetHandle) -> Option<TargetLink> {
        self.0.get(handle as usize).copied()
    }
}

fn complete(cohort: &mut Cohort<'_, '_>, expected_inputs: usize, expected_phases: usize) -> Work {
    loop {
        let before = cohort.work().units();
        let progress = cohort.advance(8191).unwrap();
        assert!(cohort.work().units() - before <= 8191);
        match progress {
            Progress::Pending => {}
            Progress::Event(selected) => {
                assert_eq!(cohort.advance(0), Ok(progress));
                if selected.phase == EventPhase::Original {
                    assert_eq!(cohort.was_committed(selected.event_index), Some(false));
                    if selected.role != Role::Event {
                        assert_eq!(
                            cohort.was_replacement_committed(selected.event_index),
                            Some(true)
                        );
                    }
                }
                cohort.commit().unwrap();
            }
            Progress::Complete { original_inputs, vertices } => {
                assert_eq!(original_inputs, expected_inputs);
                assert_eq!(vertices, expected_phases);
                assert_eq!(cohort.committed(), expected_phases);
                assert_eq!(cohort.remaining(), 0);
                return cohort.work();
            }
        }
    }
}

#[test]
fn maximum_original_inputs_retriggers_edges_reset_and_overflow_use_no_heap() {
    let mut capture = Capture(Vec::new());
    let mut events = Vec::new();
    // Four valid source-held sets of 64, replaced at this sample. Every physical
    // onset retains one old-lifetime termination phase and one new lifetime.
    for i in 0..COHORT_ONSETS {
        let source = (i / 64) as u8;
        let key = (i % 64) as u8;
        let old = Target { lifetime: u64::from(key) + 1, host_note_id: -1, channel: 0, key };
        let new = Target { lifetime: old.lifetime + 64, ..old };
        capture.0.push(TargetLink { target: new, next: NO_TARGET });
        capture.0.push(TargetLink { target: old, next: NO_TARGET });
        events.push(Event {
            id: InputId { source, sequence: u64::from(key) + 1 },
            sample: 0,
            kind: Kind::Onset,
            targets: TargetSpan { first: (2 * i) as u32, len: 1 },
            replaced: TargetSpan { first: (2 * i + 1) as u32, len: 1 },
        });
    }
    // 768 real shared-channel nodes produce a dense graph: their mutual edges
    // plus edges from both phases of source 1's 64 replacements.
    for i in 0..(COHORT_EVENTS - COHORT_ONSETS) {
        events.push(Event {
            id: InputId { source: 1, sequence: 65 + i as u64 },
            sample: 0,
            kind: Kind::Channel { channel: 0, terminal: false },
            targets: TargetSpan::default(),
            replaced: TargetSpan::default(),
        });
    }
    let mut overflow = events.clone();
    overflow.push(Event {
        id: InputId { source: 16, sequence: 1 },
        kind: Kind::Independent,
        targets: TargetSpan::default(),
        replaced: TargetSpan::default(),
        sample: 0,
    });
    let mut onset_overflow = events.clone();
    onset_overflow[256] = Event { id: InputId { source: 16, sequence: 1 }, ..events[0] };
    let mut scratch = Scratch::default();
    let work = without_heap(|| {
        let first = {
            let mut cohort = Cohort::begin(0, &events, &capture, &mut scratch).unwrap();
            let first = complete(&mut cohort, 1024, 1280);
            cohort.reset();
            assert_eq!(cohort.was_committed(0), Some(false));
            assert_eq!(cohort.was_replacement_committed(0), Some(false));
            assert_eq!(complete(&mut cohort, 1024, 1280), first);
            first
        };
        assert!(matches!(
            Cohort::begin(0, &overflow, &capture, &mut scratch),
            Err(Error::TooManyEvents)
        ));
        let mut over = Cohort::begin(0, &onset_overflow, &capture, &mut scratch).unwrap();
        assert_eq!(over.advance(usize::MAX), Err(Error::TooManyOnsets));
        assert_eq!(over.committed(), 0);
        assert_eq!(over.advance(0), Err(Error::TooManyOnsets));
        first
    });
    assert_eq!(work.prepared_inputs, 1024);
    assert_eq!(work.validated_nodes, 1280);
    assert_eq!(work.validated_targets, 512);
    assert_eq!(work.node_pairs, 1280 * 1279 / 2);
    assert_eq!(work.built_edges, 768 * 767 / 2 + 2 * 64 * 768 + 256);
    assert_eq!(work.retired_edges, work.built_edges);
    assert_eq!(work.committed_nodes, 1280);
    assert_eq!(work.selection_visits, 1280 * 1280);
    assert_eq!(work.cleared_words, 1280 * 20);
    assert_eq!(work.retired_words, 1280 * 20);
    let total = size_of::<([Option<Event>; COHORT_EVENTS], Scratch, Cohort<'_, '_>)>();
    assert!(size_of::<Option<Event>>() <= COMPACT_EVENT_BYTES);
    assert!(
        size_of::<Scratch>() + size_of::<Cohort<'_, '_>>() <= ADJACENCY_BYTES + TRAVERSAL_BYTES
    );
    assert!(total <= COHORT_BYTES);
    println!("Event={} Option<Event>={} Target={} Scratch={} Cohort={} alignment={} total={total}; {work:?}",
        size_of::<Event>(), size_of::<Option<Event>>(), size_of::<Target>(), size_of::<Scratch>(),
        size_of::<Cohort<'_, '_>>(), align_of::<Scratch>());
}

#[test]
fn maximum_unsorted_target_spans_and_full_adjacency_are_charged_without_heap() {
    // All 1024 messages address the same original 64 held lifetimes. Chains are
    // reverse-sorted and noncontiguous, exercising every local insertion shift.
    let mut links = Vec::new();
    for i in 0..64_u32 {
        let target =
            Target { lifetime: 64 - u64::from(i), host_note_id: -1, channel: 0, key: i as u8 };
        links.push(TargetLink { target, next: if i == 63 { NO_TARGET } else { 2 * (i + 1) } });
        links.push(TargetLink { target, next: NO_TARGET });
    }
    let capture = Capture(links);
    // Distinct physical linked cells, as with repeated source captures. This
    // test-only frozen owner is created before the allocation guard; its size
    // is not a proposed production transport allocation.
    let mut distinct = Capture(Vec::with_capacity(1024 * 128));
    for parent in 0..1024 {
        for original in &capture.0 {
            let mut link = *original;
            if link.next != NO_TARGET {
                link.next += parent * 128;
            }
            distinct.0.push(link);
        }
    }
    let mut scratch = Scratch::default();
    let pairs = 1024 * 1023 / 2;
    for separate_chains in [false, true] {
        let events: Vec<_> = (0..1024)
            .map(|i| Event {
                id: InputId { source: 1, sequence: i + 1 },
                sample: 0,
                kind: Kind::Expression,
                targets: TargetSpan {
                    first: if separate_chains { i as u32 * 128 } else { 0 },
                    len: 64,
                },
                replaced: TargetSpan::default(),
            })
            .collect();
        let provider: &dyn TargetAccess = if separate_chains { &distinct } else { &capture };
        let work = without_heap(|| {
            let mut cohort = Cohort::begin(0, &events, provider, &mut scratch).unwrap();
            let work = complete(&mut cohort, 1024, 1024);
            cohort.reset();
            assert_eq!(cohort.committed(), 0);
            work
        });
        assert_eq!(work.validated_targets, 1024 * 64);
        assert_eq!(work.node_pairs, pairs);
        let compared_pairs = if separate_chains { pairs } else { 0 };
        assert_eq!(work.identical_target_pairs, pairs - compared_pairs);
        assert_eq!(work.target_reads, (1024 + compared_pairs) * 64);
        // Reverse insertion costs 1+...+64; binary searches over 64 sorted entries
        // cost sum(depth) = 1+4+12+32+80+192+7 = 328 for each compared target set.
        assert_eq!(work.target_steps, 1024 * (64 * 65 / 2) + compared_pairs * 328);
        assert_eq!(work.built_edges, pairs);
        assert_eq!(work.retired_edges, pairs);
        assert_eq!(work.selection_visits, 1024 * 1024);
        println!("maximum target chain (distinct handles={separate_chains}): {work:?}");
    }
}
