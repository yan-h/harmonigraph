//! The root shell itself: the tab registry, and the repaint pacing
//! [`root_ui`] asks for.

use crate::*;
use harmonigraph_render::wgpu::TextureFormat;
use super::harness::*;

/// Every tab needs its own id, and the titles cannot supply one: the display
/// pane and its settings are both called "Analyzer" on purpose. egui_dock's
/// default `id()` is the title text, and that id keys the tab BODY's `Ui`
/// (surface + tab id, no node), so a collision made two panes share their
/// body state — scrolling the settings scrolled the analyzer display.
#[test]
fn every_tab_has_its_own_id_even_where_two_share_a_title() {
    use egui_dock::TabViewer;
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let params = RecordingBackend::default();
    let tabs = [
        panes::Tab::Lattice,
        panes::Tab::Tuning,
        panes::Tab::View,
        panes::Tab::Nodes,
        panes::Tab::Scene,
        panes::Tab::Console,
        panes::Tab::Spectral,
        panes::Tab::Analyzer,
        panes::Tab::Notes,
        panes::Tab::Video,
        panes::Tab::System,
    ];
    let mut viewer = panes::Viewer { state: &mut state, params: &params, now: 0.0 };
    let mut title = |mut tab: panes::Tab| viewer.title(&mut tab).text().to_owned();

    // The collision this guards against is real, not hypothetical.
    assert_eq!(
        title(panes::Tab::Spectral),
        title(panes::Tab::Analyzer),
        "the two Analyzer tabs are meant to share a title",
    );

    let ids: Vec<egui::Id> = tabs
        .iter()
        .map(|&tab| {
            let mut tab = tab;
            viewer.id(&mut tab)
        })
        .collect();
    for (i, a) in ids.iter().enumerate() {
        for (j, b) in ids.iter().enumerate().skip(i + 1) {
            assert_ne!(a, b, "{:?} and {:?} share a tab id", tabs[i], tabs[j]);
        }
    }
}

/// The picture panes fill their body exactly, so a scroll area around one can
/// only shift a picture that is meant to sit still.
#[test]
fn the_picture_panes_do_not_scroll() {
    use egui_dock::TabViewer;
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let params = RecordingBackend::default();
    let viewer = panes::Viewer { state: &mut state, params: &params, now: 0.0 };
    for tab in [panes::Tab::Lattice, panes::Tab::Spectral] {
        assert_eq!(viewer.scroll_bars(&tab), [false, false], "{tab:?} is scrollable");
    }
    // Settings panes are lists and must stay reachable in a short column, but
    // VERTICALLY only: a both-axes area gives the body unbounded width, and the
    // panes that fill the space then never report vertical overflow, so the
    // wheel can't scroll them. Horizontal off; vertical on.
    for tab in [panes::Tab::Tuning, panes::Tab::Analyzer, panes::Tab::System] {
        assert_eq!(viewer.scroll_bars(&tab), [false, true], "{tab:?} cannot scroll vertically");
    }
}

/// Every tab in the settings column fits on its tab bar, unclipped, at the
/// window this UI is dialled against.
///
/// The tab bar ALONE, not the column: a settings pane scrolling is a normal
/// thing, so a guard over tab bar and pane content together fires on panes that
/// are meant to scroll and has to come out — which is why this asks the
/// narrower question that stays true (see [`crate::state::SETTINGS_SPLIT`]).
/// Seven tabs is what makes it a live question rather than a margin.
///
/// What overflow actually costs is worth stating exactly, because it is not
/// unreachability: egui_dock SCROLLS a bar that does not fit (`tab_bar_scroll`,
/// and `leaf.scroll`), so every tab stays clickable. The cost is
/// discoverability — a tab you have to drag the bar sideways to find is one a
/// new user never learns is there — which is the whole thing this arrangement
/// is for, so a clipped bar undoes the split it was made by.
///
/// Measured, not derived, and measured in the REAL type. egui_dock lays the bar
/// out itself and would answer a re-derivation of its own sums with whatever it
/// was given; what a user can SEE is whether the glyphs survived the clip rect
/// they were painted under. So this asks the real dock for a real frame — and
/// [`DockHarness`] installs the theme, without which every title here is laid
/// out in egui's 12.5pt fallback rather than the editor's 13.5pt face, and the
/// number below comes out flattering by about 18pt of window.
///
/// The numbers, swept at this height on 2026-08-09: the seven-tab bar needs a
/// window of 1428pt, where the six-tab bar it replaced needed 1274pt. Both are
/// above the editor's own `DEFAULT_SIZE` of 1000pt, so the default window has
/// scrolled its tab bar since well before the split — that is issue #287, and
/// shrinking the default's tab bar is not something the split made true.
#[test]
fn every_settings_tab_fits_on_its_tab_bar() {
    // The window the UI is dialled against, and the one the column widths in
    // `SETTINGS_SPLIT` were measured at. NOT the editor's default size, which
    // is smaller than any tab bar this column has had — see above.
    const WINDOW: egui::Vec2 = egui::vec2(1512.0, 886.0);
    let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
    let mut harness = DockHarness::new();
    harness.screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), WINDOW);
    harness.settle(&mut state);
    let output = harness.frame(&mut state, vec![]);

    let column = [
        panes::Tab::Tuning,
        panes::Tab::View,
        panes::Tab::Nodes,
        panes::Tab::Scene,
        panes::Tab::Analyzer,
        panes::Tab::Video,
        panes::Tab::System,
    ];
    // The settings leaf's own rect, so a title is only counted where this bar
    // drew it. Scoping rather than matching text anywhere on screen is what
    // makes the Analyzer row mean anything: `tab_title` gives the display pane
    // the same name, that pane is a leaf of its own with one tab and room to
    // spare, and an unscoped search would find ITS unclipped copy and pass no
    // matter what the settings column did.
    let path = state.dock.find_tab(&panes::Tab::Tuning).expect("Tuning is docked");
    let leaf = state.dock[path.surface][path.node].rect().expect("the leaf is laid out");
    for tab in column {
        let title = panes::tab_title(&tab);
        // The bar paints the title of every tab in the leaf, not just the
        // selected one, so each is findable by its own text.
        let drawn: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(t)
                    if t.galley.text() == title
                        && t.pos.x >= leaf.left()
                        && t.pos.x <= leaf.right() =>
                {
                    Some((t.pos, t.galley.size(), cs.clip_rect))
                }
                _ => None,
            })
            .collect();
        assert!(!drawn.is_empty(), "the settings tab bar drew no title for {tab:?}");
        let whole = drawn.iter().any(|&(pos, size, clip)| {
            let rect = egui::Rect::from_min_size(pos, size);
            clip.contains_rect(rect)
        });
        assert!(
            whole,
            "{tab:?}'s tab title is clipped on the bar at {WINDOW:?} — the settings \
             column has run out of room for {} tabs. Shorten a name or merge a tab; \
             a clipped tab is one a user cannot read. (drawn: {drawn:?})",
            column.len(),
        );
    }
}

#[test]
fn frame_interval_converts_a_cap_to_a_spacing() {
    assert_eq!(frame_interval(None), None, "uncapped asks for no spacing");
    assert_eq!(
        frame_interval(Some(30.0)),
        Some(std::time::Duration::from_secs_f32(1.0 / 30.0)),
    );
    assert_eq!(
        frame_interval(Some(144.0)),
        Some(std::time::Duration::from_secs_f32(1.0 / 144.0)),
    );
}

#[test]
fn nonsense_caps_read_as_uncapped() {
    // The control cannot produce these, but a hand-edited persist blob can.
    // Uncapped is the safe reading: a zero interval is the uncapped
    // behaviour with extra steps, and a huge one would freeze the UI.
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(frame_interval(Some(bad)), None, "{bad} should read as uncapped");
    }
}
