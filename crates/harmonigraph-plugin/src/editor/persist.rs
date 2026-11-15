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
    #[serde(with = "nice_plug::params::persist::serialize_atomic_cell")]
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
    /// The editor kept in the host's window rather than its own (see
    /// `PopOut`), from the System tab. Saved with the project; a project
    /// from before it was opens in the editor's own window, which is the
    /// default.
    #[serde(default)]
    docked: AtomicBool,
}

impl EguiState {
    pub fn from_size(width: u32, height: u32) -> Arc<EguiState> {
        Arc::new(EguiState {
            size: AtomicCell::new((width, height)),
            requested_size: AtomicCell::new(None),
            host_resized: AtomicCell::new(None),
            open: AtomicBool::new(false),
            docked: AtomicBool::new(false),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size.load()
    }

    pub(super) fn docked(&self) -> bool {
        self.docked.load(Ordering::Relaxed)
    }

    pub(super) fn set_docked(&self, docked: bool) {
        self.docked.store(docked, Ordering::Relaxed);
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Record that the window has opened or closed.
    ///
    /// Named rather than stored inline because it is not only the host's
    /// business: [`crate::background`] reads it to decide whether to do a shut
    /// window's duties, and how often to poll, so the two transitions are a
    /// seam between threads and not just a bit.
    pub(crate) fn set_open(&self, open: bool) {
        self.open.store(open, Ordering::Release);
    }
}

impl<'a> nice_plug::params::persist::PersistentField<'a, EguiState> for Arc<EguiState> {
    fn set(&self, new_value: EguiState) {
        self.size.store(new_value.size.load());
        self.set_docked(new_value.docked());
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&EguiState) -> R,
    {
        f(self)
    }
}

/// How long a host's state save waits for the editor's state lock before it
/// settles for the inbox's fallback.
///
/// Short, because on macOS and Windows the host saves on the same thread the
/// frames run on, so a frame holding the lock there is one that is itself
/// waiting on this save (a host answering something the frame asked by
/// saving state), and the lock is not reentrant, so no wait helps. What can
/// hold it in parallel is the input worker, for a take or a give
/// (microseconds, a few milliseconds for a full ring; see
/// [`crate::background`]), and on X11, where the editor has its own thread, a
/// frame mid-run, for a few milliseconds. Bounded rather than a plain `lock`
/// so the re-entrant case costs the save its freshness and not the host a
/// hang — the kind issue #296 is about.
const SAVE_WAIT: Duration = Duration::from_millis(20);

/// The editor's saved settings, the `ui-state` field the host stores with the
/// project: an input into the live state and an output out of it, never a
/// copy that is read back (#1757).
///
/// A restore (`set`) goes to the [`Inbox`], which the next holder of the
/// editor lock installs from — a frame, the input worker, the window's build
/// or close, or a save. A save (`map`, the one thing a host save calls)
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
    fn save(&self) -> Arc<str> {
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

impl<'a> nice_plug::params::persist::PersistentField<'a, String> for UiState {
    /// Called when the host restores state, on its main thread, under
    /// nice-plug's configuration producer lock — which is why it only posts.
    fn set(&self, new_value: String) {
        self.inbox.deliver(new_value.into());
    }

    /// Called when the host saves state, on its main thread: one RON
    /// serialization per save, window open or shut.
    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&String) -> R,
    {
        f(&self.save().to_string())
    }
}

#[cfg(test)]
mod tests {
    use nice_plug::params::persist::PersistentField;

    use super::*;

    /// The System tab's Own window choice is saved with the window size, and a
    /// project saved before it existed opens in the editor's own window.
    #[test]
    fn the_own_window_choice_is_saved_and_defaults_to_own() {
        let docked = EguiState::from_size(800, 600);
        docked.set_docked(true);
        let saved = serde_json::to_string(&*docked).unwrap();
        let restored = EguiState::from_size(1, 1);
        restored.set(serde_json::from_str(&saved).unwrap());
        assert!(restored.docked());
        assert_eq!(restored.size(), (800, 600));

        let mut old: serde_json::Value = serde_json::from_str(&saved).unwrap();
        old.as_object_mut().unwrap().remove("docked");
        let restored = EguiState::from_size(1, 1);
        restored.set(serde_json::from_value(old).unwrap());
        assert!(!restored.docked());
    }
}
