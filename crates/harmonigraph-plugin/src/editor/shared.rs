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
    /// When the previous GUI update ran; used to detect event-loop stalls.
    last_frame: Option<Instant>,
    /// Param key currently inside a begin_set/end_set automation gesture.
    pub(super) gesture: std::cell::Cell<Option<harmonigraph_ui::params::ParamKey>>,
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
    /// Consecutive frames the transport has been stopped for, while a
    /// take is recording. Debounces the OnTransportStop trigger: a host
    /// reporting one still block mid-playback must not end a take.
    take_still_frames: u32,
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
            },
            ui: SharedState::new(ASSUMED_SURFACE_FORMAT),
            last_frame: None,
            gesture: std::cell::Cell::new(None),
            take,
            take_rolling: false,
            take_last_count: 0,
            take_still_frames: 0,
        }
    }

    /// Frames the transport must be still before OnTransportStop ends a
    /// take. At the editor's repaint rate this is a fraction of a second
    /// — long enough to ride out a host reporting one stalled block,
    /// short enough that the render feels immediate.
    ///
    /// Counted in calls to [`poll_take_end`](Self::poll_take_end) rather than
    /// in frames as such, so with the window shut it is 20 rounds of the
    /// background analyzer's poll instead — 0.4 s against 0.33 s at 60 fps,
    /// which is the same fraction of a second and needs no second constant.
    pub(crate) const STOP_FRAMES: u32 = 20;

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
            // `audio: true` unconditionally, rather than from a setting: the
            // render uses the selected analysis input as the spectrogram,
            // aligned to the picture by construction (no bounce, no offset).
            // Silent-but-harmless if no audio reaches that input.
            self.take.start(sample_rate, self.ui.picture.appearance.serialize(), true);
        } else if !self.ui.workspace.interaction.take.recording && recording {
            self.take.stop(harmonigraph_record::RenderRequest::from_config(
                &self.ui.picture.appearance.render,
            ));
        }

        // "Re-render take": render the last finished take with the CURRENT settings.
        // The appearance rides along as --appearance, so the frame, bounce, and
        // offset dialed in after recording all reach the video.
        self.ui.workspace.interaction.take.last_ready = self.take.last_take().is_some();
        if std::mem::take(&mut self.ui.workspace.interaction.take.render_now) {
            self.take.render_now(harmonigraph_record::RenderRequest::render_now(
                &self.ui.picture.appearance.render,
                self.ui.picture.appearance.serialize(),
            ));
        }

        // "Cancel render": stop the renderer and drop the part-written video.
        // Nothing about the take, so the button above can start another.
        if std::mem::take(&mut self.ui.workspace.interaction.take.cancel_render) {
            self.take.cancel_render();
        }

        self.poll_take_end();
        self.take.tick(self.take_rolling, self.take_last_count);
        self.ui.workspace.interaction.take.status = self.take.status();
        self.ui.workspace.interaction.take.render_progress = self.take.render_progress();
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
    /// matter which trigger was chosen. The background analyzer already holds
    /// this lock at a 20 ms poll to keep the spectrogram filling; it calls this
    /// on the same round, so the decision no longer depends on anyone watching.
    ///
    /// Nothing here draws or reads a widget, which is what makes it callable
    /// from that thread: it moves settings out to the recorder and take state
    /// back into `Interaction`, and the next frame — whenever there is one —
    /// draws what it left.
    pub(crate) fn poll_take_end(&mut self) {
        // Counted by the recorder, whichever path a note took into the take:
        // with a configuration owner installed — every CLAP host — notes never
        // pass through the plain-MIDI arm of `process` at all (#818).
        let count = self.take.captured();
        self.take_last_count = count;
        // The audio thread's own view, rather than inferring it from
        // events arriving: music has gaps, and a gap is not a stop.
        self.take_rolling = self.take.is_rolling();

        // One-file triggers finish on a rewind after accepted forward motion.
        let trigger = self.ui.picture.appearance.render.trigger;
        let ends_at_rewind = trigger.ends_at_rewind();
        self.take.set_end_at_rewind(ends_at_rewind);
        // And the bar it ends at, which is `None` under every other trigger —
        // so a stop bar saved in a project cannot end a take recorded under one
        // of them.
        self.take.set_stop_bar(self.ui.picture.appearance.render.stop_at_bar());

        // The audio thread ended the take itself, either because the transport
        // went backwards — one pass, cut exactly at the loop boundary or at the
        // point the host took the playhead back — or because it played through
        // the stop bar. Reflect it in the toggle and render that pass. This is
        // what the export case needs and the frame-counted stop below cannot
        // give it: a host that restores the playhead does so before the
        // debounce runs out, and whatever the transport does next would
        // otherwise open a pass that ends up being the one rendered.
        let ended = (ends_at_rewind && self.take.hit_rewind()) || self.take.hit_stop_bar();
        if self.take.is_recording() && ended {
            self.ui.workspace.interaction.take.recording = false;
            self.take.stop(harmonigraph_record::RenderRequest::from_config(
                &self.ui.picture.appearance.render,
            ));
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
                self.take_still_frames = 0;
            } else {
                self.take_still_frames += 1;
                if self.take_still_frames >= Self::STOP_FRAMES {
                    self.ui.workspace.interaction.take.recording = false;
                    self.take.stop(harmonigraph_record::RenderRequest::from_config(
                        &self.ui.picture.appearance.render,
                    ));
                }
            }
        } else {
            self.take_still_frames = 0;
        }
    }

    /// Record a GUI frame, logging a console warning when the event loop
    /// stalled since the previous one (run-loop mode issues, host
    /// blocking, ...) so freezes stay attributable.
    pub(super) fn note_frame(&mut self) {
        let gap = self.last_frame.map(|t| t.elapsed().as_secs_f64());
        self.last_frame = Some(Instant::now());
        if let Some(gap) = gap.filter(|g| *g > 0.1) {
            self.ui
                .picture
                .runtime
                .console
                .log(format!("frame stall: {:.0} ms between updates", gap * 1000.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EditorShared;
    use harmonigraph_core::notes::SourceId;
    use std::sync::Arc;

    #[test]
    fn recording_stop_debounce_counts_gui_callbacks_not_runtime_ticks() {
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
        shared.ui.picture.appearance.render.renderer_path =
            directory.join("absent-renderer").to_string_lossy().into_owned();
        shared.ui.workspace.interaction.take.recording = true;
        shared.sync_take(48_000.0);
        assert!(shared.take.is_recording());
        // One note captured, as `process` would on the take's first block.
        recorder.is_armed();
        let on = harmonigraph_core::NoteEventKind::On { velocity: 1.0 };
        recorder.note(0.0, SourceId::DIRECT, 0, 60, on);
        recorder.finish_callback();
        for tick in 0..100 {
            let now = tick as f64 * 0.02;
            shared.input.drain(&mut shared.ui.picture.runtime, &shared.ui.picture.appearance, now);
            shared.ui.picture.runtime.advance_time(now, &shared.ui.picture.appearance);
        }
        assert_eq!(
            shared.take_still_frames, 0,
            "closed runtime ticks must not count toward debounce"
        );
        for _ in 0..EditorShared::STOP_FRAMES - 1 {
            shared.sync_take(48_000.0);
            assert!(shared.take.is_recording());
        }
        assert_eq!(shared.take_still_frames, EditorShared::STOP_FRAMES - 1);
        shared.sync_take(48_000.0);
        assert!(!shared.take.is_recording(), "the twentieth stopped GUI callback ends the take");
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
        shared.ui.picture.appearance.spiral.zoom = 2.75;
        shared.ui.picture.appearance.render.short_edge = 2160;
        shared.ui.picture.appearance.render.renderer_path = "a renderer (with, punctuation)".into();
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
        assert!(!blob.contains("dock:") && !blob.contains("ui_scale:"));
        let appearance = harmonigraph_ui::AppearanceDocument::parse(&blob).unwrap();
        assert_eq!(appearance.serialize(), expected);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
