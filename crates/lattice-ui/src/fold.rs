//! Folding a pane sideways, so its neighbour gets the width.
//!
//! egui_dock's collapse arrow means "hand your space to your sibling": the
//! leaf shrinks to its tab bar and the pane next to it grows into the rest.
//! That is only true inside a VERTICAL split, though — `compute_rect_sizes`
//! special-cases `Node::Vertical` and nothing else — so a leaf in a horizontal
//! split folds its body away and keeps its column, leaving a tall void under
//! the tab bar. In this dock that is most of them: the lattice, the analyzer,
//! and the whole settings column are all children of horizontal splits, which
//! is exactly where collapsing is worth doing.
//!
//! So the horizontal half is done here, on the one lever egui_dock leaves
//! public: the parent split's `fraction`. A folded pane is squeezed to one tab
//! bar's THICKNESS — the same measure the vertical fold uses for its height,
//! and just wide enough for the collapse arrow — and its sibling takes what it
//! gave up. The result reads as a rail down the edge of the window, so
//! [`paint`] fills the rail in, turns the arrow sideways, and writes the folded
//! pane's name up it.
//!
//! Whether a pane is folded stays egui_dock's own `collapsed` flag, set by its
//! own arrow: nothing here duplicates that bookkeeping, it only reads it. What
//! does need remembering is the fraction the user dialed in, since the fold
//! overwrites it — that is what [`Folds`] holds, one entry per folded split,
//! persisted with the dock and given back the moment the pane unfolds.

use egui_dock::{DockState, Node, NodeIndex, Surface, SurfaceIndex, Tree};

use crate::panes::Tab;

/// Width of egui_dock's collapse-arrow button (its private
/// `Style::TAB_COLLAPSE_BUTTON_SIZE`), which a rail has to be able to hold or
/// there would be no way to unfold what was folded. Tab bars are taller than
/// this in every style the app uses, so the rail is one tab bar thick and the
/// button fits with room to spare; the number is only needed to repaint the
/// button's own square in [`paint`].
const ARROW_BUTTON: f32 = 24.0;

/// The `fraction` each sideways-folded split had before it folded.
///
/// Persisted with the dock (see `UiPersist`), so a pane folded when the editor
/// window closed still unfolds to the layout it came from.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Folds(Vec<Fold>);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Fold {
    /// Which split, as indices into the dock. Valid only while the tree keeps
    /// its shape — [`Folds::apply`] drops the entry as soon as the node stops
    /// being a horizontal split with a folded child, which covers re-docking.
    surface: usize,
    node: usize,
    /// What to give back on unfold.
    fraction: f32,
}

/// Which child of a split is the folded one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Left,
    Right,
}

impl Folds {
    /// Squeeze every sideways-folded pane down to a rail, and give back the
    /// fraction of every split that is not folded any more.
    ///
    /// Runs BEFORE the dock lays out, because `fraction` is the input layout
    /// reads. Idempotent: re-running it on an unchanged dock writes the same
    /// fractions, which is what keeps the rail a fixed number of POINTS wide
    /// as the window resizes (a fraction alone would grow with it).
    pub fn apply(&mut self, dock: &mut DockState<Tab>, style: &egui_dock::Style) {
        let rail = style.tab_bar.height;
        let separator = style.separator.width;
        let mut folded = Vec::new();
        for index in 0..dock.surfaces_count() {
            let surface = SurfaceIndex(index);
            let Some(tree) = dock.get_surface_mut(surface).and_then(Surface::node_tree_mut) else {
                continue;
            };
            for node in 0..tree.len() {
                let node = NodeIndex(node);
                let Some(side) = folded_side(tree, node) else {
                    continue;
                };
                let Node::Horizontal(split) = &mut tree[node] else {
                    continue;
                };
                // `Rect::NOTHING` until the dock has laid out once, and a split
                // with no room for two rails has nothing to hand over. Either
                // way the fold waits for a frame rather than dividing by a
                // width that isn't one.
                let width = split.rect.width();
                if !width.is_finite() || width <= 2.0 * (rail + separator) {
                    continue;
                }
                // A split hands each child the space up to half a separator
                // short of its midpoint, so the midpoint has to sit that much
                // further out for the rail itself to come out `rail` wide.
                let edge = (rail + separator * 0.5) / width;
                let fraction = match side {
                    Side::Left => edge,
                    Side::Right => 1.0 - edge,
                };
                // First frame of this fold: the fraction still in the split is
                // the user's, and this is the last chance to keep it.
                if !self.0.iter().any(|fold| fold.is(surface, node)) {
                    self.0.push(Fold {
                        surface: surface.0,
                        node: node.0,
                        fraction: split.fraction,
                    });
                }
                split.fraction = fraction;
                folded.push((surface.0, node.0));
            }
        }
        // Everything remembered that is no longer folded — unfolded by its
        // arrow, or re-docked out from under the entry — gets its fraction
        // back and is forgotten.
        let (kept, released): (Vec<Fold>, Vec<Fold>) =
            std::mem::take(&mut self.0).into_iter().partition(|fold| {
                folded.contains(&(fold.surface, fold.node))
            });
        self.0 = kept;
        for fold in released {
            if let Some(fraction) = horizontal_fraction(dock, fold.surface, fold.node) {
                *fraction = fold.fraction;
            }
        }
    }

    /// Forget every fold, for a dock that is being replaced wholesale (the
    /// Panel pane's "Reset layout"): the indices would otherwise name splits
    /// in a tree that no longer exists.
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

impl Fold {
    fn is(&self, surface: SurfaceIndex, node: NodeIndex) -> bool {
        self.surface == surface.0 && self.node == node.0
    }
}

/// Which child of `node` is folded sideways, if exactly one of them is.
///
/// `None` covers the cases where nothing can be handed over: a vertical split
/// (egui_dock folds those itself), neither child collapsed, and both collapsed
/// — which makes the split itself collapsed, leaving what that means to its own
/// parent.
fn folded_side(tree: &Tree<Tab>, node: NodeIndex) -> Option<Side> {
    if !tree[node].is_horizontal() {
        return None;
    }
    match (foldable(tree, node.left()), foldable(tree, node.right())) {
        (true, false) => Some(Side::Left),
        (false, true) => Some(Side::Right),
        _ => None,
    }
}

/// Whether `node` is collapsed and fits in a single rail.
fn foldable(tree: &Tree<Tab>, node: NodeIndex) -> bool {
    node.0 < tree.len() && tree[node].is_collapsed() && rail_columns(tree, node) == 1
}

/// How many rails wide `node` would be once folded: one per leaf that would end
/// up beside another.
///
/// A stack of collapsed leaves is one rail — they fold onto each other's tab
/// bars, top to bottom — which is what lets the whole settings column fold
/// away as a single rail once every pane in it is collapsed. Two collapsed
/// leaves side by side are two rails, and squeezing them into one would divide
/// it between them by their own fraction, so those are left alone.
fn rail_columns(tree: &Tree<Tab>, node: NodeIndex) -> i32 {
    if node.0 >= tree.len() {
        return 0;
    }
    match &tree[node] {
        Node::Leaf(_) => 1,
        Node::Horizontal(_) => {
            rail_columns(tree, node.left()) + rail_columns(tree, node.right())
        }
        Node::Vertical(_) => {
            rail_columns(tree, node.left()).max(rail_columns(tree, node.right()))
        }
        Node::Empty => 0,
    }
}

/// The split fraction at `(surface, node)`, if that is still a horizontal
/// split at all.
fn horizontal_fraction(
    dock: &mut DockState<Tab>,
    surface: usize,
    node: usize,
) -> Option<&mut f32> {
    let tree = dock.get_surface_mut(SurfaceIndex(surface))?.node_tree_mut()?;
    if node >= tree.len() {
        return None;
    }
    match &mut tree[NodeIndex(node)] {
        Node::Horizontal(split) => Some(&mut split.fraction),
        _ => None,
    }
}

/// Draw what a sideways fold needs and egui_dock cannot know it wants: the
/// rail's own surface, the folded pane's name up it, and a collapse arrow that
/// points along the axis the pane actually folds on.
///
/// Runs AFTER the dock, so it works from this frame's rectangles and paints
/// over the parts of the tab bar it is replacing. Everything it draws sits in
/// the tab bar's own color, so a rail and the separator beside it read as one
/// band rather than as a strip of leftover pane.
pub fn paint(ui: &egui::Ui, dock: &DockState<Tab>, style: &egui_dock::Style) {
    let rail = style.tab_bar.height;
    // Frameless mode hides every tab bar, which takes the arrow with it: a
    // fold there is a pane squeezed to nothing, with no chrome to draw.
    if rail <= 0.0 {
        return;
    }
    for index in 0..dock.surfaces_count() {
        let surface = SurfaceIndex(index);
        let Some(tree) = dock.get_surface(surface).and_then(Surface::node_tree) else {
            continue;
        };
        for node in 0..tree.len() {
            let node = NodeIndex(node);
            // A pane still open beside its sibling: only its arrow needs
            // turning, and only if THIS pane is the one that folds left/right.
            // One nested in a vertical split still folds downwards, and
            // egui_dock's own arrow already says so.
            if let Some(leaf) = tree[node].get_leaf() {
                let horizontal = node.parent().is_some_and(|parent| tree[parent].is_horizontal());
                if horizontal && !leaf.collapsed && leaf.rect.is_positive() {
                    paint_arrow(ui, leaf.rect, node.is_left(), false, style);
                }
            }
            // A folded side: the rail it left behind, and the arrow that
            // brings it back. Read from the SPLIT rather than from each leaf,
            // because a whole column can fold into one rail — and then the
            // leaves inside it are children of the vertical split, not of the
            // horizontal one that gave the width away.
            let Some(side) = folded_side(tree, node) else {
                continue;
            };
            let folded = match side {
                Side::Left => node.left(),
                Side::Right => node.right(),
            };
            let Some(rect) = tree[folded].rect() else {
                continue;
            };
            // Nothing to draw until the fold has actually been laid out: on
            // the frame a pane is collapsed on it is still its old width, and
            // a rail's worth of chrome across a whole pane would flash.
            if !rect.is_positive() || rect.width() >= 2.0 * rail {
                continue;
            }
            for leaf in leaves(tree, folded) {
                let mut body = leaf.rect;
                body.min.y += rail;
                if body.is_positive() {
                    ui.painter().rect_filled(body, egui::CornerRadius::ZERO, style.tab_bar.bg_fill);
                    if let Some(tab) = leaf.tabs.get(leaf.active.0) {
                        paint_name(ui.painter(), body, crate::panes::tab_title(tab));
                    }
                }
                paint_arrow(ui, leaf.rect, side == Side::Left, true, style);
            }
            deaden_separator(ui, rect, side, style);
        }
    }
}

/// Every leaf in the subtree at `node`, in tree order.
fn leaves(tree: &Tree<Tab>, node: NodeIndex) -> Vec<&egui_dock::LeafNode<Tab>> {
    if node.0 >= tree.len() {
        return Vec::new();
    }
    match &tree[node] {
        Node::Leaf(leaf) => vec![leaf],
        Node::Horizontal(_) | Node::Vertical(_) => {
            let mut found = leaves(tree, node.left());
            found.extend(leaves(tree, node.right()));
            found
        }
        Node::Empty => Vec::new(),
    }
}

/// Take the grab handle off the separator a rail sits against.
///
/// egui_dock keeps drawing the separator between a folded pane and its
/// neighbour, hover accent and resize cursor and all, but dragging it can no
/// longer do anything: the fold rewrites the fraction it would set on the very
/// next frame. So the invitation is withdrawn — the same thing egui_dock does
/// for a pane folded downwards, which simply has no separator at all.
fn deaden_separator(ui: &egui::Ui, rail: egui::Rect, side: Side, style: &egui_dock::Style) {
    let width = style.separator.width;
    let x = match side {
        Side::Left => rail.right()..=rail.right() + width,
        Side::Right => rail.left() - width..=rail.left(),
    };
    let band = egui::Rect::from_x_y_ranges(x, rail.y_range());
    ui.painter().rect_filled(band, egui::CornerRadius::ZERO, style.separator.color_idle);
    // The cursor is a frame-wide setting rather than a shape, so it is undone
    // by setting it again — which works only because this runs after the dock.
    if ui.rect_contains_pointer(band.expand(style.separator.extra_interact_width * 0.5)) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }
}

/// The folded pane's name, read bottom to top up its rail — the one thing a
/// rail can still say about which pane it is, and the difference between a
/// fold and a gap.
///
/// Skipped rather than clipped when the rail is too short for the whole name:
/// half a word up the side of the window says less than the arrow above it
/// already does.
fn paint_name(painter: &egui::Painter, rail: egui::Rect, name: &str) {
    const PAD: f32 = 8.0;
    let galley = painter.layout_no_wrap(
        name.to_owned(),
        egui::FontId::proportional(11.0),
        crate::theme::text_dim(),
    );
    if galley.size().x + 2.0 * PAD > rail.height() {
        return;
    }
    // Rotating a quarter turn anticlockwise maps the galley's own x onto the
    // rail's height (upwards, hence the anchor at the text's far end) and its
    // height onto the rail's width.
    let anchor = egui::pos2(
        rail.center().x - galley.size().y * 0.5,
        rail.center().y + galley.size().x * 0.5,
    );
    painter.add(
        egui::epaint::TextShape::new(anchor, galley, crate::theme::text_dim())
            .with_angle(-std::f32::consts::FRAC_PI_2),
    );
}

/// Repaint the collapse arrow to point along the axis this pane folds on:
/// outwards, at the edge it is about to fold into, and back inwards at the
/// space it will take once it unfolds.
///
/// egui_dock draws it as a disclosure triangle (down for open, right for
/// collapsed), which only reads correctly for a pane that folds downwards. The
/// button underneath is left alone and keeps handling the click; this is paint
/// over paint, including the hover fill, which is why it has to run after the
/// dock rather than before it.
fn paint_arrow(
    ui: &egui::Ui,
    leaf: egui::Rect,
    left_child: bool,
    folded: bool,
    style: &egui_dock::Style,
) {
    let button =
        egui::Rect::from_min_size(leaf.left_top(), egui::vec2(ARROW_BUTTON, style.tab_bar.height));
    let hovered = ui.rect_contains_pointer(button);
    let painter = ui.painter();
    painter.rect_filled(
        button,
        egui::CornerRadius::ZERO,
        if hovered { style.buttons.collapse_tabs_bg_fill } else { style.tab_bar.bg_fill },
    );
    let color = if hovered {
        style.buttons.collapse_tabs_active_color
    } else {
        style.buttons.collapse_tabs_color
    };
    // The same glyph size egui_dock uses (its `TAB_COLLAPSE_ARROW_SIZE`), so
    // the sideways arrow is the one the dock would have drawn, turned.
    let arrow = egui::Rect::from_center_size(button.center(), egui::Vec2::splat(10.0));
    // A left-hand pane folds to the left and unfolds to the right; a
    // right-hand one does the opposite.
    let rightwards = left_child == folded;
    painter.add(egui::Shape::convex_polygon(
        if rightwards {
            vec![arrow.left_top(), arrow.right_center(), arrow.left_bottom()]
        } else {
            vec![arrow.right_top(), arrow.left_center(), arrow.right_bottom()]
        },
        color,
        egui::Stroke::NONE,
    ));
    // egui_dock's own right-hand border, which the fill above covered.
    painter.vline(
        button.right(),
        button.y_range(),
        egui::Stroke::new(
            ui.ctx().pixels_per_point().recip(),
            style.buttons.collapse_tabs_border_color,
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dock shaped like the plugin's: the lattice on the left, a settings
    /// column on the right, with the column split into two stacked leaves.
    /// Rects are filled in as a laid-out frame of `width` would leave them,
    /// since that is what the fold divides.
    fn dock(width: f32) -> DockState<Tab> {
        let mut dock = DockState::new(vec![Tab::Lattice]);
        let surface = dock.main_surface_mut();
        let [_, right] = surface.split_right(NodeIndex::root(), 0.7, vec![Tab::Tuning]);
        surface.split_below(right, 0.5, vec![Tab::Notes]);
        let frame = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 600.0));
        surface[NodeIndex::root()].set_rect(frame);
        dock
    }

    fn style() -> egui_dock::Style {
        let mut style = egui_dock::Style::from_egui(&egui::Style::default());
        style.tab_bar.height = 26.0;
        style.separator.width = 4.0;
        style
    }

    fn fraction(dock: &DockState<Tab>, node: NodeIndex) -> f32 {
        match &dock[SurfaceIndex::main()][node] {
            Node::Horizontal(split) | Node::Vertical(split) => split.fraction,
            _ => panic!("{node:?} is not a split"),
        }
    }

    fn collapse(dock: &mut DockState<Tab>, tab: Tab, collapsed: bool) {
        let path = dock.find_tab(&tab).expect("tab is in the dock");
        dock[path.surface][path.node].set_collapsed(collapsed);
    }

    /// What egui_dock's arrow does to a leaf's ancestors once its own flag has
    /// flipped — a split whose every child is collapsed is collapsed itself.
    /// Its `node_update_collapsed` is crate-private, so a test that needs a
    /// fully collapsed column says so directly.
    fn collapse_split(dock: &mut DockState<Tab>, node: NodeIndex) {
        let surface = dock.main_surface_mut();
        surface[node].set_collapsed(true);
        surface[node].set_collapsed_leaf_count(2);
    }

    /// The whole point: the width a folded pane gives up ends up in the pane
    /// beside it, and the rail left behind is a tab bar thick rather than a
    /// fraction of the window.
    #[test]
    fn folding_a_pane_hands_its_width_to_its_sibling() {
        let mut dock = dock(1000.0);
        collapse(&mut dock, Tab::Lattice, true);
        Folds::default().apply(&mut dock, &style());
        let rail = fraction(&dock, NodeIndex::root()) * 1000.0;
        assert!((rail - 28.0).abs() < 0.01, "the lattice should be a rail wide, was {rail}");
    }

    /// The rail is a fixed number of points, so the same fold at another
    /// window size is the same rail — it must not scale with the window.
    #[test]
    fn a_rail_is_the_same_width_at_any_window_size() {
        let mut folds = Folds::default();
        let mut narrow = dock(600.0);
        let mut wide = dock(2400.0);
        for dock in [&mut narrow, &mut wide] {
            collapse(dock, Tab::Lattice, true);
        }
        folds.apply(&mut narrow, &style());
        Folds::default().apply(&mut wide, &style());
        let rail = |dock: &DockState<Tab>, width: f32| fraction(dock, NodeIndex::root()) * width;
        assert!((rail(&narrow, 600.0) - rail(&wide, 2400.0)).abs() < 0.01);
    }

    /// Unfolding is the fraction coming back, not a guess at a new one.
    #[test]
    fn unfolding_gives_back_the_fraction_the_user_had() {
        let mut dock = dock(1000.0);
        let mut folds = Folds::default();
        collapse(&mut dock, Tab::Lattice, true);
        folds.apply(&mut dock, &style());
        // Twice, because the fold is re-applied every frame: the second pass
        // must not mistake its own rail fraction for the user's.
        folds.apply(&mut dock, &style());
        collapse(&mut dock, Tab::Lattice, false);
        folds.apply(&mut dock, &style());
        assert!((fraction(&dock, NodeIndex::root()) - 0.7).abs() < 0.001);
    }

    /// A settings column folds away as one rail once everything in it is
    /// collapsed: the stacked leaves fold onto each other's tab bars, so the
    /// column itself is one rail wide.
    #[test]
    fn a_column_of_collapsed_panes_folds_as_a_single_rail() {
        let mut dock = dock(1000.0);
        collapse(&mut dock, Tab::Tuning, true);
        collapse(&mut dock, Tab::Notes, true);
        collapse_split(&mut dock, NodeIndex::root().right());
        Folds::default().apply(&mut dock, &style());
        let column = (1.0 - fraction(&dock, NodeIndex::root())) * 1000.0;
        assert!((column - 28.0).abs() < 0.01, "the column should be a rail wide, was {column}");
    }

    /// Vertical folds are egui_dock's, and it does them by rect, not fraction:
    /// touching the fraction here would move the pane the user's stacked
    /// neighbour sits at.
    #[test]
    fn a_pane_that_folds_downwards_is_left_alone() {
        let mut dock = dock(1000.0);
        collapse(&mut dock, Tab::Tuning, true);
        Folds::default().apply(&mut dock, &style());
        assert_eq!(fraction(&dock, NodeIndex::root().right()), 0.5);
        assert_eq!(fraction(&dock, NodeIndex::root()), 0.7, "the column keeps its width");
    }

    /// Two panes folded side by side would have to divide one rail between
    /// them, which is not a rail either of them can be unfolded from.
    #[test]
    fn both_children_folded_leaves_the_split_where_it_was() {
        let mut dock = dock(1000.0);
        collapse(&mut dock, Tab::Tuning, true);
        collapse(&mut dock, Tab::Notes, true);
        collapse(&mut dock, Tab::Lattice, true);
        collapse_split(&mut dock, NodeIndex::root().right());
        Folds::default().apply(&mut dock, &style());
        assert_eq!(fraction(&dock, NodeIndex::root()), 0.7);
    }
}
