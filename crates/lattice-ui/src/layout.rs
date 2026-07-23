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

use crate::Pane;

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
pub const PRESETS: [&str; 4] = ["side-by-side", "stacked", "lattice", "spectral"];

impl Layout {
    /// A preset by name, or `None` if it isn't one.
    pub fn preset(name: &str) -> Option<Layout> {
        let full = |pane| Placement { pane, rect: (0.0, 0.0, 1.0, 1.0) };
        let panes = match name {
            // The lattice leads and the roll runs upright beside it —
            // the Spectral pane's Auto orientation turns it upright by
            // itself at this aspect.
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
            format!("layout {spec:?} is not a preset ({}) and could not be read: {e}", PRESETS.join(", "))
        })?;
        ron::from_str(&text).map_err(|e| format!("layout {spec:?}: {e}"))
    }

    /// A two-pane composition of the lattice and the spectral pane, split at
    /// `fraction` — the lattice's share. Side by side unless `stacked`, then
    /// lattice over spectral. Shared by the Video panel's live preview and
    /// the offline renderer, so both compose the identical frame.
    pub fn split(stacked: bool, fraction: f32) -> Layout {
        let f = fraction.clamp(0.05, 0.95);
        let panes = if stacked {
            vec![
                Placement { pane: Pane::Lattice, rect: (0.0, 0.0, 1.0, f) },
                Placement { pane: Pane::Spectral, rect: (0.0, f, 1.0, 1.0) },
            ]
        } else {
            vec![
                Placement { pane: Pane::Lattice, rect: (0.0, 0.0, f, 1.0) },
                Placement { pane: Pane::Spectral, rect: (f, 0.0, 1.0, 1.0) },
            ]
        };
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
}

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
