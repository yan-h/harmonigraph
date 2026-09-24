pub(crate) fn translate_mouse_button(button: baseview::MouseButton) -> Option<egui::PointerButton> {
    match button {
        baseview::MouseButton::Left => Some(egui::PointerButton::Primary),
        baseview::MouseButton::Right => Some(egui::PointerButton::Secondary),
        baseview::MouseButton::Middle => Some(egui::PointerButton::Middle),
        _ => None,
    }
}

/// egui's modifiers from the set a baseview event carries, every field of them.
///
/// An event's set is the OS's own reading of what is held at that event, so each
/// one corrects whatever came before it. Toggling a field per modifier key, as
/// this crate used to, only hears about a key while the view has the keyboard: a
/// Cmd or Ctrl let go after Cmd-Tab, Cmd-Space, Ctrl-arrow or a Cmd-click on the
/// host has its key-up delivered somewhere else, and with mouse events writing
/// only Alt, Shift and `command`, that field stayed held. egui reads Cmd OR Ctrl
/// on a wheel as a zoom, so the one stale field turned every wheel into a zoom:
/// every `ScrollArea` stopped, while a picture reading `zoom_delta` went on
/// responding.
///
/// On macOS `command` is Cmd and Ctrl is only `ctrl`; the mouse path used to
/// report Ctrl as `command` there. Upstream egui-baseview 0.7 has this shape.
pub(crate) fn translate_modifiers(modifiers: keyboard_types::Modifiers) -> egui::Modifiers {
    use keyboard_types::Modifiers as M;
    let ctrl = modifiers.contains(M::CONTROL);
    let meta = modifiers.contains(M::META);
    let mac = cfg!(target_os = "macos");
    egui::Modifiers {
        alt: modifiers.contains(M::ALT),
        ctrl,
        shift: modifiers.contains(M::SHIFT),
        mac_cmd: mac && meta,
        command: if mac { meta } else { ctrl },
    }
}

pub(crate) fn translate_virtual_key(key: &keyboard_types::Key) -> Option<egui::Key> {
    use egui::Key;
    use keyboard_types::Key as K;

    Some(match key {
        K::ArrowDown => Key::ArrowDown,
        K::ArrowLeft => Key::ArrowLeft,
        K::ArrowRight => Key::ArrowRight,
        K::ArrowUp => Key::ArrowUp,

        K::Escape => Key::Escape,
        K::Tab => Key::Tab,
        K::Backspace => Key::Backspace,
        K::Enter => Key::Enter,

        K::Insert => Key::Insert,
        K::Delete => Key::Delete,
        K::Home => Key::Home,
        K::End => Key::End,
        K::PageUp => Key::PageUp,
        K::PageDown => Key::PageDown,

        K::Character(s) => match s.chars().next()? {
            ' ' => Key::Space,
            '0' => Key::Num0,
            '1' => Key::Num1,
            '2' => Key::Num2,
            '3' => Key::Num3,
            '4' => Key::Num4,
            '5' => Key::Num5,
            '6' => Key::Num6,
            '7' => Key::Num7,
            '8' => Key::Num8,
            '9' => Key::Num9,
            'a' => Key::A,
            'b' => Key::B,
            'c' => Key::C,
            'd' => Key::D,
            'e' => Key::E,
            'f' => Key::F,
            'g' => Key::G,
            'h' => Key::H,
            'i' => Key::I,
            'j' => Key::J,
            'k' => Key::K,
            'l' => Key::L,
            'm' => Key::M,
            'n' => Key::N,
            'o' => Key::O,
            'p' => Key::P,
            'q' => Key::Q,
            'r' => Key::R,
            's' => Key::S,
            't' => Key::T,
            'u' => Key::U,
            'v' => Key::V,
            'w' => Key::W,
            'x' => Key::X,
            'y' => Key::Y,
            'z' => Key::Z,
            _ => {
                return None;
            }
        },
        _ => {
            return None;
        }
    })
}

pub(crate) fn translate_cursor_icon(cursor: egui::CursorIcon) -> baseview::MouseCursor {
    match cursor {
        egui::CursorIcon::Default => baseview::MouseCursor::Default,
        egui::CursorIcon::None => baseview::MouseCursor::Hidden,
        egui::CursorIcon::ContextMenu => baseview::MouseCursor::Hand,
        egui::CursorIcon::Help => baseview::MouseCursor::Help,
        egui::CursorIcon::PointingHand => baseview::MouseCursor::Hand,
        egui::CursorIcon::Progress => baseview::MouseCursor::PtrWorking,
        egui::CursorIcon::Wait => baseview::MouseCursor::Working,
        egui::CursorIcon::Cell => baseview::MouseCursor::Cell,
        egui::CursorIcon::Crosshair => baseview::MouseCursor::Crosshair,
        egui::CursorIcon::Text => baseview::MouseCursor::Text,
        egui::CursorIcon::VerticalText => baseview::MouseCursor::VerticalText,
        egui::CursorIcon::Alias => baseview::MouseCursor::Alias,
        egui::CursorIcon::Copy => baseview::MouseCursor::Copy,
        egui::CursorIcon::Move => baseview::MouseCursor::Move,
        egui::CursorIcon::NoDrop => baseview::MouseCursor::NotAllowed,
        egui::CursorIcon::NotAllowed => baseview::MouseCursor::NotAllowed,
        egui::CursorIcon::Grab => baseview::MouseCursor::Hand,
        egui::CursorIcon::Grabbing => baseview::MouseCursor::HandGrabbing,
        egui::CursorIcon::AllScroll => baseview::MouseCursor::AllScroll,
        egui::CursorIcon::ResizeHorizontal => baseview::MouseCursor::EwResize,
        egui::CursorIcon::ResizeNeSw => baseview::MouseCursor::NeswResize,
        egui::CursorIcon::ResizeNwSe => baseview::MouseCursor::NwseResize,
        egui::CursorIcon::ResizeVertical => baseview::MouseCursor::NsResize,
        egui::CursorIcon::ResizeEast => baseview::MouseCursor::EResize,
        egui::CursorIcon::ResizeSouthEast => baseview::MouseCursor::SeResize,
        egui::CursorIcon::ResizeSouth => baseview::MouseCursor::SResize,
        egui::CursorIcon::ResizeSouthWest => baseview::MouseCursor::SwResize,
        egui::CursorIcon::ResizeWest => baseview::MouseCursor::WResize,
        egui::CursorIcon::ResizeNorthWest => baseview::MouseCursor::NwResize,
        egui::CursorIcon::ResizeNorth => baseview::MouseCursor::NResize,
        egui::CursorIcon::ResizeNorthEast => baseview::MouseCursor::NeResize,
        egui::CursorIcon::ResizeColumn => baseview::MouseCursor::ColResize,
        egui::CursorIcon::ResizeRow => baseview::MouseCursor::RowResize,
        egui::CursorIcon::ZoomIn => baseview::MouseCursor::ZoomIn,
        egui::CursorIcon::ZoomOut => baseview::MouseCursor::ZoomOut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyboard_types::Modifiers as M;

    /// An event with nothing held puts EVERY modifier up, including the Cmd and
    /// Ctrl that mouse events used to leave alone — the two a key-up delivered
    /// to another window left held, and the two egui reads as a zoom on the
    /// wheel. And Ctrl alone is not a Cmd on macOS, where the mouse path used to
    /// report it as one.
    #[test]
    fn an_event_reports_exactly_the_modifiers_it_carries() {
        assert_eq!(translate_modifiers(M::empty()), egui::Modifiers::NONE);
        assert!(!translate_modifiers(M::empty()).matches_any(egui::Modifiers::COMMAND));

        let mac = cfg!(target_os = "macos");
        let ctrl = translate_modifiers(M::CONTROL);
        assert!(ctrl.ctrl);
        assert_eq!(ctrl.command, !mac);

        let meta = translate_modifiers(M::META);
        assert!(!meta.ctrl);
        assert_eq!((meta.mac_cmd, meta.command), (mac, mac));
    }
}
