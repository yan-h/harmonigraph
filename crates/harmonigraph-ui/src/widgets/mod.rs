//! Custom controls. `ValueBar` is the workhorse: a flat, DAW-style
//! parameter bar (drag anywhere to set, double-click to type a value)
//! that replaces egui's rail-and-knob `Slider` + separate `DragValue`.
//! `RangeBar` is its two-handle sibling, for a pair of values that bound a
//! span rather than one value on a scale. `OctaveStrip` is the octave wheel's
//! own — two counts and the profile they produce, in one row.
//!
//! One module per control, over a shared spine in [`bar`]: the lengths every
//! bar is built out of, the reach a handle answers to, and the two readings
//! more than one of them needs. [`mesh`] sits under the two that draw a band of
//! color, which is the one thing a `rect_filled` cannot do. Everything a pane
//! names is re-exported here, so a caller says `widgets::ValueBar` and never
//! which file it is in.

mod bar;
mod gradient;
mod mesh;
mod octave;
#[cfg(test)]
mod probe;
mod range;
mod rows;
mod stack;
mod value;

pub use gradient::{GradientPreview, SpectrumBar, SpreadBar};
pub use octave::OctaveStrip;
pub use range::RangeBar;
pub use rows::{button_row, choice_row, option_label, record_button, row_field, toggle_switch};
pub use stack::StackBar;
pub use value::{progress_bar, ValueBar};

// The rest of the crate's own surface, and all of it is reached for from the
// tests two directories up: two lengths the settings sweeps measure a bar
// against, and three readers they pick a preview out of a frame with.
// Re-exported rather than reached for by module path, so which file a widget
// lives in stays this module's business.
#[cfg(test)]
pub(crate) use gradient::{preview_height, spectrum_track_width};
#[cfg(test)]
pub(crate) use mesh::{band_bounds, band_columns};
#[cfg(test)]
pub(crate) use value::curve_paths;
