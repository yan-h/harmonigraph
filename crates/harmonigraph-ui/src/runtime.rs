//! Synchronous visual progression. No dock or viewport is needed to observe
//! configuration, consume analyzed input, or advance bounded histories.

use crate::params::ParamBackend;
use crate::{AudioSpectrum, Console};
use harmonigraph_core::{NoteTracker, PitchClass, Tuning};
use std::time::{Duration, Instant};

/// How long one requested state may go unadopted by the audio thread before
/// the editor says so (#1814 Q2). An edit reaches the plugin's state at once
/// and the audio thread adopts it at its next callback, so during a drag
/// every frame's edit is unadopted for part of a frame: a label shown at once
/// would flicker. With audio stopped the label waits this long, then stays
/// until audio adopts the edit.
pub const PENDING_AFTER: Duration = Duration::from_millis(100);

/// Whether one request has gone unadopted for [`PENDING_AFTER`]. The timer is
/// keyed on the request as its source names it (the latest queued edit's id,
/// or the map state the parameters ask for before following moves it), so a
/// drag, which requests anew every frame, never lights it, and following,
/// which is no request, never restarts it.
#[derive(Debug)]
pub struct PendingLabel<K> {
    since: Option<(K, Instant)>,
}
impl<K> Default for PendingLabel<K> {
    fn default() -> Self {
        Self { since: None }
    }
}
impl<K> PendingLabel<K> {
    /// Move the wait's start back by [`PENDING_AFTER`], as if it had begun
    /// that long ago.
    #[cfg(test)]
    fn backdate(&mut self) {
        if let Some((_, since)) = &mut self.since {
            *since -= PENDING_AFTER;
        }
    }
}
impl<K: PartialEq + Copy> PendingLabel<K> {
    /// Whether to show the label, given the request the audio thread has not
    /// adopted yet, `None` once it has adopted everything, which clears it.
    pub fn shown(&mut self, unadopted: Option<K>, now: Instant) -> bool {
        let Some(requested) = unadopted else {
            self.since = None;
            return false;
        };
        match self.since {
            Some((key, since)) if key == requested => now.duration_since(since) >= PENDING_AFTER,
            _ => {
                self.since = Some((requested, now));
                false
            }
        }
    }
}

/// Synchronous input, analysis and history, independent of any viewport.
pub struct VisualRuntime {
    pub lattice_maps: Option<crate::lattice_maps::MapView>,
    pub map_destination: Option<harmonigraph_core::LatticePos>,
    /// Arrow keys pressed over the lattice, as one map translation.
    pub map_translation:
        Option<(harmonigraph_core::LatticePos, crate::lattice_maps::MapOffsetLane)>,
    pub console: Console,
    pub tracker: NoteTracker,
    /// The resolved tuning, refreshed each frame in [`root_ui`](crate::root_ui)
    /// from whichever owner holds the configuration.
    pub tuning: Tuning,
    /// Whether `tuning` is drawn in the lattice-board's tuning rather than the
    /// saved one ([`ConfigurationView::board`](crate::params::ConfigurationView::board)):
    /// the Tuning pane's fifth then edits the Decoded fifth.
    pub board_drawn: bool,
    /// While true, tuning params continuously re-learn from the held notes
    /// (v1's learn mode). Runtime-only; never persisted.
    pub learn_active: bool,
    /// The commas being tempered out: the links every naming path spells
    /// against and the tuning panes show. Resolved each frame from whichever
    /// owner holds the configuration (the plugin's audio owner, a take's
    /// recorded boundaries, or this runtime's own reducer), and never
    /// persisted by the editor: the plugin saves them in its musical settings,
    /// and a shell without a configuration owner starts from the reducer's
    /// fresh value.
    pub tempered: harmonigraph_core::Tempered,
    pub(crate) config_reducer: harmonigraph_core::configuration::ConfigReducer,
    /// Offline replay supplies recorded resolved boundaries, never frame-driven detection.
    pub replayed_configuration: Option<harmonigraph_core::configuration::ResolvedConfig>,
    pub adaptive_policy: harmonigraph_core::configuration::PolicyConfig,
    pub configuration_status: u32,
    /// Show "Tuning change pending audio adoption": the configuration the
    /// editor requested has gone unadopted for [`PENDING_AFTER`].
    pub configuration_pending_shown: bool,
    configuration_unadopted: PendingLabel<u64>,
    /// Show the map's "pending audio adoption": the map state the parameters
    /// request has gone unadopted for [`PENDING_AFTER`].
    pub maps_pending: bool,
    maps_unadopted: PendingLabel<crate::lattice_maps::MapPlayback>,
    /// Held pitch classes the last learn ran against (change detection).
    pub(crate) last_learned_classes: Option<Vec<PitchClass>>,
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
    /// The plugin's tape, as the Analyzer's Tape mode shows it. Runtime-only.
    pub tape: crate::tape::TapeView,
}
impl Default for VisualRuntime {
    fn default() -> Self {
        Self {
            lattice_maps: None,
            map_destination: None,
            map_translation: None,
            console: Console::default(),
            tracker: NoteTracker::new(),
            tuning: Tuning::default(),
            board_drawn: false,
            learn_active: false,
            tempered: harmonigraph_core::configuration::TuningModes::default().tempered,
            config_reducer: Default::default(),
            replayed_configuration: None,
            adaptive_policy: Default::default(),
            configuration_status: 0,
            configuration_pending_shown: false,
            configuration_unadopted: PendingLabel::default(),
            maps_pending: false,
            maps_unadopted: PendingLabel::default(),
            last_learned_classes: None,
            spectrum: AudioSpectrum::default(),
            ring_fade: harmonigraph_scene::RingFade::default(),
            ring_levels: crate::panes::spectral_fold::RingLevels::default(),
            tape: Default::default(),
        }
    }
}
impl VisualRuntime {
    /// Advance bounded display histories on the caller's clock, whether or not
    /// any surface is drawn, on the look's note envelope. The spectrogram's history is not among them: each rung of its pyramid
    /// is bounded by the ring it keeps, and a stopped stream scrolls out of
    /// the window without being trimmed.
    pub fn advance_time(&mut self, now: f64, appearance: &crate::AppearanceDocument) {
        self.tracker.prune(now, &appearance.view.envelope());
    }

    /// The synchronous reducer's last complete value, for standalone recording.
    pub fn resolved_configuration(&self) -> harmonigraph_core::configuration::ResolvedConfig {
        self.replayed_configuration.unwrap_or_else(|| self.config_reducer.resolved())
    }

    /// Observe the current musical configuration, then advance. Closed
    /// schedulers call `advance_time` directly; observing is independent of
    /// drawing too.
    pub fn advance(
        &mut self,
        appearance: &crate::AppearanceDocument,
        params: &dyn ParamBackend,
        now: f64,
    ) {
        self.observe_configuration(params);
        self.advance_time(now, appearance);
    }

    /// Resolve current musical values without advancing drawing or envelope state.
    /// Synchronous shells also call this after edits, before capturing their frame.
    pub fn observe_configuration(&mut self, params: &dyn ParamBackend) {
        let owned = params.configuration();
        if owned.is_none() && self.replayed_configuration.is_none() {
            self.learn_step();
        }

        self.board_drawn = owned.is_some_and(|owned| owned.board);
        if let Some(owned) = owned {
            self.apply_resolved(owned.resolved);
            self.configuration_status = owned.status;
            self.configuration_pending_shown =
                self.configuration_unadopted.shown(owned.unadopted, Instant::now());
        } else if let Some(recorded) = self.replayed_configuration {
            self.apply_resolved(recorded);
        } else {
            self.apply_resolved(self.config_reducer.resolved());
        }
    }

    /// Both pending labels' waits, as if they began [`PENDING_AFTER`] ago:
    /// for a test that draws a label without waiting for it.
    #[cfg(test)]
    pub(crate) fn backdate_pending_labels(&mut self) {
        self.configuration_unadopted.backdate();
        self.maps_unadopted.backdate();
    }

    /// Whether Learn acts: armed, under an engine where the policy lets it
    /// learn ([`PolicyConfig::learns`](harmonigraph_core::configuration::PolicyConfig::learns)),
    /// where `engine` is the Hub's selected one (`None` for a shell with no
    /// engine selector, which learns, and offers no Decode fifths). The one
    /// rule Learn's two indicators, the lattice's badge and the Tuning pane's
    /// ring, and the pane's "suspended" line all read.
    pub fn learn_acts(&self, engine: Option<harmonigraph_core::lattice_map::TuningEngine>) -> bool {
        self.learn_active && engine.is_none_or(|engine| self.adaptive_policy.learns(engine))
    }

    /// Keep this frame's map view, and decide its pending label.
    pub fn observe_maps(&mut self, maps: Option<crate::lattice_maps::MapView>) {
        self.maps_pending = maps
            .as_ref()
            .is_some_and(|view| self.maps_unadopted.shown(view.unadopted, Instant::now()));
        self.lattice_maps = maps;
    }

    fn apply_resolved(&mut self, config: harmonigraph_core::configuration::ResolvedConfig) {
        self.tuning = config.tuning;
        self.adaptive_policy = config.policy;
        self.tempered = config.modes.tempered;
        self.learn_active = config.modes.learning;
    }

    /// Install a configuration in this runtime's own reducer without rejudging
    /// its links, as a project restore would: the standalone's demo tuning,
    /// and a replay's frames before its take's first configuration record.
    pub fn open(&mut self, config: harmonigraph_core::configuration::ResolvedConfig) {
        self.config_reducer.apply(harmonigraph_core::configuration::ConfigMutation::Restore {
            raw: config.tuning,
            modes: config.modes,
            policy: config.policy,
        });
        self.apply_resolved(self.config_reducer.resolved());
    }

    /// Apply one tuning action atomically. The synchronous shell uses the same
    /// reducer as the audio owner.
    pub fn edit_tuning(
        &mut self,
        params: &dyn ParamBackend,
        edit: harmonigraph_core::configuration::ConfigEdit,
    ) {
        if let Some(accepted) = params.submit_tuning(edit) {
            if !accepted {
                self.console.log("Tuning command refused: configuration storage is full");
            }
            return;
        }
        self.config_reducer.apply(harmonigraph_core::configuration::ConfigMutation::Edit(edit));
        self.apply_resolved(self.config_reducer.resolved());
    }

    /// One tick of learn mode (v1 semantics): while armed, whenever the set of
    /// held pitch classes changes, re-infer the tuning into the runtime's own
    /// reducer. Change-detected so a held chord is learned once. No egui types.
    pub(crate) fn learn_step(&mut self) {
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
            // No audio owner and so no Tune source: the lattice axes are the
            // only target, and every value is learned.
            self.config_reducer.apply(
                harmonigraph_core::configuration::ConfigMutation::LearnResolved {
                    learned,
                    retuning: false,
                },
            );
            self.apply_resolved(self.config_reducer.resolved());
            self.console.log(format!("learn: {} held classes -> {:?}", classes.len(), learned));
        }
        self.last_learned_classes = Some(classes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One state unadopted for the whole wait lights the label, and adoption
    /// clears it; a drag's state, new every frame, never does.
    #[test]
    fn a_pending_label_waits_for_one_requested_state() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut label = PendingLabel::default();
        assert!(!label.shown(Some(1), at(0)));
        assert!(!label.shown(Some(1), at(99)));
        assert!(label.shown(Some(1), at(100)), "unadopted for the whole wait");
        assert!(!label.shown(None, at(150)), "adopted");
        assert!(!label.shown(Some(1), at(160)), "the wait starts again");
        for frame in 0..30 {
            assert!(!label.shown(Some(frame), at(200 + 16 * frame as u64)), "a drag");
        }
    }
}
