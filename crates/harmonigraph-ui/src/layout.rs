//! Where the panes go in a composed frame.
//!
//! Shared by the offline renderer (which composes a video frame headlessly)
//! and the plugin's Video panel (which previews that exact frame). A layout
//! is deliberately NOT the plugin's working dock — it is a clean composition
//! of just the panes you want in the video, at whatever proportions suit the
//! piece, with no tab bars or settings columns. Keeping it here, next to the
//! panes it places, is what lets the preview and the render be the same
//! picture.
//!
//! A layout is a list of panes with fractional rectangles, so the same layout
//! means the same picture at any output size:
//!
//! ```ron
//! (
//!     background: (14, 14, 18),
//!     margin: 24.0,
//!     gap: 16.0,
//!     panes: [
//!         (pane: Lattice,  rect: (0.0, 0.0, 0.68, 1.0)),
//!         (pane: Spectral, rect: (0.68, 0.0, 1.0, 1.0)),
//!     ],
//! )
//! ```
//!
//! `rect` is `(x0, y0, x1, y1)` as fractions of the frame, origin top-left.
//! Panes are drawn in order, so a later one overlaps an earlier one — which is
//! how you'd inset a small roll over a full-bleed lattice, if that's the look
//! you want.

use serde::{Deserialize, Serialize};

use crate::{LatticeSide, Pane};

/// One pane and the slice of the frame it fills.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Placement {
    pub pane: Pane,
    /// `(x0, y0, x1, y1)` in `0..1` of the frame, origin top-left.
    pub rect: (f32, f32, f32, f32),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layout {
    /// Frame background, RGB. Shows through the margin and the gaps.
    #[serde(default = "default_background")]
    pub background: (u8, u8, u8),
    /// Inset of the whole picture from the frame edge, in points.
    #[serde(default)]
    pub margin: f32,
    /// Space between panes, in points. Applied as a half-gap inset on
    /// every pane edge that isn't already on the picture's boundary —
    /// which is close enough, and means adjacent panes get one gap
    /// between them rather than two.
    #[serde(default)]
    pub gap: f32,
    pub panes: Vec<Placement>,
}

fn default_background() -> (u8, u8, u8) {
    // The theme's own window color, so margins and gaps read as part of
    // the UI rather than as letterboxing.
    (14, 14, 18)
}

/// The layouts you get by name. Deliberately few: these are the two
/// arrangements the panes were designed around, plus each pane alone.
pub const PRESETS: [&str; 5] = ["side-by-side", "stacked", "lattice", "spectral", "spiral"];

impl Layout {
    /// A preset by name, or `None` if it isn't one.
    pub fn preset(name: &str) -> Option<Layout> {
        let full = |pane| Placement { pane, rect: (0.0, 0.0, 1.0, 1.0) };
        let panes = match name {
            // The lattice leads and the Spectral pane takes a tall column
            // beside it. The pane draws at whatever orientation is SET, and
            // deliberately does not read this column's shape to pick one: a
            // render whose picture depends on the aspect it is rendered at is
            // not one you can dial in. Top (or Bottom) is the orientation this
            // column wants — Left leaves the spectrogram scrolling across a
            // narrow pane — so it has to be chosen, here or in the pane.
            "side-by-side" => vec![
                Placement { pane: Pane::Lattice, rect: (0.0, 0.0, 0.68, 1.0) },
                Placement { pane: Pane::Spectral, rect: (0.68, 0.0, 1.0, 1.0) },
            ],
            // The plugin's default arrangement: a wide roll under the
            // lattice, sharing the pitch axis left to right.
            "stacked" => vec![
                Placement { pane: Pane::Lattice, rect: (0.0, 0.0, 1.0, 0.74) },
                Placement { pane: Pane::Spectral, rect: (0.0, 0.74, 1.0, 1.0) },
            ],
            "lattice" => vec![full(Pane::Lattice)],
            "spectral" => vec![full(Pane::Spectral)],
            // The spiral is a DISC, so it takes what it is given and centres
            // itself in it; a composition wanting it beside something else is a
            // hand-written `.ron`, which is what the `.ron` is for.
            "spiral" => vec![full(Pane::Spiral)],
            _ => return None,
        };
        Some(Layout { background: default_background(), margin: 0.0, gap: 8.0, panes })
    }

    /// A preset name or a path to a `.ron` file.
    pub fn load(spec: &str) -> Result<Layout, String> {
        if let Some(preset) = Layout::preset(spec) {
            return Ok(preset);
        }
        let text = std::fs::read_to_string(spec).map_err(|e| {
            format!(
                "layout {spec:?} is not a preset ({}) and could not be read: {e}",
                PRESETS.join(", ")
            )
        })?;
        ron::from_str(&text).map_err(|e| format!("layout {spec:?}: {e}"))
    }

    /// A two-pane composition of the lattice and the spectral pane, with the
    /// lattice on `side` taking `fraction` of the frame. Shared by the Video
    /// panel's live preview and the offline renderer, so both compose the
    /// identical frame.
    ///
    /// `fraction` is the LATTICE's share on all four sides — the spectral pane
    /// gets the complement — so the split bar keeps meaning one thing when the
    /// side changes under it.
    pub fn split(side: LatticeSide, fraction: f32) -> Layout {
        let f = fraction.clamp(0.05, 0.95);
        let rest = 1.0 - f;
        let (lattice, spectral) = match side {
            LatticeSide::Left => ((0.0, 0.0, f, 1.0), (f, 0.0, 1.0, 1.0)),
            LatticeSide::Right => ((rest, 0.0, 1.0, 1.0), (0.0, 0.0, rest, 1.0)),
            LatticeSide::Top => ((0.0, 0.0, 1.0, f), (0.0, f, 1.0, 1.0)),
            LatticeSide::Bottom => ((0.0, rest, 1.0, 1.0), (0.0, 0.0, 1.0, rest)),
        };
        let panes = vec![
            Placement { pane: Pane::Lattice, rect: lattice },
            Placement { pane: Pane::Spectral, rect: spectral },
        ];
        Layout { background: default_background(), margin: 0.0, gap: 8.0, panes }
    }

    /// Resolve to screen rectangles inside a frame of `size` points.
    /// Panes that come out empty (a zero-width slice, or a gap wider than
    /// the pane) are dropped rather than drawn degenerate.
    pub fn resolve(&self, size: egui::Vec2) -> Vec<(Pane, egui::Rect)> {
        let frame = egui::Rect::from_min_size(egui::Pos2::ZERO, size).shrink(self.margin);
        let half_gap = self.gap * 0.5;
        self.panes
            .iter()
            .filter_map(|placement| {
                let (x0, y0, x1, y1) = placement.rect;
                let at = |fx: f32, fy: f32| {
                    egui::pos2(frame.left() + frame.width() * fx, frame.top() + frame.height() * fy)
                };
                let mut rect = egui::Rect::from_two_pos(at(x0, y0), at(x1, y1));
                // Half a gap on every edge that meets another pane, and
                // nothing on the edges that sit on the picture's own
                // boundary — so neighbours get exactly one gap between
                // them and the outside border stays the margin.
                if rect.left() > frame.left() + 0.5 {
                    rect.min.x += half_gap;
                }
                if rect.right() < frame.right() - 0.5 {
                    rect.max.x -= half_gap;
                }
                if rect.top() > frame.top() + 0.5 {
                    rect.min.y += half_gap;
                }
                if rect.bottom() < frame.bottom() - 0.5 {
                    rect.max.y -= half_gap;
                }
                (rect.width() > 1.0 && rect.height() > 1.0).then_some((placement.pane, rect))
            })
            .collect()
    }

    /// The line segments that separate adjacent panes: one down the middle of
    /// every gap [`resolve`](Self::resolve) opened between two panes, spanning
    /// exactly the edge they share.
    ///
    /// The gap alone doesn't read as a boundary — it shows the frame
    /// background, which is within a shade or two of both panes' own dark
    /// fills, so the lattice and the spectral pane melt into one continuous
    /// field. Geometry only, so it can be tested without a GPU;
    /// [`paint_dividers`](Self::paint_dividers) draws it.
    pub fn dividers(&self, placements: &[(Pane, egui::Rect)]) -> Vec<[egui::Pos2; 2]> {
        // A gap is where `resolve` put one — exactly `gap` points wide (or
        // zero, panes flush). Anything wider is two panes that simply aren't
        // neighbours, and gets no line.
        let facing = |near: f32, far: f32| {
            ((far - near - self.gap).abs() < 1.0).then_some((near + far) * 0.5)
        };
        let mut lines = Vec::new();
        for (i, (_, a)) in placements.iter().enumerate() {
            for (_, b) in &placements[i + 1..] {
                // Side by side: they share a vertical edge over the rows both
                // of them cover.
                let (top, bottom) = (a.top().max(b.top()), a.bottom().min(b.bottom()));
                if bottom > top {
                    if let Some(x) =
                        facing(a.right(), b.left()).or_else(|| facing(b.right(), a.left()))
                    {
                        lines.push([egui::pos2(x, top), egui::pos2(x, bottom)]);
                    }
                }
                // Stacked: a horizontal edge over the columns both cover.
                let (left, right) = (a.left().max(b.left()), a.right().min(b.right()));
                if right > left {
                    if let Some(y) =
                        facing(a.bottom(), b.top()).or_else(|| facing(b.bottom(), a.top()))
                    {
                        lines.push([egui::pos2(left, y), egui::pos2(right, y)]);
                    }
                }
            }
        }
        lines
    }

    /// Draw [`dividers`](Self::dividers). Called by the offline renderer and
    /// by the Video panel's preview, after the panes themselves, so both
    /// compose the identical picture.
    ///
    /// The live dock deliberately does NOT get these: Frameless mode exists so
    /// adjacent panes record as one seamless surface.
    pub fn paint_dividers(&self, painter: &egui::Painter, placements: &[(Pane, egui::Rect)]) {
        let stroke = egui::Stroke::new(DIVIDER_WIDTH, crate::theme::hairline());
        for line in self.dividers(placements) {
            painter.line_segment(line, stroke);
        }
    }
}

/// Divider thickness in points. A hairline: enough to separate two panels,
/// not enough to become a graphic element of its own.
const DIVIDER_WIDTH: f32 = 1.0;

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: egui::Vec2 = egui::vec2(1920.0, 1080.0);

    #[test]
    fn every_advertised_preset_exists_and_resolves() {
        for name in PRESETS {
            let layout =
                Layout::preset(name).unwrap_or_else(|| panic!("{name} is advertised but missing"));
            let resolved = layout.resolve(FRAME);
            assert!(!resolved.is_empty(), "{name} resolved to nothing");
        }
    }

    /// The whole point of fractional rects: one layout, any output size.
    #[test]
    fn a_layout_covers_the_same_proportions_at_any_size() {
        let layout = Layout::preset("side-by-side").unwrap();
        let small = layout.resolve(egui::vec2(1280.0, 720.0));
        let large = layout.resolve(egui::vec2(3840.0, 2160.0));
        assert_eq!(small.len(), large.len());
        for ((_, a), (_, b)) in small.iter().zip(&large) {
            let ratio = |r: &egui::Rect, w: f32| r.width() / w;
            assert!(
                (ratio(a, 1280.0) - ratio(b, 3840.0)).abs() < 0.01,
                "{a:?} vs {b:?} are not the same slice of the frame"
            );
        }
    }

    /// A gap goes *between* panes; the picture still reaches its margin
    /// on the outside, or every render would come out with an uneven
    /// border.
    #[test]
    fn gaps_are_only_inserted_between_panes() {
        let layout = Layout { gap: 20.0, margin: 0.0, ..Layout::preset("side-by-side").unwrap() };
        let resolved = layout.resolve(FRAME);
        let (_, left) = resolved[0];
        let (_, right) = resolved[1];
        assert_eq!(left.left(), 0.0, "the outer edge keeps the margin, not a half-gap");
        assert_eq!(right.right(), 1920.0);
        assert!((right.left() - left.right() - 20.0).abs() < 0.01, "one gap between them");
    }

    #[test]
    fn a_layout_round_trips_through_ron() {
        let layout = Layout::preset("stacked").unwrap();
        let text = ron::to_string(&layout).unwrap();
        let back: Layout = ron::from_str(&text).unwrap();
        assert_eq!(back.resolve(FRAME).len(), layout.resolve(FRAME).len());
        assert_eq!(back.panes[0].rect, layout.panes[0].rect);
    }

    /// The line goes down the middle of the gap and spans the shared edge —
    /// whichever way the two panes are arranged.
    #[test]
    fn adjacent_panes_get_one_divider_centered_in_their_gap() {
        let layout = Layout { gap: 20.0, ..Layout::split(LatticeSide::Left, 0.6) };
        let resolved = layout.resolve(FRAME);
        let lines = layout.dividers(&resolved);
        assert_eq!(lines.len(), 1, "two panes, one boundary");
        let [a, b] = lines[0];
        assert!((a.x - 1920.0 * 0.6).abs() < 0.01, "centered in the gap, not on a pane edge");
        assert_eq!(a.x, b.x, "side by side means a vertical line");
        assert_eq!((a.y, b.y), (0.0, 1080.0), "spanning the whole shared edge");

        let stacked = Layout { gap: 20.0, ..Layout::split(LatticeSide::Top, 0.6) };
        let lines = stacked.dividers(&stacked.resolve(FRAME));
        assert_eq!(lines.len(), 1);
        let [a, b] = lines[0];
        assert!((a.y - 1080.0 * 0.6).abs() < 0.01);
        assert_eq!((a.x, b.x), (0.0, 1920.0), "stacked means a horizontal line");
    }

    /// The whole point of naming the sides: the lattice lands on the one it is
    /// asked for, and takes the same share of the frame on each.
    #[test]
    fn the_lattice_takes_the_side_it_names_and_the_same_share_of_it() {
        for side in LatticeSide::ALL {
            let layout = Layout { gap: 0.0, ..Layout::split(side, 0.6) };
            let resolved = layout.resolve(FRAME);
            let find = |want: Pane| {
                resolved.iter().find(|(p, _)| *p == want).map(|(_, r)| *r).expect("both panes")
            };
            let (lattice, spectral) = (find(Pane::Lattice), find(Pane::Spectral));
            let share = if side.sizes_by_height() {
                lattice.height() / FRAME.y
            } else {
                lattice.width() / FRAME.x
            };
            assert!((share - 0.6).abs() < 0.01, "{side:?}: lattice took {share} of the frame");
            // The named side is the one the lattice reaches and the spectral
            // pane does not — which is what the name is a claim about.
            let (near, far) = match side {
                LatticeSide::Left => (lattice.left(), spectral.left()),
                LatticeSide::Right => (-lattice.right(), -spectral.right()),
                LatticeSide::Top => (lattice.top(), spectral.top()),
                LatticeSide::Bottom => (-lattice.bottom(), -spectral.bottom()),
            };
            assert!(near < far, "{side:?}: the lattice should be the pane on that edge");
        }
    }

    /// Nothing to delineate: one pane has no neighbour, and panes that don't
    /// face each other across a gap aren't neighbours either.
    #[test]
    fn only_neighbours_get_a_divider() {
        for name in ["lattice", "spectral"] {
            let layout = Layout::preset(name).unwrap();
            assert!(layout.dividers(&layout.resolve(FRAME)).is_empty(), "{name}");
        }
        // Two panes with a quarter of the frame between them: a gap that wide
        // is a composition choice, not a seam.
        let apart = Layout {
            background: default_background(),
            margin: 0.0,
            gap: 8.0,
            panes: vec![
                Placement { pane: Pane::Lattice, rect: (0.0, 0.0, 0.25, 1.0) },
                Placement { pane: Pane::Spectral, rect: (0.5, 0.0, 1.0, 1.0) },
            ],
        };
        assert!(apart.dividers(&apart.resolve(FRAME)).is_empty());
    }

    #[test]
    fn a_degenerate_pane_is_dropped_not_drawn() {
        let layout = Layout {
            background: default_background(),
            margin: 0.0,
            gap: 0.0,
            panes: vec![Placement { pane: Pane::Lattice, rect: (0.5, 0.0, 0.5, 1.0) }],
        };
        assert!(layout.resolve(FRAME).is_empty());
    }
}
