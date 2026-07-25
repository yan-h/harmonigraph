//! The performance overlay: a small corner HUD over the editor showing the
//! frame rate, the GUI's own CPU cost per frame, the process's memory, and the
//! workload driving them — enough to see at a glance whether the plugin is
//! working the machine hard, and to watch the cost settle once the notes stop.
//!
//! Interactive only. [`root_ui`](crate::root_ui) times the frame, folds the
//! numbers in here, and draws the overlay; the offline renderer bypasses
//! `root_ui` entirely, so nothing in this module ever runs on its
//! (deterministic) draw path — no wall-clock read reaches a recorded frame.

/// Time constant of the smoothed readouts, in seconds: how long they take to
/// chase most of the way to a new level.
///
/// Expressed as a duration rather than a per-frame fraction on purpose. A
/// fixed fraction settles in a fixed number of FRAMES, so the same constant
/// steadies the numbers at 30 fps and lets them flicker at 144 — the readout
/// got twitchier the faster the plugin ran, which is backwards.
const SMOOTH_TAU: f32 = 0.4;

/// How often the frame numbers on screen are allowed to change.
///
/// The smoothed values keep updating every frame; the overlay reads a copy
/// latched this often, so the digits hold still long enough to be read.
///
/// This rather than rounding the values to a coarse step. Rounding trades one
/// kind of churn for a worse one: a value sitting near a step boundary flips
/// between two readings that are a whole step apart, which catches the eye
/// harder than the wobble it replaced — and the resolution is gone for good,
/// which at a 2 ms frame cost means throwing away a quarter of the number.
/// Holding the display keeps every digit and simply stops it flickering.
const READOUT_INTERVAL: f64 = 0.25;

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
    /// The volume the upload had to move, rather than how long it took.
    pub prims: u32,
    pub verts: u32,
    /// Note segments the roll drew through its paint callback. NOT part of
    /// `verts`: the roll owns its instance buffer, so its geometry never
    /// reaches egui's. Four vertices each, against the several hundred a
    /// stroked rounded rect used to cost.
    pub roll_notes: u32,
    /// The lattice callback's own `prepare`, which egui-wgpu runs from inside
    /// `update_buffers` — so it is billed to the buffer uploads.
    pub prepare_ms: f32,
    /// Of that, the `device.poll` the GPU timing needs: what the measurement
    /// costs to take.
    pub poll_ms: f32,
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

/// Rolling performance numbers, updated once per interactive frame. Runtime
/// only — never persisted, and never touched by the offline renderer.
pub struct PerfStats {
    /// Smoothed frame interval in seconds (drives the FPS readout). Seeded to
    /// a plausible 60 Hz so the first frames don't read as absurd.
    frame_dt: f32,
    /// Smoothed GUI CPU time per frame in milliseconds: the wall time spent
    /// building the dock and its panes on this thread. Not GPU time — the 3D
    /// draw is submitted to wgpu and finishes off-thread (see `draw_overlay`).
    cpu_ms: f32,
    /// Smoothed milliseconds spent turning shapes into triangles.
    tess_ms: f32,
    /// Smoothed GPU milliseconds for egui's own render pass — the 2D UI, which
    /// the lattice's timer does not cover.
    egui_gpu_ms: f32,
    /// Smoothed shell work before the UI, and the surface wait after it.
    shell_ms: f32,
    acquire_ms: f32,
    tick_ms: f32,
    render_ms: f32,
    upload_ms: f32,
    texture_ms: f32,
    prims: u32,
    verts: u32,
    roll_notes: u32,
    prepare_ms: f32,
    poll_ms: f32,
    encode_ms: f32,
    submit_ms: f32,
    /// Smoothed resident set size in bytes, refreshed about once a second (0
    /// when the platform can't report it). Smoothed for the same reason the
    /// frame numbers are: this is read as a number, not watched as a trace,
    /// and raw RSS wanders by megabytes between reads as the host and the GPU
    /// driver take and give memory back.
    rss_bytes: u64,
    /// Shell-clock time of the last memory read, to throttle it.
    last_mem_read: f64,
    /// Shell-clock time of the previous frame, for measuring the interval.
    /// `None` before the first one.
    last_frame: Option<f64>,
    /// What the overlay actually prints: `frame_dt` and `cpu_ms` as they
    /// stood at the last latch, held between them (see [`READOUT_INTERVAL`]).
    shown_frame_dt: f32,
    shown_cpu_ms: f32,
    shown_tess_ms: f32,
    shown_egui_gpu_ms: f32,
    shown_shell_ms: f32,
    shown_acquire_ms: f32,
    shown_tick_ms: f32,
    shown_render_ms: f32,
    shown_upload_ms: f32,
    shown_texture_ms: f32,
    shown_prepare_ms: f32,
    shown_poll_ms: f32,
    shown_encode_ms: f32,
    shown_submit_ms: f32,
    /// Shell-clock time of that latch.
    last_readout: f64,
    /// GPU milliseconds for the lattice passes, smoothed and held like the
    /// rest. See [`GpuTime`] for the three states this can be in.
    gpu_ms: f32,
    shown_gpu_ms: f32,
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
        PerfStats {
            frame_dt: 1.0 / 60.0,
            cpu_ms: 0.0,
            tess_ms: 0.0,
            egui_gpu_ms: 0.0,
            shell_ms: 0.0,
            acquire_ms: 0.0,
            tick_ms: 0.0,
            render_ms: 0.0,
            upload_ms: 0.0,
            texture_ms: 0.0,
            prims: 0,
            verts: 0,
            roll_notes: 0,
            prepare_ms: 0.0,
            poll_ms: 0.0,
            encode_ms: 0.0,
            submit_ms: 0.0,
            rss_bytes: 0,
            last_mem_read: f64::NEG_INFINITY,
            last_frame: None,
            shown_frame_dt: 1.0 / 60.0,
            shown_cpu_ms: 0.0,
            shown_tess_ms: 0.0,
            shown_egui_gpu_ms: 0.0,
            shown_shell_ms: 0.0,
            shown_acquire_ms: 0.0,
            shown_tick_ms: 0.0,
            shown_render_ms: 0.0,
            shown_upload_ms: 0.0,
            shown_texture_ms: 0.0,
            shown_prepare_ms: 0.0,
            shown_poll_ms: 0.0,
            shown_encode_ms: 0.0,
            shown_submit_ms: 0.0,
            gpu_ms: 0.0,
            shown_gpu_ms: 0.0,
            gpu_supported: true,
            have_gpu: false,
            last_readout: f64::NEG_INFINITY,
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
            prims,
            verts,
            roll_notes,
            prepare_ms,
            poll_ms,
            encode_ms,
            submit_ms,
        } = costs;
        let dt = self.last_frame.map_or(0.0, |last| (now - last) as f32);
        self.last_frame = Some(now);
        // Convert the time constant into this frame's blend factor, so the
        // readouts settle over SMOOTH_TAU seconds whatever the frame rate.
        // A frame long enough to make this exceed 1 simply lands on the new
        // value, which is what a stall should look like.
        let alpha = (1.0 - (-dt / SMOOTH_TAU).exp()).clamp(0.0, 1.0);
        if dt > 0.0 {
            self.frame_dt += (dt - self.frame_dt) * alpha;
        }
        self.cpu_ms += (cpu_ms - self.cpu_ms) * alpha;
        self.tess_ms += (tess_ms - self.tess_ms) * alpha;
        self.egui_gpu_ms += (egui_gpu_ms - self.egui_gpu_ms) * alpha;
        self.shell_ms += (shell_ms - self.shell_ms) * alpha;
        self.acquire_ms += (acquire_ms - self.acquire_ms) * alpha;
        self.tick_ms += (tick_ms - self.tick_ms) * alpha;
        self.render_ms += (render_ms - self.render_ms) * alpha;
        self.upload_ms += (upload_ms - self.upload_ms) * alpha;
        self.texture_ms += (texture_ms - self.texture_ms) * alpha;
        self.prims = prims;
        self.verts = verts;
        self.roll_notes = roll_notes;
        self.prepare_ms += (prepare_ms - self.prepare_ms) * alpha;
        self.poll_ms += (poll_ms - self.poll_ms) * alpha;
        self.encode_ms += (encode_ms - self.encode_ms) * alpha;
        self.submit_ms += (submit_ms - self.submit_ms) * alpha;
        // Three states, not two: a real reading, "the device can't", and
        // "none has landed yet". Collapsing the last two into one "n/a" made
        // a wiring bug and an unsupported GPU look identical, which is
        // exactly the question the row exists to answer.
        match gpu_ms.to_bits() {
            lattice_render::GPU_TIME_UNSUPPORTED => self.gpu_supported = false,
            // Still waiting for the first readback; leave the row saying so.
            lattice_render::GPU_TIME_PENDING => {}
            // Anything else is a real reading, INCLUDING 0.0 — seeded rather
            // than eased up from nothing, so the GPU doesn't appear to warm
            // up over the first second of every session.
            _ => {
                self.have_gpu = true;
                self.gpu_ms = if self.have_gpu && self.gpu_ms > 0.0 {
                    self.gpu_ms + (gpu_ms - self.gpu_ms) * alpha
                } else {
                    gpu_ms
                };
            }
        }
        // Latch what the overlay prints. Seeded on the first frame rather
        // than eased up from the defaults, so the HUD opens showing the real
        // numbers instead of a quarter second of placeholder.
        if now - self.last_readout >= READOUT_INTERVAL {
            self.shown_frame_dt = self.frame_dt;
            self.shown_cpu_ms = self.cpu_ms;
            self.shown_tess_ms = self.tess_ms;
            self.shown_egui_gpu_ms = self.egui_gpu_ms;
            self.shown_shell_ms = self.shell_ms;
            self.shown_acquire_ms = self.acquire_ms;
            self.shown_tick_ms = self.tick_ms;
            self.shown_render_ms = self.render_ms;
            self.shown_upload_ms = self.upload_ms;
            self.shown_texture_ms = self.texture_ms;
            self.shown_prepare_ms = self.prepare_ms;
            self.shown_poll_ms = self.poll_ms;
            self.shown_encode_ms = self.encode_ms;
            self.shown_submit_ms = self.submit_ms;
            self.shown_gpu_ms = self.gpu_ms;
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

    /// The frame rate as printed: derived from the held frame time, so the
    /// headline number holds still with the rows under it rather than
    /// counting off every frame on its own.
    fn fps(&self) -> f32 {
        if self.shown_frame_dt > 0.0 {
            1.0 / self.shown_frame_dt
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

    let fading = perf.workload.active_voices.saturating_sub(perf.workload.held_voices);
    let ms = |v: f32| format!("{v:.1} ms");
    // Depth, label, value. The nesting is the point: every indented row is a
    // PART of the one above it, so a total and its components can be read
    // against each other instead of held in your head. Working out where a
    // frame went meant repeatedly discovering that a cost sat between two
    // readings; the shape of the list now says what contains what.
    let mut rows: Vec<(u8, &str, String)> = vec![
        (0, "frame", ms(perf.shown_frame_dt * 1000.0)),
        (0, "tick", ms(perf.shown_tick_ms)),
    ];
    if detail {
        let egui_ms = (perf.shown_tick_ms - perf.shown_render_ms).max(0.0);
        let buf_up = (perf.shown_upload_ms - perf.shown_texture_ms).max(0.0);
        rows.extend([
            (1, "egui", ms(egui_ms)),
            (2, "shell", ms(perf.shown_shell_ms)),
            (2, "ui", ms(perf.shown_cpu_ms)),
            (1, "render", ms(perf.shown_render_ms)),
            (2, "tess", ms(perf.shown_tess_ms)),
            (2, "tex up", ms(perf.shown_texture_ms)),
            (2, "buf up", ms(buf_up)),
            (3, "prep", ms(perf.shown_prepare_ms)),
            (3, "poll", ms(perf.shown_poll_ms)),
            (2, "wait", ms(perf.shown_acquire_ms)),
            (2, "encode", ms(perf.shown_encode_ms)),
            (2, "submit", ms(perf.shown_submit_ms)),
        ]);
    }
    rows.push((0, "gpu", {
        // Both passes on one line at the top level: they run alongside the CPU
        // stages rather than inside any of them, so nesting either under
        // `tick` would be a lie about what contains what.
        let lattice = if !perf.gpu_supported {
            "n/a".to_owned()
        } else if perf.have_gpu {
            format!("{:.1}", perf.shown_gpu_ms)
        } else {
            "—".to_owned()
        };
        format!("{:.1} ui · {lattice} 3d", perf.shown_egui_gpu_ms)
    }));
    if detail {
        rows.push((0, "verts", format!("{}k in {} prims", perf.verts / 1000, perf.prims)));
        // The roll's geometry, which `verts` can no longer see: it goes to the
        // GPU as instances on the roll's own buffer, four vertices a note.
        rows.push((1, "roll", format!("{} notes", perf.roll_notes)));
    }
    rows.extend([
        (0, "memory", memory_readout(perf.rss_bytes)),
        (
            0,
            "voices",
            format!("{} held · {fading} fading", perf.workload.held_voices),
        ),
        (
            0,
            "nodes",
            format!(
                "{}  ·  {:.2}× scale",
                perf.workload.visible_nodes, perf.workload.render_scale
            ),
        ),
    ]);

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

    // Two columns, both measured rather than assumed: labels left-aligned in
    // the first, values RIGHT-aligned in the second so the units line up.
    //
    // The label column used to be a hardcoded seven characters, which was
    // fine until a row was called "lattice gpu" — eleven characters, and the
    // value column started underneath it. Sizing from the widest label
    // actually present cannot drift out of step with the rows again.
    const COL_GAP: f32 = 10.0;
    let head_fps = layout(&format!("{fps:.0} fps"), &head_font, health);
    let head_state = layout(state, &mono, dim);
    // Indent by depth, so nesting reads without any drawn guides.
    let labels: Vec<_> = rows
        .iter()
        .map(|(depth, label, _)| {
            layout(&format!("{:indent$}{label}", "", indent = *depth as usize * 2), &mono, dim)
        })
        .collect();
    let values: Vec<_> = rows.iter().map(|(_, _, v)| layout(v, &mono, bright)).collect();
    let widest = |gs: &[std::sync::Arc<egui::Galley>]| {
        gs.iter().map(|g| g.rect.width()).fold(0.0f32, f32::max)
    };
    let (label_col, value_col) = (widest(&labels), widest(&values));

    let mut lines: Vec<Vec<(f32, std::sync::Arc<egui::Galley>)>> = Vec::new();
    lines.push(vec![
        (0.0, head_fps.clone()),
        (head_fps.rect.width() + 4.0, head_state),
    ]);
    for (label, value) in labels.into_iter().zip(values) {
        // Right-aligned inside the value column, so "4.2 ms" and "12.5 ms"
        // end on the same edge and can be read down the list.
        let x = label_col + COL_GAP + (value_col - value.rect.width());
        lines.push(vec![(0.0, label), (x, value)]);
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

    #[test]
    fn fps_converges_on_the_smoothed_frame_time() {
        let mut perf = PerfStats::default();
        // Seeded at a plausible 60 Hz so early frames don't read as absurd.
        assert!((perf.fps() - 60.0).abs() < 1.0);
        // Feed a steady 30 Hz on the shell clock; the EMA should chase it
        // down. The interval is derived from `now`, so the clock must advance
        // — that IS the measurement.
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

    /// The readouts must settle over the same DURATION at any frame rate.
    /// A per-frame blend factor settles in a fixed number of FRAMES instead,
    /// so one constant steadied 30 fps while letting 144 fps flicker.
    #[test]
    fn smoothing_settles_over_the_same_time_at_any_rate() {
        let settle_after_one_tau = |rate: f64| {
            let mut perf = PerfStats::default();
            // Seed the clock so every measured step is a full 1/rate.
            perf.record(FrameCosts { cpu_ms: 0.0, ..Default::default() }, 0.0, Workload::default());
            let frames = (rate * SMOOTH_TAU as f64).round() as usize;
            for i in 1..=frames {
                perf.record(FrameCosts { cpu_ms: 10.0, ..Default::default() }, i as f64 / rate, Workload::default());
            }
            perf.cpu_ms
        };
        // One time constant is ~63% of the way to the new level, whatever
        // the frame rate.
        for rate in [30.0, 60.0, 144.0] {
            let settled = settle_after_one_tau(rate);
            assert!((settled - 6.32).abs() < 0.2, "{rate} fps settled to {settled}");
        }
    }

    /// Digits that never hold still get squinted at rather than read. The
    /// values keep tracking every frame; what the overlay PRINTS is latched,
    /// so it changes a few times a second instead of 144.
    #[test]
    fn the_printed_numbers_hold_between_latches() {
        let mut perf = PerfStats::default();
        perf.record(FrameCosts { cpu_ms: 2.0, ..Default::default() }, 0.0, Workload::default());
        let shown = perf.shown_cpu_ms;

        // Frames well inside the interval: the live value moves, the printed
        // one does not.
        for i in 1..=10 {
            perf.record(FrameCosts { cpu_ms: 20.0, ..Default::default() }, i as f64 * READOUT_INTERVAL / 20.0, Workload::default());
        }
        assert!(perf.cpu_ms > shown, "the live value should have moved");
        assert_eq!(perf.shown_cpu_ms, shown, "the printed value must hold");

        // Past the interval it catches up.
        perf.record(FrameCosts { cpu_ms: 20.0, ..Default::default() }, READOUT_INTERVAL, Workload::default());
        assert_eq!(perf.shown_cpu_ms, perf.cpu_ms, "and then it latches");
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
        let from_row = 1000.0 / (perf.shown_frame_dt * 1000.0);
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
