use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::time::Instant;

use harmonigraph_ui::SharedState;

use super::clock::ClockMapper;
use super::input::LiveInput;
use super::ASSUMED_SURFACE_FORMAT;

/// State shared between the plugin (which owns the ring buffer producer)
/// and the GUI thread. Lives for the whole plugin lifetime; the editor
/// window may open and close many times around it.
pub struct EditorShared {
    pub(crate) input: LiveInput,
    /// Crate-visible because [`crate::background`] shares this state with the
    /// frame, and shares more of it than a drainer would need: it applies the
    /// host's restored blob through this field WHOLE — dock, camera, view and
    /// every setting, off the GUI thread, while the window is shut (see
    /// [`crate::background`], and `Restore` there) — as well as logging a
    /// failure to start into the same console. Its tests read back the history
    /// it filled.
    pub(crate) ui: SharedState,
    /// When the previous GUI update ran, if it was paced; used to detect
    /// event-loop stalls. See [`note_frame`](Self::note_frame).
    last_frame: Option<Instant>,
    /// Active begin_set/end_set automation gestures, including simultaneous camera axes.
    pub(super) gesture: std::cell::Cell<[bool; harmonigraph_ui::params::ParamKey::ALL.len()]>,
    /// Take recording, driven from the Video pane's toggle.
    ///
    /// `pub(crate)` for the same reason [`ui`](Self::ui) is: the background
    /// analyzer polls the end of a take through this state while the window is
    /// shut, and its tests read back what that round decided.
    pub(crate) take: harmonigraph_record::Control,
    /// Whether the transport was rolling as of the last recorded event,
    /// for the status line. Derived, not authoritative.
    take_rolling: bool,
    /// The recorder's note count for the current take, as of the last
    /// [`poll_take_end`](Self::poll_take_end), for the status line.
    take_last_count: u64,
    /// When [`poll_take_end`](Self::poll_take_end) first found the transport
    /// stopped, on [`LiveInput::now`]'s clock, while a take is recording and
    /// it has stayed stopped since. Debounces the OnTransportStop trigger: a
    /// host reporting one still block mid-playback must not end a take.
    take_still_since: Option<f64>,
    /// Notes reached the tracker since the window's last frame, which then
    /// draws now rather than at the idle repaint. Set by whoever drains them —
    /// the input worker ([`crate::background`]) — and taken by the frame.
    pub(crate) fresh_notes: bool,
    /// The input worker could not be started, so the window's frames drain
    /// the rings themselves, as they did before it existed.
    pub(crate) frame_drains: bool,
}

impl EditorShared {
    pub fn new(
        consumer: harmonigraph_record::publication::Consumer,
        audio_consumer: crate::audio_ingress::Consumer,
        sample_rate_bits: Arc<AtomicU32>,
        take: harmonigraph_record::Control,
    ) -> Self {
        EditorShared {
            input: LiveInput {
                consumer,
                audio_consumer,
                sample_rate_bits,
                start: Instant::now(),
                clock: ClockMapper::new(),
                audio_position: None,
                audio_origin: 0.0,
                spare: Default::default(),
            },
            ui: SharedState::new(ASSUMED_SURFACE_FORMAT),
            last_frame: None,
            gesture: std::cell::Cell::new([false; harmonigraph_ui::params::ParamKey::ALL.len()]),
            take,
            take_rolling: false,
            take_last_count: 0,
            take_still_since: None,
            fresh_notes: false,
            frame_drains: false,
        }
    }

    /// Seconds the transport must be still before OnTransportStop ends a
    /// take: long enough to ride out a host reporting one stalled block (a
    /// 4096-sample block at 48 kHz is 85 ms), short enough that the render
    /// feels immediate.
    ///
    /// Counted in seconds rather than in calls to
    /// [`poll_take_end`](Self::poll_take_end), because who calls it decides
    /// how often: a frame at 144 Hz or at a quarter of 60 Hz, or the input
    /// worker every [`SHUT_POLL`](crate::background::SHUT_POLL) with the window
    /// shut. Counted in calls, the same debounce ran from 139 ms to 1.3 s
    /// (#1567). It starts at the first poll to find the transport still, so a
    /// take ends up to one poll interval after this.
    pub(crate) const STOP_SECONDS: f64 = 0.4;

    /// Reflect the Video pane's toggle into the recorder, and the
    /// recorder's progress back into the pane. Called once per frame,
    /// before `root_ui` reads the state it draws.
    pub(super) fn sync_take(&mut self, sample_rate: f32) {
        // The plugin can record; the control is hidden in shells that
        // can't (the standalone uses an env var instead).
        self.ui.workspace.interaction.take.supported = true;
        let recording = self.take.is_recording();
        if self.ui.workspace.interaction.take.recording && !recording {
            // Start from the CURRENT look, not the last-saved one: what
            // is on screen right now is what the render should reproduce.
            self.take_last_count = 0;
            // Every take records audio, rather than from a setting: the
            // render uses the selected analysis input as the spectrogram,
            // aligned to the picture by construction (no bounce, no offset).
            // Silent-but-harmless if no audio reaches that input.
            self.take.start(sample_rate, self.ui.picture.appearance.serialize());
        } else if !self.ui.workspace.interaction.take.recording && recording {
            self.take.stop(harmonigraph_record::RenderRequest::recorded());
        }

        self.ui.workspace.interaction.take.last_take = self.take.last_take();
        for action in std::mem::take(&mut self.ui.workspace.interaction.take.export_actions) {
            match action {
                harmonigraph_ui::ExportAction::Queue { take, appearance, render } => {
                    self.take.queue_export(
                        take,
                        harmonigraph_record::RenderRequest::render_now(&render, appearance),
                    )
                }
                harmonigraph_ui::ExportAction::Cancel(id) => self.take.cancel_export(id),
                harmonigraph_ui::ExportAction::Retry(id) => self.take.retry_export(id),
                harmonigraph_ui::ExportAction::ClearFinished => self.take.clear_finished_exports(),
                harmonigraph_ui::ExportAction::OpenTakeFolder => {
                    open_folder(&harmonigraph_record::take_dir())
                }
                harmonigraph_ui::ExportAction::Reveal(video) => reveal_file(&video),
            }
        }

        self.poll_take_end();
        self.take.tick(self.take_rolling, self.take_last_count);
        self.ui.workspace.interaction.take.status = self.take.status();
        self.ui.workspace.interaction.take.exports = self.take.export_jobs();
        // The shell may have refused to start (unwritable directory);
        // don't leave the indicator claiming otherwise.
        self.ui.workspace.interaction.take.recording = self.take.is_recording();
        // Steady dot vs. breathing one: whether capture is actually happening.
        self.ui.workspace.interaction.take.rolling = self.take_rolling;
    }

    /// Everything that can end a take without you clicking anything: publish
    /// the two settings the audio thread decides on, then act on whichever
    /// signal came back.
    ///
    /// **Split out of [`sync_take`](Self::sync_take) so it can also run with
    /// the editor window SHUT.** Every trigger but `OnDisarm` decides here,
    /// which meant a take armed and then left with the window closed — a Bitwig
    /// audio export is exactly that — never ended itself and never rendered, no
    /// matter which trigger was chosen. The input worker calls this every
    /// [`SHUT_POLL`](crate::background::SHUT_POLL) while the window is shut, so
    /// the decision no longer depends on anyone watching.
    ///
    /// Nothing here draws or reads a widget, which is what makes it callable
    /// from that thread: it moves settings out to the recorder and take state
    /// back into `Interaction`, and the next frame — whenever there is one —
    /// draws what it left.
    pub(crate) fn poll_take_end(&mut self) {
        self.poll_take_end_at(self.input.now());
    }

    /// [`poll_take_end`](Self::poll_take_end) at `now` on [`LiveInput::now`]'s
    /// clock, which times the OnTransportStop debounce
    /// ([`STOP_SECONDS`](Self::STOP_SECONDS)): tests date polls rather than
    /// wait for them.
    fn poll_take_end_at(&mut self, now: f64) {
        // Counted by the recorder, which every note reaches the take through:
        // the Hub publishes it there, and nothing in `process` sees it (#818).
        let count = self.take.captured();
        self.take_last_count = count;
        // The audio thread's own view, rather than inferring it from
        // events arriving: music has gaps, and a gap is not a stop.
        self.take_rolling = self.take.is_rolling();

        // One-file triggers finish at transport discontinuities.
        let trigger = self.ui.picture.appearance.render.trigger;
        let ends_at_rewind = trigger.ends_at_rewind();
        self.take.set_end_at_rewind(ends_at_rewind);
        // And the bar it ends at, which is `None` under every other trigger —
        // so a stop bar saved in a project cannot end a take recorded under one
        // of them.
        self.take.set_stop_bar(self.ui.picture.appearance.render.stop_at_bar());

        // Honor the audio-owned completion even if the trigger changed before
        // this poll. Stop still waits for every publication lane to close the
        // prefix; a forward seek carries its cutoff notice into the render.
        let ended = self.take.has_ended();
        if self.take.is_recording() && ended {
            self.ui.workspace.interaction.take.recording = false;
            self.take.stop(harmonigraph_record::RenderRequest::recorded());
        }

        // "The take is done" as soon as the transport stops, if asked —
        // so a play-through or an audio export yields a video with
        // nothing further to click.
        if self.take.is_recording()
            && self.ui.picture.appearance.render.trigger
                == harmonigraph_ui::RenderTrigger::OnTransportStop
        {
            // Only after something was actually captured: arming ahead of
            // the downbeat must not immediately end the take.
            if self.take_rolling || (!self.take.has_rolled() && count == 0) {
                self.take_still_since = None;
            } else {
                let since = *self.take_still_since.get_or_insert(now);
                if now >= since + Self::STOP_SECONDS {
                    self.ui.workspace.interaction.take.recording = false;
                    self.take.stop(harmonigraph_record::RenderRequest::recorded());
                }
            }
        } else {
            self.take_still_since = None;
        }
    }

    /// Two [`poll_take_end`](Self::poll_take_end)s a whole OnTransportStop
    /// debounce apart, the second dated that far ahead rather than waited for.
    /// Ahead of the running input worker's own polls, so it cannot be the one
    /// whose clock the debounce started on and leave this one short.
    #[cfg(test)]
    pub(crate) fn poll_through_stop_debounce(&mut self) {
        let now = self.input.now();
        self.poll_take_end_at(now);
        self.poll_take_end_at(now + Self::STOP_SECONDS);
    }

    /// Record a GUI frame, logging a console warning when the event loop
    /// stalled since the previous one (run-loop mode issues, host
    /// blocking, ...) so freezes stay attributable.
    ///
    /// Only a gap out of a `paced` frame counts: one the display link asked
    /// for, or any frame off macOS, where a fixed timer drives them all. A
    /// macOS view whose link has paused (minimized, or ordered out) runs on
    /// baseview's watchdog, a frame every 0.1 s or slower, and that cadence is
    /// the pause rather than a stall: counted, it logged a line per tick.
    pub(super) fn note_frame(&mut self, paced: bool) {
        self.note_frame_at(Instant::now(), paced);
    }

    fn note_frame_at(&mut self, now: Instant, paced: bool) {
        let gap = self.last_frame.map(|t| now.duration_since(t).as_secs_f64());
        self.last_frame = paced.then_some(now);
        if let Some(gap) = gap.filter(|g| *g > 0.1) {
            self.ui
                .picture
                .runtime
                .console
                .log(format!("frame stall: {:.0} ms between updates", gap * 1000.0));
        }
    }
}

/// Open `dir` in the file browser, creating it first: before the first take
/// it does not exist yet, and it is where that take will go anyway.
fn open_folder(dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(dir);
    browse(&[], dir);
}

/// Show `file` selected in its folder. One deleted since it was written opens
/// the folder instead, as does a Linux build, whose `xdg-open` cannot select.
fn reveal_file(file: &std::path::Path) {
    if cfg!(target_os = "macos") && file.is_file() {
        browse(&["-R"], file);
    } else if let Some(dir) = file.parent() {
        browse(&[], dir);
    }
}

/// Fire-and-forget on a thread of its own, so the GUI never waits on the file
/// browser. A failure has nowhere useful to be shown and is dropped.
fn browse(flags: &'static [&'static str], path: &std::path::Path) {
    let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let path = path.to_owned();
    std::thread::spawn(move || {
        let _ = std::process::Command::new(program).args(flags).arg(path).status();
    });
}

#[cfg(test)]
mod tests {
    use super::EditorShared;
    use harmonigraph_core::notes::SourceId;
    use std::sync::Arc;

    /// The OnTransportStop debounce ends a take a fixed time after the
    /// transport was first seen still, however many polls fall in between:
    /// here a 144 Hz window's worth, which counted in calls ended it at 139 ms.
    #[test]
    fn recording_stop_debounce_counts_seconds_not_calls() {
        let (_notes, consumer) = harmonigraph_record::publication::channel();
        let (_audio, audio_consumer) = crate::audio_ingress::channel(64);
        let (mut recorder, control) = harmonigraph_record::channel();
        let directory =
            std::env::temp_dir().join(format!("runtime-recording-cadence-{}", std::process::id()));
        let probe = harmonigraph_record::testing::worker_probe(&control, directory.clone());
        let mut shared = EditorShared::new(
            consumer,
            audio_consumer,
            Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())),
            control,
        );
        shared.ui.picture.appearance.render.trigger =
            harmonigraph_ui::RenderTrigger::OnTransportStop;
        harmonigraph_record::testing::set_renderer_program(
            &shared.take,
            directory.join("absent-renderer"),
        );
        shared.ui.workspace.interaction.take.recording = true;
        shared.sync_take(48_000.0);
        assert!(shared.take.is_recording());
        // One note captured, as the Hub publishes one into the take's first pass.
        recorder.is_armed();
        let route = harmonigraph_record::publication::Route {
            address: recorder.configuration_address(),
            time_offset: 0.0,
        };
        let on = harmonigraph_core::NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0);
        let _ = recorder.publish_note(on.into(), route);
        let first_still = 1.0;
        let frame = 1.0 / 144.0;
        let mut now = first_still;
        while now < first_still + EditorShared::STOP_SECONDS - frame / 2.0 {
            shared.poll_take_end_at(now);
            assert!(shared.take.is_recording(), "ended {:.3} s after the stop", now - first_still);
            now += frame;
        }
        assert_eq!(shared.take_still_since, Some(first_still));
        shared.poll_take_end_at(first_still + EditorShared::STOP_SECONDS);
        assert!(!shared.take.is_recording(), "still for the whole debounce, the take ends");
        drop(shared);
        drop(recorder);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !probe.finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(probe.finished());
        let _ = std::fs::remove_dir_all(directory);
    }

    /// The real arm path snapshots live appearance even when workspace state
    /// is unrelated, and later knob edits cannot rewrite the starting look.
    #[test]
    fn arming_captures_the_whole_live_appearance_without_workspace_state() {
        let (_notes, consumer) = harmonigraph_record::publication::channel();
        let (_audio, audio_consumer) = crate::audio_ingress::channel(64);
        let (recorder, control) = harmonigraph_record::channel();
        let directory =
            std::env::temp_dir().join(format!("appearance-capture-{}", std::process::id()));
        let probe = harmonigraph_record::testing::worker_probe(&control, directory.clone());
        let mut shared = EditorShared::new(
            consumer,
            audio_consumer,
            Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())),
            control,
        );
        shared.ui.picture.appearance.camera.yaw = 1.23;
        shared.ui.picture.appearance.view.max_sevens = 3;
        shared.ui.picture.appearance.spectrum.low_midi = 40.5;
        shared.ui.picture.appearance.render.short_edge = 2160;
        shared.ui.workspace.interaction.ui_scale = 1.25;
        let expected = shared.ui.picture.appearance.serialize();
        shared.ui.workspace.interaction.take.recording = true;
        shared.sync_take(44_100.0);
        assert!(shared.take.is_recording(), "{}", shared.take.status());
        shared.ui.picture.appearance = Default::default();
        // Closing the producer finalizes the writer without launching an export.
        drop(shared);
        drop(recorder);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !probe.finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(probe.finished(), "the writer must close before reading its take");
        let path = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "take"))
            .unwrap();
        let take = harmonigraph_take::Take::read(path).unwrap();
        let blob = take.header.appearance.unwrap();
        assert_eq!(take.header.sample_rate, 44_100.0);
        assert_eq!(blob, expected);
        assert!(
            !blob.contains("dock:") && !blob.contains("ui_scale:") && !blob.contains("spiral:")
        );
        let appearance = harmonigraph_ui::AppearanceDocument::parse(&blob).unwrap();
        assert_eq!(appearance.serialize(), expected);
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// A view whose display link has paused (the editor docked while Bitwig
    /// is in the background, or its own window minimized) runs on the
    /// watchdog, a frame every 0.1 s or slower. Those frames are its cadence,
    /// not stalls, and logged as stalls they filled the console in minutes.
    /// The first gap out of a paced frame is still one.
    #[test]
    fn watchdog_frames_after_a_paused_link_are_not_stalls() {
        let (_notes, consumer) = harmonigraph_record::publication::channel();
        let (_audio, audio_consumer) = crate::audio_ingress::channel(64);
        let (_recorder, control) = harmonigraph_record::channel();
        let mut shared = EditorShared::new(
            consumer,
            audio_consumer,
            Arc::new(super::AtomicU32::new(48_000.0f32.to_bits())),
            control,
        );
        let start = std::time::Instant::now();
        let at = |seconds: f64| start + std::time::Duration::from_secs_f64(seconds);
        let stalls = |shared: &EditorShared| {
            let console = &shared.ui.picture.runtime.console;
            console.lines().filter(|line| line.starts_with("frame stall")).count()
        };
        shared.note_frame_at(at(0.0), true);
        shared.note_frame_at(at(0.007), true);
        assert_eq!(stalls(&shared), 0);
        shared.note_frame_at(at(0.5), true);
        assert_eq!(stalls(&shared), 1, "a gap between two link frames is a stall");
        shared.note_frame_at(at(0.7), false);
        assert_eq!(stalls(&shared), 2, "so is the first gap out of a link frame");
        for tick in 1..=50 {
            shared.note_frame_at(at(0.7 + 0.21 * f64::from(tick)), false);
        }
        shared.note_frame_at(at(11.5), true);
        shared.note_frame_at(at(11.507), true);
        assert_eq!(stalls(&shared), 2, "neither the watchdog's ticks nor the link's return");
    }
}
