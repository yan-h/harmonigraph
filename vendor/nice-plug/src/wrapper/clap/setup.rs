//! Optional off-audio setup/pairing state, independent of musical configuration.
//! The wrapper prepares before restoring any ordinary fields, commits only on
//! success, and never calls these methods under its audio/plugin runtime lock.
use nice_plug_core::plugin::PluginState;

pub trait Prepared: Send {
    /// Off-thread commit of a previously reserved complete value. This cannot
    /// fail or require an audio callback; Drop before commit abandons preparation.
    fn commit(self: Box<Self>);
}

pub trait Setup: Send + Sync {
    fn prepare(&self, state: &PluginState) -> Result<Box<dyn Prepared>, &'static str>;
    fn save(&self, state: &mut PluginState);
    /// Called during construction. The callback schedules independent main
    /// work even when the generic task queue is full and defers host reentry
    /// until any enclosing performance callback's runtime borrows have ended.
    fn install_wakeup(&self, wake: Box<dyn Fn() + Send + Sync>);
    /// Main-thread service of registry returns and setup notifications. True
    /// requests host parameter rescan/state-dirty after this call has returned.
    fn service(&self) -> bool;
}
