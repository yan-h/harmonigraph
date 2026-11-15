//! Pure take-lifecycle decisions. No publication, callback ownership or policy
//! atomics live here; the recorder applies the actions.

/// Forward motion is stronger evidence than accepting a first playing block.
/// Only the former qualifies a one-file take's rewind as completion (#839).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Motion {
    Initial,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Disarmed,
    /// No accepted block yet.
    Waiting,
    Recording {
        motion: Motion,
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
    /// Contiguous end of the blocks accepted into this take. Parked host
    /// observations do not move the audio that has already been appended.
    pub recorded_end: Option<f64>,
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
#[repr(u8)]
pub(super) enum End {
    Rewind = 1,
    Bar = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    /// Already complete/disarmed, or an unchanged arm observation.
    None,
    Arm,
    HistoryOnly,
    Record,
    Complete(End),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Transition {
    pub state: State,
    pub history: History,
    pub action: Action,
    /// Where a block that records sits in the take's audio: the transport's
    /// position at the take's first block, and the end of the audio already
    /// recorded after that, so a resume, a loop wrap or a seek continues the
    /// WAV rather than the host's position.
    pub origin: Option<f64>,
}

/// One observation, entirely by value. Bar is observed before transport for
/// each block: its completion must exclude that block.
/// The two observations remain separate because callers can lack either clock.
pub(super) fn transition(
    state: State,
    history: History,
    observation: Observation,
    policy: Policy,
) -> Transition {
    let mut next = Transition { state, history, action: Action::None, origin: None };
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
            let mut motion = match state {
                State::Recording { motion } => motion,
                _ => Motion::Initial,
            };
            let rolling = match crate::transport::step(history.position, position, playing) {
                crate::transport::Step::Back { rolling } => {
                    if policy.end_at_rewind && motion == Motion::Forward {
                        next.state = State::Complete;
                        next.action = Action::Complete(End::Rewind);
                        return next;
                    }
                    rolling
                }
                crate::transport::Step::Ahead { rolling } => {
                    // A tiny scrub inside the continuity interval records one
                    // block and can qualify a later rewind, exactly like a
                    // stopped export (#839).
                    if rolling && history.position.is_some_and(|(last, _)| position > last) {
                        motion = Motion::Forward;
                    }
                    rolling
                }
            };
            if rolling {
                // Host observation history includes parked scrubs. Audio does
                // not: compare to the accepted prefix, including after a pause.
                // Match playback's 50 ms jitter allowance, without accumulating
                // an arbitrary missing interval in an otherwise successful WAV.
                next.action = Action::Record;
                next.state = State::Recording { motion };
                let audio_origin = history.recorded_end.unwrap_or(position);
                next.history.recorded_end =
                    (duration.is_finite() && duration >= 0.0).then_some(audio_origin + duration);
                next.origin = Some(audio_origin);
            } else if matches!(state, State::Recording { .. }) {
                next.state = State::Recording { motion };
            }
        }
    }
    next
}

#[cfg(test)]
mod tests;
