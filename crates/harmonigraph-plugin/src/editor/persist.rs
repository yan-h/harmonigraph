use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use crossbeam::atomic::AtomicCell;
use serde::{Deserialize, Serialize};

use super::lock::{EditorLock, Inbox};

/// Window size persistence, modeled on nih_plug_egui's `EguiState`.
#[derive(Debug, Serialize, Deserialize)]
pub struct EguiState {
    /// Size in logical pixels (before DPI scaling).
    #[serde(with = "atomic_cell")]
    pub(super) size: AtomicCell<(u32, u32)>,
    #[serde(skip)]
    pub(super) requested_size: AtomicCell<Option<(u32, u32)>>,
    /// A size the host already applied to the parent window (native border
    /// drag); the child view and render surface must be brought to match,
    /// WITHOUT the request_resize round-trip used for plugin-initiated
    /// resizes.
    ///
    /// Collected by the window's size source (see [`LatticeEditor::spawn`](super::window::LatticeEditor))
    /// rather than in the frame, because the frame has to be LAID OUT at this
    /// size, and by the time the frame is running it is too late to say so.
    #[serde(skip)]
    pub(super) host_resized: AtomicCell<Option<(u32, u32)>>,
    #[serde(skip)]
    open: AtomicBool,
    /// How a thread that changes what the open window shows asks it to draw
    /// that on its next frame (see [`Self::request_pass`]).
    #[serde(skip)]
    pub(crate) pass_request: egui_baseview::PassRequest,
    /// The editor kept in the host's window rather than its own (see
    /// `pop_out`), from the System tab. Saved with the project; a project
    /// from before it was opens in the editor's own window, which is the
    /// default.
    #[serde(default)]
    docked: AtomicBool,
    /// The editor's view is in its own window right now, which then has the
    /// say over its size (see `pop_out`). Set as the view moves, on the main
    /// thread, where a host `set_size` also arrives (a CLAP `gui` call), so
    /// the two are ordered. Here because the editor that receives `set_size`
    /// does not hold the window, whose state is bound to the main thread; not
    /// [`Self::docked`], the setting, which the move follows a run-loop turn
    /// later, and which reads "own window" on a platform that offers none.
    #[serde(skip)]
    in_own_window: AtomicBool,
    /// The size and the Own window choice as the host was last told a save
    /// holds them, by a report or a restore ([`Self::report_edit`]); `None`
    /// from a close or an opening until the next frame ends (#1824).
    #[serde(skip)]
    told: AtomicCell<Option<Saved>>,
}

/// What the `editor-state` save holds: the size, and whether the editor is
/// kept in the host's window.
type Saved = ((u32, u32), bool);

impl EguiState {
    pub fn from_size(width: u32, height: u32) -> Arc<EguiState> {
        Arc::new(EguiState {
            size: AtomicCell::new((width, height)),
            requested_size: AtomicCell::new(None),
            host_resized: AtomicCell::new(None),
            open: AtomicBool::new(false),
            pass_request: Default::default(),
            docked: AtomicBool::new(false),
            in_own_window: AtomicBool::new(false),
            told: AtomicCell::new(None),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size.load()
    }

    pub(super) fn docked(&self) -> bool {
        self.docked.load(Ordering::Relaxed)
    }

    pub(crate) fn set_docked(&self, docked: bool) {
        self.docked.store(docked, Ordering::Relaxed);
    }

    pub(super) fn in_own_window(&self) -> bool {
        self.in_own_window.load(Ordering::Relaxed)
    }

    pub(super) fn set_in_own_window(&self, own: bool) {
        self.in_own_window.store(own, Ordering::Relaxed);
    }

    /// Whether the editor's size is its own window's, which the project
    /// saves, so that a host `set_size`, which then sizes only the host's
    /// window, must not replace it: while the editor is open, whether its view
    /// is in its own window; before the host attaches it (CLAP allows
    /// `set_size` before `set_parent`), whether it will open there, which the
    /// saved choice says where the platform offers one (Yan decided the own
    /// window opens at the size its project saved, egui-plugin-window#12).
    pub(super) fn owns_size(&self) -> bool {
        if self.is_open() {
            self.in_own_window()
        } else {
            super::OWN_WINDOW_OFFERED && !self.docked()
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Record that the window has opened or closed.
    ///
    /// Named rather than stored inline because it is not only the host's
    /// business: [`crate::background`] reads it to decide how often to poll,
    /// and the tape whether its roll is drawn, so the two transitions are a
    /// seam between threads and not just a bit.
    ///
    /// Either starts the next window's settle, so that what the host and the
    /// own window size while it opens is no edit ([`Self::report_edit`]): from
    /// the close, for a frame the own window draws on the spot as the view
    /// moves into it, inside `Editor::spawn` before the opening is recorded;
    /// and again at the opening, for the frames after.
    pub(crate) fn set_open(&self, open: bool) {
        self.told.store(None);
        self.open.store(open, Ordering::Release);
    }

    /// Ask the open window to run its UI on its next frame, from any thread.
    ///
    /// A still window runs no pass until something asks for one or its idle
    /// poll comes due, and a change made off its thread asks nothing by
    /// itself: without this it is drawn up to a poll later. Asking while the
    /// window is shut costs one store, and the next window's first frame runs
    /// a pass anyway.
    pub(crate) fn request_pass(&self) {
        self.pass_request.request();
    }
}

impl EguiState {
    /// Take what a restore saved: the size and the docked choice, the two
    /// fields the `editor-state` save holds.
    /// A restore is no edit: it is what the host was told, unless no frame has
    /// ended since a close or an opening, whose next frame takes it as told.
    pub(crate) fn restore(&self, saved: EguiState) {
        self.size.store(saved.size.load());
        self.set_docked(saved.docked());
        self.told.store(self.told.load().map(|_| self.saved()));
    }

    fn saved(&self) -> Saved {
        (self.size(), self.docked())
    }

    /// Call `notice` if the size or the Own window choice moved since the
    /// host was last told what a save holds of them, and take them as told:
    /// at the end of every editor frame, so that a resize or an Own window
    /// switch marks the project modified as an edit to `ui-state` does
    /// (#1824), once per frame that moved them. The first frame after a close
    /// or an opening only takes them as told, so what the host and the own
    /// window set while the editor opens, such as a host's `set_size` before
    /// or after it attaches the editor and the own window's move out, marks
    /// nothing.
    pub(crate) fn report_edit(&self, notice: impl FnOnce()) {
        let now = self.saved();
        if self.told.swap(Some(now)).is_some_and(|told| told != now) {
            notice();
        }
    }
}

/// An `AtomicCell` saved as the value it holds.
mod atomic_cell {
    use crossbeam::atomic::AtomicCell;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S, T>(cell: &AtomicCell<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        T: Serialize + Copy,
    {
        cell.load().serialize(serializer)
    }

    pub fn deserialize<'de, D, T>(deserializer: D) -> Result<AtomicCell<T>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de> + Copy,
    {
        T::deserialize(deserializer).map(AtomicCell::new)
    }
}

/// How long a host's state save waits for the editor's state lock before it
/// settles for the inbox's fallback.
///
/// Short, because on macOS the host saves on the same thread the frames run
/// on, so a frame holding the lock there is one that is itself waiting on
/// this save (a host answering something the frame asked by saving state),
/// and the lock is not reentrant, so no wait helps. What can hold it in
/// parallel is the input worker, for a take or a give (microseconds, a few
/// milliseconds for a full ring; see [`crate::background`]). Bounded rather
/// than a plain `lock`
/// so the re-entrant case costs the save its freshness and not the host a
/// hang — the kind issue #296 is about.
const SAVE_WAIT: Duration = Duration::from_millis(20);

/// The editor's saved settings, the `ui-state` field the host stores with the
/// project: an input into the live state and an output out of it, never a
/// copy that is read back (#1757).
///
/// A restore ([`Self::restore`]) goes to the [`Inbox`], which the next holder of the
/// editor lock installs from — a frame, the input worker, the window's build
/// or close, or a save. A save ([`Self::save`], the one thing a host save calls)
/// takes that lock, window open or shut, and stores what
/// [`SharedState::save_answer`](harmonigraph_ui::SharedState::save_answer)
/// says: the live settings, or `""` for an editor nobody has opened since it
/// was fresh. Both arrive on the host's main thread.
///
/// The one limit: a save the host makes from inside a callback the editor
/// makes under its lock — a parameter gesture's flush, a camera move's — cannot
/// take the lock, and returns the fallback: the newest of the last restore,
/// close or successful save (#1301).
pub struct UiState {
    inbox: Arc<Inbox>,
    /// Weak, so this field never keeps `EditorShared` alive: the plugin orders
    /// where that drop happens deliberately (see `Harmonigraph::_input_worker`),
    /// and these params are shared beyond the plugin (the editor holds them).
    live: OnceLock<Weak<EditorLock>>,
}

impl UiState {
    /// An empty inbox, and nothing live yet.
    pub fn new() -> UiState {
        UiState { inbox: Arc::default(), live: OnceLock::new() }
    }

    /// Where a restore waits; the editor lock is built around it.
    pub(crate) fn inbox(&self) -> Arc<Inbox> {
        self.inbox.clone()
    }

    /// Give the save the editor state to serialize. Once per plugin; the
    /// params exist before the state does, hence the two steps.
    pub(crate) fn attach(&self, lock: &Arc<EditorLock>) {
        if self.live.set(Arc::downgrade(lock)).is_err() {
            unreachable!("the editor state is attached once, by the plugin that owns both");
        }
    }

    /// What a save stores, waiting at most [`SAVE_WAIT`] for the editor lock.
    /// Called when the host saves state, on its main thread: one RON
    /// serialization per save, window open or shut.
    pub(crate) fn save(&self) -> Arc<str> {
        let Some(lock) = self.live.get().and_then(Weak::upgrade) else {
            return self.inbox.fallback();
        };
        let answer = lock.try_lock_for(SAVE_WAIT).map(|shared| {
            let answer = shared.ui.save_answer();
            self.inbox.settle(answer.clone());
            answer
        });
        answer.unwrap_or_else(|| {
            nice_plug::nice_warn!(
                "editor state busy for {SAVE_WAIT:?}; saving the settings of the last restore, \
                 close or save instead of the editor's"
            );
            self.inbox.fallback()
        })
    }
}

impl Default for UiState {
    fn default() -> UiState {
        UiState::new()
    }
}

impl UiState {
    /// Called when the host restores state, on its main thread, where it must
    /// not take the editor lock (see `lock`'s module doc) — which is why it
    /// only posts.
    pub(crate) fn restore(&self, saved: String) {
        self.inbox.deliver(saved.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The System tab's Own window choice is saved with the window size, and a
    /// project saved before it existed opens in the editor's own window.
    #[test]
    fn the_own_window_choice_is_saved_and_defaults_to_own() {
        let docked = EguiState::from_size(800, 600);
        docked.set_docked(true);
        let saved = serde_json::to_string(&*docked).unwrap();
        let restored = EguiState::from_size(1, 1);
        restored.restore(serde_json::from_str(&saved).unwrap());
        assert!(restored.docked());
        assert_eq!(restored.size(), (800, 600));

        let mut old: serde_json::Value = serde_json::from_str(&saved).unwrap();
        old.as_object_mut().unwrap().remove("docked");
        let restored = EguiState::from_size(1, 1);
        restored.restore(serde_json::from_value(old).unwrap());
        assert!(!restored.docked());
    }
}
