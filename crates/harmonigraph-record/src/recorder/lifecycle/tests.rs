use super::*;
use harmonigraph_take::{RenderConfig, RenderTrigger};

const BLOCK: f64 = 64.0 / 48_000.0;
// 120 BPM, 4/4: two seconds per bar.
const BAR_BLOCK: f64 = BLOCK / 2.0;

fn observe(next: Transition, observation: Observation, policy: Policy) -> Transition {
    let run = || transition(next.state, next.history, observation, policy);
    // The test-support feature installs the crate's guarded allocator. Guard
    // every table transition, including arm, completion and deferred actions.
    #[cfg(feature = "test-support")]
    return nice_assert_no_alloc::assert_no_alloc(run);
    #[cfg(not(feature = "test-support"))]
    run()
}

fn fresh() -> Transition {
    let next =
        Transition { state: State::Disarmed, history: History::default(), action: Action::None };
    let next = observe(next, Observation::Armed(true), Policy::default());
    assert_eq!(next.state, State::Waiting);
    assert_eq!(next.action, Action::Arm);
    next
}

fn transport(next: Transition, position: f64, playing: bool, policy: Policy) -> Transition {
    observe(next, Observation::Transport { position, playing, duration: BLOCK }, policy)
}

fn recording(motion: Motion, deferred: Deferred) -> State {
    State::Recording { motion, deferred }
}

fn policy(trigger: RenderTrigger) -> Policy {
    let config = RenderConfig { trigger, stop_bar: 6.0, ..Default::default() };
    Policy { end_at_rewind: trigger.ends_at_rewind(), stop_bar: config.stop_at_bar() }
}

#[test]
fn trigger_table_reaches_playback_and_stopped_export_rewind_actions() {
    for (trigger, end) in [
        (RenderTrigger::OnDisarm, false),
        (RenderTrigger::AtBar, false),
        (RenderTrigger::OnTransportStop, true),
        (RenderTrigger::AtLoopEnd, true),
    ] {
        for playing in [false, true] {
            let policy = policy(trigger);
            let mut next = transport(fresh(), 5.0, playing, policy);
            assert_eq!(next.action, if playing { Action::Record } else { Action::HistoryOnly });
            for block in 1..=4 {
                next = transport(next, 5.0 + f64::from(block) * BLOCK, playing, policy);
                assert_eq!(next.action, Action::Record);
                assert_eq!(next.state, recording(Motion::Forward, Deferred::None));
            }
            // Stopped restores can be smaller than playback's 50 ms jitter.
            // A playing loop goes to 4 s to deliberately exceed that allowance.
            let restore = if playing { 4.0 } else { 5.0 };
            next = transport(next, restore, playing, policy);
            assert_eq!(
                next.action,
                if end {
                    Action::Complete(End::Rewind)
                } else if playing {
                    Action::SplitAndRecord
                } else {
                    Action::HistoryOnly
                },
                "{trigger:?}, playing={playing}"
            );
            if end {
                assert_eq!(next.state, State::Complete);
                let history = next.history;
                next = transport(next, restore + BLOCK, true, policy);
                assert_eq!(next.action, Action::None);
                assert_eq!(next.history, history);
            } else {
                assert_eq!(
                    next.state,
                    recording(
                        Motion::Forward,
                        if playing { Deferred::None } else { Deferred::Split }
                    )
                );
                next = transport(next, restore + BLOCK, playing, policy);
                assert_eq!(
                    next.action,
                    if playing { Action::Record } else { Action::SplitAndRecord }
                );
                next = transport(next, restore + 2.0 * BLOCK, playing, policy);
                assert_eq!(next.action, Action::Record, "a debt is paid exactly once");
            }
        }
    }
}

#[test]
fn continuity_uses_previous_timing_with_inclusive_tolerance() {
    for duration in [64.0 / 48_000.0, 512.0 / 44_100.0] {
        for (factor, accepted) in
            [(0.499, false), (0.5, true), (1.0, true), (1.5, true), (1.501, false)]
        {
            let next = observe(
                fresh(),
                Observation::Transport { position: 0.0, playing: false, duration },
                Policy::default(),
            );
            // Current duration differs enough to reject both edges if used.
            let next = observe(
                next,
                Observation::Transport {
                    position: duration * factor,
                    playing: false,
                    duration: duration * 4.0,
                },
                Policy::default(),
            );
            assert_eq!(next.action, if accepted { Action::Record } else { Action::HistoryOnly });
            assert_eq!(
                next.state,
                if accepted { recording(Motion::Forward, Deferred::None) } else { State::Waiting }
            );
            assert_eq!(next.history.position, Some((duration * factor, duration * 4.0)));
        }
    }
    for duration in [0.0, -BLOCK, f64::NAN, f64::INFINITY] {
        for playing in [false, true] {
            let next = observe(
                fresh(),
                Observation::Transport { position: 0.0, playing: false, duration },
                Policy::default(),
            );
            let next = transport(next, BLOCK, playing, Policy::default());
            assert_eq!(next.action, if playing { Action::Record } else { Action::HistoryOnly });
        }
    }
}

#[test]
fn pre_roll_scrubs_and_first_playing_snapback_do_not_complete_or_split() {
    for trigger in [
        RenderTrigger::OnDisarm,
        RenderTrigger::AtBar,
        RenderTrigger::OnTransportStop,
        RenderTrigger::AtLoopEnd,
    ] {
        let policy = policy(trigger);
        let mut next = fresh();
        for position in [10.0, 30.0, 5.0, 5.0] {
            next = transport(next, position, false, policy);
            assert_eq!(next.action, Action::HistoryOnly);
            assert_eq!(next.state, State::Waiting, "scrubs cannot acquire split debt");
        }
        next = transport(next, 0.0, true, policy);
        assert_eq!(next.action, Action::Record);
        assert_eq!(next.state, recording(Motion::Initial, Deferred::None));
        // Another snapback before positive progress distinguishes accepted
        // audio from forward-motion evidence. One-file takes retain its origin.
        next = transport(next, -1.0, true, policy);
        assert_eq!(
            next.action,
            if policy.end_at_rewind { Action::Record } else { Action::SplitAndRecord }
        );
        next = transport(next, -1.0 + BLOCK, true, policy);
        assert_eq!(next.state, recording(Motion::Forward, Deferred::None));
    }
}

#[test]
fn playing_jitter_and_stopped_restore_have_distinct_boundaries() {
    for (back, playing, rewound) in [
        (0.049999, true, false),
        (0.05, true, false),
        (0.050001, true, true),
        (0.0, false, false),
        (BLOCK / 100.0, false, true),
    ] {
        let policy = policy(RenderTrigger::OnTransportStop);
        let next = transport(fresh(), -BLOCK, true, policy);
        let next = transport(next, 0.0, true, policy);
        assert_eq!(next.state, recording(Motion::Forward, Deferred::None));
        let next = transport(next, -back, playing, policy);
        assert_eq!(
            next.action,
            if rewound {
                Action::Complete(End::Rewind)
            } else if playing {
                Action::Record
            } else {
                Action::HistoryOnly
            }
        );
    }
}

#[test]
fn policy_changes_drop_real_split_debt_in_both_resumption_directions() {
    for forward in [false, true] {
        for trigger in [RenderTrigger::OnTransportStop, RenderTrigger::AtLoopEnd] {
            let mut next = transport(fresh(), 10.0, true, Policy::default());
            next = transport(next, 5.0, false, Policy::default());
            assert_eq!(
                next.state,
                recording(Motion::Initial, Deferred::Split),
                "must reach the debt rule"
            );
            // Repeated parked blocks preserve the debt until a recording block.
            next = transport(next, 5.0, false, policy(trigger));
            assert_eq!(next.state, recording(Motion::Initial, Deferred::Split));
            next = transport(next, if forward { 5.0 + BLOCK } else { 0.0 }, true, policy(trigger));
            assert_eq!(next.action, Action::Record);
            assert_eq!(
                next.state,
                recording(if forward { Motion::Forward } else { Motion::Initial }, Deferred::None)
            );
        }
    }
}

#[test]
fn bar_table_preserves_crossing_history_and_preempts_a_deferred_split() {
    for (last, bar, target, completes) in [
        (Some(5.0 - BAR_BLOCK), Some(5.0), Some(5.0), true),
        (Some(5.0), Some(5.0 + BAR_BLOCK), Some(5.0), false),
        (Some(5.0 - BAR_BLOCK), Some(5.0 - BAR_BLOCK / 2.0), Some(5.0), false),
        (Some(4.0), Some(5.0), Some(5.0), true), // inclusive residual scrub allowance
        (Some(4.0), Some(5.000001), Some(5.0), false),
        (Some(6.0), Some(5.0), Some(5.0), false),
        (None, Some(5.0), Some(5.0), false),
        (Some(5.0 - BAR_BLOCK), None, Some(5.0), false),
        (Some(5.0 - BAR_BLOCK), Some(5.0), None, false),
        (Some(-BAR_BLOCK), Some(0.0), Some(0.0), true),
    ] {
        let next = transport(fresh(), 20.0, true, Policy::default());
        let next = transport(next, 10.0 - BLOCK, false, Policy::default());
        assert_eq!(next.state, recording(Motion::Initial, Deferred::Split));
        // Seed with the trigger OFF: enabling it must use recent bar history.
        let next = observe(next, Observation::Bar(last), Policy::default());
        let next =
            observe(next, Observation::Bar(bar), Policy { stop_bar: target, ..Default::default() });
        assert_eq!(next.history.bar, bar);
        assert_eq!(
            next.action,
            if completes { Action::Complete(End::Bar) } else { Action::HistoryOnly }
        );
        let next = transport(next, 10.0, true, Policy::default());
        assert_eq!(next.action, if completes { Action::None } else { Action::SplitAndRecord });
    }
    let p = policy(RenderTrigger::AtBar);
    let next = observe(fresh(), Observation::Bar(Some(5.0 - BAR_BLOCK)), p);
    let next = observe(next, Observation::Bar(None), p);
    let next = observe(next, Observation::Bar(Some(5.0)), p);
    assert_eq!(next.action, Action::HistoryOnly, "missing clock erases the crossing, not skips it");
}

#[test]
fn arm_edges_reset_history_but_repeated_arm_cannot_revive_completion() {
    let p = policy(RenderTrigger::AtBar);
    let mut next = observe(fresh(), Observation::Bar(Some(5.0 - BAR_BLOCK)), p);
    next = observe(next, Observation::Bar(Some(5.0)), p);
    assert_eq!(next.state, State::Complete);
    let complete = next;
    for observation in [Observation::Armed(true), Observation::Bar(Some(0.0))] {
        next = observe(next, observation, p);
        assert_eq!(next.state, State::Complete);
        assert_eq!(next.action, Action::None);
        assert_eq!(next.history, complete.history);
    }
    next = observe(next, Observation::Armed(false), p);
    assert_eq!(next.state, State::Disarmed);
    next = transport(next, 100.0, true, p);
    assert_eq!(next.action, Action::None);
    next = observe(next, Observation::Armed(true), p);
    assert_eq!(next.state, State::Waiting);
    assert_eq!(next.action, Action::Arm);
    assert_eq!(next.history, History::default());
    next = transport(next, 5.0, false, p);
    next = observe(next, Observation::Armed(true), p);
    assert_eq!(next.history.position, Some((5.0, BLOCK)));
    next = transport(next, 5.0 + BLOCK, false, p);
    assert_eq!(next.action, Action::Record, "repeated arm does not reset continuity");
}
