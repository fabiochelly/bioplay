//! Touches winit → noms de touches mpv, pour `keypress` : mpv applique alors exactement les
//! raccourcis de `input.conf` (et ses raccourcis par défaut), comme dans le mpv habituel.

use egui_winit::winit::event::MouseButton;
use egui_winit::winit::keyboard::{Key, KeyLocation, ModifiersState, NamedKey};

pub fn name(key: &Key, location: KeyLocation, mods: ModifiersState) -> Option<String> {
    let numpad = location == KeyLocation::Numpad;
    let (base, is_char) = match key {
        Key::Character(c) => {
            let s = c.as_str();
            let n = if numpad {
                match s {
                    "+" => "KP_ADD".into(),
                    "-" => "KP_SUBTRACT".into(),
                    "*" => "KP_MULTIPLY".into(),
                    "/" => "KP_DIVIDE".into(),
                    "." | "," => "KP_DEC".into(),
                    d if d.len() == 1 && d.chars().all(|c| c.is_ascii_digit()) => format!("KP{d}"),
                    _ => s.into(),
                }
            } else {
                match s {
                    " " => "SPACE".into(),
                    "#" => "SHARP".into(),
                    _ => s.into(),
                }
            };
            (n, !numpad && s != " ")
        }
        Key::Named(n) => (
            match n {
                NamedKey::Space => "SPACE",
                NamedKey::Enter if numpad => "KP_ENTER",
                NamedKey::Enter => "ENTER",
                NamedKey::Escape => "ESC",
                NamedKey::Tab => "TAB",
                NamedKey::Backspace => "BS",
                NamedKey::Delete => "DEL",
                NamedKey::Insert => "INS",
                NamedKey::Home => "HOME",
                NamedKey::End => "END",
                NamedKey::PageUp => "PGUP",
                NamedKey::PageDown => "PGDWN",
                NamedKey::ArrowLeft => "LEFT",
                NamedKey::ArrowRight => "RIGHT",
                NamedKey::ArrowUp => "UP",
                NamedKey::ArrowDown => "DOWN",
                NamedKey::F1 => "F1", NamedKey::F2 => "F2", NamedKey::F3 => "F3", NamedKey::F4 => "F4",
                NamedKey::F5 => "F5", NamedKey::F6 => "F6", NamedKey::F7 => "F7", NamedKey::F8 => "F8",
                NamedKey::F9 => "F9", NamedKey::F10 => "F10", NamedKey::F11 => "F11", NamedKey::F12 => "F12",
                NamedKey::AudioVolumeUp => "VOLUME_UP",
                NamedKey::AudioVolumeDown => "VOLUME_DOWN",
                NamedKey::AudioVolumeMute => "MUTE",
                NamedKey::MediaPlayPause => "PLAYPAUSE",
                NamedKey::MediaPlay => "PLAY",
                NamedKey::MediaPause => "PAUSE",
                NamedKey::MediaStop => "STOP",
                NamedKey::MediaTrackNext => "NEXT",
                NamedKey::MediaTrackPrevious => "PREV",
                _ => return None,
            }
            .to_string(),
            false,
        ),
        _ => return None,
    };
    let mut out = String::new();
    if mods.control_key() {
        out += "Ctrl+";
    }
    if mods.alt_key() {
        out += "Alt+";
    }
    if mods.super_key() {
        out += "Meta+";
    }
    // Pour un caractère, Maj est déjà dans le caractère lui-même (« A », « + »).
    if mods.shift_key() && !is_char {
        out += "Shift+";
    }
    Some(out + &base)
}

pub fn mouse(button: MouseButton, double: bool) -> Option<&'static str> {
    Some(match (button, double) {
        (MouseButton::Left, false) => "MBTN_LEFT",
        (MouseButton::Left, true) => "MBTN_LEFT_DBL",
        (MouseButton::Right, false) => "MBTN_RIGHT",
        (MouseButton::Right, true) => "MBTN_RIGHT_DBL",
        (MouseButton::Middle, _) => "MBTN_MID",
        (MouseButton::Back, _) => "MBTN_BACK",
        (MouseButton::Forward, _) => "MBTN_FORWARD",
        _ => return None,
    })
}
