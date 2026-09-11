//! Pure take-lifecycle decisions. No publication, callback ownership, policy
//! atomics or pass serialization lives here; the recorder applies the actions.

/// Forward motion is stronger evidence than accepting a first playing block.
/// Only the former qualifies a one-file take's rewind as completion (#839).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Motion {
    Initial,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Deferred {
    None,
    Split,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Disarmed,
    /// No accepted block: there is no pass to split from (#828).
    Waiting,
    Recording {
        motion: Motion,
        deferred: Deferred,
    },
    /// A transport end, not a callback/configuration finalization proof.
    Complete,
}

impl State {
    pub(super) fn armed(self) -> bool {
        self != Self::Disarmed
    }
}

/// Independent host clocks, not lifecycle variants. Rejected jumps still
/// replace history; missing bars must erase the previous bar (#817).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct History {
    pub position: Option<(f64, f64)>,
    pub bar: Option<f64>,
}

/// A value snapshot of the GUI's policy; the shared atomics stay in Recorder.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Policy {
    pub end_at_rewind: bool,
    pub stop_bar: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Observation {
    Armed(bool),
    Bar(Option<f64>),
    Transport { position: f64, playing: bool, duration: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum End {
    Rewind,
    Bar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    /// Already complete/disarmed, or an unchanged arm observation.
    None,
    Arm,
    HistoryOnly,
    Record,
    SplitAndRecord,
    Complete(End),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Transition {
    pub state: State,
    pub history: History,
    pub action: Action,
}

/// One observation, entirely by value. Bar is observed before transport for
/// each block: its completion must exclude that block and any deferred split.
/// The two observations remain separate because callers can lack either clock.
pub(super) fn transition(
    state: State,
    history: History,
    observation: Observation,
    policy: Policy,
) -> Transition {
    let mut next = Transition { state, history, action: Action::None };
    if let Observation::Armed(armed) = observation {
        if !armed {
            next.state = State::Disarmed;
        } else if !state.armed() {
            next.state = State::Waiting;
            next.history = History::default();
            next.action = Action::Arm;
        }
        return next;
    }
    if matches!(state, State::Disarmed | State::Complete) {
        return next;
    }
    next.action = Action::HistoryOnly;
    match observation {
        Observation::Armed(_) => unreachable!(),
        Observation::Bar(bar) => {
            next.history.bar = bar;
            // Crossing, not level; retain the existing one-bar scrub allowance.
            // A small drag straddling the target remains indistinguishable from
            // playback and visibly ends the take (#817).
            if let (Some(last), Some(bar), Some(stop)) = (history.bar, bar, policy.stop_bar) {
                if last < stop && stop <= bar && bar - last <= 1.0 {
                    next.state = State::Complete;
                    next.action = Action::Complete(End::Bar);
                }
            }
        }
        Observation::Transport { position, playing, duration } => {
            next.history.position = Some((position, duration));
            let (mut motion, mut deferred) = match state {
                State::Recording { motion, deferred } => (motion, deferred),
                _ => (Motion::Initial, Deferred::None),
            };
            let recording = matches!(state, State::Recording { .. });
            let rolling = match history.position {
                Some((last, _)) if position < last - if playing { 0.05 } else { 0.0 } => {
                    if policy.end_at_rewind {
                        if motion == Motion::Forward {
                            next.state = State::Complete;
                            next.action = Action::Complete(End::Rewind);
                            return next;
                        }
                        deferred = Deferred::None;
                    } else if recording {
                        // Waiting has no debt: the first routed block must
                        // stay in pass 1 so its configuration can close (#828).
                        deferred = Deferred::Split;
                    }
                    playing
                }
                Some((last, previous_duration)) => {
                    // Use the PREVIOUS callback size, inclusive +/-50% (#839).
                    // A tiny scrub in this interval records one block and can
                    // qualify a later rewind, exactly like stopped export.
                    let continuous = previous_duration.is_finite()
                        && previous_duration > 0.0
                        && position >= last + previous_duration * 0.5
                        && position <= last + previous_duration * 1.5;
                    let rolling = playing || continuous;
                    if rolling && position > last {
                        motion = Motion::Forward;
                    }
                    rolling
                }
                None => playing,
            };
            if rolling {
                // A policy change to a one-file trigger cancels an old debt,
                // even if resumption is forward rather than another rewind.
                next.action = if deferred == Deferred::Split && !policy.end_at_rewind {
                    Action::SplitAndRecord
                } else {
                    Action::Record
                };
                next.state = State::Recording { motion, deferred: Deferred::None };
            } else if recording {
                next.state = State::Recording { motion, deferred };
            }
        }
    }
    next
}

#[cfg(test)]
mod tests;
