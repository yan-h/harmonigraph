//! Disposable host experiment for #1051. No nice-plug translation: Offset is
//! signed CLAP plain steps. Range changes are published only while deactivated.
use clap_sys::{
    entry::*,
    events::*,
    ext::{audio_ports::*, params::*, state::*},
    factory::plugin_factory::*,
    host::*,
    plugin::*,
    process::*,
    stream::*,
    version::*,
};
use std::{
    cell::UnsafeCell,
    ffi::{c_char, c_void, CStr},
    fs::File,
    io::Write,
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::SeqCst},
        Mutex,
    },
};

const OFFSET: u32 = 0;
const SPAN: u32 = 1;
const LIMIT: u32 = 4096;
static SERIAL: AtomicU32 = AtomicU32::new(0);
struct Features([*const c_char; 3]);
unsafe impl Sync for Features {}
static FEATURES: Features = Features([c"audio-effect".as_ptr(), c"utility".as_ptr(), ptr::null()]);
static DESCRIPTOR: clap_plugin_descriptor = clap_plugin_descriptor {
    clap_version: CLAP_VERSION,
    id: c"org.harmonigraph.offset-range-probe".as_ptr(),
    name: c"Harmonigraph Offset Range Probe".as_ptr(),
    vendor: c"Harmonigraph Range Experiment".as_ptr(),
    url: c"https://github.com/yan-h/harmonigraph/issues/1051".as_ptr(),
    manual_url: c"".as_ptr(),
    support_url: c"".as_ptr(),
    version: c"0.1.0".as_ptr(),
    description: c"Signed offset automation across an expanding range; silent test only".as_ptr(),
    features: FEATURES.0.as_ptr(),
};

#[derive(Clone, Copy, Debug)]
struct Event {
    sample: i64,
    time: u32,
    kind: u16,
    value: f64,
    span: u32,
}
struct Trace {
    file: File,
    consumer: rtrb::Consumer<Event>,
}
struct Probe {
    plugin: clap_plugin,
    host: *const clap_host,
    active: AtomicBool,
    restarting: AtomicBool,
    span: AtomicU32,
    requested: AtomicU32,
    value: AtomicU64,
    restore: Mutex<Option<(u32, f64)>>,
    // CLAP serializes process and flush. Their producer is never accessed by
    // the main-thread trace drain, which owns the other end of this SPSC queue.
    events: UnsafeCell<rtrb::Producer<Event>>,
    dropped: AtomicU32,
    trace: Mutex<Trace>,
}
unsafe fn probe<'a>(plugin: *const clap_plugin) -> &'a Probe {
    &*((*plugin).plugin_data.cast())
}
impl Probe {
    fn log(&self, message: impl std::fmt::Display) {
        let mut trace = self.trace.lock().unwrap();
        let _ = writeln!(trace.file, "{message}");
        let _ = trace.file.flush();
    }
    unsafe fn host_params(&self) -> Option<&clap_host_params> {
        let get = (*self.host).get_extension?;
        get(self.host, CLAP_EXT_PARAMS.as_ptr()).cast::<clap_host_params>().as_ref()
    }
    unsafe fn apply_span(&self) {
        assert!(!self.active.load(SeqCst));
        let restored = self.restore.lock().unwrap().take();
        if let Some((span, value)) = restored {
            self.requested.store(span, SeqCst);
            self.value.store(value.to_bits(), SeqCst);
        }
        let next = self.requested.load(SeqCst);
        let old = self.span.swap(next, SeqCst);
        self.restarting.store(false, SeqCst);
        if old != next {
            self.log(format_args!(
                "range {old} -> {next}; offset={}",
                f64::from_bits(self.value.load(SeqCst))
            ));
            if let Some(rescan) = self.host_params().and_then(|p| p.rescan) {
                rescan(self.host, CLAP_PARAM_RESCAN_ALL);
                self.log("rescan ALL (deactivated)");
            }
        } else if restored.is_some() {
            if let Some(rescan) = self.host_params().and_then(|p| p.rescan) {
                rescan(self.host, CLAP_PARAM_RESCAN_VALUES);
            }
        }
    }
    unsafe fn receive(&self, input: *const clap_input_events, sample: i64) {
        if input.is_null() {
            return;
        }
        for i in 0..((*input).size.unwrap())(input) {
            let h = ((*input).get.unwrap())(input, i);
            if h.is_null() || (*h).space_id != CLAP_CORE_EVENT_SPACE_ID {
                continue;
            }
            let (id, value) = match (*h).type_ {
                CLAP_EVENT_PARAM_VALUE
                    if (*h).size as usize >= size_of::<clap_event_param_value>() =>
                {
                    let e = &*h.cast::<clap_event_param_value>();
                    (e.param_id, e.value)
                }
                CLAP_EVENT_PARAM_MOD if (*h).size as usize >= size_of::<clap_event_param_mod>() => {
                    let e = &*h.cast::<clap_event_param_mod>();
                    (e.param_id, e.amount)
                }
                _ => continue,
            };
            if !value.is_finite() {
                continue;
            }
            if id == OFFSET {
                if (*h).type_ == CLAP_EVENT_PARAM_VALUE {
                    let span = self.span.load(SeqCst) as f64;
                    self.value.store(value.trunc().clamp(-span, span).to_bits(), SeqCst);
                }
                let event = Event {
                    sample,
                    time: (*h).time,
                    kind: (*h).type_,
                    value,
                    span: self.span.load(SeqCst),
                };
                if (&mut *self.events.get()).push(event).is_err() {
                    self.dropped.fetch_add(1, SeqCst);
                }
            } else if id == SPAN && (*h).type_ == CLAP_EVENT_PARAM_VALUE {
                // Expansion only; use a fresh instance for another baseline.
                self.requested.fetch_max((value as u32).clamp(5, LIMIT), SeqCst);
            }
        }
        if let Some(callback) = (*self.host).request_callback {
            callback(self.host);
        }
    }
}
unsafe extern "C" fn init(plugin: *const clap_plugin) -> bool {
    let p = probe(plugin);
    p.log(format_args!(
        "host={} version={}",
        CStr::from_ptr((*p.host).name).to_string_lossy(),
        CStr::from_ptr((*p.host).version).to_string_lossy()
    ));
    p.host_params().and_then(|p| p.rescan).is_some()
}
unsafe extern "C" fn destroy(plugin: *const clap_plugin) {
    drop(Box::from_raw((*plugin).plugin_data.cast::<Probe>()));
}
unsafe extern "C" fn activate(plugin: *const clap_plugin, rate: f64, _: u32, _: u32) -> bool {
    let p = probe(plugin);
    // activate is called while the host still considers us INACTIVE. Rescanning
    // from deactivate would be too early: that callback itself runs active.
    p.apply_span();
    p.active.store(true, SeqCst);
    p.log(format_args!("activate span={} rate={rate}", p.span.load(SeqCst)));
    true
}
unsafe extern "C" fn deactivate(plugin: *const clap_plugin) {
    let p = probe(plugin);
    p.active.store(false, SeqCst);
    p.log("deactivate");
    if let Some(callback) = (*p.host).request_callback {
        callback(p.host);
    }
}
unsafe extern "C" fn start(_: *const clap_plugin) -> bool {
    true
}
unsafe extern "C" fn stop(_: *const clap_plugin) {}
unsafe extern "C" fn process(
    plugin: *const clap_plugin,
    process: *const clap_process,
) -> clap_process_status {
    let p = probe(plugin);
    let process = &*process;
    p.receive(process.in_events, process.steady_time);
    // Intentionally silent: no oscillators or DC test signal can reach speakers.
    for port in 0..process.audio_outputs_count as usize {
        let output = &mut *process.audio_outputs.add(port);
        if !output.data32.is_null() {
            for ch in 0..output.channel_count as usize {
                ptr::write_bytes(*output.data32.add(ch), 0, process.frames_count as usize);
            }
        }
        output.constant_mask = u64::MAX;
    }
    CLAP_PROCESS_CONTINUE
}
unsafe extern "C" fn main_thread(plugin: *const clap_plugin) {
    let p = probe(plugin);
    {
        let mut trace = p.trace.lock().unwrap();
        while let Ok(e) = trace.consumer.pop() {
            let _ = writeln!(
                trace.file,
                "event sample={} time={} kind={} value={} span={}",
                e.sample, e.time, e.kind, e.value, e.span
            );
        }
        let dropped = p.dropped.swap(0, SeqCst);
        if dropped != 0 {
            let _ = writeln!(trace.file, "INVALID: dropped {dropped} trace events");
        }
        let _ = trace.file.flush();
    }
    if p.requested.load(SeqCst) != p.span.load(SeqCst) || p.restore.lock().unwrap().is_some() {
        if p.active.load(SeqCst) {
            if !p.restarting.swap(true, SeqCst) {
                p.log("request_restart");
                if let Some(restart) = (*p.host).request_restart {
                    restart(p.host);
                }
            }
        } else {
            p.apply_span();
        }
    }
}

unsafe fn text(buffer: *mut c_char, capacity: usize, value: &str) -> bool {
    if capacity == 0 {
        return false;
    }
    let len = value.len().min(capacity - 1);
    ptr::copy_nonoverlapping(value.as_ptr(), buffer.cast(), len);
    *buffer.add(len) = 0;
    true
}
unsafe extern "C" fn count(_: *const clap_plugin) -> u32 {
    2
}
unsafe extern "C" fn info(plugin: *const clap_plugin, id: u32, info: *mut clap_param_info) -> bool {
    let p = probe(plugin);
    let span = p.span.load(SeqCst) as f64;
    let (name, min, max, default, flags) = match id {
        OFFSET => (
            "Offset (steps)",
            -span,
            span,
            0.,
            CLAP_PARAM_IS_AUTOMATABLE | CLAP_PARAM_IS_MODULATABLE,
        ),
        // Bitwig's generic panel omits non-automatable parameters. This is a
        // probe control only: edit it manually, never draw automation for it.
        SPAN => ("Expand range to +/-", 5., LIMIT as f64, 5., CLAP_PARAM_IS_AUTOMATABLE),
        _ => return false,
    };
    *info = clap_param_info {
        id,
        flags: flags | CLAP_PARAM_IS_STEPPED,
        cookie: ptr::null_mut(),
        name: [0; 256],
        module: [0; 1024],
        min_value: min,
        max_value: max,
        default_value: default,
    };
    text((*info).name.as_mut_ptr(), 256, name);
    p.log(format_args!("get_info id={id} min={min} max={max} active={}", p.active.load(SeqCst)));
    true
}
unsafe extern "C" fn value(plugin: *const clap_plugin, id: u32, output: *mut f64) -> bool {
    let p = probe(plugin);
    *output = match id {
        OFFSET => f64::from_bits(p.value.load(SeqCst)),
        SPAN => p.span.load(SeqCst) as f64,
        _ => return false,
    };
    true
}
unsafe extern "C" fn format(
    _: *const clap_plugin,
    id: u32,
    value: f64,
    output: *mut c_char,
    size: u32,
) -> bool {
    if id > SPAN {
        return false;
    }
    text(output, size as usize, &format!("{:.0}", value.trunc()))
}
unsafe extern "C" fn parse(
    _: *const clap_plugin,
    id: u32,
    input: *const c_char,
    output: *mut f64,
) -> bool {
    if id > SPAN {
        return false;
    }
    match CStr::from_ptr(input).to_string_lossy().trim().parse::<f64>() {
        Ok(v) if v.is_finite() => {
            *output = v;
            true
        }
        _ => false,
    }
}
unsafe extern "C" fn flush(
    plugin: *const clap_plugin,
    input: *const clap_input_events,
    _: *const clap_output_events,
) {
    probe(plugin).receive(input, -1);
}
static PARAMS: clap_plugin_params = clap_plugin_params {
    count: Some(count),
    get_info: Some(info),
    get_value: Some(value),
    value_to_text: Some(format),
    text_to_value: Some(parse),
    flush: Some(flush),
};

unsafe extern "C" fn save(plugin: *const clap_plugin, stream: *const clap_ostream) -> bool {
    let p = probe(plugin);
    let (span, value) = p
        .restore
        .lock()
        .unwrap()
        .unwrap_or((p.requested.load(SeqCst), f64::from_bits(p.value.load(SeqCst))));
    let bytes = [b"RNG1".as_slice(), &span.to_le_bytes(), &value.to_bits().to_le_bytes()].concat();
    let mut pos = 0;
    while pos < bytes.len() {
        let n = ((*stream).write.unwrap())(
            stream,
            bytes[pos..].as_ptr().cast(),
            (bytes.len() - pos) as u64,
        );
        if n <= 0 || n as usize > bytes.len() - pos {
            return false;
        }
        pos += n as usize;
    }
    p.log(format_args!("save span={span} offset={value}"));
    true
}
unsafe extern "C" fn load(plugin: *const clap_plugin, stream: *const clap_istream) -> bool {
    let p = probe(plugin);
    let mut bytes = [0u8; 16];
    let mut pos = 0;
    while pos < bytes.len() {
        let n = ((*stream).read.unwrap())(
            stream,
            bytes[pos..].as_mut_ptr().cast(),
            (bytes.len() - pos) as u64,
        );
        if n <= 0 || n as usize > bytes.len() - pos {
            return false;
        }
        pos += n as usize;
    }
    if &bytes[..4] != b"RNG1" {
        return false;
    }
    let span = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let value = f64::from_bits(u64::from_le_bytes(bytes[8..].try_into().unwrap()));
    if !(5..=LIMIT).contains(&span) || !value.is_finite() || value.abs() > span as f64 {
        return false;
    }
    *p.restore.lock().unwrap() = Some((span, value));
    p.log(format_args!("load span={span} offset={value} active={}", p.active.load(SeqCst)));
    main_thread(plugin);
    true
}
static STATE: clap_plugin_state = clap_plugin_state { save: Some(save), load: Some(load) };
unsafe extern "C" fn ports(_: *const clap_plugin, _: bool) -> u32 {
    1
}
unsafe extern "C" fn port(
    _: *const clap_plugin,
    index: u32,
    _: bool,
    info: *mut clap_audio_port_info,
) -> bool {
    if index != 0 {
        return false;
    }
    *info = clap_audio_port_info {
        id: 0,
        name: [0; 256],
        flags: CLAP_AUDIO_PORT_IS_MAIN,
        channel_count: 2,
        port_type: CLAP_PORT_STEREO.as_ptr(),
        in_place_pair: 0,
    };
    text((*info).name.as_mut_ptr(), 256, "Silent probe");
    true
}
static PORTS: clap_plugin_audio_ports =
    clap_plugin_audio_ports { count: Some(ports), get: Some(port) };
unsafe extern "C" fn extension(_: *const clap_plugin, id: *const c_char) -> *const c_void {
    match CStr::from_ptr(id) {
        id if id == CLAP_EXT_PARAMS => (&PARAMS as *const clap_plugin_params).cast(),
        id if id == CLAP_EXT_STATE => (&STATE as *const clap_plugin_state).cast(),
        id if id == CLAP_EXT_AUDIO_PORTS => (&PORTS as *const clap_plugin_audio_ports).cast(),
        _ => ptr::null(),
    }
}
unsafe extern "C" fn factory_count(_: *const clap_plugin_factory) -> u32 {
    1
}
unsafe extern "C" fn descriptor(
    _: *const clap_plugin_factory,
    index: u32,
) -> *const clap_plugin_descriptor {
    if index == 0 {
        &DESCRIPTOR
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn create(
    _: *const clap_plugin_factory,
    host: *const clap_host,
    id: *const c_char,
) -> *const clap_plugin {
    if CStr::from_ptr(id) != CStr::from_ptr(DESCRIPTOR.id) {
        return ptr::null();
    }
    let path = format!(
        "/tmp/harmonigraph-offset-range-probe-{}-{}.log",
        std::process::id(),
        SERIAL.fetch_add(1, SeqCst)
    );
    let Ok(file) = File::options().write(true).create_new(true).open(path) else {
        return ptr::null();
    };
    let (producer, consumer) = rtrb::RingBuffer::new(65536);
    let mut p = Box::new(Probe {
        plugin: clap_plugin {
            desc: &DESCRIPTOR,
            plugin_data: ptr::null_mut(),
            init: Some(init),
            destroy: Some(destroy),
            activate: Some(activate),
            deactivate: Some(deactivate),
            start_processing: Some(start),
            stop_processing: Some(stop),
            reset: Some(stop),
            process: Some(process),
            get_extension: Some(extension),
            on_main_thread: Some(main_thread),
        },
        host,
        active: AtomicBool::new(false),
        restarting: AtomicBool::new(false),
        span: AtomicU32::new(5),
        requested: AtomicU32::new(5),
        value: AtomicU64::new(0f64.to_bits()),
        restore: Mutex::new(None),
        events: UnsafeCell::new(producer),
        dropped: AtomicU32::new(0),
        trace: Mutex::new(Trace { file, consumer }),
    });
    p.plugin.plugin_data = (&mut *p as *mut Probe).cast();
    &(*Box::into_raw(p)).plugin
}
static FACTORY: clap_plugin_factory = clap_plugin_factory {
    get_plugin_count: Some(factory_count),
    get_plugin_descriptor: Some(descriptor),
    create_plugin: Some(create),
};
unsafe extern "C" fn entry_init(_: *const c_char) -> bool {
    true
}
unsafe extern "C" fn entry_deinit() {}
unsafe extern "C" fn get_factory(id: *const c_char) -> *const c_void {
    if CStr::from_ptr(id) == CLAP_PLUGIN_FACTORY_ID {
        (&FACTORY as *const clap_plugin_factory).cast()
    } else {
        ptr::null()
    }
}
#[no_mangle]
pub static clap_entry: clap_plugin_entry = clap_plugin_entry {
    clap_version: CLAP_VERSION,
    init: Some(entry_init),
    deinit: Some(entry_deinit),
    get_factory: Some(get_factory),
};

#[cfg(test)]
mod tests;
