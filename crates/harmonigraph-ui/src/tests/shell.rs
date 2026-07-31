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
        panes::Tab::Nodes,
        panes::Tab::Scene,
        panes::Tab::Console,
        panes::Tab::Spectral,
        panes::Tab::Analyzer,
        panes::Tab::Notes,
        panes::Tab::Video,
        panes::Tab::Panel,
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
    for tab in [panes::Tab::Tuning, panes::Tab::Analyzer, panes::Tab::Panel] {
        assert_eq!(viewer.scroll_bars(&tab), [false, true], "{tab:?} cannot scroll vertically");
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
