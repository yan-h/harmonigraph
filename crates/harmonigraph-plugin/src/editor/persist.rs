use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam::atomic::AtomicCell;
use serde::{Deserialize, Serialize};

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
