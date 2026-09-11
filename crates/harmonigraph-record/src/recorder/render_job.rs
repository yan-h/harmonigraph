//! Render requests, per-take cancellation, subprocess lifetime and progress.

use super::home_dir;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// What to run once a take is complete.
///
/// The plugin does not render video; `harmonigraph-offline` does, with a
/// headless GPU device and an ffmpeg pipe, neither of which belongs in a
/// real-time audio plugin. This just launches it, off the audio thread
/// and off the GUI thread, so a long render never touches the DAW.
pub struct RenderRequest {
    pub program: std::path::PathBuf,
    /// Bounced audio to mux in and feed the spectrum, if any.
    pub audio: Option<String>,
    /// `--align` value (take-time the audio starts), if set; else auto-align.
    pub align: Option<String>,
    /// A appearance document passed as `--appearance`, overriding the take's record-time
    /// look — set for "Re-render take" so post-record settings reach the video;
    /// `None` for auto-render (which uses the take's own recorded look).
    pub appearance: Option<String>,
    /// Output pixels, from the Video pane's Aspect and Resolution.
    ///
    /// Passed rather than left to the renderer's own default because that
    /// default knows the take's aspect but not which resolution was picked
    /// beside it.
    pub size: [u32; 2],
    /// Whether to bake the whole-song playhead spectrogram rather than the
    /// live scrolling one.
    ///
    /// `None` leaves it to the take's own recorded setting, which is what the
    /// Video pane's Spectrogram row writes — so the choice reaches the render
    /// through the take rather than through a flag. `Some(true)` forces it on;
    /// there is no forcing it off, because the renderer ORs the flag with the
    /// take's setting and a flag cannot subtract.
    pub playhead: Option<bool>,
}

impl RenderRequest {
    /// The render that runs when a take finishes. A finished take always renders
    /// now — its own recorded audio as the spectrogram, laid out as the take's
    /// own spectrogram choice says — so this is unconditional; the `Option` is kept only for `Control::stop`'s signature.
    /// Uses the take's own recorded look.
    pub fn from_config(config: &harmonigraph_take::RenderConfig) -> Option<RenderRequest> {
        Some(Self::build(config, None))
    }

    /// Build a request for an explicit "Re-render take": always built, and it
    /// carries the CURRENT `appearance` blob so the render reflects the frame,
    /// bounce, and offset dialed in *after* recording — not the take's
    /// record-time snapshot.
    pub fn render_now(
        config: &harmonigraph_take::RenderConfig,
        appearance: String,
    ) -> RenderRequest {
        Self::build(config, Some(appearance))
    }

    /// A blank renderer path means "use the default" rather than an empty
    /// argument the renderer would reject.
    fn build(
        config: &harmonigraph_take::RenderConfig,
        appearance: Option<String>,
    ) -> RenderRequest {
        let program = if config.renderer_path.trim().is_empty() {
            default_renderer_path()
        } else {
            std::path::PathBuf::from(config.renderer_path.trim())
        };
        RenderRequest {
            program,
            // Bounced audio is shelved: every render uses the take's own
            // recording as soundtrack and spectrum, aligned by construction — so
            // no --audio replacement and no --align override.
            audio: None,
            align: None,
            appearance,
            size: config.frame.pixels(config.short_edge),
            // Not forced: the take carries the Video pane's Spectrogram choice
            // and the renderer reads it, so passing `--playhead` here would
            // OR itself over a "Scrolling" the user had picked and the row would
            // control nothing.
            playhead: None,
        }
    }
}

/// Where `update-plugin.sh` installs the renderer, and where the plugin
/// looks when the path setting is left empty. A fixed location beats
/// guessing at the host's working directory or the bundle's own path.
pub fn default_renderer_path() -> std::path::PathBuf {
    home_dir().join("Library/Application Support/Harmonigraph/harmonigraph-offline")
}

/// Frames done and frames planned for the render(s) in flight, published by
/// the thread following the renderer's stderr and read by the GUI each frame.
///
/// `in_flight` is a COUNT rather than a flag, though [`RenderControl`] now
/// serialises renders so it only ever reaches one. It is the cheaper way to be
/// wrong: a flag cleared by whichever render finished first would blank the
/// bar out from under a running one, and counting cannot.
#[derive(Default)]
pub(super) struct Progress {
    done: AtomicU64,
    /// 0 until the renderer announces how many frames it is composing.
    total: AtomicU64,
    in_flight: AtomicU64,
}

/// What a render in flight can be reached by, so the next request can cancel
/// it instead of running alongside it.
///
/// Two renders of one take write one video, and there is no useful way to
/// merge that — so a second request is treated as a correction of the first,
/// not an addition to it. That is nearly always what it is: the same take,
/// with a setting changed since.
#[derive(Default)]
pub(super) struct RenderControl {
    /// Bumped by every request, so no two runs share a number — which is what
    /// keeps each run's partial output under a name of its own.
    generation: AtomicU64,
    /// The newest generation claimed for each take.
    ///
    /// Keyed by take, not global, because "superseded" is a claim about ONE
    /// video. Every finished take renders and every take is its own file, so
    /// consecutive recordings are two requests that want two different videos,
    /// and a global newest-wins would have the second silently discard the
    /// first (see
    /// `a_render_of_another_take_waits_rather_than_replacing_this_one`).
    /// Only a second request naming the SAME take is the same video twice,
    /// which is the case "Re-render take" makes.
    ///
    /// An entry is dropped when its run finishes, so this holds one key per
    /// take being rendered rather than one per take of the session.
    claims: Mutex<std::collections::HashMap<std::path::PathBuf, u64>>,
    /// The renderer process now running and the take it is rendering, for a
    /// later request to kill if it is about the same video.
    ///
    /// The child lives here rather than on its own thread's stack precisely so
    /// another thread can reach it; the render thread borrows it back to reap
    /// it once its stderr has closed.
    child: Mutex<Option<InFlight>>,
    /// Held for the length of a render, so renders run one at a time — a
    /// replacement waits for the cancelled run's process to be reaped rather
    /// than merely asked to stop, and a render of another take waits its turn
    /// instead of putting a second ffmpeg beside the first.
    running: Mutex<()>,
}

/// The renderer process now running, and which take's video it is producing.
struct InFlight {
    take: std::path::PathBuf,
    child: std::process::Child,
}

/// Hands a take's claim back when its run ends, by whichever of the render
/// thread's several exits it takes — including the two that stand down before
/// spawning anything. A claim left behind would make the next request for that
/// take look superseded before it started.
struct Claim<'a> {
    control: &'a RenderControl,
    take: &'a std::path::Path,
    generation: u64,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.control.release(self.take, self.generation);
    }
}

impl RenderControl {
    /// Claim the flight for `take`, superseding any earlier claim on it.
    fn claim(&self, take: &std::path::Path) -> u64 {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.claims.lock().insert(take.to_path_buf(), generation);
        generation
    }

    /// Give up `generation`'s claim on `take`, if it is still the standing one.
    /// A newer request has already replaced it otherwise, and dropping that
    /// would tell the newer run it had been superseded by nobody.
    fn release(&self, take: &std::path::Path, generation: u64) {
        let mut claims = self.claims.lock();
        if claims.get(take) == Some(&generation) {
            claims.remove(take);
        }
    }

    /// Kill the render in flight if it is rendering `take`. Returns having
    /// *asked*: the process is reaped by its own thread, which the replacement
    /// waits for through [`running`](Self::running).
    ///
    /// A render of some other take is left alone — it is producing a different
    /// video, and nothing about this request says that one is unwanted.
    fn cancel_in_flight(&self, take: &std::path::Path) {
        if let Some(flight) = self.child.lock().as_mut() {
            if flight.take == take {
                let _ = flight.child.kill();
            }
        }
    }

    /// Stop the render now running and disown it, whatever take it is for.
    ///
    /// What its own thread then sees is the same `superseded` a replacement
    /// request produces, so it leaves by the same door: it deletes the
    /// part-written video and returns without reporting a failure, which is
    /// what leaves the canceller's word the last one on the status line.
    ///
    /// The claim is DROPPED rather than replaced by a newer generation, so
    /// nothing is left in `claims` for that take's next request to be measured
    /// against — a generation no run holds would sit there for the life of the
    /// process.
    ///
    /// Like [`cancel_in_flight`](Self::cancel_in_flight) this returns having
    /// *asked*: the process is reaped by the render thread, and the bar goes
    /// when that thread ends its flight.
    ///
    /// Returns whether there was a render to stop.
    pub(super) fn cancel(&self) -> bool {
        let mut in_flight = self.child.lock();
        let Some(flight) = in_flight.as_mut() else { return false };
        self.claims.lock().remove(&flight.take);
        let _ = flight.child.kill();
        true
    }

    /// Whether a newer request for the SAME take has arrived since
    /// `generation` claimed it.
    fn superseded(&self, take: &std::path::Path, generation: u64) -> bool {
        self.claims.lock().get(take) != Some(&generation)
    }
}

impl Progress {
    /// A render is starting: clear the last one's counts, then join the flight.
    /// Release-ordered against [`read`](Self::read)'s acquire, so a bar can
    /// never appear over the previous render's numbers.
    fn begin(&self) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        self.in_flight.fetch_add(1, Ordering::Release);
    }

    fn end(&self) {
        self.in_flight.fetch_sub(1, Ordering::Release);
    }

    pub(super) fn read(&self) -> Option<harmonigraph_take::RenderProgress> {
        (self.in_flight.load(Ordering::Acquire) > 0).then(|| harmonigraph_take::RenderProgress {
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
        })
    }
}

/// What one segment of the renderer's stderr says about how far it has got.
enum Report {
    /// How many frames the render is composing, from the line it opens with.
    Total(u64),
    /// Frames written of frames planned, from the counter it rewrites as it
    /// goes.
    Frames { done: u64, total: u64 },
}

/// Read a progress report out of one line of `harmonigraph-offline`'s stderr,
/// if that is what it is.
///
/// **This parses another binary's human-readable output**, which is a contract
/// worth naming: the renderer opens with `... -> 5400 frames at 60 fps ...`
/// and then rewrites `  120/5400 frames (2%)` in place. Both are matched here
/// off the count that precedes ` frames`. The alternative — a `--progress`
/// flag emitting something machine-shaped — fails far worse when the installed
/// renderer is older than the plugin: it would reject the unknown flag and
/// render nothing at all, where a format this no longer recognizes just leaves
/// the bar empty and everything else working.
fn parse_report(segment: &str) -> Option<Report> {
    // The count is the last token before ` frames`, so a take path with a
    // slash (or a space) in it has nothing to say here.
    let token = segment.split_once(" frames")?.0.split_whitespace().next_back()?;
    match token.split_once('/') {
        Some((done, total)) => {
            Some(Report::Frames { done: done.parse().ok()?, total: total.parse().ok()? })
        }
        None => Some(Report::Total(token.parse().ok()?)),
    }
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
fn rendered_status(out: &std::path::Path, warning: Option<&str>) -> String {
    match warning {
        Some(warning) => format!("rendered {} — {warning}", out.display()),
        None => format!("rendered {}", out.display()),
    }
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
        match parse_report(text) {
            Some(Report::Frames { done, total }) => {
                progress.done.store(done, Ordering::Relaxed);
                progress.total.store(total, Ordering::Relaxed);
            }
            Some(Report::Total(total)) => progress.total.store(total, Ordering::Relaxed),
            // Diagnostics, warnings, the renderer's own error. The last of
            // them is the status line's if the render fails; the first
            // `warning:` among them is its own, because success discards the
            // last and a warned-about export still needs to say so.
            None if !text.is_empty() => {
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

/// Run the renderer on the finished take, on a thread of its own so a
/// long render neither blocks the writer nor the DAW. The video lands
/// next to the take.
pub(super) fn spawn_render(
    request: RenderRequest,
    take_path: std::path::PathBuf,
    status: Arc<Mutex<String>>,
    progress: Arc<Progress>,
    control: Arc<RenderControl>,
) {
    // Claim the flight before spawning anything: a second request for THIS
    // take cancels the one running rather than joining it. Two renders of one
    // take write one video, and the newer request is always the wanted one —
    // it is the same take with settings changed since.
    //
    // A request for another take makes no such claim. It queues on `running`
    // instead and renders when its turn comes.
    let generation = control.claim(&take_path);
    // On the CALLER's thread, so the cancellation lands before the replacement
    // starts queueing behind a run that now has no reason to finish.
    control.cancel_in_flight(&take_path);

    let _ = std::thread::Builder::new().name("harmonigraph-take-render".into()).spawn(move || {
        let _claim = Claim { control: &control, take: &take_path, generation };
        // Wait out the run being cancelled, so its process is reaped
        // before this one starts. Held for the whole render, which is what
        // makes "one render at a time" true rather than hoped for — and
        // what a render of ANOTHER take queues on instead of cancelling.
        let _flight = control.running.lock();
        if control.superseded(&take_path, generation) {
            // Another request arrived while this one queued. It is already
            // waiting on the same lock, and rendering here would only be
            // work to throw away.
            return;
        }

        let out = take_path.with_extension("mp4");
        // Written under a name of this run's own, and moved onto `out`
        // only once it has succeeded.
        //
        // Killing the renderer does not kill the ffmpeg it is piping to —
        // that is a grandchild, and it outlives the kill by however long
        // finalizing takes. Sharing one output path with it is how a
        // cancelled render corrupts the video that replaces it. A path per
        // run means the straggler writes somewhere nobody is reading, and
        // the file at `out` is only ever produced whole, by rename.
        let partial = take_path.with_extension(format!("rendering-{generation}.mp4"));
        // A "Re-render take" carries the current look as a appearance document; write
        // it beside the take and pass --appearance so post-record settings
        // override the take's record-time snapshot. Per-run for the same
        // reason as `partial`, and removed after the run.
        let appearance_file = request.appearance.as_ref().and_then(|blob| {
            let path = take_path.with_extension(format!("rendernow-{generation}.ron"));
            std::fs::write(&path, blob).ok().map(|()| path)
        });

        let mut command = std::process::Command::new(&request.program);
        command.arg(&take_path).arg("--out").arg(&partial);
        if let Some(audio) = &request.audio {
            command.arg("--audio").arg(audio);
        }
        if let Some(align) = &request.align {
            command.arg("--align").arg(align);
        }
        if let Some(file) = &appearance_file {
            command.arg("--appearance").arg(file);
        }
        let [w, h] = request.size;
        command.arg("--size").arg(format!("{w}x{h}"));
        // Only when something forces it. The renderer turns the whole-song
        // playhead on for `--playhead` OR the take's own recorded setting,
        // so a flag passed unconditionally here is one the Video pane's
        // Spectrogram row can never turn back off — which is exactly what
        // it was, and why picking "Scrolling" did nothing. Left alone, the
        // take's setting is the whole answer, in both directions.
        if request.playhead == Some(true) {
            command.arg("--playhead");
        }

        // stdout is the renderer's `--dump-layout` channel and nothing
        // else; stderr carries everything this cares about, so pipe that
        // one and follow it.
        command.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped());

        *status.lock() = format!("rendering {}...", out.display());
        let spawned = command.spawn();
        let cleanup = || {
            if let Some(file) = &appearance_file {
                let _ = std::fs::remove_file(file);
            }
            let _ = std::fs::remove_file(&partial);
        };
        let mut child = match spawned {
            Ok(child) => child,
            Err(err) => {
                cleanup();
                *status.lock() = format!(
                    "could not run {}: {err} — check the Renderer path",
                    request.program.display()
                );
                return;
            }
        };

        let stderr = child.stderr.take();
        // Hand the process over so a later request can reach it. Under the
        // same lock a canceller takes, and re-checking the generation
        // inside it: a request that arrived between the spawn and here
        // found no child to kill, so this is where that one gets killed
        // instead of running to completion unnoticed.
        {
            let mut in_flight = control.child.lock();
            if control.superseded(&take_path, generation) {
                let _ = child.kill();
            }
            *in_flight = Some(InFlight { take: take_path.clone(), child });
        }
        // AFTER the handover, not before it: the bar is what the Video pane
        // hangs its Cancel off, and `RenderControl::cancel` can only reach a
        // child that has been published above. Begun any earlier, the bar
        // spends the spawn offering a cancel that would quietly do nothing.
        progress.begin();
        // Ends at EOF on the pipe, which a kill brings about immediately.
        let tail = stderr.map(|pipe| follow(pipe, &progress)).unwrap_or_default();
        let result = match control.child.lock().take() {
            Some(mut flight) => flight.child.wait(),
            // Unreachable in practice: nothing else takes the child, only
            // kills it. Reported rather than unwrapped, since a render
            // thread panicking in a DAW is not worth the tidier code.
            None => Err(std::io::Error::other("the render process went missing")),
        };
        progress.end();

        // A cancelled render has nothing to say: its replacement is
        // already running and the failure is one we caused on purpose.
        // Its partial output goes, and the status line stays the new
        // render's.
        if control.superseded(&take_path, generation) {
            cleanup();
            return;
        }

        match result {
            Ok(exit) if exit.success() => {
                // Whole, and only now under the name anything else reads.
                match std::fs::rename(&partial, &out) {
                    Ok(()) => {
                        *status.lock() = rendered_status(&out, tail.warning.as_deref());
                    }
                    Err(err) => {
                        *status.lock() = format!("rendered, but could not move into place: {err}")
                    }
                }
                if let Some(file) = &appearance_file {
                    let _ = std::fs::remove_file(file);
                }
            }
            // The renderer's own diagnostics are far more useful than the
            // exit code, and this is the only place a plugin user will
            // ever see them.
            Ok(_) => {
                cleanup();
                *status.lock() = format!("render failed: {}", tail.last);
            }
            Err(err) => {
                cleanup();
                *status.lock() = format!("render failed: {err}");
            }
        }
    });
}
