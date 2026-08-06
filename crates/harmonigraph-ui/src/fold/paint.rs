//! Drawing the rails a sideways fold leaves behind, and the handles that work
//! them: the pane's own tab painted up the rail, the arrow that brings it back,
//! and the separators the fold has pinned — which still resize what a user sees
//! them dividing, by passing the drag out to the split that can move (see
//! [`shove_target`]).
//!
//! The layout these draw is [`super`]'s; nothing here decides a width. What it
//! reads back — an arrow clicked, a pinned separator dragged — goes to the next
//! frame through the collapsed flags and the fractions in the tree, which is
//! where a width becomes layout again.

use egui_dock::{DockState, Node, NodeIndex, Surface, SurfaceIndex, Tree};

use super::*;
use crate::panes::Tab;

/// Keep the separator being dragged lit for as long as the drag lasts.
///
/// egui_dock lights it from its own drag state, and a plugin editor loses that
/// the moment the pointer crosses out of the host's window: baseview reports the
/// exit, egui-baseview turns it into `PointerGone`, and the drag that strands is
/// one the shell has to end for the wheel's sake (see `end_stranded_drag`). The
/// resize itself carries on, because what it follows is the pointer rather than
/// egui's opinion of it (see [`Grip`]) — so the bar goes dark under a hand that
/// is still resizing the pane, which is the one place the two can disagree.
///
/// Drawn from the RECTANGLES the two children came out at, not from the split's
/// fraction: egui_dock updates that after drawing the separator, so a fraction
/// read here is a frame ahead of the bar on screen and this would paint beside
/// it rather than over it.
fn lit(ui: &egui::Ui, dock: &DockState<Tab>, style: &egui_dock::Style, dial: &Dial) {
    let Some(node) = dial.held() else {
        return;
    };
    let Some(tree) = dock.get_surface(SurfaceIndex::main()).and_then(Surface::node_tree) else {
        return;
    };
    let (left, right) = (node.left(), node.right());
    if right.0 >= tree.len() || !tree[node].is_horizontal() {
        return;
    }
    let (Some(whole), Some(before), Some(after)) =
        (tree[node].rect(), tree[left].rect(), tree[right].rect())
    else {
        return;
    };
    let bar = egui::Rect::from_x_y_ranges(before.right()..=after.left(), whole.y_range());
    if bar.width() <= 0.0 || bar.height() <= 0.0 {
        return;
    }
    ui.painter().rect_filled(bar, egui::CornerRadius::ZERO, style.separator.color_dragged);
}

/// Width of egui_dock's collapse-arrow button (its private
/// `Style::TAB_COLLAPSE_BUTTON_SIZE`), which a rail has to be able to hold or
/// there would be no way to unfold what was folded. Tab bars are taller than
/// this in every style the app uses, so the rail is one tab bar thick and the
/// button fits with room to spare; the number is only needed to repaint the
/// button's own square in [`paint`](fn@crate::fold::paint).
const ARROW_BUTTON: f32 = 24.0;

/// Draw what a sideways fold needs and egui_dock cannot know it wants: the
/// rail's own surface, and on each pane's stretch of it (see [`name_bands`])
/// the arrow that brings that pane back, with its name under it.
///
/// Runs AFTER the dock, so it works from this frame's rectangles and paints
/// over the parts of the tab bar it is replacing.
///
/// It takes the CLICK too, wherever egui_dock's own button for a pane is not
/// where the pane's stretch of rail begins — down a folded column that is all
/// of them but the first. Hence `&mut`: an arrow that opens nothing would be
/// worse than one in the wrong place, so the button has to move for real
/// rather than just in paint.
///
/// A rail is drawn as the pane's own TAB — the tab's fill, the tab title's type
/// and color — because that is what it has become: a pane too narrow to hold
/// anything but the tab that names it. The tab bar's darker well stays where it
/// always is, in the collapse button's square and the separator beside the
/// rail, so a rail still ends where a pane's edge would.
pub fn paint(ui: &egui::Ui, dock: &mut DockState<Tab>, style: &egui_dock::Style, dial: &Dial) {
    let rail = style.tab_bar.height;
    // Frameless mode hides every tab bar, which takes the rail with it: a fold
    // there is a pane squeezed to NOTHING, so there is no rail to draw, no name
    // to put up it and no arrow to bring the pane back with. What is still there
    // is the separator the pane left behind, and the panes on either side of it
    // that a drag on it means (see the handles below) — so the chrome is what
    // gets skipped, not the frame.
    //
    // A pane folded in frameless mode therefore has nothing on screen that
    // opens it, and comes back by leaving frameless mode (Tab) and clicking the
    // arrow that reappears on its rail. Frameless is a toggle over the same
    // layout, so nothing is stranded by that.
    let chrome = rail > 0.0;
    // The pane an arrow of ours was clicked for, and the split a pinned
    // separator passed a drag out to: both applied once the tree is no longer
    // being read from.
    let mut opened = None;
    let mut shoved = None;
    lit(ui, dock, style, dial);
    if let Some(tree) = dock.get_surface(SurfaceIndex::main()).and_then(Surface::node_tree) {
        let holds = holds(tree);
        for node in 0..tree.len() {
            let node = NodeIndex(node);
            // A folded side: the rail it left behind, and the arrow that
            // brings it back. Read from the SPLIT rather than from each leaf,
            // because a whole column can fold into one rail — and then the
            // leaves inside it are children of the vertical split, not of the
            // horizontal one that gave the width away.
            let Some(side) = folded_side(tree, node) else {
                continue;
            };
            let folded = side.of(node);
            let Some(rect) = tree[folded].rect() else {
                continue;
            };
            // Nothing here until the fold has actually been laid out: on the
            // frame a pane is collapsed on it is still its old width, and a
            // rail's worth of chrome across a whole pane would flash, while a
            // handle would sit where the pane's edge is about to stop being.
            //
            // A rail's width is the measure of settled, and in frameless mode
            // that is zero — so the slack there is a couple of points of
            // rounding rather than a rail's worth of it, and the rect is allowed
            // to have no width at all.
            let span = rail_span(rail_columns(tree, folded), rail, style.separator.width);
            if rect.height() <= 0.0 || rect.width() < 0.0 {
                continue;
            }
            if rect.width() >= span + rail.max(2.0) {
                continue;
            }
            if chrome {
                for band in name_bands(tree, folded) {
                    if paint_band(ui, &band, side, style) {
                        opened = Some(band.node);
                    }
                }
            }
            // Every separator this fold has pinned: the one the rail sits
            // against, and any between the rails of a folded pair. None of them
            // can move what is immediately on either side of it — a rail is a
            // fixed number of points — so they all resize the same thing, the
            // nearest open pane on each side, by passing the drag outward to the
            // split that divides those two (see [`shove_target`]).
            let outer = match side {
                Side::Left => rect.right()..=rect.right() + style.separator.width,
                Side::Right => rect.left() - style.separator.width..=rect.left(),
            };
            let outer = egui::Rect::from_x_y_ranges(outer, rect.y_range());
            let target = shove_target(tree, &holds, node);
            for (index, band) in
                std::iter::once(outer).chain(inner_bands(tree, folded)).enumerate()
            {
                let id = egui::Id::new(("fold band", node.0, index));
                match target {
                    // Somewhere outward to pass it: the panes move as the drag
                    // goes, which is what any separator between two panes does.
                    Some(target) => {
                        if let Some(delta) = shove(ui, band, id, style) {
                            shoved = Some((target, delta));
                        }
                    }
                    // Nowhere: the fold is holding the whole of one side of the
                    // window, so there is no second open pane for the boundary
                    // to trade against and nothing any of these separators
                    // could resize. The arrow on the rail is what brings the
                    // pane back.
                    None => deaden(ui, band, style),
                }
            }
        }
    }
    // Opened here, and held shut again by [`Folds::apply`] on the next frame
    // until the window can hold the pane (see [`Wait`]). Nothing is handed
    // over: the hold watches the collapsed flags themselves, so it does not
    // care which arrow moved them — and egui_dock's own collapse button, which
    // it draws for a folded leaf too, moves them from inside `show` where
    // nothing of ours can intercept it.
    let Some(tree) = dock.get_surface_mut(SurfaceIndex::main()).and_then(Surface::node_tree_mut)
    else {
        return;
    };
    if let Some(node) = opened {
        uncollapse(tree, node);
    }
    if let Some((node, delta)) = shoved {
        nudge(tree, node, delta, style);
    }
}

/// The split a separator a fold has pinned passes its drag out to: the nearest
/// one above the fold that still divides two panes both of which can change
/// width, which is the boundary the user is pointing at whether they know the
/// tree or not.
///
/// A separator with a rail on either side of it cannot move what it divides, and
/// neither can the split that folded the rail — its fraction is rewritten every
/// frame to hold the rail at a rail's width. What the user sees is a boundary
/// with a pane somewhere to the left of it and a pane somewhere to the right,
/// with pinned chrome in between, and dragging it moves those two: the rails
/// travel with the boundary, keeping their width, and the open panes trade.
///
/// `None` where the fold is holding the whole of one side of the window, so the
/// only pane that can change width is the folded one itself.
fn shove_target(tree: &Tree<Tab>, holds: &[Hold], fold: NodeIndex) -> Option<NodeIndex> {
    let mut node = fold;
    while let Some(parent) = node.parent() {
        let held = holds.get(parent.0).copied().unwrap_or_default();
        // Horizontal, because a vertical split divides height and has no share
        // of the width to trade; and not itself folded, or its fraction is the
        // fold's to write and a drag would be overwritten again.
        if !held.folded() && tree[parent].is_horizontal() {
            return Some(parent);
        }
        node = parent;
    }
    None
}

/// Move a split's boundary by `delta` points, as egui_dock's own separator drag
/// does it: the fraction moves by the drag over the split's width on screen, and
/// is clamped to keep `separator.extra` points of pane on either side.
///
/// Written straight into the tree, where the next frame reads it as the drag it
/// is ([`drags`]) — which is also how egui_dock's separator hands a drag
/// over, so a shoved boundary and a dragged one arrive by the same door.
fn nudge(tree: &mut Tree<Tab>, node: NodeIndex, delta: f32, style: &egui_dock::Style) {
    let Some(rect) = tree[node].rect() else {
        return;
    };
    let range = rect.width();
    if range <= 0.0 {
        return;
    }
    let min = (style.separator.extra / range).min(1.0);
    let max = 1.0 - min;
    let (min, max) = (min.min(max), max.max(min));
    if let Node::Horizontal(split) = &mut tree[node] {
        split.fraction = (split.fraction + delta / range).clamp(min, max);
    }
}

/// A separator a fold has pinned, as the resize handle for the boundary it
/// stands on: it paints and reads like any other separator, and the drag goes to
/// the split that can actually move (see [`shove_target`]).
///
/// Answers the points the boundary has moved this frame, live, because there is
/// nothing to defer: the panes it trades are both on screen.
fn shove(
    ui: &egui::Ui,
    band: egui::Rect,
    id: egui::Id,
    style: &egui_dock::Style,
) -> Option<f32> {
    let reach = egui::vec2(style.separator.extra_interact_width * 0.5, 0.0);
    let response = ui
        .interact(band.expand2(reach), id, egui::Sense::click_and_drag())
        .on_hover_and_drag_cursor(egui::CursorIcon::ResizeHorizontal);
    let color = if response.dragged() {
        style.separator.color_dragged
    } else if response.hovered() {
        style.separator.color_hovered
    } else {
        style.separator.color_idle
    };
    ui.painter().rect_filled(band, egui::CornerRadius::ZERO, color);
    let delta = response.drag_delta().x;
    (response.dragged() && delta != 0.0).then_some(delta)
}

/// One pane's stretch of a rail: its own arrow at the top, its name under it.
/// Answers whether that arrow was clicked.
///
/// The arrow goes at the top of the pane's OWN stretch, which is not where
/// egui_dock puts it. A collapsed leaf gets one tab bar at the top of its
/// rectangle, and down a folded column those rectangles start one after
/// another — so the whole column's arrows end up stacked in the first inches
/// of the rail, identical and all pointing the same way, with nothing to say
/// which pane each one opens. So this draws the arrow where it belongs and
/// takes the click there, and egui_dock's own is painted over and lifted out
/// of the frame's hit test: left live it would open a pane from a point the
/// rail now gives to another pane's name.
fn paint_band(ui: &egui::Ui, band: &Band, side: Side, style: &egui_dock::Style) -> bool {
    if !band.rect.is_positive() {
        return false;
    }
    let rail = style.tab_bar.height;
    ui.painter().rect_filled(band.rect, egui::CornerRadius::ZERO, style.tab.active.bg_fill);
    let arrow = egui::Rect::from_min_size(band.rect.left_top(), egui::vec2(ARROW_BUTTON, rail));
    let mut body = band.rect;
    body.min.y += rail;
    if let Some(tab) = band.leaf.tabs.get(band.leaf.active.0) {
        paint_name(ui, body, crate::panes::tab_title(tab), style);
    }
    paint_arrow(ui, arrow, side, style);
    // Where egui_dock already put the button, a rail with one pane on it and
    // the first pane of a column alike: its own arrow is under ours and needs
    // nothing from us.
    if (band.rect.top() - band.leaf.rect.top()).abs() < 0.5 {
        return false;
    }
    let id = egui::Id::new(("fold arrow", band.node.0));
    let clicked = ui.interact(arrow, id, egui::Sense::click()).clicked();
    ui.interact(arrow_button(band.leaf.rect, style), id.with("stacked"), egui::Sense::click());
    clicked
}

/// Each pane in a folded subtree with the stretch of rail that is ITS pane:
/// the arrow that brings it back at the top, its name below that.
///
/// A folded column is ONE rail holding several panes, and egui_dock's division
/// of it is all or nothing: a collapsed leaf is one tab bar tall and whichever
/// is last takes everything left over. So every arrow in the column lands in
/// the first few inches of the rail — one tab bar each, stacked, all pointing
/// the same way — and every name but the last had a 0px body to go in, which
/// is how a folded settings column came to read as "Notes", the pane at the
/// bottom of it.
///
/// The rail is shared out instead by the fractions the column is dialled at,
/// the same ones that decide the panes' heights when it is open. Each pane
/// gets a stretch of rail the size it will come back at, with the button that
/// brings it back at the top of it — the rail as a miniature of the column it
/// restores, rather than a stack of arrows over a list of names.
struct Band<'a> {
    node: NodeIndex,
    leaf: &'a egui_dock::LeafNode<Tab>,
    rect: egui::Rect,
}

fn name_bands(tree: &Tree<Tab>, node: NodeIndex) -> Vec<Band<'_>> {
    let mut bands = Vec::new();
    let Some(rect) = tree[node].rect() else {
        return bands;
    };
    divide(tree, node, rect.top(), rect.bottom(), &mut bands);
    bands
}

/// Share a folded subtree's height out among its panes, as
/// [`Points::spread`](super::Points::spread) shares out its width.
fn divide<'a>(
    tree: &'a Tree<Tab>,
    node: NodeIndex,
    top: f32,
    bottom: f32,
    bands: &mut Vec<Band<'a>>,
) {
    if node.0 >= tree.len() {
        return;
    }
    match &tree[node] {
        Node::Leaf(leaf) => bands.push(Band {
            node,
            leaf,
            rect: egui::Rect::from_x_y_ranges(leaf.rect.x_range(), top..=bottom),
        }),
        Node::Vertical(split) => {
            let mid = top + (bottom - top) * split.fraction;
            divide(tree, node.left(), top, mid, bands);
            divide(tree, node.right(), mid, bottom, bands);
        }
        // Panes side by side, a rail each: they divide the fold's WIDTH, so
        // both of them get the whole of its height.
        Node::Horizontal(_) => {
            divide(tree, node.left(), top, bottom, bands);
            divide(tree, node.right(), top, bottom, bands);
        }
        Node::Empty => {}
    }
}

/// Every separator inside the subtree at `node`: the gaps between panes that
/// have folded into rails beside each other.
fn inner_bands(tree: &Tree<Tab>, node: NodeIndex) -> Vec<egui::Rect> {
    if node.0 >= tree.len() || !tree[node].is_parent() {
        return Vec::new();
    }
    let (left, right) = (node.left(), node.right());
    if right.0 >= tree.len() {
        return Vec::new();
    }
    let mut bands = inner_bands(tree, left);
    bands.extend(inner_bands(tree, right));
    if let Node::Horizontal(split) = &tree[node] {
        if let (Some(before), Some(after)) = (tree[left].rect(), tree[right].rect()) {
            bands.push(egui::Rect::from_x_y_ranges(
                before.right()..=after.left(),
                split.rect.y_range(),
            ));
        }
    }
    bands
}

/// Take the resize handle off a separator a fold has pinned with nothing outward
/// to pass a drag to — a fold holding the whole of one side of the window, where
/// there is no second open pane for the boundary to trade against.
///
/// egui_dock keeps drawing them, hover accent and resize cursor and all, and
/// dragging one cannot do anything: neither side of it can change width, the fold
/// rewrites the fraction it would set on the very next frame, and here there is
/// no boundary further out that the drag would mean instead. So the invitation is
/// withdrawn — the same thing egui_dock does for a pane folded downwards, which
/// simply has no separator at all.
///
/// A rail's arrow is what brings its pane back, and the only thing that does.
/// The separator beside it could instead be the pane's own handle, unfolding it
/// at the width it is dragged to — but a width dragged out of a rail is one the
/// pane can be left too narrow to have a working separator at, since a pane
/// under `separator.extra` saturates the split's fraction and [`drags`] reads a
/// saturated fraction as egui_dock's own clamp (see [`unmoved`]). One
/// affordance for one thing costs nothing here: the arrow is on the rail, in
/// view, and it hands the pane back at the width it folded from.
///
/// The separator a fold has pinned with somewhere outward to pass a drag DOES
/// resize, and [`shove`] is where it goes.
fn deaden(ui: &egui::Ui, band: egui::Rect, style: &egui_dock::Style) {
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
/// Set as the pane's own tab title would be, in the same type and color, since
/// that is what it stands in for: a rail is the tab of a pane too narrow to
/// hold one, and `TextStyle::Button` is what egui_dock lays a tab title out in.
///
/// Hung from the top of `body`, which is the pane's stretch of rail below its
/// own arrow: the name captions the button that brings the pane back. Centred,
/// it would drift half a window away from that button down a tall rail, and a
/// name that far off says nothing about which arrow it belongs to.
///
/// Skipped rather than clipped when the stretch is too short for the whole
/// name: half a word up the side of the window says less than the arrow above
/// it already does.
fn paint_name(ui: &egui::Ui, body: egui::Rect, name: &str, style: &egui_dock::Style) {
    const PAD: f32 = 8.0;
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(
        name.to_owned(),
        egui::TextStyle::Button.resolve(ui.style()),
        style.tab.active.text_color,
    );
    if !body.is_positive() || galley.size().x + 2.0 * PAD > body.height() {
        return;
    }
    // Rotating a quarter turn anticlockwise maps the galley's own x onto the
    // rail's height (upwards, hence the anchor at the text's far end) and its
    // height onto the rail's width.
    let anchor = egui::pos2(
        body.center().x - galley.size().y * 0.5,
        body.top() + PAD + galley.size().x,
    );
    painter.add(
        egui::epaint::TextShape::new(anchor, galley, style.tab.active.text_color)
            .with_angle(-std::f32::consts::FRAC_PI_2),
    );
}

/// egui_dock's own collapse button for a leaf: its square at the left end of
/// the leaf's tab bar, which for a folded pane is the top of its rect.
fn arrow_button(leaf: egui::Rect, style: &egui_dock::Style) -> egui::Rect {
    egui::Rect::from_min_size(leaf.left_top(), egui::vec2(ARROW_BUTTON, style.tab_bar.height))
}

/// Repaint a rail's collapse arrow to point at the space its pane will take
/// when it comes back: rightwards out of a rail on the left, leftwards out of
/// one on the right.
///
/// Only a rail gets this. An open pane keeps egui_dock's disclosure triangle
/// (down for open, right for collapsed), which claims no direction at all and
/// therefore cannot disagree with the pane next to it — and two open panes DO
/// fold opposite ways, since each shrinks toward its own outer edge, so a
/// direction there is a mismatch on display for no gain: the tab title beside
/// it already says which pane it is. A rail has no title, and which way its
/// pane returns is the only thing left to say.
///
/// The button underneath is left alone and keeps handling the click; this is
/// paint over paint, including the hover fill, which is why it has to run after
/// the dock rather than before it.
fn paint_arrow(ui: &egui::Ui, button: egui::Rect, side: Side, style: &egui_dock::Style) {
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
    painter.add(egui::Shape::convex_polygon(
        if side == Side::Left {
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