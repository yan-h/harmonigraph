//! Observations at the real controls: the values a bar is handed at its
//! entry, before interaction can repair input, and where each bar is laid.
//! Compiled out entirely outside unit tests. Scopes nest and unwind safely;
//! other tests drawing on another thread cannot contribute observations.

use std::{cell::RefCell, ops::RangeInclusive, thread::LocalKey};

#[derive(Debug)]
pub(crate) struct Visit {
    pub label: String,
    pub values: Vec<f32>,
    pub range: RangeInclusive<f32>,
    /// `Some` for a [`ValueBar`](super::value::ValueBar) or a
    /// [`RangeBar`](super::range::RangeBar), holding its double-click target,
    /// one number per value, if it has one; `None` for every other control.
    pub default: Option<Option<Vec<f32>>>,
}

/// Where a bar was laid: the rect it allocated, and the horizontal room its
/// `Ui` offered it just before, which is the column it was meant to fill.
/// Read before the allocation because egui grows a `Ui`'s `max_rect` to hold
/// whatever it allocates, so a bar wider than its column would otherwise have
/// already widened the column it is checked against.
///
/// Recorded by [`bar::allocate_row`](super::bar::allocate_row), which every
/// row-high control that sizes itself from its column allocates through, so a
/// new bar is counted by construction and the settings sweeps read a bar's
/// geometry from the bar rather than picking its well out of the paint. The
/// plots are several rows tall and not bars, and record nothing.
#[derive(Clone, Debug)]
pub(crate) struct Placed {
    /// The bar's label, or its kind where it carries none of its own.
    pub name: String,
    pub rect: egui::Rect,
    pub offered: egui::Rangef,
}

type Scopes<T> = RefCell<Vec<Vec<T>>>;

thread_local! {
    static SCOPES: Scopes<Visit> = const { RefCell::new(Vec::new()) };
    static PLACED: Scopes<Placed> = const { RefCell::new(Vec::new()) };
}

/// Record that the bar `name` took `rect` of the room `offered` across.
pub(super) fn record_placed(name: &str, rect: egui::Rect, offered: egui::Rangef) {
    push_into(&PLACED, || Placed { name: name.to_owned(), rect, offered });
}

pub(super) fn record(label: &str, values: &[f32], range: &RangeInclusive<f32>) {
    push(label, values, range, None);
}

pub(super) fn record_value_bar(
    label: &str,
    value: f32,
    range: &RangeInclusive<f32>,
    default: Option<f32>,
) {
    push(label, &[value], range, Some(default.map(|d| vec![d])));
}

pub(super) fn record_range_bar(
    label: &str,
    (low, high): (f32, f32),
    range: &RangeInclusive<f32>,
    default: Option<(f32, f32)>,
) {
    push(label, &[low, high], range, Some(default.map(|(l, h)| vec![l, h])));
}

fn push(
    label: &str,
    values: &[f32],
    range: &RangeInclusive<f32>,
    default: Option<Option<Vec<f32>>>,
) {
    push_into(&SCOPES, || Visit {
        label: label.to_owned(),
        values: values.to_vec(),
        range: range.clone(),
        default,
    });
}

/// Push what `make` builds into the innermost open scope, building nothing
/// when no scope is open: every frame of every test draws through here,
/// the allocation profiles included, and only a collecting one pays.
fn push_into<T: 'static>(key: &'static LocalKey<Scopes<T>>, make: impl FnOnce() -> T) {
    key.with_borrow_mut(|scopes| {
        if let Some(items) = scopes.last_mut() {
            items.push(make());
        }
    });
}

/// The values every bar `draw` drew was handed.
pub(crate) fn collect(draw: impl FnOnce()) -> Vec<Visit> {
    collect_in(&SCOPES, draw)
}

/// Where every bar `draw` drew was laid.
pub(crate) fn placed(draw: impl FnOnce()) -> Vec<Placed> {
    collect_in(&PLACED, draw)
}

fn collect_in<T: 'static>(key: &'static LocalKey<Scopes<T>>, draw: impl FnOnce()) -> Vec<T> {
    struct Scope<T: 'static>(&'static LocalKey<Scopes<T>>);
    impl<T: 'static> Drop for Scope<T> {
        fn drop(&mut self) {
            self.0.with_borrow_mut(|scopes| {
                scopes.pop().unwrap();
            });
        }
    }
    key.with_borrow_mut(|scopes| scopes.push(Vec::new()));
    let scope = Scope(key);
    draw();
    let items = key.with_borrow_mut(|scopes| std::mem::take(scopes.last_mut().unwrap()));
    drop(scope);
    items
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
