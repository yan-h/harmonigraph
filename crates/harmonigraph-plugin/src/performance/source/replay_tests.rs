//! Defensive duplicate injection after real LIFO reuse. CLAP completions are
//! synchronous; this is not a claim that the host asynchronously returns tokens.
use super::*;
use std::cell::RefCell;

#[derive(Clone, Copy)]
struct Replay {
    owner: usize,
    saved: Option<api::Completion>,
    lifetime: u64,
    prepared: bool,
    completed: bool,
}
thread_local! {
    static REPLAY: RefCell<Option<Replay>> = const { RefCell::new(None) };
}
impl Source {
    pub fn test_start_replay(&self) {
        REPLAY.with(|state| {
            assert!(state.borrow().is_none());
            *state.borrow_mut() = Some(Replay {
                owner: Arc::as_ptr(&self.shared) as usize,
                saved: None,
                lifetime: 0,
                prepared: false,
                completed: false,
            });
        });
    }
    pub fn test_finish_replay(&self) {
        let state = REPLAY.with(|state| state.borrow_mut().take().unwrap());
        assert_eq!(state.owner, Arc::as_ptr(&self.shared) as usize);
        assert!(
            state.prepared && state.completed,
            "both defensive injections reached the reused live child"
        );
    }
    pub fn test_stale_prepare(&mut self, group: api::Group) {
        if !token::is_work(group.token) {
            return;
        }
        let replay = REPLAY.with(|state| *state.borrow());
        let Some(replay) = replay.filter(|state| {
            state.owner == Arc::as_ptr(&self.shared) as usize
                && state.saved.is_some()
                && !state.prepared
        }) else {
            return;
        };
        let saved = replay.saved.unwrap();
        self.check_reused_child(group, saved, replay.lifetime);
        assert!(self.permit.is_none());
        let before = self.test_snapshot();
        assert!(!self.prepare(saved.group));
        assert_eq!(self.test_snapshot(), before);
        assert!(self.permit.is_none());
        REPLAY.with(|state| state.borrow_mut().as_mut().unwrap().prepared = true);
    }
    pub fn test_stale_complete(
        &mut self,
        completion: api::Completion,
        output: &mut api::Output<'_>,
    ) {
        if !token::is_work(completion.group.token) || completion.accepted != 1 {
            return;
        }
        let replay = REPLAY.with(|state| {
            let mut state = state.borrow_mut();
            let state = state.as_mut().filter(|state| {
                state.owner == Arc::as_ptr(&self.shared) as usize && !state.completed
            })?;
            if state.saved.is_none() {
                state.saved = Some(completion);
                state.lifetime = self.work.at(token::child(completion.group.token)).serial;
                return None;
            }
            assert!(state.prepared);
            state.completed = true;
            Some(*state)
        });
        let Some(replay) = replay else {
            return;
        };
        let saved = replay.saved.unwrap();
        self.check_reused_child(completion.group, saved, replay.lifetime);
        let before = self.test_snapshot();
        let permit = self.permit;
        assert!(permit.is_some());
        self.complete(saved, output);
        assert_eq!(self.test_snapshot(), before);
        assert_eq!(self.permit, permit);
        assert!(self.pending.at(completion.group.token.0[1] as usize).unwrap().staged);
    }
    fn check_reused_child(&self, current: api::Group, old: api::Completion, lifetime: u64) {
        assert_eq!(current.token.0[1], old.group.token.0[1], "the actual parent slot was reused");
        assert_eq!(current.token.0[3], old.group.token.0[3], "the actual child slot was reused");
        assert_ne!(current.token.0[2], old.group.token.0[2]);
        assert_ne!(self.work.at(token::child(current.token)).serial, lifetime);
        assert!(self.pending.at(current.token.0[1] as usize).unwrap().staged);
    }
}
