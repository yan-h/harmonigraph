//! Three fixed sections, with remembered sizes independent of folded rails.
//! The shell owns the outer window; this module requests a size once and fits
//! the picture to whatever the host actually grants without changing its memory.

use egui::{pos2, vec2, Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::panes::display::DisplayPage;
use crate::panes::{Tab, Viewer};
use crate::theme;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Position {
    #[default]
    Right,
    Below,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    Lattice = 0,
    Analyzer = 1,
    Settings = 2,
}

impl Section {
    const ALL: [Self; 3] = [Self::Lattice, Self::Analyzer, Self::Settings];

    pub(crate) fn of(tab: Tab) -> Self {
        match tab {
            Tab::Lattice => Self::Lattice,
            Tab::Spectral | Tab::Spiral => Self::Analyzer,
            _ => Self::Settings,
        }
    }
}

/// Picture sizes along the arrangement's axis, its shared cross-axis size,
/// and the settings column width. These include the section's own header.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Sizes {
    pub lattice: f32,
    pub analyzer: f32,
    pub cross: f32,
    pub settings: f32,
}

impl Default for Sizes {
    fn default() -> Self {
        Self { lattice: 510.0, analyzer: 200.0, cross: 700.0, settings: 280.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Layout {
    pub position: Position,
    pub sized: bool,
    pub folded: [bool; 3],
    pub right: Sizes,
    pub below: Sizes,
    /// Width removed by each internal analyzer fold in the Right arrangement.
    pub region_widths: [f32; 2],
    pub analyzer_tab: Tab,
    pub settings_tab: Tab,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            position: Position::Right,
            sized: false,
            folded: [false; 3],
            right: Sizes::default(),
            below: Sizes { lattice: 450.0, analyzer: 250.0, cross: 710.0, settings: 280.0 },
            region_widths: [0.0; 2],
            analyzer_tab: Tab::Spectral,
            settings_tab: Tab::Tuning,
        }
    }
}

impl Layout {
    fn sizes(&self) -> Sizes {
        match self.position {
            Position::Right => self.right,
            Position::Below => self.below,
        }
    }

    fn sizes_mut(&mut self) -> &mut Sizes {
        match self.position {
            Position::Right => &mut self.right,
            Position::Below => &mut self.below,
        }
    }

    pub(crate) fn sanitize(&mut self) {
        for sizes in [&mut self.right, &mut self.below] {
            for size in
                [&mut sizes.lattice, &mut sizes.analyzer, &mut sizes.cross, &mut sizes.settings]
            {
                *size = if size.is_finite() { size.clamp(1.0, 8192.0) } else { 300.0 };
            }
        }
        for width in &mut self.region_widths {
            *width = if width.is_finite() { width.clamp(0.0, 8192.0) } else { 0.0 };
        }
        if !matches!(self.analyzer_tab, Tab::Spectral | Tab::Spiral) {
            self.analyzer_tab = Tab::Spectral;
        }
        if self.settings_tab.is_picture() {
            self.settings_tab = Tab::Tuning;
        }
    }

    fn resize_region(&mut self, region: usize, width: Option<f32>, rail: f32) {
        if let Some(width) = width {
            let removed = if self.position == Position::Right {
                width.max(0.0).min((self.right.analyzer - rail).max(0.0))
            } else {
                0.0
            };
            self.region_widths[region] = removed;
            self.right.analyzer -= removed;
        } else {
            // Repay the Right arrangement even if the picture or arrangement
            // now uses a different axis from the fold that removed this width.
            self.right.analyzer += std::mem::take(&mut self.region_widths[region]);
        }
    }

    pub(crate) fn select(&mut self, tab: Tab) {
        match Section::of(tab) {
            Section::Lattice => {}
            Section::Analyzer => self.analyzer_tab = tab,
            Section::Settings => self.settings_tab = tab,
        }
    }

    pub(crate) fn tab(&self, section: Section) -> Tab {
        match section {
            Section::Lattice => Tab::Lattice,
            Section::Analyzer => self.analyzer_tab,
            Section::Settings => self.settings_tab,
        }
    }

    pub(crate) fn visible(&self, tab: Tab) -> bool {
        let section = Section::of(tab);
        !self.folded[section as usize] && self.tab(section) == tab
    }

    #[cfg(test)]
    pub(crate) fn solo(tab: Tab) -> Self {
        let mut layout = Self { folded: [true; 3], ..Self::default() };
        layout.folded[Section::of(tab) as usize] = false;
        layout.select(tab);
        layout
    }

    fn compact(&self) -> bool {
        self.position == Position::Below && self.folded[0] && self.folded[1]
    }

    fn natural_size(&self, rail: f32, gap: f32) -> Vec2 {
        let sizes = self.sizes();
        let extent = |index, size| if self.folded[index] { rail } else { size };
        let a = extent(0, sizes.lattice);
        let b = extent(1, sizes.analyzer);
        let settings = extent(2, sizes.settings);
        if self.compact() {
            return vec2(2.0 * rail + settings + 2.0 * gap, 2.0 * rail + gap);
        }
        match self.position {
            Position::Right => vec2(a + b + settings + 2.0 * gap, sizes.cross),
            Position::Below => vec2(sizes.cross + settings + gap, a + b + gap),
        }
    }

    /// Fit only visible sections. Hidden dimensions are never charged for a
    /// window resize, and a host refusal only fits a temporary drawing copy.
    fn fit(&mut self, area: Vec2, rail: f32, gap: f32) {
        let folded = self.folded;
        let compact = self.compact();
        let position = self.position;
        let sizes = self.sizes_mut();
        if compact {
            if !folded[2] {
                sizes.settings = (area.x - 2.0 * (rail + gap)).max(0.0);
            }
            return;
        }
        match position {
            Position::Right => {
                let widths = fit_axis(
                    [sizes.lattice, sizes.analyzer, sizes.settings],
                    folded,
                    area.x - 2.0 * gap,
                    rail,
                );
                [sizes.lattice, sizes.analyzer, sizes.settings] = widths;
                sizes.cross = area.y;
            }
            Position::Below => {
                let widths =
                    fit_axis([sizes.cross, sizes.settings], [false, folded[2]], area.x - gap, rail);
                [sizes.cross, sizes.settings] = widths;
                let heights = fit_axis(
                    [sizes.lattice, sizes.analyzer],
                    [folded[0], folded[1]],
                    area.y - gap,
                    rail,
                );
                [sizes.lattice, sizes.analyzer] = heights;
            }
        }
    }

    fn rects(&self, area: Rect, rail: f32, gap: f32) -> [Rect; 3] {
        let sizes = self.sizes();
        let extent = |index, size| if self.folded[index] { rail } else { size };
        let a = extent(0, sizes.lattice);
        let b = extent(1, sizes.analyzer);
        let settings = extent(2, sizes.settings);
        let at = area.min;
        if self.compact() {
            return [
                Rect::from_min_size(at, vec2(rail, area.height())),
                Rect::from_min_size(at + vec2(rail + gap, 0.0), vec2(rail, area.height())),
                Rect::from_min_size(
                    at + vec2(2.0 * (rail + gap), 0.0),
                    vec2(settings, area.height()),
                ),
            ];
        }
        match self.position {
            Position::Right => [
                Rect::from_min_size(at, vec2(a, area.height())),
                Rect::from_min_size(at + vec2(a + gap, 0.0), vec2(b, area.height())),
                Rect::from_min_size(
                    at + vec2(a + b + 2.0 * gap, 0.0),
                    vec2(settings, area.height()),
                ),
            ],
            Position::Below => [
                Rect::from_min_size(at, vec2(sizes.cross, a)),
                Rect::from_min_size(at + vec2(0.0, a + gap), vec2(sizes.cross, b)),
                Rect::from_min_size(
                    at + vec2(sizes.cross + gap, 0.0),
                    vec2(settings, area.height()),
                ),
            ],
        }
    }
}

fn fit_axis<const N: usize>(
    mut sizes: [f32; N],
    folded: [bool; N],
    area: f32,
    rail: f32,
) -> [f32; N] {
    let fixed = folded.iter().filter(|&&fold| fold).count() as f32 * rail;
    let total: f32 = sizes.iter().zip(folded).filter(|(_, fold)| !fold).map(|(size, _)| size).sum();
    if total > 0.0 {
        let ratio = (area - fixed).max(0.0) / total;
        for (size, fold) in sizes.iter_mut().zip(folded) {
            if !fold {
                *size *= ratio;
            }
        }
    }
    sizes
}

pub(crate) struct Runtime {
    area: Option<Vec2>,
    requested: Option<u64>,
    before_request: Option<Layout>,
    pub rects: [Rect; 3],
    pub bodies: [Option<(Tab, Rect)>; 3],
    grip: Option<(usize, f32, Sizes)>,
    frame: Option<u64>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            area: None,
            requested: None,
            before_request: None,
            rects: [Rect::NOTHING; 3],
            bodies: [None; 3],
            grip: None,
            frame: None,
        }
    }
}

impl Runtime {
    #[cfg(test)]
    pub(crate) fn body(&self, tab: Tab) -> Option<Rect> {
        self.bodies[Section::of(tab) as usize]
            .filter(|(active, _)| *active == tab)
            .map(|(_, rect)| rect)
    }
}

/// Draw all sections before applying any fold, so the click's frame still
/// shows the old layout. The shell applies the request before the next frame.
/// Owning the buttons directly removes the dock's flag/fraction feedback loop.
pub(crate) fn show(
    ui: &mut egui::Ui,
    layout: &mut Layout,
    runtime: &mut Runtime,
    viewer: &mut Viewer<'_>,
    frameless: bool,
) -> Option<Vec2> {
    let area = ui.available_rect_before_wrap();
    if !area.is_positive() || !area.is_finite() {
        return None;
    }
    let scale = theme::ui_scale(ui.ctx());
    let rail = if frameless { 0.0 } else { theme::tab_bar_height(scale) };
    let gap = 3.0 * scale;
    let resized = runtime.area.is_none_or(|last| (last - area.size()).length_sq() > 0.25);
    // A request is answered before the next plugin frame. Keep the guard
    // across egui's discarded/repeated passes as well as the answering frame.
    let frame = ui.ctx().cumulative_frame_nr();
    if runtime.frame != Some(frame) {
        viewer.interaction.analyzer_regions.begin_frame();
        runtime.frame = Some(frame);
    }
    let answering = runtime.requested.is_some_and(|asked| frame <= asked + 1);
    let repeated = runtime.requested == Some(frame);
    // A discarded pass must redraw the click's original geometry. Replaying
    // its controls into a temporary copy cannot apply that click twice.
    let mut replay = if repeated { runtime.before_request.clone() } else { None };
    let layout = replay.as_mut().unwrap_or(layout);
    if !layout.sized || (resized && runtime.area.is_some() && !answering) {
        layout.fit(area.size(), rail, gap);
        layout.sized = true;
    }
    if !answering {
        runtime.requested = None;
    }
    runtime.area = Some(area.size());
    let mut drawn = layout.clone();
    drawn.fit(area.size(), rail, gap);
    runtime.rects = drawn.rects(area, rail, gap);
    runtime.bodies = [None; 3];
    let before = layout.clone();
    // The dock is set on the Analyzer settings page, which draws through the
    // viewer rather than the layout; it reads and writes this copy.
    viewer.interaction.dock = layout.position;
    for section in Section::ALL {
        let rect = runtime.rects[section as usize].intersect(area);
        if rect.is_positive() {
            runtime.bodies[section as usize] =
                section_ui(ui, section, rect, layout, viewer, rail, drawn.compact());
        }
    }
    layout.position = viewer.interaction.dock;
    let open = viewer.interaction.open_settings.take();
    if let Some(page) = open.filter(|_| !repeated) {
        layout.select(Tab::Display);
        layout.folded[Section::Settings as usize] = false;
        viewer.interaction.display_page = page;
    }
    dividers(ui, layout, runtime, &drawn, scale);
    let reset = std::mem::take(&mut viewer.interaction.reset_layout);
    if repeated {
        viewer.interaction.analyzer_regions.request = None;
        return None;
    }
    let region_request = viewer.interaction.analyzer_regions.request;
    if let Some(request) = region_request {
        layout.resize_region(request.region, request.width, rail);
        viewer.interaction.analyzer_regions.land();
    }
    if reset {
        viewer.interaction.analyzer_regions = Default::default();
        *layout = Layout { sized: true, ..Layout::default() };
    }
    if reset
        || region_request.is_some()
        || (before.position, before.folded) != (layout.position, layout.folded)
    {
        runtime.grip = None;
        runtime.requested = Some(frame);
        runtime.before_request = Some(before);
        ui.ctx().request_repaint();
        return Some(layout.natural_size(rail, gap) - area.size());
    }
    None
}

fn section_ui(
    ui: &mut egui::Ui,
    section: Section,
    rect: Rect,
    layout: &mut Layout,
    viewer: &mut Viewer<'_>,
    rail: f32,
    compact: bool,
) -> Option<(Tab, Rect)> {
    let index = section as usize;
    let folded = layout.folded[index];
    let vertical_rail =
        section == Section::Settings || layout.position == Position::Right || compact;
    let tab = layout.tab(section);
    let title = match section {
        Section::Settings => "Settings",
        Section::Analyzer => "Analyzer",
        Section::Lattice => "Lattice",
    };
    let mut pane = ui.new_child(egui::UiBuilder::new().id_salt(("section", index)).max_rect(rect));
    pane.set_clip_rect(rect.intersect(ui.clip_rect()));
    pane.painter().rect_filled(rect, 0.0, theme::panel());
    if folded {
        if rail > 0.0 {
            let response = pane.interact(rect, pane.id().with("unfold"), egui::Sense::click());
            pane.painter().rect_filled(rect, 0.0, theme::well());
            crate::widgets::paint_fold(
                &pane,
                &response,
                Rect::from_min_size(rect.min, Vec2::splat(rail)),
                if vertical_rail { Vec2::X } else { Vec2::Y },
            );
            let galley = pane.painter().layout_no_wrap(
                title.into(),
                egui::TextStyle::Button.resolve(pane.style()),
                theme::text(),
            );
            if vertical_rail {
                let at = pos2(
                    rect.center().x - galley.size().y * 0.5,
                    rect.top() + rail + 8.0 + galley.size().x,
                );
                pane.painter().add(
                    egui::epaint::TextShape::new(at, galley, theme::text())
                        .with_angle(-std::f32::consts::FRAC_PI_2),
                );
            } else {
                pane.painter().galley(
                    rect.min + vec2(rail + 4.0, (rail - galley.size().y) * 0.5),
                    galley,
                    theme::text(),
                );
            }
            if response.clicked() {
                layout.folded[index] = false;
            }
        }
        return None;
    }
    let mut top = rect.top();
    if rail > 0.0 {
        // Align the header controls' right edge with the settings content gutter.
        let scale = theme::ui_scale(ui.ctx());
        let gutter = theme::pane_inner_margin(scale);
        let header_rect =
            Rect::from_min_max(rect.min, pos2(rect.right() - gutter, rect.top() + rail));
        let mut header =
            pane.new_child(egui::UiBuilder::new().id_salt("header").max_rect(header_rect));
        header.set_clip_rect(header_rect.intersect(pane.clip_rect()));
        // One gap throughout the header: the fold cell's own margin around its
        // button, which is also its gap to the pane edge and to the first tab.
        let margin = ((rail - theme::row_height(scale)) * 0.5).max(0.0);
        header.spacing_mut().item_spacing.x = margin;
        header.horizontal(|ui| {
            // The cell's margin already stands between the button and the first
            // tab, so no spacing is added after it.
            let gap = std::mem::replace(&mut ui.spacing_mut().item_spacing.x, 0.0);
            let (cell, response) = ui.allocate_exact_size(Vec2::splat(rail), egui::Sense::click());
            ui.spacing_mut().item_spacing.x = gap;
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    ui.is_enabled(),
                    format!("Collapse {title}"),
                )
            });
            crate::widgets::paint_fold(
                ui,
                &response,
                cell,
                if vertical_rail { -Vec2::X } else { -Vec2::Y },
            );
            if response.on_hover_text(format!("Collapse {title}")).clicked() {
                layout.folded[index] = true;
            }
            let tabs: &[Tab] = match section {
                Section::Lattice => &[Tab::Lattice],
                Section::Analyzer => &[Tab::Spectral, Tab::Spiral],
                Section::Settings => &[Tab::Tuning, Tab::Display, Tab::Video, Tab::Console],
            };
            let options: Vec<_> =
                tabs.iter().map(|&choice| (choice, crate::panes::tab_title(&choice), "")).collect();
            let mut selected = tab;
            let width = options
                .iter()
                .map(|(_, label, _)| crate::widgets::option_width(ui, label))
                .sum::<f32>()
                + ui.spacing().item_spacing.x * (options.len() - 1) as f32;
            if section == Section::Settings && width > ui.available_width() {
                crate::widgets::selected_combo(
                    ui,
                    egui::ComboBox::from_id_salt("section tabs")
                        .selected_text(crate::panes::tab_title(&tab))
                        .width(ui.available_width())
                        .truncate(),
                    |ui| {
                        for &(choice, label, _) in &options {
                            ui.selectable_value(&mut selected, choice, label);
                        }
                    },
                );
            } else {
                crate::widgets::choice_buttons(ui, "section tabs", &mut selected, &options);
            }
            if selected != tab {
                layout.select(selected);
            }
        });
        top += rail;
    }
    let body = Rect::from_min_max(pos2(rect.left(), top), rect.max);
    if !body.is_positive() {
        return None;
    }
    if section != Section::Settings {
        settings_link(&pane, rect, tab, &mut viewer.interaction.open_settings);
    }
    let mut tab = layout.tab(section);
    let mut content =
        pane.new_child(egui::UiBuilder::new().id_salt(viewer.id(&tab)).max_rect(body));
    content.set_clip_rect(body.intersect(pane.clip_rect()));
    if tab.is_picture() {
        viewer.ui(&mut content, &mut tab);
    } else {
        // The scroll area owns the whole body; its content has an inner
        // margin. This leaves the floating scroll bar in the outer gutter.
        let margin = theme::dock_pane_margin(theme::ui_scale(ui.ctx()));
        egui::ScrollArea::new(viewer.scroll_bars(&tab))
            .id_salt(("pane-scroll", tab))
            .auto_shrink([false, false])
            .show(&mut content, |ui| {
                egui::Frame::new().inner_margin(margin).show(ui, |ui| viewer.ui(ui, &mut tab));
            });
    }
    Some((tab, body))
}

/// A right-click anywhere on a picture section, header included, offers the
/// Display pages that set it up. It links to those pages rather than holding
/// controls of its own, so every setting keeps exactly one home.
///
/// Read from raw input rather than from a response: the analyzer senses drag
/// only, and egui gives a click to nothing beneath a drag-only widget, so no
/// response over or under the picture could see it without taking its drags or
/// its fold buttons' clicks. `rect_contains_pointer` still yields to a popup
/// or overlay covering the section.
fn settings_link(ui: &egui::Ui, rect: Rect, tab: Tab, open: &mut Option<DisplayPage>) {
    let pages: &[(DisplayPage, &str)] = match tab {
        Tab::Lattice => &[(DisplayPage::Lattice, "Lattice settings")],
        Tab::Spectral => &[
            (DisplayPage::Analyzer, "Analyzer settings"),
            (DisplayPage::Spectrogram, "Spectrogram settings"),
        ],
        Tab::Spiral => &[(DisplayPage::Analyzer, "Analyzer settings")],
        _ => return,
    };
    let clicked =
        ui.input(|input| input.pointer.secondary_clicked()) && ui.rect_contains_pointer(rect);
    // The header's own gap between a button and the edge around it.
    let scale = theme::ui_scale(ui.ctx());
    let margin = ((theme::tab_bar_height(scale) - theme::row_height(scale)) * 0.5).round() as u8;
    egui::Popup::new(
        ui.id().with("settings link"),
        ui.ctx().clone(),
        egui::PopupAnchor::PointerFixed,
        ui.layer_id(),
    )
    .kind(egui::PopupKind::Menu)
    .layout(egui::Layout::top_down_justified(egui::Align::Min))
    // egui's menu look, but with the section tabs' button padding rather than
    // its tighter one, the header's margin rather than its wider one, and no
    // outline. The corners are concentric with the highlight's, so the margin
    // holds its width around them too. Items touch: only one is ever
    // highlighted, so a gap between them would only be blank space.
    .style(move |style: &mut egui::Style| {
        let padding = style.spacing.button_padding;
        egui::containers::menu::menu_style(style);
        style.spacing.button_padding = padding;
        style.spacing.menu_margin = egui::Margin::same(margin as i8);
        style.spacing.item_spacing.y = 0.0;
        let inner = style.visuals.widgets.hovered.corner_radius.nw;
        style.visuals.menu_corner_radius = egui::CornerRadius::same(inner.saturating_add(margin));
        style.visuals.window_stroke = egui::Stroke::NONE;
    })
    .gap(0.0)
    .open_memory(clicked.then_some(egui::SetOpenCommand::Bool(true)))
    // A right-click while this menu is already open moves it to the pointer.
    // Left to the default, that same click would count as one outside the
    // menu it has just reopened, and close it again at once.
    .close_behavior(if clicked {
        egui::PopupCloseBehavior::IgnoreClicks
    } else {
        egui::PopupCloseBehavior::CloseOnClick
    })
    .show(|ui| {
        for &(page, label) in pages {
            if ui.button(label).clicked() {
                *open = Some(page);
                ui.close();
            }
        }
    });
}

fn dividers(ui: &egui::Ui, layout: &mut Layout, runtime: &mut Runtime, drawn: &Layout, scale: f32) {
    let [lattice, analyzer, settings] = runtime.rects;
    let below = drawn.position == Position::Below && !drawn.compact();
    let picture_edge = if below { lattice.right() } else { analyzer.right() };
    let bars = [
        if below {
            Rect::from_min_max(
                pos2(lattice.left(), lattice.bottom()),
                pos2(lattice.right(), analyzer.top()),
            )
        } else {
            Rect::from_min_max(
                pos2(lattice.right(), lattice.top()),
                pos2(analyzer.left(), lattice.bottom()),
            )
        },
        Rect::from_min_max(
            pos2(picture_edge, settings.top()),
            pos2(settings.left(), settings.bottom()),
        ),
    ];
    let unchanged = layout.position == drawn.position && layout.folded == drawn.folded;
    let enabled = [
        unchanged && !drawn.folded[0] && !drawn.folded[1],
        unchanged && !drawn.folded[2] && (!drawn.folded[0] || !drawn.folded[1]),
    ];
    for (index, bar) in bars.into_iter().enumerate() {
        if !bar.is_positive() {
            continue;
        }
        let horizontal = below && index == 0;
        let id = ui.id().with(("section-divider", index));
        let hit =
            if horizontal { bar.expand2(vec2(0.0, 3.0)) } else { bar.expand2(vec2(3.0, 0.0)) };
        let response = ui.interact(
            hit,
            id,
            if enabled[index] { egui::Sense::drag() } else { egui::Sense::hover() },
        );
        let color = if response.dragged() {
            theme::accent()
        } else if enabled[index] && response.hovered() {
            theme::accent_edge()
        } else {
            theme::hairline()
        };
        ui.painter().rect_filled(bar, 0.0, color);
        if !enabled[index] {
            continue;
        }
        let response = response.on_hover_cursor(if horizontal {
            egui::CursorIcon::ResizeVertical
        } else {
            egui::CursorIcon::ResizeHorizontal
        });
        if response.drag_started() {
            if let Some(at) = ui.input(|input| input.pointer.press_origin()) {
                runtime.grip = Some((index, if horizontal { at.y } else { at.x }, drawn.sizes()));
            }
        }
        if let Some((_, start, sizes)) =
            runtime.grip.filter(|(held, _, _)| *held == index && response.dragged())
        {
            let Some(at) = ui.input(|input| input.pointer.latest_pos()) else { continue };
            let delta = if horizontal { at.y } else { at.x } - start;
            let min = theme::min_pane(scale);
            let mut next = sizes;
            if index == 0 {
                let delta =
                    delta.clamp(-(sizes.lattice - min).max(0.0), (sizes.analyzer - min).max(0.0));
                next.lattice += delta;
                next.analyzer -= delta;
            } else {
                let picture = if below {
                    &mut next.cross
                } else if !layout.folded[1] {
                    &mut next.analyzer
                } else {
                    &mut next.lattice
                };
                let delta =
                    delta.clamp(-(*picture - min).max(0.0), (sizes.settings - min).max(0.0));
                *picture += delta;
                next.settings -= delta;
            }
            *layout.sizes_mut() = next;
        }
    }
    if !ui.input(|input| input.pointer.any_down()) {
        runtime.grip = None;
    }
}

#[cfg(test)]
mod tests;
