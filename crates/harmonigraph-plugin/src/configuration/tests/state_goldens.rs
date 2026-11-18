//! Golden saves for the CLAP-shell rework (#1814): each saved state in
//! `goldens/` is loaded through the CLAP state extension into a fresh
//! instance, and what that instance then saves must match, byte for byte,
//! what the layers before the rework saved for it.
//!
//! The inputs are the eight distinct Harmonigraph and Tune states found in
//! real projects (the current shape, four saved before #1756 and three Tunes
//! carrying the retired `tuning_delay`), plus one of each plugin with every
//! parameter and field off its default, so a restore that restores nothing
//! fails.
//!
//! These are not lasting tests. A save re-serializes the editor's look at
//! today's defaults, which move often, so they hold only while the rework
//! runs, and its last layer replaces them with a test of the bytes the shell
//! owns. Until then, regenerate them only from the commit that added them,
//! rebased onto the new `main`, with
//! `HARMONIGRAPH_BLESS=1 cargo test -p harmonigraph-plugin state_goldens`;
//! a later step that needs them regenerated has changed what a save writes.
use super::host_surface::{compare_or_bless, raw_save};
use super::*;

const GOLDENS: &str = "src/configuration/tests/goldens";

/// Load `input`, framed as the CLAP state extension expects, into `plugin`.
fn raw_load(plugin: *const clap_plugin, input: &[u8]) -> bool {
    let mut bytes = (input.len() as u64).to_le_bytes().to_vec();
    bytes.extend(input);
    let mut reader = Reader { bytes: &bytes, offset: 0 };
    let stream = clap_istream { ctx: (&mut reader as *mut Reader).cast(), read: Some(read) };
    let state = unsafe {
        &*(((*plugin).get_extension.unwrap())(plugin, CLAP_EXT_STATE.as_ptr())
            .cast::<clap_plugin_state>())
    };
    unsafe { (state.load.unwrap())(plugin, &stream) }
}

/// A save with its `version` replaced, since that is the crate version and
/// moves with every release.
fn without_version(save: &[u8]) -> Vec<u8> {
    let json = &save[8..];
    let head = br#"{"version":""#;
    assert!(json.starts_with(head), "a save begins with its version");
    let end = head.len() + json[head.len()..].iter().position(|&b| b == b'"').unwrap();
    [&head[..], b"(crate)", &json[end..]].concat()
}

/// The JSON a fresh `id` saves after loading `name`'s input.
fn resaved(id: &CStr, name: &str) -> Vec<u8> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDENS);
    let input = std::fs::read(dir.join(format!("{name}.in.json"))).unwrap();
    let instance = super::host_surface::Instance::new(id);
    assert!(raw_load(instance.plugin, &input), "{name} loads");
    without_version(&raw_save(instance.plugin))
}

fn fresh(id: &CStr) -> serde_json::Value {
    let instance = super::host_surface::Instance::new(id);
    serde_json::from_slice(&raw_save(instance.plugin)[8..]).unwrap()
}

const CASES: [(&CStr, &str); 10] = [
    (c"com.yan-h.harmonigraph", "harmonigraph-current"),
    (c"com.yan-h.harmonigraph", "harmonigraph-pre1756-a"),
    (c"com.yan-h.harmonigraph", "harmonigraph-pre1756-b"),
    (c"com.yan-h.harmonigraph", "harmonigraph-pre1756-c"),
    (c"com.yan-h.harmonigraph", "harmonigraph-pre1756-d"),
    (c"com.yan-h.harmonigraph", "harmonigraph-off-default"),
    (c"com.yan-h.harmonigraph-tune", "tune-a"),
    (c"com.yan-h.harmonigraph-tune", "tune-b"),
    (c"com.yan-h.harmonigraph-tune", "tune-c"),
    (c"com.yan-h.harmonigraph-tune", "tune-off-default"),
];

#[test]
fn saved_states_resave_as_the_layers_before_the_rework_saved_them() {
    let _scope = crate::test_scope::enter();
    for (id, name) in CASES {
        compare_or_bless(&format!("{GOLDENS}/{name}.saved.json"), &resaved(id, name));
    }
}

/// The off-default inputs reach every value a save writes: each parameter and
/// field differs from what a fresh instance saves, so these goldens fail on a
/// restore that restores nothing, whatever the committed files hold.
#[test]
fn the_off_default_states_differ_from_a_fresh_save_in_every_value() {
    let _scope = crate::test_scope::enter();
    for (id, name) in [
        (c"com.yan-h.harmonigraph", "harmonigraph-off-default"),
        (c"com.yan-h.harmonigraph-tune", "tune-off-default"),
    ] {
        let fresh = fresh(id);
        let restored: serde_json::Value = serde_json::from_slice(&resaved(id, name)).unwrap();
        for section in ["params", "fields"] {
            let keys = fresh[section].as_object().unwrap().keys();
            assert_eq!(
                keys.clone().collect::<Vec<_>>(),
                restored[section].as_object().unwrap().keys().collect::<Vec<_>>(),
                "{name}: the same {section}",
            );
            for key in keys {
                assert_ne!(fresh[section][key], restored[section][key], "{name}: {section}.{key}");
            }
        }
    }
}
