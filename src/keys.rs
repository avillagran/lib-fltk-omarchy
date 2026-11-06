//! Human-readable shortcuts: "Ctrl+S", "Ctrl+Shift+P", "Alt+Enter", "F5", "Escape".
//!
//! **Portable shortcuts**: write `Mod` (also `Primary`, `CmdOrCtrl`) for
//! the platform's primary modifier: Cmd on macOS, Ctrl elsewhere.
//! `"Mod+,"` is Preferences everywhere (`Ctrl+,` on Linux and Windows,
//! `Cmd+,` on macOS). `Ctrl` always means the Control key. [`label`] shows
//! a spec the way the platform writes it ("Ctrl+," / "⌘,").

use crate::platform::{Os, OS};
use fltk::{app, enums::{Key, Shortcut}};
use std::cell::{Cell, RefCell};

/// The platform's primary modifier: Cmd (`Shortcut::Meta`) on macOS, Ctrl
/// elsewhere. Copy, select all, toggle-in-selection and app shortcuts use it.
pub const PRIMARY: Shortcut = if matches!(OS, Os::MacOs) { Shortcut::Meta } else { Shortcut::Ctrl };

/// True when the primary modifier is held in the current event.
pub fn primary_held() -> bool {
    app::event_state().contains(PRIMARY)
}

/// True when `state` includes the primary modifier.
pub fn is_primary(state: Shortcut) -> bool {
    state.contains(PRIMARY)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keys {
    pub mods: Shortcut,
    pub key: Key,
}

pub fn parse(spec: &str) -> Result<Keys, String> {
    // The plus key itself: "+", "Ctrl++".
    let t = spec.trim();
    let (head, plus) = match t.strip_suffix('+') {
        Some(rest) if rest.is_empty() || rest.trim_end().ends_with('+') => (rest.trim_end().strip_suffix('+').unwrap_or(""), true),
        _ => (t, false),
    };
    let mut parts: Vec<&str> = head.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    if plus {
        parts.push("+");
    }
    let key = parts.pop().ok_or_else(|| format!("empty shortcut {spec:?}"))?;
    let mut mods = Shortcut::None;
    for m in parts {
        mods |= match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Shortcut::Ctrl,
            "shift" => Shortcut::Shift,
            "alt" | "option" | "opt" => Shortcut::Alt,
            "super" | "meta" | "cmd" | "command" => Shortcut::Meta,
            "mod" | "primary" | "cmdorctrl" | "ctrlorcmd" | "commandorcontrol" => PRIMARY,
            other => return Err(format!("unknown modifier {other:?} in {spec:?} (use Mod, Ctrl, Shift, Alt, Super)")),
        };
    }
    let lower = key.to_ascii_lowercase();
    let key = match lower.as_str() {
        "enter" | "return" => Key::Enter,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "backspace" => Key::BackSpace,
        "delete" | "del" => Key::Delete,
        "space" => Key::from_char(' '),
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "menu" => Key::Menu,
        f if f.len() >= 2 && f.starts_with('f') && f[1..].parse::<i32>().is_ok_and(|n| (1..=12).contains(&n)) => {
            Key::from_i32(Key::F1.bits() + f[1..].parse::<i32>().unwrap() - 1)
        }
        c if c.chars().count() == 1 => Key::from_char(c.chars().next().unwrap()),
        other => return Err(format!("unknown key {other:?} in {spec:?}")),
    };
    Ok(Keys { mods, key })
}

impl Keys {
    /// True when the current FLTK event is this key with exactly these modifiers.
    pub fn matches_event(&self) -> bool {
        self.matches(app::event_key(), app::event_state())
    }

    pub fn matches(&self, key: Key, state: Shortcut) -> bool {
        let mask = Shortcut::Ctrl | Shortcut::Shift | Shortcut::Alt | Shortcut::Meta;
        let key = if (Key::from_char('A').bits()..=Key::from_char('Z').bits()).contains(&key.bits()) {
            Key::from_i32(key.bits() + 32)
        } else {
            key
        };
        (state & mask) == self.mods && key == self.key
    }
}

/// `spec` as this platform writes shortcuts, for menus and the F1 help:
/// `Mod` becomes Ctrl (Cmd on macOS); macOS uses its symbols in its order
/// (⌃⌥⇧⌘, "⌘,", "⇧⌘P"). Text that is not a shortcut ("Up/Down", "Enter")
/// is returned as is. On Linux and Windows every spec without `Mod` is
/// unchanged.
pub fn label(spec: &str) -> String {
    let parts: Vec<&str> = spec.split('+').map(str::trim).collect();
    if parts.len() < 2 || parts.iter().any(|p| p.is_empty()) {
        return spec.to_string();
    }
    let (mods, key) = parts.split_at(parts.len() - 1);
    let mut set = Shortcut::None;
    for m in mods {
        set |= match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => Shortcut::Ctrl,
            "shift" => Shortcut::Shift,
            "alt" | "option" | "opt" => Shortcut::Alt,
            "super" | "meta" | "cmd" | "command" => Shortcut::Meta,
            "mod" | "primary" | "cmdorctrl" | "ctrlorcmd" | "commandorcontrol" => PRIMARY,
            _ => return spec.to_string(),
        };
    }
    let key = key[0];
    if OS == Os::MacOs {
        let mut out = String::new();
        for (m, sym) in [(Shortcut::Ctrl, "⌃"), (Shortcut::Alt, "⌥"), (Shortcut::Shift, "⇧"), (Shortcut::Meta, "⌘")] {
            if set.contains(m) {
                out.push_str(sym);
            }
        }
        out.push_str(&upper_letter(key));
        return out;
    }
    if !mods.iter().any(|m| matches!(m.to_ascii_lowercase().as_str(), "mod" | "primary" | "cmdorctrl" | "ctrlorcmd" | "commandorcontrol")) {
        return spec.to_string(); // unchanged: no portable modifier in it
    }
    let mut out: Vec<&str> = Vec::new();
    let meta = if OS == Os::Windows { "Win" } else { "Super" };
    for (m, name) in [(Shortcut::Ctrl, "Ctrl"), (Shortcut::Alt, "Alt"), (Shortcut::Shift, "Shift"), (Shortcut::Meta, meta)] {
        if set.contains(m) {
            out.push(name);
        }
    }
    out.push(key);
    out.join("+")
}

/// "p" → "P" (macOS menus show letter keys in capitals); other keys as is.
fn upper_letter(key: &str) -> String {
    let mut c = key.chars();
    match (c.next(), c.next()) {
        (Some(ch), None) if ch.is_alphabetic() => ch.to_uppercase().collect(),
        _ => key.to_string(),
    }
}

// ============================================== repeat, suspend, record

thread_local! {
    static DOWN: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    static REPEAT: Cell<bool> = const { Cell::new(false) };
    static SUSPENDED: Cell<u32> = const { Cell::new(0) };
    static RECORDER: RefCell<Option<Box<dyn FnMut(Option<String>)>>> = const { RefCell::new(None) };
}

/// The keyboard router calls this for every key event: it tells a held
/// key's automatic repeat from a new press.
pub(crate) fn note(ev: fltk::enums::Event, key: Key) {
    use fltk::enums::Event;
    let k = key.bits();
    match ev {
        Event::KeyDown => DOWN.with(|d| {
            let mut d = d.borrow_mut();
            REPEAT.with(|r| r.set(d.contains(&k)));
            if !d.contains(&k) {
                d.push(k);
            }
        }),
        Event::KeyUp => DOWN.with(|d| d.borrow_mut().retain(|x| *x != k)),
        _ => {}
    }
}

/// Forgets which keys are held (`focus::press` sends no key release).
pub(crate) fn release_all() {
    DOWN.with(|d| d.borrow_mut().clear());
    REPEAT.with(|r| r.set(false));
}

/// Marks `key` as held, so the next key-down of it is a repeat (tests).
pub(crate) fn hold(key: Key) {
    DOWN.with(|d| {
        let mut d = d.borrow_mut();
        if !d.contains(&key.bits()) {
            d.push(key.bits());
        }
    });
}

/// True while the current key-down is the automatic repeat of a held key.
/// Shortcuts that should not repeat ignore these.
pub fn is_repeat() -> bool {
    REPEAT.with(|r| r.get())
}

/// Turns every app shortcut off while the guard lives ([`AppWindow::shortcut`]
/// and [`Shortcuts`] tables): a key-recording field gets the keys instead.
/// Guards nest.
///
/// [`AppWindow::shortcut`]: crate::AppWindow::shortcut
/// [`Shortcuts`]: crate::shortcuts::Shortcuts
#[must_use = "shortcuts come back when the guard is dropped"]
pub struct Suspend(());

impl Suspend {
    pub fn new() -> Suspend {
        SUSPENDED.with(|s| s.set(s.get() + 1));
        Suspend(())
    }
}

impl Default for Suspend {
    fn default() -> Self {
        Suspend::new()
    }
}

impl Drop for Suspend {
    fn drop(&mut self) {
        SUSPENDED.with(|s| s.set(s.get().saturating_sub(1)));
    }
}

/// True while app shortcuts are suspended ([`Suspend`], [`record`]).
pub fn suspended() -> bool {
    SUSPENDED.with(|s| s.get() > 0) || recording()
}

/// True while [`record`] waits for a key.
pub fn recording() -> bool {
    RECORDER.with(|r| r.borrow().is_some())
}

/// Records the next shortcut typed anywhere in the app: modifier keys
/// alone are ignored, the first other key with the modifiers held is
/// reported as a spec ([`spec_of`]), Escape without modifiers cancels
/// (`None`). App shortcuts and the focused control do not see that key.
/// A new call replaces a pending recording (the old one gets `None`).
pub fn record(f: impl FnMut(Option<String>) + 'static) {
    let old = RECORDER.with(|r| r.borrow_mut().replace(Box::new(f)));
    if let Some(mut old) = old {
        old(None);
    }
}

/// Stops a pending [`record`] (its callback gets `None`).
pub fn cancel_record() {
    if let Some(mut f) = RECORDER.with(|r| r.borrow_mut().take()) {
        f(None);
    }
}

fn is_modifier(key: Key) -> bool {
    // Shift, Control, Caps Lock, Meta, Alt, Super, Hyper (left and right).
    (0xffe1..=0xffee).contains(&key.bits()) || key.bits() == 0xfe03
}

/// Called by the keyboard router on key-down: true when a recording took it.
pub(crate) fn record_key(key: Key, state: Shortcut) -> bool {
    if !recording() || is_modifier(key) {
        return recording();
    }
    let mask = Shortcut::Ctrl | Shortcut::Shift | Shortcut::Alt | Shortcut::Meta;
    let spec = if key == Key::Escape && (state & mask) == Shortcut::None { None } else { spec_of(key, state) };
    if let Some(mut f) = RECORDER.with(|r| r.borrow_mut().take()) {
        f(spec);
    }
    true
}

/// The spec [`parse`] reads back for `key` with the modifiers in `state`:
/// `"Mod+Shift+p"`, `"Alt+F4"`, `"Ctrl+2"`. The platform's primary modifier
/// is written `Mod` (portable), the key is FLTK's unshifted key (Shift+2 is
/// `"Shift+2"`, not `"@"`). `None` for keys without a name.
pub fn spec_of(key: Key, state: Shortcut) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    let other = if PRIMARY == Shortcut::Ctrl { Shortcut::Meta } else { Shortcut::Ctrl };
    if state.contains(PRIMARY) {
        parts.push("Mod");
    }
    if state.contains(other) {
        parts.push(if other == Shortcut::Meta { "Super" } else { "Ctrl" });
    }
    if state.contains(Shortcut::Alt) {
        parts.push("Alt");
    }
    if state.contains(Shortcut::Shift) {
        parts.push("Shift");
    }
    let name = key_name(key)?;
    let mut out = parts.join("+");
    if !out.is_empty() {
        out.push('+');
    }
    out.push_str(&name);
    Some(out)
}

fn key_name(key: Key) -> Option<String> {
    let named = [
        (Key::Enter, "Enter"),
        (Key::KPEnter, "Enter"),
        (Key::Escape, "Escape"),
        (Key::Tab, "Tab"),
        (Key::BackSpace, "Backspace"),
        (Key::Delete, "Delete"),
        (Key::Up, "Up"),
        (Key::Down, "Down"),
        (Key::Left, "Left"),
        (Key::Right, "Right"),
        (Key::Home, "Home"),
        (Key::End, "End"),
        (Key::PageUp, "PageUp"),
        (Key::PageDown, "PageDown"),
        (Key::Menu, "Menu"),
    ];
    if let Some((_, n)) = named.iter().find(|(k, _)| *k == key) {
        return Some((*n).to_string());
    }
    let b = key.bits();
    if (Key::F1.bits()..=Key::F1.bits() + 11).contains(&b) {
        return Some(format!("F{}", b - Key::F1.bits() + 1));
    }
    if b == ' ' as i32 {
        return Some("Space".into());
    }
    if (0x21..0x7f).contains(&b) && b != '+' as i32 {
        return Some((b as u8 as char).to_ascii_lowercase().to_string());
    }
    if b == '+' as i32 {
        return Some("+".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_shortcuts() {
        let k = parse("Ctrl+Shift+P").unwrap();
        assert_eq!(k.mods, Shortcut::Ctrl | Shortcut::Shift);
        assert_eq!(k.key, Key::from_char('p'));
        assert!(k.matches(Key::from_char('p'), Shortcut::Ctrl | Shortcut::Shift));
        assert!(!k.matches(Key::from_char('p'), Shortcut::Ctrl));
        assert_eq!(parse("F5").unwrap().key, Key::F5);
        assert!(parse("Hyper+X").is_err());
        assert_eq!(parse("Ctrl++").unwrap(), Keys { mods: Shortcut::Ctrl, key: Key::from_char('+') });
        assert_eq!(parse("+").unwrap(), Keys { mods: Shortcut::None, key: Key::from_char('+') });
    }

    #[test]
    fn primary_modifier_is_portable() {
        let k = parse("Mod+,").unwrap();
        assert_eq!(k.mods, PRIMARY);
        assert_eq!(parse("CmdOrCtrl+Shift+P").unwrap().mods, PRIMARY | Shortcut::Shift);
        assert!(is_primary(PRIMARY | Shortcut::Shift));
        match OS {
            Os::MacOs => {
                assert_eq!(PRIMARY, Shortcut::Meta);
                assert_eq!(label("Mod+,"), "⌘,");
                assert_eq!(label("Mod+Shift+p"), "⇧⌘P");
                assert_eq!(label("Ctrl+Tab"), "⌃Tab");
            }
            _ => {
                assert_eq!(PRIMARY, Shortcut::Ctrl);
                assert_eq!(label("Mod+,"), "Ctrl+,");
                assert_eq!(label("Mod+Shift+P"), "Ctrl+Shift+P");
                // Specs without Mod are shown exactly as written.
                assert_eq!(label("Ctrl+Tab"), "Ctrl+Tab");
                assert_eq!(label("Shift+Tab"), "Shift+Tab");
            }
        }
        assert_eq!(label("Up/Down"), "Up/Down");
        assert_eq!(label("F1"), "F1");
        assert_eq!(label("Ctrl++"), "Ctrl++");
    }

    #[test]
    fn spec_of_round_trips() {
        let k = parse("Mod+Shift+p").unwrap();
        let spec = spec_of(k.key, k.mods).unwrap();
        assert_eq!(spec, "Mod+Shift+p");
        assert_eq!(parse(&spec).unwrap(), k);
        assert_eq!(spec_of(Key::F4, Shortcut::Alt).as_deref(), Some("Alt+F4"));
        assert_eq!(spec_of(Key::from_char('2'), Shortcut::Ctrl | Shortcut::Shift).map(|s| parse(&s).unwrap()), Some(Keys { mods: Shortcut::Ctrl | Shortcut::Shift, key: Key::from_char('2') }));
        assert_eq!(spec_of(Key::from_char(' '), Shortcut::None).as_deref(), Some("Space"));
        assert_eq!(spec_of(Key::from_i32(0xffe1), Shortcut::None), None); // Shift alone
        for s in ["Enter", "Escape", "PageDown", "F12", "Alt+Up", "Ctrl++"] {
            let k = parse(s).unwrap();
            assert_eq!(parse(&spec_of(k.key, k.mods).unwrap()).unwrap(), k, "{s}");
        }
    }

    #[test]
    fn repeat_and_suspend() {
        use fltk::enums::Event;
        release_all();
        note(Event::KeyDown, Key::Down);
        assert!(!is_repeat());
        note(Event::KeyDown, Key::Down);
        assert!(is_repeat());
        note(Event::KeyUp, Key::Down);
        note(Event::KeyDown, Key::Down);
        assert!(!is_repeat());
        release_all();
        assert!(!suspended());
        {
            let _a = Suspend::new();
            let _b = Suspend::new();
            assert!(suspended());
        }
        assert!(!suspended());
    }
}
