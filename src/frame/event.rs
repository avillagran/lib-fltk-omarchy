//! Toolkit-neutral input events (view → producer) and messages
//! (producer → view), with their JSON-lines encoding.
//!
//! | `type` | fields |
//! |---|---|
//! | `move` | `x`, `y` (frame pixels), `buttons`, `mods` |
//! | `press`, `release`, `double` | `x`, `y`, `button`, `buttons`, `mods` |
//! | `leave` | |
//! | `wheel` | `x`, `y`, `dx`, `dy` (notches; positive `dy` scrolls down, positive `dx` right), `mods` |
//! | `key`, `keyup` | `key` (W3C-style name: `"a"`, `"Enter"`, `"ArrowLeft"`, `"F5"`), `text` (typed text, may be empty), `mods`, `repeat` (`key` only) |
//! | `focus-in`, `focus-out` | |
//! | `resize` | `width`, `height` (logical pixels the view has), `scale` (device pixels per logical pixel) |
//! | `zoom` | `value` (0.5–2) |
//! | `copy` | (the user asked to copy the selection: answer with a `clipboard` message) |
//!
//! `button` and `buttons` use the bits left 1, right 2, middle 4; `mods`
//! uses Shift 1, Ctrl 2, Alt 4, Meta (Super, Cmd) 8. Messages back:
//! `{"type":"content-size","width":..,"height":..}`,
//! `{"type":"clipboard","text":".."}`, anything else is passed to the app
//! as [`Message::Other`]. [`super::qt`] translates to Qt codes.

use serde_json::{json, Map, Value};

/// Modifier bits held during an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Mods(pub u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const SHIFT: Mods = Mods(1);
    pub const CTRL: Mods = Mods(2);
    pub const ALT: Mods = Mods(4);
    /// Super/Windows key; Command on macOS.
    pub const META: Mods = Mods(8);

    pub fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0 && other.0 != 0
    }
}

impl std::ops::BitOr for Mods {
    type Output = Mods;
    fn bitor(self, o: Mods) -> Mods {
        Mods(self.0 | o.0)
    }
}

/// A mouse button (bits match [`Input`]'s `buttons` mask).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Left = 1,
    Right = 2,
    Middle = 4,
}

impl Button {
    fn from_bits(b: u64) -> Option<Button> {
        match b {
            1 => Some(Button::Left),
            2 => Some(Button::Right),
            4 => Some(Button::Middle),
            _ => None,
        }
    }
}

/// An input event for the producer. Pointer coordinates are in frame
/// pixels (the frame the view showed when the event happened).
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Move { x: f32, y: f32, buttons: u8, mods: Mods },
    Press { x: f32, y: f32, button: Button, buttons: u8, mods: Mods },
    Release { x: f32, y: f32, button: Button, buttons: u8, mods: Mods },
    /// The second press of a double click (sent after its `Press`).
    Double { x: f32, y: f32, button: Button, buttons: u8, mods: Mods },
    /// The pointer left the view.
    Leave,
    Wheel { x: f32, y: f32, dx: f32, dy: f32, mods: Mods },
    KeyDown { key: String, text: String, mods: Mods, repeat: bool },
    KeyUp { key: String, text: String, mods: Mods },
    FocusIn,
    FocusOut,
    /// The view's size in logical pixels and its device scale: frames of
    /// `width * scale × height * scale` pixels show 1:1.
    Resize { width: u32, height: u32, scale: f32 },
    Zoom { value: f32 },
    /// Copy the selection (answer with [`Message::Clipboard`]).
    Copy,
}

fn num(v: f32) -> Value {
    // Whole numbers stay integers in the JSON ("x":12, not 12.0).
    if v.fract() == 0.0 && v.abs() < 1e9 {
        json!(v as i64)
    } else {
        json!((v as f64 * 100.0).round() / 100.0)
    }
}

impl Input {
    /// The JSON `type` of this event.
    pub fn kind(&self) -> &'static str {
        match self {
            Input::Move { .. } => "move",
            Input::Press { .. } => "press",
            Input::Release { .. } => "release",
            Input::Double { .. } => "double",
            Input::Leave => "leave",
            Input::Wheel { .. } => "wheel",
            Input::KeyDown { .. } => "key",
            Input::KeyUp { .. } => "keyup",
            Input::FocusIn => "focus-in",
            Input::FocusOut => "focus-out",
            Input::Resize { .. } => "resize",
            Input::Zoom { .. } => "zoom",
            Input::Copy => "copy",
        }
    }

    /// As a JSON object (see the module table).
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("type".into(), json!(self.kind()));
        let mut put = |k: &str, v: Value| {
            m.insert(k.into(), v);
        };
        match self {
            Input::Move { x, y, buttons, mods } => {
                put("x", num(*x));
                put("y", num(*y));
                put("buttons", json!(buttons));
                put("mods", json!(mods.0));
            }
            Input::Press { x, y, button, buttons, mods }
            | Input::Release { x, y, button, buttons, mods }
            | Input::Double { x, y, button, buttons, mods } => {
                put("x", num(*x));
                put("y", num(*y));
                put("button", json!(*button as u8));
                put("buttons", json!(buttons));
                put("mods", json!(mods.0));
            }
            Input::Wheel { x, y, dx, dy, mods } => {
                put("x", num(*x));
                put("y", num(*y));
                put("dx", num(*dx));
                put("dy", num(*dy));
                put("mods", json!(mods.0));
            }
            Input::KeyDown { key, text, mods, repeat } => {
                put("key", json!(key));
                put("text", json!(text));
                put("mods", json!(mods.0));
                put("repeat", json!(repeat));
            }
            Input::KeyUp { key, text, mods } => {
                put("key", json!(key));
                put("text", json!(text));
                put("mods", json!(mods.0));
            }
            Input::Resize { width, height, scale } => {
                put("width", json!(width));
                put("height", json!(height));
                put("scale", num(*scale));
            }
            Input::Zoom { value } => put("value", num(*value)),
            Input::Leave | Input::FocusIn | Input::FocusOut | Input::Copy => {}
        }
        Value::Object(m)
    }

    /// One JSON line, newline included.
    pub fn to_line(&self) -> String {
        let mut s = self.to_json().to_string();
        s.push('\n');
        s
    }

    /// Parses an event written by [`Input::to_json`] (unknown fields are
    /// ignored; None for an unknown type or missing fields).
    pub fn from_json(v: &Value) -> Option<Input> {
        let f = |k: &str| v.get(k).and_then(Value::as_f64).map(|x| x as f32);
        let u = |k: &str| v.get(k).and_then(Value::as_u64);
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let mods = Mods(u("mods").unwrap_or(0) as u8);
        let buttons = u("buttons").unwrap_or(0) as u8;
        let xy = || Some((f("x")?, f("y")?));
        Some(match v.get("type")?.as_str()? {
            "move" => {
                let (x, y) = xy()?;
                Input::Move { x, y, buttons, mods }
            }
            t @ ("press" | "release" | "double") => {
                let (x, y) = xy()?;
                let button = Button::from_bits(u("button")?)?;
                match t {
                    "press" => Input::Press { x, y, button, buttons, mods },
                    "release" => Input::Release { x, y, button, buttons, mods },
                    _ => Input::Double { x, y, button, buttons, mods },
                }
            }
            "leave" => Input::Leave,
            "wheel" => {
                let (x, y) = xy()?;
                Input::Wheel { x, y, dx: f("dx").unwrap_or(0.0), dy: f("dy").unwrap_or(0.0), mods }
            }
            "key" => Input::KeyDown {
                key: s("key")?,
                text: s("text").unwrap_or_default(),
                mods,
                repeat: v.get("repeat").and_then(Value::as_bool).unwrap_or(false),
            },
            "keyup" => Input::KeyUp { key: s("key")?, text: s("text").unwrap_or_default(), mods },
            "focus-in" => Input::FocusIn,
            "focus-out" => Input::FocusOut,
            "resize" => Input::Resize { width: u("width")? as u32, height: u("height")? as u32, scale: f("scale").unwrap_or(1.0) },
            "zoom" => Input::Zoom { value: f("value")? },
            "copy" => Input::Copy,
            _ => return None,
        })
    }

    /// Parses one JSON line.
    pub fn from_line(line: &str) -> Option<Input> {
        Input::from_json(&serde_json::from_str(line.trim()).ok()?)
    }
}

/// A message from the producer.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// The natural size of the content in logical pixels (a page's
    /// scroll height): an inline view can size itself to it.
    ContentSize { width: u32, height: u32 },
    /// Text for the clipboard, in answer to [`Input::Copy`]. The view sets
    /// the clipboard only within a second of a copy request.
    Clipboard(String),
    /// Anything else (the app's own protocol).
    Other(Value),
}

impl Message {
    pub fn to_json(&self) -> Value {
        match self {
            Message::ContentSize { width, height } => json!({"type": "content-size", "width": width, "height": height}),
            Message::Clipboard(t) => json!({"type": "clipboard", "text": t}),
            Message::Other(v) => v.clone(),
        }
    }

    pub fn to_line(&self) -> String {
        let mut s = self.to_json().to_string();
        s.push('\n');
        s
    }

    pub fn from_json(v: Value) -> Message {
        let u = |k: &str| v.get(k).and_then(Value::as_u64).map(|x| x.min(u32::MAX as u64) as u32);
        match v.get("type").and_then(Value::as_str) {
            Some("content-size") => {
                if let (Some(width), Some(height)) = (u("width"), u("height")) {
                    return Message::ContentSize { width, height };
                }
            }
            Some("clipboard") => {
                if let Some(t) = v.get("text").and_then(Value::as_str) {
                    return Message::Clipboard(t.to_string());
                }
            }
            _ => {}
        }
        Message::Other(v)
    }

    /// Parses one JSON line (None when it is not JSON).
    pub fn from_line(line: &str) -> Option<Message> {
        serde_json::from_str(line.trim()).ok().map(Message::from_json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Vec<Input> {
        let m = Mods::SHIFT | Mods::CTRL;
        vec![
            Input::Move { x: 1.5, y: 2.0, buttons: 0, mods: Mods::NONE },
            Input::Press { x: 10.0, y: 20.0, button: Button::Left, buttons: 1, mods: m },
            Input::Release { x: 10.0, y: 20.0, button: Button::Left, buttons: 0, mods: m },
            Input::Double { x: 10.0, y: 20.0, button: Button::Right, buttons: 2, mods: Mods::NONE },
            Input::Leave,
            Input::Wheel { x: 3.0, y: 4.0, dx: 0.0, dy: -2.0, mods: Mods::ALT },
            Input::KeyDown { key: "a".into(), text: "A".into(), mods: Mods::SHIFT, repeat: true },
            Input::KeyUp { key: "Enter".into(), text: String::new(), mods: Mods::META },
            Input::FocusIn,
            Input::FocusOut,
            Input::Resize { width: 640, height: 400, scale: 1.5 },
            Input::Zoom { value: 1.25 },
            Input::Copy,
        ]
    }

    #[test]
    fn every_event_round_trips_through_a_json_line() {
        for e in all() {
            let line = e.to_line();
            assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'), "{line}");
            assert_eq!(Input::from_line(&line), Some(e.clone()), "{line}");
        }
    }

    #[test]
    fn wire_format_is_stable() {
        let e = Input::Press { x: 10.0, y: 20.5, button: Button::Left, buttons: 1, mods: Mods::CTRL };
        assert_eq!(e.to_json().to_string(), r#"{"button":1,"buttons":1,"mods":2,"type":"press","x":10,"y":20.5}"#);
        let k = Input::KeyDown { key: "ArrowLeft".into(), text: String::new(), mods: Mods::NONE, repeat: false };
        assert_eq!(k.to_json()["type"], "key");
        assert_eq!(Input::from_line(r#"{"type":"nope"}"#), None);
        assert_eq!(Input::from_line("not json"), None);
        assert_eq!(Input::from_line(r#"{"type":"press","x":1,"y":2,"button":3}"#), None, "3 is not a button");
    }

    #[test]
    fn messages_parse_known_types_and_keep_the_rest() {
        assert_eq!(Message::from_line(r#"{"type":"content-size","width":800,"height":1200}"#), Some(Message::ContentSize { width: 800, height: 1200 }));
        assert_eq!(Message::from_line(r#"{"type":"clipboard","text":"hi"}"#), Some(Message::Clipboard("hi".into())));
        let other = Message::from_line(r#"{"appRequest":{"id":1}}"#).unwrap();
        assert!(matches!(other, Message::Other(_)));
        for m in [Message::ContentSize { width: 1, height: 2 }, Message::Clipboard("x\ny".into())] {
            assert_eq!(Message::from_line(&m.to_line()), Some(m.clone()));
        }
        assert!(Mods(3).contains(Mods::SHIFT) && !Mods(3).contains(Mods::ALT));
    }
}
