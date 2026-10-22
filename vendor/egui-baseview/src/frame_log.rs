//! A per-frame timing log, for telling apart the causes of a missed refresh
//! (#1551): a display link that fired late, a frame whose CPU or GPU work ran
//! past its refresh, or one that was done in time and still latched late.
//!
//! Built only with the `frame-log` feature, on macOS with wgpu. Rows go to
//! `~/Library/Logs/Harmonigraph/frames-<unix seconds>-<pid>.csv`, one file per
//! process. Every time is in seconds on the media clock (`CACurrentMediaTime`,
//! the clock the display link, Metal's GPU times and `presentedTime` share).
//!
//! Three kinds of row share one header and are joined on `frame`:
//! - `tick`, from the main thread at the end of every frame callback;
//! - `gpu`, from Metal's completion handler on the present's command buffer.
//!   That buffer only presents, and the queue reaches it once the frame's own
//!   work is through, so its `gpu_end` is when the GPU finished the frame;
//! - `present`, from the drawable's presented handler. A `presented` of 0
//!   means the drawable was never shown.

use std::cell::Cell;
use std::fs::File;
use std::io::{LineWriter, Write};
use std::ptr::NonNull;
use std::sync::Mutex;

use block2::RcBlock;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLCommandBuffer, MTLDrawable};
use objc2_quartz_core::CACurrentMediaTime;

const HEADER: &str = "kind,frame,t,fired,timestamp,target,refresh,tick_ms,acquire_ms,\
                      render_ms,submitted,drawables,gpu_start,gpu_end,presented,window";

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// Written a line at a time: the static is never dropped, so anything still
/// buffered when the host quits or crashes would be lost, and those are the
/// moments worth having.
struct Log {
    file: LineWriter<File>,
}

thread_local! {
    /// The frame now running on the main thread, which a present belongs to.
    static FRAME: Cell<u64> = const { Cell::new(0) };
    static SUBMITTED: Cell<f64> = const { Cell::new(0.0) };
}

/// One frame callback's main-thread readings.
pub(crate) struct Tick {
    /// Which window the frame was for, so two open editors can be told apart.
    /// Only tick rows carry it; a gpu or present row joins on `frame`.
    pub window: usize,
    pub start: f64,
    pub timing: Option<baseview::FrameTiming>,
    pub tick_ms: f32,
    pub acquire_ms: f32,
    pub render_ms: f32,
    pub rendered: bool,
    pub drawables: Option<usize>,
}

/// Start a frame callback: its id, and the time it started.
pub(crate) fn begin() -> f64 {
    FRAME.set(FRAME.get() + 1);
    SUBMITTED.set(0.0);
    CACurrentMediaTime()
}

/// Mark the moment this frame's work is submitted, just before the present.
pub(crate) fn submitted() {
    SUBMITTED.set(CACurrentMediaTime());
}

pub(crate) fn end(tick: Tick) {
    let (fired, timestamp, target, refresh) = tick.timing.map_or((0.0, 0.0, 0.0, 0.0), |t| {
        (t.fired, t.timestamp, t.target_timestamp, t.refresh)
    });
    let submitted = if tick.rendered { SUBMITTED.get() } else { 0.0 };
    write(&format!(
        "tick,{},{:.6},{fired:.6},{timestamp:.6},{target:.6},{refresh:.6},{:.3},{:.3},{:.3},\
         {submitted:.6},{},,,,{:x}",
        FRAME.get(),
        tick.start,
        tick.tick_ms,
        tick.acquire_ms,
        tick.render_ms,
        tick.drawables.map_or(String::new(), |n| n.to_string()),
        tick.window,
    ));
}

/// The observer wgpu-hal calls with each present, before it commits.
pub(crate) fn observe_present(
    command_buffer: &ProtocolObject<dyn MTLCommandBuffer>,
    drawable: &ProtocolObject<dyn MTLDrawable>,
) {
    let frame = FRAME.get();
    let completed = RcBlock::new(move |buffer: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
        // SAFETY: Metal hands the handler the buffer it is attached to.
        let buffer = unsafe { buffer.as_ref() };
        write(&format!(
            "gpu,{frame},,,,,,,,,,,{:.6},{:.6},,",
            buffer.GPUStartTime(),
            buffer.GPUEndTime()
        ));
    });
    let presented = RcBlock::new(move |drawable: NonNull<ProtocolObject<dyn MTLDrawable>>| {
        // SAFETY: Metal hands the handler the drawable it is attached to.
        let drawable = unsafe { drawable.as_ref() };
        write(&format!("present,{frame},,,,,,,,,,,,,{:.6},", drawable.presentedTime()));
    });
    // SAFETY: both blocks are 'static and Metal copies them before returning.
    unsafe {
        command_buffer.addCompletedHandler(RcBlock::as_ptr(&completed));
        drawable.addPresentedHandler(RcBlock::as_ptr(&presented));
    }
}

fn write(row: &str) {
    let Ok(mut log) = LOG.lock() else { return };
    if log.is_none() {
        *log = open();
    }
    let Some(log) = log.as_mut() else { return };
    let _ = writeln!(log.file, "{row}");
}

fn open() -> Option<Log> {
    let dir = std::path::Path::new(&std::env::var_os("HOME")?).join("Library/Logs/Harmonigraph");
    std::fs::create_dir_all(&dir).ok()?;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let path = dir.join(format!("frames-{secs}-{}.csv", std::process::id()));
    let mut file = LineWriter::new(File::create(path).ok()?);
    writeln!(file, "{HEADER}").ok()?;
    Some(Log { file })
}
