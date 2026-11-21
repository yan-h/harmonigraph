use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use crossbeam::atomic::AtomicCell;
use plugin_own_window::{EditorSize, Opened};
use serde::{Deserialize, Serialize};

use super::lock::{EditorLock, Inbox};

/// The editor's window: its size and the own window, which egui-plugin-window's
/// [`EditorSize`] owns, and the System tab's Own window choice, which the
/// project saves beside the size.
#[derive(Debug)]
pub struct EguiState {
    /// The size the host accepted or the own window gave, the host's
    /// negotiation of it, and the own window itself.
    pub(super) window: Arc<EditorSize>,
    /// How a thread that changes what the open window shows asks it to draw
    /// that on its next frame (see [`Self::request_pass`]).
    pub(crate) pass_request: egui_baseview::PassRequest,
    /// The editor kept in the host's window rather than its own, from the
    /// System tab. Saved with the project; a project
    /// from before it was opens in the editor's own window, which is the
    /// default.
    docked: AtomicBool,
    /// The size and the Own window choice as the host was last told a save
    /// holds them, by a report ([`Self::report_edit`]); `None` from a close,
    /// an opening or a restore until the next frame ends (#1824).
    told: AtomicCell<Option<SavedWindow>>,
}

/// What the `editor-state` save holds, `{"size":[w,h],"docked":b}`: the size
/// in logical points, and whether the editor is kept in the host's window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedWindow {
    pub(crate) size: (u32, u32),
    #[serde(default)]
    pub(crate) docked: bool,
}

impl EguiState {
    pub fn from_size(width: u32, height: u32) -> Arc<EguiState> {
        Arc::new(EguiState {
            window: EditorSize::new(super::NAME, (width, height), super::LIMITS),
            pass_request: Default::default(),
            docked: AtomicBool::new(false),
            told: AtomicCell::new(None),
        })
    }

    /// The size the host last accepted, or the own window gave, in logical
    /// points: what a save writes and what the window has.
    pub fn size(&self) -> (u32, u32) {
        self.window.size()
    }

    pub(super) fn docked(&self) -> bool {
        self.docked.load(Ordering::Relaxed)
    }

    pub(crate) fn set_docked(&self, docked: bool) {
        self.docked.store(docked, Ordering::Relaxed);
    }

    /// Whether the host has the editor attached.
    ///
    /// Not only the host's business: [`crate::background`] reads it to decide
    /// how often to poll, and the tape whether its roll is drawn.
    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }

    /// The host attached the editor, before its window opens: the token its
    /// size source hands to [`EditorSize::next_frame`].
    pub(crate) fn open(&self) -> Opened {
        self.window.open()
    }

    /// Start the window's settle ([`Self::report_edit`]) again, after the one
    /// the last close started, once the editor has opened, so that what the
    /// host and the own window size while it opens is no edit: the first
    /// frame after this only takes the size and the choice as told. Called
    /// after the view moves into its own window, which can draw a frame on
    /// the spot, and before the host's own `set_size` that follows an attach.
    pub(crate) fn settle(&self) {
        self.told.store(None);
    }

    /// The editor is closing, before baseview closes the view: the own window
    /// closes, a size asked for and not yet negotiated is installed for the
    /// save and the next open ([`EditorSize::close`]), and the next window's
    /// settle starts.
    pub(crate) fn close(&self) {
        self.window.close();
        self.told.store(None);
    }

    /// Open or close the editor as the host's attach and destroy do, with no
    /// window: for tests of what reads whether it is open.
    #[cfg(test)]
    pub(crate) fn set_open(&self, open: bool) {
        if open {
            self.open();
        } else {
            self.close();
        }
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
    /// fields the `editor-state` save holds. With the editor shut the size is
    /// installed at once; open, the window is resized to it at its next frame
    /// ([`EditorSize::request_size`]), and a save until then holds the size
    /// the window has.
    /// A restore is no edit: the frame that applies it only takes the result
    /// as told, whatever the host or the own window made of the size.
    pub(crate) fn restore(&self, saved: SavedWindow) {
        self.window.request_size(saved.size);
        self.set_docked(saved.docked);
        self.told.store(None);
    }

    /// What the `editor-state` save holds now.
    pub(crate) fn saved(&self) -> SavedWindow {
        SavedWindow { size: self.size(), docked: self.docked() }
    }

    /// Call `notice` if the size or the Own window choice moved since the
    /// host was last told what a save holds of them, and take them as told:
    /// at the end of every editor frame, so that a resize or an Own window
    /// switch marks the project modified as an edit to `ui-state` does
    /// (#1824), once per frame that moved them. The first frame after a close,
    /// an opening or a restore only takes them as told, so what the host and
    /// the own window set while the editor opens, such as a host's `set_size`
    /// before or after it attaches the editor and the own window's move out,
    /// and the resize a restore makes, mark nothing.
    pub(crate) fn report_edit(&self, notice: impl FnOnce()) {
        let now = self.saved();
        if self.told.swap(Some(now)).is_some_and(|told| told != now) {
            notice();
        }
    }
}

/// How long a host's state save waits for the editor's state lock before it
/// settles for the inbox's fallback.
///
/// Short, because on macOS the host saves on the same thread the frames run
/// on, so a frame holding the lock there is one that is itself waiting on
/// this save (a host answering something the frame asked by saving state),
/// and the lock is not reentrant, so no wait helps. What can hold it in
/// parallel is the input worker, for a step or the lattice-board lights' poll
/// (microseconds, a few milliseconds for a full ring; see [`crate::background`]). Bounded rather
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
            harmonigraph_clap::shell_warn!(
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
        let saved = serde_json::to_string(&docked.saved()).unwrap();
        assert_eq!(saved, r#"{"size":[800,600],"docked":true}"#, "the bytes a project holds");
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
