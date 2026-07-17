//! Standalone dev harness: the full UI + renderer running in a plain window
//! with a mock MIDI source or a hardware MIDI input, so visual iteration
//! never requires a DAW.
//!
//! Run with `cargo run -p lattice-standalone`. A floating "MIDI input"
//! window picks between the mock progression and any connected port.

use std::cell::Cell;
use std::sync::mpsc;
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

/// Where the harness gets its notes.
#[derive(Clone, PartialEq, Eq)]
enum MidiSource {
    Mock,
    /// A hardware/virtual port, by name (stable across re-enumeration).
    Port(String),
}

struct App {
    state: SharedState,
    params: StandaloneParams,
    mock: MockMidi,
    start: Instant,
    source: MidiSource,
    /// Names of the known input ports (refreshed on demand).
    known_ports: Vec<String>,
    /// Held open while a hardware source is selected.
    connection: Option<midir::MidiInputConnection<()>>,
    midi_rx: mpsc::Receiver<NoteEvent>,
    midi_tx: mpsc::Sender<NoteEvent>,
}

fn enumerate_ports() -> Vec<String> {
    midir::MidiInput::new("midi-lattice-3d")
        .map(|input| {
            input
                .ports()
                .iter()
                .filter_map(|p| input.port_name(p).ok())
                .collect()
        })
        .unwrap_or_default()
}

impl App {
    /// (Re)connect to the named port; back to mock on any failure.
    fn connect(&mut self, name: &str) {
        self.connection = None;
        let Ok(input) = midir::MidiInput::new("midi-lattice-3d") else {
            self.source = MidiSource::Mock;
            return;
        };
        let Some(port) = input
            .ports()
            .into_iter()
            .find(|p| input.port_name(p).as_deref() == Ok(name))
        else {
            self.state.log(format!("MIDI port not found: {name}"));
            self.source = MidiSource::Mock;
            return;
        };

        let tx = self.midi_tx.clone();
        let epoch = self.start;
        match input.connect(
            &port,
            "lattice-in",
            move |_stamp, message, _| {
                if let Some(event) = parse_midi(message, epoch.elapsed().as_secs_f64()) {
                    let _ = tx.send(event);
                }
            },
            (),
        ) {
            Ok(connection) => {
                self.connection = Some(connection);
                self.source = MidiSource::Port(name.to_owned());
                self.state.log(format!("MIDI input: {name}"));
            }
            Err(err) => {
                self.state.log(format!("MIDI connect failed: {err}"));
                self.source = MidiSource::Mock;
            }
        }
    }
}

/// Note on/off from a raw MIDI message (velocity-0 note-on = off, per the
/// spec). Everything else is ignored for now.
fn parse_midi(message: &[u8], time: f64) -> Option<NoteEvent> {
    let (&status, rest) = message.split_first()?;
    let (&note, rest) = rest.split_first()?;
    let &velocity = rest.first()?;
    let channel = status & 0x0F;
    match status & 0xF0 {
        0x90 if velocity > 0 => Some(NoteEvent {
            time,
            channel,
            note,
            kind: NoteEventKind::On { velocity: f32::from(velocity) / 127.0 },
        }),
        0x80 | 0x90 => Some(NoteEvent { time, channel, note, kind: NoteEventKind::Off }),
        _ => None,
    }
}

impl App {
    // eframe and lattice-render resolve to the same wgpu version, so
    // eframe's reported target format can be passed straight through.
    fn new(target_format: lattice_render::wgpu::TextureFormat) -> Self {
        let mut state = SharedState::new(target_format);
        state.log("dev harness started; mock MIDI is playing");
        let (midi_tx, midi_rx) = mpsc::channel();
        App {
            state,
            params: StandaloneParams::default(),
            mock: MockMidi::default(),
            start: Instant::now(),
            source: MidiSource::Mock,
            known_ports: enumerate_ports(),
            connection: None,
            midi_rx,
            midi_tx,
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = self.start.elapsed().as_secs_f64();

        // Source picker: mock progression or a hardware port.
        let mut switch_to: Option<MidiSource> = None;
        egui::Window::new("MIDI input")
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
            .resizable(false)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.source == MidiSource::Mock, "Mock")
                        .clicked()
                    {
                        switch_to = Some(MidiSource::Mock);
                    }
                    if ui.button("rescan").on_hover_text("Re-enumerate ports").clicked() {
                        self.known_ports = enumerate_ports();
                    }
                });
                for name in self.known_ports.clone() {
                    let selected = self.source == MidiSource::Port(name.clone());
                    if ui.selectable_label(selected, &name).clicked() && !selected {
                        switch_to = Some(MidiSource::Port(name));
                    }
                }
            });
        if let Some(source) = switch_to {
            // Silence whatever the previous source left sounding.
            self.state.tracker.all_notes_off(now);
            match source {
                MidiSource::Mock => {
                    self.connection = None;
                    self.source = MidiSource::Mock;
                    self.mock = MockMidi::default();
                    self.state.log("MIDI input: mock progression");
                }
                MidiSource::Port(name) => self.connect(&name),
            }
        }

        let mut events = Vec::new();
        if self.source == MidiSource::Mock {
            self.mock.poll(now, &mut events);
        }
        while let Ok(event) = self.midi_rx.try_recv() {
            events.push(event);
        }
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
                (ParamKey::PitchClassFade, Cell::new(1.0)),
                (ParamKey::OctaveFade, Cell::new(1.0)),
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
