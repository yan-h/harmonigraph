//! The saved state's encoding: a [`PluginState`] as JSON.

use crate::plugin::PluginState;
use anyhow::{Context, Result};

/// Encode a state as the JSON a save stores.
pub(crate) fn serialize_state_json(plugin_state: &PluginState) -> Result<Vec<u8>> {
    serde_json::to_vec(plugin_state).context("Could not format as JSON")
}

/// Decode a saved state, or `None` with a log line when it is not one.
pub(crate) fn deserialize_json(state: &[u8]) -> Option<PluginState> {
    decode(state)
        .inspect_err(|err| crate::shell_error!("Error while deserializing state: {err}"))
        .ok()
}

/// Decode a saved state as a load does: a state that does not decode refuses
/// the load whole, while a parameter value that does not decode costs that
/// value alone ([`ParamValue::Unread`][crate::plugin::ParamValue::Unread]).
pub fn decode(state: &[u8]) -> Result<PluginState, serde_json::Error> {
    serde_json::from_slice(state)
}
