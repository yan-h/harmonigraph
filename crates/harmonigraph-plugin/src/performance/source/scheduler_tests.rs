//! Test-only probes at real preparation and destruction boundaries.
//! They run synchronously inside the production owner and never replace its
//! scheduler, host completion or release-debt state machine.
use super::*;
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Boundary {
    Readiness,
    Admission,
    Gate,
}

struct PrepareProbe {
    boundary: Boundary,
    reached: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct JoinProbe {
    before: Option<(usize, usize, usize)>,
    empty_after: bool,
}

thread_local! {
    static PREPARE_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static PREPARE: RefCell<Option<PrepareProbe>> = const { RefCell::new(None) };
    static CLOSE_GATE: Cell<bool> = const { Cell::new(false) };
    static JOIN_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static JOIN: RefCell<JoinProbe> = const { RefCell::new(JoinProbe {
        before: None,
        empty_after: false,
    }) };
}

pub(crate) fn start_prepare(boundary: Boundary) {
    PREPARE.with(|probe| {
        assert!(probe.borrow().is_none());
        *probe.borrow_mut() = Some(PrepareProbe { boundary, reached: false });
    });
    PREPARE_ACTIVE.with(|active| active.set(true));
}

pub(crate) fn finish_prepare() {
    PREPARE_ACTIVE.with(|active| active.set(false));
    let probe = PREPARE.with(|probe| probe.borrow_mut().take().unwrap());
    assert!(probe.reached, "the production replacement must reach prepare");
    assert!(!CLOSE_GATE.with(Cell::get), "the gate probe must reach the CAS itself");
}

pub(super) fn close_gate_for_probe(gate: &std::sync::atomic::AtomicU64) {
    if CLOSE_GATE.with(|probe| probe.replace(false)) {
        assert_eq!(gate.swap(BUSY, Ordering::AcqRel), OPEN);
    }
}

pub(crate) fn start_join() {
    assert!(!JOIN_ACTIVE.with(|active| active.replace(true)));
    JOIN.with(|probe| *probe.borrow_mut() = JoinProbe::default());
}

pub(crate) fn finish_join(expected: (usize, usize, usize)) {
    JOIN_ACTIVE.with(|active| active.set(false));
    let probe = JOIN.with(|probe| std::mem::take(&mut *probe.borrow_mut()));
    assert_eq!(probe.before, Some(expected));
    assert!(probe.empty_after, "join must vacate every release owner");
}

pub(super) fn record_join_before(debt: &debt::Debt) {
    if !JOIN_ACTIVE.with(Cell::get) {
        return;
    }
    let mut counts = (0, 0, 0);
    for release in debt.releases() {
        if release.staged {
            counts.1 += 1;
        } else if release.accepted.is_some() {
            counts.2 += 1;
        } else {
            counts.0 += 1;
        }
    }
    JOIN.with(|probe| probe.borrow_mut().before = Some(counts));
}

pub(super) fn record_join_after(debt: &debt::Debt) {
    if JOIN_ACTIVE.with(Cell::get) {
        JOIN.with(|probe| probe.borrow_mut().empty_after = !debt.any_release());
    }
}

impl Source {
    pub(super) fn test_prepare_probe(&mut self, group: api::Group) -> Option<bool> {
        if !PREPARE_ACTIVE.with(Cell::get)
            || token::is_emergency(group.token)
            || group.sequence_parts().is_none()
        {
            return None;
        }
        // Hold this measured refusal through later scheduler passes in the same
        // callback. The next callback restores and uses every real resource.
        if PREPARE.with(|probe| probe.borrow().as_ref().is_some_and(|probe| probe.reached)) {
            return Some(false);
        }
        let mut probe = PREPARE.with(|probe| probe.borrow_mut().take())?;
        let (first, second) = group.sequence_parts().unwrap();
        let position = first.token.0[1] as usize;
        let child = token::child(first.token);
        let parent = self.pending.at(position).unwrap();
        let predecessor = self.resolved(position, child).life;
        let onset = self.resolved(position, NONE);
        assert_eq!((second.token.0[1] as usize, second.token.0[2]), (position, parent.serial));
        assert!(self.lives.at(predecessor).unwrap().sounded);
        assert!(self.lives.at(predecessor).unwrap().terminal.is_none());
        assert!(self.assignment_ready(onset.life));
        assert!(self.admitted(parent.generation));
        assert!(self.ordinary_stream_ready());
        let coverage = self.coverage;
        let generation = self.offer.as_ref().unwrap().generation;
        match probe.boundary {
            Boundary::Readiness => self.coverage = None,
            Boundary::Admission => self.offer.as_mut().unwrap().generation = parent.generation - 1,
            Boundary::Gate => CLOSE_GATE.with(|probe| probe.set(true)),
        }
        assert!(!self.prepare(group), "the selected boundary must refuse preparation");
        assert!(self.permit.is_none());
        assert!(self.lives.at(predecessor).unwrap().sounded);
        assert!(self.lives.at(predecessor).unwrap().terminal.is_none());
        assert!(self.lives.at(predecessor).unwrap().reserved);
        assert!(!self.lives.at(onset.life).unwrap().reserved);
        match probe.boundary {
            Boundary::Readiness => self.coverage = coverage,
            Boundary::Admission => self.offer.as_mut().unwrap().generation = generation,
            Boundary::Gate => {
                assert!(!CLOSE_GATE.with(Cell::get));
                let offer = self.offer.as_ref().unwrap();
                let row = &offer.session.rows[usize::from(offer.lease.slot - 1)];
                assert_eq!(row.emission_gate.swap(OPEN, Ordering::AcqRel), BUSY);
            }
        }
        probe.reached = true;
        PREPARE.with(|state| *state.borrow_mut() = Some(probe));
        Some(false)
    }

    pub(crate) fn test_permit_held(&self) -> bool {
        self.permit.is_some()
    }
}
