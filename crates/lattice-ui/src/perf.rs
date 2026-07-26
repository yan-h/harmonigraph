//! The performance overlay: a small corner HUD over the editor showing the
//! frame rate, the process's memory, and the workload driving them — enough
//! to see at a glance whether the plugin is working the machine hard.
//!
//! What a frame COSTS lives one checkbox further in, under `show_perf_detail`
//! (see [`draw_overlay`]). The headline list carries the frame interval and
//! its worst recent frame, but the interval's mean is the rate in another
//! unit — `fps` is literally its reciprocal — so the only cost reading the
//! HUD gives without the breakdown is that peak.
//!
//! Interactive only. [`root_ui`](crate::root_ui) times the frame, folds the
//! numbers in here, and draws the overlay; the offline renderer bypasses
//! `root_ui` entirely, so nothing in this module ever runs on its
//! (deterministic) draw path — no wall-clock read reaches a recorded frame.

/// The averaging window, in seconds. Every frame cost on the overlay is the
/// plain mean of the frames measured over this long, recomputed from scratch
/// each time and printed unchanged until the next window closes.
///
/// A window mean rather than an exponential moving average, which is the
/// wrong filter for the question the HUD gets asked. An EMA has unbounded
/// memory, so one stalled frame lifts every row for the best part of a
/// second, and a steady 2 ms is indistinguishable from 0.8 ms plus a
/// hiccup — the two want completely different fixes. Worse, keeping one
/// frame-rate-independent means deriving its blend factor from the frame
/// interval, which makes that factor COMMON to every row: when pacing turns
/// ragged the whole HUD lurches at once and reads as correlated cost that is
/// not there. A window mean has no memory to smear and no shared term to
/// couple the rows. What a row shows happened inside that window and nowhere
/// else.
///
/// Holding the value between windows rather than rounding it to a coarse step.
/// Rounding trades one kind of churn for a worse one: a value sitting near a
/// step boundary flips between two readings a whole step apart, which catches
/// the eye harder than the wobble it replaced — and the resolution is gone for
/// good, which at a 2 ms frame cost means throwing away a quarter of the
/// number.
///
/// A quarter second is about fifteen frames at 60 fps: enough to average out
/// ordinary jitter, short enough that a change you just made shows up while
/// you are still looking.
const READOUT_INTERVAL: f64 = 0.25;

/// How many windows the peak column looks back over. Eight of them is two
/// seconds.
///
/// The mean says what a frame usually costs; it cannot say whether the cost is
/// steady, which is the entire difficulty with a spiky signal. A peak answers
/// that — but only if it stays up long enough to be read, and a peak over the
/// current window alone flashes for one latch and is gone before your eye has
/// travelled down the list. Keeping the last few windows' maxima and printing
/// the largest holds a stall visible for two seconds without inventing a decay
/// curve: it is still the worst frame actually measured, just not necessarily
/// one from this quarter second.
const PEAK_WINDOWS: usize = 8;

/// Seconds between memory reads. RSS moves slowly and the read is a syscall,
/// so once a second is plenty and keeps it off the per-frame path.
const MEM_INTERVAL: f64 = 1.0;

/// How quickly the memory readout chases a new reading. Applied per READ, so
/// at [`MEM_INTERVAL`] this settles over a few seconds — fast enough to show
/// something growing, slow enough that ordinary churn doesn't reach the
/// digits.
const MEM_SMOOTH: f64 = 0.3;

/// Granularity of the memory readout, in MB. See [`memory_readout`].
const MEM_STEP_MB: u64 = 10;

/// One frame's measured costs, in milliseconds — every stage of it that
/// anything can see.
///
/// A struct rather than eight arguments because the list kept growing: each
/// time a cost turned out to be hiding between two existing readings, closing
/// that gap meant another parameter. Named fields also stop a caller
/// transposing two of them, which is a silent wrong answer rather than a
/// compile error.
#[derive(Clone, Copy, Default)]
pub struct FrameCosts {
    /// Shell work before the UI ran: draining the event rings.
    pub shell_ms: f32,
    /// Building the UI — the dock and its panes.
    pub cpu_ms: f32,
    /// Turning the resulting shapes into triangles.
    pub tess_ms: f32,
    /// GPU time for egui's own pass: everything 2D.
    pub egui_gpu_ms: f32,
    /// GPU time for the lattice's passes: the 3D scene and its bloom chain.
    /// Carries the `GPU_TIME_UNSUPPORTED` / `PENDING` sentinels.
    pub lattice_gpu_ms: f32,
    /// Blocked acquiring the surface — the vsync wait, which is not work.
    pub acquire_ms: f32,
    /// The whole frame callback, end to end. The other fields are stages of
    /// it, and they do not have to add up to it: whatever is missing is work
    /// nothing measures yet.
    pub tick_ms: f32,
    /// Of that, the renderer half. The difference is the egui half.
    pub render_ms: f32,
    /// The renderer's stages: uploads (including paint callbacks' `prepare`),
    /// encoding egui's draw calls, and finish + submit + present.
    pub upload_ms: f32,
    /// Of the uploads, the texture half.
    pub texture_ms: f32,
    /// Of the uploads, `update_buffers` itself. What the upload reading spans
    /// BESIDES this call — the command encoder, the renderer's write lock, the
    /// MSAA resize — is the difference, and it is not nothing.
    pub ubuf_ms: f32,
    /// The volume the upload had to move, rather than how long it took.
    pub prims: u32,
    pub verts: u32,
    /// Note segments the roll drew through its paint callback. NOT part of
    /// `verts`: the roll owns its instance buffer, so its geometry never
    /// reaches egui's. Four vertices each, against the several hundred a
    /// stroked rounded rect costs.
    pub roll_notes: u32,
    /// The spectrogram's cache fallbacks since the plugin opened: full
    /// re-aggregations of the window, and ring restarts BY REASON (see
    /// `panes::spectrogram::Restart`). CUMULATIVE, so the readout below can
    /// difference them into a rate without a dropped frame losing an event.
    pub spectrogram_fallbacks: (u32, [u32; crate::panes::spectrogram::Restart::COUNT]),
    /// The lattice callback's own `prepare`, which egui-wgpu runs from inside
    /// `update_buffers` — so it is billed to the buffer uploads.
    pub prepare_ms: f32,
    /// Of that, the `device.poll` the GPU timing needs: what the measurement
    /// costs to take.
    pub poll_ms: f32,
    /// Of that, staging the frame: offscreen sizing and the three buffer
    /// writes.
    pub write_ms: f32,
    /// Of that, encoding the scene pass and the bloom chain. Despite living
    /// under a row called "buf up", this is the frame's largest single piece
    /// of CPU work whenever the lattice is on screen.
    pub scene_ms: f32,
    pub encode_ms: f32,
    pub submit_ms: f32,
}

/// One interactive frame's workload: what the overlay reports as the load
/// driving the frame rate and CPU cost. Built by the shell each frame and
/// folded in via [`PerfStats::record`].
#[derive(Clone, Copy)]
pub(crate) struct Workload {
    pub(crate) active_voices: usize,
    pub(crate) held_voices: usize,
    pub(crate) visible_nodes: usize,
    pub(crate) render_scale: f32,
    /// Whether the last frame was repainting continuously (something moving)
    /// rather than idling. The FPS number means different things in each: idle
    /// caps at the ~20 Hz poll by design, so a low idle rate is not a problem.
    ///
    /// Expect "live" almost always in a host, though, and do not read a high
    /// rate as "something must be animating". This is true whenever
    /// `spectrum.is_flowing` is, i.e. whenever samples arrived recently — and
    /// a DAW streams buffers continuously, silence included. With the Analyzer
    /// on screen the editor therefore reads "live" at the full refresh rate
    /// with nothing playing, which is correct: the spectrogram scrolls in real
    /// time, so every frame does have a new column to draw.
    pub(crate) animating: bool,
}

impl Default for Workload {
    fn default() -> Self {
        // render_scale is the 1.0 identity, not 0.0, so an unmeasured frame
        // reads as "full scale, idle".
        Workload {
            active_voices: 0,
            held_voices: 0,
            visible_nodes: 0,
            render_scale: 1.0,
            animating: false,
        }
    }
}

/// One metric's readout: the frames measured since the last latch, the recent
/// windows' maxima, and the two numbers currently printed for it.
///
/// A struct per metric rather than the parallel lists of `x_ms` and
/// `shown_x_ms` fields this replaced. The overlay tracks twenty costs, and
/// with two flat lists every new row meant touching five places — declare,
/// default, accumulate, latch, print — any one of which could be forgotten
/// without the compiler minding.
#[derive(Clone, Copy, Default)]
struct Window {
    /// This window so far: running total, frames counted, and the largest
    /// single frame in it.
    sum: f32,
    n: u32,
    max: f32,
    /// The last [`PEAK_WINDOWS`] closed windows' maxima, by latch slot.
    recent_max: [f32; PEAK_WINDOWS],
    /// What the overlay prints: the last closed window's mean, and the largest
    /// frame anywhere in `recent_max`.
    shown_mean: f32,
    shown_peak: f32,
}

impl Window {
    fn record(&mut self, v: f32) {
        self.sum += v;
        self.n += 1;
        self.max = self.max.max(v);
    }

    /// Close this window and open the next. `slot` is shared by every metric,
    /// so all twenty roll their peak history over on the same latch.
    fn latch(&mut self, slot: usize) {
        // A window with no frames in it holds the printed mean rather than
        // dropping the row to zero. The lattice GPU row only gets a sample on
        // the frames a readback actually lands, and a quiet quarter second
        // there means "nothing new to say", not "it became free".
        if self.n > 0 {
            self.shown_mean = self.sum / self.n as f32;
        }
        self.recent_max[slot] = self.max;
        self.shown_peak = self.recent_max.iter().copied().fold(0.0, f32::max);
        self.sum = 0.0;
        self.n = 0;
        self.max = 0.0;
    }
}

/// Rolling performance numbers, updated once per interactive frame. Runtime
/// only — never persisted, and never touched by the offline renderer.
pub struct PerfStats {
    /// Frame interval in MILLISECONDS, which also drives the FPS readout.
    /// Stored in the same unit as every other row so the two columns can be
    /// formatted by one shared helper.
    frame_ms: Window,
    /// The frame callback end to end, and the two halves it divides into.
    tick: Window,
    /// `tick` minus `render`. Accumulated per frame rather than subtracted
    /// from the two printed means, because the maximum of a difference is not
    /// the difference of the maxima — the peak column would have been quietly
    /// wrong. Same reason `buf_up` is accumulated rather than derived.
    egui: Window,
    shell: Window,
    /// Building the dock and its panes. Wall time on this thread, not GPU
    /// time — the 3D draw is submitted to wgpu and finishes off-thread.
    ui: Window,
    render: Window,
    /// Turning shapes into triangles.
    tess: Window,
    /// The texture half of the uploads, and everything else in them.
    texture: Window,
    buf_up: Window,
    /// `update_buffers` itself, and everything the upload spans around it.
    ubuf: Window,
    around: Window,
    prepare: Window,
    poll: Window,
    /// The two halves of `prepare` worth telling apart: staging the frame's
    /// data, and encoding the scene pass plus the bloom chain.
    write: Window,
    scene: Window,
    /// Blocked acquiring the surface: the vsync wait, which is not work.
    acquire: Window,
    encode: Window,
    submit: Window,
    /// GPU milliseconds for egui's own render pass — the 2D UI, which the
    /// lattice's timer does not cover — and for the lattice's own passes. The
    /// latter only takes a sample on the frames a readback lands.
    egui_gpu: Window,
    gpu: Window,
    /// Which slot of every window's `recent_max` this latch writes.
    peak_slot: usize,
    prims: u32,
    verts: u32,
    roll_notes: u32,
    /// The spectrogram's fallbacks PER SECOND — window re-aggregations and ring
    /// restarts — latched with everything else.
    ///
    /// A rate and not a total, because the total climbs by one whenever the
    /// layout legitimately changes and would read the same as a cache that has
    /// stopped working. What tells those apart is how FAST it climbs: both of
    /// this pane's silent performance bugs sat at hundreds a second while a
    /// healthy build sits at zero.
    spec_fallbacks: (f32, f32),
    /// Which reason accounted for most of the restarts in that interval — the
    /// question that turns a rate into somewhere to look. Empty while there are
    /// none.
    spec_restart_reason: &'static str,
    /// The totals the rates were last differenced from.
    last_fallbacks: (u32, [u32; crate::panes::spectrogram::Restart::COUNT]),
    /// Smoothed resident set size in bytes, refreshed about once a second (0
    /// when the platform can't report it).
    ///
    /// Smoothed where the frame costs are not, because it is a different
    /// kind of signal: a slow-moving LEVEL read once a second, not a
    /// per-frame cost. There is no spike here for a filter to smear across
    /// time — raw RSS simply wanders by megabytes between reads as the host
    /// and the GPU driver take memory and give it back.
    rss_bytes: u64,
    /// Shell-clock time of the last memory read, to throttle it.
    last_mem_read: f64,
    /// Shell-clock time of the previous frame, for measuring the interval.
    /// `None` before the first one.
    last_frame: Option<f64>,
    /// Shell-clock time of the last latch.
    last_readout: f64,
    /// Whether the device ever said it could measure GPU time at all.
    gpu_supported: bool,
    /// Whether a measurement has actually come back.
    have_gpu: bool,
    /// This frame's workload (voice counts, visible nodes, render scale,
    /// whether it was animating).
    workload: Workload,
}

impl Default for PerfStats {
    fn default() -> Self {
        let w = Window::default();
        PerfStats {
            // Seeded to a plausible 60 Hz so the opening frames don't read as
            // absurd: the first frame has no predecessor to measure against,
            // so it contributes no interval at all and this is what shows.
            frame_ms: Window { shown_mean: 1000.0 / 60.0, ..w },
            tick: w,
            egui: w,
            shell: w,
            ui: w,
            render: w,
            tess: w,
            texture: w,
            buf_up: w,
            ubuf: w,
            around: w,
            prepare: w,
            poll: w,
            write: w,
            scene: w,
            acquire: w,
            encode: w,
            submit: w,
            egui_gpu: w,
            gpu: w,
            peak_slot: 0,
            prims: 0,
            verts: 0,
            roll_notes: 0,
            spec_fallbacks: (0.0, 0.0),
            spec_restart_reason: "",
            last_fallbacks: (0, [0; crate::panes::spectrogram::Restart::COUNT]),
            rss_bytes: 0,
            last_mem_read: f64::NEG_INFINITY,
            last_frame: None,
            last_readout: f64::NEG_INFINITY,
            gpu_supported: true,
            have_gpu: false,
            workload: Workload::default(),
        }
    }
}

impl PerfStats {
    /// Fold this frame's measurements in. `cpu_ms` is the measured dock-build
    /// time and `now` the shell clock, which doubles as the frame timestamp.
    ///
    /// The interval is measured here rather than taken from egui's
    /// `stable_dt`, which is only a measured delta when the PREVIOUS pass
    /// asked for an *immediate* repaint; otherwise egui hands back
    /// `RawInput::predicted_dt`, and egui-baseview never sets that, so it
    /// stays at egui's 1/60 default forever. Uncapped that went unnoticed —
    /// every frame asks for an immediate repaint, so every reading was real.
    /// Under a frame-rate cap the request is a delayed one, so the readout
    /// blended a hardcoded 60 with the true rate and reported ~45 fps for a
    /// perfectly steady 30.
    pub(crate) fn record(&mut self, costs: FrameCosts, now: f64, workload: Workload) {
        let FrameCosts {
            shell_ms,
            cpu_ms,
            tess_ms,
            egui_gpu_ms,
            lattice_gpu_ms: gpu_ms,
            acquire_ms,
            tick_ms,
            render_ms,
            upload_ms,
            texture_ms,
            ubuf_ms,
            prims,
            verts,
            roll_notes,
            spectrogram_fallbacks,
            prepare_ms,
            poll_ms,
            write_ms,
            scene_ms,
            encode_ms,
            submit_ms,
        } = costs;
        let dt = self.last_frame.map_or(0.0, |last| (now - last) as f32);
        self.last_frame = Some(now);
        // The first frame has nothing to measure an interval against, so it
        // contributes no sample rather than a zero that would drag the mean.
        if dt > 0.0 {
            self.frame_ms.record(dt * 1000.0);
        }
        self.tick.record(tick_ms);
        // Clamped at zero because these are two independent readings of
        // nested spans, and measurement noise can put the inner one a hair
        // above the outer.
        self.egui.record((tick_ms - render_ms).max(0.0));
        self.shell.record(shell_ms);
        self.ui.record(cpu_ms);
        self.render.record(render_ms);
        self.tess.record(tess_ms);
        self.texture.record(texture_ms);
        self.buf_up.record((upload_ms - texture_ms).max(0.0));
        self.ubuf.record(ubuf_ms);
        // What the upload spans outside `update_buffers`: creating the command
        // encoder, taking the renderer's write lock, and the MSAA resize.
        self.around.record((upload_ms - texture_ms - ubuf_ms).max(0.0));
        self.prepare.record(prepare_ms);
        self.poll.record(poll_ms);
        self.write.record(write_ms);
        self.scene.record(scene_ms);
        self.acquire.record(acquire_ms);
        self.encode.record(encode_ms);
        self.submit.record(submit_ms);
        self.egui_gpu.record(egui_gpu_ms);
        // Three states, not two: a real reading, "the device can't", and
        // "none has landed yet". Collapsing the last two into one "n/a" made
        // a wiring bug and an unsupported GPU look identical, which is
        // exactly the question the row exists to answer.
        match gpu_ms.to_bits() {
            lattice_render::GPU_TIME_UNSUPPORTED => self.gpu_supported = false,
            // Still waiting for the first readback; leave the row saying so.
            lattice_render::GPU_TIME_PENDING => {}
            // Anything else is a real reading, INCLUDING 0.0.
            _ => {
                self.have_gpu = true;
                self.gpu.record(gpu_ms);
            }
        }
        self.prims = prims;
        self.verts = verts;
        self.roll_notes = roll_notes;
        // Close every window at once. `last_readout` starts at -inf, so the
        // first frame latches immediately and the HUD opens showing that
        // frame's real numbers rather than a quarter second of placeholder.
        if now - self.last_readout >= READOUT_INTERVAL {
            let slot = self.peak_slot;
            for window in self.windows_mut() {
                window.latch(slot);
            }
            self.peak_slot = (slot + 1) % PEAK_WINDOWS;
            // Per second over the interval just closed. `last_readout` starts
            // at -inf, so the first latch spans forever and reads zero rather
            // than reporting the opening build as an infinite rate.
            let elapsed = now - self.last_readout;
            if elapsed.is_finite() && elapsed > 0.0 {
                let rate =
                    |total: u32, then: u32| (total.saturating_sub(then) as f64 / elapsed) as f32;
                let by_reason: Vec<u32> = spectrogram_fallbacks
                    .1
                    .iter()
                    .zip(self.last_fallbacks.1)
                    .map(|(total, then)| total.saturating_sub(then))
                    .collect();
                let restarts: u32 = by_reason.iter().sum();
                self.spec_fallbacks = (
                    rate(spectrogram_fallbacks.0, self.last_fallbacks.0),
                    (restarts as f64 / elapsed) as f32,
                );
                // The reason that accounted for most of them, which is the one
                // worth naming.
                self.spec_restart_reason = by_reason
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, n)| **n)
                    .filter(|(_, n)| **n > 0)
                    .map_or("", |(i, _)| crate::panes::spectrogram::Restart::LABELS[i]);
            }
            self.last_fallbacks = spectrogram_fallbacks;
            self.last_readout = now;
        }
        self.workload = workload;
        if now - self.last_mem_read >= MEM_INTERVAL {
            let sample = rss_bytes();
            // Seeded on the first reading rather than eased up from zero,
            // which would read as a plugin growing into its memory over the
            // first few seconds of every session.
            self.rss_bytes = if self.rss_bytes == 0 {
                sample
            } else {
                let (from, to) = (self.rss_bytes as f64, sample as f64);
                (from + (to - from) * MEM_SMOOTH) as u64
            };
            self.last_mem_read = now;
        }
    }

    /// Every window the overlay averages, so a latch cannot quietly miss one.
    /// Borrowing sixteen disjoint fields at once is fine; forgetting to latch
    /// a new row, which is what the flat list of fields invited, was not.
    fn windows_mut(&mut self) -> [&mut Window; 20] {
        [
            &mut self.frame_ms,
            &mut self.tick,
            &mut self.egui,
            &mut self.shell,
            &mut self.ui,
            &mut self.render,
            &mut self.tess,
            &mut self.texture,
            &mut self.buf_up,
            &mut self.ubuf,
            &mut self.around,
            &mut self.prepare,
            &mut self.poll,
            &mut self.write,
            &mut self.scene,
            &mut self.acquire,
            &mut self.encode,
            &mut self.submit,
            &mut self.egui_gpu,
            &mut self.gpu,
        ]
    }

    /// The frame rate as printed: derived from the held mean frame time, so
    /// the headline number holds still with the rows under it rather than
    /// counting off every frame on its own.
    ///
    /// Taking the reciprocal of the MEAN INTERVAL rather than averaging a
    /// per-frame rate, which is the classic way to get this wrong. The
    /// intervals in a window sum to the window, so their mean is (window /
    /// frames) and its reciprocal is exactly (frames / window) — the true
    /// average rate. Averaging 1/dt instead would weight the quick frames
    /// hardest and read high on precisely the ragged pacing worth noticing.
    fn fps(&self) -> f32 {
        if self.frame_ms.shown_mean > 0.0 {
            1000.0 / self.frame_ms.shown_mean
        } else {
            0.0
        }
    }
}

/// The memory row's text: resident size to the nearest [`MEM_STEP_MB`], or
/// "n/a" where the platform won't say.
///
/// Quantized because of what the number is FOR. It answers "is this plugin
/// sitting on a sane amount of memory, and is that amount growing?" — and
/// neither reading needs the exact megabyte, which moves on every refresh.
/// An unquantized readout spent every second rewriting its last digits, and
/// digits that never hold still get squinted at rather than read.
fn memory_readout(rss_bytes: u64) -> String {
    if rss_bytes == 0 {
        return "n/a".to_string();
    }
    let mb = rss_bytes / (1024 * 1024);
    // Nearest step, but never down to a bare "0 MB" while there IS a reading:
    // "<10 MB" is the honest thing to say about a process too small to round.
    let rounded = (mb + MEM_STEP_MB / 2) / MEM_STEP_MB * MEM_STEP_MB;
    if rounded == 0 {
        format!("<{MEM_STEP_MB} MB")
    } else {
        format!("{rounded} MB")
    }
}

/// Which build this binary IS: `<branch> @<short sha>`, stamped at compile
/// time by `build.rs` (`worktree-` stripped, so it is exactly what
/// `./load-plugin.sh <branch>` takes).
///
/// Bitwig loads one bundle and every session builds into its own worktree, so
/// two builds are indistinguishable from inside the DAW — and swapping the
/// slot is a step that can silently not have happened (no rescan, a build that
/// landed in a different worktree, the wrong branch named). The overlay saying
/// it in the picture is the one check that a reload cannot fool.
///
/// Names the last COMMIT, not the working tree — see `build.rs` for why there
/// is no dirty marker.
pub const BUILD_TAG: &str = env!("LATTICE_BUILD_TAG");

/// Points between the overlay and the corner it sits in.
const OVERLAY_INSET: f32 = 8.0;

/// The overlay's rows: `(depth, label, value, peak)`.
///
/// Split out from the drawing so what the two modes SHOW is assertable
/// without a font, a context, or a painter — the reason `tick` is absent
/// from the basic list is a decision worth a test, and it is invisible in a
/// picture.
fn overlay_rows(perf: &PerfStats, detail: bool) -> Vec<(u8, &'static str, String, Option<String>)> {
    let fading = perf.workload.active_voices.saturating_sub(perf.workload.held_voices);
    // A timed row: the window mean, and the worst frame of the last few
    // windows. Two numbers because neither answers the question alone — the
    // mean says what the stage costs, the peak says whether that cost is
    // steady, and until both were on screen a row reading 2 ms could equally
    // be a stage that got slower or a stage that hiccupped once.
    let timed = |depth: u8, label: &'static str, w: &Window| {
        (depth, label, format!("{:.1}", w.shown_mean), Some(format!("{:.1} ms", w.shown_peak)))
    };
    // Depth, label, value, peak. The nesting is the point: every indented row
    // is a PART of the one above it, so a total and its components can be read
    // against each other instead of held in your head. Working out where a
    // frame went meant repeatedly discovering that a cost sat between two
    // readings; the shape of the list now says what contains what.
    //
    // That reading holds for the MEAN column only, which is why the peak sits
    // apart and dimmer. Means are additive — `egui` and `render` sum to `tick`
    // because each frame's parts do — but two stages' worst frames are almost
    // never the same frame, so the peaks do not sum to anything and a column
    // of them must not look like it does.
    //
    // Without `detail` the list is what you read to NOTICE something is wrong:
    // the rate, its worst recent frame, and the workload behind them. A row
    // that only answers WHERE the frame went belongs to the breakdown —
    // `tick` and everything nested under it, and the GPU passes beside them —
    // because that is the question the breakdown exists to answer, and until
    // it is asked the rows are scaffolding sitting over the picture.
    //
    // Note what that leaves: `frame`'s MEAN is the header's `fps` in another
    // unit (`fps()` is its reciprocal), so the peak column is the only thing
    // the headline list says about cost. Reading a rising cost off the basic
    // HUD means watching that peak, or turning the breakdown on.
    let mut rows: Vec<(u8, &str, String, Option<String>)> =
        vec![timed(0, "frame", &perf.frame_ms)];
    if detail {
        rows.extend([
            timed(0, "tick", &perf.tick),
            timed(1, "egui", &perf.egui),
            timed(2, "shell", &perf.shell),
            timed(2, "ui", &perf.ui),
            timed(1, "render", &perf.render),
            timed(2, "tess", &perf.tess),
            timed(2, "tex up", &perf.texture),
            timed(2, "buf up", &perf.buf_up),
            timed(3, "ubuf", &perf.ubuf),
            timed(4, "prep", &perf.prepare),
            timed(4, "poll", &perf.poll),
            timed(4, "write", &perf.write),
            timed(4, "scene", &perf.scene),
            timed(3, "around", &perf.around),
            timed(2, "wait", &perf.acquire),
            timed(2, "encode", &perf.encode),
            timed(2, "submit", &perf.submit),
        ]);
        rows.push((0, "gpu", {
            // Both passes on one line at the top level: they run alongside the
            // CPU stages rather than inside any of them, so nesting either
            // under `tick` would be a lie about what contains what.
            //
            // Means only. Two peaks as well would be four numbers on one row,
            // and the row would stop being readable long before it became more
            // useful.
            let lattice = if !perf.gpu_supported {
                "n/a".to_owned()
            } else if perf.have_gpu {
                format!("{:.1}", perf.gpu.shown_mean)
            } else {
                "—".to_owned()
            };
            format!("{:.1} ui · {lattice} 3d", perf.egui_gpu.shown_mean)
        }, None));
        rows.push((0, "verts", format!("{}k in {} prims", perf.verts / 1000, perf.prims), None));
        // The roll's geometry, which `verts` does not see: it goes to the
        // GPU as instances on the roll's own buffer, four vertices a note.
        rows.push((1, "roll", format!("{} notes", perf.roll_notes), None));
        // What the spectrogram's two caches are NOT absorbing. Both should read
        // zero while a window merely scrolls; anything else is a layer that has
        // fallen back to redrawing the whole heatmap, which costs milliseconds
        // and looks exactly like a correct picture.
        let (folds, rings) = perf.spec_fallbacks;
        // Naming which reason dominated is what turns the rate into somewhere
        // to look: the image changed, the pane changed, or the window moved
        // where the texture could not follow.
        let why = match perf.spec_restart_reason {
            "" => String::new(),
            reason => format!(" ({reason})"),
        };
        rows.push((1, "spec", format!("{folds:.0}/s refold · {rings:.0}/s ring{why}"), None));
    }
    rows.extend([
        (0, "memory", memory_readout(perf.rss_bytes), None),
        (
            0,
            "voices",
            format!("{} held · {fading} fading", perf.workload.held_voices),
            None,
        ),
        (
            0,
            "nodes",
            format!(
                "{}  ·  {:.2}× scale",
                perf.workload.visible_nodes, perf.workload.render_scale
            ),
            None,
        ),
    ]);
    rows
}

/// Draw the overlay in the top-right corner of `area` — the analyzer pane when
/// it is on screen, the whole editor otherwise (see `perf_overlay_area`). A
/// floating, non-interactive panel so it never steals clicks from the view
/// under it.
pub(crate) fn draw_overlay(
    ctx: &egui::Context,
    area: egui::Rect,
    perf: &PerfStats,
    detail: bool,
) {
    let fps = perf.fps();
    // Only flag a low rate while something is actually animating — an idle
    // editor is meant to drop to the poll rate, so a low idle number is fine.
    let health = if perf.workload.animating && fps < 30.0 {
        egui::Color32::from_rgb(0xE5, 0x7A, 0x5A) // warm red
    } else if perf.workload.animating && fps < 50.0 {
        egui::Color32::from_rgb(0xE0, 0xB0, 0x4A) // amber
    } else {
        egui::Color32::from_rgb(0x7A, 0xC8, 0x8A) // calm green
    };
    let state = if perf.workload.animating { "live" } else { "idle" };

    let dim = egui::Color32::from_gray(0x9A);
    let bright = egui::Color32::from_gray(0xE6);
    let mono = egui::FontId::monospace(11.0);
    let head_font = egui::FontId::monospace(12.0);

    let rows = overlay_rows(perf, detail);

    // Painted straight onto a foreground layer rather than assembled from
    // widgets inside an Area.
    //
    // The Area was already `interactable(false)`, which is enough to keep it
    // out of `layer_id_at` — but every `ui.label` inside it still registered a
    // widget rect, and those win the pointer regardless. The result was a dead
    // zone the size of the HUD in the corner of the lattice: no scroll-to-zoom
    // and no drag-to-orbit under it, whenever the overlay was on, which is by
    // default. A readout that changes the thing it is measuring is worse than
    // no readout. Nothing below allocates a widget, so nothing can take the
    // pointer.
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("perf_overlay"),
    ));
    let layout = |text: &str, font: &egui::FontId, color: egui::Color32| {
        ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), color))
    };

    // Three columns, all measured rather than assumed: labels left-aligned in
    // the first, means RIGHT-aligned in the second and peaks in the third, so
    // the digits and the unit line up down the list.
    //
    // The label column is sized from the widest label actually present, not
    // a hardcoded width: a fixed seven characters fits until a row is called
    // "lattice gpu" — eleven characters, with the value column starting
    // underneath it. Measuring cannot drift out of step with the rows.
    const COL_GAP: f32 = 10.0;
    // Between the labels and the means: the peak answers a different question
    // from the cost beside it, and reading it as a second opinion on that cost
    // is the misreading worth designing against.
    let peak_ink = egui::Color32::from_gray(0xB4);
    let head_fps = layout(&format!("{fps:.0} fps"), &head_font, health);
    let head_state = layout(state, &mono, dim);
    // Say which column is which. Two unlabelled columns of milliseconds is a
    // guess, and the wrong guess — taking a peak for a cost — sends you
    // optimizing a stage that is already fast.
    let head_mean = layout("avg", &mono, dim);
    let head_peak = layout("peak", &mono, dim);
    // Indent by depth, so nesting reads without any drawn guides.
    let labels: Vec<_> = rows
        .iter()
        .map(|(depth, label, _, _)| {
            layout(&format!("{:indent$}{label}", "", indent = *depth as usize * 2), &mono, dim)
        })
        .collect();
    let values: Vec<_> = rows.iter().map(|(_, _, v, _)| layout(v, &mono, bright)).collect();
    let peaks: Vec<_> = rows
        .iter()
        .map(|(_, _, _, p)| p.as_ref().map(|p| layout(p, &mono, peak_ink)))
        .collect();

    let label_col = labels.iter().map(|g| g.rect.width()).fold(0.0f32, f32::max);
    let peak_col = peaks
        .iter()
        .flatten()
        .map(|g| g.rect.width())
        .fold(head_peak.rect.width(), f32::max);
    // Sized over the rows that HAVE a peak only. A row without one (memory,
    // voices) spans both number columns instead, so letting "3 held · 1
    // fading" set this width would shove the peaks off the right edge for
    // nothing.
    let mean_col = values
        .iter()
        .zip(&peaks)
        .filter(|(_, p)| p.is_some())
        .map(|(v, _)| v.rect.width())
        .fold(head_mean.rect.width(), f32::max);
    let spanned = values
        .iter()
        .zip(&peaks)
        .filter(|(_, p)| p.is_none())
        .map(|(v, _)| v.rect.width())
        .fold(0.0f32, f32::max);
    // Everything right of the labels: whichever of the two number columns
    // together and the widest spanning row claims more.
    let nums_x = label_col + COL_GAP;
    let mean_right = nums_x + (mean_col + COL_GAP + peak_col).max(spanned) - peak_col - COL_GAP;
    let peak_right = nums_x + (mean_col + COL_GAP + peak_col).max(spanned);

    let mut lines: Vec<Vec<(f32, std::sync::Arc<egui::Galley>)>> = Vec::new();
    lines.push(vec![
        (0.0, head_fps.clone()),
        (head_fps.rect.width() + 4.0, head_state),
    ]);
    lines.push(vec![
        (mean_right - head_mean.rect.width(), head_mean),
        (peak_right - head_peak.rect.width(), head_peak),
    ]);
    for ((label, value), peak) in labels.into_iter().zip(values).zip(peaks) {
        // Right-aligned inside its column, so "4.2" and "12.5" end on the same
        // edge and can be read down the list.
        let mut parts = vec![(0.0, label)];
        match peak {
            Some(peak) => {
                parts.push((mean_right - value.rect.width(), value));
                parts.push((peak_right - peak.rect.width(), peak));
            }
            // Nothing to put in the peak column, so the value takes both and
            // ends on the same right edge — the block still squares off.
            None => parts.push((peak_right - value.rect.width(), value)),
        }
        lines.push(parts);
    }
    // Which build this is — the answer to "did the swap take?", which is a
    // question you have before you trust any number above it.
    //
    // Its OWN line rather than a row in the grid above: the tag is identity,
    // not a measurement, and a long branch name in the value column would
    // widen BOTH columns for every row. And WRAPPED to the width the grid
    // already needs, so it cannot widen the HUD either — the overlay has to
    // fit inside the analyzer pane, which is not always wide, and a branch
    // name is arbitrarily long. A long one costs a second line, where there is
    // room to spare.
    let grid_width = lines
        .iter()
        .map(|parts| parts.iter().map(|(x, g)| x + g.rect.width()).fold(0.0f32, f32::max))
        .fold(0.0f32, f32::max);
    let tag = ctx.fonts_mut(|f| {
        f.layout(format!("build  {BUILD_TAG}"), mono.clone(), dim, grid_width)
    });
    lines.push(vec![(0.0, tag)]);

    const ROW_GAP: f32 = 1.0;
    let width = lines
        .iter()
        .map(|parts| parts.iter().map(|(x, g)| x + g.rect.width()).fold(0.0f32, f32::max))
        .fold(0.0f32, f32::max);
    let height = lines
        .iter()
        .map(|parts| parts.iter().map(|(_, g)| g.rect.height()).fold(0.0f32, f32::max))
        .sum::<f32>()
        + ROW_GAP * lines.len().saturating_sub(1) as f32;

    let margin = egui::vec2(8.0, 6.0);
    // Top-RIGHT of `area`, and clamped to it so a pane too narrow to hold the
    // overlay shows its left edge rather than pushing the numbers off screen.
    //
    // Placed outright rather than anchored. Anchoring existed because a
    // widget-built overlay only learns its own width after laying out, and the
    // right edge has to stay put as the numbers change width underneath it.
    // Measuring the galleys up front settles the size before anything is
    // drawn, so the position is simply known.
    let size = egui::vec2(width, height) + margin * 2.0;
    let origin = egui::pos2(
        (area.right() - OVERLAY_INSET - size.x).max(area.left()),
        area.top() + OVERLAY_INSET,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(origin, size),
        4.0,
        egui::Color32::from_black_alpha(0xC0),
    );

    let mut y = origin.y + margin.y;
    for parts in lines {
        let row_height =
            parts.iter().map(|(_, g)| g.rect.height()).fold(0.0f32, f32::max);
        for (dx, galley) in parts {
            painter.galley(egui::pos2(origin.x + margin.x + dx, y), galley, bright);
        }
        y += row_height + ROW_GAP;
    }
}


/// Resident set size of THIS process in bytes, or 0 when the platform can't
/// report it. Called about once a second (see [`MEM_INTERVAL`]).
#[cfg(target_os = "macos")]
fn rss_bytes() -> u64 {
    // proc_pidinfo(PROC_PIDTASKINFO) fills a proc_taskinfo whose
    // pti_resident_size is the physical footprint — the same "Memory" column
    // Activity Monitor shows. Returns the bytes written, == the struct size on
    // success.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    let written = unsafe {
        libc::proc_pidinfo(
            std::process::id() as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            (&mut info as *mut libc::proc_taskinfo).cast(),
            size,
        )
    };
    if written == size {
        info.pti_resident_size
    } else {
        0
    }
}

#[cfg(target_os = "linux")]
fn rss_bytes() -> u64 {
    // /proc/self/statm reports memory in pages; the second field is resident.
    // The page size is 4096 on every common Linux target, so skip the syscall.
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    statm
        .split_whitespace()
        .nth(1)
        .and_then(|field| field.parse::<u64>().ok())
        .map(|pages| pages * 4096)
        .unwrap_or(0)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn rss_bytes() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `frames` frames costing `cpu_ms` each at 60 Hz, advancing the shell
    /// clock as it goes — the interval is derived from `now`, so the clock
    /// moving IS the measurement.
    fn feed(perf: &mut PerfStats, now: &mut f64, frames: usize, cpu_ms: f32) {
        for _ in 0..frames {
            *now += 1.0 / 60.0;
            perf.record(
                FrameCosts { cpu_ms, ..Default::default() },
                *now,
                Workload::default(),
            );
        }
    }

    /// The basic overlay answers "is something wrong", the breakdown answers
    /// "where did the frame go" — so every row that only serves the second
    /// question waits for `detail`, `tick` and `gpu` included. The breakdown
    /// EXPANDS the headline list rather than replacing it: the headline rows
    /// keep their order and their place, so a glance at one mode transfers
    /// to the other.
    ///
    /// Worth asserting rather than eyeballing: both modes draw a plausible
    /// HUD, and a row leaking back into the basic list is a change nobody
    /// notices until the corner is cluttered again.
    #[test]
    fn the_basic_overlay_is_the_headline_rows_only() {
        let perf = PerfStats::default();
        let label_of = |rows: &[(u8, &'static str, String, Option<String>)]| {
            rows.iter().map(|(_, label, _, _)| *label).collect::<Vec<_>>()
        };

        let basic = label_of(&overlay_rows(&perf, false));
        assert_eq!(
            basic,
            ["frame", "memory", "voices", "nodes"],
            "the basic overlay is the rate, its worst frame, and the workload behind them",
        );

        let detail = label_of(&overlay_rows(&perf, true));
        for row in ["tick", "gpu", "egui", "render", "verts", "spec"] {
            assert!(detail.contains(&row), "the breakdown should still carry `{row}`");
        }
        // The breakdown EXPANDS the headline list: every basic row survives,
        // in order, and — the part a subsequence check alone does not say —
        // each stays where it was, with the new rows spliced INTO the list
        // rather than parked before or after it. Checking only the
        // subsequence let the whole breakdown move to the end of the HUD and
        // still pass, which is exactly the glance that stops transferring.
        assert_eq!(detail.first(), basic.first(), "the breakdown must still open on `frame`");
        let tail = basic.len() - 1;
        assert_eq!(
            detail[detail.len() - tail..],
            basic[1..],
            "the workload rows belong at the foot of both lists",
        );

        // Depth, not just labels: the nesting is what lets a total and its
        // parts be read against each other, and a flattened breakdown draws
        // a plausible list of numbers that has quietly stopped saying what
        // contains what.
        let depth_of = |label: &str| {
            overlay_rows(&perf, true).into_iter().find(|(_, l, _, _)| *l == label).map(|(d, ..)| d)
        };
        for (label, depth) in [("frame", 0), ("tick", 0), ("egui", 1), ("shell", 2), ("prep", 4)] {
            assert_eq!(depth_of(label), Some(depth), "`{label}` sits at depth {depth}");
        }
    }

    /// The readout that would have caught both of the spectrogram's silent
    /// performance bugs: a cache that has stopped absorbing scrolls redraws the
    /// whole heatmap every frame, which is CORRECT and so invisible on screen.
    ///
    /// A rate, not a total. A total climbs by one whenever the layout
    /// legitimately changes — a resize, a palette change — and would read the
    /// same as a cache that has stopped working; what tells them apart is how
    /// fast it climbs.
    #[test]
    fn the_overlay_reports_cache_fallbacks_as_a_rate() {
        let mut perf = PerfStats::default();
        let mut now = 0.0;
        let mut totals = (0u32, 0u32);
        let mut tick = |perf: &mut PerfStats, now: &mut f64, folds: u32, rings: u32| {
            totals = (totals.0 + folds, totals.1 + rings);
            *now += 1.0 / 60.0;
            perf.record(
                FrameCosts {
                    spectrogram_fallbacks: (totals.0, [totals.1, 0, 0, 0, 0, 0]),
                    ..Default::default()
                },
                *now,
                Workload::default(),
            );
        };

        // A healthy build: the caches absorb every frame, so nothing is counted
        // and the rate stays at zero however long it runs.
        for _ in 0..120 {
            tick(&mut perf, &mut now, 0, 0);
        }
        assert_eq!(perf.spec_fallbacks, (0.0, 0.0), "an idle build must read zero");

        // One legitimate re-layout — a resize, say. A total would now read
        // "1 forever"; the rate returns to zero once it is past.
        tick(&mut perf, &mut now, 1, 1);
        for _ in 0..120 {
            tick(&mut perf, &mut now, 0, 0);
        }
        assert_eq!(perf.spec_fallbacks, (0.0, 0.0), "a one-off re-layout must not linger");

        // And the failure this exists to show: a layer falling back on every
        // frame, at 60 Hz.
        for _ in 0..120 {
            tick(&mut perf, &mut now, 1, 1);
        }
        let (folds, rings) = perf.spec_fallbacks;
        assert!((folds - 60.0).abs() < 5.0, "a per-frame refold read as {folds}/s, not ~60");
        assert!((rings - 60.0).abs() < 5.0, "a per-frame restart read as {rings}/s, not ~60");

        // And it names WHERE to look: the reason that accounted for most of the
        // restarts in the interval, which is the difference between "the image
        // changed", "the pane changed" and "the window jumped".
        assert_eq!(perf.spec_restart_reason, crate::panes::spectrogram::Restart::LABELS[0]);
        let mut heavier = (totals.0, [totals.1, totals.1 + 200, 0, 0, 0, 0]);
        for _ in 0..120 {
            heavier = (heavier.0, [heavier.1[0], heavier.1[1] + 1, 0, 0, 0, 0]);
            now += 1.0 / 60.0;
            perf.record(
                FrameCosts { spectrogram_fallbacks: heavier, ..Default::default() },
                now,
                Workload::default(),
            );
        }
        assert_eq!(
            perf.spec_restart_reason,
            crate::panes::spectrogram::Restart::LABELS[1],
            "the dominant reason must follow the counts",
        );
    }

    #[test]
    fn fps_converges_on_the_measured_frame_time() {
        let mut perf = PerfStats::default();
        // Seeded at a plausible 60 Hz so early frames don't read as absurd.
        assert!((perf.fps() - 60.0).abs() < 1.0);
        for i in 1..=500 {
            let now = i as f64 / 30.0;
            perf.record(FrameCosts { cpu_ms: 2.0, ..Default::default() }, now, Workload { animating: true, ..Default::default() });
        }
        assert!((perf.fps() - 30.0).abs() < 0.5, "fps = {}", perf.fps());
    }

    #[test]
    fn records_workload_and_reads_memory() {
        let mut perf = PerfStats::default();
        perf.record(
            FrameCosts { cpu_ms: 1.5, ..Default::default() },
            1.0,
            Workload {
                active_voices: 5,
                held_voices: 3,
                visible_nodes: 49,
                render_scale: 2.0,
                animating: true,
            },
        );
        assert_eq!(perf.workload.active_voices, 5);
        assert_eq!(perf.workload.held_voices, 3);
        assert_eq!(perf.workload.visible_nodes, 49);
        // The first read always fires (last_mem_read starts at -inf). On the
        // platforms with a reader it must return a real footprint; elsewhere
        // 0 ("n/a") is the documented result.
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert!(perf.rss_bytes > 0, "expected a resident-size reading");
    }

    /// The megabyte that moves on every read is noise; the readout answers
    /// "roughly how much, and is it growing", so it steps rather than churns.
    #[test]
    fn the_memory_readout_steps_rather_than_churning() {
        let mb = |n: u64| n * 1024 * 1024;
        // Everything inside one step reads the same, so the digits hold still.
        for bytes in [mb(485), mb(487), mb(492), mb(494)] {
            assert_eq!(memory_readout(bytes), "490 MB", "{bytes} bytes");
        }
        assert_eq!(memory_readout(mb(495)), "500 MB", "and it does step");
        assert_eq!(memory_readout(0), "n/a", "no reading at all");
        // A process too small to round still says something true, not "0 MB".
        assert_eq!(memory_readout(mb(3)), "<10 MB");
    }

    /// A single spike shouldn't jump the number: it eases toward each new
    /// reading, so what reaches the digits is where memory actually sits.
    #[test]
    fn memory_eases_toward_a_new_reading() {
        let mut perf = PerfStats {
            rss_bytes: 400 * 1024 * 1024,
            last_mem_read: 0.0,
            ..Default::default()
        };
        // Force a read whose sample is whatever the platform reports; what is
        // under test is that the stored value MOVES but does not teleport.
        let before = perf.rss_bytes;
        perf.record(FrameCosts { cpu_ms: 1.0, ..Default::default() }, MEM_INTERVAL, Workload::default());
        let after = perf.rss_bytes as f64;
        let sample = super::rss_bytes() as f64;
        if sample > 0.0 && (sample - before as f64).abs() > 1.0 {
            let step = (after - before as f64).abs();
            let jump = (sample - before as f64).abs();
            assert!(step < jump, "the readout teleported: {before} -> {after}");
            assert!(step > 0.0, "the readout never moved at all");
        }
    }

    #[test]
    fn memory_read_is_throttled_to_one_per_interval() {
        let mut perf = PerfStats::default();
        perf.record(FrameCosts { cpu_ms: 1.0, ..Default::default() }, 10.0, Workload::default());
        let first = perf.last_mem_read;
        assert_eq!(first, 10.0);
        // A read less than MEM_INTERVAL later must not refresh the timestamp.
        perf.record(FrameCosts { cpu_ms: 1.0, ..Default::default() }, 10.0 + MEM_INTERVAL / 2.0, Workload::default());
        assert_eq!(perf.last_mem_read, first, "read again too soon");
        // Past the interval, it refreshes.
        perf.record(FrameCosts { cpu_ms: 1.0, ..Default::default() }, 10.0 + MEM_INTERVAL, Workload::default());
        assert_eq!(perf.last_mem_read, 10.0 + MEM_INTERVAL);
    }

    /// What a row prints is the plain mean of the frames inside its window:
    /// every frame weighted alike, and nothing carried in from before it.
    ///
    /// The clock is driven straight to the window boundaries rather than
    /// stepped at some frame rate and assumed to land on them. A latch fires
    /// on whichever frame first crosses [`READOUT_INTERVAL`] past the last
    /// one, so a sequence started at an arbitrary instant gets cut somewhere
    /// in the middle and the assertion ends up over a window nobody chose.
    #[test]
    fn the_printed_value_is_the_windows_arithmetic_mean() {
        let mut perf = PerfStats::default();
        // Closes whatever window was open and opens the one under test at a
        // known instant.
        perf.record(FrameCosts { cpu_ms: 0.0, ..Default::default() }, 0.0, Workload::default());

        // Ten frames well inside the window, then one on the boundary that
        // closes it: eleven in all, and the row prints their plain mean —
        // deliberately not a figure a median or an EMA would also produce.
        let costs = [1.0f32, 3.0, 2.0, 8.0, 1.0, 1.0, 4.0, 2.0, 3.0, 5.0, 6.0];
        let last = costs.len() - 1;
        for (i, cpu_ms) in costs.iter().enumerate() {
            let now = if i == last {
                READOUT_INTERVAL
            } else {
                (i + 1) as f64 * READOUT_INTERVAL / 20.0
            };
            perf.record(
                FrameCosts { cpu_ms: *cpu_ms, ..Default::default() },
                now,
                Workload::default(),
            );
        }
        let expected = costs.iter().sum::<f32>() / costs.len() as f32;
        assert!(
            (perf.ui.shown_mean - expected).abs() < 1e-4,
            "{} vs {expected}",
            perf.ui.shown_mean
        );
    }

    /// The window mean has NO MEMORY: once the window holding a stall has
    /// closed, the row is back to what the frames actually cost.
    ///
    /// This is the property the exponential moving average could not have, and
    /// the reason it went. There, one 40 ms frame lifted every row for the
    /// best part of a second, so "this stage got slower" and "this stage
    /// hiccupped once" printed the same number.
    #[test]
    fn a_spike_leaves_the_mean_once_its_window_closes() {
        let mut perf = PerfStats::default();
        let mut now = 0.0;
        feed(&mut perf, &mut now, 40, 1.0);
        assert!((perf.ui.shown_mean - 1.0).abs() < 0.01, "{}", perf.ui.shown_mean);

        // One catastrophic frame, then a steady 1 ms again.
        feed(&mut perf, &mut now, 1, 100.0);
        feed(&mut perf, &mut now, 40, 1.0);
        assert!(
            (perf.ui.shown_mean - 1.0).abs() < 0.01,
            "the spike is still dragging the mean: {}",
            perf.ui.shown_mean
        );
        // And it is still on screen — in the column that exists for it.
        assert!(perf.ui.shown_peak > 90.0, "the peak lost it: {}", perf.ui.shown_peak);
    }

    /// The peak holds a stall up long enough to be read and then lets go. One
    /// that never expired would be a high-water mark for the session, which
    /// says nothing about now.
    #[test]
    fn the_peak_expires_once_its_windows_roll_past() {
        let mut perf = PerfStats::default();
        let mut now = 0.0;
        feed(&mut perf, &mut now, 1, 100.0);
        feed(&mut perf, &mut now, 30, 1.0); // half a second on
        assert!(perf.ui.shown_peak > 90.0, "gone too soon: {}", perf.ui.shown_peak);

        let quiet = (READOUT_INTERVAL * PEAK_WINDOWS as f64 * 60.0) as usize + 60;
        feed(&mut perf, &mut now, quiet, 1.0);
        assert!(perf.ui.shown_peak < 2.0, "the peak never let go: {}", perf.ui.shown_peak);
    }

    /// The nested rows must still add up, which is exactly what the indented
    /// layout promises: `egui` and `render` are the two halves of `tick`, and
    /// reading a total against its parts is why they sit under it.
    ///
    /// True of the MEAN and of nothing else, which is why the value column is
    /// one. Two stages' worst frames are almost never the same frame, so the
    /// peaks beside them do not sum to anything — nor would medians.
    #[test]
    fn the_nested_means_still_sum_to_their_parent() {
        let mut perf = PerfStats::default();
        let mut now = 0.0;
        // Costs that vary frame to frame, so a constant input cannot make
        // this pass by accident.
        for i in 0..60 {
            now += 1.0 / 60.0;
            let render_ms = 2.0 + (i % 5) as f32;
            let texture_ms = 0.5 + (i % 4) as f32 * 0.25;
            perf.record(
                FrameCosts {
                    tick_ms: render_ms + 1.0 + (i % 3) as f32,
                    render_ms,
                    upload_ms: texture_ms + 1.5,
                    texture_ms,
                    ..Default::default()
                },
                now,
                Workload::default(),
            );
        }
        let (tick, egui, render) =
            (perf.tick.shown_mean, perf.egui.shown_mean, perf.render.shown_mean);
        assert!((egui + render - tick).abs() < 1e-4, "{egui} + {render} != {tick}");
        // Same for the upload's two halves, accumulated the same way.
        assert!((perf.buf_up.shown_mean - 1.5).abs() < 1e-4, "{}", perf.buf_up.shown_mean);
    }

    /// Digits that never hold still get squinted at rather than read. The
    /// window keeps accumulating every frame; what the overlay PRINTS is
    /// latched, so it changes four times a second instead of 144.
    #[test]
    fn the_printed_numbers_hold_between_latches() {
        let mut perf = PerfStats::default();
        perf.record(FrameCosts { cpu_ms: 2.0, ..Default::default() }, 0.0, Workload::default());
        let shown = perf.ui.shown_mean;
        assert_eq!(shown, 2.0, "the opening frame latches at once, not from a placeholder");

        // Frames well inside the interval: they accumulate, the printed value
        // does not budge.
        for i in 1..=10 {
            perf.record(FrameCosts { cpu_ms: 20.0, ..Default::default() }, i as f64 * READOUT_INTERVAL / 20.0, Workload::default());
        }
        assert_eq!(perf.ui.n, 10, "the frames should have accumulated");
        assert_eq!(perf.ui.shown_mean, shown, "the printed value must hold");

        // Past the interval it catches up — to the mean of that window, which
        // holds nothing but the 20s.
        perf.record(FrameCosts { cpu_ms: 20.0, ..Default::default() }, READOUT_INTERVAL, Workload::default());
        assert_eq!(perf.ui.shown_mean, 20.0, "and then it latches");
    }

    /// Latching must not cost resolution: a reading a tenth of a millisecond
    /// away from another has to print differently. (Rounding to a coarse step
    /// was the alternative, and this is the property it gave up.)
    #[test]
    fn latching_keeps_the_tenths() {
        let readout = |ms: f32| format!("{ms:.1} ms");
        assert_ne!(readout(2.7), readout(2.8));
        assert_eq!(readout(2.75), "2.8 ms");
    }

    /// The headline rate is derived from the SAME held frame time as the row
    /// below it, so the two can never disagree on screen.
    #[test]
    fn fps_is_read_off_the_held_frame_time() {
        let mut perf = PerfStats::default();
        for i in 1..=200 {
            perf.record(FrameCosts { cpu_ms: 1.0, ..Default::default() }, i as f64 / 120.0, Workload::default());
        }
        let from_row = 1000.0 / perf.frame_ms.shown_mean;
        assert!((perf.fps() - from_row).abs() < 1e-3, "{} vs {from_row}", perf.fps());
    }

    /// Labels and values must not collide, whatever the rows are called.
    ///
    /// The label column was a hardcoded seven characters until a row named
    /// "lattice gpu" arrived and the values started printing on top of it.
    /// Driving the assertion off the SAME `rows` the overlay builds means a
    /// future row long enough to break the layout fails here instead.
    #[test]
    fn the_value_column_clears_the_longest_label() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx); // real metrics, not egui's fallback
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
        let mut perf = PerfStats::default();
        // A reading in every row, so none of them lays out as a short
        // placeholder and hides the widest case.
        perf.record(
            FrameCosts {
                // Enough of both to lay out at their widest: the row carries
                // two rates, so a build falling back hard is the long case.
                spectrogram_fallbacks: (900, [150; crate::panes::spectrogram::Restart::COUNT]),
                shell_ms: 1.0,
                cpu_ms: 2.0,
                tess_ms: 3.0,
                egui_gpu_ms: 4.0,
                lattice_gpu_ms: 5.0,
                acquire_ms: 6.0,
                tick_ms: 7.0,
                render_ms: 8.0,
                upload_ms: 9.0,
                texture_ms: 8.5,
                prims: 0,
                verts: 0,
                roll_notes: 0,
                prepare_ms: 1.0,
                poll_ms: 0.5,
                ubuf_ms: 1.2,
                write_ms: 0.25,
                scene_ms: 0.25,
                encode_ms: 10.0,
                submit_ms: 11.0,
            },
            1.0,
            Workload { animating: true, ..Default::default() },
        );

        let output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(area), ..Default::default() },
            |ui| draw_overlay(ui.ctx(), area, &perf, true), // detail on: the widest case
        );
        let mut texts: Vec<(egui::Rect, String)> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                    text.galley.text().to_owned(),
                )),
                _ => None,
            })
            .collect();
        texts.sort_by(|a, b| a.0.top().total_cmp(&b.0.top()));
        assert!(texts.len() > 8, "expected a row per reading, got {}", texts.len());

        // Within each row (same top), nothing may start before the previous
        // piece ends.
        for pair in texts.windows(2) {
            let ((a, at), (b, bt)) = (&pair[0], &pair[1]);
            if (a.top() - b.top()).abs() > 0.5 {
                continue; // different rows
            }
            assert!(
                b.left() >= a.right(),
                "{at:?} and {bt:?} overlap: {a:?} then {b:?}",
            );
        }
    }

}
