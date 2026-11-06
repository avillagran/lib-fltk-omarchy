//! Persistent, non-modal notice cards with app-owned actions.
use super::*;
use std::rc::Weak;

type Rect = (i32, i32, i32, i32);
type ActionCallback = dyn FnMut(&NoticeHandle, &str);

/// An action's stable app ID and live translated label.
#[derive(Clone)]
pub struct NoticeAction {
    id: String,
    label: State<String>,
    enabled: bool,
}
impl NoticeAction {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.into(),
            label: State::new(label.into()),
            enabled: true,
        }
    }
    pub fn bind_text(mut self, label: &State<String>) -> Self {
        self.label = label.clone();
        self
    }
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// Why an explicitly dismissed notice closed. Actions remain app-owned:
/// an action closes only when its callback calls `handle.close()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeDismiss {
    Closed,
    CloseButton,
    Keyboard,
}

/// Persistent notification. Showing it never takes focus. Action/body
/// clicks never dismiss implicitly, and there is no expiry timer.
pub struct Notice {
    summary: State<String>,
    body: State<String>,
    icon: String,
    urgency: Urgency,
    actions: Vec<NoticeAction>,
    on_action: Option<Box<ActionCallback>>,
    on_dismiss: Option<Box<dyn FnOnce(NoticeDismiss)>>,
}
impl Notice {
    pub fn new(summary: &str, body: &str) -> Self {
        Self {
            summary: State::new(summary.into()),
            body: State::new(body.into()),
            icon: String::new(),
            urgency: Urgency::Normal,
            actions: Vec::new(),
            on_action: None,
            on_dismiss: None,
        }
    }
    pub fn icon(mut self, icon: &str) -> Self {
        self.icon = icon.into();
        self
    }
    /// Changes the accessibility announcement priority, not persistence.
    pub fn urgency(mut self, urgency: Urgency) -> Self {
        self.urgency = urgency;
        self
    }
    pub fn bind_summary(mut self, text: &State<String>) -> Self {
        self.summary = text.clone();
        self
    }
    pub fn bind_body(mut self, text: &State<String>) -> Self {
        self.body = text.clone();
        self
    }
    /// IDs must be nonempty and unique in this card.
    pub fn action(mut self, action: NoticeAction) -> Self {
        assert!(
            !action.id.is_empty() && !self.actions.iter().any(|a| a.id == action.id),
            "notice action IDs must be nonempty and unique"
        );
        self.actions.push(action);
        self
    }
    pub fn on_action(mut self, f: impl FnMut(&NoticeHandle, &str) + 'static) -> Self {
        self.on_action = Some(Box::new(f));
        self
    }
    pub fn on_dismiss(mut self, f: impl FnOnce(NoticeDismiss) + 'static) -> Self {
        self.on_dismiss = Some(Box::new(f));
        self
    }
    /// Shows in the anchor's containing window. Dropping the returned
    /// handle leaves it open; closing/deleting that window destroys it.
    pub fn show<A: WidgetExt>(self, anchor: &A) -> Option<NoticeHandle> {
        let win = anchor.top_window()?;
        let cursor = self
            .actions
            .iter()
            .position(|a| a.enabled)
            .map(Hit::Action)
            .unwrap_or(Hit::Close);
        let data = Rc::new(Data {
            summary: self.summary,
            body: self.body,
            icon: self.icon,
            urgency: self.urgency,
            actions: self.actions,
            open: Cell::new(true),
            cursor: Cell::new(cursor),
            hover: Cell::new(None),
            pressed: Cell::new(None),
            metrics: RefCell::new(None),
        });
        let (w, h) = size(&data, win.w());
        let mut layer = open_layer(anchor, Some((0, 0, w, h)), false)?;
        let callbacks = Rc::new(Callbacks {
            action: RefCell::new(self.on_action),
            dismiss: RefCell::new(self.on_dismiss),
        });
        let handle = NoticeHandle {
            layer: layer.clone(),
            data: data.clone(),
            callbacks: Rc::downgrade(&callbacks),
        };
        let d = data.clone();
        register_toast(&layer, Rc::new(move |ww| size(&d, ww)));
        let d = data.clone();
        draw_layer(&mut layer, move |w| draw(w, &d));
        layer.set_visible_focus();
        let me = handle.clone();
        // Only the native widget owns the callbacks. An external handle
        // cannot keep their captured resources alive after window deletion.
        layer.handle(move |w, ev| {
            let _keep = &callbacks;
            let d = &me.data;
            if !d.open.get() {
                return false;
            }
            match ev {
                Event::Focus => {
                    crate::focus::remember(w);
                    w.redraw();
                    true
                }
                Event::Unfocus => {
                    d.pressed.set(None);
                    w.redraw();
                    true
                }
                Event::Enter | Event::Move => {
                    let next = hit(w, d, app::event_x(), app::event_y());
                    if d.hover.replace(next) != next {
                        w.redraw();
                    }
                    true
                }
                Event::Leave => {
                    if d.hover.take().is_some() {
                        w.redraw();
                    }
                    true
                }
                Event::Push => {
                    d.pressed.set(None);
                    if app::event_button() == 1 {
                        let h =
                            hit(w, d, app::event_x(), app::event_y()).filter(|h| enabled(d, *h));
                        if let Some(h) = h {
                            d.cursor.set(h);
                            let _ = w.take_focus();
                            d.pressed.set(Some(h));
                            w.redraw();
                        }
                    }
                    true
                }
                Event::Released => {
                    if app::event_button() != 1 {
                        return true;
                    }
                    if let Some(h) = d
                        .pressed
                        .take()
                        .filter(|h| Some(*h) == hit(w, d, app::event_x(), app::event_y()))
                    {
                        me.invoke(h, NoticeDismiss::CloseButton);
                    }
                    true
                }
                Event::KeyDown => {
                    let key = crate::layout::logical_key(w, app::event_key());
                    if matches!(key, Key::Escape | Key::Delete | Key::BackSpace) {
                        me.dismiss(NoticeDismiss::Keyboard);
                        return true;
                    }
                    if matches!(key, Key::Enter | Key::KPEnter) || key == Key::from_char(' ') {
                        me.invoke(d.cursor.get(), NoticeDismiss::Keyboard);
                        return true;
                    }
                    let options: Vec<_> = (0..d.actions.len())
                        .map(Hit::Action)
                        .chain(std::iter::once(Hit::Close))
                        .filter(|h| enabled(d, *h))
                        .collect();
                    let i = options
                        .iter()
                        .position(|h| *h == d.cursor.get())
                        .unwrap_or(0);
                    let next = match key {
                        Key::Left => i.saturating_sub(1),
                        Key::Right => (i + 1).min(options.len() - 1),
                        Key::Home => 0,
                        Key::End => options.len() - 1,
                        _ => return false,
                    };
                    d.cursor.set(options[next]);
                    w.redraw();
                    true
                }
                _ => false,
            }
        });
        for text in std::iter::once(&data.summary)
            .chain(std::iter::once(&data.body))
            .chain(data.actions.iter().map(|a| &a.label))
        {
            let mut w = layer.clone();
            let d = data.clone();
            crate::state::bind_widget(&layer, text, false, move |_| {
                d.metrics.borrow_mut().take();
                layout_toasts();
                w.redraw();
            });
        }
        let mut w = layer.clone();
        crate::i18n::bind(&layer, move || {
            layout_toasts();
            w.redraw();
        });
        let me = handle.clone();
        a11y::describe(&layer, move |n| {
            if !me.is_open() {
                return;
            }
            let d = &me.data;
            n.role(Role::Alert)
                .name(&d.summary.get())
                .description(&d.body.get())
                .live(if d.urgency == Urgency::Critical {
                    Live::Assertive
                } else {
                    Live::Polite
                })
                .focusable();
            let g = geometry(&me.layer, d);
            for (i, a) in d.actions.iter().enumerate() {
                let (x, y, w, h) = g.actions[i];
                n.child(i as u64 + 1, |n| {
                    n.role(Role::Button)
                        .name(&a.label.get())
                        .bounds(x, y, w, h)
                        .disabled(!a.enabled)
                        .focusable()
                        .action(Action::Click);
                    if crate::focus::is_focused(&me.layer) && d.cursor.get() == Hit::Action(i) {
                        n.focused();
                    }
                });
            }
            let (x, y, w, h) = g.close;
            n.child(u64::MAX, |n| {
                n.role(Role::Button)
                    .name(&crate::tr!("Close"))
                    .bounds(x, y, w, h)
                    .focusable()
                    .action(Action::Click);
                if crate::focus::is_focused(&me.layer) && d.cursor.get() == Hit::Close {
                    n.focused();
                }
            });
        });
        let me = handle.clone();
        a11y::on_action(&layer, move |r| {
            if !me.is_open() {
                return false;
            }
            let h = match r.key() {
                Some(u64::MAX) => Hit::Close,
                Some(i) if i > 0 && (i as usize) <= me.data.actions.len() => {
                    Hit::Action(i as usize - 1)
                }
                None => me.data.cursor.get(),
                _ => return false,
            };
            if !enabled(&me.data, h) {
                return false;
            }
            match r.action {
                Action::Click => return me.invoke(h, NoticeDismiss::CloseButton),
                Action::Focus => {
                    me.data.cursor.set(h);
                    let _ = me.layer.clone().take_focus();
                    me.layer.clone().redraw();
                }
                _ => return false,
            }
            true
        });
        Some(handle)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Action(usize),
    Close,
}
struct Data {
    summary: State<String>,
    body: State<String>,
    icon: String,
    urgency: Urgency,
    actions: Vec<NoticeAction>,
    open: Cell<bool>,
    cursor: Cell<Hit>,
    hover: Cell<Option<Hit>>,
    pressed: Cell<Option<Hit>>,
    metrics: RefCell<Option<(i32, u64, Rc<Metrics>)>>,
}
struct Callbacks {
    action: RefCell<Option<Box<ActionCallback>>>,
    dismiss: RefCell<Option<Box<dyn FnOnce(NoticeDismiss)>>>,
}

/// Deletion-tracked notice handle. It does not own a native window or
/// extend the lifetime of callback captures after that window is deleted.
#[derive(Clone)]
pub struct NoticeHandle {
    layer: Widget,
    data: Rc<Data>,
    callbacks: Weak<Callbacks>,
}
impl NoticeHandle {
    pub fn is_open(&self) -> bool {
        self.data.open.get() && !self.layer.was_deleted() && self.layer.parent().is_some()
    }
    pub fn close(&self) {
        self.dismiss(NoticeDismiss::Closed);
    }
    fn dismiss(&self, why: NoticeDismiss) {
        if !self.is_open() || !self.data.open.replace(false) {
            return;
        }
        let cb = self.callbacks.upgrade();
        if let Some(cb) = &cb {
            cb.action.borrow_mut().take();
        }
        dismiss(&self.layer);
        let done = cb.and_then(|cb| cb.dismiss.borrow_mut().take());
        if let Some(done) = done {
            done(why);
        }
    }
    /// Same action path as mouse, keyboard and assistive technology.
    /// Returns false for disabled/unknown actions, closed/deleted notices,
    /// a missing callback or reentry while the callback is already running.
    pub fn activate(&self, id: &str) -> bool {
        let Some(i) = self
            .data
            .actions
            .iter()
            .position(|a| a.id == id && a.enabled)
        else {
            return false;
        };
        if !self.is_open() {
            return false;
        }
        self.invoke(Hit::Action(i), NoticeDismiss::Closed)
    }
    fn invoke(&self, h: Hit, why: NoticeDismiss) -> bool {
        if !self.is_open() || !enabled(&self.data, h) {
            return false;
        }
        let Hit::Action(i) = h else {
            self.dismiss(why);
            return true;
        };
        let Some(callbacks) = self.callbacks.upgrade() else {
            return false;
        };
        let callback = callbacks.action.borrow_mut().take();
        if let Some(mut callback) = callback {
            callback(self, &self.data.actions[i].id);
            if self.is_open() {
                *callbacks.action.borrow_mut() = Some(callback);
            }
            true
        } else {
            false
        }
    }
    /// Card rectangle in window logical coordinates; None after close/deletion.
    pub fn rect(&self) -> Option<(i32, i32, i32, i32)> {
        self.is_open().then(|| {
            (
                self.layer.x(),
                self.layer.y(),
                self.layer.w(),
                self.layer.h(),
            )
        })
    }
    /// Actual action hit rectangle, including its clickable padding.
    pub fn action_rect(&self, id: &str) -> Option<(i32, i32, i32, i32)> {
        if !self.is_open() {
            return None;
        }
        let i = self.data.actions.iter().position(|a| a.id == id)?;
        geometry(&self.layer, &self.data).actions.get(i).copied()
    }
    /// Actual close hit rectangle in window logical coordinates.
    pub fn close_rect(&self) -> Option<(i32, i32, i32, i32)> {
        self.is_open()
            .then(|| geometry(&self.layer, &self.data).close)
    }
    /// Focus a particular action without running it.
    pub fn focus_action(&self, id: &str) -> bool {
        if !self.is_open() {
            return false;
        }
        let Some(i) = self
            .data
            .actions
            .iter()
            .position(|a| a.id == id && a.enabled)
        else {
            return false;
        };
        self.data.cursor.set(Hit::Action(i));
        self.layer.clone().redraw();
        self.layer.clone().take_focus().is_ok()
    }
}

fn enabled(d: &Data, h: Hit) -> bool {
    match h {
        Hit::Action(i) => d.actions.get(i).is_some_and(|a| a.enabled),
        Hit::Close => true,
    }
}
struct Geometry {
    summary: Rect,
    body: Rect,
    close: Rect,
    icon: Rect,
    actions: Vec<Rect>,
}
struct Metrics {
    height: i32,
    actions: Vec<Rect>,
}
fn metrics(d: &Data, width: i32) -> Rc<Metrics> {
    let generation = theme::generation();
    if let Some((w, g, m)) = d.metrics.borrow().as_ref() {
        if *w == width && *g == generation {
            return m.clone();
        }
    }
    let measured = Rc::new(theme::with(|t| {
        let pad = t.spacing.popup_padding.max(8);
        let line = t.font.title + 6;
        let mut y = pad
            + line
            + if d.body.get().is_empty() {
                0
            } else {
                2 * line + 4
            };
        let mut x = pad;
        let mut actions = Vec::new();
        let available = (width - 2 * pad).max(1);
        if !d.actions.is_empty() {
            y += t.spacing.sm;
        }
        for action in &d.actions {
            let w = (paint::measure(&action.label.get(), t.font.body, false)
                + 2 * t.spacing.control_padding_x)
                .min(available)
                .max(1);
            if x > pad && x + w > width - pad {
                x = pad;
                y += t.spacing.control_height + t.spacing.sm;
            }
            actions.push((x, y, w, t.spacing.control_height));
            x += w + t.spacing.sm;
        }
        if !actions.is_empty() {
            y += t.spacing.control_height;
        }
        Metrics {
            height: y + pad,
            actions,
        }
    }));
    *d.metrics.borrow_mut() = Some((width, generation, measured.clone()));
    measured
}
fn size(d: &Data, ww: i32) -> (i32, i32) {
    let width = theme::with(|t| {
        (380 * t.font.base / 12)
            .min(ww - 2 * t.gaps_out.max(8))
            .max(1)
    });
    (width, metrics(d, width).height)
}
fn geometry(w: &Widget, d: &Data) -> Geometry {
    theme::with(|t| {
        let pad = t.spacing.popup_padding.max(8);
        let line = t.font.title + 6;
        let m = crate::layout::Mirror::of(w);
        let rect = |x, y, rw, rh| (m.x(w.x() + x, rw), w.y() + y, rw, rh);
        let close = rect((w.w() - pad - line).max(0), pad, line.min(w.w()), line);
        let slot = if d.icon.is_empty() { 0 } else { line + pad };
        Geometry {
            summary: rect(
                pad + slot,
                pad,
                (w.w() - 2 * pad - line - slot - 4).max(0),
                line,
            ),
            body: rect(pad, pad + line + 4, (w.w() - 2 * pad).max(0), 2 * line),
            close,
            icon: rect(pad, pad, line, line),
            actions: metrics(d, w.w())
                .actions
                .iter()
                .map(|&(x, y, rw, rh)| rect(x, y, rw, rh))
                .collect(),
        }
    })
}
fn contains((x, y, w, h): Rect, ex: i32, ey: i32) -> bool {
    ex >= x && ey >= y && ex < x + w && ey < y + h
}
fn hit(w: &Widget, d: &Data, x: i32, y: i32) -> Option<Hit> {
    if !contains((w.x(), w.y(), w.w(), w.h()), x, y) {
        return None;
    }
    let g = geometry(w, d);
    if contains(g.close, x, y) {
        return Some(Hit::Close);
    }
    g.actions
        .iter()
        .position(|r| contains(*r, x, y))
        .map(Hit::Action)
}
fn draw(w: &Widget, d: &Data) {
    let g = geometry(w, d);
    theme::with(|t| {
        let n = &t.notifications;
        card(t, w.x(), w.y(), w.w(), w.h(), n, 2);
        let m = crate::layout::Mirror::of(w);
        let text = |s: &str, (x, y, rw, rh): Rect, bold: bool, color: Rgb, wrap: bool| {
            if wrap {
                paint::text(
                    s,
                    x,
                    y,
                    rw,
                    rh,
                    t.font.title,
                    bold,
                    color,
                    m.align(Align::Left) | Align::Top | Align::Wrap,
                );
            } else {
                paint::text_elided(
                    s,
                    x,
                    y,
                    rw,
                    rh,
                    t.font.title,
                    bold,
                    color,
                    m.align(Align::Left),
                );
            }
        };
        text(&d.summary.get(), g.summary, true, n.text, false);
        if !d.body.get().is_empty() {
            text(&d.body.get(), g.body, false, n.text.darker(1.15), true);
        }
        if !d.icon.is_empty() {
            let (x, y, rw, rh) = g.icon;
            paint::glyph(&d.icon, x, y, rw, rh, t.font.title, n.text);
        }
        for (h, r, label) in d
            .actions
            .iter()
            .enumerate()
            .map(|(i, a)| (Hit::Action(i), g.actions[i], a.label.get()))
            .chain(std::iter::once((Hit::Close, g.close, String::new())))
        {
            let (x, y, rw, rh) = r;
            let active = enabled(d, h);
            let under = n.fill(bg(t));
            let foreground = if active { n.text } else { n.text.darker(2.0) };
            paint::control(
                t,
                x,
                y,
                rw,
                rh,
                paint::Interaction {
                    hover: active && d.hover.get() == Some(h),
                    pressed: active && d.pressed.get() == Some(h),
                    focus: active && d.cursor.get() == h && crate::focus::show_ring(w),
                    disabled: !active,
                    ..Default::default()
                },
                under,
                None,
                true,
            );
            if h == Hit::Close {
                paint::glyph(glyph::CROSS, x, y, rw, rh, t.font.body, foreground);
            } else {
                paint::text_elided(
                    &label,
                    x + 4,
                    y,
                    (rw - 8).max(0),
                    rh,
                    t.font.body,
                    false,
                    foreground,
                    Align::Center,
                );
            }
            if d.cursor.get() == h && crate::focus::show_ring(w) {
                paint::stroke(
                    x,
                    y,
                    rw,
                    rh,
                    t.radius,
                    1,
                    t.border(&t.controls.focus, under),
                );
            }
        }
    });
}
