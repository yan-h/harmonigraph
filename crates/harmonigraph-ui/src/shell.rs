//! The sequence a windowed shell owes the UI, as three calls.
//!
//! Two shells drive [`root_ui`](crate::root_ui) and neither can share the
//! other's windowing stack — the plugin editor runs on egui-baseview inside a
//! host's window, the dev harness on eframe in one of its own. What they can
//! share is everything AROUND the drawing, which is the same steps in the same
//! order: a freshly built context has to be themed and stripped of the last
//! one's textures, the fold floor and the saved layout have to reach the state
//! before the first frame, the state has to be drawn before the folds it
//! settles can be spent on the window, and the blob goes back to whoever
//! restores it on the way out.
//!
//! Kept by hand in two places that sequence is a doc comment, and a doc
//! comment is what the next step gets added in front of. Here each phase is
//! one call over a struct the shell fills in: what a shell states is its own
//! DATA — its context, its saved blob, how wide its window is — and the steps
//! run over that data in one place, so a shell cannot reorder them and a step
//! it fails to account for is a missing field rather than a missing line.
//!
//! What deliberately stays outside, because it is where the two shells really
//! do differ: the [`ParamBackend`] each provides, the internals of feeding the
//! tracker and the analyzer (one drains lock-free rings an audio thread
//! writes, the other polls a mock progression and a MIDI port), and the
//! plugin's frame pacing, which exists because only it is asked for frames
//! faster than it will draw them.

use crate::params::ParamBackend;
use crate::SharedState;

/// The narrowest window a sideways fold may leave a shell with, in logical
/// points.
///
/// One number rather than one per shell because it has to be two things at
/// once: the floor a window is actually held to, and the floor the pane layout
/// is TOLD about
/// ([`Workspace::min_window_width`](crate::state::Workspace::min_window_width)).
/// Split the two and the layout dials to a width the window will not give — it
/// banks the difference the window refused and hands it back on the way out,
/// which is a fold resizing a pane nobody folded.
pub const MIN_WINDOW_WIDTH: f32 = 400.0;

/// What a shell hands over once it has built an egui `Context`.
///
/// A context, rather than a process or a window, is the unit: the plugin
/// editor builds a new one every time its window opens while the state it
/// draws lives on across all of them, and that gap is what
/// [`SharedState::release_context_resources`] exists to close.
pub struct Opening<'a> {
    /// The context that has just been built. [`open`](Self::open) themes it,
    /// so a shell with styling of its own applies that afterwards.
    pub ctx: &'a egui::Context,
    /// The state that will be drawn in it — older than the context wherever a
    /// window can close and reopen.
    pub state: &'a mut SharedState,
    /// The blob this shell saved when it last closed, from wherever it keeps
    /// one: the host's plugin state, eframe's storage.
    ///
    /// `None` for a shell that has nothing saved, and so is `Some("")` — worth
    /// stating once here rather than in each shell, because a plugin whose
    /// editor has never been open carries an EMPTY `ui_state`, and reading
    /// that as a blob would put a parse failure on the console of every first
    /// open.
    pub persist: Option<&'a str>,
}

impl Opening<'_> {
    /// Theme the context, drop what belonged to the previous one, tell the
    /// layout its floor, then restore the saved blob.
    ///
    /// All four every time a context is built, which is the part a hand-kept
    /// copy loses one of: skip the release and the new window draws through
    /// handles naming textures its renderer never allocated (the spectrogram
    /// vanishes for good), skip the floor and the first fold asks for a window
    /// narrower than the shell will ever grant.
    ///
    /// The blob comes last because it is the only step that can fail, and
    /// [`SharedState::load_persist`] says why on the console that the steps
    /// before it just made drawable — so there is nothing for this to answer
    /// with.
    pub fn open(self) {
        crate::theme::apply_theme(self.ctx);
        self.state.release_context_resources();
        self.state.workspace.min_window_width = MIN_WINDOW_WIDTH;
        if let Some(persist) = self.persist.filter(|blob| !blob.is_empty()) {
            self.state.load_persist(persist);
        }
    }
}

/// One frame: the state a shell has just fed, and the window it is drawn in.
pub struct Frame<'a> {
    /// The window-filling `Ui` the shell's windowing stack hands the frame.
    pub ui: &'a mut egui::Ui,
    /// The state to draw. This frame's MIDI is already in its tracker and this
    /// frame's audio in its analyzer — see [`root_ui`](crate::root_ui), which
    /// is also where the reason that feed cannot move in here lives.
    pub state: &'a mut SharedState,
    /// Where the automatable parameters live for this shell.
    pub params: &'a dyn ParamBackend,
    /// Seconds on the shell's clock, and the same clock that stamped the
    /// events fed above: an envelope is the difference between the two.
    pub now: f64,
    /// How wide the window is at this frame, in the logical points the fold
    /// layout measures in.
    ///
    /// Asked for rather than read off the `Ui`, because in a plugin the two
    /// are not the same question: the editor's window is whatever size its
    /// host last agreed to, which is the size a fold has to be priced against.
    pub window_width: f32,
}

impl Frame<'_> {
    /// Draw the frame, and answer with the width the window has to become for
    /// the sideways folds it settled — `None` on nearly every frame, since
    /// nearly every frame folds nothing.
    ///
    /// What comes back is the width to ASK for, already floored — at the floor
    /// that can be the width the window already has, which is an ask a host
    /// answers with a yes and nothing else.
    ///
    /// The floor is read back out of the state rather than taken from
    /// [`MIN_WINDOW_WIDTH`] here, so the width a window stops at and the width
    /// the layout is dialling to cannot drift into two numbers: there is one
    /// field, [`Opening`] is what puts the constant in it, and both readers
    /// take it from there. A state no shell opened has no floor, and asks for
    /// whatever the fold asked for — which is what a shell that never resizes
    /// wants anyway.
    ///
    /// Taken from the state rather than read out of it, so one fold is one
    /// ask: a host that refuses the size is not asked again on the next frame
    /// (see
    /// [`Workspace::take_window_width_change`](crate::state::Workspace::take_window_width_change)).
    #[must_use = "a fold is spent by resizing the window; dropping this leaves the layout waiting"]
    pub fn draw(self) -> Option<f32> {
        crate::root_ui(self.ui, self.state, self.params, self.now);
        let change = self.state.workspace.take_window_width_change()?;
        Some((self.window_width + change).max(self.state.workspace.min_window_width))
    }
}

/// The blob to hand back to whatever this shell restores from, on the way out.
///
/// Where "the way out" is differs and cannot be shared: the plugin writes it
/// into `params.ui_state` from the editor handle's `Drop`, so a project holds
/// the CLOSE of the last session that had the editor open; the harness answers
/// eframe's `save`. What is shared is that this is the only writer of either —
/// the take recorder serializes the same state mid-session so a render
/// reproduces the look it was dialed in at, and that is a payload, not a save.
#[must_use = "this IS the save; dropping it closes the shell without one"]
pub fn close(state: &SharedState) -> String {
    state.save_persist()
}

#[cfg(test)]
mod tests {
    use super::{close, Frame, Opening, MIN_WINDOW_WIDTH};
    use crate::params::{ParamBackend, ParamKey};
    use crate::tests::probe::fresh;
    use crate::SharedState;

    /// Parameters at their defaults, because nothing here reads one: this
    /// sequence is about the state around a frame, not what the frame draws.
    struct DefaultParams;

    impl ParamBackend for DefaultParams {
        fn get(&self, key: ParamKey) -> f32 {
            key.default_value()
        }
        fn set(&self, _key: ParamKey, _value: f32) {}
    }

    /// Draw one frame into a window `width` points wide, and return what it
    /// asks that window for.
    ///
    /// Whatever any PASS asked for, because egui may run the closure more than
    /// once for one frame — it discards and re-runs a pass when a layout only
    /// settles on the second look — and the fold is taken by whichever pass
    /// settled it.
    ///
    /// A shell has one slot for the answer, so the last pass to ask is the one
    /// that reaches the window. Nothing is lost by that: every pass of a frame
    /// prices against the same window width, so two passes cannot add up the
    /// way two consecutive frames do.
    fn frame(ctx: &egui::Context, state: &mut SharedState, width: f32) -> Option<f32> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(width, 800.0),
            )),
            ..Default::default()
        };
        let mut ask = None;
        // The `FullOutput` is the frame's shapes and platform commands; what
        // this asks about is only what the shell was left needing.
        let _ = ctx.run_ui(raw, |ui| {
            let pass =
                Frame { ui, state, params: &DefaultParams, now: 1.0, window_width: width }.draw();
            ask = pass.or(ask);
        });
        ask
    }

    /// The floor the layout dials to is the floor the shell holds: one number
    /// reaching both, rather than a constant per crate with nothing standing
    /// between them.
    #[test]
    fn opening_tells_the_layout_the_floor_the_shell_holds() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        assert_eq!(state.workspace.min_window_width, 0.0, "a state no shell opened has no floor");
        Opening { ctx: &ctx, state: &mut state, persist: None }.open();
        assert_eq!(state.workspace.min_window_width, MIN_WINDOW_WIDTH);
    }

    #[test]
    fn a_frame_that_folds_nothing_asks_for_nothing() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        Opening { ctx: &ctx, state: &mut state, persist: None }.open();
        assert_eq!(frame(&ctx, &mut state, 1000.0), None);
    }

    /// A fold comes back as a WIDTH, floored, rather than as a change each
    /// shell floors for itself against a constant of its own.
    #[test]
    fn a_fold_comes_back_as_a_width_never_below_the_floor() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        Opening { ctx: &ctx, state: &mut state, persist: None }.open();

        state.workspace.window_width_change = -120.0;
        assert_eq!(frame(&ctx, &mut state, 1000.0), Some(880.0));

        // A fold asking for more than the window has to give stops at the
        // floor rather than following it down.
        state.workspace.window_width_change = -1000.0;
        assert_eq!(frame(&ctx, &mut state, 1000.0), Some(MIN_WINDOW_WIDTH));
    }

    /// The fold a frame settles is spent by THAT frame, which is what the two
    /// steps being in this order buys.
    ///
    /// Its own test because the seeded fold above cannot see the order at all:
    /// a change already in the state on the way in comes back whether it is
    /// taken before the drawing or after it. Here the fold is made by the
    /// frame — a pane collapsed inside a horizontal split, which is the width
    /// `root_ui` banks — so taking first would answer `None` and leave the ask
    /// a frame late, with the layout held at the window it has not got yet
    /// (see `fold::Wait`).
    #[test]
    fn the_fold_a_frame_settles_is_what_that_frame_asks_for() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        Opening { ctx: &ctx, state: &mut state, persist: None }.open();
        // Settled first, so what comes back is the fold and nothing else.
        assert_eq!(frame(&ctx, &mut state, 1000.0), None);

        // The Spectral pane shares a horizontal split with the lattice, so
        // folding it sideways takes its width off the window.
        let path = state
            .workspace
            .dock
            .find_tab(&crate::panes::Tab::Spectral)
            .expect("the analyzer is docked");
        state.workspace.dock[path.surface][path.node].set_collapsed(true);

        let ask = frame(&ctx, &mut state, 1000.0).expect("the fold asks for a narrower window");
        assert!(ask < 1000.0, "a folded pane gives width back, asked for {ask}");
    }

    /// An empty blob is a shell that has never saved, not a broken save. The
    /// plugin's `ui_state` is "" until its editor has closed once, so a
    /// stricter reading costs the console of every first open.
    #[test]
    fn an_unsaved_shell_is_not_a_corrupt_one() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        Opening { ctx: &ctx, state: &mut state, persist: Some("") }.open();
        assert_eq!(state.console.lines().count(), 0, "an empty blob is not a parse failure");

        Opening { ctx: &ctx, state: &mut state, persist: Some("not a blob at all") }.open();
        assert!(
            state.console.lines().any(|line| line.contains("persist ignored")),
            "a blob that really is broken still says so",
        );
    }

    /// The round trip the outer two phases are: what `close` hands the host is
    /// what the next `Opening` restores.
    #[test]
    fn what_close_saves_is_what_the_next_opening_restores() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        Opening { ctx: &ctx, state: &mut state, persist: None }.open();
        state.ui_scale = 1.25;
        let saved = close(&state);

        let mut reopened = fresh();
        Opening { ctx: &ctx, state: &mut reopened, persist: Some(&saved) }.open();
        assert_eq!(reopened.ui_scale, 1.25);
        assert_eq!(
            reopened.workspace.min_window_width,
            MIN_WINDOW_WIDTH,
            "and the floor survives a load",
        );
    }
}
