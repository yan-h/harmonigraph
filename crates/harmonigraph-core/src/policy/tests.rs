use super::*;
use crate::configuration::{ConfigReducer, TuningModes};
use crate::tuning::OCTAVE_MICROCENTS;

fn just() -> MusicalConfig {
    // Explicit independent Just axes: default configuration engages 12-TET locks.
    ConfigReducer::new(
        Tuning::just(),
        TuningModes { tempered: Tempered::default(), auto: [false; 2], learning: false },
    )
    .resolved()
    .into()
}

fn custom(three: f32, five: f32, seven: f32) -> MusicalConfig {
    ConfigReducer::new(
        Tuning::from_cents(0.0, three, five, seven, 0.5),
        TuningModes { tempered: Tempered::default(), auto: [false; 2], learning: false },
    )
    .resolved()
    .into()
}

fn at(config: MusicalConfig, node: LatticePos) -> ContextPitch {
    ContextPitch { pitch: config.tuning().pitch_class(node), node: Some(node) }
}

fn choose(
    config: MusicalConfig,
    context: &[ContextPitch],
    history: Option<LatticePos>,
    key: u8,
    scratch: &mut PolicyScratch,
) -> Decision {
    assign_new_note(config, context, history, OrderedOnset { key }, scratch).unwrap()
}

fn selected(decision: Decision) -> (LatticePos, i32) {
    match decision.assignment {
        Assignment::Selected { node, correction_microcents } => {
            assert_eq!(decision.history_update, HistoryUpdate::Set(node));
            assert!(correction_microcents.unsigned_abs() <= CANDIDATE_RADIUS);
            (node, correction_microcents)
        }
        Assignment::NoCandidate => panic!("fixture must reach selected assignment"),
    }
}

fn candidates(scratch: &PolicyScratch) -> impl Iterator<Item = LatticePos> + '_ {
    scratch.candidates[..scratch.candidate_len]
        .iter()
        .map(|&i| scratch.domain[usize::from(i)].position)
}

#[test]
fn locked_default_is_zero_for_every_key_and_discards_display_inputs() {
    let resolved = ConfigReducer::default().resolved();
    assert_eq!(resolved.modes.tempered, Tempered { syntonic: true, septimal_kleisma: true });
    let config = MusicalConfig::from(resolved);
    let mut changed = resolved;
    changed.tuning.tolerance = i32::MAX;
    changed.revision += 1;
    changed.modes.auto = [false; 2];
    changed.modes.learning = true;
    assert_eq!(MusicalConfig::from(changed), config);
    let context = [at(config, LatticePos::ORIGIN)];
    let mut scratch = PolicyScratch::default();
    for key in 0..=127 {
        assert_eq!(selected(choose(config, &context, None, key, &mut scratch)).1, 0);
        assert_eq!(scratch.domain_len, 29);
    }
}

#[test]
fn held_c_g_admits_both_major_thirds_and_selects_five_four() {
    let config = just();
    let context = [at(config, LatticePos::ORIGIN), at(config, LatticePos::new(1, 0, 0))];
    let mut scratch = PolicyScratch::default();
    let result = choose(config, &context, None, 64, &mut scratch);
    let just_e = LatticePos::new(0, 1, 0);
    let pythagorean_e = LatticePos::new(4, 0, 0);
    assert!(candidates(&scratch).any(|p| p == just_e));
    assert!(candidates(&scratch).any(|p| p == pythagorean_e));
    assert_eq!(score(just_e, &scratch.context[..2], None), 12);
    assert_eq!(score(pythagorean_e, &scratch.context[..2], None), 28);
    assert_eq!(selected(result), (just_e, config.axes[1] - 400_000_000));
}

#[test]
fn d_f_a_incorporates_each_predecessor_into_a_five_limit_minor_triad() {
    let config = just();
    let mut scratch = PolicyScratch::default();
    let empty = ContextPitch { pitch: PitchClass::from_microcents(0), node: None };
    let mut context = [empty; 3];
    let expected = [LatticePos::new(2, 0, 0), LatticePos::new(3, -1, 0), LatticePos::new(3, 0, 0)];
    for (i, key) in [62, 65, 69].into_iter().enumerate() {
        let (node, _) = selected(choose(config, &context[..i], None, key, &mut scratch));
        assert!(candidates(&scratch).any(|p| p == expected[i]));
        assert_eq!(node, expected[i]);
        context[i] = at(config, node);
    }
    // 9/8, 27/20, 27/16. Exact differences of quantized core axes,
    // not a float logarithm pretending to equal those quantized values.
    let octave = i64::from(OCTAVE_MICROCENTS);
    assert_eq!(
        context[0].pitch,
        PitchClass::from_microcents(2 * i64::from(config.axes[0]) - octave)
    );
    assert_eq!(
        context[1].pitch - context[0].pitch,
        config.tuning().pitch_class(LatticePos::new(1, -1, 0))
    ); // 6/5
    assert_eq!(
        context[2].pitch - context[0].pitch,
        config.tuning().pitch_class(LatticePos::new(1, 0, 0))
    ); // 3/2
    assert_ne!(selected(choose(config, &[], None, 65, &mut scratch)).0, expected[1]);
}

#[test]
fn ii_v_i_keeps_common_tones_and_bounds_each_key_relative_correction() {
    let config = just();
    let mut scratch = PolicyScratch::default();
    let mut context = [at(config, LatticePos::ORIGIN); 3];
    // ii: D F A; keep D into V, then keep G into I. Each phase's new
    // independent onsets are in key order and see preceding assignments.
    let phases: [(&[u8], &[LatticePos]); 3] = [
        (
            &[50, 53, 57],
            &[LatticePos::new(2, 0, 0), LatticePos::new(3, -1, 0), LatticePos::new(3, 0, 0)],
        ),
        (&[55, 59], &[LatticePos::new(1, 0, 0), LatticePos::new(1, 1, 0)]),
        (&[48, 52], &[LatticePos::ORIGIN, LatticePos::new(0, 1, 0)]),
    ];
    let mut len = 0;
    for (phase, (keys, nodes)) in phases.into_iter().enumerate() {
        for (&key, &expected) in keys.iter().zip(nodes) {
            let decision = choose(config, &context[..len], None, key, &mut scratch);
            let (node, correction) = selected(decision);
            assert_eq!(node, expected);
            assert_eq!(
                PitchClass::from_microcents(i64::from(key) * 100_000_000 + i64::from(correction)),
                config.tuning().pitch_class(expected)
            );
            println!("phase={phase} key={key} node={node:?} correction_microcents={correction}");
            context[len] = at(config, node);
            len += 1;
        }
        if phase == 1 {
            context[0] = context[1];
        } // V's G survives into I.
        len = 1;
    }
}

#[test]
fn separate_history_changes_a_real_choice_and_clearing_removes_it() {
    let config = just();
    let context = [at(config, LatticePos::new(2, 0, 0))];
    let mut scratch = PolicyScratch::default();
    let just_e = LatticePos::new(0, 1, 0);
    let pythagorean_e = LatticePos::new(4, 0, 0);
    let mut history = Some(just_e); // Stored by an earlier released E.
    assert_eq!(selected(choose(config, &context, None, 64, &mut scratch)).0, pythagorean_e);
    assert_eq!(score(pythagorean_e, &scratch.context[..1], history), 13);
    assert_eq!(score(just_e, &scratch.context[..1], history), 12);
    let decision = choose(config, &context, history, 64, &mut scratch);
    assert_eq!(selected(decision).0, just_e);
    decision.history_update.apply(&mut history);
    HistoryUpdate::Clear.apply(&mut history);
    assert_eq!(history, None);
    assert_eq!(selected(choose(config, &context, history, 64, &mut scratch)).0, pythagorean_e);
    // An empty phrase ignores even a history that prefers the other E.
    history = Some(pythagorean_e);
    assert_eq!(
        choose(config, &[], history, 64, &mut scratch),
        choose(config, &[], None, 64, &mut scratch)
    );
    assert_eq!(selected(choose(config, &[], history, 64, &mut scratch)).0, just_e);
    assert_ne!(just_e, LatticePos::ORIGIN);
}

#[test]
fn context_projects_actual_bent_zero_and_old_configuration_pitch_without_retuning() {
    let config = just();
    let mut scratch = PolicyScratch::default();
    let stale = LatticePos::new(4, 0, 0);
    let e400 = PitchClass::from_microcents(400_000_000);
    let mut context = [ContextPitch { pitch: e400, node: Some(stale) }];
    let before = context;
    let with_stale_metadata = choose(config, &context, None, 67, &mut scratch);
    assert_eq!(scratch.context[0], LatticePos::new(-4, -1, 0));
    assert_eq!(context, before);
    assert_eq!(context[0].pitch, e400); // Still the emitted 400-cent E.
    context[0].node = None; // Zero adaptive offset has the same authority.
    assert_eq!(choose(config, &context, None, 67, &mut scratch), with_stale_metadata);
    // Bend an attack at C to the exact emitted G; attack metadata must lose.
    let bent = ContextPitch {
        pitch: config.tuning().pitch_class(LatticePos::new(1, 0, 0)),
        node: Some(LatticePos::ORIGIN),
    };
    choose(config, &[bent], None, 64, &mut scratch);
    assert_eq!(scratch.context[0], LatticePos::new(1, 0, 0));
    // An equal-pitch node beyond the effective domain cannot be reused.
    let locked: MusicalConfig = ConfigReducer::default().resolved().into();
    let outsider = ContextPitch {
        pitch: PitchClass::from_microcents(0),
        node: Some(LatticePos::new(24, 0, 0)),
    };
    choose(locked, &[outsider], None, 64, &mut scratch);
    assert_eq!(scratch.context[0], LatticePos::ORIGIN);
    // Exact valid metadata is retained even when another coordinate shares pitch.
    let exact = at(locked, LatticePos::new(12, 0, 0));
    choose(locked, &[exact], None, 64, &mut scratch);
    assert_eq!(scratch.context[0], LatticePos::new(12, 0, 0));
}

#[test]
fn real_custom_axes_reach_empty_candidates_and_empty_context_projection() {
    let config = custom(720.0, 360.0, 960.0);
    let mut scratch = PolicyScratch::default();
    let mut history = Some(LatticePos::new(4, 0, 0));
    let decision = choose(config, &[], history, 63, &mut scratch); // 300c is 60c off the 120c grid.
    assert_eq!(scratch.domain_len, 65);
    assert_eq!(scratch.candidate_len, 0);
    assert!(scratch.domain[..scratch.domain_len].iter().all(|node| node
        .pitch
        .signed_microcents_from(PitchClass::from_microcents(300_000_000))
        .unsigned_abs()
        >= 60_000_000));
    assert_eq!(
        decision,
        Decision { assignment: Assignment::NoCandidate, history_update: HistoryUpdate::Clear }
    );
    assert_eq!(decision.assignment.correction_microcents(), 0);
    decision.history_update.apply(&mut history);
    assert_eq!(history, None);
    let voice = ContextPitch {
        pitch: PitchClass::from_microcents(300_000_000),
        node: Some(LatticePos::ORIGIN),
    };
    let result = choose(config, &[voice], Some(LatticePos::new(-6, -2, 0)), 60, &mut scratch);
    assert!(scratch.candidate_len > 0);
    assert_eq!(scratch.context_len, 0);
    assert_eq!(result, choose(config, &[], None, 60, &mut scratch));
    // Clearing from NoCandidate affects the next context-bearing choice too.
    let config = just();
    assert_eq!(
        selected(choose(
            config,
            &[at(config, LatticePos::new(2, 0, 0))],
            history,
            64,
            &mut scratch
        ))
        .0,
        LatticePos::new(4, 0, 0)
    );
}

#[test]
fn octave_wrap_and_inclusive_radii_use_single_microcent_precision() {
    let mut config = custom(0.0, 0.0, 0.0);
    let mut scratch = PolicyScratch::default();
    for offset in [-50_000_000, 50_000_000] {
        config.c_offset = offset;
        assert_eq!(selected(choose(config, &[], None, 60, &mut scratch)).1, offset);
        assert_eq!(scratch.candidate_len, MAX_CANDIDATES);
        let voice = ContextPitch { pitch: PitchClass::from_microcents(0), node: None };
        choose(config, &[voice], None, 60, &mut scratch);
        assert_eq!(scratch.context_len, 1);
        assert_eq!(scratch.context[0], LatticePos::ORIGIN);
        config.c_offset += offset.signum();
        assert_eq!(choose(config, &[], None, 60, &mut scratch).assignment, Assignment::NoCandidate);
    }
    config.c_offset = 0;
    for offset in [-50_000_001, 50_000_001] {
        let voice = ContextPitch { pitch: PitchClass::from_microcents(offset), node: None };
        choose(config, &[voice], None, 60, &mut scratch);
        assert_eq!(scratch.context_len, 0);
    }
    let a = PitchClass::from_microcents(1_199_999_999);
    let b = PitchClass::from_microcents(1);
    assert_eq!(b.signed_microcents_from(a), 2);
    assert_eq!(a.signed_microcents_from(b), -2);
    assert_eq!(
        PitchClass::from_microcents(600_000_000)
            .signed_microcents_from(PitchClass::from_microcents(0)),
        -600_000_000
    );
}

#[test]
fn respelling_deduplicates_coordinates_without_clipping_or_merging_pitch() {
    let mut scratch = PolicyScratch::default();
    for syntonic in [false, true] {
        for septimal_kleisma in [false, true] {
            let config: MusicalConfig = ConfigReducer::new(
                Tuning::default(),
                TuningModes {
                    tempered: Tempered { syntonic, septimal_kleisma },
                    auto: [false; 2],
                    learning: false,
                },
            )
            .resolved()
            .into();
            choose(config, &[], None, 60, &mut scratch);
            let expected_count = if syntonic { 29 } else { 65 };
            assert_eq!(scratch.domain_len, expected_count);
            let domain = &scratch.domain[..scratch.domain_len];
            for (i, node) in domain.iter().enumerate() {
                assert!(domain[..i].iter().all(|other| node.position != other.position));
            }
            if syntonic {
                assert!(domain.iter().any(|node| node.position == LatticePos::new(14, 0, 0)));
                assert!(domain.iter().any(|node| node.position == LatticePos::new(-14, 0, 0)));
                assert_eq!(
                    domain
                        .iter()
                        .filter(|node| node.pitch == PitchClass::from_microcents(0))
                        .count(),
                    3
                );
            }
        }
    }
}

#[test]
fn exact_score_and_projection_ties_follow_the_coordinate_order() {
    let config: MusicalConfig = ConfigReducer::default().resolved().into();
    let mut scratch = PolicyScratch::default();
    let (node, correction) = selected(choose(config, &[], None, 66, &mut scratch));
    assert!(candidates(&scratch).any(|node| node == LatticePos::new(6, 0, 0)));
    assert!(candidates(&scratch).any(|node| node == LatticePos::new(-6, 0, 0)));
    assert_eq!(node, LatticePos::new(-6, 0, 0));
    assert_eq!(correction, 0);
    let voice = ContextPitch { pitch: PitchClass::from_microcents(550_000_000), node: None };
    choose(config, &[voice], None, 64, &mut scratch);
    assert_eq!(scratch.context[0], LatticePos::new(-1, 0, 0)); // F beats either tritone at equal 50c.
    let config = just();
    let history = Some(LatticePos::new(i32::MIN, i32::MAX, i32::MIN));
    assert!(assign_new_note(
        config,
        &[at(config, LatticePos::ORIGIN)],
        history,
        OrderedOnset { key: 64 },
        &mut scratch
    )
    .is_ok());
}

#[test]
fn input_limits_are_explicit_and_never_truncate() {
    let config = just();
    let context = [at(config, LatticePos::ORIGIN); MAX_CONTEXT + 1];
    let mut scratch = PolicyScratch::default();
    choose(config, &context[..MAX_CONTEXT], None, 64, &mut scratch);
    assert_eq!(scratch.context_len, MAX_CONTEXT);
    assert_eq!(
        assign_new_note(config, &context, None, OrderedOnset { key: 64 }, &mut scratch),
        Err(InputError::TooMuchContext)
    );
    assert_eq!(
        assign_new_note(config, &[], None, OrderedOnset { key: 128 }, &mut scratch),
        Err(InputError::InvalidMidiKey)
    );
}

// Thread-local counters keep unrelated parallel test-harness allocations out
// of the measured region. Const TLS has no allocator-backed initialization.
struct CountingAllocator;
thread_local! {
    static COUNTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static HEAP_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn count_heap_call() {
    if COUNTING.try_with(|counting| counting.get()).unwrap_or(false) {
        let _ = HEAP_CALLS.try_with(|calls| calls.set(calls.get() + 1));
    }
}

unsafe impl std::alloc::GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_heap_call();
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        count_heap_call();
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        count_heap_call();
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, size: usize) -> *mut u8 {
        count_heap_call();
        unsafe { std::alloc::System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn without_heap<T>(run: impl FnOnce() -> T) -> T {
    struct StopCounting;
    impl Drop for StopCounting {
        fn drop(&mut self) {
            COUNTING.set(false);
        }
    }
    HEAP_CALLS.set(0);
    COUNTING.set(true);
    let guard = StopCounting;
    let result = run();
    drop(guard);
    assert_eq!(HEAP_CALLS.get(), 0, "production policy allocated, reallocated or freed");
    result
}

#[test]
fn full_pure_cohort_and_maximum_scratch_paths_use_no_heap() {
    let config = just();
    let mut scratch = PolicyScratch::default();
    let mut context = [at(config, LatticePos::ORIGIN); MAX_CONTEXT];
    // Four source rows each contribute 64 independent channel-zero/key cells.
    // This harness's direct indexing is not the production history owner.
    let mut history = [[None; 128]; 4];
    without_heap(|| {
        for onset in 0..MAX_COHORT_ONSETS {
            let key = (onset / 4) as u8;
            let source = onset % 4;
            let decision = choose(
                config,
                &context[..onset],
                history[source][usize::from(key)],
                key,
                &mut scratch,
            );
            let (node, correction) = selected(decision);
            decision.history_update.apply(&mut history[source][usize::from(key)]);
            context[onset] = ContextPitch {
                pitch: PitchClass::from_microcents(
                    i64::from(key) * 100_000_000 + i64::from(correction),
                ),
                node: Some(node),
            };
        }
        assert_eq!(scratch.context_len, MAX_CONTEXT - 1);
        // Real degenerate axes admit ALL 65 coordinates at once; none merge
        // just because they share pitch. Strip metadata to exercise projection.
        let crowded = custom(0.0, 0.0, 0.0);
        context.fill(ContextPitch { pitch: PitchClass::from_microcents(0), node: None });
        choose(crowded, &context, Some(LatticePos::new(1, 1, 0)), 60, &mut scratch);
        assert_eq!(scratch.domain_len, MAX_DOMAIN_NODES);
        assert_eq!(scratch.candidate_len, MAX_CANDIDATES);
        assert_eq!(scratch.context_len, MAX_CONTEXT);
        assert_eq!(
            choose(custom(720.0, 360.0, 960.0), &context, None, 63, &mut scratch).assignment,
            Assignment::NoCandidate
        );
        assert_eq!(
            assign_new_note(config, &context, None, OrderedOnset { key: 255 }, &mut scratch),
            Err(InputError::InvalidMidiKey)
        );
    });
}

#[test]
fn compiled_storage_and_candidate_counts() {
    use std::mem::{align_of, size_of};
    // Conservative portable ceilings, not assumed field/padding arithmetic.
    const {
        assert!(size_of::<PolicyScratch>() <= 4_224);
    }
    const {
        assert!(align_of::<PolicyScratch>() <= 8);
    }
    const {
        assert!(MAX_DOMAIN_NODES <= u8::MAX as usize);
    }
    println!("PolicyScratch={} align={} MusicalConfig={} ContextPitch={} OrderedOnset={} Assignment={} HistoryUpdate={} Decision={} DomainNode={} history_cell={}",
        size_of::<PolicyScratch>(), align_of::<PolicyScratch>(), size_of::<MusicalConfig>(), size_of::<ContextPitch>(),
        size_of::<OrderedOnset>(), size_of::<Assignment>(), size_of::<HistoryUpdate>(), size_of::<Decision>(), size_of::<DomainNode>(), size_of::<Option<LatticePos>>());
    println!("domain_storage={} candidate_storage={} context_scratch={} max_context_inputs={} cohort_context_inputs={} harness_history={}",
        size_of::<[DomainNode; MAX_DOMAIN_NODES]>(), size_of::<[u8; MAX_CANDIDATES]>(), size_of::<[LatticePos; MAX_CONTEXT]>(),
        size_of::<[ContextPitch; MAX_CONTEXT]>(), size_of::<[ContextPitch; MAX_COHORT_ONSETS]>(), size_of::<[[Option<LatticePos>;128];4]>());
    let mut scratch = PolicyScratch::default();
    for (name, config) in [
        ("Just", just()),
        ("locked12TET", ConfigReducer::default().resolved().into()),
        ("custom120grid", custom(720.0, 360.0, 960.0)),
        ("coincident", custom(0.0, 0.0, 0.0)),
    ] {
        let mut counts = [0; 12];
        for key in 0..12 {
            choose(config, &[], None, key, &mut scratch);
            counts[usize::from(key)] = scratch.candidate_len;
        }
        println!("{name} domain={} candidates={counts:?}", scratch.domain_len);
    }
}
