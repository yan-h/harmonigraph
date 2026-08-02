//! The bridge between UI widgets and "wherever parameters actually live".
//!
//! In the plugin, automatable parameters live in nice-plug's parameter system
//! and must be changed through a `ParamSetter` so the host sees the
//! automation. In the standalone harness they're plain values. Panes only
//! talk to this trait, so every pane works in both shells.
//!
//! [`ParamKey`] itself — which parameter, and the id it is known by outside
//! the program — lives in `harmonigraph-take`, because a take records
//! parameter changes by that id and so the identity is part of the take format
//! rather than of any one shell. It is re-exported here so a pane sees the key
//! and the seam that edits it together, which is how every call site already
//! reads.

pub use harmonigraph_take::params::{MAX_TUNING_OFFSET, ParamKey};


/// Read/write access to the automatable parameters.
///
/// Continuous edits (drags) should be bracketed with `begin_set`/`end_set`
/// so hosts record them as one automation gesture. `set` alone is fine for
/// one-shot changes (preset buttons, typed values) — backends wrap those in
/// an implicit gesture.
pub trait ParamBackend {
    fn get(&self, key: ParamKey) -> f32;
    fn set(&self, key: ParamKey, value: f32);
    /// Start of a continuous edit (e.g. drag). Default: no-op.
    fn begin_set(&self, key: ParamKey) {
        let _ = key;
    }
    /// End of a continuous edit. Default: no-op.
    fn end_set(&self, key: ParamKey) {
        let _ = key;
    }
}

/// Assemble a [`harmonigraph_core::Tuning`] from the current parameter values.
/// The f32 cent params are quantized to microcents here, once, at the
/// param boundary; all lattice pitch math downstream is exact integers.
pub fn tuning_from_params(params: &dyn ParamBackend) -> harmonigraph_core::Tuning {
    harmonigraph_core::Tuning::from_cents(
        params.get(ParamKey::COffset),
        params.get(ParamKey::Three),
        params.get(ParamKey::Five),
        params.get(ParamKey::Seven),
        params.get(ParamKey::Tolerance),
    )
}
