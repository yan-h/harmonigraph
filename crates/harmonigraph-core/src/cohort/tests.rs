use super::*;

#[derive(Default)]
struct Capture(Vec<TargetLink>);

impl TargetAccess for Capture {
    fn get(&self, _: u8, handle: TargetHandle) -> Option<TargetLink> {
        self.0.get(handle as usize).copied()
    }
}

impl Capture {
    fn span(&mut self, targets: &[Target]) -> TargetSpan {
        if targets.is_empty() {
            return TargetSpan::default();
        }
        let first = self.0.len() as u32;
        for (index, &target) in targets.iter().enumerate() {
            // Deliberately noncontiguous; filler cells are never in the chain.
            self.0.push(TargetLink {
                target,
                next: if index + 1 < targets.len() {
                    first + 2 * (index as u32 + 1)
                } else {
                    NO_TARGET
                },
            });
            self.0.push(TargetLink { target: EMPTY_TARGET, next: NO_TARGET });
        }
        TargetSpan { first, len: targets.len() as u8 }
    }

    fn event(&mut self, source: u8, sequence: u64, kind: Kind, targets: &[Target]) -> Event {
        Event {
            id: InputId { source, sequence },
            sample: 50,
            kind,
            targets: self.span(targets),
            replaced: TargetSpan::default(),
        }
    }
}

fn voice(lifetime: u64, channel: u8, key: u8) -> Target {
    Target { lifetime, channel, key, host_note_id: lifetime as i32 }
}

fn tuning(value: f64) -> Kind {
    Kind::Tuning { value_bits: value.to_bits() }
}

fn run(cohort: &mut Cohort<'_, '_>, budget: usize) -> Vec<Selected> {
    let mut out = Vec::new();
    loop {
        let before = cohort.work().units();
        let progress = cohort.advance(budget).unwrap();
        assert!(cohort.work().units() - before <= budget as u64);
        match progress {
            Progress::Pending => {}
            Progress::Event(event) => {
                assert_eq!(cohort.advance(0), Ok(progress));
                assert_eq!(cohort.advance(100), Ok(progress));
                out.push(event);
                cohort.commit().unwrap();
            }
            Progress::Complete { vertices, .. } => {
                assert_eq!(cohort.committed(), vertices);
                assert_eq!(cohort.remaining(), 0);
                return out;
            }
        }
    }
}

#[test]
fn all_six_arrivals_use_one_assignment_chain() {
    let mut capture = Capture::default();
    let events = [
        capture.event(1, 1, Kind::Onset, &[voice(1, 0, 62)]),
        capture.event(2, 1, Kind::Onset, &[voice(1, 0, 65)]),
        capture.event(3, 1, Kind::Onset, &[voice(1, 0, 69)]),
    ];
    let mut scratch = Scratch::default();
    for permutation in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
        let permuted = permutation.map(|i| events[i]);
        let mut cohort = Cohort::begin(50, &permuted, &capture, &mut scratch).unwrap();
        let mut history = [0_u64; 3];
        let mut context = 0;
        let mut order = Vec::new();
        loop {
            match cohort.advance(3).unwrap() {
                Progress::Pending => {}
                Progress::Event(selected) => {
                    assert_eq!(selected.role, Role::TunerOnset);
                    let event = permuted[selected.event_index];
                    let key = u64::from(
                        capture.get(event.id.source, event.targets.first).unwrap().target.key,
                    );
                    // Artificial fixture only: its preceding assignment and
                    // history become visible before the next selection.
                    context = context * 100 + key;
                    history[usize::from(event.id.source - 1)] = context;
                    order.push(event.id.source);
                    cohort.commit().unwrap();
                }
                Progress::Complete { original_inputs: 3, vertices: 3 } => break,
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(order, [1, 2, 3]);
        assert_eq!(history, [62, 6265, 626569]);
        assert_ne!(history, [62, 65, 69], "independent snapshot evaluation must differ");
    }
}

#[test]
fn replacement_phases_preserve_identity_and_release_before_independent_context() {
    let mut capture = Capture::default();
    let old = voice(1, 0, 48);
    let new = voice(2, 0, 48);
    let pitch = capture.event(1, 9, tuning(0.25), &[old]);
    let mut replacement = capture.event(1, 10, Kind::Onset, &[new]);
    replacement.replaced = capture.span(&[old]);
    let independent = capture.event(1, 11, Kind::Onset, &[voice(3, 0, 40)]);
    // Still addressed to the captured OLD lifetime, after its choke. This must
    // wait for that release phase but need not wait for the new attack.
    let old_off = capture.event(1, 12, Kind::Terminal, &[old]);
    let events = [independent, old_off, replacement, pitch];
    let mut scratch = Scratch::default();
    let mut cohort = Cohort::begin(50, &events, &capture, &mut scratch).unwrap();
    let mut context = vec![old.lifetime];
    let mut visited = Vec::new();
    loop {
        match cohort.advance(7).unwrap() {
            Progress::Pending => {}
            Progress::Event(selected) => {
                visited.push((selected.id.sequence, selected.phase));
                match selected.phase {
                    EventPhase::ReplacedRelease => {
                        assert_eq!(selected.event_index, 2);
                        assert_eq!(selected.id, replacement.id);
                        assert_eq!(cohort.was_committed(2), Some(false));
                        context.clear();
                    }
                    EventPhase::Original if selected.role == Role::TunerOnset => {
                        assert!(!context.contains(&old.lifetime));
                        if selected.id == independent.id {
                            assert!(context.is_empty());
                        } else {
                            assert_eq!(context, [3]);
                        }
                        let event = events[selected.event_index];
                        context.push(capture.get(1, event.targets.first).unwrap().target.lifetime);
                    }
                    _ => {}
                }
                cohort.commit().unwrap();
            }
            Progress::Complete { original_inputs: 4, vertices: 5 } => break,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(
        visited,
        [
            (9, EventPhase::Original),
            (10, EventPhase::ReplacedRelease),
            (12, EventPhase::Original),
            (11, EventPhase::Original),
            (10, EventPhase::Original)
        ]
    );
    assert_eq!(cohort.was_committed(2), Some(true));
    assert_eq!(cohort.was_replacement_committed(2), Some(true));
}

#[test]
fn zero_duration_and_old_release_are_dependencies_not_a_global_source_chain() {
    let mut capture = Capture::default();
    let old = voice(1, 0, 60);
    let new = voice(2, 0, 60);
    let events = [
        capture.event(1, 1, Kind::Onset, &[old]),
        capture.event(1, 2, Kind::Terminal, &[old]),
        capture.event(1, 3, Kind::Onset, &[new]),
        capture.event(1, 4, Kind::Onset, &[voice(3, 0, 40)]),
        capture.event(2, 1, tuning(0.5), &[voice(99, 0, 90)]),
        capture.event(3, 1, Kind::Terminal, &[voice(99, 0, 91)]),
    ];
    let mut scratch = Scratch::default();
    let mut cohort = Cohort::begin(50, &events, &capture, &mut scratch).unwrap();
    assert_eq!(
        run(&mut cohort, 1).iter().map(|s| s.event_index).collect::<Vec<_>>(),
        [4, 5, 3, 0, 1, 2]
    );
    assert_eq!(cohort.work().built_edges, 2);
}

#[test]
fn shared_channel_pedal_and_captured_wildcards_leave_other_channels_independent() {
    let mut capture = Capture::default();
    let a = voice(10, 0, 70);
    let b = voice(20, 0, 71);
    let later = voice(30, 0, 70);
    let events = [
        capture.event(1, 1, Kind::Onset, &[a]),
        capture.event(1, 2, Kind::Onset, &[b]),
        capture.event(1, 3, Kind::Channel { channel: 0, terminal: false }, &[]),
        // Noncontiguous, unsorted captured set. No later retrigger in the set.
        capture.event(1, 4, Kind::Terminal, &[b, a]),
        capture.event(1, 5, Kind::Onset, &[later]),
        capture.event(1, 6, Kind::Onset, &[voice(40, 1, 30)]),
        capture.event(1, 7, tuning(-0.25), &[a]),
    ];
    let mut scratch = Scratch::default();
    let mut cohort = Cohort::begin(50, &events, &capture, &mut scratch).unwrap();
    let order = run(&mut cohort, 11);
    assert_eq!(order.iter().map(|s| s.event_index).collect::<Vec<_>>(), [5, 0, 1, 2, 3, 6, 4]);
    assert!(order.last().unwrap().initial_tuning.is_none());
    assert_eq!(events[3].targets.len, 2);
    assert_eq!(capture.get(1, events[3].targets.first).unwrap().target, b);
}

#[test]
fn initial_tuning_is_first_exact_own_lifetime_value_before_terminal() {
    let mut capture = Capture::default();
    let a = voice(1, 0, 60);
    let b = voice(2, 0, 61);
    let c = voice(3, 0, 62);
    let precise = f64::from_bits(0x3ff0000000000001);
    assert_ne!(precise, f64::from(precise as f32));
    let events = [
        capture.event(1, 1, Kind::Onset, &[a]),
        capture.event(1, 2, Kind::Expression, &[a]),
        capture.event(1, 3, tuning(precise), &[a]),
        capture.event(1, 4, tuning(-0.0), &[a]),
        capture.event(1, 5, Kind::Terminal, &[a]),
        capture.event(1, 6, tuning(2.0), &[a]),
        capture.event(1, 7, Kind::Onset, &[b]),
        capture.event(1, 8, Kind::Terminal, &[b]),
        capture.event(1, 9, tuning(3.0), &[b]),
        capture.event(0, 1, Kind::Onset, &[c]),
    ];
    let mut scratch = Scratch::default();
    let mut cohort = Cohort::begin(50, &events, &capture, &mut scratch).unwrap();
    let out = run(&mut cohort, 9);
    assert_eq!(
        out[0].initial_tuning,
        Some(InitialTuning { event_index: 2, id: events[2].id, value_bits: precise.to_bits() })
    );
    assert_eq!(
        out.iter().filter(|s| s.id.source == 1).map(|s| s.event_index).collect::<Vec<_>>(),
        (0..9).collect::<Vec<_>>()
    );
    assert_eq!(out.iter().find(|s| s.event_index == 6).unwrap().initial_tuning, None);
    let direct = out.iter().find(|s| s.event_index == 9).unwrap();
    assert_eq!(direct.role, Role::ObservedDirectOnset);
    assert_eq!(direct.initial_tuning, None);
    // A later sample cannot be slipped into a same-sample cohort at all.
    let mut later = events;
    later[2].sample += 1;
    let mut invalid = Cohort::begin(50, &later, &capture, &mut scratch).unwrap();
    assert_eq!(invalid.advance(usize::MAX), Err(Error::InvalidMetadata { event: 2 }));
    assert_eq!(invalid.committed(), 0);
}

#[test]
fn resume_and_reset_preserve_stable_offers_and_owned_remaining_events() {
    let mut capture = Capture::default();
    let old = voice(1, 0, 60);
    let mut replacement = capture.event(1, 5, Kind::Onset, &[voice(2, 0, 60)]);
    replacement.replaced = capture.span(&[old]);
    let events = [
        replacement,
        capture.event(2, 1, Kind::Onset, &[voice(1, 0, 40)]),
        capture.event(1, 6, tuning(0.25), &[voice(2, 0, 60)]),
    ];
    let mut scratch = Scratch::default();
    let mut cohort = Cohort::begin(50, &events, &capture, &mut scratch).unwrap();
    let full = run(&mut cohort, usize::MAX);
    let full_work = cohort.work();
    cohort.reset();
    assert_eq!(cohort.was_committed(0), Some(false));
    assert_eq!(cohort.was_replacement_committed(0), Some(false));
    assert_eq!(cohort.commit(), Err(Error::NoSelectedEvent));
    let first = loop {
        if let Progress::Event(selected) = cohort.advance(1).unwrap() {
            break selected;
        }
    };
    assert_eq!(first, full[0]);
    assert_eq!(cohort.remaining(), 4);
    cohort.commit().unwrap();
    assert_eq!(cohort.was_committed(0), Some(false));
    assert_eq!(cohort.was_replacement_committed(0), Some(true));
    assert_eq!(cohort.remaining(), 3);
    let mut resumed = vec![first];
    resumed.extend(run(&mut cohort, 1));
    assert_eq!(resumed, full);
    assert_eq!(cohort.work(), full_work);
}

#[test]
fn malformed_metadata_is_rejected_before_any_selection() {
    let mut capture = Capture::default();
    let target = voice(1, 0, 60);
    let a = capture.event(1, 1, Kind::Onset, &[target]);
    let mut scratch = Scratch::default();
    let error = |events: &[Event], scratch: &mut Scratch| {
        let mut cohort = Cohort::begin(50, events, &capture, scratch).unwrap();
        let result = cohort.advance(usize::MAX);
        assert_eq!(cohort.committed(), 0);
        assert_eq!(cohort.advance(0), result, "error is latched");
        result
    };
    assert!(matches!(error(&[a, a], &mut scratch), Err(Error::DuplicateInput { .. })));
    let mut duplicate_on = a;
    duplicate_on.id.sequence += 1;
    assert!(matches!(error(&[a, duplicate_on], &mut scratch), Err(Error::DuplicateOnset { .. })));
    let mut before = a;
    before.kind = tuning(0.1);
    assert!(matches!(
        error(&[before, duplicate_on], &mut scratch),
        Err(Error::TargetBeforeOnset { .. })
    ));
    for invalid in [
        Event { id: InputId { source: 17, sequence: 1 }, ..a },
        Event { id: InputId { source: 1, sequence: 0 }, ..a },
        Event { kind: tuning(f64::NAN), ..a },
    ] {
        assert!(matches!(error(&[invalid], &mut scratch), Err(Error::InvalidMetadata { .. })));
    }
    let mut changed = target;
    changed.key = 61;
    let expression = capture.event(1, 2, tuning(0.2), &[changed]);
    let changed_events = [a, expression];
    let mut cohort = Cohort::begin(50, &changed_events, &capture, &mut scratch).unwrap();
    assert!(matches!(cohort.advance(usize::MAX), Err(Error::InconsistentLifetime { .. })));
    let duplicate = capture.event(1, 1, Kind::Terminal, &[target, target]);
    let duplicate_events = [duplicate];
    let mut cohort = Cohort::begin(50, &duplicate_events, &capture, &mut scratch).unwrap();
    assert_eq!(cohort.advance(usize::MAX), Err(Error::DuplicateTarget { event: 0 }));
}

#[test]
fn all_seventeen_source_ties_include_direct_without_a_plan() {
    let mut capture = Capture::default();
    let events: Vec<_> = (0..SOURCE_ORDINALS)
        .rev()
        .map(|source| capture.event(source, 1, Kind::Onset, &[voice(1, 0, 60)]))
        .collect();
    let mut scratch = Scratch::default();
    let mut cohort = Cohort::begin(50, &events, &capture, &mut scratch).unwrap();
    let out = run(&mut cohort, 13);
    assert_eq!(out.iter().map(|s| s.id.source).collect::<Vec<_>>(), (0..17).collect::<Vec<_>>());
    assert_eq!(out[0].role, Role::ObservedDirectOnset);
    assert!(out[1..].iter().all(|s| s.role == Role::TunerOnset));
}

#[test]
fn bad_captured_chains_addresses_and_replacements_are_explicit_errors() {
    let mut capture = Capture::default();
    let long: Vec<_> = (1..=65).map(|i| voice(i, 0, i as u8)).collect();
    let too_many = capture.event(1, 1, Kind::Expression, &long);
    let mut bad_key = voice(1, 0, 60);
    bad_key.key = 128;
    let invalid_key = capture.event(1, 1, Kind::Terminal, &[bad_key]);
    let wrong_channel =
        capture.event(1, 1, Kind::Channel { channel: 1, terminal: true }, &[voice(1, 0, 60)]);
    let mut replacement = capture.event(1, 1, Kind::Onset, &[voice(2, 0, 60)]);
    replacement.replaced = capture.span(&[voice(1, 0, 61)]);
    let bad_chain = Event { targets: TargetSpan { first: NO_TARGET, len: 1 }, ..too_many };
    let missing_handle = Event { targets: TargetSpan { first: 900_000, len: 1 }, ..too_many };
    let short_chain = Event { targets: TargetSpan { len: 1, ..too_many.targets }, ..too_many };
    let mut scratch = Scratch::default();
    for (event, expected) in [
        (too_many, Error::TooManyTargets { event: 0 }),
        (invalid_key, Error::InvalidTarget { event: 0 }),
        (wrong_channel, Error::InvalidTarget { event: 0 }),
        (replacement, Error::InvalidReplacement { event: 0 }),
        (bad_chain, Error::InvalidTargetSpan { event: 0 }),
        (missing_handle, Error::InvalidTargetSpan { event: 0 }),
        (short_chain, Error::InvalidTargetSpan { event: 0 }),
    ] {
        let events = [event];
        let mut cohort = Cohort::begin(50, &events, &capture, &mut scratch).unwrap();
        assert_eq!(cohort.advance(usize::MAX), Err(expected));
        assert_eq!(cohort.committed(), 0);
    }
    let mut empty = Cohort::begin(50, &[], &capture, &mut scratch).unwrap();
    assert_eq!(empty.advance(0), Ok(Progress::Complete { vertices: 0, original_inputs: 0 }));
    assert_eq!(empty.work().units(), 0);
}
