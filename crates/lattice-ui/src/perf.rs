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
    /// Shell-clock time of that latch.
    last_readout: f64,
    /// GPU milliseconds for the lattice passes, smoothed and held like the
    /// rest. See [`GpuTime`] for the three states this can be in.
    gpu_ms: f32,
    shown_gpu_ms: f32,
    /// Whether the device ever said it could measure GPU time at all.
    gpu_supported: bool,
    /// This frame's workload (voice counts, visible nodes, render scale,
    /// whether it was animating).
    workload: Workload,
}

impl Default for PerfStats {
    fn default() -> Self {
        PerfStats {
            frame_dt: 1.0 / 60.0,
            cpu_ms: 0.0,
            rss_bytes: 0,
            last_mem_read: f64::NEG_INFINITY,
            last_frame: None,
            shown_frame_dt: 1.0 / 60.0,
            shown_cpu_ms: 0.0,
            gpu_ms: 0.0,
            shown_gpu_ms: 0.0,
            gpu_supported: true,
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
    pub(crate) fn record(&mut self, cpu_ms: f32, gpu_ms: f32, now: f64, workload: Workload) {
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
        // Three states, not two: a real reading, "the device can't", and
        // "none has landed yet". Collapsing the last two into one "n/a" made
        // a wiring bug and an unsupported GPU look identical, which is
        // exactly the question the row exists to answer.
        if gpu_ms.to_bits() == lattice_render::GPU_TIME_UNSUPPORTED {
            self.gpu_supported = false;
        } else if gpu_ms > 0.0 {
            self.gpu_ms = if self.gpu_ms > 0.0 {
                self.gpu_ms + (gpu_ms - self.gpu_ms) * alpha
            } else {
                gpu_ms
            };
        }
        // Latch what the overlay prints. Seeded on the first frame rather
        // than eased up from the defaults, so the HUD opens showing the real
        // numbers instead of a quarter second of placeholder.
        if now - self.last_readout >= READOUT_INTERVAL {
            self.shown_frame_dt = self.frame_dt;
            self.shown_cpu_ms = self.cpu_ms;
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

/// Draw the overlay in the top-left corner of `area` (the whole editor rect).
/// A floating, non-interactive panel so it never steals clicks from the view
/// under it.
pub(crate) fn draw_overlay(ctx: &egui::Context, area: egui::Rect, perf: &PerfStats) {
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
    // Right-pad labels to one column so the values line up.
    let row = |ui: &mut egui::Ui, label: &str, value: String| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(egui::RichText::new(format!("{label:<7}")).color(dim).font(mono.clone()));
            ui.label(egui::RichText::new(value).color(bright).font(mono.clone()));
        });
    };

    let memory = memory_readout(perf.rss_bytes);
    let fading = perf.workload.active_voices.saturating_sub(perf.workload.held_voices);

    egui::Area::new(egui::Id::new("perf_overlay"))
        .order(egui::Order::Foreground)
        .fixed_pos(area.left_top() + egui::vec2(8.0, 8.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::from_black_alpha(0xC0))
                .inner_margin(egui::Margin::symmetric(8, 6))
                .corner_radius(egui::CornerRadius::same(4))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(
                            egui::RichText::new(format!("{fps:.0} fps"))
                                .color(health)
                                .font(egui::FontId::monospace(12.0))
                                .strong(),
                        );
                        ui.label(egui::RichText::new(state).color(dim).font(mono.clone()));
                    });
                    row(ui, "frame", format!("{:.1} ms", perf.shown_frame_dt * 1000.0));
                    row(ui, "ui cpu", format!("{:.1} ms", perf.shown_cpu_ms));
                    // "n/a" rather than 0.0 where the GPU won't report, the
                    // same answer the memory row gives on a platform that
                    // won't say — a zero would read as "free".
                    row(
                        ui,
                        "gpu",
                        if !perf.gpu_supported {
                            "n/a (no timestamps)".to_owned()
                        } else if perf.shown_gpu_ms > 0.0 {
                            format!("{:.1} ms", perf.shown_gpu_ms)
                        } else {
                            "measuring...".to_owned()
                        },
                    );
                    row(ui, "memory", memory);
                    row(
                        ui,
                        "voices",
                        format!("{} held · {fading} fading", perf.workload.held_voices),
                    );
                    row(
                        ui,
                        "nodes",
                        format!(
                            "{}  ·  {:.2}× scale",
                            perf.workload.visible_nodes, perf.workload.render_scale
                        ),
                    );
                });
        });
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
            perf.record(2.0, 0.0, now, Workload { animating: true, ..Default::default() });
        }
        assert!((perf.fps() - 30.0).abs() < 0.5, "fps = {}", perf.fps());
    }

    #[test]
    fn records_workload_and_reads_memory() {
        let mut perf = PerfStats::default();
        perf.record(
            1.5,
            0.0,
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
        perf.record(1.0, 0.0, MEM_INTERVAL, Workload::default());
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
        perf.record(1.0, 0.0, 10.0, Workload::default());
        let first = perf.last_mem_read;
        assert_eq!(first, 10.0);
        // A read less than MEM_INTERVAL later must not refresh the timestamp.
        perf.record(1.0, 0.0, 10.0 + MEM_INTERVAL / 2.0, Workload::default());
        assert_eq!(perf.last_mem_read, first, "read again too soon");
        // Past the interval, it refreshes.
        perf.record(1.0, 0.0, 10.0 + MEM_INTERVAL, Workload::default());
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
            perf.record(0.0, 0.0, 0.0, Workload::default());
            let frames = (rate * SMOOTH_TAU as f64).round() as usize;
            for i in 1..=frames {
                perf.record(10.0, 0.0, i as f64 / rate, Workload::default());
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
        perf.record(2.0, 0.0, 0.0, Workload::default());
        let shown = perf.shown_cpu_ms;

        // Frames well inside the interval: the live value moves, the printed
        // one does not.
        for i in 1..=10 {
            perf.record(20.0, 0.0, i as f64 * READOUT_INTERVAL / 20.0, Workload::default());
        }
        assert!(perf.cpu_ms > shown, "the live value should have moved");
        assert_eq!(perf.shown_cpu_ms, shown, "the printed value must hold");

        // Past the interval it catches up.
        perf.record(20.0, 0.0, READOUT_INTERVAL, Workload::default());
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
            perf.record(1.0, 0.0, i as f64 / 120.0, Workload::default());
        }
        let from_row = 1000.0 / (perf.shown_frame_dt * 1000.0);
        assert!((perf.fps() - from_row).abs() < 1e-3, "{} vs {from_row}", perf.fps());
    }
}
