//! The sequence a windowed shell owes the UI, as three calls.
//!
//! Two shells drive [`root_ui`] and neither can share the
//! other's windowing stack — the plugin editor runs on egui-baseview inside a
//! host's window, the dev harness on eframe in one of its own. What they can
//! share is everything AROUND the drawing, which is the same steps in the same
//! order: a freshly built context has to be themed and the last window's
//! leftovers have to leave the state before the first frame, and the state
//! has to be drawn before the folds it settles can be spent on the window.
//!
//! Kept by hand in two places that sequence is a doc comment, and a doc
//! comment is what the next step gets added in front of. Here each phase is
//! one call over a struct the shell fills in: what a shell states is its own
//! DATA — its context, how wide its window is — and the steps
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
/// draws lives on across all of them, so what a context needs is owed again
/// with every window.
pub struct Opening<'a> {
    /// The context that has just been built. [`open`](Self::open) themes it,
    /// so a shell with styling of its own applies that afterwards.
    pub ctx: &'a egui::Context,
    /// The state that will be drawn in it — older than the context wherever a
    /// window can close and reopen.
    pub state: &'a mut SharedState,
}

impl Opening<'_> {
    /// Theme the context, then start the window session.
    ///
    /// Both every time a context is built, which is the part a hand-kept copy
    /// loses one of.
    ///
    /// Nothing saved is read here. The state outlives every window, so it
    /// already holds the settings the last window left; a saved document
    /// reaches it only through [`SharedState::install_document`], at a host
    /// restore or the standalone harness's start.
    pub fn open(self) {
        crate::theme::apply_theme(self.ctx);
        self.state.start_window_session();
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
    /// Identity stamped into this interactive shell at build time.
    pub build_tag: &'a str,
    /// Actual window size agreed to by the host, in logical points.
    pub window_size: egui::Vec2,
}

impl Frame<'_> {
    /// Draw first, then return a one-shot, floored window size request.
    #[must_use = "a fold is spent by resizing the window"]
    pub fn draw(self) -> Option<egui::Vec2> {
        crate::root_ui(self.ui, self.state, self.params, self.now, self.build_tag);
        let change = self.state.workspace.take_window_size_change()?;
        Some((self.window_size + change).max(MIN_WINDOW_SIZE))
    }
}

/// The blob the standalone harness hands eframe's `save` on the way out.
///
/// The plugin has no way out to write at: its state outlives every window, and
/// a host save answers from [`SharedState::save_answer`], window open or shut.
/// The take recorder serializes the same state mid-session so a render
/// reproduces the look it was dialed in at, and that is a payload, not a save.
#[must_use = "this IS the save; dropping it closes the shell without one"]
pub fn close(state: &SharedState) -> String {
    state.save_persist()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::probe::{fresh, Defaults};

    fn frame(ctx: &egui::Context, state: &mut SharedState, size: egui::Vec2) -> Option<egui::Vec2> {
        frame_with(ctx, state, size, Vec::new())
    }

    fn frame_with(
        ctx: &egui::Context,
        state: &mut SharedState,
        size: egui::Vec2,
        events: Vec<egui::Event>,
    ) -> Option<egui::Vec2> {
        let mut ask = None;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        };
        let _ = crate::tests::probe::run(ctx, raw, |ui| {
            ask = Frame {
                ui,
                state,
                params: &Defaults,
                now: 1.0,
                build_tag: "test @0123456",
                window_size: size,
            }
            .draw()
            .or(ask);
        });
        ask
    }

    const SIZE: egui::Vec2 = egui::vec2(1000.0, 800.0);

    #[test]
    fn requests_are_two_dimensional_floored_and_consumed_once() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        Opening { ctx: &ctx, state: &mut state }.open();
        assert_eq!(frame(&ctx, &mut state, SIZE), None);
        state.workspace.window_size_change = egui::vec2(-120.0, -220.0);
        assert_eq!(frame(&ctx, &mut state, SIZE), Some(egui::vec2(880.0, 580.0)));
        assert_eq!(frame(&ctx, &mut state, SIZE), None);
        state.workspace.window_size_change = -SIZE;
        assert_eq!(frame(&ctx, &mut state, SIZE), Some(MIN_WINDOW_SIZE));
    }

    /// The standalone harness's round trip: what it saves on the way out, its
    /// next start installs before the window opens.
    #[test]
    fn what_close_saves_is_what_the_next_start_installs() {
        let ctx = egui::Context::default();
        let mut state = fresh();
        state.workspace.interaction.prefs.ui_scale = 1.25;
        state.workspace.interaction.prefs.skin_dials.accent_hue = 120.0;
        state.workspace.layout.position = crate::workspace::Position::Below;
        state.workspace.layout.folded[0] = true;
        let saved = close(&state);
        let mut reopened = fresh();
        assert!(reopened.install_document(&saved));
        Opening { ctx: &ctx, state: &mut reopened }.open();
        assert_eq!(reopened.workspace.interaction.prefs.ui_scale, 1.25);
        assert_eq!(reopened.workspace.interaction.prefs.skin_dials.accent_hue, 120.0);
        assert_eq!(reopened.workspace.layout.position, crate::workspace::Position::Below);
        assert!(reopened.workspace.layout.folded[0]);
    }

    /// An edit to what a save stores is told once, at the end of the gesture
    /// that made it, even one that came back to where it began (#1813). A
    /// restore, the layout fitted to the window and the host's camera are no
    /// edit: a project saved before its editor ever opened,
    /// restored into an open editor, fits its layout to the window, and so
    /// does a window that grows after.
    #[test]
    fn an_edit_to_what_a_save_stores_is_told_once_at_its_gestures_end() {
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        let told = std::sync::Arc::new(AtomicUsize::new(0));
        let mut state = fresh();
        let counter = told.clone();
        state.on_edit(move || {
            counter.fetch_add(1, Relaxed);
        });
        let ctx = egui::Context::default();
        Opening { ctx: &ctx, state: &mut state }.open();
        let _ = frame(&ctx, &mut state, SIZE);
        assert!(state.install_document(""));
        let grown = SIZE + egui::vec2(300.0, 100.0);
        for size in [SIZE, SIZE, grown, grown] {
            let _ = frame(&ctx, &mut state, size);
        }
        state.picture.appearance.camera.yaw += 0.5;
        let _ = frame(&ctx, &mut state, grown);
        assert_eq!(told.load(Relaxed), 0, "nothing a save stores was edited");

        // A camera preset saved is an edit.
        let preset = crate::CameraPreset { name: "reading".into(), yaw: 0.7, pitch: 0.2 };
        state.workspace.interaction.camera_presets.push(preset);
        let _ = frame(&ctx, &mut state, grown);
        assert_eq!(told.load(Relaxed), 1, "a camera preset saved");

        // The divider dragged away and back after the pane grew, within one
        // pinch: nothing until it ends, then once.
        let dial = state.picture.appearance.spectrum.roll_fraction;
        let away = if dial < 0.5 { dial + 0.25 } else { dial - 0.25 };
        for fraction in [away, dial] {
            state.picture.appearance.spectrum.roll_fraction = fraction;
            let _ = frame_with(&ctx, &mut state, grown, vec![egui::Event::Zoom(1.1)]);
        }
        assert_eq!(told.load(Relaxed), 1, "not while a pinch is open");
        let _ = frame(&ctx, &mut state, grown);
        let _ = frame(&ctx, &mut state, grown);
        assert_eq!(told.load(Relaxed), 2, "once, as the pinch ends");

        let folded = &mut state.workspace.layout.folded[0];
        *folded = !*folded;
        let _ = frame(&ctx, &mut state, grown);
        assert_eq!(told.load(Relaxed), 3, "a fold");
    }
}
