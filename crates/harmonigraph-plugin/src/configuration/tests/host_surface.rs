//! What a CLAP host reads of both plugins, pinned while the CLAP shell is
//! rewritten (#1814): descriptors, the extensions answered, ports, latency,
//! tail, and every parameter's info, text both ways and host-value
//! conversion. Saved automation, modulator mappings and devices in a Bitwig
//! project hold onto exactly these, so a change to any of them is a change
//! to what an existing project means, and the snapshot shows it.
//!
//! Everything here goes through the C ABI alone, except the editor's write,
//! which goes through the editor's own parameter backend. The read-back after
//! an editor write is taken after one process callback, which reports it to
//! the host; the table holds the value from the write on (#1814 Q1).
//!
//! The typed-text lines pin what the host's entry box does today, defects
//! included: Lattice Map parses typed text as the index from 0 while its label
//! counts from 1, and Note Retuning and Map Follow accept only their integers
//! (#1814's side findings). A layer that fixes them edits the snapshot.
//!
//! Regenerate the snapshot, after an intended change to the surface, with
//! `HARMONIGRAPH_BLESS=1 cargo test -p harmonigraph-plugin host_surface`.
use super::*;
use clap_sys::ext::audio_ports::{
    clap_audio_port_info, clap_plugin_audio_ports, CLAP_EXT_AUDIO_PORTS,
};
use clap_sys::ext::audio_ports_config::{
    clap_audio_ports_config, clap_plugin_audio_ports_config, CLAP_EXT_AUDIO_PORTS_CONFIG,
};
use clap_sys::ext::latency::{clap_plugin_latency, CLAP_EXT_LATENCY};
use clap_sys::ext::note_ports::{clap_note_port_info, clap_plugin_note_ports, CLAP_EXT_NOTE_PORTS};
use clap_sys::ext::params::*;
use clap_sys::ext::remote_controls::{clap_plugin_remote_controls, CLAP_EXT_REMOTE_CONTROLS};
use clap_sys::ext::render::{clap_plugin_render, CLAP_EXT_RENDER};
use clap_sys::ext::tail::{clap_plugin_tail, CLAP_EXT_TAIL};
use std::fmt::Write as _;

const SNAPSHOT: &str = "src/configuration/tests/host_surface.txt";

/// Compare `actual` with the committed file at `path` (relative to this
/// crate), or overwrite the file when `HARMONIGRAPH_BLESS` is set.
pub(super) fn compare_or_bless(path: &str, actual: &[u8]) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    if std::env::var_os("HARMONIGRAPH_BLESS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    if expected != actual {
        let at = expected.iter().zip(actual).position(|(e, a)| e != a);
        let at = at.unwrap_or(expected.len().min(actual.len()));
        let line = expected[..at].iter().filter(|&&b| b == b'\n').count() + 1;
        let around = |bytes: &[u8]| {
            let start = at.saturating_sub(80);
            String::from_utf8_lossy(&bytes[start..(at + 80).min(bytes.len())]).into_owned()
        };
        panic!(
            "{} differs from this build at line {line}, byte {at}:\n  committed: {:?}\n  this build: {:?}",
            path.display(),
            around(&expected),
            around(actual),
        );
    }
}

fn c_str(text: *const c_char) -> String {
    if text.is_null() {
        return "(null)".into();
    }
    unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned()
}
fn c_name(text: &[c_char]) -> String {
    c_str(text.as_ptr())
}
fn plugin_extension<T>(plugin: *const clap_plugin, id: &CStr) -> Option<&'static T> {
    let found = unsafe { ((*plugin).get_extension.unwrap())(plugin, id.as_ptr()) };
    (!found.is_null()).then(|| unsafe { &*found.cast::<T>() })
}

/// Every extension id this file asks about; the report lists those answered.
const EXTENSIONS: [&CStr; 21] = [
    c"clap.ambisonic/3",
    c"clap.audio-ports",
    c"clap.audio-ports-activation/2",
    c"clap.audio-ports-config",
    c"clap.audio-ports-config-info/1",
    c"clap.configurable-audio-ports/1",
    c"clap.context-menu/1",
    c"clap.gui",
    c"clap.latency",
    c"clap.note-name",
    c"clap.note-ports",
    c"clap.param-indication/4",
    c"clap.params",
    c"clap.preset-load/2",
    c"clap.remote-controls/2",
    c"clap.render",
    c"clap.state",
    c"clap.state-context/2",
    c"clap.tail",
    c"clap.track-info/1",
    c"clap.voice-info",
];

fn describe_plugin(out: &mut String, plugin: *const clap_plugin) {
    let answered: Vec<_> = EXTENSIONS
        .iter()
        .filter(|id| plugin_extension::<c_void>(plugin, id).is_some())
        .map(|id| id.to_string_lossy())
        .collect();
    writeln!(out, "  extensions: {}", answered.join(" ")).unwrap();
    if let Some(configs) =
        plugin_extension::<clap_plugin_audio_ports_config>(plugin, CLAP_EXT_AUDIO_PORTS_CONFIG)
    {
        for index in 0..unsafe { (configs.count.unwrap())(plugin) } {
            let mut c: clap_audio_ports_config = unsafe { std::mem::zeroed() };
            assert!(unsafe { (configs.get.unwrap())(plugin, index, &mut c) });
            writeln!(
                out,
                "  audio-ports-config {}: id {} {:?} in {} out {} main in {} x{} {} main out {} x{} {}",
                index,
                c.id,
                c_name(&c.name),
                c.input_port_count,
                c.output_port_count,
                c.has_main_input,
                c.main_input_channel_count,
                c_str(c.main_input_port_type),
                c.has_main_output,
                c.main_output_channel_count,
                c_str(c.main_output_port_type),
            )
            .unwrap();
        }
    }
    if let Some(ports) = plugin_extension::<clap_plugin_audio_ports>(plugin, CLAP_EXT_AUDIO_PORTS) {
        for input in [true, false] {
            for index in 0..unsafe { (ports.count.unwrap())(plugin, input) } {
                let mut p: clap_audio_port_info = unsafe { std::mem::zeroed() };
                assert!(unsafe { (ports.get.unwrap())(plugin, index, input, &mut p) });
                writeln!(
                    out,
                    "  audio {} {}: id {} {:?} flags {:#x} x{} {} pair {}",
                    if input { "in" } else { "out" },
                    index,
                    p.id,
                    c_name(&p.name),
                    p.flags,
                    p.channel_count,
                    c_str(p.port_type),
                    p.in_place_pair as i64,
                )
                .unwrap();
            }
        }
    }
    if let Some(ports) = plugin_extension::<clap_plugin_note_ports>(plugin, CLAP_EXT_NOTE_PORTS) {
        for input in [true, false] {
            for index in 0..unsafe { (ports.count.unwrap())(plugin, input) } {
                let mut p: clap_note_port_info = unsafe { std::mem::zeroed() };
                assert!(unsafe { (ports.get.unwrap())(plugin, index, input, &mut p) });
                writeln!(
                    out,
                    "  note {} {}: id {} {:?} dialects {:#x} preferred {:#x}",
                    if input { "in" } else { "out" },
                    index,
                    p.id,
                    c_name(&p.name),
                    p.supported_dialects,
                    p.preferred_dialect,
                )
                .unwrap();
            }
        }
    }
    if let Some(render) = plugin_extension::<clap_plugin_render>(plugin, CLAP_EXT_RENDER) {
        let hard = unsafe { (render.has_hard_realtime_requirement.unwrap())(plugin) };
        writeln!(out, "  render: hard realtime {hard}").unwrap();
    }
    if let Some(remote) =
        plugin_extension::<clap_plugin_remote_controls>(plugin, CLAP_EXT_REMOTE_CONTROLS)
    {
        writeln!(out, "  remote-control pages: {}", unsafe { (remote.count.unwrap())(plugin) })
            .unwrap();
    }
    if let Some(gui) = plugin_extension::<clap_plugin_gui>(plugin, CLAP_EXT_GUI) {
        let supported = |api: &CStr, floating| unsafe {
            (gui.is_api_supported.unwrap())(plugin, api.as_ptr(), floating)
        };
        writeln!(
            out,
            "  gui: cocoa embedded {} floating {}; show {} hide {}",
            supported(c"cocoa", false),
            supported(c"cocoa", true),
            unsafe { (gui.show.unwrap())(plugin) },
            unsafe { (gui.hide.unwrap())(plugin) },
        )
        .unwrap();
    }
}

/// Latency and tail as the host reads them after activation at 48 kHz with
/// at most `frames` per block.
fn describe_activated(out: &mut String, plugin: *const clap_plugin, frames: u32) {
    assert!(unsafe { ((*plugin).activate.unwrap())(plugin, 48000.0, 1, frames) });
    let latency = plugin_extension::<clap_plugin_latency>(plugin, CLAP_EXT_LATENCY)
        .map(|l| unsafe { (l.get.unwrap())(plugin) });
    let tail = plugin_extension::<clap_plugin_tail>(plugin, CLAP_EXT_TAIL)
        .map(|t| unsafe { (t.get.unwrap())(plugin) });
    writeln!(out, "  activated at 48000 Hz, {frames} frames: latency {latency:?} tail {tail:?}")
        .unwrap();
    unsafe { ((*plugin).deactivate.unwrap())(plugin) };
}

fn flag_names(flags: u32) -> String {
    let names = [
        (CLAP_PARAM_IS_STEPPED, "stepped"),
        (CLAP_PARAM_IS_PERIODIC, "periodic"),
        (CLAP_PARAM_IS_HIDDEN, "hidden"),
        (CLAP_PARAM_IS_READONLY, "readonly"),
        (CLAP_PARAM_IS_BYPASS, "bypass"),
        (CLAP_PARAM_IS_AUTOMATABLE, "automatable"),
        (CLAP_PARAM_IS_AUTOMATABLE_PER_NOTE_ID, "automatable-per-note"),
        (CLAP_PARAM_IS_AUTOMATABLE_PER_KEY, "automatable-per-key"),
        (CLAP_PARAM_IS_AUTOMATABLE_PER_CHANNEL, "automatable-per-channel"),
        (CLAP_PARAM_IS_AUTOMATABLE_PER_PORT, "automatable-per-port"),
        (CLAP_PARAM_IS_MODULATABLE, "modulatable"),
        (CLAP_PARAM_IS_MODULATABLE_PER_NOTE_ID, "modulatable-per-note"),
        (CLAP_PARAM_IS_MODULATABLE_PER_KEY, "modulatable-per-key"),
        (CLAP_PARAM_IS_MODULATABLE_PER_CHANNEL, "modulatable-per-channel"),
        (CLAP_PARAM_IS_MODULATABLE_PER_PORT, "modulatable-per-port"),
        (CLAP_PARAM_REQUIRES_PROCESS, "requires-process"),
        (CLAP_PARAM_IS_ENUM, "enum"),
    ];
    let known = names.iter().fold(0, |all, (bit, _)| all | bit);
    let mut out: Vec<String> = names
        .iter()
        .filter(|(bit, _)| flags & bit != 0)
        .map(|(_, name)| name.to_string())
        .collect();
    if flags & !known != 0 {
        out.push(format!("{:#x}", flags & !known));
    }
    out.join(",")
}

fn infos(device: &Device) -> Vec<clap_param_info> {
    let params = device.params();
    (0..unsafe { (params.count.unwrap())(device.plugin) })
        .map(|index| {
            let mut info: clap_param_info = unsafe { std::mem::zeroed() };
            assert!(unsafe { (params.get_info.unwrap())(device.plugin, index, &mut info) });
            info
        })
        .collect()
}
fn to_text(device: &Device, id: u32, value: f64) -> Option<String> {
    let mut text = [0 as c_char; 256];
    let shown = unsafe {
        (device.params().value_to_text.unwrap())(device.plugin, id, value, text.as_mut_ptr(), 256)
    };
    shown.then(|| c_name(&text))
}
fn from_text(device: &Device, id: u32, text: &str) -> Option<f64> {
    let text = std::ffi::CString::new(text).unwrap();
    let mut value = f64::NAN;
    let parsed = unsafe {
        (device.params().text_to_value.unwrap())(device.plugin, id, text.as_ptr(), &mut value)
    };
    parsed.then_some(value)
}
fn host_value(device: &Device, id: u32) -> f64 {
    let mut value = f64::NAN;
    assert!(unsafe { (device.params().get_value.unwrap())(device.plugin, id, &mut value) });
    value
}
/// A value event at the host's value `value`, as a host sends for automation.
fn value_event(id: u32, value: f64) -> Input {
    Input::Param(clap_event_param_value {
        header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, 0),
        param_id: id,
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    })
}
fn mod_event(id: u32, amount: f64) -> Input {
    Input::Mod(clap_event_param_mod {
        header: header::<clap_event_param_mod>(CLAP_EVENT_PARAM_MOD, 0),
        param_id: id,
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        amount,
    })
}
/// What the plugin's own save holds for parameter `id`, through the CLAP
/// state extension, with the length prefix checked.
fn saved_value(device: &Device, id: &str) -> serde_json::Value {
    let bytes = raw_save(device.plugin);
    let json: serde_json::Value = serde_json::from_slice(&bytes[8..]).unwrap();
    json["params"][id].clone()
}
/// The bytes the CLAP state extension writes, its 8-byte length prefix
/// checked against what follows it.
pub(super) fn raw_save(plugin: *const clap_plugin) -> Vec<u8> {
    let mut bytes = Vec::<u8>::new();
    let stream = clap_ostream { ctx: (&mut bytes as *mut Vec<u8>).cast(), write: Some(write) };
    let state = plugin_extension::<clap_plugin_state>(plugin, CLAP_EXT_STATE).unwrap();
    assert!(unsafe { (state.save.unwrap())(plugin, &stream) });
    let length = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    assert_eq!(length as usize, bytes.len() - 8, "the length prefix counts what follows it");
    bytes
}
/// Exact `f32` bits beside the decimal, so a last-digit change shows.
fn float_value(value: &serde_json::Value) -> String {
    match value.get("f32").and_then(|v| v.as_f64()) {
        Some(v) => format!("{} ({:#010x})", v as f32, (v as f32).to_bits()),
        None => value.to_string(),
    }
}

/// The host value the editor sends for plain value `plain` of a stepped
/// parameter with `steps` steps from `min`: nice-plug's `f32` normalization
/// times the step count, so a whole value arrives just below itself
/// (Lattice Map 3 as 2.999999988824129).
fn editor_host_value(index: i32, steps: f64) -> f64 {
    f64::from((index as f32) / (steps as f32)) * steps
}

/// A plugin from the exported factory by id, initialized and nothing more.
pub(super) struct Instance {
    pub(super) plugin: *const clap_plugin,
    _host: Box<clap_host>,
    _stats: Box<Host>,
}
impl Instance {
    pub(super) fn new(id: &CStr) -> Self {
        let mut stats = Box::<Host>::default();
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: (&mut *stats as *mut Host).cast(),
            name: c"Host surface fixture".as_ptr(),
            vendor: c"test".as_ptr(),
            url: c"".as_ptr(),
            version: c"1".as_ptr(),
            get_extension: Some(extension),
            request_restart: Some(request),
            request_process: Some(request),
            request_callback: Some(callback),
        });
        let factory =
            unsafe { (crate::clap_entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }
                .cast::<clap_plugin_factory>();
        let plugin = unsafe { ((*factory).create_plugin.unwrap())(factory, &*host, id.as_ptr()) };
        assert!(!plugin.is_null());
        assert!(unsafe { ((*plugin).init.unwrap())(plugin) });
        Self { plugin, _host: host, _stats: stats }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe { ((*self.plugin).destroy.unwrap())(self.plugin) };
    }
}

fn report() -> String {
    let mut out = String::new();
    let factory =
        unsafe { (crate::clap_entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }
            .cast::<clap_plugin_factory>();
    let count = unsafe { ((*factory).get_plugin_count.unwrap())(factory) };
    writeln!(out, "factory: {count} plugins").unwrap();
    for index in 0..count {
        let d = unsafe { &*((*factory).get_plugin_descriptor.unwrap())(factory, index) };
        assert_eq!(c_str(d.version), env!("CARGO_PKG_VERSION"), "the crate version");
        let mut features = Vec::new();
        let mut feature = d.features;
        while !unsafe { *feature }.is_null() {
            features.push(c_str(unsafe { *feature }));
            feature = unsafe { feature.add(1) };
        }
        writeln!(
            out,
            "plugin {}: {:?} vendor {:?} url {:?} manual {:?} support {:?} version (crate) \
             description {:?} features {}",
            c_str(d.id),
            c_str(d.name),
            c_str(d.vendor),
            c_str(d.url),
            c_str(d.manual_url),
            c_str(d.support_url),
            c_str(d.description),
            features.join(","),
        )
        .unwrap();
    }

    let mut device = Device::new();
    writeln!(out, "\ncom.yan-h.harmonigraph").unwrap();
    describe_plugin(&mut out, device.plugin);
    describe_activated(&mut out, device.plugin, 512);
    let tune = Instance::new(c"com.yan-h.harmonigraph-tune");
    writeln!(out, "\ncom.yan-h.harmonigraph-tune").unwrap();
    describe_plugin(&mut out, tune.plugin);
    describe_activated(&mut out, tune.plugin, 512);
    describe_activated(&mut out, tune.plugin, 64);
    writeln!(
        out,
        "  parameters: {}",
        plugin_extension::<clap_plugin_params>(tune.plugin, CLAP_EXT_PARAMS)
            .map_or(0, |p| unsafe { (p.count.unwrap())(tune.plugin) })
    )
    .unwrap();
    drop(tune);

    writeln!(out, "\nparameters of com.yan-h.harmonigraph").unwrap();
    device.activate();
    let mut steady = 0;
    let mut apply = |device: &Device, events: Vec<Input>| {
        device.run(steady, events, false);
        steady += 64;
    };
    for info in infos(&device) {
        let (id, name) = (info.id, c_name(&info.name));
        writeln!(
            out,
            "{name:?}: id {id} module {:?} range {}..{} default {} flags {}",
            c_name(&info.module),
            info.min_value,
            info.max_value,
            info.default_value,
            flag_names(info.flags),
        )
        .unwrap();
        let string_id = PARAM_IDS
            .iter()
            .find(|s| nice_plug::wrapper::hash_param_id(s) == id)
            .expect("a known id");
        let stepped = info.flags & CLAP_PARAM_IS_STEPPED != 0;
        let probes: Vec<f64> = if stepped {
            // Every whole value as the editor sends it, and a value either
            // side of a whole one, for the rounding.
            let steps = info.max_value;
            let mut probes: Vec<f64> = [0, 1, 2, 3, steps as i32]
                .into_iter()
                .filter(|&n| f64::from(n) <= steps)
                .map(|n| editor_host_value(n, steps))
                .collect();
            if steps >= 2.0 {
                probes.extend([1.4, 1.6]);
            }
            probes.push(steps);
            probes.dedup();
            probes
        } else {
            vec![0.0, 0.125, 1.0 / 3.0, 0.5, 0.123456789, 1.0]
        };
        for value in probes {
            apply(&device, vec![value_event(id, value)]);
            let text = to_text(&device, id, value);
            writeln!(
                out,
                "  host {value:?}: text {text:?} applied {} read back {:?}",
                float_value(&saved_value(&device, string_id)),
                host_value(&device, id),
            )
            .unwrap();
        }
        let shown = to_text(&device, id, info.default_value).unwrap_or_default();
        let typed = [shown.as_str(), "0", "1", "3", "10", "-1", "200", "0.5", " 2 ", "Off", "Main"];
        let parsed: Vec<String> = typed
            .iter()
            .map(|text| format!("{text:?} -> {:?}", from_text(&device, id, text)))
            .collect();
        writeln!(out, "  typed: {}", parsed.join(", ")).unwrap();
        apply(&device, vec![value_event(id, info.default_value)]);
    }

    writeln!(out, "\nmodulation").unwrap();
    let yaw = device.id(ParamKey::CameraYaw);
    apply(&device, vec![value_event(yaw, 0.5), mod_event(yaw, 0.1)]);
    writeln!(
        out,
        "Camera Yaw at 0.5 modulated by 0.1: read back {:?} saved {}",
        host_value(&device, yaw),
        float_value(&saved_value(&device, ParamKey::CameraYaw.id())),
    )
    .unwrap();
    apply(&device, vec![mod_event(yaw, 0.0)]);
    let map = device.id_named("Lattice Map");
    apply(&device, vec![value_event(map, 2.0), mod_event(map, 1.0)]);
    writeln!(
        out,
        "Lattice Map at 2 modulated by 1: read back {:?} saved {}",
        host_value(&device, map),
        saved_value(&device, "lattice-map"),
    )
    .unwrap();

    writeln!(out, "\neditor writes, read after one process callback").unwrap();
    {
        use harmonigraph_ui::params::ParamBackend;
        let (context, _) = device.wrapper().test_gui_context("camera-yaw");
        let params = device.wrapper().test_inspect_plugin(|plugin| plugin.params.clone());
        let gestures = std::cell::Cell::default();
        let backend = crate::PluginParamBackend {
            params: &params,
            host: context.as_ref(),
            configuration: params.settings.view(),
            gestures: &gestures,
        };
        for (key, plain) in [(ParamKey::CameraYaw, 0.3), (ParamKey::CameraDistance, 7.123_456_7)] {
            backend.begin_set(key);
            backend.set(key, plain);
            backend.end_set(key);
            apply(&device, vec![]);
            writeln!(
                out,
                "{key:?} set to {plain}: read back {:?} saved {}",
                host_value(&device, device.id(key)),
                float_value(&saved_value(&device, key.id())),
            )
            .unwrap();
        }
        drop(context);
    }
    out
}

/// The string ids of Harmonigraph's host parameters, which the report needs
/// to find each parameter's value in a save.
const PARAM_IDS: [&str; 15] = [
    "lattice-map",
    "map-fifths",
    "map-thirds",
    "map-sevenths",
    "map-fifths-extension",
    "map-thirds-extension",
    "map-sevenths-extension",
    "tuning-engine",
    "map-follow",
    "analysis-input",
    "camera-yaw",
    "camera-pitch",
    "camera-distance",
    "camera-pan-x",
    "camera-pan-y",
];

#[test]
fn the_host_surface_matches_its_snapshot() {
    let _scope = crate::test_scope::enter();
    compare_or_bless(SNAPSHOT, report().as_bytes());
}

/// A modulated camera parameter is recorded in a take at its modulated value
/// and saved at its unmodulated one: Bitwig's modulators move the picture an
/// export draws without moving what the project stores.
#[test]
fn a_modulated_camera_is_recorded_modulated_and_saved_unmodulated() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let yaw = device.id(ParamKey::CameraYaw);
    device.run(0, vec![value_event(yaw, 0.5), mod_event(yaw, 0.1)], false);
    let shared = device.wrapper().test_inspect_plugin(|plugin| plugin.editor_shared.clone());
    let dir = std::env::temp_dir().join(format!("harmonigraph-modulated-{}", std::process::id()));
    let probe = {
        let mut shared = shared.lock();
        let probe = harmonigraph_record::testing::worker_probe(&shared.take, dir.clone());
        harmonigraph_record::testing::set_renderer_program(&shared.take, dir.join("absent"));
        shared.take.start(48_000.0, shared.ui.picture.appearance.serialize());
        shared.poll_take_end();
        probe
    };
    device.run_transport(64, vec![], false, None, Some(transport(4.0, 0)));
    device.run_transport(128, vec![], false, None, Some(transport(4.0 + 64.0 / 48_000.0, 0)));
    stop(&shared);
    device.finish_notes(192, &[]);
    assert_eq!(saved_value(&device, "camera-yaw"), serde_json::json!({"f32": 0.0}));
    drop(shared);
    drop(device);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !probe.finished() && !probe.failed() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(probe.finished() && !probe.failed());
    let path = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|e| e == "take"))
        .unwrap();
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let record = take.params.iter().find(|p| p.id == ParamKey::CameraYaw.id()).unwrap();
    let modulated = 0.6_f32 * std::f32::consts::TAU - std::f32::consts::PI;
    assert!((record.value - modulated).abs() < 1e-5, "recorded {}", record.value);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A host asking for the parameter one past the last, the Tune's first
/// included, is answered no rather than read out of bounds.
#[test]
fn a_parameter_index_past_the_last_is_refused() {
    let _scope = crate::test_scope::enter();
    for id in [c"com.yan-h.harmonigraph", c"com.yan-h.harmonigraph-tune"] {
        let instance = Instance::new(id);
        let params = unsafe {
            &*(((*instance.plugin).get_extension.unwrap())(
                instance.plugin,
                CLAP_EXT_PARAMS.as_ptr(),
            )
            .cast::<clap_plugin_params>())
        };
        let count = unsafe { (params.count.unwrap())(instance.plugin) };
        let mut info: clap_param_info = unsafe { std::mem::zeroed() };
        assert!(
            !unsafe { (params.get_info.unwrap())(instance.plugin, count, &mut info) },
            "{id:?}"
        );
    }
}
