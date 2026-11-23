//! The editor state's lock, and the inbox a host restore waits in.
//!
//! [`EditorShared`] lives as long as the plugin, and its settings have one
//! input and one output (#1757): a host restore is installed into it, and a
//! host save serializes out of it ([`UiState`](super::UiState)). Nothing reads
//! a saved document back, so closing and reopening the window leaves the
//! state, its look slots' undo histories included, where it was (#1740).
//!
//! A restore is not installed where it arrives. The host calls it on its
//! main thread, where taking the editor lock would wait out the input
//! worker's holds (`background.rs`), time taken from the host's main thread,
//! and would deadlock if a host ever restored from inside a frame's own hold.
//! It waits in the [`Inbox`] instead, a leaf lock held only for a swap or a
//! clone, and every acquisition of the editor lock installs it before handing
//! out the state. That is a property of the
//! lock type rather than a step each of its holders remembers: the raw mutex
//! is private, and no accessor skips the delivery.
//!
//! Lock order: editor, then the tuning document (a frame's edit or view, and
//! the input worker's Learn); editor, then inbox (a delivery, a save, a
//! close). A restore takes the document and the inbox each alone, and no
//! document closure takes the editor lock (`crate::shared`), so the graph has
//! no cycle.

use std::sync::Arc;
use std::time::Duration;

use harmonigraph_ui::SharedState;
use parking_lot::{Mutex, MutexGuard};

use super::gestures::Gestures;
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
    /// the editor lock, rather than in the host's restore call: its Console
    /// lines and the settings it installs are this lock's state.
    fn delivered<'a>(
        &self,
        mut guard: MutexGuard<'a, EditorShared>,
    ) -> MutexGuard<'a, EditorShared> {
        self.inbox.install_pending(&mut guard.ui);
        guard
    }

    /// The window is closing: the plugin's close, as one call a test reaches.
    /// An edit still open becomes one undo entry, the fallback a save answers
    /// with when it cannot take this lock is refreshed with the session, and
    /// the open host gestures are handed back, cleared, for the caller to end
    /// ([`gestures::end`](super::gestures::end)) once this lock is let go: a
    /// host may save from inside `end_set_parameter`, and that save then
    /// takes the lock and answers with the live settings.
    pub(crate) fn end_window_session(&self) -> Gestures {
        let mut shared = self.lock();
        shared.ui.finish_open_edit();
        self.inbox.settle(shared.ui.save_answer());
        shared.gestures.get_mut().take()
    }

    /// The editor has stopped being seen, and runs no frames until it is
    /// seen again: what its frames would have ended is ended here. An edit
    /// still open becomes one undo entry, and the open host gestures are
    /// handed back, cleared, for the caller to end once this lock is let go,
    /// as at a close. No save answer is made: a loss of sight is no session's
    /// end, and serializing the settings on every hide buys nothing.
    ///
    /// `None` if the lock is not free within [`UNSEEN_WAIT`]. The notice that
    /// calls this comes on the main thread, outside any frame, so the only
    /// holder it can meet is the input worker, for a step or the lattice-board
    /// lights' poll (at most a few milliseconds); the host's saves, the window's build and its close
    /// hold the lock on the main thread but hide or move no view while they
    /// do. Should one ever, the gestures stay open, as before this existed,
    /// until the reveal's pass ends them or the close does.
    pub(crate) fn end_unseen(&self) -> Option<Gestures> {
        let mut shared = self.try_lock_for(UNSEEN_WAIT)?;
        shared.ui.finish_open_edit();
        Some(shared.gestures.get_mut().take())
    }
}

/// How long [`EditorLock::end_unseen`] waits for the editor lock: past any
/// hold the input worker makes, short of a wait the host's main thread
/// would feel.
const UNSEEN_WAIT: Duration = Duration::from_millis(20);

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

    /// Install the newest restore waiting into `ui`, if any: the document,
    /// and whether its settings were installed
    /// ([`SharedState::install_document`]). Called by every holder of the
    /// editor lock, and by the plugin's state reader (`saved_state`), which
    /// has no editor around its state.
    pub(crate) fn install_pending(&self, ui: &mut SharedState) -> Option<(Arc<str>, bool)> {
        let document = self.0.lock().pending.take()?;
        let installed = ui.install_document(&document);
        Some((document, installed))
    }

    pub(crate) fn fallback(&self) -> Arc<str> {
        self.0.lock().fallback.clone()
    }

    /// Keep `answer`, which a holder of the editor lock just computed, as the
    /// fallback. A restore, a save and a close all arrive on the host's main
    /// thread, so none lands inside another's hold.
    pub(crate) fn settle(&self, answer: Arc<str>) {
        self.0.lock().fallback = answer;
    }
}
