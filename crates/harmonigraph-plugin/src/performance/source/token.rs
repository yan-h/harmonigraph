//! Source-local selectors inside the wrapper's opaque staging token.
use super::{api, NONE};

pub(super) const RELEASE: u64 = 1;
const RESET: u64 = 2;
// Only this paired inline onset skips the completion charge paid by its choke.
// Its unique attempt word is unchanged when the selector is marked prepaid.
pub(super) const PREPAID_ONSET: u64 = u64::MAX;
const FIRST_CHILD: u64 = 3;

pub(super) fn ordinary(attempt: u64, position: usize, serial: u64, child: u16) -> api::Token {
    api::Token([
        attempt,
        position as u64,
        serial,
        if child == NONE { 0 } else { u64::from(child) + FIRST_CHILD },
    ])
}

pub(super) fn release(attempt: u64, index: usize, serial: u64) -> api::Token {
    api::Token([attempt, index as u64, serial, RELEASE])
}

pub(super) fn reset(channel: usize, bit: usize) -> api::Token {
    api::Token([0, channel as u64, bit as u64, RESET])
}

pub(super) fn is_emergency(token: api::Token) -> bool {
    matches!(token.0[3], RELEASE | RESET)
}

pub(super) fn child(token: api::Token) -> u16 {
    if matches!(token.0[3], 0 | PREPAID_ONSET) {
        NONE
    } else {
        (token.0[3] - FIRST_CHILD) as u16
    }
}

#[cfg(all(test, debug_assertions))]
pub(super) fn is_work(token: api::Token) -> bool {
    (FIRST_CHILD..FIRST_CHILD + super::work::CAPACITY as u64).contains(&token.0[3])
}
