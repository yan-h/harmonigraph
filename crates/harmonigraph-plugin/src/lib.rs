//! The two plugins on harmonigraph-clap's shell: parameters, MIDI ingestion,
//! and the CLAP export. All interesting logic lives in the `harmonigraph-*` crates; this
//! crate only adapts them to the plugin world.

use std::sync::Arc;

use editor::gestures::Gesture;
use harmonigraph_clap::prelude::*;
use harmonigraph_core::lattice_map::{Follow, TuningEngine};
use harmonigraph_core::notes::{NoteEvent as CoreNoteEvent, SourceId};
use harmonigraph_record::TAKE_CHANNELS;
use harmonigraph_ui::lattice_maps::{MapOffsetLane, EXTENSION_STEP, OFFSET_LIMIT};
use harmonigraph_ui::params::{AnalysisInput, ParamBackend, ParamKey};

mod audio_ingress;
mod background;
mod board;
mod configuration;
mod editor;
mod lattice_maps;
mod presentation;
mod saved_fields;
/// What a saved state loads as, for the `read-plugin-state` tool.
pub mod saved_state;
mod shared;
mod tape;

/// Identity of this plugin build, supplied to the shared performance overlay.
pub(crate) const BUILD_TAG: &str = BUILD_TAG_NUL.split_at(BUILD_TAG_NUL.len() - 1).0;

/// The bytes `BUILD_TAG` points into, NUL-terminated so that `load-plugin.sh`
/// can tell where the tag ends. A bare `&str` has no terminator, and the linker
/// packs the next string straight after it: one starting with a hex digit read
/// as more of the sha (#1428, #1430).
const BUILD_TAG_NUL: &str = concat!(env!("LATTICE_BUILD_TAG"), "\0");

#[cfg(all(feature = "startup-probe", target_os = "macos"))]
pub use editor::startup_probe::run as editor_startup_probe;
// The global allocator: one that counts our Rust heap for the memory row
// (`harmonigraph_perf::heap`, #1512) in every release build, and in a release
// TEST binary the allocation guard under it as well.
//
// Release only, because in a debug build harmonigraph-clap may already have
// installed one, and two `#[global_allocator]`s cannot link. It declares its own
// (`MeasuredAllocator` over `AllocDisabler`) under `#[cfg(debug_assertions)]`
// plus its `assert_process_allocs` feature — see
// `crates/harmonigraph-clap/src/wrapper/util.rs`. That feature arrives through this
// crate's dev-dependency, and cargo unifies it into every unit of a build that
// has dev-dependencies, so a debug `cargo test` that links the cdylib would
// carry both. Keyed on `debug_assertions` alone, the two are complements: the
// debug plugin bundle, which nobody loads into a DAW, has no heap reading and
// the row leaves it out.
//
// In a test binary the guard is the point. `assert_no_alloc` only raises a
// thread-local forbid count; the abort lives in the global allocator that reads
// it, so without `AllocDisabler` under the counter the `configuration::tests`
// blocks pass without guarding: measured, a deliberate 64-byte `Vec` inside
// `canonical_publication_slots_and_loss_are_allocation_free` was reported `ok`.
//
// The two declarations are complements only while the profiles they read agree
// about `debug_assertions`, and that is an agreement BETWEEN two profiles
// rather than a property of one: a `cargo test` lib-test unit compiles under
// `test` while harmonigraph-clap compiles under `dev`. They agree by default, and this
// workspace already customises `dev` and `dev.package."*"`, so both ways of
// breaking it are reachable from `Cargo.toml`:
//
//   * `[profile.test] debug-assertions = false` leaves harmonigraph-clap's on
//     and turns this one on too — two `#[global_allocator]`s in one binary,
//     which is a hard "cannot define multiple global allocators" and therefore
//     audible.
//   * `[profile.dev.package.harmonigraph-clap] debug-assertions = false`
//     reaches the shell and not this crate, so NEITHER is installed and the
//     guard silently goes back to passing without guarding. (Before the shell
//     was a workspace member, the usual build-speed trick
//     `[profile.dev.package."*"]` reached it the same way.) `tests::exactly_one_global_allocator_is_installed`
//     stops compiling on that one: the probe it reads is gated the same way.
//
// If a guarded block here ever needs an escape hatch, it must call
// `nice_assert_no_alloc::permit_alloc` and NOT `harmonigraph_clap::util::permit_alloc`,
// which is what the rest of the tree and all of harmonigraph-clap reach for.
// harmonigraph-clap gates its helper on
// `all(debug_assertions, feature = "assert_process_allocs")`, so under
// `--release` it is an identity function and the allocation it was supposed to
// permit aborts the WHOLE test binary, taking every other test's result with
// it. The `nice_assert_no_alloc` one is live in both profiles, because no
// manifest turns on its `disable_release` (the workspace `Cargo.toml` entry).
#[cfg(all(not(test), not(debug_assertions)))]
#[global_allocator]
static ALLOCATOR: harmonigraph_perf::heap::Counting<std::alloc::System> =
    harmonigraph_perf::heap::Counting::new(std::alloc::System, &harmonigraph_perf::heap::RUST_HEAP);
#[cfg(all(test, not(debug_assertions)))]
#[global_allocator]
static ALLOCATOR: harmonigraph_perf::heap::Counting<nice_assert_no_alloc::AllocDisabler> =
    harmonigraph_perf::heap::Counting::new(
        nice_assert_no_alloc::AllocDisabler,
        &harmonigraph_perf::heap::RUST_HEAP,
    );

#[cfg(test)]
mod test_device;
#[cfg(test)]
mod test_scope;
mod tuning;
use tuning::plugin::HarmonigraphTune;

/// Capacity of the audio→GUI sample ring feeding the Spectral pane's
/// analyzer: >1 s of STEREO at 48 kHz. Overflow just drops frames — a spectrum
/// meter would rather skip than stall the audio thread.
///
/// Doubled when the ring stopped carrying a mono mixdown and started carrying
/// the channels themselves (see `process`), so the seconds it holds — which is
/// what bounds the input worker's catch-up work after a stall, see
/// `SpectrumAnalysis::feed_source` — stayed where it was.
///
/// It is also what paces [`background`], whose longest poll has to stay well
/// inside the span this holds; see `SHUT_POLL` there.
pub(crate) const AUDIO_RING_CAPACITY: usize = 131_072;

/// Sample rate assumed until the host reports the real one in `activate`.
const DEFAULT_SAMPLE_RATE: f64 = 44_100.0;

pub struct Harmonigraph {
    /// Built at CLAP init and taken at destruction, so every host callback
    /// sees one. Only a test that builds the plugin without the wrapper runs
    /// `process` with none, and then nothing records.
    configuration: Option<Box<configuration::Owner>>,
    aggregation: Option<Box<tuning::hub::Hub>>,
    /// The input worker, the one thing that drains the audio thread's rings
    /// and runs the analysis, window open or shut (see [`background`]). Held
    /// only to be dropped with the plugin, which is what stops its thread.
    ///
    /// FIRST, because fields drop in declaration order and this one has to stop
    /// before any of what it touches is torn down. Declared last instead, the
    /// worker holds the surviving `Arc` once `editor_shared` drops, so the
    /// teardown races a live drain and `EditorShared` — `SharedState`, and any
    /// `egui::TextureHandle` still in its spectrogram surfaces — is destroyed on
    /// the analyzer thread rather than on the one dropping the plugin. That
    /// appears to be sound, but PR #112 was a texture freed from the wrong point
    /// and it cost a crash; ordering the field is cheaper than being sure.
    input_worker: background::InputWorker,
    /// The thread this plugin dropped on, once it has, for a teardown test.
    #[cfg(test)]
    dropped_on: Arc<std::sync::Mutex<Option<Option<String>>>>,
    params: Arc<HarmonigraphParams>,
    /// Coherent, bounded audio and source metadata for the live analyzer.
    audio_producer: audio_ingress::Producer,
    /// State shared with the editor; created eagerly, since the input worker
    /// drains into it, and installs a host restore through it, before the
    /// GUI opens.
    editor_shared: Arc<editor::EditorLock>,
    /// The host's side of the editor, given before the editor is made: the
    /// editor's parameter edits, its resize requests and its own window's
    /// close go through it.
    editor_host: Option<Arc<dyn EditorHost>>,
    sample_rate: f64,
    samples_processed: u64,
    /// Presentation time: what the audio, the tape, notes, reset markers and
    /// the take's runs are all dated by (#1885). It runs on across a reset
    /// and an activation, so historical queued records keep their place.
    clock: presentation::PresentationClock,
    /// Take recording (see `harmonigraph_record`). The recorder is always
    /// present; it only writes while the user has armed it from the Video pane.
    take: RecorderSlot,
    /// The tape's audio-thread half (#1719): it watches the transport and
    /// mixes an audition into the main buffer.
    tape: tape::Deck,
    /// The host activated us to render offline. The tape never sounds then,
    /// though Bitwig is not known to flag the export pass this way (see
    /// `harmonigraph_record`'s crate docs); it is one guard among several.
    offline: bool,
}

impl Drop for Harmonigraph {
    fn drop(&mut self) {
        #[cfg(test)]
        {
            let thread = std::thread::current().name().map(str::to_owned);
            *self.dropped_on.lock().unwrap() = Some(thread);
        }
        // Window state can retain its own Arc during native teardown. Join the
        // graphics worker at plugin destruction, not only at the last Arc drop.
        // One hold for both handles, and each joined with the lock free.
        let (graphics, take) = {
            let shared = self.editor_shared.lock();
            (shared.ui.picture.editor_graphics(), shared.take.clone())
        };
        graphics.shutdown_startup();
        take.shutdown_exports();
    }
}

/// The live shell always owns its recorder. Destruction transfers that same
/// producer, alongside its original route owner, into bounded session retirement.
struct RecorderSlot(Option<harmonigraph_record::Recorder>);
impl std::ops::Deref for RecorderSlot {
    type Target = harmonigraph_record::Recorder;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("live recorder")
    }
}
impl std::ops::DerefMut for RecorderSlot {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().expect("live recorder")
    }
}

pub struct HarmonigraphParams {
    /// The host parameters, one row per [`Row`].
    pub table: Arc<Table>,
    /// Saved in the `lattice-maps` field (`saved_fields`).
    pub(crate) maps: Arc<lattice_maps::Maps>,
    pub map_editor: Arc<parking_lot::Mutex<harmonigraph_ui::lattice_maps::MapEditor>>,
    pub map_playback: Arc<parking_lot::Mutex<harmonigraph_ui::lattice_maps::MapPlayback>>,
    /// Where following has moved the map, packed by [`lattice_maps::pack`]:
    /// written by the Hub on audio, read by the editor.
    pub map_followed: Arc<std::sync::atomic::AtomicU64>,
    /// Fifths Adaptive's strip has slid from its Position before the clamp, as
    /// the Hub published.
    pub strip_slide: Arc<std::sync::atomic::AtomicI32>,
    /// The tuning axes, links, Learn and adaptive policy: plugin-owned
    /// settings rather than host parameters, saved in `musical-settings`.
    settings: Arc<configuration::Settings>,
    /// Window size in logical pixels, saved in the `editor-state` field.
    pub editor_state: Arc<editor::EguiState>,
    /// Serialized UI state (layout, camera, view settings), saved in the
    /// `ui-state` field: a restore installs into the editor state and a save
    /// serializes out of it (see `UiState`).
    pub ui_state: editor::UiState,
}

/// Harmonigraph's host parameters, in the order the host lists them; each is
/// its row in [`HarmonigraphParams::table`]. The order is the host's, so a
/// new row goes at the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    /// The Lattice Map slot, numbered from 0 and labelled from 1.
    LatticeMap,
    MapFifths,
    MapThirds,
    MapSevenths,
    MapFifthsExtension,
    MapThirdsExtension,
    MapSeventhsExtension,
    /// Which engine retunes notes ([`TuningEngine`]), as an index.
    TuningEngine,
    /// Whether Lattice Map slides the map by itself ([`Follow`]), as an index.
    MapFollow,
    /// Which input feeds the analyzer and the audio recorded with a take. This
    /// is a host parameter rather than part of `ui_state`, so the audio thread
    /// can read it directly and a restored project applies it before an editor
    /// has ever opened. It is deliberately not automatable: changing sources is
    /// an editing decision, not a signal to splice into the spectrogram.
    AnalysisInput,
    CameraYaw,
    CameraPitch,
    CameraDistance,
    CameraPanX,
    CameraPanY,
}

impl Row {
    /// Every row, in host order.
    pub const ALL: [Row; 15] = [
        Row::LatticeMap,
        Row::MapFifths,
        Row::MapThirds,
        Row::MapSevenths,
        Row::MapFifthsExtension,
        Row::MapThirdsExtension,
        Row::MapSeventhsExtension,
        Row::TuningEngine,
        Row::MapFollow,
        Row::AnalysisInput,
        Row::CameraYaw,
        Row::CameraPitch,
        Row::CameraDistance,
        Row::CameraPanX,
        Row::CameraPanY,
    ];

    /// The row's index in the table.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The id a save stores the row under and the host knows it by: saved
    /// projects and automation lanes hold these, so one never changes.
    pub fn id(self) -> &'static str {
        match self {
            Row::LatticeMap => "lattice-map",
            Row::MapFifths => "map-fifths",
            Row::MapThirds => "map-thirds",
            Row::MapSevenths => "map-sevenths",
            Row::MapFifthsExtension => "map-fifths-extension",
            Row::MapThirdsExtension => "map-thirds-extension",
            Row::MapSeventhsExtension => "map-sevenths-extension",
            Row::TuningEngine => "tuning-engine",
            Row::MapFollow => "map-follow",
            Row::AnalysisInput => "analysis-input",
            Row::CameraYaw => ParamKey::CameraYaw.id(),
            Row::CameraPitch => ParamKey::CameraPitch.id(),
            Row::CameraDistance => ParamKey::CameraDistance.id(),
            Row::CameraPanX => ParamKey::CameraPanX.id(),
            Row::CameraPanY => ParamKey::CameraPanY.id(),
        }
    }

    /// The row's declaration. The Lattice Map's label names the map in its
    /// slot, which it reads from `maps`.
    fn spec(self, maps: &Arc<lattice_maps::Maps>) -> Spec {
        let id = self.id();
        let lane = |name, grain| {
            let spec = Spec::int(id, name, 0, -OFFSET_LIMIT, OFFSET_LIMIT).non_modulatable();
            match grain {
                MapOffsetLane::Fine => spec,
                // An extension lane reads and parses in steps of EXTENSION_STEP.
                MapOffsetLane::Extension => spec
                    .with_to_text(|value| (value * EXTENSION_STEP).to_string())
                    .with_from_text(|text| {
                        let steps: i32 = text.trim().parse().ok()?;
                        (steps % EXTENSION_STEP == 0).then_some(steps / EXTENSION_STEP)
                    }),
            }
        };
        // Entirely from the ParamKey metadata (name, default and range), so the
        // host-facing parameter can never drift from what the UI clamps and
        // displays.
        let camera = |key: ParamKey| {
            let range = key.range();
            Spec::float(id, key.host_name(), key.default_value(), *range.start(), *range.end())
        };
        match self {
            Row::LatticeMap => {
                let names = maps.clone();
                Spec::int(id, "Lattice Map", 0, 0, lattice_maps::LAST_MAP)
                    .non_modulatable()
                    .with_to_text(move |slot| {
                        names.read(|document| match document.name(slot as usize) {
                            Some(name) => format!("{} · {name}", slot + 1),
                            None => format!("{} · unavailable", slot + 1),
                        })
                    })
            }
            Row::MapFifths => lane("Map Fifths", MapOffsetLane::Fine),
            Row::MapThirds => lane("Map Thirds", MapOffsetLane::Fine),
            Row::MapSevenths => lane("Map Sevenths", MapOffsetLane::Fine),
            Row::MapFifthsExtension => lane("Map Fifths x10", MapOffsetLane::Extension),
            Row::MapThirdsExtension => lane("Map Thirds x10", MapOffsetLane::Extension),
            Row::MapSeventhsExtension => lane("Map Sevenths x10", MapOffsetLane::Extension),
            Row::TuningEngine => Spec::int(
                id,
                "Note Retuning",
                TuningEngine::default().index(),
                0,
                TuningEngine::HOST_MAX,
            )
            .with_to_text(|value| TuningEngine::from_index(value).label().into())
            .non_automatable(),
            Row::MapFollow => {
                Spec::int(id, "Map Follow", Follow::default().index(), 0, Follow::HOST_MAX)
                    .non_modulatable()
                    .with_to_text(|value| Follow::from_index(value).label().into())
            }
            Row::AnalysisInput => Spec::int(
                id,
                "Analyzer Audio Input",
                AnalysisInput::default().index(),
                0,
                AnalysisInput::HOST_MAX,
            )
            .with_to_text(|value| AnalysisInput::from_index(value).label().into())
            .with_from_text(|text| {
                let input = AnalysisInput::HOST.into_iter().find(|input| input.label() == text);
                input.map(AnalysisInput::index)
            })
            .non_automatable(),
            Row::CameraYaw => camera(ParamKey::CameraYaw),
            Row::CameraPitch => camera(ParamKey::CameraPitch),
            Row::CameraDistance => camera(ParamKey::CameraDistance),
            Row::CameraPanX => camera(ParamKey::CameraPanX),
            Row::CameraPanY => camera(ParamKey::CameraPanY),
        }
    }

    /// The row a camera key is saved and automated through.
    pub fn camera(key: ParamKey) -> Row {
        match key {
            ParamKey::CameraYaw => Row::CameraYaw,
            ParamKey::CameraPitch => Row::CameraPitch,
            ParamKey::CameraDistance => Row::CameraDistance,
            ParamKey::CameraPanX => Row::CameraPanX,
            ParamKey::CameraPanY => Row::CameraPanY,
        }
    }
}

impl Default for HarmonigraphParams {
    fn default() -> Self {
        let maps = Arc::new(lattice_maps::Maps::default());
        let table = Table::new(Row::ALL.map(|row| row.spec(&maps)));
        HarmonigraphParams {
            table: Arc::new(table),
            maps,
            map_editor: Default::default(),
            map_playback: Default::default(),
            map_followed: Default::default(),
            strip_slide: Default::default(),
            settings: Default::default(),
            editor_state: editor::EguiState::from_size(
                editor::DEFAULT_SIZE.0,
                editor::DEFAULT_SIZE.1,
            ),
            ui_state: editor::UiState::new(),
        }
    }
}

impl HarmonigraphParams {
    /// An integer row's value, modulated.
    pub fn int(&self, row: Row) -> i32 {
        self.table.int(row.index())
    }

    /// The configuration as the editor shows it: drawn under the engine the
    /// parameters select (`ResolvedConfig::drawn`), as the audio side records
    /// it for a take. A save reads the configuration undrawn.
    pub(crate) fn configuration_view(&self) -> harmonigraph_ui::params::ConfigurationView {
        let mut view = self.settings.view();
        let engine = TuningEngine::from_index(self.int(Row::TuningEngine));
        view.board = view.resolved.policy.board_drawn(engine);
        view.resolved = view.resolved.drawn(engine);
        view
    }

    /// A camera row's value, modulated.
    pub fn camera(&self, key: ParamKey) -> f32 {
        self.table.float(Row::camera(key).index())
    }

    /// A camera row's value before the host's modulation, which a save
    /// stores and an editor write sets.
    pub fn camera_base(&self, key: ParamKey) -> f32 {
        self.table.base(Row::camera(key).index())
    }

    /// Which input feeds the analyzer.
    pub fn analysis_input(&self) -> AnalysisInput {
        AnalysisInput::from_index(self.int(Row::AnalysisInput))
    }

    /// What the host shows for `row`'s value.
    #[cfg(test)]
    pub(crate) fn label(&self, row: Row) -> String {
        let spec = self.table.spec(row.index());
        spec.text(spec.to_host(self.int(row) as f32))
    }

    /// Set `row` to `plain` as a host would, with no wrapper in between.
    #[cfg(test)]
    pub(crate) fn set(&self, row: Row, plain: f32) {
        let spec = self.table.spec(row.index());
        self.table.set_host(row.index(), spec.to_host(plain));
    }
}

/// Adapts the host parameters to the UI's `ParamBackend`, so panes don't
/// know they're running inside a plugin.
pub(crate) struct PluginParamBackend<'a> {
    pub params: &'a HarmonigraphParams,
    /// Where the editor's writes go: the host, then the table.
    pub host: &'a dyn EditorHost,
    /// The tuning configuration as this frame reads it, once, so every
    /// slider in the frame shows one coherent value.
    pub configuration: harmonigraph_ui::params::ConfigurationView,
    /// The host gestures the editor holds open (`editor::gestures`). Lives
    /// in EditorShared so simultaneous axes survive across frames (this
    /// adapter is rebuilt every frame), and so a close or a loss of sight
    /// ends every one.
    pub gestures: &'a std::cell::Cell<editor::gestures::Gestures>,
}

impl ParamBackend for PluginParamBackend<'_> {
    fn lattice_maps(&self) -> Option<harmonigraph_ui::lattice_maps::MapView> {
        Some(lattice_maps::view(self.params))
    }
    fn edit_lattice_map(&self, edit: harmonigraph_ui::lattice_maps::MapEdit) {
        use harmonigraph_ui::lattice_maps::MapEdit;
        // A lane's gesture is the registry's, as the camera's is: a value set
        // with none open (a drag still held after the editor was unseen)
        // begins one first.
        match edit {
            MapEdit::BeginOffset(axis, lane) => self.begin(Gesture::Lane(axis, lane)),
            MapEdit::EndOffset(axis, lane) => self.end(Gesture::Lane(axis, lane)),
            MapEdit::Offset(axis, lane, _) => {
                self.begin(Gesture::Lane(axis, lane));
                lattice_maps::edit(self.params, self.host, edit);
            }
            _ => lattice_maps::edit(self.params, self.host, edit),
        }
    }
    fn tuning_instances(&self) -> Vec<harmonigraph_ui::params::TuningInstance> {
        tuning::instances::snapshots()
    }
    fn edit_tuning_instance(&self, id: u64, edit: harmonigraph_ui::params::InstanceEdit) {
        tuning::instances::edit(id, edit);
    }
    fn configuration(&self) -> Option<harmonigraph_ui::params::ConfigurationView> {
        Some(self.configuration)
    }
    /// The view after the edit, as `configuration_view` draws it, so the rest
    /// of this frame draws the edit: under Pass through with Decode fifths on,
    /// in the board's tuning rather than the saved one.
    fn submit_tuning(
        &self,
        edit: harmonigraph_core::configuration::ConfigEdit,
    ) -> Option<harmonigraph_ui::params::ConfigurationView> {
        self.params.settings.submit(edit);
        Some(self.params.configuration_view())
    }

    fn camera_value(&self, key: ParamKey) -> Option<f32> {
        Some(self.get(key))
    }

    fn camera_base(&self, key: ParamKey) -> Option<f32> {
        Some(self.params.camera_base(key))
    }

    fn get(&self, key: ParamKey) -> f32 {
        self.params.camera(key)
    }

    fn set(&self, key: ParamKey, value: f32) {
        let row = Row::camera(key).index();
        if self.gestures.get().is_open(Gesture::Param(key)) {
            // Inside an explicit gesture (drag): just set.
            self.host.set_parameter(row, value);
        } else {
            // One-shot change: implicit single-value gesture.
            self.host.begin_set_parameter(row);
            self.host.set_parameter(row, value);
            self.host.end_set_parameter(row);
        }
    }

    fn analysis_input(&self) -> Option<AnalysisInput> {
        Some(self.params.analysis_input())
    }

    fn set_analysis_input(&self, input: AnalysisInput) {
        let row = Row::AnalysisInput.index();
        self.host.begin_set_parameter(row);
        self.host.set_parameter(row, input.index() as f32);
        self.host.end_set_parameter(row);
    }

    fn begin_set(&self, key: ParamKey) {
        self.begin(Gesture::Param(key));
    }

    fn end_set(&self, key: ParamKey) {
        self.end(Gesture::Param(key));
    }
}

impl PluginParamBackend<'_> {
    /// Open `gesture` in the registry, and send its begin if it was closed.
    fn begin(&self, gesture: Gesture) {
        let mut gestures = self.gestures.get();
        if gestures.begin(gesture) {
            self.gestures.set(gestures);
            self.host.begin_set_parameter(gesture.row().index());
        }
    }

    /// Close `gesture` in the registry, and send its end if it was open.
    fn end(&self, gesture: Gesture) {
        let mut gestures = self.gestures.get();
        if gestures.end(gesture) {
            self.gestures.set(gestures);
            let mut only = editor::gestures::Gestures::default();
            only.begin(gesture);
            editor::gestures::end(only, self.host);
        }
    }
}

// The decisions `process` makes that do not need a host.
//
// A focused fixture below reaches the real process wiring with live buffers,
// but these are the decisions whose edge cases are clearer as pure values — a
// ring left out of phase, a take stamped on the wrong clock — and exactly the
// class a build cannot catch and a listener notices late.

/// What this block's take timestamps hang off, before the recorder gets a say.
#[derive(Debug, Clone, Copy, PartialEq)]
enum OriginSource {
    /// Not armed: nothing is recorded this block. The recorder has still been
    /// consulted by then — see the `armed` argument, which only `is_armed` can
    /// supply and which cannot be skipped on a disarmed block.
    Idle,
    /// The host reports a position. Only `observe_transport` can say whether it
    /// counts, so the caller still has to ask.
    Transport(f64),
    /// The host reports no position at all: fall back to the plugin's own clock,
    /// so "just record what I play" still works.
    LocalClock(f64),
}

/// Take timestamps come from the TRANSPORT, not the plugin's own sample
/// counter, so a take lines up with a bounce of the same song without an offset
/// to work out. Nothing is recorded while the transport is stopped.
///
/// `pos_seconds` is read even when disarmed; it is a plain getter, so asking
/// for it on a block that will not use it costs nothing.
fn origin_source(
    armed: bool,
    pos_seconds: Option<f64>,
    block_start: u64,
    sample_rate: f64,
) -> OriginSource {
    if !armed {
        OriginSource::Idle
    } else if let Some(seconds) = pos_seconds {
        OriginSource::Transport(seconds)
    } else {
        OriginSource::LocalClock(block_start as f64 / sample_rate)
    }
}

/// The host's transport at a block's start, as the tape, a take's origin and
/// its stop bar read it: CLAP's fields behind their flags, with the position
/// fallbacks nice-plug's `Transport` gave them, which moved here when the
/// shell stopped building one (#1814).
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Transport {
    pub playing: bool,
    pub tempo: Option<f64>,
    pub time_sig_numerator: Option<i32>,
    pub time_sig_denominator: Option<i32>,
    pub pos_seconds: Option<f64>,
    /// In quarter notes.
    pub pos_beats: Option<f64>,
    pub bar_start_pos_beats: Option<f64>,
    /// From 0 at the song's first bar.
    pub bar_number: Option<i32>,
}

impl Transport {
    /// No transport is one that reports nothing and is not playing.
    fn read(transport: Option<&harmonigraph_clap::events::Transport>) -> Self {
        let Some(t) = transport else { return Self::default() };
        let signature = t.time_signature();
        // The bar is always given: see `bar_position` for one that is zero.
        let (bar, bar_start) = t.bar();
        Self {
            playing: t.playing(),
            tempo: t.tempo(),
            time_sig_numerator: signature.map(|(numerator, _)| i32::from(numerator)),
            time_sig_denominator: signature.map(|(_, denominator)| i32::from(denominator)),
            pos_seconds: t.seconds(),
            pos_beats: t.beats(),
            bar_start_pos_beats: Some(bar_start),
            bar_number: Some(bar),
        }
    }

    /// The position in seconds, from the beats and the tempo where the host
    /// gives no seconds timeline.
    fn pos_seconds(&self) -> Option<f64> {
        match (self.pos_seconds, self.pos_beats, self.tempo) {
            (Some(seconds), _, _) => Some(seconds),
            (_, Some(beats), Some(tempo)) => Some(beats / tempo * 60.0),
            _ => None,
        }
    }

    /// The position in quarter notes, from the seconds and the tempo where
    /// the host gives no beats timeline.
    fn pos_beats(&self) -> Option<f64> {
        match (self.pos_beats, self.pos_seconds, self.tempo) {
            (Some(beats), _, _) => Some(beats),
            (_, Some(seconds), Some(tempo)) => Some(seconds / 60.0 * tempo),
            _ => None,
        }
    }
}

/// Where the transport sits in BARS, counted the way an arranger counts:
/// bar 1 is the song's start, so this returns 0.0 there and 64.0 at bar 65.
/// `None` when the host reports no beats timeline, which reads downstream as
/// "never crossed a stop bar" — see `Recorder::observe_bar`.
///
/// Two sources, and the fallback is the reason both are here. `bar_number`
/// plus the offset into the bar is exact across a TIME-SIGNATURE CHANGE, where
/// dividing the beat count by the current meter is not. But CLAP does not put
/// `bar_number` behind a flag, so a host that fills in none of it hands us a
/// constant zero — and zero bars is a position that can never be crossed from
/// below, so that degrades to a take that never ends rather than to one that
/// ends at the wrong bar. When the bar fields are absent entirely, the beat
/// count divided by the meter is the honest answer for a song in one meter.
///
/// `pos_beats` is in QUARTER notes, so the meter converts as
/// `numerator * 4 / denominator` — 4 quarters to a 4/4 bar, 3 to a 6/8 one.
fn bar_position(transport: &Transport) -> Option<f64> {
    let meter = host_meter(transport)?;
    Some(f64::from(meter.bar) + (meter.beats - meter.bar_start) / meter.bar_length())
}

/// The host's beat, bar and time signature where the transport sits, read as
/// [`bar_position`] describes, with its tempo or 0.0 where it sends none.
fn host_meter(transport: &Transport) -> Option<harmonigraph_ui::tape::Meter> {
    let beats = transport.pos_beats()?;
    let (numerator, denominator) =
        match (transport.time_sig_numerator, transport.time_sig_denominator) {
            (Some(numerator), Some(denominator)) if numerator > 0 && denominator > 0 => {
                (numerator, denominator)
            }
            // 4/4 is what a host that reports no meter is almost always in,
            // and the alternative is refusing to stop at all.
            _ => (4, 4),
        };
    let per_bar = f64::from(numerator) * 4.0 / f64::from(denominator);
    let (bar, bar_start) = match (transport.bar_number, transport.bar_start_pos_beats) {
        (Some(bar), Some(start)) => (bar, start),
        _ => {
            let bar = (beats / per_bar).floor();
            (bar as i32, bar * per_bar)
        }
    };
    let tempo = transport.tempo.unwrap_or(0.0);
    Some(harmonigraph_ui::tape::Meter { beats, tempo, bar_start, bar, numerator, denominator })
}

/// The host's meter, for Tape mode's ruler: only with a tempo to place
/// its bars in time.
fn tape_meter(transport: &Transport) -> Option<harmonigraph_ui::tape::Meter> {
    host_meter(transport).filter(|meter| meter.tempo > 0.0)
}

/// Which input plane the take's right channel reads from. A take's WAV is
/// always stereo, matching the plugin's ports, so a mono input is duplicated
/// rather than written as half a frame — which would desync every frame after
/// it.
fn take_right_channel(channels: usize) -> usize {
    usize::from(channels > 1)
}

/// One stereo frame for the take WAV, from the same selected buffer the live
/// analyzer reads. The caller has already established that the buffer has at
/// least one channel and that `frame` is in range.
fn take_stereo_frame(planes: &[&[f32]], frame: usize) -> [f32; TAKE_CHANNELS] {
    [planes[0][frame], planes[take_right_channel(planes.len())][frame]]
}

/// One source decision for the whole process block. Both the live analyzer and
/// the take recorder read the returned buffer, so changing the source can never
/// put the live and offline pictures on opposite inputs. Missing auxiliary data
/// falls back only as a defensive layout guard; an advertised but unrouted
/// sidechain is a real zero-filled buffer and remains silence.
fn selected_analysis_input<'a, T: ?Sized>(
    main: &'a T,
    sidechain: Option<&'a T>,
    input: AnalysisInput,
) -> &'a T {
    match input {
        AnalysisInput::Main => main,
        AnalysisInput::Sidechain => sidechain.unwrap_or(main),
    }
}

impl Default for Harmonigraph {
    fn default() -> Self {
        let (audio_producer, audio_consumer) = audio_ingress::channel(AUDIO_RING_CAPACITY);
        let (take, mut take_control) = harmonigraph_record::channel(crate::BUILD_TAG);
        let consumer = take_control.take_display().expect("one display consumer");
        #[cfg(test)]
        let take = test_device::injected_recorder().unwrap_or(take);
        let params = Arc::new(HarmonigraphParams::default());
        let (tape, tape_worker) = tape::channel(params.clone());
        let aggregation = tuning::hub::Hub::new();
        let wake = aggregation.shared.clone();
        take_control.set_stop_wakeup(move || wake.request_main());
        let mut editor = editor::EditorShared::new(take_control);
        // An edit to what `ui-state` saves marks the project modified, as a
        // musical setting's does (#1813).
        let settings = params.settings.clone();
        editor.ui.on_edit(move || settings.changed());
        let editor_shared = Arc::new(editor::EditorLock::new(editor, params.ui_state.inbox()));
        // From HERE, not from `activate` or the editor: the point of the
        // thread is to cover the stretches nothing else does, and both of those
        // are stretches. `activate` runs on activation, so a suspended plugin
        // would go dark; the editor is the very thing being covered for. It
        // also installs a project's saved settings before any window is built,
        // through the lock both share (#324).
        let input = editor::input::LiveInput::new(consumer, audio_consumer);
        let input_worker = background::InputWorker::spawn(
            editor_shared.clone(),
            params.editor_state.clone(),
            tape_worker,
            input,
            board::Lights::live(params.settings.clone(), params.map_playback.clone()),
            Some(params.clone()),
        );
        params.ui_state.attach(&editor_shared);
        Harmonigraph {
            configuration: None,
            aggregation: Some(aggregation),
            params,
            audio_producer,
            editor_shared,
            editor_host: None,
            sample_rate: DEFAULT_SAMPLE_RATE,
            samples_processed: 0,
            clock: presentation::PresentationClock::default(),
            take: RecorderSlot(Some(take)),
            tape,
            offline: false,
            input_worker,
            #[cfg(test)]
            dropped_on: Default::default(),
        }
    }
}

impl Harmonigraph {
    /// The audio body of one callback, after the Hub has sequenced its input:
    /// the take, the tape and the analyzer, all on the transport at the
    /// block's start.
    fn play(&mut self, transport: &Transport, audio: Audio<'_>) {
        let block_start = self.samples_processed;
        let block_samples = audio.main.first().map_or(0, |channel| channel.len());
        // EVERY block, armed or not. `is_armed` is an edge detector, not a
        // getter: it advances the lifecycle and clears the recorder's per-take
        // state on the arming edge. Skipping it on a disarmed block means the
        // next arm edge never fires and recording silently never resumes.
        // Without a configuration owner there is no route to record through.
        let armed = self
            .configuration
            .as_ref()
            .is_some_and(|owner| self.take.is_armed_at(owner.recording_intent()));
        let take_origin =
            match origin_source(armed, transport.pos_seconds(), block_start, self.sample_rate) {
                OriginSource::Idle => None,
                // observe_transport decides whether the transport is rolling (and
                // ends the take at a loop wrap under Loop end). It is
                // deliberately more permissive than the host's `playing` flag —
                // see there.
                OriginSource::Transport(seconds) => {
                    // The stop bar is asked FIRST: a block at or past the bar
                    // belongs to no take, so the cut lands on a block
                    // boundary at or before the bar rather than one after it.
                    let stopped = self.take.observe_bar(bar_position(transport));
                    // Where this block sits in the take's AUDIO, not the
                    // host's position: see `observe_transport`.
                    let origin = self.take.observe_transport(
                        seconds,
                        transport.playing,
                        block_samples as f64 / self.sample_rate,
                    );
                    origin.filter(|_| !stopped)
                }
                OriginSource::LocalClock(seconds) => Some(seconds),
            };

        if let Some(owner) = self.configuration.as_mut() {
            owner.record(&mut self.take, take_origin);
        }
        // The selected input for the GUI's spectrum analyzer, INTERLEAVED:
        // it analyzes the channels separately and combines them in the power
        // domain, so a mixdown here would cancel anti-phase content before it
        // could (see `harmonigraph_analysis::ChannelBank`). A full ring — the
        // input worker stalled — drops frames. The next retained
        // source position explicitly resets analysis across that gap.
        // Read the parameter and choose ONCE per block: the same buffer goes to
        // the live ring below and the take recorder after it.
        let selected = self.params.analysis_input();
        // The tape first, so the analyzer and anything after Harmonigraph hear
        // what is played: the live input plus an audition. It sounds only
        // while the transport is stopped, and records only while it rolls, so
        // it never records itself. With nothing to mix it leaves the buffer
        // untouched, bit for bit.
        let tape = self.tape.process(
            tape::deck::BlockIn {
                position: transport.pos_seconds(),
                playing: transport.playing,
                frames: block_samples,
                rate: self.sample_rate,
                enabled: selected == AnalysisInput::Main && !self.offline,
            },
            audio.main,
        );
        let mut main: [&[f32]; harmonigraph_clap::audio_setup::MAX_CHANNELS] = Default::default();
        for (view, plane) in main.iter_mut().zip(audio.main.iter()) {
            *view = plane;
        }
        let main = &main[..audio.main.len()];
        let planes = selected_analysis_input(main, audio.aux.first().copied(), selected);
        let channels = planes.len();
        self.audio_producer.publish(
            block_samples,
            audio_ingress::Format {
                channels,
                sample_rate: self.sample_rate as f32,
                sidechain: selected == AnalysisInput::Sidechain,
            },
            self.clock.start(),
            tape.song.map(|seconds| audio_ingress::Song { seconds, meter: tape_meter(transport) }),
            tape.play,
            (0..block_samples).flat_map(|f| (0..channels).map(move |c| planes[c][f])),
        );

        if let Some(owner) = self.configuration.as_mut() {
            self.aggregation.as_mut().unwrap().publish(owner, &mut self.take);
        }
        // A block that records, after the run it belongs to: parameters are
        // timed on the take's own clock and placed by that run.
        let recorded = self.configuration.as_ref().filter(|_| self.take.recording());
        if let Some(at) = recorded.map(|owner| owner.take_time()) {
            self.take.params(at, ParamKey::ALL.map(|key| self.params.camera(key)));

            // The take's own audio, when asked for: the selected analysis input
            // exactly as it arrives, so the render gets a spectrum and a
            // soundtrack without a separate bounce. Sized by TAKE_CHANNELS
            // rather than the host's own channel count — the WAV is stereo
            // whatever arrives, which is what `take_right_channel` duplicates
            // a mono input for.
            if self.take.wants_audio() && channels > 0 {
                let wanted = block_samples * TAKE_CHANNELS;
                let mut interleaved =
                    (0..block_samples).flat_map(|frame| take_stereo_frame(planes, frame));
                self.take.audio(&mut interleaved, wanted, self.sample_rate as f32);
            }
        }

        self.samples_processed += block_samples as u64;
        self.clock.processed(block_samples);
        self.take.publish_clock(self.clock.end());
    }
}

impl Plugin for Harmonigraph {
    const NAME: &'static str = "Harmonigraph";
    const VENDOR: &'static str = "Yan Han";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const CLAP_ID: &'static str = "com.yan-h.harmonigraph";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Tonnetz harmony and spectrum visualizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> =
        Some("https://github.com/yan-h/harmonigraph/issues");
    const CLAP_FEATURES: &'static [ClapFeature] =
        &[ClapFeature::NoteEffect, ClapFeature::Analyzer, ClapFeature::Utility];

    // Main audio passes through, untouched unless the tape is auditioning
    // with the transport stopped (see `tape`). The selected input is tapped for
    // the GUI's spectrum analyzer — INTERLEAVED, channel by channel, not as a
    // mixdown (see `play`, and `interleaved_reservation` for what that
    // costs). MIDI goes out through the Hub, which delays it by D and states
    // each note's tuning (see `tuning`).
    const PORTS: Ports =
        Ports { main_input: 2, main_output: 2, aux_inputs: &[(2, "Visualization Input")] };

    const NOTE_PORTS: bool = true;

    fn params(&self) -> Arc<Table> {
        self.params.table.clone()
    }

    fn setup(&self) -> Option<Arc<dyn harmonigraph_clap::wrapper::clap::setup::Setup>> {
        Some(Arc::new(configuration::HubSetup {
            instance: tuning::setup::Adapter(self.aggregation.as_ref().unwrap().shared.clone()),
            params: self.params.clone(),
        }))
    }

    /// Every call the editor makes into the host goes through the document
    /// doors' check (`shared::Checked`).
    fn editor_host(&mut self, host: Arc<dyn EditorHost>) {
        self.editor_host = Some(Arc::new(shared::Checked(host)));
    }

    fn editor(&mut self) -> Option<Box<dyn Editor>> {
        let host = self.editor_host.clone()?;
        Some(editor::create(self.params.clone(), self.editor_shared.clone(), host))
    }

    fn init(&mut self) -> bool {
        self.configuration = Some(Box::new(configuration::Owner::new(&self.params)));
        self.aggregation.as_mut().unwrap().register();
        true
    }

    fn activate(&mut self, config: &BufferConfig) -> bool {
        // Audio is stopped: the first callback starts from the document.
        if let Some(owner) = self.configuration.as_mut() {
            owner.seed();
        }
        self.aggregation
            .as_mut()
            .unwrap()
            .activate(f64::from(config.sample_rate), config.max_buffer_size);
        self.sample_rate = f64::from(config.sample_rate);
        self.clock.activate(self.sample_rate);
        self.offline = config.process_mode == ProcessMode::Offline;
        // The editor's device and its loading job, started here so a first
        // open finds them (#1746). On macOS, the one platform that ships; not
        // for an export pass, which draws no editor; nor under test, where
        // every activation would start a GPU compile beside the tests timing
        // themselves.
        if cfg!(target_os = "macos") && !cfg!(test) && !self.offline {
            let (context, cache, format) = {
                let shared = self.editor_shared.lock();
                let picture = &shared.ui.picture;
                (
                    shared.gpu_context.clone(),
                    picture.editor_graphics(),
                    picture.surfaces.target_format,
                )
            };
            editor::start_graphics(&context, &cache, &editor::graphics_config(), format);
        }
        true
    }

    /// For a host that stops calling the audio thread after a Stop: the
    /// stopped take's run closes here, so the writer can seal it.
    fn inactive(&mut self) {
        self.configuration.as_mut().unwrap().recording.close_stopped(&mut self.take);
    }

    /// Destruction gives the rings back. A take still recording fails, which
    /// the recorder's own drop decides: a Tune learns the Hub is gone from the
    /// same slot it learned the Hub was here.
    fn destroy(&mut self) {
        // First, on the host's main thread: a worker step can be inside a
        // host wake, holding the wrapper's last other reference, and the
        // plugin must not then drop on the worker, which joins itself and
        // whose step may hold the editor lock the drop takes (#1938).
        self.input_worker.stop();
        let mut hub = self.aggregation.take().unwrap();
        self.configuration.take();
        self.take.0.take();
        hub.retire();
    }

    fn stop(&mut self) {
        self.aggregation.as_mut().unwrap().stop();
        // The cut's snapshots wait for a callback that may not come: what was
        // held ends here, for the lattice, the board's keys and Learn alike,
        // in the stream's own order, so a restart's notes after it stay held
        // (#1948). The tape is no source of the session's, and plays on.
        let _ = self.take.publish_note(CoreNoteEvent::session_reset(self.clock.end()).into());
    }

    fn reset(&mut self) {
        if let Some(owner) = self.configuration.as_mut() {
            // The session's cut first, as the audio engine's own reset took
            // it before the plugin's state was reset. Only with an owner: a
            // plugin built outside the shell has no session to cut.
            self.aggregation.as_mut().unwrap().stop();
            // Stop may arrive after activation but before start_processing
            // resets this owner. Close its run before forgetting routes.
            owner.recording.close_stopped(&mut self.take);
            owner.reset(&mut self.take);
        }
        self.samples_processed = 0;
        self.audio_producer.reset();
        // Before anything else can render: an export may follow a reset.
        self.tape.reset();
        // Reset only observed direct input. The session owner must publish
        // its own explicit source/session controls after lifecycle validation.
        // Presentation time stays continuous across the reset, so history
        // queued before it keeps its place. The marker is timed where the
        // clock left off, which the next callback is pinned at if it does not
        // run on, so its notes start at or after it.
        self.take.publish_clock(self.clock.end());
        let _ = self
            .take
            .publish_note(CoreNoteEvent::source_reset(self.clock.end(), SourceId::DIRECT).into());
    }

    /// The Hub first, which the configuration's `begin` relies on, then the
    /// configuration's boundary: the configuration the document last
    /// published, and the map state the parameters held before its automation.
    fn begin(&mut self, boundary: &Boundary) {
        let owner = self.configuration.as_mut().unwrap();
        self.clock.begin(boundary);
        self.aggregation.as_mut().unwrap().begin(boundary, owner, self.clock.mapping());
        if boundary.valid {
            let seed = lattice_maps::Seed::read(&self.params);
            owner.begin(boundary, seed, &self.take);
        }
        if boundary.input_lost {
            owner.fault(&self.take);
        }
    }

    fn process(&mut self, block: &mut Block<'_>) {
        let owner = self.configuration.as_mut().unwrap();
        let hub = self.aggregation.as_mut().unwrap();
        // The automation timeline, at each event's own sample. The Hub's own
        // notes reach learning through its own sequenced row, at the time
        // they are scheduled to sound, so there is nothing left for a second
        // observation of the same input to add.
        for input in block.events() {
            owner.maps.observe(input, owner.current());
        }
        for input in block.events() {
            hub.input(input);
        }
        hub.input_boundary(owner);
        hub.schedule(&mut block.output);
        // A lost input has no audio: nothing records, and the host discards
        // the callback.
        if let Some(audio) = block.audio.take() {
            let transport = Transport::read(block.boundary.transport.as_ref());
            self.play(&transport, audio);
        }
    }

    fn end(&mut self) {
        if let Some(owner) = self.configuration.as_mut() {
            self.aggregation.as_mut().unwrap().end(owner, &mut self.take);
        }
    }
}

export_clap!(Harmonigraph, HarmonigraphTune);

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// How long a "the analyzer has done the work" assertion waits before it
    /// gives up — on the analyzer, or on the machine, a distinction a wait
    /// cannot resolve, so its failure message must not claim either.
    ///
    /// Two orders of magnitude over the 20 ms poll, because what has to be
    /// bounded is not how fast the analyzer works but how long the scheduler
    /// may ignore it: on a shared CI runner a round can simply fail to happen
    /// inside any window a fixed sleep would pick, which is what took main red
    /// at `f9cbc4e0` on a tree that had already passed (#598). Nothing healthy
    /// ever spends this — a wait returns on the round that lands — so it
    /// costs the suite only when it is about to fail anyway.
    const ANALYSIS_DEADLINE: Duration = Duration::from_secs(5);

    /// The global allocator this profile is meant to have is the one that
    /// actually runs: ours, counting the heap over the guard, under
    /// `--release`; nice-plug's measuring guard in a debug build.
    ///
    /// Two installed is a link error and needs no test. NEITHER is the silent
    /// way for the cfg pair at the top of this file to break: the memory row
    /// loses its heap and every `assert_no_alloc` block passes without
    /// guarding. Each arm asserts the allocator it expects saw an allocation.
    #[test]
    fn exactly_one_global_allocator_is_installed() {
        #[cfg(not(debug_assertions))]
        assert!(
            harmonigraph_perf::heap::rust_heap().is_some(),
            "a release build runs without the heap-counting allocator",
        );
        #[cfg(debug_assertions)]
        {
            let (_, measured) =
                harmonigraph_clap::wrapper::allocation_probe::measure_allocations(|| {
                    std::hint::black_box(Box::new(0u64))
                });
            assert!(measured.allocations > 0, "a debug build runs without nice-plug's allocator");
        }
    }

    /// Whether the analyzer completed a loop round after `before`.
    ///
    /// This is the synchronization a negative assertion cannot obtain from
    /// columns: a round that sees an open editor correctly produces none. The
    /// counter exists only under `cfg(test)`, so observing that silence does not
    /// add state or atomic traffic to the plugin build.
    fn a_round_after(plugin: &Harmonigraph, before: u64) -> bool {
        let started = Instant::now();
        loop {
            if plugin.input_worker.completed_rounds() > before {
                return true;
            }
            if started.elapsed() >= ANALYSIS_DEADLINE {
                return false;
            }
            std::thread::sleep(background::POLL / 2);
        }
    }

    /// Stop the plugin's input worker and take the input it owned, after
    /// its final drain: for a test that drains by hand at times it chooses,
    /// or reads what the worker's analysis ran at.
    fn stop_input_worker(plugin: &mut Harmonigraph) -> editor::input::LiveInput {
        plugin.input_worker.join().expect("the input worker ran and ended")
    }

    fn publish_test_audio(plugin: &mut Harmonigraph, frames: usize) {
        plugin.audio_producer.publish(
            frames,
            audio_ingress::Format {
                channels: 1,
                sample_rate: DEFAULT_SAMPLE_RATE as f32,
                sidechain: false,
            },
            {
                plugin.clock.step(frames, DEFAULT_SAMPLE_RATE);
                plugin.clock.start()
            },
            None,
            audio_ingress::TapePlay::Idle,
            (0..frames).map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).sin() * 0.5),
        );
        plugin.clock.processed(frames);
        plugin.take.publish_clock(plugin.clock.end());
    }

    /// The editor's host is the one the plugin installs, checked: a call
    /// from inside a document closure fails the depth check before it
    /// reaches the host, and one outside passes through (#1938).
    #[cfg(debug_assertions)]
    #[test]
    fn the_installed_editor_host_refuses_a_call_from_inside_a_document_closure() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counting(AtomicUsize);
        impl EditorHost for Counting {
            fn closed(&self) -> bool {
                false
            }
            fn request_resize(&self, _: (u32, u32)) -> bool {
                false
            }
            fn begin_set_parameter(&self, _: usize) {}
            fn set_parameter(&self, _: usize, _: f32) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
            fn end_set_parameter(&self, _: usize) {}
        }
        let mut plugin = Harmonigraph::default();
        let counting = Arc::new(Counting(AtomicUsize::new(0)));
        plugin.editor_host(counting.clone());
        let host = plugin.editor_host.clone().expect("installed");
        let inside = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            plugin.params.maps.write(|_| host.set_parameter(0, 0.0))
        }));
        assert!(inside.is_err(), "refused");
        assert_eq!(counting.0.load(Ordering::Relaxed), 0, "before it reached the host");
        host.set_parameter(0, 0.0);
        assert_eq!(counting.0.load(Ordering::Relaxed), 1, "outside, it passes");
    }

    /// The input worker and the editor share one `EguiState`, and nothing but
    /// this says so: `Default` hands one to [`background::InputWorker::spawn`]
    /// and `editor` hands the same field to `editor::create`. A second one on
    /// either side compiles and passes every other test, and leaves a still
    /// editor drawing the first note after silence at its next idle poll,
    /// since the worker would ask a state no window reads for the pass.
    ///
    /// Driven through the REAL spawned thread rather than by calling `step`,
    /// because the wiring is precisely what a direct call would bypass.
    #[test]
    fn the_input_worker_asks_the_editors_own_state_for_a_pass() {
        let mut plugin = Harmonigraph::default();
        let state = plugin.params.editor_state.clone();
        state.set_open(true);
        state.pass_request.take();
        plugin.take.publish_clock(1.0);
        plugin
            .take
            .publish_note(CoreNoteEvent::on(1.0, SourceId::DIRECT, 0, 60, 1.0).into())
            .expect_both();
        let started = Instant::now();
        while !state.pass_request.take() {
            assert!(
                started.elapsed() < ANALYSIS_DEADLINE,
                "no pass asked of the editor's state in {ANALYSIS_DEADLINE:?} after a strike: \
                 the worker asks another EguiState, or this runner never scheduled it",
            );
            std::thread::sleep(background::POLL);
        }
    }

    /// The project's saved settings have to reach the analyzer with no editor
    /// ever constructed, which is the whole of issue #324: the host restores
    /// them at a moment of its own choosing, and the plugin analyzes audio from
    /// the moment it is instantiated.
    ///
    /// Driven through the REAL spawned thread and the real `params` field —
    /// nothing here calls [`Plugin::editor`] — because that pairing IS the
    /// claim: a restore waiting in an inbox no lock the worker takes delivers
    /// from reaches nothing a host ever writes.
    #[test]
    fn a_projects_saved_window_reaches_the_analyzer_with_no_editor_built() {
        use harmonigraph_ui::SpectrumWindow;

        // What the analyzer's lag reads at the window it runs, on the rate a
        // plugin runs at until a host reports its own.
        let lag_of = |window: SpectrumWindow| {
            harmonigraph_ui::SpectrumConfig { window, ..Default::default() }
                .column_lag(DEFAULT_SAMPLE_RATE as f32)
        };
        let blob = {
            let mut saved = harmonigraph_ui::SharedState::new(editor::ASSUMED_SURFACE_FORMAT);
            saved.picture.appearance.spectrum.window = SpectrumWindow::Precise;
            saved.picture.appearance.camera.yaw = 1.23;
            saved.picture.appearance.view.max_sevens = 3;
            saved.picture.appearance.render.short_edge = 2160;
            harmonigraph_ui::shell::close(&saved)
        };

        let mut plugin = Harmonigraph::default();
        let state = plugin.params.editor_state.clone();
        state.pass_request.take();
        // What the host does when it restores a project, and all it does: no
        // editor, no activation, just the field.
        restore(&plugin, &blob);
        // A Precise frame is 24,576 samples; this is more than one frameful,
        // so the analyzer cannot come out of the drain empty.
        publish_test_audio(&mut plugin, 40_000);
        // Waited for without taking the editor lock, which would install the
        // restore itself: the worker asks for a pass once it has given the
        // first sound to the spectrum, and nothing else here asks for one.
        let started = Instant::now();
        while !state.pass_request.take() {
            assert!(
                started.elapsed() < ANALYSIS_DEADLINE,
                "nothing was analyzed in {ANALYSIS_DEADLINE:?}, or this runner never \
                 scheduled the analyzer",
            );
            std::thread::sleep(background::POLL / 2);
        }

        // The analysis is the worker's own, so read it once the worker has
        // stopped and handed it back.
        let lag = stop_input_worker(&mut plugin).analysis_lag();
        let shared = plugin.editor_shared.lock();
        assert_eq!(shared.ui.picture.appearance.camera.yaw, 1.23);
        assert_eq!(shared.ui.picture.appearance.view.max_sevens, 3);
        assert_eq!(shared.ui.picture.appearance.render.short_edge, 2160);
        assert!(
            (lag - lag_of(SpectrumWindow::Precise)).abs() < 1e-9,
            "the input worker ran at a {:.1} ms lag where the project's saved window \
             lags {:.1} ms: the restored blob reaches nothing until the editor opens",
            lag * 1000.0,
            lag_of(SpectrumWindow::Precise) * 1000.0,
        );
    }

    /// What a host save stores in `ui-state`: through `saved_fields::save`,
    /// which is what a host save calls, on the real plugin whose `Default`
    /// attaches the editor state — a `UiState` attached by hand would pass
    /// these tests and reach nothing a host saves.
    fn saved(plugin: &Harmonigraph) -> String {
        let mut state = empty_state();
        saved_fields::save(&plugin.params, &mut state);
        let field = state.fields.remove("ui-state").expect("ui-state saved");
        serde_json::from_str(&field).expect("the field is a JSON string")
    }

    fn empty_state() -> harmonigraph_clap::plugin::PluginState {
        harmonigraph_clap::plugin::PluginState {
            version: String::new(),
            params: Default::default(),
            fields: Default::default(),
        }
    }

    /// [`saved`], read back as the camera yaw.
    fn saved_yaw(plugin: &Harmonigraph) -> f32 {
        let mut state = harmonigraph_ui::SharedState::new(editor::ASSUMED_SURFACE_FORMAT);
        assert!(state.install_document(&saved(plugin)), "the saved blob does not load");
        state.picture.appearance.camera.yaw
    }

    /// A host restore of `blob`: through `saved_fields`' read and install,
    /// which a host's state load calls, so it reaches `UiState::restore` the
    /// way a project or preset load does.
    fn restore(plugin: &Harmonigraph, blob: &str) {
        let mut state = empty_state();
        state.fields.insert("ui-state".into(), serde_json::to_string(blob).unwrap());
        saved_fields::read(&plugin.params, &state, Vec::new()).install(&plugin.params);
    }

    /// A host restore of a project whose camera yaw is `yaw`, and its blob.
    fn restore_yaw(plugin: &Harmonigraph, yaw: f32) -> String {
        let mut state = harmonigraph_ui::SharedState::new(editor::ASSUMED_SURFACE_FORMAT);
        state.picture.appearance.camera.yaw = yaw;
        let blob = harmonigraph_ui::shell::close(&state);
        restore(plugin, &blob);
        blob
    }

    fn live_yaw(plugin: &Harmonigraph) -> f32 {
        plugin.editor_shared.lock().ui.picture.appearance.camera.yaw
    }

    fn turn_live_yaw(plugin: &Harmonigraph, yaw: f32) {
        plugin.editor_shared.lock().ui.picture.appearance.camera.yaw = yaw;
    }

    /// A host save stores what the editor holds, not what the host last
    /// restored (#1301), window open or shut.
    #[test]
    fn a_save_stores_what_the_editor_holds() {
        let plugin = Harmonigraph::default();
        restore_yaw(&plugin, 0.5);
        turn_live_yaw(&plugin, 1.25);
        assert_eq!(saved_yaw(&plugin), 1.25, "a save stored the restore over the editor's edit");
    }

    /// A document this build wrote comes back from a save byte for byte, so a
    /// project reopened and saved unedited is not rewritten.
    #[test]
    fn a_restored_document_saves_back_byte_for_byte() {
        let plugin = Harmonigraph::default();
        let blob = restore_yaw(&plugin, 0.5);
        assert_eq!(saved(&plugin), blob);
    }

    /// An editor never opened saves nothing, so its project keeps following
    /// the fresh look of whichever build loads it (#1757 Q2); once a window
    /// has opened, a save stores the settings it showed.
    #[test]
    fn an_editor_never_opened_saves_nothing() {
        let plugin = Harmonigraph::default();
        assert_eq!(saved(&plugin), "");
        editor::open_window_session(&plugin.editor_shared, &egui::Context::default());
        let shown = plugin.editor_shared.lock().ui.save_persist();
        assert_eq!(saved(&plugin), shown, "an opened editor saved nothing");
    }

    /// An empty restore into an open window (a host undo back to an untouched
    /// preset) installs the fresh look, and the edits made after it are what
    /// the next save stores, not `""`.
    #[test]
    fn an_empty_restore_into_an_open_window_keeps_saving_its_edits() {
        let plugin = Harmonigraph::default();
        editor::open_window_session(&plugin.editor_shared, &egui::Context::default());
        restore(&plugin, "");
        turn_live_yaw(&plugin, 1.25);
        assert_eq!(saved_yaw(&plugin), 1.25, "a save after the empty restore lost the edit");
    }

    /// An empty restore is what an untouched editor's project carries, so it
    /// puts the fresh look back (#1757 Q3): undoing a preset load whose preset
    /// came from an untouched editor goes back to the fresh look, and a save
    /// then stores that look.
    #[test]
    fn an_empty_restore_is_the_fresh_look() {
        let plugin = Harmonigraph::default();
        let fresh = live_yaw(&plugin);
        restore_yaw(&plugin, fresh + 0.5);
        assert_eq!(saved_yaw(&plugin), fresh + 0.5);

        restore(&plugin, "");
        assert_eq!(live_yaw(&plugin), fresh, "the editor kept the restore an empty one replaced");
        assert_eq!(
            saved_yaw(&plugin),
            fresh,
            "a save after the empty restore is not the fresh look"
        );
    }

    /// A save that cannot take the editor lock — one the host makes from a
    /// callback the editor makes under it — returns at once rather than hang
    /// the host (#296), with the newest of the last restore, close or
    /// successful save (#1301).
    #[test]
    fn a_save_that_cannot_take_the_editor_lock_returns_the_fallback() {
        let plugin = Harmonigraph::default();
        restore_yaw(&plugin, 0.5);
        turn_live_yaw(&plugin, 1.25);
        assert_eq!(saved_yaw(&plugin), 1.25);

        let mut held = plugin.editor_shared.lock();
        held.ui.picture.appearance.camera.yaw = 2.0;
        assert_eq!(saved_yaw(&plugin), 1.25, "not the last successful save");
        restore_yaw(&plugin, 0.75);
        assert_eq!(saved_yaw(&plugin), 0.75, "older than a restore still waiting");
        drop(held);
    }

    /// The close ends the window session and refreshes the fallback, then
    /// hands back the host's gestures to end with the editor lock free:
    /// `end_set_parameter` is a host callback, and a save the host makes from
    /// inside it takes the lock and answers with the session; a restore made
    /// there survives the close.
    #[test]
    fn the_close_ends_the_host_gestures_with_the_lock_free() {
        let plugin = Harmonigraph::default();
        restore_yaw(&plugin, 0.5);
        editor::open_window_session(&plugin.editor_shared, &egui::Context::default());
        turn_live_yaw(&plugin, 1.25);
        {
            let shared = plugin.editor_shared.lock();
            let mut gestures = shared.gestures.get();
            gestures.begin(Gesture::Param(ParamKey::CameraYaw));
            shared.gestures.set(gestures);
        }

        let open = plugin.editor_shared.end_window_session();
        let mut reentered = None;
        // What `editor::gestures::end` does for each, with a host that saves
        // and restores from inside the call.
        for _ in open.open() {
            assert!(plugin.editor_shared.try_lock().is_some(), "ended under the editor lock");
            reentered = Some(saved_yaw(&plugin));
            restore_yaw(&plugin, 0.75);
        }
        assert_eq!(reentered, Some(1.25), "a save from the end of a gesture lost the session");
        assert_eq!(saved_yaw(&plugin), 0.75, "the restore made from the end was lost");
    }

    /// A look edit a drag left open when the editor stops being seen becomes
    /// one undo entry then, as at a close, and no save answer is made for it.
    #[test]
    fn going_unseen_finishes_an_open_look_edit() {
        use harmonigraph_ui::testing::{edit_look, undo_depth};

        let plugin = Harmonigraph::default();
        editor::open_window_session(&plugin.editor_shared, &egui::Context::default());
        edit_look(&mut plugin.editor_shared.lock().ui, true);
        assert_eq!(undo_depth(&plugin.editor_shared.lock().ui), 0, "the drag is still open");
        let fallback = plugin.params.ui_state.inbox().fallback();

        assert!(plugin.editor_shared.end_unseen().is_some(), "the editor lock is free");
        assert_eq!(
            undo_depth(&plugin.editor_shared.lock().ui),
            1,
            "the open edit was not finished"
        );
        assert_eq!(plugin.params.ui_state.inbox().fallback(), fallback, "a save answer was made");
    }

    /// Issue #1740: closing and reopening the editor window keeps the look's
    /// undo history, and an edit the close cut off mid-drag becomes one
    /// entry of it. Through the plugin's own close, a round of the input
    /// worker and the window build's state half: the last two each reloaded
    /// the close's blob, and with it a fresh, empty history.
    #[test]
    fn undo_history_survives_closing_and_reopening_the_window() {
        use harmonigraph_ui::testing::{edit_look, undo_depth};

        let plugin = Harmonigraph::default();
        editor::open_window_session(&plugin.editor_shared, &egui::Context::default());
        plugin.params.editor_state.set_open(true);
        edit_look(&mut plugin.editor_shared.lock().ui, false);
        edit_look(&mut plugin.editor_shared.lock().ui, true);
        assert_eq!(undo_depth(&plugin.editor_shared.lock().ui), 1, "the drag is still open");

        let _ = plugin.editor_shared.end_window_session();
        plugin.params.editor_state.set_open(false);
        let rounds = plugin.input_worker.completed_rounds();
        assert!(
            a_round_after(&plugin, rounds + 1),
            "no worker round in {ANALYSIS_DEADLINE:?}: this runner never scheduled it",
        );
        editor::open_window_session(&plugin.editor_shared, &egui::Context::default());

        assert_eq!(undo_depth(&plugin.editor_shared.lock().ui), 2, "closing the window lost undo");
    }

    /// `ParamKey::id` is what a recorded take names its automation by, and
    /// what the host names its automation lanes by. If a key's row drifts from
    /// its id, takes silently replay with default parameters and projects
    /// silently lose their automation.
    #[test]
    fn every_param_key_id_matches_the_host_facing_id() {
        let params = HarmonigraphParams::default();
        for key in ParamKey::ALL {
            assert_eq!(params.table.spec(Row::camera(key).index()).id, key.id(), "{key:?}");
        }
        for (index, row) in Row::ALL.into_iter().enumerate() {
            assert_eq!(row.index(), index, "{row:?} is not at its own index");
        }
    }

    /// Saved projects and automation hold these integers and hosts show these
    /// words, so the map and engine parameters' names, ranges, defaults and
    /// every label are pinned here as literals rather than read back from the
    /// tables that build them.
    #[test]
    fn map_and_engine_parameters_keep_their_host_contract() {
        let params = HarmonigraphParams::default();
        let contract = |row: Row| {
            let spec = params.table.spec(row.index());
            let (low, high) = (spec.unnormalize(0.0) as i32, spec.unnormalize(1.0) as i32);
            let labels: Vec<String> =
                (low..=high).map(|value| spec.text(spec.to_host(value as f32))).collect();
            (spec.name.to_owned(), spec.default as i32, low..=high, labels)
        };
        let words = |labels: &[&str]| labels.iter().map(|&label| label.to_owned()).collect();
        assert_eq!(
            contract(Row::TuningEngine),
            ("Note Retuning".into(), 0, 0..=2, words(&["Pass through", "Adaptive", "Lattice Map"]))
        );
        assert_eq!(
            contract(Row::MapFollow),
            ("Map Follow".into(), 0, 0..=2, words(&["Off", "Thirds", "Thirds and fifths"]))
        );
        let (name, default, range, _) = contract(Row::LatticeMap);
        assert_eq!((name.as_str(), default, range), ("Lattice Map", 0, 0..=127));
        let fine: Vec<String> = (-9..=9).map(|value: i32| value.to_string()).collect();
        let extension: Vec<String> = (-9..=9).map(|value: i32| (value * 10).to_string()).collect();
        // Each lane in `LANES` is the row of its id.
        let lane_ids = harmonigraph_ui::lattice_maps::LANES
            .map(|(axis, grain)| params.table.spec(lattice_maps::lane_row(axis, grain).index()).id);
        assert_eq!(
            lane_ids,
            [
                "map-fifths",
                "map-thirds",
                "map-sevenths",
                "map-fifths-extension",
                "map-thirds-extension",
                "map-sevenths-extension",
            ]
        );
        let non_modulatable: Vec<&str> = (0..params.table.len())
            .map(|row| params.table.spec(row))
            .filter(|spec| spec.automatable && !spec.modulatable)
            .map(|spec| spec.id)
            .collect();
        let expected: Vec<&str> =
            ["lattice-map"].into_iter().chain(lane_ids).chain(["map-follow"]).collect();
        assert_eq!(non_modulatable, expected);
        for (row, name, labels) in [
            (Row::MapFifths, "Map Fifths", &fine),
            (Row::MapThirds, "Map Thirds", &fine),
            (Row::MapSevenths, "Map Sevenths", &fine),
            (Row::MapFifthsExtension, "Map Fifths x10", &extension),
            (Row::MapThirdsExtension, "Map Thirds x10", &extension),
            (Row::MapSeventhsExtension, "Map Sevenths x10", &extension),
        ] {
            assert_eq!(contract(row), (name.into(), 0, -9..=9, labels.clone()));
        }
        let spec = params.table.spec(Row::MapFifthsExtension.index());
        assert_eq!(spec.parse("-90"), Some(spec.to_host(-9.0)));
        assert_eq!(spec.parse("95"), None, "an extension lane takes whole tens");
    }

    /// The source choice belongs to the host's project state and is read on the
    /// audio thread, but is not automation: a lane splicing two unrelated
    /// signals into one spectrum history is not a useful visual control. Main
    /// remains the default so adding the auxiliary port does not restyle an
    /// existing project into silence.
    #[test]
    fn analysis_input_is_a_stable_non_automatable_main_default() {
        let params = HarmonigraphParams::default();
        assert_eq!(params.analysis_input(), AnalysisInput::Main);
        assert!(!params.table.spec(Row::AnalysisInput.index()).automatable);
    }

    /// The descriptor is what makes the host show its sidechain routing control.
    /// Reach the feature through the callback the host actually invokes. The
    /// main and sidechain samples are all distinct, so selecting either wrong
    /// plane or accidentally writing into the pass-through cannot hide behind
    /// equality. That the take records the same selected buffer is
    /// `a_sidechain_selection_reaches_the_armed_takes_audio`, which needs the
    /// configuration owner only the CLAP wrapper installs.
    #[test]
    fn a_sidechain_process_block_feeds_analysis_and_leaves_main_untouched() {
        let mut plugin = Harmonigraph::default();
        let mut input = stop_input_worker(&mut plugin);
        plugin.params.set(Row::AnalysisInput, 1.0);
        assert_eq!(plugin.params.analysis_input(), AnalysisInput::Sidechain);
        let mut main_left = [1.0, 2.0, 3.0, 4.0];
        let mut main_right = [5.0, 6.0, 7.0, 8.0];
        let expected_main_left = main_left;
        let expected_main_right = main_right;
        let sidechain_left = [11.0, 12.0, 13.0, 14.0];
        let sidechain_right = [21.0, 22.0, 23.0, 24.0];
        let expected_sidechain = [11.0, 21.0, 12.0, 22.0, 13.0, 23.0, 14.0, 24.0];

        {
            let mut main = [&mut main_left[..], &mut main_right[..]];
            let sidechain = [&sidechain_left[..], &sidechain_right[..]];
            plugin.play(&Transport::default(), Audio { main: &mut main, aux: &[&sidechain] });
        }

        assert_eq!(main_left, expected_main_left, "process changed the main left pass-through");
        assert_eq!(main_right, expected_main_right, "process changed the main right pass-through");
        assert_eq!(
            input.drain_analysis_audio_for_test().as_slice(),
            &expected_sidechain,
            "the live analyzer received Main instead of Sidechain",
        );
        assert_eq!(plugin.samples_processed, expected_main_left.len() as u64);
    }

    /// Source selection is explicit and block-wide. Silence says nothing about
    /// connectivity, so Sidechain chooses the auxiliary value even when it is
    /// zero; only a structurally missing buffer takes the defensive Main path.
    #[test]
    fn analysis_input_selection_never_falls_back_on_signal_content() {
        let (main, sidechain) = (11, 0);
        assert_eq!(*selected_analysis_input(&main, Some(&sidechain), AnalysisInput::Main), main,);
        assert_eq!(
            *selected_analysis_input(&main, Some(&sidechain), AnalysisInput::Sidechain),
            sidechain,
            "a silent sidechain fell back to the main input",
        );
        assert_eq!(
            *selected_analysis_input(&main, None, AnalysisInput::Sidechain),
            main,
            "a missing auxiliary buffer has no safe source except Main",
        );
    }

    /// The three ways a block decides where its take timestamps start. The
    /// middle rung is the one that matters: a take stamped on the plugin's own
    /// clock rather than the transport's lines up with no bounce of the same
    /// song, and nothing looks wrong until the audio is dropped underneath it.
    #[test]
    fn a_take_origin_prefers_the_transport_over_the_local_clock() {
        // Disarmed: the recorder is not consulted, whatever the host reports.
        assert_eq!(origin_source(false, Some(12.0), 22_050, 44_100.0), OriginSource::Idle);
        assert_eq!(origin_source(false, None, 22_050, 44_100.0), OriginSource::Idle);

        // Armed, and the host reports a position: that position, untouched.
        assert_eq!(
            origin_source(true, Some(12.0), 22_050, 44_100.0),
            OriginSource::Transport(12.0)
        );

        // Armed, and the host reports nothing at all: the plugin's own sample
        // counter in seconds, so "just record what I play" still works.
        assert_eq!(origin_source(true, None, 22_050, 44_100.0), OriginSource::LocalClock(0.5));
    }

    /// The bar the stop trigger compares against, off each of the three shapes
    /// of transport a host actually hands over.
    ///
    /// The 6/8 rung is the one that earns its place: `pos_beats` is in QUARTER
    /// notes, so a bar there is three of them and not six, and reading the
    /// numerator as the bar length would put the stop bar half again too far
    /// along in every compound meter.
    #[test]
    fn a_bar_position_counts_from_the_songs_start_in_whatever_meter() {
        let mut transport = Transport::default();
        assert_eq!(bar_position(&transport), None, "no beats timeline, no bar");

        // 4/4, no bar fields: the beat count over the meter. Beat 258 is two
        // quarter notes into the arranger's bar 65.
        transport.pos_beats = Some(258.0);
        transport.time_sig_numerator = Some(4);
        transport.time_sig_denominator = Some(4);
        assert_eq!(bar_position(&transport), Some(64.5));

        // 6/8: three quarter notes to the bar, so the same beat count is
        // further along than the numerator alone would say.
        transport.pos_beats = Some(6.0);
        transport.time_sig_numerator = Some(6);
        transport.time_sig_denominator = Some(8);
        assert_eq!(bar_position(&transport), Some(2.0));

        // With the bar fields, the bar number is taken as given and only the
        // offset into it is divided — which is what survives a meter change
        // partway through a song.
        transport.pos_beats = Some(6.0);
        transport.bar_number = Some(9);
        transport.bar_start_pos_beats = Some(4.5);
        assert_eq!(bar_position(&transport), Some(9.5));
    }

    /// The bar Yan types and the bar the transport reports are the same bar.
    ///
    /// This is the one test that spans the base change, and its absence is what
    /// let an off-by-one ship: `stop_bar` is the arranger's 1-based number and
    /// `bar_position` counts the song's first bar as zero, but every other test
    /// is written in its own side's units and passes either way. Asserting the
    /// two against each other is the only shape that fails when the conversion
    /// in `RenderConfig::stop_at_bar` goes missing.
    #[test]
    fn the_stop_bar_names_the_bar_the_transport_is_reporting() {
        let config = harmonigraph_ui::RenderConfig {
            trigger: harmonigraph_ui::RenderTrigger::AtBar,
            stop_bar: 65.0,
            ..Default::default()
        };
        // Where Bitwig's arranger says bar 65 in 4/4: 64 bars of four quarter
        // notes behind it.
        let transport = Transport {
            pos_beats: Some(64.0 * 4.0),
            time_sig_numerator: Some(4),
            time_sig_denominator: Some(4),
            ..Default::default()
        };
        assert_eq!(
            bar_position(&transport),
            config.stop_at_bar(),
            "asking to stop at bar 65 must stop where the host says bar 65 is"
        );
    }

    /// One event, two clocks. The ring uses continuous presentation seconds,
    /// which is what the editor's ClockMapper reads; the take is
    /// stamped on the transport, which is what lets it be lined up against a
    /// bounce later. Collapsing them into one would look right on screen and
    /// put every recorded take at the wrong song position.
    ///
    /// Every constant here divides to an exact binary fraction, which is what
    /// lets `assert_eq!` compare f64 at all. Picking more realistic numbers — a
    /// 512-sample block, an odd offset at 44.1 kHz — lands off a representable
    /// value and fails on the last bit with the implementation still correct.
    #[test]
    fn queued_history_keeps_its_clock_across_the_actual_plugin_reset() {
        let mut plugin = Harmonigraph::default();
        let mut input = stop_input_worker(&mut plugin);
        let shared = plugin.editor_shared.clone();
        let mut shared = shared.lock();
        let shared = &mut *shared;
        plugin.clock.set_end(11.0);
        plugin.samples_processed = 48_000;
        plugin
            .take
            .publish_note(CoreNoteEvent::on(10.0, SourceId::DIRECT, 0, 60, 0.8).into())
            .expect_both();
        plugin.reset();
        assert_eq!(plugin.samples_processed, 0);
        plugin.sample_rate = 96_000.0;
        let next = plugin.clock.end() + 48_000.0 / plugin.sample_rate;
        plugin
            .take
            .publish_note(CoreNoteEvent::on(next, SourceId::DIRECT, 0, 60, 0.8).into())
            .expect_both();
        plugin.take.publish_clock(12.0);
        let deadline = Instant::now() + ANALYSIS_DEADLINE;
        while shared.ui.picture.runtime.tracker.roll().notes().count() < 2
            && Instant::now() < deadline
        {
            input.drain(&mut shared.clock, &mut shared.ui.picture, &mut Vec::new(), 22.0);
            std::thread::yield_now();
        }
        let notes: Vec<_> =
            shared.ui.picture.runtime.tracker.roll().notes().map(|n| (n.start, n.end)).collect();
        assert_eq!(notes, [(20.0, Some(21.0)), (21.5, None)]);
    }

    #[test]
    fn analyzer_source_time_ignores_transport_loops_stops_and_missing_positions() {
        let mut plugin = Harmonigraph::default();
        let mut input = stop_input_worker(&mut plugin);
        plugin.sample_rate = 48_000.0;
        let shared = plugin.editor_shared.clone();
        let mut shared = shared.lock();
        let shared = &mut *shared;
        let mut left = vec![0.5; 10_000];
        let mut right = vec![-0.5; 10_000];
        let mut transport = Transport::default();
        for (playing, position) in [(true, Some(100_000.0)), (true, Some(0.0)), (false, None)] {
            transport.playing = playing;
            transport.pos_seconds = position.map(|samples: f64| samples / 48_000.0);
            let mut main = [&mut left[..], &mut right[..]];
            plugin.clock.step(10_000, plugin.sample_rate);
            plugin.play(&transport, Audio { main: &mut main, aux: &[] });
        }
        let columns =
            input.drain_columns(&mut shared.clock, &mut shared.ui.picture, plugin.clock.end());
        assert!(columns.len() > 40, "all three callbacks must reach the analyzer");
        let before = shared.ui.picture.runtime.spectrum.columns_taken();
        let times: Vec<_> = columns.iter().map(|(time, _)| *time).collect();
        for pair in times.windows(2) {
            assert!((pair[1] - pair[0] - 0.008).abs() < 1e-12, "transport moved the source grid");
        }
        // An actual plugin reset, unlike project transport, must discard the
        // carried FFT window. A tiny next callback cannot add another column.
        plugin.reset();
        let mut left = [0.0; 384];
        let mut right = [0.0; 384];
        let mut main = [&mut left[..], &mut right[..]];
        plugin.clock.step(384, plugin.sample_rate);
        plugin.play(&transport, Audio { main: &mut main, aux: &[] });
        input.drain(&mut shared.clock, &mut shared.ui.picture, &mut Vec::new(), plugin.clock.end());
        assert_eq!(shared.ui.picture.runtime.spectrum.columns_taken(), before);
    }

    /// A take's WAV is always stereo. A mono host input is DUPLICATED into both
    /// planes rather than written as half a frame, which would put every
    /// subsequent frame out of step with the notes it is meant to match.
    #[test]
    fn a_mono_input_is_duplicated_into_both_take_channels() {
        assert_eq!(take_right_channel(1), 0, "mono reads plane 0 twice");
        assert_eq!(take_right_channel(2), 1);
        // A host offering more than two planes still writes a stereo take.
        assert_eq!(take_right_channel(8), 1);
    }

    /// One stereo block through `play`, at song seconds `position`.
    fn tape_block(
        plugin: &mut Harmonigraph,
        position: f64,
        playing: bool,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        let transport = Transport { pos_seconds: Some(position), playing, ..Default::default() };
        let mut main = [left, right];
        plugin.play(&transport, Audio { main: &mut main, aux: &[] });
    }

    /// Click in Tape mode, and wait until the input worker has passed the
    /// request on: two whole rounds, so one began after it was set.
    fn ask(plugin: &Harmonigraph, request: harmonigraph_ui::tape::TapeRequest) {
        plugin.editor_shared.lock().ui.picture.runtime.tape.request = Some(request);
        let rounds = plugin.input_worker.completed_rounds();
        assert!(a_round_after(plugin, rounds + 1), "the worker never ran a round");
    }

    /// Blocks of `frames` from song seconds `from`, as many as fit in
    /// `seconds`.
    fn song_blocks(plugin: &Harmonigraph, from: f64, seconds: f64, frames: usize) -> Vec<f64> {
        let block = frames as f64 / plugin.sample_rate;
        (0..(seconds / block).ceil() as usize).map(|i| from + i as f64 * block).collect()
    }

    /// The plugin's first write to the main buffer must leave it exactly as
    /// the host handed it whenever no tape sounds: nothing asked, or a cue
    /// somewhere nothing was recorded. Bits, not values, because
    /// even adding a zero would turn a -0.0 into a +0.0, and a NaN payload is
    /// a value nothing compares equal.
    #[test]
    fn with_no_tape_sounding_the_main_output_is_the_input_bit_for_bit() {
        let odd = [
            -0.0f32,
            f32::from_bits(0x7fc0_1234),
            f32::MIN_POSITIVE / 2.0,
            f32::INFINITY,
            0.25,
            -1.5,
            0.0,
            1e-30,
        ];
        let input: Vec<f32> = (0..512).map(|i| odd[i % odd.len()]).collect();
        let mut plugin = Harmonigraph::default();
        let mut position = 10.0;
        for asked in [false, true] {
            let mut blocks: Vec<(f64, bool)> =
                song_blocks(&plugin, position, 0.1, 512).into_iter().map(|p| (p, true)).collect();
            blocks.extend(std::iter::repeat_n((position, false), 40));
            let mut cued = false;
            for (i, (at, playing)) in blocks.into_iter().enumerate() {
                if asked && i == 30 {
                    ask(&plugin, harmonigraph_ui::tape::TapeRequest::Cue(position + 100.0));
                }
                let (mut left, mut right) = (input.clone(), input.clone());
                tape_block(&mut plugin, at, playing, &mut left, &mut right);
                cued |= plugin.tape.sounding();
                let bits = |v: &[f32]| v.iter().map(|s| s.to_bits()).collect::<Vec<_>>();
                assert_eq!(bits(&left), bits(&input), "asked {asked}, block at {at}");
                assert_eq!(bits(&right), bits(&input), "asked {asked}, block at {at}");
            }
            assert_eq!(cued, asked, "the cue must have reached the deck");
            position += 200.0;
        }
    }

    /// The whole path through the real input worker: rolling blocks reach the
    /// tape through the analyzer's ring, and a Tape mode cue plays them back
    /// into the main buffer while it sounds; the transport
    /// starting cuts it and the output is the input again.
    #[test]
    fn a_tape_tab_cue_plays_back_what_the_worker_recorded() {
        let mut plugin = Harmonigraph::default();
        let rate = plugin.sample_rate;
        // Unique per song frame and channel.
        let song = |frame: usize, channel: usize| {
            let value = (frame + 1) as f32 * 1e-5;
            if channel == 0 {
                value
            } else {
                -value
            }
        };
        // One second from the song's start: inside the analyzer ring, so a
        // worker that has not run yet loses nothing.
        for (i, at) in song_blocks(&plugin, 0.0, 1.0, 512).into_iter().enumerate() {
            let mut left: Vec<f32> = (0..512).map(|f| song(i * 512 + f, 0)).collect();
            let mut right: Vec<f32> = (0..512).map(|f| song(i * 512 + f, 1)).collect();
            tape_block(&mut plugin, at, true, &mut left, &mut right);
            assert!(!plugin.tape.sounding());
        }
        let silence = |plugin: &mut Harmonigraph, at: f64| {
            let (mut left, mut right) = (vec![0.0; 512], vec![0.0; 512]);
            tape_block(plugin, at, false, &mut left, &mut right);
            (left, right)
        };
        for _ in 0..40 {
            silence(&mut plugin, 0.0);
        }
        // Two whole rounds after the stop: one began after it was published,
        // so the pass is committed before the click.
        let rounds = plugin.input_worker.completed_rounds();
        assert!(a_round_after(&plugin, rounds + 1), "the worker never ran a round");

        // The host keeps reporting where playback stopped.
        let parked = 0.0;
        let cue = 0.5;
        ask(&plugin, harmonigraph_ui::tape::TapeRequest::Cue(cue));
        let started = Instant::now();
        let first = (cue * rate).round() as usize;
        let mut heard = Vec::new();
        while heard.len() < 2048 {
            // A worker round before each block: a cue gives up after half a
            // second of AUDIO time without a piece, so blocks fed on a timer
            // would starve it whenever a busy runner held the worker back.
            let rounds = plugin.input_worker.completed_rounds();
            assert!(a_round_after(&plugin, rounds), "the worker never ran a round");
            let (left, right) = silence(&mut plugin, parked);
            assert!(plugin.tape.sounding(), "the tape is cued or sounding");
            let start = if heard.is_empty() {
                left.iter().position(|&s| s != 0.0).unwrap_or(left.len())
            } else {
                0
            };
            heard.extend(left[start..].iter().zip(&right[start..]).map(|(&l, &r)| (l, r)));
            assert!(started.elapsed() < ANALYSIS_DEADLINE, "no tape audio arrived");
        }
        let fade = (tape::store::FADE_SECONDS * rate).ceil() as usize;
        for (i, &(l, r)) in heard.iter().enumerate().skip(fade) {
            assert_eq!((l, r), (song(first + i, 0), song(first + i, 1)), "frame {i} after the cue");
        }
        let lines: Vec<String> = plugin
            .editor_shared
            .lock()
            .ui
            .picture
            .runtime
            .console
            .lines()
            .filter(|l| l.starts_with("tape:"))
            .map(str::to_owned)
            .collect();
        assert!(
            lines.iter().any(|l| l.starts_with("tape: audition from 0.500 s (warm (from the song's start)")),
            "{lines:?}",
        );
        // The tab is shown what sounds, and where the transport stopped.
        let rounds = plugin.input_worker.completed_rounds();
        assert!(a_round_after(&plugin, rounds + 1), "the worker never ran a round");
        {
            let shared = plugin.editor_shared.lock();
            let view = &shared.ui.picture.runtime.tape;
            assert!(view.present && view.enabled && !view.rolling);
            assert_eq!(view.position, Some(parked));
            let heard = view.heard.expect("the audition is sounding");
            assert!(heard > cue && heard < cue + 0.2, "{heard}");
        }

        let input: Vec<f32> = (0..512).map(|f| f as f32).collect();
        let (mut left, mut right) = (input.clone(), input.clone());
        tape_block(&mut plugin, parked, true, &mut left, &mut right);
        assert!(!plugin.tape.sounding());
        assert_eq!((left, right), (input.clone(), input));
    }
}
