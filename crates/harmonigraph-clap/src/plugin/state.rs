//! A [`crate::plugin::Plugin`]'s saved state.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A plain, unnormalized value for a parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamValue {
    F32(f32),
    I32(i32),
    Bool(bool),
    /// An enumeration row's value, by its variant's stable id.
    String(String),
}

/// A plugin's state so it can be restored at a later point. This object can be serialized and
/// deserialized using serde.
///
/// The fields are stored as `BTreeMap`s so the order in the serialized file is consistent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginState {
    /// The plugin version this state was saved with. Right now this is not used, but later versions
    /// of nice-plug may allow you to modify the plugin state object directly before it is loaded to
    /// allow migrating plugin states between breaking parameter changes.
    ///
    /// # Notes
    ///
    /// If the saved state is very old, then this field may be empty.
    #[serde(default)]
    pub version: String,

    /// The plugin's parameter values. These are stored unnormalized. This means the old values will
    /// be recalled when when the parameter's range gets increased. Doing so may still mess with
    /// parameter automation though, depending on how the host implements that.
    pub params: BTreeMap<String, ParamValue>,
    /// Saved settings beside the parameters, each written and read by the plugin's
    /// [`Setup`][crate::wrapper::clap::setup::Setup].
    ///
    /// The individual fields are also serialized as JSON so they can safely be restored
    /// independently of the other fields.
    pub fields: BTreeMap<String, String>,
}
