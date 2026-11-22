//! Read the plugin's settings back out of a saved Bitwig project: for copying
//! a dialled-in look into the defaults, or for debugging a persistence bug
//! against real state rather than a guess. `./read-plugin-state` at the
//! repository root builds and runs this.
//!
//! ```text
//! ./read-plugin-state                     # the newest Bitwig project
//! ./read-plugin-state path/to.bwproject
//! ./read-plugin-state --rust              # the view as an `impl Default` body
//! ./read-plugin-state --appearance project.bwproject > appearance.ron
//! ./read-plugin-state --raw               # what the file holds
//! ```
//!
//! Every output but `--raw` is what the plugin loads, through its own load
//! ([`harmonigraph_plugin::saved_state`]): a value out of range is clamped, a
//! key this build has no field for is gone, a state the plugin refuses is
//! reported as refused, and the camera is the host's. `--raw` prints what the
//! file holds instead, before any decoding a load does, which is what a
//! persistence bug needs. `--rust` and `--appearance` print their artifact
//! alone on stdout, and what the restore says on stderr.
//!
//! Where the bytes are: a `.bwproject` is Bitwig's own tagged binary
//! container, and each plugin instance's state sits in a raw DEFLATE section
//! of it (no zlib header), as the JSON every nice-plug plugin saves:
//! `{"version":..,"params":{..},"fields":{..}}`. Each field is itself JSON,
//! and `ui-state` is a JSON string holding the editor's RON. The scan reads
//! the file as it is and what inflates from every offset, which takes in a
//! zlib-wrapped section's stream too, keeping a stream cut short up to where
//! it stops. A state is ours by its keys (`saved_state::plugin_of`). A
//! 650 KB project takes about 1.4 s.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use harmonigraph_plugin::saved_state::{self, Look, Plugin, Reading};
use serde_json::Value;

/// The opening of every nice-plug state.
const STATE_START: &[u8] = br#"{"version""#;
/// Past this a decompression is garbage rather than a section: Bitwig's are
/// kilobytes.
const INFLATE_LIMIT: u64 = 64 << 20;
/// How deep the editor's settings print one field to a line: the appearance
/// groups' own fields, and the layout's and preferences'.
const LOOK_DEPTH: usize = 3;

/// One state of ours as the file holds it.
struct Instance {
    plugin: Plugin,
    /// The state's JSON, as stored.
    json: Vec<u8>,
    value: Value,
}

#[derive(Clone, Copy, PartialEq)]
enum Output {
    Summary,
    Rust,
    Appearance,
    Raw,
}

fn main() -> ExitCode {
    // The shell's log lines (a parameter the save names and no row has, a
    // field refused) go to stderr, as they go to the log in a plugin.
    harmonigraph_clap::wrapper::setup_logger();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut output = Output::Summary;
    let mut project = None;
    for arg in std::env::args().skip(1) {
        let chosen = match arg.as_str() {
            "--rust" => Output::Rust,
            "--appearance" => Output::Appearance,
            "--raw" => Output::Raw,
            "-h" | "--help" => {
                println!("usage: read-plugin-state [--rust | --appearance | --raw] [PROJECT]");
                return Ok(());
            }
            flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
            path => {
                if project.replace(PathBuf::from(path)).is_some() {
                    return Err("one project at a time".into());
                }
                continue;
            }
        };
        if output != Output::Summary {
            return Err("--rust, --appearance and --raw are one each".into());
        }
        output = chosen;
    }
    let path = match project {
        Some(path) => path,
        None => newest_project()?,
    };
    eprintln!("# {}", path.display());
    let data = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let instances = instances(&data);
    if instances.is_empty() {
        return Err("No Harmonigraph state found in this project.".into());
    }
    if output == Output::Raw {
        raw(&instances);
        return Ok(());
    }
    let readings: Vec<Reading> = instances
        .iter()
        .map(|instance| saved_state::read(instance.plugin, &instance.json))
        .collect();
    match output {
        Output::Summary => summary(&readings),
        Output::Rust => rust(&readings)?,
        Output::Appearance => appearance(&readings)?,
        Output::Raw => unreachable!("printed above"),
    }
    Ok(())
}

/// The most recently modified `.bwproject`, auto-backups included: an
/// autosave is as good a source as a manual save. Spotlight does not index
/// the cloud-synced folder the real projects live in, so that folder is
/// walked as well as asking `mdfind`, which alone found only Bitwig's own
/// month-old temp-project backups.
fn newest_project() -> Result<PathBuf, String> {
    let mut paths: Vec<PathBuf> = std::process::Command::new("mdfind")
        .args(["-name", ".bwproject"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).lines().map(PathBuf::from).collect())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    for drive in
        std::fs::read_dir(home.join("Library/CloudStorage")).into_iter().flatten().flatten()
    {
        if drive.file_name().to_string_lossy().starts_with("GoogleDrive-") {
            walk(&drive.path().join("My Drive/music"), &mut paths);
        }
    }
    paths
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "bwproject"))
        .filter_map(|path| Some((path.metadata().ok()?.modified().ok()?, path)))
        .max()
        .map(|(_, path)| path)
        .ok_or_else(|| "No .bwproject found by mdfind or under Google Drive. Pass one.".into())
}

fn walk(dir: &Path, paths: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            walk(&path, paths);
        } else {
            paths.push(path);
        }
    }
}

/// Every Harmonigraph and Tune state in a project's bytes, once each, in the
/// order found. The whole file, not the first section that holds one: a
/// project's Tunes sit in sections apart from the editor's. A state is taken
/// as JSON alone, so one the plugin would refuse is still found.
fn instances(data: &[u8]) -> Vec<Instance> {
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    for buffer in std::iter::once(data.to_vec()).chain(sections(data)) {
        for start in offsets(&buffer, STATE_START) {
            let mut stream =
                serde_json::Deserializer::from_slice(&buffer[start..]).into_iter::<Value>();
            let Some(Ok(value)) = stream.next() else { continue };
            let Some(plugin) = saved_state::plugin_of(&value) else { continue };
            // By content, so the same state in two sections, or in a section
            // and plain, is one instance.
            if seen.insert(value.to_string()) {
                let json = buffer[start..start + stream.byte_offset()].to_vec();
                found.push(Instance { plugin, json, value });
            }
        }
    }
    found
}

/// What inflates as raw DEFLATE from each offset of `data`, where it holds a
/// state's start. A stream cut short keeps what inflated before the cut.
fn sections(data: &[u8]) -> impl Iterator<Item = Vec<u8>> + '_ {
    (0..data.len()).filter_map(move |offset| {
        let mut out = Vec::new();
        let decoder = flate2::bufread::DeflateDecoder::new(&data[offset..]);
        // An error ends the stream; what came before it stays in `out`.
        let _ = decoder.take(INFLATE_LIMIT).read_to_end(&mut out);
        let holds = out.windows(STATE_START.len()).any(|window| window == STATE_START);
        holds.then_some(out)
    })
}

/// Where `needle` starts in `haystack`.
fn offsets<'a>(haystack: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    haystack.windows(needle.len()).enumerate().filter(move |(_, w)| *w == needle).map(|(i, _)| i)
}

fn installed(reading: &Reading) -> Option<&harmonigraph_ui::SharedState> {
    match &reading.look {
        Some(Look::Installed(editor)) => Some(editor),
        _ => None,
    }
}

fn heading(n: usize, count: usize, plugin: Plugin) {
    let name = match plugin {
        Plugin::Harmonigraph => "Harmonigraph",
        Plugin::Tune => "Harmonigraph Tune",
    };
    println!("=== instance {n} of {count}: {name} ===");
}

fn summary(readings: &[Reading]) {
    for (n, reading) in readings.iter().enumerate() {
        if n > 0 {
            println!();
        }
        heading(n + 1, readings.len(), reading.plugin);
        if let Some(reason) = &reading.refused {
            println!(
                "\nThe plugin refuses this state whole ({reason}) and keeps what it held; \
                 --raw shows what it holds."
            );
            continue;
        }
        if !reading.params.is_empty() {
            println!("\n--- params (host parameters, as restored) ---");
            for (id, value) in &reading.params {
                println!("  {id}: {value}");
            }
        }
        println!("\n--- fields (as the plugin saves them after this restore) ---");
        for (key, value) in &reading.fields {
            println!("  {key}: {value}");
        }
        match &reading.look {
            None => {}
            Some(Look::Missing) => {
                println!(
                    "\n(no ui-state, which every current save writes: an older build saved this)"
                )
            }
            Some(Look::Unopened) => println!(
                "\n(ui-state is empty: the editor was never opened, so it opens at the fresh look \
                 of whichever build loads it)"
            ),
            Some(Look::Refused) => {
                println!("\n(ui-state refused: the editor opens at the fresh look; the Console says why)")
            }
            Some(Look::Installed(editor)) => {
                println!("\n--- ui-state (as the editor installs it, with the host's camera) ---");
                println!("{}", editor.save_persist_pretty(LOOK_DEPTH));
            }
        }
        if !reading.notes.is_empty() {
            println!("\n--- Console (what the restore says) ---");
            for note in &reading.notes {
                println!("  {note}");
            }
        }
    }
}

/// What the restore of instance `n` says, on stderr, so that stdout holds
/// the artifact alone.
fn report(n: usize, reading: &Reading) {
    if let Some(reason) = &reading.refused {
        eprintln!("# instance {n}: the plugin refuses this state whole ({reason})");
    }
    for note in &reading.notes {
        eprintln!("# instance {n}: {note}");
    }
}

/// The installed looks, by instance number, each instance's notes reported.
fn looks(readings: &[Reading]) -> Vec<(usize, &harmonigraph_ui::SharedState)> {
    let numbered = readings.iter().enumerate().map(|(index, reading)| (index + 1, reading));
    numbered
        .filter_map(|(n, reading)| {
            report(n, reading);
            Some((n, installed(reading)?))
        })
        .collect()
}

fn appearance(readings: &[Reading]) -> Result<(), String> {
    let looks = looks(readings);
    let [(_, editor)] = looks[..] else {
        return Err(format!("Expected one editor look, found {}.", looks.len()));
    };
    println!("{}", editor.picture.appearance.serialize());
    Ok(())
}

/// The view's fields as an `impl Default` body for `ViewConfig`. Its values
/// are RON, so an enum or a nested struct needs its Rust path on paste.
fn rust(readings: &[Reading]) -> Result<(), String> {
    let looks = looks(readings);
    if looks.is_empty() {
        return Err("No editor look in any instance.".into());
    }
    let config = ron::ser::PrettyConfig::new().depth_limit(1);
    for (n, editor) in &looks {
        // Labelled by instance where there is more than one look, so two runs
        // of fields cannot be pasted as one.
        let which = if looks.len() > 1 { format!(" instance {n}") } else { String::new() };
        println!("// From a live Bitwig session{which}; see read-plugin-state.");
        let view = ron::ser::to_string_pretty(&editor.picture.appearance.view, config.clone())
            .map_err(|error| error.to_string())?;
        for line in view.lines().filter(|line| line.starts_with(' ')) {
            println!("{line}");
        }
    }
    Ok(())
}

/// What the file holds for each instance, before any decoding a load does:
/// every field that is a JSON string decoded once (`ui-state` to its RON),
/// the rest as stored.
fn raw(instances: &[Instance]) {
    for (n, instance) in instances.iter().enumerate() {
        if n > 0 {
            println!();
        }
        heading(n + 1, instances.len(), instance.plugin);
        let value = &instance.value;
        println!("version: {}", value.get("version").unwrap_or(&Value::Null));
        for (name, title) in [("params", "params"), ("fields", "fields")] {
            println!("\n--- {title} (as saved) ---");
            let Some(map) = value.get(name).and_then(Value::as_object) else {
                println!("  (none: {})", value.get(name).unwrap_or(&Value::Null));
                continue;
            };
            for (key, item) in map {
                let decoded = item.as_str().map(|text| {
                    serde_json::from_str::<String>(text).unwrap_or_else(|_| text.to_owned())
                });
                println!("  {key}: {}", decoded.unwrap_or_else(|| item.to_string()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::{DeflateEncoder, ZlibEncoder};
    use flate2::Compression;
    use std::io::Write;

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!(
            "{}/../harmonigraph-plugin/fixtures/saved-state/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::read(path).unwrap()
    }

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    /// Each state of ours in a project is found once: plain, in a raw
    /// DEFLATE section, in a zlib one, or in one cut short; two distinct
    /// states of one plugin are two; one a load would refuse is found; and
    /// another plugin's state is not, though every nice-plug plugin saves the
    /// same shape.
    #[test]
    fn every_state_of_ours_is_found_once() {
        let zlib = |bytes: &[u8]| {
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(bytes).unwrap();
            encoder.finish().unwrap()
        };
        let other =
            br#"{"version":"1.0.0","params":{"gain":{"f32":0.25}},"fields":{"editor-state":"{}"}}"#;
        let (hub, tune) = (fixture("harmonigraph"), fixture("tune"));
        let plain_tune =
            br#"{"version":"0.6.1","params":{},"fields":{"tuning-instance":"{\"name\":\"Plain\"}"}}"#;
        // Padded, so that the cut lands in the padding and leaves the state
        // whole.
        let cut = {
            let padded = [&fixture("harmonigraph-rejected")[..], &[b' '; 4096]].concat();
            let mut stream = deflate(&padded);
            stream.truncate(stream.len() - 8);
            stream
        };
        let project = [
            &b"BtWg\0"[..],
            plain_tune,
            b"\0\xff",
            &deflate(&hub),
            b"\0\xff",
            &zlib(&tune),
            b"\0\xff",
            &deflate(other),
            b"\0\xff",
            &deflate(&hub),
            b"\0\xff",
            &cut,
            b"\0\xff",
            other,
        ]
        .concat();
        let found = instances(&project);
        let kinds: Vec<Plugin> = found.iter().map(|instance| instance.plugin).collect();
        assert_eq!(kinds, [Plugin::Tune, Plugin::Harmonigraph, Plugin::Tune, Plugin::Harmonigraph]);
        assert!(found[3].value["version"].is_null(), "the state cut short");
    }
}
