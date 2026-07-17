//! Standalone dev harness: the full UI + renderer running in a plain window
//! with a mock MIDI source, so visual iteration never requires a DAW.
//!
//! Run with `cargo run -p lattice-standalone`.
//!
//! TODO: real MIDI input (midir) and/or replaying a recorded event log.

use std::cell::Cell;
use std::time::Instant;

use lattice_core::{NoteEvent, NoteEventKind};
use lattice_ui::params::{ParamBackend, ParamKey};
use lattice_ui::SharedState;

const UI_STATE_STORAGE_KEY: &str = "midi-lattice-3d-ui-state";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_title("MIDI Lattice 3D — dev harness"),
        ..Default::default()
    };
    eframe::run_native(
        "MIDI Lattice 3D — dev harness",
        options,
        Box::new(|cc| {
            lattice_ui::theme::apply_theme(&cc.egui_ctx);
            let render_state = cc
                .wgpu_render_state
                .as_ref()
                .expect("eframe was built with the wgpu backend");
            let mut app = App::new(render_state.target_format);
            // Restore dock layout / camera / view settings from eframe's
            // storage (mirrors the plugin's persist blob).
            if let Some(serialized) =
                cc.storage.and_then(|s| s.get_string(UI_STATE_STORAGE_KEY))
            {
                app.state.load_persist(&serialized);
            }
            Ok(Box::new(app))
        }),
    )
}

struct App {
    state: SharedState,
    params: StandaloneParams,
    mock: MockMidi,
    start: Instant,
}

impl App {
    // eframe and lattice-render resolve to the same wgpu version, so
    // eframe's reported target format can be passed straight through.
    fn new(target_format: lattice_render::wgpu::TextureFormat) -> Self {
        let mut state = SharedState::new(target_format);
        state.log("dev harness started; mock MIDI is playing");
        App {
            state,
            params: StandaloneParams::default(),
            mock: MockMidi::default(),
            start: Instant::now(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = self.start.elapsed().as_secs_f64();

        let mut events = Vec::new();
        self.mock.poll(now, &mut events);
        for event in events {
            self.state.log(format!(
                "{:7.2}s ch{:<2} note {:<3} {}",
                event.time,
                event.channel + 1,
                event.note,
                match event.kind {
                    NoteEventKind::On { .. } => "on",
                    NoteEventKind::Off => "off",
                    NoteEventKind::Tuning { .. } => "tune",
                }
            ));
            self.state.tracker.handle_event(event);
        }

        lattice_ui::root_ui(ui, &mut self.state, &self.params, now);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(UI_STATE_STORAGE_KEY, self.state.save_persist());
    }
}

/// Plain-value parameter store for the harness.
struct StandaloneParams {
    values: [(ParamKey, Cell<f32>); 9],
}

impl Default for StandaloneParams {
    fn default() -> Self {
        let tuning = lattice_core::Tuning::just();
        StandaloneParams {
            values: [
                (ParamKey::COffset, Cell::new(tuning.c_offset)),
                (ParamKey::Three, Cell::new(tuning.three)),
                (ParamKey::Five, Cell::new(tuning.five)),
                (ParamKey::Seven, Cell::new(tuning.seven)),
                // Wide tolerance so 12-TET mock notes light up the justly
                // tuned lattice out of the box.
                (ParamKey::Tolerance, Cell::new(20.0)),
                (ParamKey::HighlightTime, Cell::new(1.0)),
                (ParamKey::OctaveHighlightTime, Cell::new(1.0)),
                (ParamKey::DarkestPitch, Cell::new(24.0)),
                (ParamKey::BrightestPitch, Cell::new(108.0)),
            ],
        }
    }
}

impl ParamBackend for StandaloneParams {
    fn get(&self, key: ParamKey) -> f32 {
        self.values
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.get())
            .unwrap_or(0.0)
    }

    fn set(&self, key: ParamKey, value: f32) {
        if let Some((_, v)) = self.values.iter().find(|(k, _)| *k == key) {
            v.set(value);
        }
    }
}

/// Loops a little 7-limit chord progression across a few channels.
#[derive(Default)]
struct MockMidi {
    /// (chord index, sounding) emitted last poll.
    last: Option<(usize, bool)>,
}

/// (notes, channel); one entry per chord, played round-robin.
const CHORDS: &[(&[u8], u8)] = &[
    (&[48, 60, 64, 67], 0),      // C major, doubled root
    (&[50, 57, 65, 69], 1),      // D minor 7 flavor
    (&[43, 55, 62, 67, 71], 2),  // G major spread
    (&[48, 60, 64, 67, 70], 3),  // C dominant 7
    (&[45, 57, 60, 64], 1),      // A minor
    (&[41, 53, 60, 65, 69], 4),  // F major spread
];

const CHORD_PERIOD: f64 = 2.0;
const CHORD_GATE: f64 = 1.55;

impl MockMidi {
    fn poll(&mut self, now: f64, out: &mut Vec<NoteEvent>) {
        let index = ((now / CHORD_PERIOD) as usize) % CHORDS.len();
        let sounding = now % CHORD_PERIOD < CHORD_GATE;
        let current = (index, sounding);
        if self.last == Some(current) {
            return;
        }

        // Release whatever was sounding before.
        if let Some((prev_index, true)) = self.last {
            let (notes, channel) = CHORDS[prev_index];
            for &note in notes {
                out.push(NoteEvent { time: now, channel, note, kind: NoteEventKind::Off });
            }
        }
        // Start the new chord.
        if sounding {
            let (notes, channel) = CHORDS[index];
            for &note in notes {
                out.push(NoteEvent {
                    time: now,
                    channel,
                    note,
                    kind: NoteEventKind::On { velocity: 0.8 },
                });
            }
        }
        self.last = Some(current);
    }
}
