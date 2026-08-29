//! The performance overlay's instrumentation: what a frame cost, what the
//! workload behind it was, and the averaging that turns both into numbers
//! steady enough to read. The picture is the UI crate's half (`perf::draw_overlay`);
//! everything here computes, and none of it needs a font or a painter.
//!
//! Its own crate rather than a module of `harmonigraph-ui`, on three grounds
//! that point the same way. Two items here are the whole reason a pane shell
//! otherwise declares a platform syscall dependency and carries a build
//! script: the `libc` that [`rss_bytes`] needs on macOS, and the `build.rs`
//! that stamps [`BUILD_TAG`]. Neither is a BUILD cost that moving it removes,
//! and reading it that way is the mistake worth heading off — `libc` reaches
//! that crate transitively through wgpu and parking_lot whatever its manifest
//! says, and a stamp keyed on `HEAD` re-links on every commit wherever it
//! lives, taking its dependents with it. What moves is which crate ANSWERS
//! for them, and the crate that draws panes is the wrong one to ask about
//! `proc_pidinfo`. [`ShellTimings`] is a contract between a windowed shell and
//! this model, and a shell reaching it through the UI crate's exports says the
//! UI owns a measurement it never reads. And the model/drawing seam is already
//! cut exactly where a crate boundary goes, so drawing one there costs
//! nothing.
//!
//! Interactive only. `root_ui` times the frame, folds the numbers in through
//! [`PerfStats::record`], and draws the overlay; the offline renderer bypasses
//! `root_ui` entirely, so nothing here ever runs on its (deterministic) draw
//! path — no wall-clock read reaches a recorded frame.

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

/// What the SHELL measures about the previous frame, filled in before it
/// calls the UI's `root_ui`.
///
/// One struct on the UI's `SharedState` rather than a field per
/// reading, because none of it is state the UI acts on: no pane reads a
/// millisecond, and the only consumer is `FrameCosts::assemble` one call
/// later. Kept apart, the readings were thirteen `pub` fields on the type
/// every pane borrows, which says they are part of what the UI IS. They are
/// instrumentation passing through it.
///
/// A shell fills in what it can measure and leaves the rest at zero — the
/// standalone harness's eframe loop is not ours to instrument, so its
/// readings are the ones the UI takes for itself. Zero therefore means "not
/// measured here" as much as "free", which is why the overlay is a plugin
/// tool.
#[derive(Clone, Copy, Default)]
pub struct ShellTimings {
    /// Milliseconds the shell spent tessellating egui's shapes.
    ///
    /// Its own reading rather than part of the frame's CPU time because it is
    /// not the same work: `ui cpu` covers building the UI, which only APPENDS
    /// shapes, and this covers turning those shapes into triangles afterwards.
    /// A cost can be entirely in one and invisible in the other.
    pub tess_ms: f32,
    /// Milliseconds the GPU spent on egui's own render pass.
    ///
    /// Disjoint from the lattice's `gpu_ms`, which brackets only its own
    /// passes: between them they cover the frame's GPU work, and the two were
    /// separated because the lattice turned out to be the cheap half.
    pub egui_gpu_ms: f32,
    /// Milliseconds the shell spent on its own per-frame work before the UI
    /// ran — draining the event rings and reconciling the take.
    ///
    /// Separate from the frame's CPU time because that starts at the dock
    /// build: this stretch scales with events ARRIVING rather than with what
    /// is drawn, and there was no reading it could show up in.
    pub shell_ms: f32,
    /// Milliseconds blocked acquiring the surface — the vsync wait. Large
    /// here with every cost small means the frame is early, not slow.
    pub acquire_ms: f32,
    /// Milliseconds the frame callback took end to end.
    ///
    /// The other readings are stages of it. This is the total, and against the
    /// interval between frames it answers what no stage can: whether a long
    /// frame was SLOW, or just late being asked for.
    pub tick_ms: f32,
    /// Milliseconds of that callback spent inside the renderer. `tick_ms`
    /// minus this is the egui half — the UI closure plus egui's own
    /// end-of-pass work — so the two bracket the whole frame between them.
    pub render_ms: f32,
    /// The renderer's stages. `upload_ms` also covers paint callbacks'
    /// `prepare`, so the lattice's own buffer writes are inside it.
    pub upload_ms: f32,
    /// Of that, `update_buffers` itself. The difference is the command-encoder
    /// creation, the renderer's write lock and the MSAA resize, which the
    /// upload reading also spans.
    pub ubuf_ms: f32,
    /// Of the uploads, the TEXTURE half — the rest is buffer uploads, and
    /// with them the paint callbacks' `prepare`.
    pub texture_ms: f32,
    /// How many primitives and vertices the frame uploaded — the volume
    /// behind the upload cost, rather than another duration.
    pub prims: u32,
    pub verts: u32,
    pub encode_ms: f32,
    pub submit_ms: f32,
}

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
    /// re-aggregations of the window, and full uploads of the slab grid.
    /// CUMULATIVE, so the readout below can difference them into a rate without
    /// a dropped frame losing an event.
    pub spectrogram_fallbacks: (u32, u32),
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
pub struct Workload {
    pub active_voices: usize,
    pub held_voices: usize,
    pub visible_nodes: usize,
    pub render_scale: f32,
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
    pub animating: bool,
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
/// A struct per metric rather than parallel lists of `x_ms` and `shown_x_ms`
/// values. The overlay tracks twenty costs, and with two flat lists every new
/// row means touching both of them in step, which nothing but attention
/// enforces.
#[derive(Clone, Copy, Default)]
pub struct Window {
    /// This window so far: running total, frames counted, and the largest
    /// single frame in it.
    sum: f32,
    n: u32,
    max: f32,
    /// The last [`PEAK_WINDOWS`] closed windows' maxima, by latch slot.
    recent_max: [f32; PEAK_WINDOWS],
    /// What the overlay prints: the last closed window's mean, and the largest
    /// frame anywhere in `recent_max`. The two readings that leave this crate;
    /// what they are accumulated FROM stays in here, because a half-closed
    /// window is not a number anything should be printing.
    pub shown_mean: f32,
    pub shown_peak: f32,
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

/// Every stage the overlay averages, in the order the breakdown prints them.
///
/// The discriminant is the index into [`STAGES`], which describes a stage, and
/// into [`PerfStats::windows`], which measures it — so naming a stage here is
/// what ties the two together, and neither can hold a row the other does not.
///
/// This is the list of what is MEASURED; [`STAGES`] says how each one reaches
/// the screen — which for three of them is by hand rather than as a row of
/// the breakdown.
#[derive(Clone, Copy)]
pub enum Stage {
    /// The frame interval, in MILLISECONDS — which also drives the FPS
    /// readout. Held in the same unit as every other row so one shared helper
    /// can format both number columns.
    Frame,
    /// The frame callback end to end. `egui` and `render` are its two halves
    /// by construction, so those three always add up; the rows under THOSE do
    /// not have to, and what is missing there is work nothing measures yet.
    Tick,
    /// The egui half of the callback — the UI closure plus egui's own
    /// end-of-pass work, which is the part of it nothing below measures.
    Egui,
    /// Shell work before the UI ran: draining the event rings.
    Shell,
    /// Building the dock and its panes. Wall time on this thread, not GPU
    /// time — the 3D draw is submitted to wgpu and finishes off-thread.
    Ui,
    /// The renderer half of the callback.
    Render,
    /// Turning shapes into triangles.
    Tess,
    /// The texture half of the uploads.
    Texture,
    /// Everything else in the uploads: buffer writes, and with them the paint
    /// callbacks' `prepare`.
    BufUp,
    /// `update_buffers` itself.
    Ubuf,
    /// The lattice callback's own `prepare`, which egui-wgpu runs from inside
    /// `update_buffers` — so it is billed to the buffer uploads.
    Prepare,
    /// Of that, the `device.poll` the GPU timing needs: what the measurement
    /// costs to take.
    Poll,
    /// Of that, staging the frame: offscreen sizing and the three buffer
    /// writes. Worth telling apart from `scene` — a cost that tracks the node
    /// count is staging and one that does not is the encoder, and the two
    /// have no fix in common.
    Write,
    /// Of that, encoding the scene pass and the bloom chain. Despite sitting
    /// under a row called "buf up", this is the frame's largest single piece
    /// of CPU work whenever the lattice is on screen.
    Scene,
    /// What the upload spans AROUND `update_buffers`, which is not nothing.
    Around,
    /// Blocked acquiring the surface: the vsync wait, which is not work.
    Acquire,
    /// Encoding egui's draw calls.
    Encode,
    /// Finish, submit and present.
    Submit,
    /// GPU milliseconds for egui's own render pass: the 2D UI, which the
    /// lattice's timer does not cover.
    EguiGpu,
    /// GPU milliseconds for the lattice's own passes — the 3D scene and its
    /// bloom chain. Only takes a sample on the frames a readback lands.
    Gpu,
}

impl Stage {
    /// How many there are, so the table and the window array size themselves
    /// from the enum rather than from a number someone has to remember to
    /// bump.
    ///
    /// The match below is exhaustive, so a variant added ANYWHERE — not only
    /// after `Gpu` — fails to compile until it is named there, which is what
    /// `Gpu as usize + 1` could never promise. What the match alone does not
    /// enforce is the array on the last line staying in step with it; that
    /// still wants the same per-variant care [`STAGES`] below already asks
    /// for, and a variant named in one but not the other is what STAGES's
    /// own array-length check catches, not this one.
    pub const COUNT: usize = {
        use Stage::*;
        // Exhaustive, and the compiler checks it. The arms are `()` because
        // what is wanted is the coverage error, not the value — a const fn
        // cannot build the array itself.
        const fn covered(s: Stage) {
            match s {
                Frame | Tick | Egui | Shell | Ui | Render | Tess | Texture | BufUp | Ubuf
                | Prepare | Poll | Write | Scene | Around | Acquire | Encode | Submit | EguiGpu
                | Gpu => (),
            }
        }
        covered(Gpu);
        [
            Frame, Tick, Egui, Shell, Ui, Render, Tess, Texture, BufUp, Ubuf, Prepare, Poll, Write,
            Scene, Around, Acquire, Encode, Submit, EguiGpu, Gpu,
        ]
        .len()
    };
}

/// What the overlay needs to know about one [`Stage`]: where it sits in the
/// breakdown, what it is called there, and what one frame contributes to it.
///
/// One table rather than the same twenty names written out in a struct, a
/// default, an accumulate, a latch and a print. Spread over five lists, a row
/// left out of one of them still compiles and still draws — it simply holds
/// whatever it last showed, which is the failure that reads as a plausible
/// number. Here, adding a stage is a variant and a line, and the compiler
/// will not let the two of them disagree.
pub struct StageInfo {
    /// Which stage this describes, and by construction its own index in
    /// [`STAGES`] — see the check under the table.
    ///
    /// Private, like `sample` below and for the same reason: that check is the
    /// only thing that reads it, and a caller wanting one stage's entry
    /// indexes [`STAGES`] with the stage it already holds.
    stage: Stage,
    /// How deep the breakdown indents it.
    pub depth: u8,
    /// What the row is called. `egui gpu` is the one stage whose label never
    /// reaches the screen: only its number does, inside `gpu`'s row.
    pub label: &'static str,
    /// Whether the breakdown prints a row of its own for it, at `depth`.
    /// False for the three the overlay places by hand.
    pub breakdown: bool,
    /// This frame's value, read or computed from the frame's costs. `None`
    /// where [`PerfStats::record`] feeds the window itself.
    ///
    /// Private, unlike the three the overlay prints: nothing outside this
    /// crate draws a sample, and a table only this crate can build is a table
    /// only this crate can get wrong.
    sample: Option<fn(&FrameCosts) -> f32>,
}

/// A stage the breakdown prints a row for, read straight out of the frame's
/// costs or computed from them. The ordinary case, and the whole of what
/// adding a stage takes.
const fn measured(
    stage: Stage,
    depth: u8,
    label: &'static str,
    sample: fn(&FrameCosts) -> f32,
) -> StageInfo {
    StageInfo { stage, depth, label, breakdown: true, sample: Some(sample) }
}

/// A stage the overlay places by hand instead, because where it prints
/// does not follow from the table — and, where `sample` is `None`, one
/// [`PerfStats::record`] feeds by hand as well.
///
/// Three of them, for three different reasons. `frame` heads BOTH lists, so
/// the breakdown cannot own it, and only `record` knows whether a frame had a
/// predecessor to measure an interval against. The GPU pair share one printed
/// line, so `egui gpu`'s label and depth are never drawn, only its number.
/// And `gpu` is fed by hand because its sentinel decides more than a value:
/// the same reading is what sets `gpu_supported` and `have_gpu`, and two of
/// its three answers are no sample at all — none of which a
/// `fn(&FrameCosts) -> f32` can say.
const fn by_hand(
    stage: Stage,
    depth: u8,
    label: &'static str,
    sample: Option<fn(&FrameCosts) -> f32>,
) -> StageInfo {
    StageInfo { stage, depth, label, breakdown: false, sample }
}

/// Every stage: the depth it prints at, the label it prints under, and where
/// its per-frame value comes from. In the order the breakdown reads, which is
/// also the order [`PerfStats::windows`] holds and latches them in.
///
/// The nesting is the point: every indented row is a PART of the one above
/// it, so a total and its components can be read against each other instead
/// of held in your head. Working out where a frame went means repeatedly
/// discovering that a cost sits between two readings; the shape of this list
/// is what says what contains what.
///
/// Three of the values are ARITHMETIC over the readings rather than readings
/// of their own. They are accumulated per frame, as a stage in their own
/// right, rather than subtracted from the printed means — because the maximum
/// of a difference is not the difference of the maxima, and the peak column
/// would be quietly wrong.
///
/// A cost that reaches [`FrameCosts`] and no stage here reads is caught rather
/// than quietly dropped, by `every_frame_cost_reaches_a_stage` below, which
/// reads this file. `dead_code` cannot say it: the readings are this crate's
/// public API, and a `pub` field counts as read whether or not anything reads
/// it. Wiring a new reading into the overlay is this table and nothing else.
pub const STAGES: [StageInfo; Stage::COUNT] = [
    by_hand(Stage::Frame, 0, "frame", None),
    measured(Stage::Tick, 0, "tick", |c| c.tick_ms),
    // Clamped at zero because the two readings are of independently timed
    // nested spans, and measurement noise can put the inner one a hair above
    // the outer. Same for the two upload differences below.
    measured(Stage::Egui, 1, "egui", |c| (c.tick_ms - c.render_ms).max(0.0)),
    measured(Stage::Shell, 2, "shell", |c| c.shell_ms),
    measured(Stage::Ui, 2, "ui", |c| c.cpu_ms),
    measured(Stage::Render, 1, "render", |c| c.render_ms),
    measured(Stage::Tess, 2, "tess", |c| c.tess_ms),
    measured(Stage::Texture, 2, "tex up", |c| c.texture_ms),
    measured(Stage::BufUp, 2, "buf up", |c| (c.upload_ms - c.texture_ms).max(0.0)),
    measured(Stage::Ubuf, 3, "ubuf", |c| c.ubuf_ms),
    measured(Stage::Prepare, 4, "prep", |c| c.prepare_ms),
    measured(Stage::Poll, 4, "poll", |c| c.poll_ms),
    measured(Stage::Write, 4, "write", |c| c.write_ms),
    measured(Stage::Scene, 4, "scene", |c| c.scene_ms),
    // What the upload spans outside `update_buffers`: creating the command
    // encoder, taking the renderer's write lock, and the MSAA resize.
    measured(Stage::Around, 3, "around", |c| (c.upload_ms - c.texture_ms - c.ubuf_ms).max(0.0)),
    measured(Stage::Acquire, 2, "wait", |c| c.acquire_ms),
    measured(Stage::Encode, 2, "encode", |c| c.encode_ms),
    measured(Stage::Submit, 2, "submit", |c| c.submit_ms),
    // The GPU pair, at the TOP level: they run alongside the CPU stages rather
    // than inside any of them, so nesting either under `tick` would be a lie
    // about what contains what. They share one printed line, which is `gpu`'s,
    // so it is `gpu`'s depth that the overlay reads.
    by_hand(Stage::EguiGpu, 0, "egui gpu", Some(|c| c.egui_gpu_ms)),
    by_hand(Stage::Gpu, 0, "gpu", None),
];

// Every entry sits at its own stage's index, which is what makes
// `STAGES[stage as usize]` describe that stage and `windows[stage as usize]`
// measure it. Checked rather than trusted: the enum and the table are written
// in the same order by hand, and reordering one alone would silently relabel
// every row past the edit — a HUD of plausible numbers against the wrong
// names. A compile error instead.
const _: () = {
    let mut i = 0;
    while i < STAGES.len() {
        assert!(STAGES[i].stage as usize == i);
        i += 1;
    }
};

/// Rolling performance numbers, updated once per interactive frame. Runtime
/// only — never persisted, and never touched by the offline renderer.
///
/// What the overlay PRINTS is `pub`; what a printed number is accumulated from
/// is not. The bookkeeping below — the peak slot, the three shell-clock
/// timestamps, the totals a rate is differenced from — is a set of fields that
/// only mean anything to each other, and a caller writing one of them mid-run
/// would move a number on screen without any frame having cost differently.
pub struct PerfStats {
    /// One window per [`Stage`], indexed by it and latched together.
    ///
    /// An array rather than a field per stage. Twenty named fields have to be
    /// declared, defaulted, accumulated, latched and printed in five separate
    /// lists, and a stage missing from any one of them still compiles and
    /// still draws — a row that never latches simply holds the last figure it
    /// showed, which is a plausible number and so invisible. Indexed by the
    /// enum, [`STAGES`] is the only list there is to be missing from.
    windows: [Window; Stage::COUNT],
    /// Which slot of every window's `recent_max` this latch writes.
    peak_slot: usize,
    pub prims: u32,
    pub verts: u32,
    pub roll_notes: u32,
    /// The spectrogram's fallbacks PER SECOND — window re-aggregations and ring
    /// full uploads — latched with everything else.
    ///
    /// A rate and not a total, because the total climbs by one whenever the
    /// layout legitimately changes and would read the same as a cache that has
    /// stopped working. What tells those apart is how FAST it climbs: both of
    /// this pane's silent performance bugs sat at hundreds a second while a
    /// healthy build sits at zero.
    pub spec_fallbacks: (f32, f32),
    /// The totals the rates were last differenced from.
    last_fallbacks: (u32, u32),
    /// Smoothed resident set size in bytes, refreshed about once a second (0
    /// when the platform can't report it).
    ///
    /// Smoothed where the frame costs are not, because it is a different
    /// kind of signal: a slow-moving LEVEL read once a second, not a
    /// per-frame cost. There is no spike here for a filter to smear across
    /// time — raw RSS simply wanders by megabytes between reads as the host
    /// and the GPU driver take memory and give it back.
    pub rss_bytes: u64,
    /// Shell-clock time of the last memory read, to throttle it.
    last_mem_read: f64,
    /// Shell-clock time of the previous frame, for measuring the interval.
    /// `None` before the first one.
    last_frame: Option<f64>,
    /// Shell-clock time of the last latch.
    last_readout: f64,
    /// Whether the device ever said it could measure GPU time at all.
    pub gpu_supported: bool,
    /// Whether a measurement has actually come back.
    pub have_gpu: bool,
    /// This frame's workload (voice counts, visible nodes, render scale,
    /// whether it was animating).
    pub workload: Workload,
}

impl Default for PerfStats {
    fn default() -> Self {
        let mut windows = [Window::default(); Stage::COUNT];
        // Seeded to a plausible 60 Hz so the opening frames don't read as
        // absurd: the first frame has no predecessor to measure against, so it
        // contributes no interval at all and this is what shows.
        windows[Stage::Frame as usize].shown_mean = 1000.0 / 60.0;
        PerfStats {
            windows,
            peak_slot: 0,
            prims: 0,
            verts: 0,
            roll_notes: 0,
            spec_fallbacks: (0.0, 0.0),
            last_fallbacks: (0, 0),
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

impl FrameCosts {
    /// Gather a frame's costs from the three places they are measured: what
    /// the shell timed before the UI ran, the dock build this pass, and the
    /// lattice renderer's own atomics.
    ///
    /// The renderer publishes into atomics because it runs from a paint
    /// callback holding a `&SharedState`, so its readings are f32 bit
    /// patterns until something unpacks them. That something is here rather
    /// than in the UI's `root_ui`: the encoding is the renderer's
    /// business and the meaning of each field is this crate's, and neither
    /// is the root function's.
    pub fn assemble(
        shell: ShellTimings,
        cpu_ms: f32,
        lattice: &harmonigraph_render::LatticeStats,
        roll_notes: u32,
        spectrogram_fallbacks: (u32, u32),
    ) -> FrameCosts {
        let ms = |bits: &std::sync::atomic::AtomicU32| {
            f32::from_bits(bits.load(std::sync::atomic::Ordering::Relaxed))
        };
        FrameCosts {
            shell_ms: shell.shell_ms,
            cpu_ms,
            tess_ms: shell.tess_ms,
            egui_gpu_ms: shell.egui_gpu_ms,
            lattice_gpu_ms: ms(&lattice.gpu_ms),
            prepare_ms: ms(&lattice.prepare_ms),
            poll_ms: ms(&lattice.poll_ms),
            write_ms: ms(&lattice.write_ms),
            scene_ms: ms(&lattice.scene_ms),
            acquire_ms: shell.acquire_ms,
            tick_ms: shell.tick_ms,
            render_ms: shell.render_ms,
            upload_ms: shell.upload_ms,
            ubuf_ms: shell.ubuf_ms,
            texture_ms: shell.texture_ms,
            prims: shell.prims,
            verts: shell.verts,
            roll_notes,
            spectrogram_fallbacks,
            encode_ms: shell.encode_ms,
            submit_ms: shell.submit_ms,
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
    pub fn record(&mut self, costs: FrameCosts, now: f64, workload: Workload) {
        let dt = self.last_frame.map_or(0.0, |last| (now - last) as f32);
        self.last_frame = Some(now);
        // Every stage the frame's costs answer for, off the table — so a stage
        // that exists is a stage that accumulates, with no second list to
        // remember it in.
        for (stage, window) in STAGES.iter().zip(&mut self.windows) {
            if let Some(sample) = stage.sample {
                window.record(sample(&costs));
            }
        }
        // ...and the two the table leaves to be fed here, each because whether
        // this frame has a sample at all is a decision rather than a reading.
        //
        // The first frame has nothing to measure an interval against, so it
        // contributes no sample rather than a zero that would drag the mean.
        if dt > 0.0 {
            self.windows[Stage::Frame as usize].record(dt * 1000.0);
        }
        // Three states, not two: a real reading, "the device can't", and
        // "none has landed yet". Collapsing the last two into one "n/a" makes
        // a wiring bug and an unsupported GPU look identical, which is
        // exactly the question the row exists to answer.
        match costs.lattice_gpu_ms.to_bits() {
            harmonigraph_render::GPU_TIME_UNSUPPORTED => self.gpu_supported = false,
            // Still waiting for the first readback; leave the row saying so.
            harmonigraph_render::GPU_TIME_PENDING => {}
            // Anything else is a real reading, INCLUDING 0.0.
            _ => {
                self.have_gpu = true;
                self.windows[Stage::Gpu as usize].record(costs.lattice_gpu_ms);
            }
        }
        self.prims = costs.prims;
        self.verts = costs.verts;
        self.roll_notes = costs.roll_notes;
        // Close every window at once. `last_readout` starts at -inf, so the
        // first frame latches immediately and the HUD opens showing that
        // frame's real numbers rather than a quarter second of placeholder.
        if now - self.last_readout >= READOUT_INTERVAL {
            let slot = self.peak_slot;
            for window in &mut self.windows {
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
                self.spec_fallbacks = (
                    rate(costs.spectrogram_fallbacks.0, self.last_fallbacks.0),
                    rate(costs.spectrogram_fallbacks.1, self.last_fallbacks.1),
                );
            }
            self.last_fallbacks = costs.spectrogram_fallbacks;
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

    /// One stage's readout, for the rows that name a stage rather than walking
    /// the table. Indexed by the enum, so there is no stage the overlay can
    /// name that the array does not hold.
    pub fn window(&self, stage: Stage) -> &Window {
        &self.windows[stage as usize]
    }

    /// Every stage's readout, in [`STAGES`] order — for the rows that walk the
    /// table rather than naming a stage. The same index in both, which is what
    /// the check under the table is for: a row's label and its number can only
    /// come from the same entry.
    pub fn windows(&self) -> &[Window; Stage::COUNT] {
        &self.windows
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
    pub fn fps(&self) -> f32 {
        let frame = self.window(Stage::Frame).shown_mean;
        if frame > 0.0 {
            1000.0 / frame
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
pub fn memory_readout(rss_bytes: u64) -> String {
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
/// slot is a step that can silently not have happened (no reactivate, a build that
/// landed in a different worktree, the wrong branch named). The overlay saying
/// it in the picture is the one check that a reload cannot fool.
///
/// Names the last COMMIT, not the working tree — see `build.rs` for why there
/// is no dirty marker.
pub const BUILD_TAG: &str = env!("LATTICE_BUILD_TAG");

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

    /// The two numbers a stage's row currently prints, which is what most of
    /// the assertions below are about.
    fn mean(perf: &PerfStats, stage: Stage) -> f32 {
        perf.window(stage).shown_mean
    }
    fn peak(perf: &PerfStats, stage: Stage) -> f32 {
        perf.window(stage).shown_peak
    }

    /// `frames` frames costing `cpu_ms` each at 60 Hz, advancing the shell
    /// clock as it goes — the interval is derived from `now`, so the clock
    /// moving IS the measurement.
    fn feed(perf: &mut PerfStats, now: &mut f64, frames: usize, cpu_ms: f32) {
        for _ in 0..frames {
            *now += 1.0 / 60.0;
            perf.record(FrameCosts { cpu_ms, ..Default::default() }, *now, Workload::default());
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
        let mut tick = |perf: &mut PerfStats, now: &mut f64, folds: u32, uploads: u32| {
            totals = (totals.0 + folds, totals.1 + uploads);
            *now += 1.0 / 60.0;
            perf.record(
                FrameCosts { spectrogram_fallbacks: totals, ..Default::default() },
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
        let (folds, uploads) = perf.spec_fallbacks;
        assert!((folds - 60.0).abs() < 5.0, "a per-frame refold read as {folds}/s, not ~60");
        assert!(
            (uploads - 60.0).abs() < 5.0,
            "a per-frame full upload read as {uploads}/s, not ~60"
        );
    }

    #[test]
    fn fps_converges_on_the_measured_frame_time() {
        let mut perf = PerfStats::default();
        // Seeded at a plausible 60 Hz so early frames don't read as absurd.
        assert!((perf.fps() - 60.0).abs() < 1.0);
        for i in 1..=500 {
            let now = i as f64 / 30.0;
            perf.record(
                FrameCosts { cpu_ms: 2.0, ..Default::default() },
                now,
                Workload { animating: true, ..Default::default() },
            );
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
        let mut perf =
            PerfStats { rss_bytes: 400 * 1024 * 1024, last_mem_read: 0.0, ..Default::default() };
        // Force a read whose sample is whatever the platform reports; what is
        // under test is that the stored value MOVES but does not teleport.
        let before = perf.rss_bytes;
        perf.record(
            FrameCosts { cpu_ms: 1.0, ..Default::default() },
            MEM_INTERVAL,
            Workload::default(),
        );
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
        perf.record(
            FrameCosts { cpu_ms: 1.0, ..Default::default() },
            10.0 + MEM_INTERVAL / 2.0,
            Workload::default(),
        );
        assert_eq!(perf.last_mem_read, first, "read again too soon");
        // Past the interval, it refreshes.
        perf.record(
            FrameCosts { cpu_ms: 1.0, ..Default::default() },
            10.0 + MEM_INTERVAL,
            Workload::default(),
        );
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
            let now =
                if i == last { READOUT_INTERVAL } else { (i + 1) as f64 * READOUT_INTERVAL / 20.0 };
            perf.record(
                FrameCosts { cpu_ms: *cpu_ms, ..Default::default() },
                now,
                Workload::default(),
            );
        }
        let expected = costs.iter().sum::<f32>() / costs.len() as f32;
        assert!(
            (mean(&perf, Stage::Ui) - expected).abs() < 1e-4,
            "{} vs {expected}",
            mean(&perf, Stage::Ui)
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
        assert!((mean(&perf, Stage::Ui) - 1.0).abs() < 0.01, "{}", mean(&perf, Stage::Ui));

        // One catastrophic frame, then a steady 1 ms again.
        feed(&mut perf, &mut now, 1, 100.0);
        feed(&mut perf, &mut now, 40, 1.0);
        assert!(
            (mean(&perf, Stage::Ui) - 1.0).abs() < 0.01,
            "the spike is still dragging the mean: {}",
            mean(&perf, Stage::Ui)
        );
        // And it is still on screen — in the column that exists for it.
        assert!(peak(&perf, Stage::Ui) > 90.0, "the peak lost it: {}", peak(&perf, Stage::Ui));
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
        assert!(peak(&perf, Stage::Ui) > 90.0, "gone too soon: {}", peak(&perf, Stage::Ui));

        let quiet = (READOUT_INTERVAL * PEAK_WINDOWS as f64 * 60.0) as usize + 60;
        feed(&mut perf, &mut now, quiet, 1.0);
        assert!(peak(&perf, Stage::Ui) < 2.0, "the peak never let go: {}", peak(&perf, Stage::Ui));
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
            // `ubuf` nonzero, so `buf up` and `around` differ: they are the
            // same subtraction bar this term, and with it at zero the two
            // stages compute the same number — a row reading its neighbour's
            // cost would print a plausible figure and pass.
            let ubuf_ms = 0.2 + (i % 3) as f32 * 0.1;
            perf.record(
                FrameCosts {
                    tick_ms: render_ms + 1.0 + (i % 3) as f32,
                    render_ms,
                    upload_ms: texture_ms + 1.5,
                    texture_ms,
                    ubuf_ms,
                    ..Default::default()
                },
                now,
                Workload::default(),
            );
        }
        let (tick, egui, render) =
            (mean(&perf, Stage::Tick), mean(&perf, Stage::Egui), mean(&perf, Stage::Render));
        assert!((egui + render - tick).abs() < 1e-4, "{egui} + {render} != {tick}");
        // Same for the upload's two halves, accumulated the same way.
        assert!((mean(&perf, Stage::BufUp) - 1.5).abs() < 1e-4, "{}", mean(&perf, Stage::BufUp));
        // ...and one level further in, where the same promise is made about
        // `buf up` and the two rows indented under it.
        let (buf_up, ubuf, around) =
            (mean(&perf, Stage::BufUp), mean(&perf, Stage::Ubuf), mean(&perf, Stage::Around));
        assert!((ubuf + around - buf_up).abs() < 1e-4, "{ubuf} + {around} != {buf_up}");
        assert!(ubuf > 0.0 && around > 0.0, "both halves have to be real: {ubuf}, {around}");
    }

    /// Digits that never hold still get squinted at rather than read. The
    /// window keeps accumulating every frame; what the overlay PRINTS is
    /// latched, so it changes four times a second instead of 144.
    #[test]
    fn the_printed_numbers_hold_between_latches() {
        let mut perf = PerfStats::default();
        perf.record(FrameCosts { cpu_ms: 2.0, ..Default::default() }, 0.0, Workload::default());
        let shown = mean(&perf, Stage::Ui);
        assert_eq!(shown, 2.0, "the opening frame latches at once, not from a placeholder");

        // Frames well inside the interval: they accumulate, the printed value
        // does not budge.
        for i in 1..=10 {
            perf.record(
                FrameCosts { cpu_ms: 20.0, ..Default::default() },
                i as f64 * READOUT_INTERVAL / 20.0,
                Workload::default(),
            );
        }
        assert_eq!(perf.window(Stage::Ui).n, 10, "the frames should have accumulated");
        assert_eq!(mean(&perf, Stage::Ui), shown, "the printed value must hold");

        // Past the interval it catches up — to the mean of that window, which
        // holds nothing but the 20s.
        perf.record(
            FrameCosts { cpu_ms: 20.0, ..Default::default() },
            READOUT_INTERVAL,
            Workload::default(),
        );
        assert_eq!(mean(&perf, Stage::Ui), 20.0, "and then it latches");
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
            perf.record(
                FrameCosts { cpu_ms: 1.0, ..Default::default() },
                i as f64 / 120.0,
                Workload::default(),
            );
        }
        let from_row = 1000.0 / mean(&perf, Stage::Frame);
        assert!((perf.fps() - from_row).abs() < 1e-3, "{} vs {from_row}", perf.fps());
    }

    /// Every reading a frame carries reaches a stage: as `c.<field>` in a
    /// [`STAGES`] sample, or as `costs.<field>` where [`PerfStats::record`]
    /// feeds a window by hand.
    ///
    /// A cost measured every frame and read by nothing is the failure worth
    /// guarding, because it shows up as nothing at all — the overlay draws its
    /// usual rows, none of them looks wrong, and the reading someone went to
    /// the trouble of plumbing through the shell is simply absent. `dead_code`
    /// cannot see it here: [`FrameCosts`] is this crate's public API, and a
    /// `pub` field of a public struct counts as read whether or not anything
    /// reads it.
    ///
    /// Reading the source is the cheap way to ask the question, in the same
    /// shallow way `harmonigraph-render`'s `struct_field_names` reads its two
    /// `Uniforms` — one field per line, an identifier before the first `:`.
    /// What it cannot check is that a field reaches the RIGHT stage; the
    /// overlay's own `every_breakdown_row_reports_the_cost_it_names` is what
    /// relates a reading to the label it prints under.
    #[test]
    fn every_frame_cost_reaches_a_stage() {
        // Code only, and only the code above this module: a field named in a
        // doc comment as `costs.foo_ms`, or bound under that name by a test
        // below, would otherwise answer for itself while no stage reads it.
        let src = include_str!("lib.rs");
        let code: String = src
            .split_once("#[cfg(test)]")
            .map_or(src, |(above, _)| above)
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let after_kw =
            code.split_once("pub struct FrameCosts").expect("FrameCosts is declared here").1;
        let body = after_kw
            .split_once('{')
            .expect("FrameCosts has a body")
            .1
            .split_once('}')
            .expect("FrameCosts's body ends")
            .0;
        let fields: Vec<&str> = body
            .lines()
            .map(str::trim)
            .filter_map(|line| line.strip_prefix("pub "))
            .map(|field| field.split_once(':').expect("a field line has a `:`").0)
            .collect();
        // A shallow parse fails by finding nothing, which would pass every
        // assertion below, so it has to show it found the list first: a plain
        // reading, and the one field whose type is not a bare `f32`.
        assert!(
            fields.contains(&"tick_ms") && fields.contains(&"spectrogram_fallbacks"),
            "the field list did not parse: {fields:?}",
        );
        for field in fields {
            assert!(
                code.contains(&format!("c.{field}")) || code.contains(&format!("costs.{field}")),
                "`FrameCosts::{field}` is measured every frame and read by nothing — give it a \
                 line in STAGES, or feed its window from `record`",
            );
        }
    }
}
