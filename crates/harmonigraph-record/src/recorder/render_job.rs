//! Instance-owned sequential export queue and renderer subprocess lifetime.
use super::home_dir;
use harmonigraph_take::{
    render::{ExportJob, ExportStatus},
    RenderProgress,
};
use parking_lot::Mutex;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
};
#[cfg(test)]
mod tests;

/// What to run once a take is complete.
///
/// The plugin does not render video; `harmonigraph-offline` does, with a
/// headless GPU device and an ffmpeg pipe, neither of which belongs in a
/// real-time audio plugin. This just launches it, off the audio thread
/// and off the GUI thread, so a long render never touches the DAW.
#[derive(Clone)]
pub struct RenderRequest {
    pub program: std::path::PathBuf,
    /// A appearance document passed as `--appearance`, overriding the take's record-time
    /// look — set for "Re-render take" so post-record settings reach the video;
    /// `None` for auto-render (which uses the take's own recorded look).
    pub appearance: Option<String>,
    /// Explicit output override for Re-render. Automatic exports read the
    /// captured Aspect and Output size from their recorded appearance.
    pub size: Option<[u32; 2]>,
    /// A session-local completion notice, owned by this request so a new
    /// recording cannot erase the reason an earlier render was cut short.
    pub notice: Option<&'static str>,
}

impl RenderRequest {
    /// The render that runs when a take finishes: its own recorded audio as the
    /// spectrogram, laid out as the take's own spectrogram choice says, in the
    /// take's own recorded look.
    pub fn recorded() -> RenderRequest {
        RenderRequest {
            program: default_renderer_path(),
            appearance: None,
            size: None,
            notice: None,
        }
    }

    /// Build a request for an explicit "Re-render take": always built, and it
    /// carries the CURRENT `appearance` blob so the render reflects the framing
    /// and appearance dialed in *after* recording — not the take's
    /// record-time snapshot.
    pub fn render_now(
        config: &harmonigraph_take::RenderConfig,
        appearance: String,
    ) -> RenderRequest {
        Self::build(config, Some(appearance))
    }

    fn build(
        config: &harmonigraph_take::RenderConfig,
        appearance: Option<String>,
    ) -> RenderRequest {
        RenderRequest {
            program: default_renderer_path(),
            appearance,
            size: Some(config.frame.pixels(config.short_edge)),
            notice: None,
        }
    }
}

/// Where `update-plugin.sh` installs the renderer, and where the plugin
/// finds its paired renderer. A fixed location beats
/// guessing at the host's working directory or the bundle's own path.
pub fn default_renderer_path() -> std::path::PathBuf {
    home_dir().join("Library/Application Support/Harmonigraph/harmonigraph-offline")
}

/// Only the FIFO worker writes progress; the editor reads its active job.
#[derive(Default)]
pub(super) struct Progress {
    done: AtomicU64,
    total: AtomicU64,
    in_flight: AtomicBool,
}
impl Progress {
    fn begin(&self) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        self.in_flight.store(true, Ordering::Release);
    }
    fn end(&self) {
        self.in_flight.store(false, Ordering::Release);
    }
    pub(super) fn read(&self) -> Option<RenderProgress> {
        self.in_flight.load(Ordering::Acquire).then(|| RenderProgress {
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
        })
    }
}
struct Job {
    snapshot: ExportJob,
    request: RenderRequest,
    capture_failed: bool,
}
#[derive(Default)]
struct Queue {
    jobs: Vec<Job>,
    next_id: u64,
    working: bool,
    closed: bool,
    worker: Option<std::thread::JoinHandle<()>>,
}
#[derive(Default)]
pub(super) struct RenderControl {
    #[cfg(feature = "test-support")]
    pub(super) test_program: Mutex<Option<PathBuf>>,
    queue: Mutex<Queue>,
    child: Mutex<Option<InFlight>>,
    // A fixture can hold queued work without replacing a renderer process.
    #[cfg(test)]
    pub(super) running: Mutex<()>,
    #[cfg(test)]
    before_publish: Mutex<()>,
}
struct InFlight {
    id: u64,
    child: std::process::Child,
}

impl RenderControl {
    pub(super) fn snapshots(&self, progress: &Progress) -> Vec<ExportJob> {
        self.queue
            .lock()
            .jobs
            .iter()
            .map(|job| {
                let mut snapshot = job.snapshot.clone();
                if matches!(snapshot.state, ExportStatus::Running | ExportStatus::Cancelling) {
                    snapshot.progress = progress.read().unwrap_or_default();
                }
                snapshot
            })
            .collect()
    }
    pub(super) fn cancel_job(&self, id: u64) {
        {
            let mut queue = self.queue.lock();
            let Some(job) = queue.jobs.iter_mut().find(|job| job.snapshot.id == id) else { return };
            job.snapshot.state = match job.snapshot.state {
                ExportStatus::Pending => ExportStatus::Cancelled,
                ExportStatus::Running => ExportStatus::Cancelling,
                state => state,
            };
        }
        // Never wait for the child lock while holding queue: handover takes them
        // in the opposite order to catch cancellation during process spawn.
        if let Some(flight) = self.child.lock().as_mut().filter(|flight| flight.id == id) {
            kill_render(&mut flight.child);
        }
    }
    pub(super) fn clear_finished(&self) {
        self.queue.lock().jobs.retain(|job| !job.snapshot.state.is_finished());
    }
    pub(super) fn shutdown(&self) {
        let worker = {
            let mut queue = self.queue.lock();
            queue.closed = true;
            for job in &mut queue.jobs {
                match job.snapshot.state {
                    ExportStatus::Pending => job.snapshot.state = ExportStatus::Cancelled,
                    ExportStatus::Running => job.snapshot.state = ExportStatus::Cancelling,
                    _ => (),
                }
            }
            queue.worker.take()
        };
        if let Some(flight) = self.child.lock().as_mut() {
            kill_render(&mut flight.child);
        }
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }
    pub(super) fn retry(
        self: &Arc<Self>,
        id: u64,
        status: Arc<Mutex<String>>,
        progress: Arc<Progress>,
    ) {
        let mut queue = self.queue.lock();
        if queue.closed {
            return;
        }
        if let Some(index) = queue.jobs.iter().position(|j| {
            j.snapshot.id == id
                && matches!(j.snapshot.state, ExportStatus::Failed | ExportStatus::Cancelled)
        }) {
            let job = &mut queue.jobs[index];
            if job.capture_failed {
                match harmonigraph_take::Header::read(&job.snapshot.take) {
                    Ok(header) => {
                        job.request.appearance = header.appearance;
                        job.capture_failed = false;
                    }
                    Err(error) => {
                        job.snapshot.detail =
                            format!("could not capture recorded appearance: {error}");
                        return;
                    }
                }
            }
            let mut job = queue.jobs.remove(index);
            job.snapshot.state = ExportStatus::Pending;
            job.snapshot.detail.clear();
            job.snapshot.progress = RenderProgress::default();
            queue.jobs.push(job);
            start_worker(self, &mut queue, status, progress);
        }
    }
}

pub(super) fn spawn_render(
    mut request: RenderRequest,
    take: PathBuf,
    status: Arc<Mutex<String>>,
    progress: Arc<Progress>,
    control: Arc<RenderControl>,
) {
    let error = if request.appearance.is_none() {
        match harmonigraph_take::Header::read(&take) {
            Ok(header) => {
                request.appearance = header.appearance;
                None
            }
            Err(error) => Some(format!("could not capture recorded appearance: {error}")),
        }
    } else {
        None
    };
    let mut queue = control.queue.lock();
    if queue.closed {
        return;
    }
    queue.next_id += 1;
    let id = queue.next_id;
    let output = available_output(&take, |candidate| {
        candidate.symlink_metadata().is_ok()
            || queue.jobs.iter().any(|job| job.snapshot.output == candidate)
    });
    queue.jobs.push(Job {
        capture_failed: error.is_some(),
        snapshot: ExportJob {
            id,
            take,
            output,
            size: request.size,
            state: if error.is_some() { ExportStatus::Failed } else { ExportStatus::Pending },
            progress: RenderProgress::default(),
            detail: error.unwrap_or_default(),
        },
        request,
    });
    start_worker(&control, &mut queue, status, progress);
}

fn start_worker(
    control: &Arc<RenderControl>,
    queue: &mut Queue,
    status: Arc<Mutex<String>>,
    progress: Arc<Progress>,
) {
    if queue.working {
        return;
    }
    queue.working = true;
    let control = control.clone();
    match std::thread::Builder::new().name("harmonigraph-export-queue".into()).spawn(move || loop {
        #[cfg(test)]
        let _gate = control.running.lock();
        let (snapshot, request) = {
            let mut queue = control.queue.lock();
            let Some(job) =
                queue.jobs.iter_mut().find(|job| job.snapshot.state == ExportStatus::Pending)
            else {
                queue.working = false;
                return;
            };
            progress.begin();
            job.snapshot.state = ExportStatus::Running;
            (job.snapshot.clone(), job.request.clone())
        };
        *status.lock() = format!("rendering {}...", snapshot.output.display());
        let result = run_job(&control, &snapshot, &request, &progress);
        let mut queue = control.queue.lock();
        let final_progress = progress.read().unwrap_or_default();
        progress.end();
        let Some(job) = queue.jobs.iter_mut().find(|job| job.snapshot.id == snapshot.id) else {
            continue;
        };
        job.snapshot.progress = final_progress;
        if job.snapshot.state == ExportStatus::Cancelling {
            job.snapshot.state = ExportStatus::Cancelled;
            job.snapshot.detail = "Cancelled; partial output removed".into();
        } else {
            match result {
                Ok((output, detail)) => {
                    job.snapshot.output = output;
                    job.snapshot.state = ExportStatus::Completed;
                    job.snapshot.detail = detail;
                }
                Err(error) => {
                    job.snapshot.state = ExportStatus::Failed;
                    job.snapshot.detail = error;
                }
            }
        }
        *status.lock() = job.snapshot.detail.clone();
    }) {
        Ok(worker) => queue.worker = Some(worker),
        Err(error) => {
            queue.working = false;
            for job in &mut queue.jobs {
                if job.snapshot.state == ExportStatus::Pending {
                    job.snapshot.state = ExportStatus::Failed;
                    job.snapshot.detail = format!("could not start export worker: {error}");
                }
            }
        }
    }
}

fn available_output(take: &Path, mut occupied: impl FnMut(&Path) -> bool) -> PathBuf {
    for suffix in 0u64.. {
        let candidate = output_candidate(take, suffix);
        if !occupied(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn output_candidate(take: &Path, suffix: u64) -> PathBuf {
    if suffix == 0 {
        take.with_extension("mp4")
    } else {
        take.with_file_name(format!(
            "{}-{suffix}.mp4",
            take.file_stem().unwrap_or_default().to_string_lossy()
        ))
    }
}

/// Exclusive directory creation owns every scratch file beneath it. Separate
/// plugin instances can share a take without sharing partial output or cleanup.
struct Staging(PathBuf);
impl Staging {
    fn create(take: &Path, id: u64) -> std::io::Result<Self> {
        for suffix in 0u64.. {
            let path = take.with_extension(format!("export-{}-{id}-{suffix}", std::process::id()));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run_job(
    control: &RenderControl,
    job: &ExportJob,
    request: &RenderRequest,
    progress: &Progress,
) -> Result<(PathBuf, String), String> {
    let stage = Staging::create(&job.take, job.id)
        .map_err(|e| format!("could not create export scratch directory: {e}"))?;
    let partial = stage.0.join("video.mp4");
    let appearance = stage.0.join("appearance.ron");
    if let Some(blob) = &request.appearance {
        std::fs::write(&appearance, blob)
            .map_err(|e| format!("could not save queued appearance: {e}"))?;
    }
    #[cfg(feature = "test-support")]
    let program = control.test_program.lock().clone().unwrap_or_else(|| request.program.clone());
    #[cfg(not(feature = "test-support"))]
    let program = &request.program;
    let mut command = std::process::Command::new(program);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.arg(&job.take).arg("--out").arg(&partial);
    if request.appearance.is_some() {
        command.arg("--appearance").arg(&appearance);
    }
    if let Some([w, h]) = request.size {
        command.arg("--size").arg(format!("{w}x{h}"));
    }
    command.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());
    let mut child = command.spawn().map_err(|e| format!("could not run paired renderer: {e}"))?;
    let stderr = child.stderr.take();
    {
        let mut flight = control.child.lock();
        if !control
            .queue
            .lock()
            .jobs
            .iter()
            .any(|j| j.snapshot.id == job.id && j.snapshot.state == ExportStatus::Running)
        {
            kill_render(&mut child);
        }
        *flight = Some(InFlight { id: job.id, child });
    }
    let tail = stderr.map(|pipe| follow(pipe, progress)).unwrap_or_default();
    let result = loop {
        let mut flight = control.child.lock();
        match flight.as_mut().expect("worker owns child").child.try_wait() {
            Ok(Some(exit)) => {
                flight.take();
                break exit;
            }
            Ok(None) => (),
            Err(error) => {
                let mut child = flight.take().expect("worker owns child").child;
                kill_render(&mut child);
                let _ = child.wait();
                return Err(format!("render failed: {error}"));
            }
        }
        drop(flight);
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    if !result.success() {
        return Err(format!("render failed: {}", tail.last));
    }
    #[cfg(test)]
    let _gate = control.before_publish.lock();
    // Publication and cancellation linearize under the same lock. A completed
    // file wins over a late cancel; a cancellation can never publish a partial.
    let mut queue = control.queue.lock();
    let index =
        queue.jobs.iter().position(|j| j.snapshot.id == job.id).expect("active job retained");
    if queue.jobs[index].snapshot.state != ExportStatus::Running {
        return Err("cancelled".into());
    }
    let mut output = job.output.clone();
    let mut suffix = 0;
    loop {
        match std::fs::hard_link(&partial, &output) {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                output = output_candidate(&job.take, suffix);
                suffix += 1;
                while queue
                    .jobs
                    .iter()
                    .any(|j| j.snapshot.id != job.id && j.snapshot.output == output)
                {
                    output = output_candidate(&job.take, suffix);
                    suffix += 1;
                }
            }
            Err(e) => return Err(format!("could not publish video without overwriting: {e}")),
        }
    }
    let current = &mut queue.jobs[index];
    current.snapshot.state = ExportStatus::Completed;
    current.snapshot.output = output.clone();
    current.snapshot.progress = progress.read().unwrap_or_default();
    let detail = rendered_status(&output, request.notice, tail.warning.as_deref());
    current.snapshot.detail = detail.clone();
    Ok((output, detail))
}
/// Stop the renderer and its encoder together. Killing only the renderer
/// leaves ffmpeg holding stderr while it pads audio through the planned end,
/// so `follow` and the next queued render would wait for all of that work.
/// The unreaped child leads the private process group established at spawn.
fn kill_render(child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: a spawned child's PID is positive and cannot be reused
        // before wait; its negative names only the group we created for it.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    // Also serves non-Unix targets and falls back if group signalling failed.
    let _ = child.kill();
}

/// Longest stderr run with no separator in it that is worth keeping. Past this
/// the segment is not a line the renderer meant to print, and buffering it
/// only costs memory.
const STDERR_SEGMENT_CAP: usize = 8 * 1024;

/// What following a render's stderr left worth saying afterwards.
#[derive(Default)]
struct Tail {
    /// The last segment that was not a progress counter: the renderer's own
    /// last word, and what a failed render is reported by.
    last: String,
    /// The FIRST segment the renderer marked `warning:`, kept even when the
    /// render then succeeds.
    ///
    /// A render that warns and finishes is the case #712 asks for — the video
    /// exists and something about it is not what was played — and success
    /// otherwise throws [`last`](Self::last) away, so without this the only
    /// notice of it is a stderr pipe nobody reads. The status line is the one
    /// place a plugin user sees the renderer at all.
    ///
    /// First rather than last: the renderer prints the take's own warnings
    /// before anything the command line can add, so what a person is told
    /// about is the recording rather than the flags.
    warning: Option<String>,
}

/// What the status line says when a render finished. A warning it printed on
/// the way rides along, because "rendered X" alone would say a take with holes
/// in its note history came out whole.
fn rendered_status(out: &std::path::Path, notice: Option<&str>, warning: Option<&str>) -> String {
    let mut status = format!("rendered {}", out.display());
    for detail in [notice, warning].into_iter().flatten() {
        status.push_str(" — ");
        status.push_str(detail);
    }
    status
}

/// Follow the renderer's stderr to its end, publishing progress as it arrives,
/// and hand back what it said that was not progress.
///
/// Read continuously rather than collected at the end (what `Command::output`
/// would do) for two reasons: a frame counter is only useful while the render
/// is still running, and the renderer — plus the ffmpeg it inherits this pipe
/// to — would eventually block on a pipe nobody is draining.
///
/// Split on `\r` as well as `\n`: the counter is REWRITTEN in place on one
/// terminal line, so newlines alone would deliver the whole run of it as a
/// single line, once, at the end.
fn follow(mut stderr: impl std::io::Read, progress: &Progress) -> Tail {
    let mut buffer = [0u8; 4096];
    let mut segment: Vec<u8> = Vec::new();
    let mut tail = Tail::default();
    let take = |segment: &mut Vec<u8>, tail: &mut Tail| {
        let text = String::from_utf8_lossy(segment);
        let text = text.trim();
        match RenderProgress::parse_record(text) {
            Some(RenderProgress { done, total }) => {
                progress.done.store(done, Ordering::Relaxed);
                progress.total.store(total, Ordering::Relaxed);
            }
            // Diagnostics, warnings, the renderer's own error. The last of
            // them is the status line's if the render fails; the first
            // `warning:` among them is its own, because success discards the
            // last and a warned-about export still needs to say so.
            None if !text.is_empty() && !text.starts_with(RenderProgress::PREFIX) => {
                if tail.warning.is_none() && text.starts_with("warning:") {
                    tail.warning = Some(text.to_owned());
                }
                tail.last = text.to_owned();
            }
            None => {}
        }
        segment.clear();
    };
    loop {
        let read = match stderr.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            // A signal arriving mid-read is not the end of the render, and
            // treating it as one would freeze the bar and truncate the
            // diagnostics for a render still going perfectly well.
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        for &byte in &buffer[..read] {
            if byte == b'\r' || byte == b'\n' {
                take(&mut segment, &mut tail);
            } else {
                segment.push(byte);
                if segment.len() >= STDERR_SEGMENT_CAP {
                    take(&mut segment, &mut tail);
                }
            }
        }
    }
    // Whatever the renderer left unterminated on its way out.
    take(&mut segment, &mut tail);
    tail
}
