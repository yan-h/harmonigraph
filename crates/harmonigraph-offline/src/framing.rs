//! Frame lattice to notes: the render-time half of the Video pane's Frame
//! notes, over the whole take instead of what has been drawn.

use harmonigraph_render::wgpu::TextureFormat;
use harmonigraph_take::Take;
use harmonigraph_ui::framing::Heard;
use harmonigraph_ui::params::ParamKey;
use harmonigraph_ui::{AppearanceDocument, PictureState};

use crate::replay::Replay;

/// What framing a take came to.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Framed; whether everything fits, as
    /// [`AppearanceDocument::frame_lattice`] says.
    Framed { fits: bool },
    /// The take lights no node between `start` and `end`, so nothing changed.
    Nothing,
}

/// Replay the take's notes from `start` to `end`, drawing nothing, and frame
/// `appearance` on every node they light, under the tuning each was played
/// under. The camera is the one the video opens on, so its orbit is the
/// take's; framing replaces its pan and zoom, and the split.
///
/// Every event time is visited, so each voice is heard at least once with its
/// pitch as it starts and as every bend leaves it.
pub fn frame_take(
    take: &Take,
    appearance: &mut AppearanceDocument,
    start: f64,
    end: f64,
) -> Outcome {
    let mut replay = Replay::new(take.clone());
    let mut state = PictureState::new(TextureFormat::Rgba8Unorm);
    state.install_appearance(appearance.clone());
    let mut times: Vec<f64> = take
        .events
        .iter()
        .map(|event| event.time())
        .chain(take.params.iter().map(|param| param.t))
        .chain(take.configurations.iter().map(|configuration| configuration.t))
        .filter(|&t| t > start && t <= end)
        .collect();
    times.sort_unstable_by(f64::total_cmp);
    times.dedup();
    let mut heard = Heard::default();
    let mut opening = None;
    for now in std::iter::once(start).chain(times) {
        replay.advance_to(&mut state.runtime, now);
        harmonigraph_ui::begin_frame(&mut state, &replay.params, now);
        let voices = state.runtime.tracker.voices().map(|voice| voice.pitch_class);
        heard.hear(state.runtime.tuning, voices);
        // The camera the take opens on, and the window center that goes with
        // its pan; nothing else the first step may have touched.
        let view = &state.appearance.view;
        opening.get_or_insert((state.appearance.camera, view.center_fives, view.center_threes));
    }
    let (camera, fives, threes) = opening.expect("the start is always visited");
    let mut framed = appearance.clone();
    framed.camera = camera;
    (framed.view.center_fives, framed.view.center_threes) = (fives, threes);
    let positions = heard.positions(&framed);
    match framed.frame_lattice(&positions) {
        Some(fits) => {
            *appearance = framed;
            Outcome::Framed { fits }
        }
        None => Outcome::Nothing,
    }
}

/// Whether the take moves its camera after `start`, which a framed render does
/// not replay.
pub fn camera_moves(take: &Take, start: f64) -> bool {
    take.params.iter().any(|param| {
        param.t > start
            && ParamKey::from_id(&param.id).is_some_and(|key| ParamKey::CAMERA.contains(&key))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_take::{CanonicalRecord, Header, NoteKind, NoteRecord, ParamRecord};

    fn note(t: f64, note: u8, kind: NoteKind) -> CanonicalRecord {
        CanonicalRecord::Note(NoteRecord { source: 0, t, channel: 0, note, kind })
    }

    /// A take in the fresh 12-TET tuning: C, then E, then D, across five
    /// seconds, with a camera that pans away halfway. Each note lights a node
    /// on every copy of its pitch class; the copies nearest the opening
    /// camera, at the origin, are C there, E a third up and D two fifths up.
    fn take() -> Take {
        let on = NoteKind::On { velocity: 0.8 };
        let param = |t, key: ParamKey, value| ParamRecord { t, id: key.id().to_string(), value };
        let params = vec![
            param(0.0, ParamKey::CameraPanX, 0.0),
            param(0.0, ParamKey::CameraPanY, 0.0),
            param(2.5, ParamKey::CameraPanX, 9.0),
        ];
        Take {
            header: Header::default(),
            events: vec![
                note(0.5, 60, on),
                note(1.0, 60, NoteKind::Off),
                note(2.0, 64, on),
                note(3.0, 64, NoteKind::Off),
                note(4.0, 62, on),
                note(4.5, 62, NoteKind::Off),
            ],
            params,
            configurations: Vec::new(),
            truncated: false,
            incomplete: None,
        }
    }

    /// Every note of the take is framed, not just the ones sounding at the
    /// start, and the camera then stays put through the recorded pan.
    #[test]
    fn a_framed_take_holds_one_camera_on_all_its_notes() {
        let take = take();
        let mut look = AppearanceDocument::default();
        let outcome = frame_take(&take, &mut look, 0.0, 10.0);
        assert_eq!(outcome, Outcome::Framed { fits: true });
        // The middle of the three is where the camera points.
        let [_, _, _, fives, threes] = look.camera_movement();
        assert!((fives - 0.5).abs() < 1e-3 && (threes - 1.0).abs() < 1e-3, "{fives} {threes}");
        assert!(camera_moves(&take, 0.0));

        let mut replay = Replay::new(take);
        replay.params.hold_camera();
        let mut state = PictureState::new(TextureFormat::Rgba8Unorm);
        state.install_appearance(look.clone());
        for now in [0.0, 3.0, 6.0] {
            replay.advance_to(&mut state.runtime, now);
            harmonigraph_ui::begin_frame(&mut state, &replay.params, now);
            assert_eq!(state.appearance.camera_movement(), look.camera_movement(), "at {now}");
        }
    }

    /// Notes past the render's end are not framed, and a stretch with no notes
    /// leaves the look alone.
    #[test]
    fn only_the_rendered_stretch_is_framed() {
        let take = take();
        let mut early = AppearanceDocument::default();
        frame_take(&take, &mut early, 0.0, 1.5);
        let mut whole = AppearanceDocument::default();
        frame_take(&take, &mut whole, 0.0, 10.0);
        assert!(early.camera.distance < whole.camera.distance, "one node frames tighter");

        let mut look = AppearanceDocument::default();
        let before = look.serialize();
        assert_eq!(frame_take(&take, &mut look, 6.0, 10.0), Outcome::Nothing);
        assert_eq!(look.serialize(), before);
    }
}
