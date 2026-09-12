//! Observations at the real controls' entry, before interaction can repair input.
//! Compiled out entirely outside unit tests. Scopes nest and unwind safely;
//! other tests drawing on another thread cannot contribute observations.

use std::{cell::RefCell, ops::RangeInclusive};

#[derive(Debug)]
pub(crate) struct Visit {
    pub label: String,
    pub values: Vec<f32>,
    pub range: RangeInclusive<f32>,
}

thread_local! {
    static SCOPES: RefCell<Vec<Vec<Visit>>> = const { RefCell::new(Vec::new()) };
}

pub(super) fn record(label: &str, values: &[f32], range: &RangeInclusive<f32>) {
    SCOPES.with_borrow_mut(|scopes| {
        if let Some(visits) = scopes.last_mut() {
            visits.push(Visit {
                label: label.to_owned(),
                values: values.to_vec(),
                range: range.clone(),
            });
        }
    });
}

pub(crate) fn collect(draw: impl FnOnce()) -> Vec<Visit> {
    struct Scope;
    impl Drop for Scope {
        fn drop(&mut self) {
            SCOPES.with_borrow_mut(|scopes| {
                scopes.pop().unwrap();
            });
        }
    }
    SCOPES.with_borrow_mut(|scopes| scopes.push(Vec::new()));
    let scope = Scope;
    draw();
    let visits = SCOPES.with_borrow_mut(|scopes| std::mem::take(scopes.last_mut().unwrap()));
    drop(scope);
    visits
}

#[test]
fn scopes_isolate_threads_nesting_and_unwinding() {
    let visits = collect(|| {
        record("outer", &[1.0], &(0.0..=2.0));
        let other = std::thread::spawn(|| collect(|| record("thread", &[2.0], &(0.0..=2.0))));
        assert!(std::panic::catch_unwind(|| collect(|| {
            record("unwound", &[0.0], &(0.0..=2.0));
            panic!("exercise scope cleanup");
        }))
        .is_err());
        assert_eq!(other.join().unwrap()[0].label, "thread");
        assert_eq!(collect(|| record("nested", &[0.0], &(0.0..=2.0)))[0].label, "nested");
    });
    assert_eq!(visits.len(), 1);
    assert_eq!(visits[0].label, "outer");
    assert!(collect(|| {}).is_empty());
}
