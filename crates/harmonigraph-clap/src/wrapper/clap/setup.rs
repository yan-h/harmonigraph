//! Optional plugin-owned saved settings, outside the host parameter list.
//! The wrapper restores them after the parameters, and never calls these
//! methods under its audio/plugin runtime lock.
use crate::params::Table;
use crate::plugin::PluginState;

pub trait Setup: Send + Sync {
    /// Main-thread host metadata, refreshed at init and on track-info changes.
    /// None means the host does not supply a name. This is not saved state.
    fn track_name_changed(&self, _name: Option<&str>) {}
    /// Install what `state` holds for this setup, after the parameters, off
    /// audio. Nothing refuses: a field that does not parse costs that field
    /// alone, which the setup reports itself.
    fn restore(&self, state: &PluginState);
    fn save(&self, state: &mut PluginState);
    /// Called during construction. The callback asks the host for a
    /// main-thread call, deferred past any process callback it runs in.
    fn install_wakeup(&self, wake: Box<dyn Fn() + Send + Sync>);
    /// Main-thread service of registry returns and setup notifications. What
    /// it reports, the wrapper tells the host after this call has returned.
    fn service(&self) -> Serviced;
    /// Main thread: a line about the host the user should see, which the
    /// shell has already written to the log.
    fn notice(&self, _line: &str) {}
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

/// The one restore, the wrapper's and the plugin's state reader's: the
/// parameters, then the setup. Nothing reinitializes the plugin.
pub fn restore(setup: Option<&dyn Setup>, params: &Table, state: &PluginState) {
    params.restore(&state.params);
    if let Some(setup) = setup {
        setup.restore(state);
    }
}
