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
            app.recorder = Recorder::from_env(&app.state);
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
    /// Fake audio for the Spectral pane's analyzer (see MockSynth).
    synth: MockSynth,
    synth_buf: Vec<f32>,
    start: Instant,
    source: MidiSource,
    /// Names of the known input ports (refreshed on demand).
    known_ports: Vec<String>,
    /// Held open while a hardware source is selected.
    connection: Option<midir::MidiInputConnection<()>>,
    midi_rx: mpsc::Receiver<RawMidi>,
    midi_tx: mpsc::Sender<RawMidi>,
    /// Raw-message decoder for the hardware path (bend state lives here).
    decoder: MidiDecoder,
    /// Echo every note event to the console (hardware MIDI can flood the
    /// scrollback during real playing; toggle in the source picker).
    log_events: bool,
    /// Scripted self-screenshot (see [`SelfShot`]); None in normal runs.
    self_shot: Option<SelfShot>,
    /// Take recorder (see [`Recorder`]); None unless LATTICE_TAKE is set.
    recorder: Option<Recorder>,
}

/// Env-gated take recorder: `LATTICE_TAKE=/path/piece.take` writes every
/// note event this harness sees, in the format `lattice-offline` replays.
///
/// This is the cheap half of the offline video pipeline, and the half
/// that needs no DAW at all — play into the harness from a keyboard or
/// over an IAC bus, then render the take at 4K afterwards. It is also how
/// the renderer gets exercised without depending on Bitwig's export
/// behavior, which is the one part of the pipeline this repo cannot
/// verify on its own.
///
/// The harness has no audio worth recording (the spectrum runs off a mock
/// synth), so takes from here are note-only; pass the real bounce to
/// `lattice-offline --audio` at render time.
struct Recorder {
    writer: lattice_take::Writer,
    /// Last value seen for each parameter, so only changes are written.
    last: [f32; ParamKey::ALL.len()],
}

impl Recorder {
    fn from_env(state: &SharedState) -> Option<Recorder> {
        let path = std::env::var("LATTICE_TAKE").ok()?;
        let header = lattice_take::Header {
            sample_rate: SYNTH_RATE as f32,
            ui_state: Some(state.save_persist()),
            source: "lattice-standalone".into(),
            ..Default::default()
        };
        match lattice_take::Writer::create(&path, &header) {
            Ok(writer) => {
                eprintln!("recording take: {path}");
                Some(Recorder { writer, last: [f32::NAN; ParamKey::ALL.len()] })
            }
            Err(err) => {
                eprintln!("could not record take to {path}: {err}");
                None
            }
        }
    }

    fn note(&mut self, event: &NoteEvent) {
        let kind = match event.kind {
            NoteEventKind::On { velocity } => lattice_take::NoteKind::On { velocity },
            NoteEventKind::Off => lattice_take::NoteKind::Off,
            NoteEventKind::Tuning { semitones } => lattice_take::NoteKind::Tuning { semitones },
            NoteEventKind::AllOff => lattice_take::NoteKind::AllOff,
        };
        let _ = self.writer.note(lattice_take::NoteRecord {
            t: event.time,
            channel: event.channel,
            note: event.note,
            kind,
        });
    }

    /// Write any parameter that moved since the last frame.
    fn params(&mut self, params: &StandaloneParams, now: f64) {
        for (i, key) in ParamKey::ALL.into_iter().enumerate() {
            let value = params.get(key);
            if self.last[i] != value {
                self.last[i] = value;
                let _ = self.writer.param(lattice_take::ParamRecord {
                    t: now,
                    id: key.id().to_string(),
                    value,
                });
            }
        }
    }
}

/// Env-gated self-screenshot for scripted visual verification:
/// `LATTICE_SCREENSHOT=/path/out.png` renders normally, captures the
/// window after `LATTICE_SCREENSHOT_DELAY_S` seconds (default 2 — long
/// enough for the mock progression to light things up), writes the PNG,
/// and exits. The app captures its own swapchain via egui, so macOS
/// screen-recording permissions never get in the way of automation.
struct SelfShot {
    path: String,
    at: f64,
    requested: bool,
}

impl SelfShot {
    fn from_env() -> Option<Self> {
        let path = std::env::var("LATTICE_SCREENSHOT").ok()?;
        let at = std::env::var("LATTICE_SCREENSHOT_DELAY_S")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2.0);
        Some(SelfShot { path, at, requested: false })
    }

    /// One tick of the capture state machine; call every frame.
    fn step(&mut self, ctx: &egui::Context, now: f64) {
        if !self.requested && now >= self.at {
            self.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let [w, h] = image.size;
            if let Err(err) = image::save_buffer(
                &self.path,
                image.as_raw(),
                w as u32,
                h as u32,
                image::ExtendedColorType::Rgba8,
            ) {
                eprintln!("screenshot failed: {err}");
                std::process::exit(1);
            }
            eprintln!("screenshot saved: {}", self.path);
            // Exit without eframe's save(): a scripted run must not
            // overwrite the interactively persisted UI state.
            std::process::exit(0);
        }
    }
}

fn enumerate_ports() -> Result<Vec<String>, midir::InitError> {
    let input = midir::MidiInput::new("midi-lattice-3d")?;
    Ok(input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect())
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
                // Channel-voice messages are 2-3 bytes; SysEx and realtime
                // are not decoded, so don't ship them across the channel.
                if (2..=3).contains(&message.len()) {
                    let mut data = [0u8; 3];
                    data[..message.len()].copy_from_slice(message);
                    let _ = tx.send(RawMidi { time: epoch.elapsed().as_secs_f64(), data });
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

/// A channel-voice message as it came off the wire, timestamped on the
/// harness clock. Decoding happens on the GUI thread (see [`MidiDecoder`])
/// so the bend state and the bend-range setting live in one place.
#[derive(Clone, Copy, Debug)]
struct RawMidi {
    time: f64,
    /// Status + up to two data bytes; 2-byte messages pad with zero.
    data: [u8; 3],
}

/// Stateful decoder for the hardware-MIDI path. Beyond note on/off it
/// gives the standalone the same per-note tuning the plugin gets from its
/// host: channel pitch bend becomes a `Tuning` event for every note held
/// on that channel — which is exactly per-note under MPE, where each note
/// sits alone on a member channel — and CC 120/123 release everything.
struct MidiDecoder {
    /// Held (channel, note) pairs, for routing channel bend to notes.
    held: Vec<(u8, u8)>,
    /// Latest bend per channel, in semitones.
    bend: [f32; 16],
    /// Semitones at full bend deflection. Non-MPE keyboards default to
    /// +/-2; MPE member channels conventionally use +/-48. Picked in the
    /// MIDI input window.
    bend_range: f32,
}

impl Default for MidiDecoder {
    fn default() -> Self {
        MidiDecoder { held: Vec::new(), bend: [0.0; 16], bend_range: 2.0 }
    }
}

impl MidiDecoder {
    /// Decode one message, appending the resulting events.
    fn decode(&mut self, raw: RawMidi, out: &mut Vec<NoteEvent>) {
        let RawMidi { time, data: [status, d1, d2] } = raw;
        let channel = status & 0x0F;
        match status & 0xF0 {
            0x90 if d2 > 0 => {
                let (note, velocity) = (d1, d2);
                if !self.held.contains(&(channel, note)) {
                    self.held.push((channel, note));
                }
                out.push(NoteEvent {
                    time,
                    channel,
                    note,
                    kind: NoteEventKind::On { velocity: f32::from(velocity) / 127.0 },
                });
                // MPE sends the bend before the note-on; a note born on a
                // bent channel must start at the bent pitch.
                if self.bend[channel as usize] != 0.0 {
                    out.push(NoteEvent {
                        time,
                        channel,
                        note,
                        kind: NoteEventKind::Tuning { semitones: self.bend[channel as usize] },
                    });
                }
            }
            // Velocity-0 note-on is a note-off, per the spec.
            0x80 | 0x90 => {
                let note = d1;
                self.held.retain(|&held| held != (channel, note));
                out.push(NoteEvent { time, channel, note, kind: NoteEventKind::Off });
            }
            0xE0 => {
                // 14-bit bend, center 8192.
                let value = u16::from(d1) | (u16::from(d2) << 7);
                let semitones =
                    (f32::from(value) - 8192.0) / 8192.0 * self.bend_range;
                self.bend[channel as usize] = semitones;
                for &(ch, note) in &self.held {
                    if ch == channel {
                        out.push(NoteEvent {
                            time,
                            channel,
                            note,
                            kind: NoteEventKind::Tuning { semitones },
                        });
                    }
                }
            }
            // All sound off / all notes off. The tracker's release is
            // global rather than per-channel; close enough for a panic
            // button on a dev harness.
            0xB0 if d1 == 120 || d1 == 123 => {
                self.held.clear();
                out.push(NoteEvent { time, channel, note: 0, kind: NoteEventKind::AllOff });
            }
            _ => {}
        }
    }

    /// Forget held notes and bends (source switches, reconnects).
    fn reset(&mut self) {
        self.held.clear();
        self.bend = [0.0; 16];
    }
}

impl App {
    // eframe and lattice-render resolve to the same wgpu version, so
    // eframe's reported target format can be passed straight through.
    fn new(target_format: lattice_render::wgpu::TextureFormat) -> Self {
        let mut state = SharedState::new(target_format);
        state.log("dev harness started; mock MIDI is playing");
        // An empty port list with no message would read as "no MIDI gear";
        // tell the difference when enumeration itself failed.
        let known_ports = enumerate_ports().unwrap_or_else(|err| {
            state.log(format!("MIDI port enumeration failed: {err}"));
            Vec::new()
        });
        let (midi_tx, midi_rx) = mpsc::channel();
        App {
            state,
            params: StandaloneParams::default(),
            mock: MockMidi::default(),
            synth: MockSynth::default(),
            synth_buf: Vec::new(),
            start: Instant::now(),
            source: MidiSource::Mock,
            known_ports,
            connection: None,
            midi_rx,
            midi_tx,
            decoder: MidiDecoder::default(),
            log_events: true,
            self_shot: SelfShot::from_env(),
            // Needs the restored UI state in the header, so it is
            // installed after load_persist rather than here.
            recorder: None,
        }
    }

    /// Floating source-picker window. Returns the source the user picked,
    /// if any (applied by the caller so the switch happens outside the
    /// window closure).
    fn source_picker(&mut self, ctx: &egui::Context) -> Option<MidiSource> {
        let mut switch_to: Option<MidiSource> = None;
        egui::Window::new("MIDI input")
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -12.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(self.source == MidiSource::Mock, "Mock")
                        .clicked()
                    {
                        switch_to = Some(MidiSource::Mock);
                    }
                    if ui.button("rescan").on_hover_text("Re-enumerate ports").clicked() {
                        match enumerate_ports() {
                            Ok(ports) => self.known_ports = ports,
                            Err(err) => self
                                .state
                                .log(format!("MIDI port enumeration failed: {err}")),
                        }
                    }
                    ui.checkbox(&mut self.log_events, "Log events");
                });
                // Pitch-bend deflection of the connected hardware. 2 is
                // the non-MPE default; 48 is the MPE member-channel
                // convention (what MPE controllers actually send).
                ui.horizontal(|ui| {
                    ui.label("Bend range").on_hover_text(
                        "Semitones at full pitch-bend deflection; match the \
                         controller (2 = plain keyboards, 48 = MPE)",
                    );
                    for range in [2.0, 12.0, 24.0, 48.0] {
                        if ui
                            .selectable_label(
                                self.decoder.bend_range == range,
                                format!("{range:.0}"),
                            )
                            .clicked()
                        {
                            self.decoder.bend_range = range;
                        }
                    }
                });
                for name in &self.known_ports {
                    let selected =
                        matches!(&self.source, MidiSource::Port(current) if current == name);
                    if ui.selectable_label(selected, name).clicked() && !selected {
                        switch_to = Some(MidiSource::Port(name.clone()));
                    }
                }
            });
        switch_to
    }

    fn switch_source(&mut self, source: MidiSource, now: f64) {
        // Silence whatever the previous source left sounding.
        self.state.tracker.all_notes_off(now);
        self.decoder.reset();
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

    /// Gather this frame's events from the active source: the mock
    /// progression's poll plus anything the midir callback thread queued
    /// (decoded here, on the thread that owns the bend state).
    fn collect_events(&mut self, now: f64) -> Vec<NoteEvent> {
        let mut events = Vec::new();
        if self.source == MidiSource::Mock {
            self.mock.poll(now, &mut events);
        }
        while let Ok(raw) = self.midi_rx.try_recv() {
            self.decoder.decode(raw, &mut events);
        }
        events
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = self.start.elapsed().as_secs_f64();

        if let Some(source) = self.source_picker(ui.ctx()) {
            self.switch_source(source, now);
        }

        for event in self.collect_events(now) {
            if self.log_events {
                self.state.log(format!(
                    "{:7.2}s ch{:<2} note {:<3} {}",
                    event.time,
                    event.channel + 1,
                    event.note,
                    match event.kind {
                        NoteEventKind::On { .. } => "on",
                        NoteEventKind::Off => "off",
                        NoteEventKind::Tuning { .. } => "tune",
                        NoteEventKind::AllOff => "all-off",
                    }
                ));
            }
            if let Some(recorder) = &mut self.recorder {
                recorder.note(&event);
            }
            self.state.tracker.handle_event(event);
        }
        if let Some(recorder) = &mut self.recorder {
            recorder.params(&self.params, now);
        }

        // The harness has no real audio; synthesize the held notes so the
        // Spectral pane's audio overlay is demoable without a DAW.
        if self.state.spectrum_config.show_audio {
            self.synth.render(&self.state.tracker, now, &mut self.synth_buf);
            self.state
                .spectrum
                .push_samples(&self.synth_buf, SYNTH_RATE as f32, now);
        } else {
            self.synth.reset();
        }

        lattice_ui::root_ui(ui, &mut self.state, &self.params, now);

        if let Some(shot) = &mut self.self_shot {
            shot.step(ui.ctx(), now);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(UI_STATE_STORAGE_KEY, self.state.save_persist());
    }
}

/// Plain-value parameter store for the harness: one Cell per ParamKey,
/// built from ParamKey::ALL so a newly added parameter can't be missed.
struct StandaloneParams {
    values: [(ParamKey, Cell<f32>); ParamKey::ALL.len()],
}

impl Default for StandaloneParams {
    fn default() -> Self {
        let params = StandaloneParams {
            values: ParamKey::ALL.map(|key| (key, Cell::new(key.default_value()))),
        };
        // Demo overrides, applied on top of the shared defaults so the
        // deliberate divergence from the plugin stays visible as a delta:
        // a justly tuned lattice, with the tolerance wide enough that the
        // 12-TET mock notes light it up out of the box.
        let just = lattice_core::Tuning::just();
        params.set(ParamKey::COffset, just.c_offset_cents());
        params.set(ParamKey::Three, just.three_cents());
        params.set(ParamKey::Five, just.five_cents());
        params.set(ParamKey::Seven, just.seven_cents());
        params.set(ParamKey::Tolerance, 20.0);
        params
    }
}

impl StandaloneParams {
    fn cell(&self, key: ParamKey) -> &Cell<f32> {
        self.values
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
            .expect("built from ParamKey::ALL, so every key is present")
    }
}

impl ParamBackend for StandaloneParams {
    fn get(&self, key: ParamKey) -> f32 {
        self.cell(key).get()
    }

    fn set(&self, key: ParamKey, value: f32) {
        self.cell(key).set(value);
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

/// Sample rate of the mock synth's imaginary audio.
const SYNTH_RATE: f64 = 48_000.0;

/// Additive synth over the held notes — fundamental plus two harmonics,
/// analyzed by the Spectral pane but never heard. The harmonics are the
/// demo: a held C also deposits pitch-class energy at the fifth (3rd
/// harmonic) and major third (5th... within the first two, at 702c),
/// showing the fold doing something MIDI bars can't.
#[derive(Default)]
struct MockSynth {
    /// Wall time up to which samples have been rendered.
    rendered_to: Option<f64>,
}

impl MockSynth {
    /// Per-frame cap: a stalled frame must not synthesize seconds.
    const MAX_CHUNK: usize = 9_600; // 200 ms

    /// Fill `out` with the samples from the previous call's time up to
    /// `now`. Phase comes from absolute time, so held notes stay
    /// continuous across frames without per-voice state.
    fn render(&mut self, tracker: &lattice_core::NoteTracker, now: f64, out: &mut Vec<f32>) {
        out.clear();
        let Some(from) = self.rendered_to.replace(now) else {
            return; // first frame after enabling: only set the epoch
        };
        let n = (((now - from) * SYNTH_RATE) as usize).min(Self::MAX_CHUNK);

        let voices: Vec<(f64, f64)> = tracker
            .voices()
            .filter(|v| v.state == lattice_core::VoiceState::Held)
            .map(|v| {
                // lattice_core::spectrum::midi_to_hz, in f64 (this synth's
                // phase accumulator is f64, and it is only ever FFT'd).
                let freq = 440.0 * f64::from((v.pitch - 69.0) / 12.0).exp2();
                (freq, f64::from(v.velocity) * 0.1)
            })
            .collect();

        out.reserve(n);
        for i in 0..n {
            let t = from + (i + 1) as f64 / SYNTH_RATE;
            let mut sample = 0.0f64;
            for &(freq, amp) in &voices {
                let phase = std::f64::consts::TAU * freq * t;
                sample += amp
                    * (phase.sin() + 0.35 * (2.0 * phase).sin() + 0.2 * (3.0 * phase).sin());
            }
            out.push(sample as f32);
        }
    }

    /// Forget the clock while the overlay is off, so re-enabling starts
    /// fresh instead of rendering the whole gap.
    fn reset(&mut self) {
        self.rendered_to = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode a sequence of raw messages at time 0.
    fn decode_all(decoder: &mut MidiDecoder, messages: &[[u8; 3]]) -> Vec<NoteEvent> {
        let mut events = Vec::new();
        for &data in messages {
            decoder.decode(RawMidi { time: 0.0, data }, &mut events);
        }
        events
    }

    #[test]
    fn decoder_handles_note_messages() {
        let mut decoder = MidiDecoder::default();
        let mut events = Vec::new();
        decoder.decode(RawMidi { time: 1.0, data: [0x90, 60, 100] }, &mut events);
        let on = events[0];
        assert_eq!((on.channel, on.note, on.time), (0, 60, 1.0));
        assert!(matches!(
            on.kind,
            NoteEventKind::On { velocity } if (velocity - 100.0 / 127.0).abs() < 1e-6
        ));

        // Velocity-0 note-on is a note-off, per the MIDI spec.
        let events = decode_all(&mut decoder, &[[0x91, 60, 0]]);
        assert_eq!((events[0].kind, events[0].channel), (NoteEventKind::Off, 1));

        // Real note-off, with the channel nibble split out.
        let events = decode_all(&mut decoder, &[[0x85, 61, 40]]);
        assert_eq!(
            (events[0].kind, events[0].channel, events[0].note),
            (NoteEventKind::Off, 5, 61)
        );

        // Unhandled channel messages decode to nothing.
        assert!(decode_all(&mut decoder, &[[0xB0, 1, 2]]).is_empty()); // plain CC
        assert!(decode_all(&mut decoder, &[[0xC0, 5, 0]]).is_empty()); // program
    }

    #[test]
    fn bend_tunes_only_the_held_notes_on_its_channel() {
        let mut decoder = MidiDecoder { bend_range: 2.0, ..MidiDecoder::default() };
        // Two notes on channel 0, one on channel 1.
        decode_all(&mut decoder, &[[0x90, 60, 100], [0x90, 64, 100], [0x91, 67, 100]]);

        // Full up-bend on channel 0: 16383 -> just under +2 semitones.
        let events = decode_all(&mut decoder, &[[0xE0, 0x7F, 0x7F]]);
        assert_eq!(events.len(), 2, "one Tuning per held channel-0 note");
        for event in &events {
            assert_eq!(event.channel, 0);
            assert!(matches!(
                event.kind,
                NoteEventKind::Tuning { semitones } if (semitones - 2.0).abs() < 0.01
            ));
        }

        // Center on channel 1: exactly 0 semitones.
        let events = decode_all(&mut decoder, &[[0xE1, 0x00, 0x40]]);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events[0].kind,
            NoteEventKind::Tuning { semitones } if semitones == 0.0
        ));

        // Released notes stop receiving bend.
        decode_all(&mut decoder, &[[0x80, 60, 0]]);
        let events = decode_all(&mut decoder, &[[0xE0, 0x00, 0x60]]);
        assert_eq!(events.len(), 1, "only the still-held channel-0 note");
        assert_eq!(events[0].note, 64);
    }

    #[test]
    fn bend_range_scales_and_mpe_note_starts_bent() {
        let mut decoder = MidiDecoder { bend_range: 48.0, ..MidiDecoder::default() };
        // MPE order: bend arrives on the member channel BEFORE the note.
        // Half up-bend (8192 + 4096 = 12288): +24 of 48 semitones.
        let events =
            decode_all(&mut decoder, &[[0xE2, 0x00, 0x60], [0x92, 60, 100]]);
        assert!(matches!(events[0].kind, NoteEventKind::On { .. }));
        assert!(matches!(
            events[1].kind,
            NoteEventKind::Tuning { semitones } if (semitones - 24.0).abs() < 0.01
        ));
        assert_eq!((events[1].channel, events[1].note), (2, 60));
    }

    #[test]
    fn all_notes_off_releases_everything() {
        let mut decoder = MidiDecoder::default();
        decode_all(&mut decoder, &[[0x90, 60, 100], [0x91, 64, 100]]);
        // CC 123 (all notes off).
        let events = decode_all(&mut decoder, &[[0xB0, 123, 0]]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, NoteEventKind::AllOff);
        // Held state is gone: a later bend tunes nothing.
        assert!(decode_all(&mut decoder, &[[0xE0, 0x7F, 0x7F]]).is_empty());
        // CC 120 (all sound off) does the same.
        decode_all(&mut decoder, &[[0x90, 60, 100]]);
        let events = decode_all(&mut decoder, &[[0xB5, 120, 0]]);
        assert_eq!(events[0].kind, NoteEventKind::AllOff);
    }

    #[test]
    fn mock_releases_each_chord_before_the_next_starts() {
        let mut mock = MockMidi::default();
        let mut events = Vec::new();

        mock.poll(0.1, &mut events);
        let first_chord = events.len();
        assert!(first_chord > 0);
        assert!(events.iter().all(|e| matches!(e.kind, NoteEventKind::On { .. })));

        // Same chord phase again: nothing new is emitted.
        events.clear();
        mock.poll(0.2, &mut events);
        assert!(events.is_empty());

        // Gate closes: exactly the first chord's notes are released.
        events.clear();
        mock.poll(CHORD_GATE + 0.01, &mut events);
        assert_eq!(events.len(), first_chord);
        assert!(events.iter().all(|e| e.kind == NoteEventKind::Off));

        // Next period: the following chord starts cleanly.
        events.clear();
        mock.poll(CHORD_PERIOD + 0.01, &mut events);
        assert!(!events.is_empty());
        assert!(events.iter().all(|e| matches!(e.kind, NoteEventKind::On { .. })));
    }

    #[test]
    fn mock_skipping_the_gap_still_releases_the_held_chord() {
        // A slow frame can jump from mid-chord straight into the next
        // period; the previous chord must still be released first.
        let mut mock = MockMidi::default();
        let mut events = Vec::new();
        mock.poll(0.1, &mut events);
        let first_chord = events.len();

        events.clear();
        mock.poll(CHORD_PERIOD + 0.1, &mut events);
        let offs = events.iter().filter(|e| e.kind == NoteEventKind::Off).count();
        assert_eq!(offs, first_chord, "held chord must be released");
        assert!(events.len() > offs, "and the new chord must start");
    }
}
