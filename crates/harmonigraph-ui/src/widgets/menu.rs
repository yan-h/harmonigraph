//! The one look every popup list wears: a dropdown's choices, the tab strip's
//! overflow, a right-click menu, a preset row folded into a menu.
//!
//! One function rather than a look per caller, because egui gives each kind of
//! popup a different default — a ComboBox, a `menu_button` and a bare `Popup`
//! each frame and pad themselves their own way — and the app should have one.

use egui::style::StyleModifier;

use crate::theme;

/// egui's menu look, but with the section tabs' button padding rather than its
/// tighter one, the header's margin rather than its wider one, and no outline.
/// The corners are concentric with the highlight's, so the margin holds its
/// width around them too. Items touch: only one is ever highlighted, so a gap
/// between them would only be blank space.
///
/// Text never wraps. egui keeps a popup's size from one opening to the next,
/// so a menu first opened over short entries would otherwise wrap a longer one
/// that joins it later, rather than growing to fit it.
pub(crate) fn menu_style(ctx: &egui::Context) -> StyleModifier {
    // The header's own gap between a button and the edge around it.
    let scale = theme::ui_scale(ctx);
    let margin = ((theme::tab_bar_height(scale) - theme::row_height(scale)) * 0.5).round() as u8;
    StyleModifier::new(move |style: &mut egui::Style| {
        let padding = style.spacing.button_padding;
        egui::containers::menu::menu_style(style);
        style.spacing.button_padding = padding;
        style.spacing.menu_margin = egui::Margin::same(margin as i8);
        style.spacing.item_spacing.y = 0.0;
        let inner = style.visuals.widgets.hovered.corner_radius.nw;
        style.visuals.menu_corner_radius = egui::CornerRadius::same(inner.saturating_add(margin));
        style.visuals.window_stroke = egui::Stroke::NONE;
        style.wrap_mode = Some(egui::TextWrapMode::Extend);
    })
}
