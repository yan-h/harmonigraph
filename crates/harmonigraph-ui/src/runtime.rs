//! Synchronous visual progression. No dock or viewport is needed to observe
//! configuration, consume analyzed input, or advance bounded histories.

use crate::params::{self, ParamBackend};
use crate::{AudioSpectrum, Console, WholeSong};
use harmonigraph_core::{Comma, NoteTracker, PitchClass, Tuning};
use harmonigraph_scene::FrameParams;

/// Synchronous input, analysis and history, independent of any viewport.
pub struct VisualRuntime {
    pub console: Console,
    pub tracker: NoteTracker,
    /// Snapshot of the tuning parameters, refreshed each frame in
    /// [`root_ui`](crate::root_ui) so core/scene code never touches the param system.
    pub tuning: Tuning,
    /// Per-frame mirrors of the appearance parameters, refreshed alongside
    /// `tuning` (the param system owns the real values; these are never
    /// persisted).
    pub frame_params: FrameParams,
    /// While true, tuning params continuously re-learn from the held notes
    /// (v1's learn mode). Runtime-only; never persisted.
    pub learn_active: bool,
    pub(crate) config_reducer: harmonigraph_core::configuration::ConfigReducer,
    /// Offline replay supplies recorded resolved boundaries, never frame-driven detection.
    pub replayed_configuration: Option<harmonigraph_core::configuration::ResolvedConfig>,
    pub neighbourhood: crate::adaptive::Neighbourhood,
    pub adaptive_policy: harmonigraph_core::configuration::PolicyConfig,
    pub configuration_status: u32,
    pub configuration_pending: bool,
    /// Held pitch classes the last learn ran against (change detection).
    pub(crate) last_learned_classes: Option<Vec<PitchClass>>,
    /// Per comma (indexed by [`Comma::index`]): the tuning axes (microcents)
    /// that comma's auto-detect last saw, so it judges each tuning exactly
    /// once.
    ///
    /// This is what lets a comma switch be switched OFF: an unchanged tuning
    /// gets no second verdict, so the mode stays where it was put until the
    /// tuning itself moves. It also keeps the detect off the plugin's
    /// in-flight parameter writes, which report the value being written away
    /// from for a frame or more (see `begin_frame`).
    ///
    /// One entry per comma, and each holds only the axes ITS identity reads
    /// (see `judged_axes`) — a seventh that moved must not re-open the
    /// syntonic question, or dragging the seventh would re-engage a meantone
    /// that was just switched off.
    ///
    /// Runtime-only. A saved project carries the modes themselves, and
    /// reopening one is exactly when the detects should look afresh.
    pub(crate) temper_judged: [Option<(i32, i32, i32)>; Comma::COUNT],
    /// Audio-derived spectrum for the Spectral pane. Runtime-only.
    pub spectrum: AudioSpectrum,
    /// How far open the audio ring's Gate stands at each bucket of the
    /// analyzer's grid, so a ring arrives and leaves on the note Fade rather
    /// than at the instant the spectrum crosses the bar
    /// ([`RingFade`](harmonigraph_scene::RingFade)). Runtime-only.
    ///
    /// Shared by live and offline drawing. Stepped against the clock by
    /// `spectral_fold::apply`, so dock and preview step once between them.
    pub ring_fade: harmonigraph_scene::RingFade,
    /// What the ring's wedges currently READ, carried across frames on the
    /// ring's own attack and release — the level inside the annulus, where
    /// [`ring_fade`](Self::ring_fade) above is whether the annulus is there at
    /// all. Both belong to the shared reading, not to viewport geometry.
    pub ring_levels: crate::panes::spectral_fold::RingLevels,
    /// Offline playhead render: the whole take's spectrogram laid out
    /// statically with a playhead at `now`, instead of the live scrolling
    /// window. `Some` only in the offline renderer. Runtime-only, never
    /// persisted (mirrors `learn_active`).
    pub whole_song: Option<WholeSong>,
}
impl Default for VisualRuntime {
    fn default() -> Self {
        Self {
            console: Console::default(),
            tracker: NoteTracker::new(),
            tuning: Tuning::default(),
            frame_params: FrameParams::default(),
            learn_active: false,
            config_reducer: Default::default(),
            replayed_configuration: None,
            neighbourhood: Default::default(),
            adaptive_policy: Default::default(),
            configuration_status: 0,
            configuration_pending: false,
            last_learned_classes: None,
            temper_judged: [None; Comma::COUNT],
            spectrum: AudioSpectrum::default(),
            ring_fade: harmonigraph_scene::RingFade::default(),
            ring_levels: crate::panes::spectral_fold::RingLevels::default(),
            whole_song: None,
        }
    }
}
impl VisualRuntime {
    /// Advance bounded display histories on the caller's clock, whether or not
    /// any surface is drawn. The latest observed parameters own the envelope;
    /// drawing observes them before advancing, closed draining retains them.
    /// Raw spectral history also expires while input is stopped; this trims
    /// retained data, not the capacity of every surface cache.
    pub fn advance_time(&mut self, now: f64, appearance: &crate::AppearanceDocument) {
        self.tracker.prune(now, &appearance.view.envelope(&self.frame_params));
        self.spectrum.history.trim_older_than(now - AudioSpectrum::HISTORY_MAX_SECONDS);
    }

    /// The synchronous reducer's last complete value, for standalone recording.
    pub fn resolved_configuration(&self) -> harmonigraph_core::configuration::ResolvedConfig {
        self.replayed_configuration.unwrap_or_else(|| self.config_reducer.resolved())
    }

    /// Observe current parameters before pruning, so a longer fade takes effect
    /// in the same frame. Closed schedulers retain the last observed mirrors and
    /// call `advance_time` directly; observing is independent of drawing too.
    pub fn advance(
        &mut self,
        appearance: &mut crate::AppearanceDocument,
        params: &dyn ParamBackend,
        now: f64,
    ) {
        self.observe_configuration(appearance, params);
        self.frame_params = FrameParams {
            fade_time: params.get(params::ParamKey::Fade),
            darkest_pitch: params.get(params::ParamKey::DarkestPitch),
            brightest_pitch: params.get(params::ParamKey::BrightestPitch),
        };
        self.advance_time(now, appearance);
    }

    /// Resolve current musical values without advancing drawing or envelope state.
    /// Synchronous shells also call this after edits, before capturing their frame.
    pub fn observe_configuration(
        &mut self,
        appearance: &mut crate::AppearanceDocument,
        params: &dyn ParamBackend,
    ) {
        let owned = params.configuration();
        if owned.is_none() && self.replayed_configuration.is_none() {
            self.learn_step(appearance, params);
        }

        if let Some(owned) = owned {
            self.apply_resolved(appearance, owned.resolved);
            self.configuration_status = owned.status;
            self.configuration_pending = owned.pending;
        } else if let Some(recorded) = self.replayed_configuration {
            self.apply_resolved(appearance, recorded);
        } else {
            let modes = self.tuning_modes(appearance);
            self.config_reducer.sync_display(
                params::tuning_from_params(params),
                modes,
                self.temper_judged,
            );
            self.temper_judged = self.config_reducer.judged();
            self.apply_resolved(appearance, self.config_reducer.resolved());
        }
    }

    fn tuning_modes(
        &self,
        appearance: &crate::AppearanceDocument,
    ) -> harmonigraph_core::configuration::TuningModes {
        harmonigraph_core::configuration::TuningModes {
            tempered: harmonigraph_core::Tempered {
                syntonic: appearance.view.meantone,
                septimal_kleisma: appearance.view.marvel,
            },
            auto: [appearance.view.meantone_auto, appearance.view.marvel_auto],
            learning: self.learn_active,
        }
    }

    fn apply_resolved(
        &mut self,
        appearance: &mut crate::AppearanceDocument,
        config: harmonigraph_core::configuration::ResolvedConfig,
    ) {
        self.tuning = config.tuning;
        self.adaptive_policy = config.policy;
        appearance.view.meantone = config.modes.tempered.syntonic;
        appearance.view.marvel = config.modes.tempered.septimal_kleisma;
        appearance.view.meantone_auto = config.modes.auto[0];
        appearance.view.marvel_auto = config.modes.auto[1];
        self.learn_active = config.modes.learning;
    }

    /// One semantic action, including every axis and explicit unlock in a preset.
    /// CLAP submits it once; standalone/legacy writes synchronously through the same
    /// pure reducer at the next frame boundary.
    pub(crate) fn edit_tuning(
        &mut self,
        appearance: &mut crate::AppearanceDocument,
        params: &dyn ParamBackend,
        edit: harmonigraph_core::configuration::ConfigEdit,
    ) {
        if let Some(accepted) = params.submit_tuning(edit) {
            self.configuration_pending = accepted;
            if !accepted {
                self.console.log("Tuning command refused: configuration storage is full");
            }
            return;
        }
        if let Some(policy) = edit.policy {
            self.config_reducer.apply(harmonigraph_core::configuration::ConfigMutation::Edit(
                harmonigraph_core::configuration::ConfigEdit {
                    policy: Some(policy),
                    ..Default::default()
                },
            ));
            self.adaptive_policy = policy;
        }
        for (key, value) in params::ParamKey::TUNING.into_iter().zip(edit.axes) {
            if let Some(value) = value {
                params.set(key, value as f32 / 1_000_000.0);
            }
        }
        for comma in Comma::ALL {
            let i = comma.index();
            if let Some(on) = edit.tempered[i] {
                *appearance.view.temper_mut(comma) = on;
            }
            if let Some(on) = edit.auto[i] {
                *appearance.view.temper_auto_mut(comma) = on;
                if on {
                    self.temper_judged[i] = None;
                }
            }
        }
        if let Some(on) = edit.learning {
            self.learn_active = on;
        }
    }

    /// One tick of learn mode (v1 semantics): while armed, whenever the set of
    /// held pitch classes changes, re-infer the tuning and write it through the
    /// param backend. Change-detected so the host only sees parameter sets when
    /// something actually changed. No egui types — testable with a stub
    /// backend.
    pub(crate) fn learn_step(
        &mut self,
        appearance: &mut crate::AppearanceDocument,
        params: &dyn ParamBackend,
    ) {
        if !self.learn_active {
            self.last_learned_classes = None;
            return;
        }
        let mut classes: Vec<PitchClass> = self
            .tracker
            .voices()
            .filter(|v| v.state == harmonigraph_core::VoiceState::Held)
            .map(|v| v.pitch_class)
            .collect();
        classes.sort_unstable();
        classes.dedup();
        if self.last_learned_classes.as_ref() == Some(&classes) {
            return;
        }
        if !classes.is_empty() {
            let learned = harmonigraph_core::learn_tuning(&classes);
            for (value, key) in [
                (learned.c_offset, params::ParamKey::COffset),
                (learned.three, params::ParamKey::Three),
                (learned.five, params::ParamKey::Five),
                (learned.seven, params::ParamKey::Seven),
            ] {
                if let Some(value) = value {
                    params.set(key, value);
                }
            }
            let modes = harmonigraph_core::configuration::learned_modes(
                learned,
                self.tuning_modes(appearance),
            );
            appearance.view.meantone = modes.tempered.syntonic;
            appearance.view.marvel = modes.tempered.septimal_kleisma;
            self.console.log(format!("learn: {} held classes -> {:?}", classes.len(), learned));
        }
        self.last_learned_classes = Some(classes);
    }
}
