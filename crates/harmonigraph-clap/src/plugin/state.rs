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
}

/// A plugin's state so it can be restored at a later point. This object can be serialized and
/// deserialized using serde.
///
/// The fields are stored as `BTreeMap`s so the order in the serialized file is consistent.
///
/// A parameter value this build cannot read, such as one under a variant it no longer has,
/// costs that value alone: decoding leaves it out of `params` and names it in `unread`, so a
/// restore opens its row at the default and says so, and the rest of the state loads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(from = "SavedState")]
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
    /// The parameter ids whose saved value did not decode, which `params` leaves out. Never
    /// saved.
    #[serde(skip)]
    pub unread: Vec<String>,
}

/// A [`PluginState`] as it is read, each parameter value apart.
#[derive(Deserialize)]
struct SavedState {
    #[serde(default)]
    version: String,
    params: BTreeMap<String, SavedValue>,
    fields: BTreeMap<String, String>,
}

/// One saved parameter value: one this build reads, or one it does not.
#[derive(Deserialize)]
#[serde(untagged)]
enum SavedValue {
    Read(ParamValue),
    Unread(serde::de::IgnoredAny),
}

impl From<SavedState> for PluginState {
    fn from(saved: SavedState) -> Self {
        let mut state =
            PluginState { version: saved.version, fields: saved.fields, ..Default::default() };
        for (id, value) in saved.params {
            match value {
                SavedValue::Read(value) => {
                    state.params.insert(id, value);
                }
                SavedValue::Unread(_) => state.unread.push(id),
            }
        }
        state
    }
}
