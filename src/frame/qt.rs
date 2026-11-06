//! Translation of [`Input`](super::Input) fields to the Qt codes that Qt
//! helper processes expect (o3-code's WebEngine and Qt Quick hosts build
//! `QKeyEvent`/`QMouseEvent`/`QWheelEvent` from them). The events stay
//! toolkit-neutral on the wire; the app maps them when it talks to a Qt
//! helper, e.g. with [`to_qt_json`].
//!
//! Qt on macOS swaps Control and Meta (`Qt::ControlModifier` is Command):
//! [`modifiers`] follows Qt's convention for the platform it runs on.

use super::event::{Input, Mods};
use serde_json::{json, Value};

const SHIFT: i32 = 0x0200_0000;
const CONTROL: i32 = 0x0400_0000;
const ALT: i32 = 0x0800_0000;
const META: i32 = 0x1000_0000;

/// `Qt::KeyboardModifiers` for `m`.
pub fn modifiers(m: Mods) -> i32 {
    let (ctrl, meta) = if cfg!(target_os = "macos") { (META, CONTROL) } else { (CONTROL, META) };
    let mut q = 0;
    if m.contains(Mods::SHIFT) {
        q |= SHIFT;
    }
    if m.contains(Mods::CTRL) {
        q |= ctrl;
    }
    if m.contains(Mods::ALT) {
        q |= ALT;
    }
    if m.contains(Mods::META) {
        q |= meta;
    }
    q
}

/// `Qt::Key` for a key name of [`Input::KeyDown`]. Printable keys map to
/// their uppercase code point (`"a"` → `Qt::Key_A`); 0 when unknown.
pub fn key(name: &str) -> i32 {
    let special = match name {
        "Escape" => 0x0100_0000,
        "Tab" => 0x0100_0001,
        "Backspace" => 0x0100_0003,
        "Enter" => 0x0100_0004,
        "Insert" => 0x0100_0006,
        "Delete" => 0x0100_0007,
        "Pause" => 0x0100_0008,
        "PrintScreen" => 0x0100_0009,
        "Home" => 0x0100_0010,
        "End" => 0x0100_0011,
        "ArrowLeft" => 0x0100_0012,
        "ArrowUp" => 0x0100_0013,
        "ArrowRight" => 0x0100_0014,
        "ArrowDown" => 0x0100_0015,
        "PageUp" => 0x0100_0016,
        "PageDown" => 0x0100_0017,
        "Shift" => 0x0100_0020,
        "Control" => 0x0100_0021,
        "Meta" => 0x0100_0022,
        "Alt" => 0x0100_0023,
        "CapsLock" => 0x0100_0024,
        "NumLock" => 0x0100_0025,
        "ScrollLock" => 0x0100_0026,
        "ContextMenu" => 0x0100_0055,
        "Help" => 0x0100_0058,
        _ => 0,
    };
    if special != 0 {
        return special;
    }
    if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<i32>().ok()) {
        if (1..=35).contains(&n) {
            return 0x0100_0030 + n - 1;
        }
    }
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => c.to_uppercase().next().map_or(0, |u| u as i32),
        _ => 0,
    }
}

/// `Qt::MouseButton(s)`: the neutral bits are Qt's (left 1, right 2, middle 4).
pub fn buttons(bits: u8) -> i32 {
    i32::from(bits & 7)
}

/// `QWheelEvent::angleDelta` component for `notches` (positive: down or
/// right): Qt counts 120 per notch, positive away from the user.
pub fn angle_delta(notches: f32) -> i32 {
    (-notches * 120.0).round() as i32
}

/// The event in the JSON shape o3-code's Qt helpers read today
/// (`type`, `x`, `y`, `button`, `buttons`, `modifiers`, `key`, `text`,
/// `ax`, `ay`, `width`, `height`, `value`), with Qt codes. None for events
/// those helpers have no command for.
pub fn to_qt_json(e: &Input) -> Option<Value> {
    Some(match e {
        Input::Move { x, y, buttons: b, mods } => {
            json!({"type": "move", "x": x, "y": y, "button": 0, "buttons": buttons(*b), "modifiers": modifiers(*mods)})
        }
        Input::Press { x, y, button, buttons: b, mods }
        | Input::Release { x, y, button, buttons: b, mods }
        | Input::Double { x, y, button, buttons: b, mods } => json!({
            "type": e.kind(), "x": x, "y": y, "button": *button as i32,
            "buttons": buttons(*b), "modifiers": modifiers(*mods)
        }),
        Input::Wheel { x, y, dx, dy, mods } => json!({
            "type": "wheel", "x": x, "y": y, "ax": angle_delta(*dx), "ay": angle_delta(*dy),
            "px": 0, "py": 0, "modifiers": modifiers(*mods)
        }),
        Input::KeyDown { key: k, text, mods, .. } => json!({"type": "key", "key": key(k), "text": text, "modifiers": modifiers(*mods)}),
        Input::KeyUp { key: k, text, mods } => json!({"type": "keyup", "key": key(k), "text": text, "modifiers": modifiers(*mods)}),
        Input::Resize { width, height, .. } => json!({"type": "resize", "width": width, "height": height}),
        Input::Zoom { value } => json!({"type": "zoom", "value": value}),
        Input::Leave | Input::FocusIn | Input::FocusOut | Input::Copy => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Button;

    #[test]
    fn keys_and_modifiers_map_to_qt_codes() {
        assert_eq!(key("a"), 0x41);
        assert_eq!(key("Z"), 0x5a);
        assert_eq!(key("1"), 0x31);
        assert_eq!(key("Escape"), 0x0100_0000);
        assert_eq!(key("ArrowDown"), 0x0100_0015);
        assert_eq!(key("F1"), 0x0100_0030);
        assert_eq!(key("F12"), 0x0100_003b);
        assert_eq!(key("Nope"), 0);
        assert_eq!(modifiers(Mods::SHIFT), SHIFT);
        assert_eq!(modifiers(Mods::SHIFT | Mods::ALT), SHIFT | ALT);
        let ctrl = if cfg!(target_os = "macos") { META } else { CONTROL };
        assert_eq!(modifiers(Mods::CTRL), ctrl);
        assert_eq!(angle_delta(1.0), -120);
        assert_eq!(buttons(Button::Middle as u8), 4);
    }

    #[test]
    fn qt_json_matches_o3_helper_fields() {
        let w = to_qt_json(&Input::Wheel { x: 1.0, y: 2.0, dx: 0.0, dy: -1.0, mods: Mods::NONE }).unwrap();
        assert_eq!(w["ay"], 120);
        assert_eq!(w["type"], "wheel");
        let k = to_qt_json(&Input::KeyDown { key: "c".into(), text: "c".into(), mods: Mods::CTRL, repeat: false }).unwrap();
        assert_eq!(k["key"], 0x43);
        assert!(to_qt_json(&Input::Copy).is_none());
    }
}
