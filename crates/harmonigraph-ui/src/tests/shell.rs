//! The root shell itself: the tab registry, and the repaint pacing
//! [`root_ui`] asks for.

use crate::*;
use super::harness::*;

/// Every tab needs an id of its own, and the title is not allowed to be its
/// source: egui_dock's default `id()` is the title text, and that id keys the
/// tab BODY's `Ui` (surface + tab id, no node), so two tabs sharing a title
/// would share their body state — scrolling one pane scrolls the other.
/// Variant-keyed ids are what leave a name free to be repeated, and the dock
/// still trades on that freedom across surfaces: the Spectral pane wears
/// "Analyzer", the same word as the Display section that holds its settings,
/// because the display and its knobs are one feature.
#[test]
fn every_tab_has_its_own_id_whatever_its_title_says() {
    use egui_dock::TabViewer;
    let mut state = fresh();
    let params = RecordingBackend::default();
    let tabs = [
        panes::Tab::Lattice,
        panes::Tab::Tuning,
        panes::Tab::Display,
        panes::Tab::Console,
        panes::Tab::Spectral,
        panes::Tab::Notes,
        panes::Tab::Video,
        panes::Tab::System,
    ];
    let mut viewer = panes::Viewer { state: &mut state, params: &params, now: 0.0 };

    // The sharing the variant-keyed id keeps safe is real, not hypothetical.
    assert_eq!(
        panes::tab_title(&panes::Tab::Spectral),
        panes::display::Section::Analyzer.title(),
        "the Spectral pane and the Display section holding its settings are \
         meant to share the Analyzer name",
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
    let mut state = fresh();
    let params = RecordingBackend::default();
    let viewer = panes::Viewer { state: &mut state, params: &params, now: 0.0 };
    for tab in [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Spiral] {
        assert_eq!(viewer.scroll_bars(&tab), [false, false], "{tab:?} is scrollable");
    }
    // Settings panes are lists and must stay reachable in a short column, but
    // VERTICALLY only: a both-axes area gives the body unbounded width, and the
    // panes that fill the space then never report vertical overflow, so the
    // wheel can't scroll them. Horizontal off; vertical on.
    for tab in [panes::Tab::Tuning, panes::Tab::Display, panes::Tab::System] {
        assert_eq!(viewer.scroll_bars(&tab), [false, true], "{tab:?} cannot scroll vertically");
    }
}

/// Every tab in the settings column fits on its tab bar, unclipped, at the
/// editor's default window as well as the window this UI is dialled against.
///
/// The tab bar ALONE, not the column: a settings pane scrolling is a normal
/// thing, so a guard over tab bar and pane content together fires on panes that
/// are meant to scroll and has to come out — which is why this asks the
/// narrower question that stays true (see [`crate::state::SETTINGS_SPLIT`]).
///
/// What overflow actually costs is worth stating exactly, because it is not
/// unreachability: egui_dock SCROLLS a bar that does not fit (`tab_bar_scroll`,
/// and `leaf.scroll`), so every tab stays clickable. The cost is
/// discoverability — a tab you have to drag the bar sideways to find is one a
/// new user never learns is there — which is the whole thing this arrangement
/// is for, so a clipped bar undoes the naming and merging it was made by.
///
/// Measured, not derived, and measured in the REAL type. egui_dock lays the bar
/// out itself and would answer a re-derivation of its own sums with whatever it
/// was given; what a user can SEE is whether the glyphs survived the clip rect
/// they were painted under. So this asks the real dock for a real frame — and
/// [`DockHarness`] installs the theme, without which every title here is laid
/// out in egui's 12.5pt fallback rather than the editor's 13.5pt face, and the
/// numbers below come out flattering by about 18pt of window.
///
/// The default-window row is what the Display merge bought, and the margin it
/// bought is worth knowing when adding a tab: a tab per settings pane wants
/// 1428pt of window at seven tabs where this bar's four have about 270pt of
/// column to share at 1000pt — so the room for a NEW tab is real but shallow,
/// and a new settings surface should be a Display section first (#287).
#[test]
fn every_settings_tab_fits_on_its_tab_bar() {
    // Two windows: the one this UI is dialled against (and the one the column
    // widths in `SETTINGS_SPLIT` were measured at), and the editor's own
    // `DEFAULT_SIZE` — restated here because `editor.rs` is a crate this one
    // does not see. The default is the window every fresh instance opens at,
    // so a bar that overflows there is one a new user never sees whole; that
    // is issue #287, and this row of the sweep is what holds the fix.
    for window in [egui::vec2(1512.0, 886.0), egui::vec2(1000.0, 700.0)] {
    let mut state = fresh();
    let mut harness = DockHarness::new();
    harness.screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), window);
    harness.settle(&mut state);
    let output = harness.frame(&mut state, vec![]);

    let column = [
        panes::Tab::Tuning,
        panes::Tab::Display,
        panes::Tab::Video,
        panes::Tab::System,
    ];
    // The settings leaf's own rect, so a title is only counted where this bar
    // drew it. Scoping rather than matching text anywhere on screen is what
    // makes the Analyzer row mean anything: `tab_title` gives the display pane
    // the same name, that pane is a leaf of its own with one tab and room to
    // spare, and an unscoped search would find ITS unclipped copy and pass no
    // matter what the settings column did.
    let path = state.workspace.dock.find_tab(&panes::Tab::Tuning).expect("Tuning is docked");
    let leaf = state.workspace.dock[path.surface][path.node].rect().expect("the leaf is laid out");
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
            "{tab:?}'s tab title is clipped on the bar at {window:?} — the settings \
             column has run out of room for {} tabs. Shorten a name or merge a tab; \
             a clipped tab is one a user cannot read. (drawn: {drawn:?})",
            column.len(),
        );
    }
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
