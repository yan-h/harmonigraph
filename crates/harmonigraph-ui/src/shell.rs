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

/// The smallest editor size supported by both interactive shells.
pub const MIN_WINDOW_SIZE: egui::Vec2 = egui::vec2(400.0, 300.0);

/// Largest side of the font atlas an interactive shell advertises to egui.
///
/// egui fixes the atlas width at this value, so reporting an 8192-wide device
/// retains 64–128 MiB of CPU font pixels after a full lattice zoom. The
/// renderer still owns the device's real limit; this bounds only the atlas
/// egui chooses for type it rasterizes itself.
pub const FONT_ATLAS_MAX_SIDE: usize = 4096;

/// Bound egui's font atlas without lowering the renderer's texture limit.
pub fn limit_font_atlas(input: &mut egui::RawInput) {
    input.max_texture_side = input.max_texture_side.map(|side| side.min(FONT_ATLAS_MAX_SIDE));
}

/// What a shell hands over once it has built an egui `Context`.
///
/// A context, rather than a process or a window, is the unit: the plugin
/// editor builds a new one every time its window opens while the state it
/// draws lives on across all of them, and that gap is what
/// [`crate::PictureState::release_context_resources`] exists to close.
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
        self.state.picture.release_context_resources();
        self.state.workspace.min_window_size = MIN_WINDOW_SIZE;
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
    /// Actual window size agreed to by the host, in logical points.
    pub window_size: egui::Vec2,
}

impl Frame<'_> {
    /// Draw first, then return a one-shot, floored window size request.
    #[must_use = "a fold is spent by resizing the window"]
    pub fn draw(self) -> Option<egui::Vec2> {
        crate::root_ui(self.ui, self.state, self.params, self.now);
        let change = self.state.workspace.take_window_size_change()?;
        Some((self.window_size + change).max(self.state.workspace.min_window_size))
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
    use super::*;
    use crate::params::{ParamBackend, ParamKey};
    use crate::tests::probe::fresh;

    struct Defaults;
    impl ParamBackend for Defaults {
        fn get(&self, key: ParamKey) -> f32 {
            key.default_value()
        }
        fn set(&self, _: ParamKey, _: f32) {}
    }

    fn frame(ctx: &egui::Context, state: &mut SharedState, size: egui::Vec2) -> Option<egui::Vec2> {
        let mut ask = None;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let _ = ctx.run_ui(raw, |ui| {
            ask =
                Frame { ui, state, params: &Defaults, now: 1.0, window_size: size }.draw().or(ask);
        });
        ask
    }

    #[test]
    fn requests_are_two_dimensional_floored_and_consumed_once() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        Opening { ctx: &ctx, state: &mut state, persist: None }.open();
        assert_eq!(state.workspace.min_window_size, MIN_WINDOW_SIZE);
        let size = egui::vec2(1000.0, 800.0);
        assert_eq!(frame(&ctx, &mut state, size), None);
        state.workspace.window_size_change = egui::vec2(-120.0, -220.0);
        assert_eq!(frame(&ctx, &mut state, size), Some(egui::vec2(880.0, 580.0)));
        assert_eq!(frame(&ctx, &mut state, size), None);
        state.workspace.window_size_change = -size;
        assert_eq!(frame(&ctx, &mut state, size), Some(MIN_WINDOW_SIZE));
    }

    #[test]
    fn an_unsaved_shell_is_not_a_corrupt_one() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        Opening { ctx: &ctx, state: &mut state, persist: Some("") }.open();
        assert_eq!(state.picture.runtime.console.lines().count(), 0);
        Opening { ctx: &ctx, state: &mut state, persist: Some("not a blob") }.open();
        assert!(state.picture.runtime.console.lines().any(|line| line.contains("persist ignored")));
        assert!(state.workspace.layout.visible(crate::panes::Tab::Console));
    }

    #[test]
    fn a_skin_that_no_longer_exists_opens_as_default_and_says_so() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        state.workspace.interaction.skin = "original".to_owned();
        state.workspace.interaction.ui_scale = 1.25;
        let saved = close(&state).replace("\"original\"", "\"retired\"");
        let mut reopened = fresh();
        Opening { ctx: &ctx, state: &mut reopened, persist: Some(&saved) }.open();
        assert_eq!(reopened.workspace.interaction.skin, harmonigraph_scene::skin::DEFAULT_SKIN);
        assert_eq!(
            reopened.workspace.interaction.ui_scale, 1.25,
            "the rest of the blob still loads"
        );
        assert!(reopened.picture.runtime.console.lines().any(|line| line.contains("retired")));
    }

    #[test]
    fn what_close_saves_is_what_the_next_opening_restores() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        state.workspace.interaction.ui_scale = 1.25;
        state.workspace.interaction.skin = "original".to_owned();
        state.workspace.layout.position = crate::workspace::Position::Below;
        state.workspace.layout.folded[0] = true;
        let saved = close(&state);
        let mut reopened = fresh();
        Opening { ctx: &ctx, state: &mut reopened, persist: Some(&saved) }.open();
        assert_eq!(reopened.workspace.interaction.ui_scale, 1.25);
        assert_eq!(reopened.workspace.interaction.skin, "original");
        assert_eq!(reopened.workspace.layout.position, crate::workspace::Position::Below);
        assert!(reopened.workspace.layout.folded[0]);
        assert_eq!(reopened.workspace.min_window_size, MIN_WINDOW_SIZE);
    }
}
