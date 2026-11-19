//! One claim over the whole picture: whatever a [`ViewConfig`] holds, the
//! [`Scene`] derived from it is made of numbers a shader can draw.
//!
//! Held here as ONE assertion rather than one per clamp, because that is the
//! shape the defect has. A bare `clamp` is no guard against a NaN — NaN
//! answers no to every comparison a clamp makes, so it passes straight
//! through — and the site that acquires one next is covered by an assertion
//! over the scene rather than by a test somebody remembers to add beside it.
//!
//! Production loads normalize their view. This intentionally bypasses loading
//! to hold scene entry to the same repair when handed damaged values.

use super::harness::*;
use crate::*;
use harmonigraph_core::Tuning;

/// A [`ViewConfig`] with every float NaN, and every other field set to what it
/// takes to reach the picture: a window with nodes in it, a sounding note's
/// marks, and every intensity weight filled, since an empty one holds no
/// number to poison.
///
/// The floats are found in the view's serialized text rather than written
/// out, so one added anywhere in the view, a nested group or a gradient knob
/// included, is poisoned without an edit here; the test below holds that the
/// fixture leaves no real float behind.
fn poisoned_view() -> ViewConfig {
    let mut base = ViewConfig {
        extent_threes: 3,
        extent_fives: 3,
        min_sevens: -1,
        max_sevens: 1,
        center_threes: 0,
        center_fives: 0,
        center_sevens: 0,
        show_cents: true,
        note_names: NoteNames::Played,
        ..ViewConfig::default()
    };
    let intensity = &mut base.intensity;
    for source in [
        &mut intensity.velocity,
        &mut intensity.gain,
        &mut intensity.pressure,
        &mut intensity.timbre,
    ] {
        *source = crate::IntensitySource { opacity: Some(1.0), thickness: Some(1.0) };
    }
    let text = ron::to_string(&base).expect("a view serializes");
    ron::from_str(&nan_floats(&text)).expect("a poisoned view parses")
}

/// `text` with every float literal in it made `NaN`: RON writes a float with a
/// fraction or an exponent, and an integer with neither. A number starts
/// where no name runs into it, so a digit inside a key is left alone.
fn nan_floats(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut at) = (String::with_capacity(text.len()), 0);
    while at < text.len() {
        let named = at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_');
        if named || !(bytes[at].is_ascii_digit() || bytes[at] == b'-') {
            out.push(char::from(bytes[at]));
            at += 1;
            continue;
        }
        let end = text[at + 1..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+')))
            .map_or(text.len(), |n| at + 1 + n);
        let token = &text[at..end];
        let float = token.parse::<f64>().is_ok() && token.contains(['.', 'e', 'E']);
        out.push_str(if float { "NaN" } else { token });
        at = end;
    }
    out
}

/// Every float in a value's `{:#?}` dump that `keep` picks, by its path through
/// the dump: "something is NaN" costs a bisect, "`marker_unit` is NaN" costs a
/// read. Read off the debug output rather than walked by hand, so a field
/// added to the scene, a node, a marker or the view is covered without being
/// named here. It sees a value only where derived pretty `Debug` prints it
/// alone on its line, which glam's vectors, arrays, options and every derived
/// struct here do; a `Range<f32>`, a `Duration` or a hand-written `Debug`
/// would go unseen.
fn floats_where(value: &impl std::fmt::Debug, keep: impl Fn(f64) -> bool) -> Vec<String> {
    // Each open container's name and how many unnamed items it has shown.
    let (mut path, mut found) = (Vec::<(String, usize)>::new(), Vec::new());
    for line in format!("{value:#?}").lines() {
        let line = line.trim();
        if line.starts_with(['}', ']', ')']) {
            path.pop();
            continue;
        }
        let (name, value) = match (line.split_once(": "), path.last_mut()) {
            (Some((name, value)), _) => (name.to_string(), value),
            (None, Some(open)) => {
                open.1 += 1;
                (format!("[{}]", open.1 - 1), line)
            }
            // The type name the dump opens with.
            (None, None) => (String::new(), line),
        };
        let value = value.trim_end_matches(',');
        // An integer prints with neither a fraction nor an exponent.
        let float =
            value.parse::<f64>().ok().filter(|v| !v.is_finite() || value.contains(['.', 'e']));
        if float.is_some_and(&keep) {
            let at: Vec<_> = path.iter().map(|(name, _)| name.as_str()).chain([&*name]).collect();
            found.push(format!("{} = {value}", at.join(".").trim_matches('.')));
        }
        if line.ends_with(['{', '[', '(']) {
            path.push((name, 0));
        }
    }
    found
}

/// Every value in a dump that is not a real number, by its path.
fn broken(value: &impl std::fmt::Debug) -> Vec<String> {
    floats_where(value, |v| !v.is_finite())
}

#[test]
fn a_view_of_nothing_but_nan_still_derives_a_scene_of_real_numbers() {
    // Past the fresh Note fade a NaN one repairs to, so the held note has
    // arrived and the node layers it lights are in the walk.
    const NOW: f64 = 1.0;
    let real = floats_where(&poisoned_view(), f64::is_finite);
    assert!(real.is_empty(), "the poisoned view still holds real floats: {}", real.join(", "));
    let scene = scene_of(&sounding(), &Tuning::default(), &poisoned_view(), NOW);

    // The fixture has to ARRIVE, or the sweep below passes over an empty
    // picture and reads as coverage while measuring nothing.
    assert!(!scene.nodes.is_empty(), "a poisoned view left no nodes to measure");
    assert!(
        scene.nodes.iter().any(|n| n.on_home),
        "a poisoned view left no home position, where the marker field and the ring stack stand",
    );
    let off_sheet = scene
        .nodes
        .iter()
        .find(|n| !n.on_home)
        .expect("the fixture has sevens layers, so the sevens scale below has something to size")
        .scale;
    let mut normalized = poisoned_view();
    normalized.sanitize();
    // The requested window is independent of the view's repaired dials.
    let mut expected = derive_scene(
        &sounding(),
        &Tuning::default(),
        &normalized,
        &poisoned_view().reach(),
        Camera::default(),
        None,
    );
    NodeMotion::default().step(
        &mut expected,
        &sounding(),
        &Tuning::default(),
        &RingFade::default(),
        NOW,
    );
    // The claim, by name, before the comparison below: a NaN would also fail
    // that, unnamed, and an infinity left on both sides would pass it.
    let names = broken(&scene);
    assert!(names.is_empty(), "a NaN view reached the scene at: {}", names.join(", "));
    assert_eq!(scene, expected);
    assert_eq!(off_sheet, normalized.sevens_size);

    // A nonzero arm makes the marker walk reach its geometry and ink too.
    let drawable = ViewConfig { plus_arm: 0.5, ..poisoned_view() };
    let scene = scene_of(&sounding(), &Tuning::default(), &drawable, NOW);
    assert!(!scene.pluses.is_empty(), "a real arm still shipped no marker field");
    let names = broken(&scene);
    assert!(names.is_empty(), "a NaN view reached the marked scene at: {}", names.join(", "));
}

/// An angle a hair below zero is stored as 0, not as 360: a second sanitize
/// must leave the view alone, or the live view and its sanitized copy disagree
/// (`panes/lattice.rs` asserts they agree in debug builds).
#[test]
fn angles_just_below_zero_sanitize_once() {
    let below = -1e-6_f32;
    assert_eq!(below.rem_euclid(360.0), 360.0, "the fixture must reach the rounding");
    let mut view = ViewConfig::default();
    view.atmosphere.material_direction = below;
    view.pitch_gradient.hue_start = below;
    view.sanitize();
    assert_eq!(view.atmosphere.material_direction, 0.0);
    assert_eq!(view.pitch_gradient.hue_start, 0.0);
    let mut again = view.clone();
    again.sanitize();
    assert_eq!(again, view);
    let spectral = SpectralAtmosphere { cloud_direction: below, ..Default::default() }.sanitized();
    assert_eq!(spectral.cloud_direction, 0.0);
}
