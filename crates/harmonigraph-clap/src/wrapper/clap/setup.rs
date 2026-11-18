//! Optional plugin-owned saved settings, outside the host parameter list.
//! The wrapper prepares before restoring any ordinary fields, commits only on
//! success, and never calls these methods under its audio/plugin runtime lock.
use nice_plug_core::plugin::PluginState;

pub trait Prepared: Send {
    /// Off-audio commit of a prepared complete value, after the ordinary
    /// fields restored. It needs no audio callback and reports nothing back:
    /// whatever prepare could refuse, it refused. Drop before commit abandons
    /// preparation.
    fn commit(self: Box<Self>);
}

pub trait Setup: Send + Sync {
    /// Main-thread host metadata, refreshed at init and on track-info changes.
    /// None means the host does not supply a name. This is not saved state.
    fn track_name_changed(&self, _name: Option<&str>) {}
    fn prepare(&self, state: &PluginState) -> Result<Box<dyn Prepared>, &'static str>;
    fn save(&self, state: &mut PluginState);
    /// Called during construction. The callback schedules independent main
    /// work even when the generic task queue is full and defers host reentry
    /// until any enclosing performance callback's runtime borrows have ended.
    fn install_wakeup(&self, wake: Box<dyn Fn() + Send + Sync>);
    /// Main-thread service of registry returns and setup notifications. What
    /// it reports, the wrapper tells the host after this call has returned.
    fn service(&self) -> Serviced;
}

/// What a setup's service changed since the last call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Serviced {
    /// The saved state changed: the host marks the project modified.
    pub modified: bool,
    /// A host parameter's value changed: the host rescans the values.
    pub values: bool,
    /// A host parameter's text changed: the host rescans the text.
    pub text: bool,
}
