use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use crossbeam::atomic::AtomicCell;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use super::shared::EditorShared;

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
}

impl EguiState {
    pub fn from_size(width: u32, height: u32) -> Arc<EguiState> {
        Arc::new(EguiState {
            size: AtomicCell::new((width, height)),
            requested_size: AtomicCell::new(None),
            host_resized: AtomicCell::new(None),
            open: AtomicBool::new(false),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size.load()
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Record that the window has opened or closed.
    ///
    /// Named rather than stored inline because it is not only the host's
    /// business: [`crate::background`] reads it to decide whether a frame is
    /// already draining the rings, so the two transitions are a seam between
    /// threads and not just a bit.
    pub(crate) fn set_open(&self, open: bool) {
        self.open.store(open, Ordering::Release);
    }
}

impl<'a> nice_plug::params::persist::PersistentField<'a, EguiState> for Arc<EguiState> {
    fn set(&self, new_value: EguiState) {
        self.size.store(new_value.size.load());
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&EguiState) -> R,
    {
        f(self)
    }
}

/// How long a host's state save waits for a frame to let go of the editor's
/// state before it settles for the blob the last close wrote.
///
/// Short, because on macOS and Windows the host saves on the same thread the
/// frames run on: the lock is then either free at once, or held by a frame
/// that is itself waiting on this save (a host answering something the frame
/// asked by saving state), and the lock is not reentrant, so no wait helps.
/// Only on X11, where the editor has its own thread, can a frame be mid-run
/// in parallel, and a frame holds the lock for a few milliseconds. Bounded
/// rather than a plain `lock` so the re-entrant case costs the save its
/// freshness and not the host a hang — the kind issue #296 is about.
const SAVE_WAIT: std::time::Duration = std::time::Duration::from_millis(20);

/// The editor's saved settings: the `ui-state` blob the host stores with the
/// project, plus a way to serialize them live while a window is open.
///
/// The blob is written by the host's restore and by the editor window's
/// `Drop`, and nothing else. A save that only read it would store the settings
/// of the last CLOSE, so with a window open, [`PersistentField::map`](nice_plug::params::persist::PersistentField::map) — which is the
/// one thing a host save calls — serializes what the window is showing
/// instead. The blob itself is left alone: the background analyzer adopts it
/// whenever it changes with the window shut, and that has to keep meaning a
/// host restore or a close (see [`crate::background`]).
///
/// A restore also marks the blob `pending`, which is how one reaches a window
/// that is already open (issue #1329): the background analyzer stays off an
/// open window, and `Opening` has already run. The window's next frame applies
/// it ([`apply_restore`](Self::apply_restore)), and until then a save stores the
/// blob, since the window is still showing what the restore replaced.
///
/// **Threads.** A restore, a save and the window's close all arrive on the
/// host's main thread, so they never interleave with one another. The window's
/// build and frames run on that thread too on macOS and Windows, and on an
/// editor thread of their own on X11. So the one real race is a frame or a
/// build against a restore or a save, and it is settled by a single rule:
/// `pending` is only ever CLEARED with the editor-state lock held, in the same
/// hold that applies the blob to the state, and a save reads it under that
/// lock too. A save then sees either the flag still set (the state does not
/// hold the restore yet, so it stores the blob) or the state with the restore
/// in it. A restore needs no lock: it writes the blob and only then sets the
/// flag, and a reader clears the flag and only then reads the blob, so a
/// restore landing between the two leaves the flag set and costs at most one
/// redundant re-apply on the next frame, never a lost restore. Two restores
/// before a frame are just the second overwriting the first.
pub struct UiState {
    blob: Arc<RwLock<String>>,
    /// Set by a host restore of a non-empty blob, cleared by whichever of the
    /// window's build, its frame or its close next applies the blob — always
    /// under the editor-state lock (see the type's docs on why that matters).
    /// Set whether or not a window is open: one being built on X11 reads the
    /// blob on its own thread, and a restore landing with the window half
    /// built would otherwise reach nothing.
    pending: AtomicBool,
    /// Weak, so this field never keeps `EditorShared` alive: the plugin orders
    /// where that drop happens deliberately (see `Harmonigraph::_background`),
    /// and these params are shared beyond the plugin (the editor holds them).
    live: OnceLock<(Weak<Mutex<EditorShared>>, Arc<EguiState>)>,
}

impl UiState {
    /// An empty blob, which is what a project whose editor has never been
    /// open carries, and nothing live yet.
    pub fn new() -> UiState {
        UiState {
            blob: Arc::new(RwLock::new(String::new())),
            pending: AtomicBool::new(false),
            live: OnceLock::new(),
        }
    }

    /// The stored blob: the last host restore or window close.
    pub fn blob(&self) -> &Arc<RwLock<String>> {
        &self.blob
    }

    /// Give the save the editor state to serialize while `window` says a
    /// window is open. Once per plugin; the params exist before the state
    /// does, hence the two steps.
    pub(crate) fn attach(&self, shared: &Arc<Mutex<EditorShared>>, window: Arc<EguiState>) {
        if self.live.set((Arc::downgrade(shared), window)).is_err() {
            unreachable!("the editor state is attached once, by the plugin that owns both");
        }
    }

    /// The blob a window being built opens with, which also answers any
    /// restore pending against it.
    ///
    /// Call with the editor-state lock held, and apply the result in the same
    /// hold: clearing the flag is a claim that the state is about to hold the
    /// blob, and a save on another thread reads the flag under that lock.
    pub(crate) fn opening_blob(&self) -> String {
        // Cleared BEFORE the read, so a restore landing in between leaves the
        // flag set rather than being taken as read.
        self.pending.swap(false, Ordering::Acquire);
        self.blob.read().clone()
    }

    /// Apply a host restore the open window has not shown yet. Called by every
    /// frame, so it is one atomic swap when there is none.
    ///
    /// Taking the state as `&mut` is what keeps the flag's rule: the only way
    /// to one outside a test is through the editor-state lock.
    pub(crate) fn apply_restore(&self, ui: &mut harmonigraph_ui::SharedState) {
        if self.pending.swap(false, Ordering::Acquire) {
            let blob = self.blob.read().clone();
            // A restore of an empty blob clears the flag rather than setting
            // it, but one can still land between the swap and the read on X11.
            if !blob.is_empty() {
                ui.load_persist(&blob);
            }
        }
    }

    /// Store what a closing window leaves, for the save and the next open.
    ///
    /// A restore that landed after the window's last frame is applied first:
    /// it is newer than anything the window showed, and writing the window's
    /// state over it would lose it at the close instead of at the next save.
    /// The blob written is then the restore's own round trip.
    pub(crate) fn store_close(&self, ui: &mut harmonigraph_ui::SharedState) {
        self.apply_restore(ui);
        *self.blob.write() = harmonigraph_ui::shell::close(ui);
    }
}

impl Default for UiState {
    fn default() -> UiState {
        UiState::new()
    }
}

impl<'a> nice_plug::params::persist::PersistentField<'a, String> for UiState {
    /// Called when the host restores state, on its main thread.
    ///
    /// An EMPTY blob clears the flag instead of setting it: it is a project
    /// with nothing saved, which the window and the background analyzer both
    /// read as "keep what you have" rather than as a reset.
    fn set(&self, new_value: String) {
        let pending = !new_value.is_empty();
        *self.blob.write() = new_value;
        // AFTER the write, with Release, so a frame that takes the flag reads
        // this blob or a newer one.
        self.pending.store(pending, Ordering::Release);
    }

    /// Called when the host saves state, on its main thread. One RON
    /// serialization per save, which is what a window close already spends;
    /// nothing per frame.
    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&String) -> R,
    {
        if let Some((shared, window)) = self.live.get() {
            if let Some(shared) = shared.upgrade().filter(|_| window.is_open()) {
                // `save_persist` rather than `shell::close`: this is a save
                // of an editor that stays open, not the way out of one.
                match shared.try_lock_for(SAVE_WAIT) {
                    // Read under the lock, where it is cleared: a flag still
                    // up means the state does not hold the latest restore
                    // yet, and the blob is that restore.
                    Some(_) if self.pending.load(Ordering::Acquire) => {}
                    Some(shared) => {
                        let live = shared.ui.save_persist();
                        drop(shared);
                        return f(&live);
                    }
                    None => nice_plug::nice_warn!(
                        "editor state busy for {SAVE_WAIT:?}; saving the settings of the \
                         last restore or editor close instead of the open window's"
                    ),
                }
            }
        }
        f(&self.blob.read())
    }
}
