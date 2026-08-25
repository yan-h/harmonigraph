//! Print a UI-state blob (`SharedState::save_persist`) with chosen view and
//! camera fields overridden — the offline renderer's `--ui-state` input,
//! synthesized instead of recovered from a DAW project.
//!
//! `read-plugin-state.py` is the usual way to get one of these, but it needs
//! Bitwig, a saved project, and the editor window closed first. That is the
//! right tool for capturing a look Yan actually dialed in; it is the wrong one
//! for rendering a still of a look nobody has dialed in yet — a study frame,
//! a before/after for a settings change, a regression picture.
//!
//! ```text
//! cargo run -p harmonigraph-ui --example ui-state -- \
//!     extent_sevens=1 sevens_size=0.55 cabinet_scale=0.71 > view.ron
//! harmonigraph-offline chord.take --ui-state view.ron --layout lattice -o frame.png
//! ```
//!
//! Unknown keys are an error rather than a shrug: a typo that silently
//! rendered the default view would be indistinguishable from the setting
//! having no effect, which is exactly the question these frames are asked to
//! answer.
//!
//! Every key is a field of [`ViewConfig`] or [`Camera`], reached by splicing
//! `key:value` into that config's own RON text and parsing the result — not
//! a hand-maintained table of the two, which is what let this tool fall 13
//! fields behind `ViewConfig` (issue #317 §6). `value` is RON syntax for the
//! field's type, which is also what the CLI already asked for one field at a
//! time: a bare number, `true`/`false`, a bare enum variant name
//! (`sevens_label=Name`), or a nested `(..)` to set a whole sub-struct like
//! `pitch_gradient` at once. The one behavior change from the table this
//! replaces: `cabinet_angle` now takes radians, matching every other field,
//! rather than degrees converted on the way in.

use harmonigraph_render::wgpu::TextureFormat;
use harmonigraph_scene::{Camera, ViewConfig};
use harmonigraph_ui::SharedState;

fn main() -> Result<(), String> {
    let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
    let mut view_text = ron::to_string(&state.view).map_err(|e| e.to_string())?;
    let mut camera_text = ron::to_string(&state.camera).map_err(|e| e.to_string())?;

    for arg in std::env::args().skip(1) {
        let (key, value) =
            arg.split_once('=').ok_or_else(|| format!("expected key=value, got `{arg}`"))?;
        if let Some(spliced) = splice(&view_text, key, value) {
            ron::from_str::<ViewConfig>(&spliced).map_err(|e| format!("`{key}`: {e}"))?;
            view_text = spliced;
        } else if let Some(spliced) = splice(&camera_text, key, value) {
            ron::from_str::<Camera>(&spliced).map_err(|e| format!("`{key}`: {e}"))?;
            camera_text = spliced;
        } else {
            return Err(format!("unknown key `{key}`"));
        }
    }

    state.view = ron::from_str(&view_text).map_err(|e| e.to_string())?;
    state.camera = ron::from_str(&camera_text).map_err(|e| e.to_string())?;

    println!("{}", state.save_persist());
    Ok(())
}

/// Replace the top-level `key:value` pair named `key` in a serialized RON
/// struct's text with `key:value` verbatim, or `None` if no top-level field
/// is named `key`. Depth-aware, so a nested struct field's own commas (a
/// `pitch_gradient:(...)`) don't fool the split into treating its inside as
/// more top-level pairs.
fn splice(text: &str, key: &str, value: &str) -> Option<String> {
    let inner = text.trim().trim_start_matches('(').trim_end_matches(')');
    let (mut pairs, mut depth, mut start) = (Vec::new(), 0i32, 0usize);
    for (i, c) in inner.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                pairs.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    pairs.push(&inner[start..]);

    let mut found = false;
    let spliced: Vec<String> = pairs
        .into_iter()
        .map(|pair| {
            if pair.split_once(':').is_some_and(|(k, _)| k == key) {
                found = true;
                format!("{key}:{value}")
            } else {
                pair.to_string()
            }
        })
        .collect();
    found.then(|| format!("({})", spliced.join(",")))
}
