//! Three fixed sections, with remembered sizes independent of folded rails.
//! The shell owns the outer window; this module requests a size once and fits
//! the picture to whatever the host actually grants without changing its memory.

use egui::{pos2, vec2, Rect, Vec2};
use serde::{Deserialize, Serialize};

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

    /// The section's tabs, in strip order. The Settings tabs run from the
    /// pictures' own settings to the editor's, so the ones a narrow column
    /// folds into its overflow menu first are the ones visited least.
    pub(crate) const fn tabs(self) -> &'static [Tab] {
        match self {
            Self::Lattice => &[Tab::Lattice],
            Self::Analyzer => &[Tab::Spectral, Tab::Spiral],
            Self::Settings => &[
                Tab::Tuning,
                Tab::LatticeSettings,
                Tab::AnalyzerSettings,
                Tab::Mappings,
                Tab::Video,
                Tab::System,
                Tab::Console,
            ],
        }
    }

    /// The section whose strip lists `tab`. Read off [`Self::tabs`] rather
    /// than matched a second time, so a new tab cannot be put on one strip and
    /// filed under another section by a catch-all arm.
    pub(crate) fn of(tab: Tab) -> Self {
        Self::ALL
            .into_iter()
            .find(|section| section.tabs().contains(&tab))
            .expect("every tab is on exactly one section's strip")
    }
}

/// Picture sizes along the arrangement's axis, its shared cross-axis size,
/// and the settings column width. These include the section's own header.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Only the Spectral tab has those regions, so another analyzer tab is
    /// laid out with it paid back (see [`Layout::repaid`]).
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
        self.sizes_at(self.position)
    }

    fn sizes_at(&mut self, position: Position) -> &mut Sizes {
        match position {
            Position::Right => &mut self.right,
            Position::Below => &mut self.below,
        }
    }

    /// Width the Spectral tab's folded regions took from the Analyzer, lent
    /// back while another analyzer tab shows: that tab has no region rail to
    /// unfold them from. Derived rather than stored, so a tab switch edits no
    /// saved width and returning to Spectral lands on exactly its old layout.
    ///
    /// Never more than the shown sections' own stored widths. A fixed lend
    /// would let a window shrunk below it fit the stored sizes into nothing,
    /// and write those zeros into the saved layout; capped, a narrow enough
    /// window shrinks the lend along with everything else (see [`Self::fit`]).
    /// Dividers edit only the stored sizes, so while this is lent the Analyzer
    /// cannot be dragged narrower than it plus the minimum pane.
    fn repaid(&self) -> f32 {
        let right = self.right;
        let shown: f32 = [right.lattice, right.analyzer, right.settings]
            .into_iter()
            .zip(self.folded)
            .filter(|(_, folded)| !folded)
            .map(|(size, _)| size)
            .sum();
        self.lent().min(shown)
    }

    /// The whole folded-region width, when another analyzer tab shows.
    fn lent(&self) -> f32 {
        let lent = self.position == Position::Right
            && !self.folded[Section::Analyzer as usize]
            && self.analyzer_tab != Tab::Spectral;
        if lent {
            self.region_widths.iter().sum()
        } else {
            0.0
        }
    }

    /// The sizes the sections are drawn at: the stored ones plus [`Self::repaid`].
    fn laid_out(&self) -> Sizes {
        let mut sizes = self.sizes();
        sizes.analyzer += self.repaid();
        sizes
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

    fn resize_region(&mut self, region: usize, width: Option<f32>, min_pane: f32) {
        if let Some(width) = width {
            let removed = if self.position == Position::Right {
                width.max(0.0).min((self.right.analyzer - min_pane).max(0.0))
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

    /// Which sections take any room: open ones, and folded ones while there is
    /// a rail to fold them to. With the tab bars hidden a folded section is
    /// nothing, and the gap beside it would be a bare divider at the edge.
    fn takes_room(&self, rail: f32) -> [bool; 3] {
        self.folded.map(|folded| !folded || rail > 0.0)
    }

    /// The window these sizes ask for. A settings column saved narrower than
    /// `floor` asks for the floor, which is what [`Self::rects`] draws it at,
    /// so the pictures are not narrowed to pay for the difference. An open
    /// column has usually been saved at the floor already ([`Self::settled`]);
    /// one being unfolded, or one saved while a lend was in force, has not.
    fn natural_size(&self, rail: f32, gap: f32, floor: f32) -> Vec2 {
        let sizes = self.laid_out();
        let extent = |index, size| if self.folded[index] { rail } else { size };
        let a = extent(0, sizes.lattice);
        let b = extent(1, sizes.analyzer);
        let settings = extent(2, sizes.settings.max(floor));
        let room = self.takes_room(rail);
        if self.compact() {
            return vec2(
                stack([rail, rail, settings], room, gap).1,
                stack([rail, rail], [room[0], room[1]], gap).1,
            );
        }
        match self.position {
            Position::Right => vec2(stack([a, b, settings], room, gap).1, sizes.cross),
            Position::Below => vec2(
                stack([sizes.cross, settings], [true, room[2]], gap).1,
                stack([a, b], [room[0], room[1]], gap).1,
            ),
        }
    }

    /// Fit only visible sections. Hidden dimensions are never charged for a
    /// window resize, and a host refusal only fits a temporary drawing copy.
    fn fit(&mut self, area: Vec2, rail: f32, gap: f32) {
        let folded = self.folded;
        let room = self.takes_room(rail);
        let compact = self.compact();
        let position = self.position;
        let lent = self.lent();
        let sizes = self.sizes_mut();
        if compact {
            if !folded[2] {
                sizes.settings = (area.x - 2.0 * rail - gaps(room, gap)).max(0.0);
            }
            return;
        }
        match position {
            Position::Right => {
                // Scale the held region widths with the visible sections.
                // Spiral borrows them back as `repaid`, so that lend must
                // share the same scale or the fitted widths will not fill
                // the window after the held widths change.
                let rails = folded.iter().filter(|&&fold| fold).count() as f32 * rail;
                let free = (area.x - gaps(room, gap) - rails).max(0.0);
                let shown: f32 = [sizes.lattice, sizes.analyzer, sizes.settings]
                    .into_iter()
                    .zip(folded)
                    .filter(|(_, fold)| !fold)
                    .map(|(size, _)| size)
                    .sum();
                let repay_source = lent.min(shown);
                let repaid =
                    if shown > 0.0 { repay_source * free / (shown + repay_source) } else { 0.0 };
                let before_analyzer = sizes.analyzer;
                let widths = fit_axis(
                    [sizes.lattice, sizes.analyzer, sizes.settings],
                    folded,
                    area.x - gaps(room, gap) - repaid,
                    rail,
                );
                [sizes.lattice, sizes.analyzer, sizes.settings] = widths;
                sizes.cross = area.y;
                if !folded[Section::Analyzer as usize] && before_analyzer > 0.0 {
                    let scale = sizes.analyzer / before_analyzer;
                    for width in &mut self.region_widths {
                        *width *= scale;
                    }
                }
            }
            Position::Below => {
                let widths = fit_axis(
                    [sizes.cross, sizes.settings],
                    [false, folded[2]],
                    area.x - gaps([true, room[2]], gap),
                    rail,
                );
                [sizes.cross, sizes.settings] = widths;
                let heights = fit_axis(
                    [sizes.lattice, sizes.analyzer],
                    [folded[0], folded[1]],
                    area.y - gaps([room[0], room[1]], gap),
                    rail,
                );
                [sizes.lattice, sizes.analyzer] = heights;
            }
        }
    }

    /// `sizes` with an open settings column narrower than `floor` widened to
    /// it out of the open pictures beside it, in proportion to their widths and
    /// as far as they have width to give. The total is unchanged.
    ///
    /// Applied when drawing ([`Self::rects`]) rather than in [`Self::fit`],
    /// which keeps the saved sizes in the proportions the window was fitted at:
    /// a window narrowed past the floor and widened again comes back to the same
    /// layout, rather than to one whose settings column kept the share the floor
    /// took. And the Spiral tab's lent width (see [`Self::repaid`]) is reckoned
    /// on those proportional sizes, so it still adds up to the window it was
    /// fitted to. An edit of the layout saves it first ([`Self::settled`]).
    fn floored(&self, mut sizes: Sizes, floor: f32) -> Sizes {
        let below = self.position == Position::Below;
        if !self.folded[2] && !self.compact() && sizes.settings < floor {
            let open = |index: usize| !self.folded[index];
            let pictures = if below {
                sizes.cross
            } else {
                [sizes.lattice, sizes.analyzer]
                    .into_iter()
                    .zip([open(0), open(1)])
                    .filter(|(_, open)| *open)
                    .map(|(size, _)| size)
                    .sum()
            };
            let taken = (floor - sizes.settings).min(pictures);
            if taken > 0.0 {
                let ratio = (pictures - taken) / pictures;
                if below {
                    sizes.cross *= ratio;
                } else {
                    for (size, index) in [(&mut sizes.lattice, 0), (&mut sizes.analyzer, 1)] {
                        if open(index) {
                            *size *= ratio;
                        }
                    }
                }
                sizes.settings += taken;
            }
        }
        sizes
    }

    /// The saved sizes as they are drawn, floor and all ([`Self::floored`]),
    /// for an edit to start from: a drag of the settings divider, a fold, a dock
    /// move, a region fold. Measured against saved sizes the floor had
    /// overridden, a drag widening the column would have to make up the
    /// difference before it moved, and a fold would hand the other picture back
    /// its share of it. A window resize never saves them, and nor does a switch
    /// of analyzer tab, which [`Self::repaid`] promises edits no saved width.
    ///
    /// `None` while the Spiral tab is borrowing a folded region's width
    /// ([`Self::lent`]): the drawn analyzer carries that lend and the saved one
    /// does not, so saving the drawn width would mean rescaling the region
    /// widths the lend is made of, and a lend wider than the window has no
    /// consistent answer at all. A drag there keeps the gap between the saved
    /// width and the floor, a rarity of a rarity.
    fn settled(&self, floor: f32) -> Option<Sizes> {
        (self.lent() == 0.0).then(|| self.floored(self.sizes(), floor))
    }

    /// Where each section is drawn: the saved sizes, [`Self::floored`].
    fn rects(&self, area: Rect, rail: f32, gap: f32, floor: f32) -> [Rect; 3] {
        let sizes = self.floored(self.laid_out(), floor);
        let extent = |index, size| if self.folded[index] { rail } else { size };
        let a = extent(0, sizes.lattice);
        let b = extent(1, sizes.analyzer);
        let settings = extent(2, sizes.settings);
        let room = self.takes_room(rail);
        let at = area.min;
        let column =
            |x: f32, width: f32| Rect::from_min_size(at + vec2(x, 0.0), vec2(width, area.height()));
        if self.compact() {
            let ([x0, x1, x2], _) = stack([rail, rail, settings], room, gap);
            return [column(x0, rail), column(x1, rail), column(x2, settings)];
        }
        match self.position {
            Position::Right => {
                let ([x0, x1, x2], _) = stack([a, b, settings], room, gap);
                [column(x0, a), column(x1, b), column(x2, settings)]
            }
            Position::Below => {
                let ([y0, y1], _) = stack([a, b], [room[0], room[1]], gap);
                let ([_, x], _) = stack([sizes.cross, settings], [true, room[2]], gap);
                [
                    Rect::from_min_size(at + vec2(0.0, y0), vec2(sizes.cross, a)),
                    Rect::from_min_size(at + vec2(0.0, y1), vec2(sizes.cross, b)),
                    column(x, settings),
                ]
            }
        }
    }
}

/// Where each extent starts along one axis, and where the last one ends, with
/// a gap only between sections that [`Layout::takes_room`] marks.
fn stack<const N: usize>(extents: [f32; N], room: [bool; N], gap: f32) -> ([f32; N], f32) {
    let mut starts = [0.0; N];
    let mut at = 0.0;
    let mut any = false;
    for ((start, extent), room) in starts.iter_mut().zip(extents).zip(room) {
        if room && std::mem::replace(&mut any, true) {
            at += gap;
        }
        *start = at;
        at += extent;
    }
    (starts, at)
}

/// The total gap [`stack`] puts between the sections, for [`Layout::fit`],
/// which must set it aside before it knows their sizes.
fn gaps<const N: usize>(room: [bool; N], gap: f32) -> f32 {
    stack([0.0; N], room, gap).1
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
    grip: Option<Grip>,
    frame: Option<u64>,
}

#[derive(Clone, Copy)]
struct Grip {
    divider: usize,
    start: f32,
    saved: Sizes,
    pointer_to_saved: f32,
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
    // The gap between sections is the divider itself, so it is drawn a
    // hairline; `dividers` widens the band a hand can grab around it.
    let gap = 1.0 * scale;
    let floor = theme::min_settings(scale);
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
    runtime.rects = drawn.rects(area, rail, gap, floor);
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
    if let Some(tab) = open.filter(|_| !repeated) {
        layout.select(tab);
        layout.folded[Section::Settings as usize] = false;
    }
    // Compared in saved units: `drawn` carries a lent width at the window's
    // fit ratio, which differs from the saved one whenever the window is not
    // exactly the size the lend asked for.
    let unchanged = (before.position, before.folded, before.repaid())
        == (layout.position, layout.folded, layout.repaid());
    dividers(ui, layout, runtime, &drawn, unchanged, scale, floor);
    let reset = std::mem::take(&mut viewer.interaction.reset_layout);
    if repeated {
        viewer.interaction.analyzer_regions.request = None;
        return None;
    }
    let region_request = viewer.interaction.analyzer_regions.request;
    // A fold, a dock move or a region fold reshapes the layout, starting from
    // the layout as drawn. A switch between analyzer tabs, which lends or takes
    // back the region width, edits no saved width but asks the window for its
    // new size the same way.
    let reshaped = region_request.is_some()
        || (before.position, before.folded) != (layout.position, layout.folded);
    if let Some(sizes) = before.settled(floor).filter(|_| reshaped) {
        *layout.sizes_at(before.position) = sizes;
    }
    let edited = reshaped || before.repaid() != layout.repaid();
    if let Some(request) = region_request {
        layout.resize_region(request.region, request.width, theme::min_pane(scale));
        viewer.interaction.analyzer_regions.land();
    }
    if reset {
        viewer.interaction.analyzer_regions = Default::default();
        *layout = Layout { sized: true, ..Layout::default() };
    }
    if reset || edited {
        runtime.grip = None;
        runtime.requested = Some(frame);
        runtime.before_request = Some(before);
        ui.ctx().request_repaint();
        return Some(layout.natural_size(rail, gap, floor) - area.size());
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
            pane.painter().rect_filled(rect, 0.0, theme::header());
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
        let gutter = theme::pane_gutter(scale);
        let header_rect =
            Rect::from_min_max(rect.min, pos2(rect.right() - gutter, rect.top() + rail));
        // The header is chrome, not the first row of the page: its own fill,
        // across the pane's full width. Its edge is the fill's; a rule there
        // would be one more line in the colour of the page's own section rules.
        let band = Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + rail));
        pane.painter().rect_filled(band, 0.0, theme::header());
        let mut header =
            pane.new_child(egui::UiBuilder::new().id_salt("header").max_rect(header_rect));
        header.set_clip_rect(header_rect.intersect(pane.clip_rect()));
        // One gap throughout the header: the fold cell's own margin around its
        // button, which is also its gap to the pane edge and to the first tab.
        header.spacing_mut().item_spacing.x = theme::button_gap(scale);
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
            let options: Vec<_> = section
                .tabs()
                .iter()
                .map(|&choice| (choice, crate::panes::tab_title(&choice)))
                .collect();
            let mut selected = tab;
            crate::widgets::tab_strip(ui, "section tabs", &mut selected, &options);
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
/// settings page that sets it up. It links to those pages rather than holding
/// controls of its own, so every setting keeps exactly one home.
///
/// Read from raw input rather than from a response: the analyzer senses drag
/// only, and egui gives a click to nothing beneath a drag-only widget, so no
/// response over or under the picture could see it without taking its drags or
/// its fold buttons' clicks. `rect_contains_pointer` still yields to a popup
/// or overlay covering the section.
fn settings_link(ui: &egui::Ui, rect: Rect, tab: Tab, open: &mut Option<Tab>) {
    let pages: &[(Tab, &str)] = match tab {
        Tab::Lattice => &[(Tab::LatticeSettings, "Lattice settings")],
        Tab::Spectral | Tab::Spiral => &[(Tab::AnalyzerSettings, "Analyzer settings")],
        _ => return,
    };
    let clicked =
        ui.input(|input| input.pointer.secondary_clicked()) && ui.rect_contains_pointer(rect);
    egui::Popup::new(
        ui.id().with("settings link"),
        ui.ctx().clone(),
        egui::PopupAnchor::PointerFixed,
        ui.layer_id(),
    )
    .kind(egui::PopupKind::Menu)
    .layout(egui::Layout::top_down_justified(egui::Align::Min))
    .style(crate::widgets::menu_style(ui.ctx()))
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

fn dividers(
    ui: &egui::Ui,
    layout: &mut Layout,
    runtime: &mut Runtime,
    drawn: &Layout,
    unchanged: bool,
    ui_scale: f32,
    floor: f32,
) {
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
            if horizontal { bar.expand2(vec2(0.0, 4.0)) } else { bar.expand2(vec2(4.0, 0.0)) };
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
                // The pictures' divider is measured against the lattice as
                // drawn: the floor narrows both pictures by one ratio, which
                // holds through a drag that only trades width between them.
                // The settings divider is measured against the fitted width,
                // and starts from the drawn one (`settled`), which the floor
                // moves by a fixed amount rather than a ratio.
                let (source, target) = if index == 0 {
                    let lattice = if horizontal { lattice.height() } else { lattice.width() };
                    (layout.sizes().lattice, lattice)
                } else {
                    (layout.sizes().settings, drawn.sizes().settings)
                };
                let drag_scale = if target > 1.0 { source / target } else { 1.0 };
                if let Some(sizes) = layout.settled(floor).filter(|_| index == 1) {
                    *layout.sizes_mut() = sizes;
                }
                let saved = layout.sizes();
                runtime.grip = Some(Grip {
                    divider: index,
                    start: if horizontal { at.y } else { at.x },
                    saved,
                    pointer_to_saved: drag_scale,
                });
            }
        }
        if let Some(grip) = runtime.grip.filter(|grip| grip.divider == index && response.dragged())
        {
            let Some(at) = ui.input(|input| input.pointer.latest_pos()) else { continue };
            let delta = (if horizontal { at.y } else { at.x } - grip.start) * grip.pointer_to_saved;
            let min = theme::min_pane(ui_scale);
            let sizes = grip.saved;
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
                    delta.clamp(-(*picture - min).max(0.0), (sizes.settings - floor).max(0.0));
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
