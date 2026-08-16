//! What every test module in the crate builds a state with and reads a
//! picture back through.
//!
//! Three things, each of which every test module would otherwise restate: the
//! state a fixture starts from, a context carrying the real theme, and one
//! frame of something painted into a rect. The first two are one line each and
//! the third is ten, but all three are the same kind of liability — a fixture
//! that quietly differs from its neighbour by a font or a screen size measures
//! a different picture and says so nowhere.
//!
//! Beside this, [`super::harness::DockHarness`] drives the whole dock through
//! `root_ui`; that is the fixture for anything about pointer routing, folds or
//! tab bodies. What is here is for a pane, a widget or a label drawn on its
//! own.

use harmonigraph_render::wgpu::TextureFormat;

use crate::SharedState;

/// A fresh [`SharedState`] on the surface format every fixture in the tree
/// uses.
///
/// The format is not a choice a test makes: nothing under test reads it (it is
/// handed straight to `harmonigraph-render` for pipeline creation, and no
/// pipeline is created without a GPU), and Bgra8Unorm is what the plugin's own
/// editor assumes its surface to be. Naming it once means a test says "a fresh
/// state" rather than restating a graphics constant it has no opinion about.
pub(crate) fn fresh() -> SharedState {
    SharedState::new(TextureFormat::Bgra8Unorm)
}

/// A context carrying the app's own theme — the real Iosevka faces and sizes,
/// not egui's 12.5pt fallback.
///
/// Load-bearing for anything that measures text, and invisible when it is
/// missing: a bare context lays every string out in a font the app never
/// ships, so a label that fits on screen here can overflow there and no
/// assertion changes shape. Both label suites and the spectral text tests say
/// "the real Iosevka metrics" outright for this reason.
pub(crate) fn themed() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::apply_theme(&ctx);
    ctx
}

/// The same at a chosen device scale, which is a second thing entirely: `ppp`
/// decides what a glyph's quad ROUNDS to, so a clearance that holds at 1 can
/// be half a pixel at 2. A fixture that leaves it at 1 cannot see that at all.
pub(crate) fn themed_at(ppp: f32) -> egui::Context {
    let ctx = themed();
    ctx.set_pixels_per_point(ppp);
    ctx
}

/// The same at a chosen CHROME scale, which is the third of the three and not
/// to be confused with the second: [`themed_at`] changes how many device
/// pixels a point is worth and nothing about the layout, while this resizes
/// every font, margin and bar in the app — the setting the System page's
/// slider drives.
pub(crate) fn themed_scaled(scale: f32) -> egui::Context {
    let ctx = themed();
    crate::theme::set_ui_scale(&ctx, scale);
    ctx
}

/// One frame on `ctx`, with `add` painting into a child `Ui` whose max rect is
/// `rect`, inside a screen `screen` points across.
///
/// The rect and the screen are separate on purpose, and it is the whole reason
/// this is not `run_ui` with one rect: a pane is handed a box inside a bigger
/// window, and the questions asked of it here are mostly "does anything land
/// outside the box". A screen the size of the box answers those by clipping
/// the evidence away.
///
/// `ctx` is a parameter rather than made here because a few fixtures need one
/// context across several frames — a texture handle is owned by the context
/// that uploaded it, and `set_pixels_per_point` lands on the frame after the
/// one that asks. Everything else wants [`painted_into`], which gives each
/// call a context of its own: egui's temp store has no expiry, so a shared
/// context is one fixture reading the frame before it.
///
/// `add` is `FnMut` because egui runs a frame's closure more than once when a
/// layout only settles on the second look — the same reason `shell::Frame`
/// takes whatever the LAST pass asked for.
pub(crate) fn frame_into(
    ctx: &egui::Context,
    screen: egui::Vec2,
    rect: egui::Rect,
    add: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    events_into(ctx, screen, rect, vec![], add)
}

/// The same with pointer events delivered to the frame, which is what a
/// gesture fixture drives a pane with: a move, a press, a move, a release,
/// each its own frame, because egui resolves the widget under the pointer
/// from the PREVIOUS pass.
pub(crate) fn events_into(
    ctx: &egui::Context,
    screen: egui::Vec2,
    rect: egui::Rect,
    events: Vec<egui::Event>,
    mut add: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), screen);
    let raw = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
    ctx.run_ui(raw, |ui| {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        add(&mut child);
    })
}

/// [`frame_into`] on a themed context of its own — what a test that paints
/// once and reads the shapes back wants.
pub(crate) fn painted_into(
    screen: egui::Vec2,
    rect: egui::Rect,
    add: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    frame_into(&themed(), screen, rect, add)
}

/// The same where what is drawn FILLS the window: a label on its own, a
/// settings column that takes the whole of it, a picture pane drawn the way
/// the offline renderer draws it — no dock and no box inside a box.
///
/// Its own name rather than [`frame_into`] with the window's own rect, because
/// "there is no pane rect here" is a different claim from "the pane rect is
/// the window", and only the first is safe to change the window size under.
pub(crate) fn frame_full(
    ctx: &egui::Context,
    screen: egui::Vec2,
    add: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    frame_into(ctx, screen, egui::Rect::from_min_size(egui::pos2(0.0, 0.0), screen), add)
}

/// [`frame_full`] on a themed context of its own.
pub(crate) fn painted_full(
    screen: egui::Vec2,
    add: impl FnMut(&mut egui::Ui),
) -> egui::FullOutput {
    frame_full(&themed(), screen, add)
}

/// A primary-button press or release at `pos`.
pub(crate) fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}
