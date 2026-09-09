//! Source-local selectors inside the wrapper's opaque staging token.
use super::{api, NONE};

pub(super) const RELEASE: u64 = 1;
const RESET: u64 = 2;
const FIRST_CHILD: u64 = 3;

pub(super) fn ordinary(attempt: u64, position: usize, serial: u64, child: u16) -> api::Token {
    api::Token([
        attempt,
        position as u64,
        serial,
        if child == NONE { 0 } else { u64::from(child) + FIRST_CHILD },
    ])
}

/// The life serial makes traces identify the release owner; it is not an
/// independent slot generation. The occupied debt slot itself remains the
/// owner through synchronous completion and accepted acknowledgement.
pub(super) fn release(ordinary_attempt: u64, index: usize, life_serial: u64) -> api::Token {
    api::Token([ordinary_attempt, index as u64, life_serial, RELEASE])
}

pub(super) fn reset(channel: usize, bit: usize) -> api::Token {
    api::Token([0, channel as u64, bit as u64, RESET])
}

pub(super) fn is_emergency(token: api::Token) -> bool {
    matches!(token.0[3], RELEASE | RESET)
}

pub(super) fn child(token: api::Token) -> u16 {
    if token.0[3] == 0 {
        NONE
    } else {
        (token.0[3] - FIRST_CHILD) as u16
    }
}

#[cfg(all(test, debug_assertions))]
pub(super) fn is_work(token: api::Token) -> bool {
    (FIRST_CHILD..FIRST_CHILD + super::work::CAPACITY as u64).contains(&token.0[3])
}
