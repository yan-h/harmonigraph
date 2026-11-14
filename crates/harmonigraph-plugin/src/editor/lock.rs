//! The editor state's lock, and the inbox a host restore waits in.
//!
//! [`EditorShared`] lives as long as the plugin, and its settings have one
//! input and one output (#1757): a host restore is installed into it, and a
//! host save serializes out of it ([`UiState`](super::UiState)). Nothing reads
//! a saved document back, so closing and reopening the window leaves the
//! state, its look slots' undo histories included, where it was (#1740).
//!
//! A restore cannot be installed where it arrives. The host calls it on its
//! main thread inside nice-plug's configuration producer lock, which a frame
//! takes while holding the editor lock, so taking the editor lock there would
//! order the two locks both ways. It waits in the [`Inbox`] instead, a leaf
//! lock held only for a swap or a clone, and every acquisition of the editor
//! lock installs it before handing out the state. That is a property of the
//! lock type rather than a step each of its holders remembers: the raw mutex
//! is private, and no accessor skips the delivery.
//!
//! Lock order: editor, then producer (a frame's submit); producer, then inbox
//! (a restore); editor, then inbox (a delivery, a save, a close). The inbox
//! takes nothing, so the graph has no cycle.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, MutexGuard};

use super::shared::EditorShared;

/// The editor state behind a lock that delivers host restores by construction.
pub struct EditorLock {
    state: Mutex<EditorShared>,
    inbox: Arc<Inbox>,
}

impl EditorLock {
    pub(crate) fn new(state: EditorShared, inbox: Arc<Inbox>) -> EditorLock {
        EditorLock { state: Mutex::new(state), inbox }
    }

    /// The state, holding every restore the host has made.
    pub(crate) fn lock(&self) -> MutexGuard<'_, EditorShared> {
        self.delivered(self.state.lock())
    }

    pub(crate) fn try_lock(&self) -> Option<MutexGuard<'_, EditorShared>> {
        self.state.try_lock().map(|guard| self.delivered(guard))
    }

    pub(crate) fn try_lock_for(&self, wait: Duration) -> Option<MutexGuard<'_, EditorShared>> {
        self.state.try_lock_for(wait).map(|guard| self.delivered(guard))
    }

    /// Install the newest restore waiting, if any. The parse runs here, under
    /// the editor lock, rather than in the restore: its outcome decides what a
    /// save stores, which lives under this lock too.
    fn delivered<'a>(
        &self,
        mut guard: MutexGuard<'a, EditorShared>,
    ) -> MutexGuard<'a, EditorShared> {
        if let Some(document) = self.inbox.take() {
            guard.ui.install_document(&document);
        }
        guard
    }

    /// The window is closing: the plugin's close, as one call a test reaches.
    ///
    /// The order is the point. `end_gestures` is a host callback made under
    /// this lock (`end_set_parameter`), and a host may save from inside it; a
    /// save there cannot take the lock, so it answers with the inbox's
    /// fallback, which must already hold the session. So the session ends and
    /// the fallback is refreshed before the gestures end.
    pub(crate) fn end_window_session(
        &self,
        end_gestures: impl FnOnce([bool; harmonigraph_ui::params::ParamKey::ALL.len()]),
    ) {
        let mut shared = self.lock();
        shared.ui.end_window_session();
        self.inbox.settle(shared.ui.save_answer());
        let gestures =
            shared.gesture.replace([false; harmonigraph_ui::params::ParamKey::ALL.len()]);
        end_gestures(gestures);
        shared.forget_frames();
    }

    /// The state as it stands, any restore still waiting left waiting: what a
    /// protocol test compares against its oracle between events.
    #[cfg(test)]
    pub(crate) fn undelivered(&self) -> MutexGuard<'_, EditorShared> {
        self.state.lock()
    }
}

/// The state half of a window's build: the lock (which delivers a waiting
/// restore), [`Opening`](harmonigraph_ui::shell::Opening), and the loading
/// screen. Nothing saved is read: the state already holds the settings.
pub(crate) fn open_window_session(lock: &EditorLock, ctx: &egui::Context) {
    let mut shared = lock.lock();
    harmonigraph_ui::shell::Opening { ctx, state: &mut shared.ui }.open();
    harmonigraph_ui::begin_editor_loading(ctx);
}

/// Where a host restore waits for the editor lock, and what a save answers
/// when it cannot take that lock.
#[derive(Default)]
pub struct Inbox(Mutex<Mail>);

#[derive(Default)]
struct Mail {
    /// The newest restore no lock holder has installed yet.
    pending: Option<Arc<str>>,
    /// What a save that cannot take the editor lock returns: the newest of
    /// the last restore, close or successful save. Never read back into the
    /// state. It exists because the editor lock is not reentrant, and a host
    /// may save from a callback the editor makes under it.
    fallback: Arc<str>,
}

impl Inbox {
    /// A host restore, the empty document included: the newest replaces any
    /// still waiting, and is what a save answers until the state holds it.
    pub(crate) fn deliver(&self, document: Arc<str>) {
        let mut mail = self.0.lock();
        mail.pending = Some(document.clone());
        mail.fallback = document;
    }

    fn take(&self) -> Option<Arc<str>> {
        self.0.lock().pending.take()
    }

    pub(crate) fn fallback(&self) -> Arc<str> {
        self.0.lock().fallback.clone()
    }

    /// Keep `answer`, which a holder of the editor lock just computed, as the
    /// fallback — unless a restore has arrived since that holder's delivery:
    /// the restore is newer, and the fallback must not predate it.
    pub(crate) fn settle(&self, answer: Arc<str>) {
        let mut mail = self.0.lock();
        if mail.pending.is_none() {
            mail.fallback = answer;
        }
    }
}

#[cfg(test)]
mod tests;
