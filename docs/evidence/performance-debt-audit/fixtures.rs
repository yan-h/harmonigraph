use harmonigraph_scene::{Camera, ViewConfig};

fn main() {
    let mut tracker = harmonigraph_core::NoteTracker::new();
    let source = harmonigraph_core::SourceId::DIRECT;
    tracker.handle_event(harmonigraph_core::NoteEvent::on(1.0, source, 0, 60, 0.8));
    println!("held after On at 1.0: {}", tracker.held_count());
    tracker.handle_event(harmonigraph_core::NoteEvent::off(1.25, source, 0, 60));
    println!(
        "held after immediately delivering Off dated 1.25: {}; live roll notes={}",
        tracker.held_count(),
        tracker.roll().notes().filter(|n| n.is_live()).count()
    );
    let camera = Camera::default();
    let base = ViewConfig::default();
    for (label, view) in [
        ("default", base.clone()),
        (
            "sevens",
            ViewConfig {
                min_sevens: -1,
                max_sevens: 1,
                ..base.clone()
            },
        ),
        (
            "3075 label",
            ViewConfig {
                extent_threes: 20,
                extent_fives: 12,
                min_sevens: -1,
                max_sevens: 1,
                ..base.clone()
            },
        ),
    ] {
        for aspect in [0.51, 0.821] {
            let drawn = view.scrolled(&camera, aspect);
            let reach = view.reach();
            println!(
                "{label} aspect {aspect}: drawn={} {:?}; reach={}",
                drawn.count(),
                drawn,
                reach.count()
            );
        }
    }
}
