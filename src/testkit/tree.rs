//! The element dump: widgets and the virtual rows near the viewport.

use super::lite;
use crate::a11y::{Node, Role};
use fltk::prelude::*;
use fltk::widget::Widget;
use serde_json::{json, Map, Value};

type Rect = (i32, i32, i32, i32);

/// Longest name or value kept, in characters (a transcript's text).
const TEXT_MAX: usize = 300;

/// One widget, or one virtual row of a widget (a list row, a tree item, a
/// transcript block), as [`tree`] sees it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Element {
    /// Child indices from the window, `/`-separated (`"0/1/3"`; the window
    /// itself is `""`); a virtual row adds `#key` per level (`"0/2#17"`).
    pub path: String,
    /// The role (`Button`, `TextInput`, `ListBoxOption`, `Document`…), or
    /// the fltk class for plain containers (`Window`, `Flex`, `Group`…).
    pub kind: String,
    /// Accessible name: the visible text, `accessible_name`, else the
    /// tooltip. Empty when there is none. At most 300 characters.
    pub name: String,
    /// Current value (field text, slider value, `on`/`off`), at most 300
    /// characters.
    pub value: Option<String>,
    /// Window rectangle `(x, y, w, h)`.
    pub rect: Rect,
    /// Shown: the widget and its groups are visible and the rectangle
    /// meets the area its parents show.
    pub visible: bool,
    /// Has the keyboard focus (a virtual row: its widget has the focus and
    /// the row is the current one).
    pub focused: bool,
    /// Selected (rows, tabs), or checked (switches, check boxes).
    pub selected: bool,
    pub disabled: bool,
    /// Nesting depth (window = 0).
    pub depth: usize,
    /// Key of a virtual row; None for widgets.
    pub key: Option<u64>,
}

impl Element {
    /// Center of the rectangle (where a scripted click lands).
    pub fn center(&self) -> (i32, i32) {
        (self.rect.0 + self.rect.2 / 2, self.rect.1 + self.rect.3 / 2)
    }

    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("path".into(), json!(self.path));
        m.insert("kind".into(), json!(self.kind));
        if !self.name.is_empty() {
            m.insert("name".into(), json!(self.name));
        }
        if let Some(v) = &self.value {
            m.insert("value".into(), json!(v));
        }
        m.insert("rect".into(), json!([self.rect.0, self.rect.1, self.rect.2, self.rect.3]));
        m.insert("visible".into(), json!(self.visible));
        if self.focused {
            m.insert("focused".into(), json!(true));
        }
        if self.selected {
            m.insert("selected".into(), json!(true));
        }
        if self.disabled {
            m.insert("disabled".into(), json!(true));
        }
        m.insert("depth".into(), json!(self.depth));
        if let Some(k) = self.key {
            m.insert("key".into(), json!(k));
        }
        Value::Object(m)
    }
}

/// Every widget of `window` in depth-first order, with the virtual rows
/// components expose near their viewport (list rows, table and tree rows,
/// NavTree items, diff lines; transcript blocks with `a11y`, messages
/// without). Hidden widgets are listed (`visible: false`) but not their
/// children. Builds now, whether or not an assistive technology runs.
pub fn tree<W: WindowExt>(window: &W) -> Vec<Element> {
    // SAFETY: a live window.
    let w = unsafe { Widget::from_widget_ptr(window.as_widget_ptr()) };
    let mut out = Vec::new();
    let focus = fltk::app::focus().map(|f| f.as_widget_ptr() as usize);
    let clip = (0, 0, w.w(), w.h());
    visit(&w, String::new(), 0, (0, 0), clip, true, focus, &mut out);
    out
}

/// [`tree`] as pretty JSON: `{"window": title, "size": [w, h], "elements": [..]}`.
pub fn tree_json<W: WindowExt>(window: &W) -> String {
    let els: Vec<Value> = tree(window).iter().map(Element::to_json).collect();
    let v = json!({
        "window": window.label(),
        "size": [window.w(), window.h()],
        "elements": els,
    });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

/// The first element named `name` (exact match), preferring visible ones.
pub fn find<W: WindowExt>(window: &W, name: &str) -> Option<Element> {
    let all = tree(window);
    pick(&all, |e| e.name == name)
}

pub(crate) fn pick(all: &[Element], f: impl Fn(&Element) -> bool) -> Option<Element> {
    all.iter().find(|e| e.visible && f(e)).or_else(|| all.iter().find(|e| f(e))).cloned()
}

fn clip_to(a: Rect, b: Rect) -> Rect {
    let x0 = a.0.max(b.0);
    let y0 = a.1.max(b.1);
    let x1 = (a.0 + a.2).min(b.0 + b.2);
    let y1 = (a.1 + a.3).min(b.1 + b.3);
    (x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
}

fn meets(a: Rect, clip: Rect) -> bool {
    let c = clip_to(a, clip);
    c.2 > 0 && c.3 > 0
}

fn short(s: &str) -> String {
    if s.chars().count() <= TEXT_MAX {
        return s.to_string();
    }
    let mut t: String = s.chars().take(TEXT_MAX).collect();
    t.push('…');
    t
}

#[allow(clippy::too_many_arguments)]
fn visit(
    w: &Widget,
    path: String,
    depth: usize,
    off: (i32, i32),
    clip: Rect,
    parent_visible: bool,
    focus: Option<usize>,
    out: &mut Vec<Element>,
) {
    let is_window = w.as_window().is_some();
    // Window coordinates of children: fltk's are relative to the nearest
    // window; a subwindow adds its own position.
    let rect = if depth == 0 { (0, 0, w.w(), w.h()) } else { (off.0 + w.x(), off.1 + w.y(), w.w(), w.h()) };
    // The window itself counts as shown: headless windows render offscreen.
    let shown = depth == 0 || (parent_visible && w.visible());
    let visible = shown && meets(rect, clip);
    let d = lite::described(w);
    let mut n = Node { bounds: Some(rect), ..Node::default() };
    if let Some(f) = &d.describe {
        f(&mut n);
    } else {
        basic(w, &mut n);
    }
    let mut name = n.name.clone().unwrap_or_default();
    if let Some(o) = d.name {
        name = o;
    }
    if name.is_empty() {
        if let Some(t) = w.tooltip().filter(|t| !t.is_empty()) {
            name = t;
        }
    }
    if name.is_empty() && (n.role != Role::Unknown || depth == 0) {
        name = clean_label(&w.label());
    }
    let kind = if n.role == Role::Unknown { class_of(w).to_string() } else { format!("{:?}", n.role) };
    // A leaf (a composite control describing itself) has the focus when
    // one of its inner widgets has it.
    let focused = focus == Some(w.as_widget_ptr() as usize)
        || (n.leaf && fltk::app::focus().is_some_and(|f| f.inside(w)));
    out.push(Element {
        path: path.clone(),
        kind,
        name: short(&name),
        value: value_of(&n),
        rect,
        visible,
        focused,
        selected: n.selected == Some(true) || n.toggled == Some(true),
        disabled: !w.active_r() || n.disabled == Some(true),
        depth,
        key: None,
    });
    if !shown {
        return;
    }
    let inner = if depth == 0 { (0, 0, w.w(), w.h()) } else { clip_to(clip, rect) };
    let dy = n.scroll.map_or(0, |(y, _)| y.round() as i32);
    for c in &n.children {
        rows(c, &path, depth + 1, off, -dy, inner, focused, out);
    }
    if n.leaf {
        return;
    }
    let child_off = if is_window && depth > 0 { (off.0 + w.x(), off.1 + w.y()) } else { off };
    if let Some(g) = w.as_group() {
        for i in 0..g.children() {
            if let Some(c) = g.child(i) {
                let p = if path.is_empty() { i.to_string() } else { format!("{path}/{i}") };
                visit(&c, p, depth + 1, child_off, inner, true, focus, out);
            }
        }
    }
}

/// Virtual rows (a11y children): bounds are window coordinates of the
/// nearest window, shifted by a scrolled parent's offset.
#[allow(clippy::too_many_arguments)]
fn rows(n: &Node, parent: &str, depth: usize, off: (i32, i32), dy: i32, clip: Rect, host_focused: bool, out: &mut Vec<Element>) {
    if n.run {
        return;
    }
    let path = format!("{parent}#{}", n.key);
    let rect = n.bounds.map_or((0, 0, 0, 0), |(x, y, w, h)| (off.0 + x, off.1 + y + dy, w, h));
    let visible = !n.hidden && n.bounds.is_some() && meets(rect, clip);
    out.push(Element {
        path: path.clone(),
        kind: format!("{:?}", n.role),
        name: short(n.name.as_deref().unwrap_or("")),
        value: value_of(n),
        rect,
        visible,
        focused: host_focused && n.focused,
        selected: n.selected == Some(true) || n.toggled == Some(true),
        disabled: n.disabled == Some(true),
        depth,
        key: Some(n.key),
    });
    let dy = dy - n.scroll.map_or(0, |(y, _)| y.round() as i32);
    for c in &n.children {
        rows(c, &path, depth + 1, off, dy, clip, host_focused, out);
    }
}

fn value_of(n: &Node) -> Option<String> {
    if let Some(v) = &n.value {
        return Some(short(v));
    }
    if let Some((v, ..)) = n.numeric {
        return Some(crate::a11y::number_text(v));
    }
    n.toggled.map(|t| if t { "on" } else { "off" }.to_string())
}

/// FLTK labels: `&` marks a shortcut letter, `@` starts a symbol.
fn clean_label(s: &str) -> String {
    if s.starts_with('@') {
        return String::new();
    }
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '&' {
            if it.peek() == Some(&'&') {
                out.push('&');
                it.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// The fltk class of a widget without a role of its own.
fn class_of(w: &Widget) -> &'static str {
    use fltk::{group, window};
    let p = w.as_widget_ptr();
    if window::Window::from_dyn_widget_ptr(p as _).is_some() || w.as_window().is_some() {
        "Window"
    } else if group::Flex::from_dyn_widget_ptr(p as _).is_some() {
        "Flex"
    } else if group::Scroll::from_dyn_widget_ptr(p as _).is_some() {
        "Scroll"
    } else if w.as_group().is_some() {
        "Group"
    } else {
        "Widget"
    }
}

/// A node for a plain fltk widget (no description of its own), by type.
fn basic(w: &Widget, n: &mut Node) {
    use fltk::{button, frame, input, menu, misc, text, valuator};
    let p = w.as_widget_ptr();
    let label = clean_label(&w.label());
    if let Some(b) = button::CheckButton::from_dyn_widget_ptr(p as _) {
        n.role(Role::CheckBox).name(&label).toggled(b.value());
    } else if let Some(b) = button::RoundButton::from_dyn_widget_ptr(p as _) {
        n.role(Role::RadioButton).name(&label).toggled(b.value());
    } else if let Some(b) = button::LightButton::from_dyn_widget_ptr(p as _) {
        n.role(Role::Switch).name(&label).toggled(b.value());
    } else if let Some(b) = button::Button::from_dyn_widget_ptr(p as _) {
        n.role(Role::Button).name(&label);
        if matches!(b.get_type::<button::ButtonType>(), button::ButtonType::Toggle | button::ButtonType::Radio) {
            n.toggled(b.value());
        }
    } else if let Some(i) = input::Input::from_dyn_widget_ptr(p as _) {
        n.role(Role::TextInput).name(&label).value(&i.value());
    } else if let Some(t) = text::TextDisplay::from_dyn_widget_ptr(p as _) {
        let editable = text::TextEditor::from_dyn_widget_ptr(p as _).is_some();
        let v = t.buffer().map(|b| b.text()).unwrap_or_default();
        n.role(if editable { Role::MultilineTextInput } else { Role::Document }).name(&label).value(&v);
    } else if valuator::Scrollbar::from_dyn_widget_ptr(p as _).is_some() {
        // Scroll bars belong to their scroll view.
    } else if let Some(s) = valuator::Slider::from_dyn_widget_ptr(p as _) {
        n.role(Role::Slider).name(&label).numeric(s.value(), s.minimum(), s.maximum(), s.step());
    } else if let Some(pr) = misc::Progress::from_dyn_widget_ptr(p as _) {
        n.role(Role::ProgressIndicator).name(&label).numeric(pr.value(), pr.minimum(), pr.maximum(), 0.0);
    } else if let Some(c) = menu::Choice::from_dyn_widget_ptr(p as _) {
        n.role(Role::ComboBox).name(&label).value(&c.choice().unwrap_or_default());
    } else if menu::MenuButton::from_dyn_widget_ptr(p as _).is_some() {
        n.role(Role::Button).name(&label);
    } else if frame::Frame::from_dyn_widget_ptr(p as _).is_some() {
        if !label.is_empty() {
            n.role(Role::Label).name(&label);
        }
    } else if w.as_group().is_some() {
        if !label.is_empty() && w.as_window().is_none() {
            n.role(Role::Group).name(&label);
        }
    } else if !label.is_empty() {
        n.role(Role::Label).name(&label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_clipping() {
        assert_eq!(clean_label("&Save"), "Save");
        assert_eq!(clean_label("A && B"), "A & B");
        assert_eq!(clean_label("@>"), "");
        assert_eq!(clip_to((0, 0, 10, 10), (5, 5, 10, 10)), (5, 5, 5, 5));
        assert!(!meets((0, 0, 10, 10), (10, 0, 10, 10)));
        assert_eq!(short(&"x".repeat(400)).chars().count(), TEXT_MAX + 1);
    }

    #[test]
    fn element_json_skips_defaults() {
        let e = Element { path: "0/1".into(), kind: "Button".into(), name: "Save".into(), rect: (1, 2, 3, 4), visible: true, ..Element::default() };
        let v = e.to_json();
        assert_eq!(v["name"], "Save");
        assert!(v.get("focused").is_none() && v.get("value").is_none());
        assert_eq!(v["rect"], json!([1, 2, 3, 4]));
        assert_eq!(e.center(), (2, 4));
    }
}
