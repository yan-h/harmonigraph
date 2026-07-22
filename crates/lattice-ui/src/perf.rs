//! The performance overlay: a small corner HUD over the editor showing the
//! frame rate, the GUI's own CPU cost per frame, the process's memory, and the
//! workload driving them — enough to see at a glance whether the plugin is
//! working the machine hard, and to watch the cost settle once the notes stop.
//!
//! Interactive only. [`root_ui`](crate::root_ui) times the frame, folds the
//! numbers in here, and draws the overlay; the offline renderer bypasses
//! `root_ui` entirely, so nothing in this module ever runs on its
//! (deterministic) draw path — no wall-clock read reaches a recorded frame.

/// How quickly the smoothed readouts chase the latest sample. egui already
/// smooths `stable_dt`; this steadies the display a touch more so the numbers
/// are readable rather than flickering every frame.
const SMOOTH: f32 = 0.1;

/// Seconds between memory reads. RSS moves slowly and the read is a syscall,
/// so once a second is plenty and keeps it off the per-frame path.
const MEM_INTERVAL: f64 = 1.0;

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
    /// Resident set size in bytes, refreshed about once a second (0 when the
    /// platform can't report it).
    rss_bytes: u64,
    /// Shell-clock time of the last memory read, to throttle it.
    last_mem_read: f64,
    active_voices: usize,
    held_voices: usize,
    visible_nodes: usize,
    render_scale: f32,
    /// Whether the last frame was repainting continuously (something moving)
    /// rather than idling. The FPS number means different things in each: idle
    /// caps at the ~20 Hz poll by design, so a low idle rate is not a problem.
    animating: bool,
}

impl Default for PerfStats {
    fn default() -> Self {
        PerfStats {
            frame_dt: 1.0 / 60.0,
            cpu_ms: 0.0,
            rss_bytes: 0,
            last_mem_read: f64::NEG_INFINITY,
            active_voices: 0,
            held_voices: 0,
            visible_nodes: 0,
            render_scale: 1.0,
            animating: false,
        }
    }
}

impl PerfStats {
    /// Fold this frame's measurements in. `dt` is egui's stable frame time,
    /// `cpu_ms` the measured dock-build time, `now` the shell clock (for
    /// throttling the memory read).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &mut self,
        dt: f32,
        cpu_ms: f32,
        now: f64,
        active_voices: usize,
        held_voices: usize,
        visible_nodes: usize,
        render_scale: f32,
        animating: bool,
    ) {
        if dt > 0.0 {
            self.frame_dt += (dt - self.frame_dt) * SMOOTH;
        }
        self.cpu_ms += (cpu_ms - self.cpu_ms) * SMOOTH;
        self.active_voices = active_voices;
        self.held_voices = held_voices;
        self.visible_nodes = visible_nodes;
        self.render_scale = render_scale;
        self.animating = animating;
        if now - self.last_mem_read >= MEM_INTERVAL {
            self.rss_bytes = rss_bytes();
            self.last_mem_read = now;
        }
    }

    fn fps(&self) -> f32 {
        if self.frame_dt > 0.0 {
            1.0 / self.frame_dt
        } else {
            0.0
        }
    }
}

/// Draw the overlay in the top-left corner of `area` (the whole editor rect).
/// A floating, non-interactive panel so it never steals clicks from the view
/// under it.
pub(crate) fn draw_overlay(ctx: &egui::Context, area: egui::Rect, perf: &PerfStats) {
    let fps = perf.fps();
    // Only flag a low rate while something is actually animating — an idle
    // editor is meant to drop to the poll rate, so a low idle number is fine.
    let health = if perf.animating && fps < 30.0 {
        egui::Color32::from_rgb(0xE5, 0x7A, 0x5A) // warm red
    } else if perf.animating && fps < 50.0 {
        egui::Color32::from_rgb(0xE0, 0xB0, 0x4A) // amber
    } else {
        egui::Color32::from_rgb(0x7A, 0xC8, 0x8A) // calm green
    };
    let state = if perf.animating { "live" } else { "idle" };

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

    let memory = if perf.rss_bytes > 0 {
        format!("{} MB", perf.rss_bytes / (1024 * 1024))
    } else {
        "n/a".to_string()
    };
    let fading = perf.active_voices.saturating_sub(perf.held_voices);

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
                    row(ui, "frame", format!("{:.1} ms", perf.frame_dt * 1000.0));
                    row(ui, "ui cpu", format!("{:.1} ms", perf.cpu_ms));
                    row(ui, "memory", memory);
                    row(ui, "voices", format!("{} held · {fading} fading", perf.held_voices));
                    row(
                        ui,
                        "nodes",
                        format!("{}  ·  {:.2}× scale", perf.visible_nodes, perf.render_scale),
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
        // Feed a steady 30 Hz; the EMA should chase it down.
        for _ in 0..500 {
            perf.record(1.0 / 30.0, 2.0, 0.0, 0, 0, 0, 1.0, true);
        }
        assert!((perf.fps() - 30.0).abs() < 0.5, "fps = {}", perf.fps());
    }

    #[test]
    fn records_workload_and_reads_memory() {
        let mut perf = PerfStats::default();
        perf.record(1.0 / 60.0, 1.5, 1.0, 5, 3, 49, 2.0, true);
        assert_eq!(perf.active_voices, 5);
        assert_eq!(perf.held_voices, 3);
        assert_eq!(perf.visible_nodes, 49);
        // The first read always fires (last_mem_read starts at -inf). On the
        // platforms with a reader it must return a real footprint; elsewhere
        // 0 ("n/a") is the documented result.
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert!(perf.rss_bytes > 0, "expected a resident-size reading");
    }

    #[test]
    fn memory_read_is_throttled_to_one_per_interval() {
        let mut perf = PerfStats::default();
        perf.record(1.0 / 60.0, 1.0, 10.0, 0, 0, 0, 1.0, false);
        let first = perf.last_mem_read;
        assert_eq!(first, 10.0);
        // A read less than MEM_INTERVAL later must not refresh the timestamp.
        perf.record(1.0 / 60.0, 1.0, 10.0 + MEM_INTERVAL / 2.0, 0, 0, 0, 1.0, false);
        assert_eq!(perf.last_mem_read, first, "read again too soon");
        // Past the interval, it refreshes.
        perf.record(1.0 / 60.0, 1.0, 10.0 + MEM_INTERVAL, 0, 0, 0, 1.0, false);
        assert_eq!(perf.last_mem_read, 10.0 + MEM_INTERVAL);
    }
}
