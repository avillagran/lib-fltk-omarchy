//! Resizable split between panes of a toolkit row or column.
//!
//! A [`Split`] is a thin grip placed in a `Flex` next to the pane whose
//! size it controls (o3-code's sidebar width resizer and `T3ShelfResizer`).
//! Dragging it sets that pane's `fixed` size within a range; the other
//! children take the rest.
//!
//! ```no_run
//! use fltk::prelude::*;
//! use fltk_omarchy::{layout::{self, FlexExt}, split::Split, State};
//! # let _app = fltk_omarchy::App::new("dev.example.split");
//! let width = State::new(300);
//! let mut row = layout::row();
//! let sidebar = fltk::frame::Frame::default();
//! let _grip = Split::new(&mut row, &sidebar).range(260, 600).initial(300).bind(&width);
//! let content = fltk::frame::Frame::default();
//! row.add(&content);
//! row.end();
//! ```
//!
//! - Mouse: drag the grip (cursor ↔ or ↕); double-click resets to the
//!   initial size. Layout motion is off while dragging and on for the
//!   reset and key steps.
//! - Keyboard: the grip is a Tab stop. The arrow keys along the split move
//!   the grip on the screen (10 px, `step`), Home/End go to the minimum and
//!   maximum, Enter resets. The pane can sit on either side and the split
//!   works the same in right-to-left layouts: the grip follows the arrow.
//! - Screen readers: a splitter with its value, Increment/Decrement.

use crate::a11y::{self, Action, Role};
use crate::paint;
use crate::state::State;
use crate::theme;
use fltk::{app, enums::*, group::Flex, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::rc::Rc;

struct Inner {
    min: i32,
    max: i32,
    initial: i32,
    step: i32,
    thickness: i32,
    /// (pointer at press along the axis, size at press, animated before)
    drag: Option<(i32, i32, bool)>,
    hot: bool,
    /// Applying a value that came from the bound State.
    from_app: bool,
}

/// The grip of a resizable split. See the [module docs](self).
#[derive(Clone)]
pub struct Split {
    inner: Widget,
    flex: Flex,
    pane: Widget,
    size: State<i32>,
    st: Rc<RefCell<Inner>>,
}
fltk::widget_extends!(Split, Widget, inner);

fn is_column(f: &Flex) -> bool {
    f.get_type::<fltk::group::FlexType>() == fltk::group::FlexType::Column
}

impl Split {
    /// Adds a grip at the end of `flex` that sizes `pane` (a child of
    /// `flex`, on either side of the grip). Range 0..=10 000 and the pane's
    /// current size until [`Split::range`] / [`Split::initial`] say
    /// otherwise.
    pub fn new<W: WidgetExt>(flex: &mut Flex, pane: &W) -> Split {
        let column = is_column(flex);
        let thickness = theme::with(|t| (t.spacing.sm).max(6));
        let mut inner = Widget::default();
        inner.set_frame(FrameType::NoBox);
        inner.set_visible_focus();
        flex.add(&inner);
        flex.fixed(&inner, thickness);
        // SAFETY: a live widget; the copy only reads and resizes it.
        let pane = unsafe { Widget::from_widget_ptr(pane.as_widget_ptr()) };
        let current = if column { pane.h() } else { pane.w() };
        let st = Rc::new(RefCell::new(Inner {
            min: 0,
            max: 10_000,
            initial: current.max(0),
            step: 10,
            thickness,
            drag: None,
            hot: false,
            from_app: false,
        }));
        let size = State::new(current.max(0));
        let me = Split { inner, flex: flex.clone(), pane, size, st };
        me.install(column);
        me
    }

    /// Smallest and largest pane size in pixels.
    pub fn range(self, min: i32, max: i32) -> Self {
        {
            let mut s = self.st.borrow_mut();
            s.min = min.max(0);
            s.max = max.max(s.min);
        }
        let v = self.size.get();
        self.apply(v, false);
        self
    }

    /// Size used at start and restored by double-click or Enter.
    pub fn initial(self, px: i32) -> Self {
        self.st.borrow_mut().initial = px;
        self.apply(px, false);
        self
    }

    /// Pixels per arrow key (default 10).
    pub fn step(self, px: i32) -> Self {
        self.st.borrow_mut().step = px.max(1);
        self
    }

    /// The pane size, two-way: dragging or keys set it; setting it resizes
    /// the pane (persist it in the app's settings).
    pub fn bind(self, size: &State<i32>) -> Self {
        let me = self.clone();
        let theirs = size.clone();
        crate::state::bind_widget(&self.inner, size, true, move |v| {
            me.st.borrow_mut().from_app = true;
            me.apply(*v, true);
            me.st.borrow_mut().from_app = false;
            // Out of range: write the clamped size back once this
            // notification is over (a State cannot change in its own listener).
            let got = me.size.get();
            if got != *v {
                let (t, inner) = (theirs.clone(), me.inner.clone());
                fltk::app::add_timeout3(0.0, move |_| {
                    if !inner.was_deleted() {
                        t.set_if_changed(got);
                    }
                });
            }
        });
        let (me, theirs) = (self.clone(), size.clone());
        crate::state::bind_widget(&self.inner, &self.size, false, move |v| {
            if !me.st.borrow().from_app {
                theirs.set_if_changed(*v);
            }
        });
        self
    }

    /// The pane size now.
    pub fn size(&self) -> i32 {
        self.size.get()
    }

    /// Sets the pane size (clamped to the range); `animate` lets the
    /// layout motion slide the panes.
    pub fn set_size(&self, px: i32, animate: bool) {
        self.apply(px, animate);
    }

    fn clamp(&self, px: i32) -> i32 {
        let s = self.st.borrow();
        px.clamp(s.min, s.max)
    }

    fn apply(&self, px: i32, animate: bool) {
        let v = self.clamp(px);
        let mut flex = self.flex.clone();
        let was = crate::motion::animated(&flex);
        if !animate && was {
            crate::motion::set_animated(&flex, false);
        }
        flex.fixed(&self.pane, v);
        flex.layout();
        flex.redraw();
        if !animate && was {
            crate::motion::set_animated(&flex, true);
        }
        self.size.set_if_changed(v);
    }

    /// +1 when moving the grip toward the screen's end (right or down)
    /// grows the pane, −1 when it shrinks it.
    fn grow_sign(&self, column: bool) -> i32 {
        let (p, g) = (&self.pane, &self.inner);
        let before = if column { p.y() + p.h() / 2 < g.y() + g.h() / 2 } else { p.x() + p.w() / 2 < g.x() + g.w() / 2 };
        if before { 1 } else { -1 }
    }

    fn install(&self, column: bool) {
        let me = self.clone();
        self.inner.clone().handle(move |w, ev| {
            let along = || if column { app::event_y_root() } else { app::event_x_root() };
            match ev {
                Event::Enter | Event::Move => {
                    me.st.borrow_mut().hot = true;
                    if let Some(mut win) = w.window() {
                        win.set_cursor(if column { Cursor::NS } else { Cursor::WE });
                    }
                    w.redraw();
                    true
                }
                Event::Leave => {
                    let dragging = me.st.borrow().drag.is_some();
                    me.st.borrow_mut().hot = false;
                    if !dragging {
                        if let Some(mut win) = w.window() {
                            win.set_cursor(Cursor::Default);
                        }
                    }
                    w.redraw();
                    true
                }
                Event::Push if app::event_mouse_button() == app::MouseButton::Left => {
                    if app::event_clicks() {
                        let i = me.st.borrow().initial;
                        me.apply(i, true);
                        return true;
                    }
                    let was = crate::motion::animated(&me.flex);
                    me.st.borrow_mut().drag = Some((along(), me.size.get(), was));
                    if was {
                        crate::motion::set_animated(&me.flex, false);
                    }
                    w.redraw();
                    true
                }
                Event::Drag => {
                    let Some((start, size, _)) = me.st.borrow().drag else { return false };
                    let d = (along() - start) * me.grow_sign(column);
                    let v = me.clamp(size + d);
                    if v != me.size.get() {
                        let mut flex = me.flex.clone();
                        flex.fixed(&me.pane, v);
                        flex.layout();
                        flex.redraw();
                        me.size.set(v);
                    }
                    true
                }
                Event::Released => {
                    let Some((_, _, was)) = me.st.borrow_mut().drag.take() else { return false };
                    if was {
                        crate::motion::set_animated(&me.flex, true);
                    }
                    if !me.st.borrow().hot {
                        if let Some(mut win) = w.window() {
                            win.set_cursor(Cursor::Default);
                        }
                    }
                    w.redraw();
                    true
                }
                Event::KeyDown => {
                    // Screen directions: in a mirrored row the arrows still
                    // move the grip the way they point.
                    let key = app::event_key();
                    let screen = if column {
                        match key {
                            Key::Down => 1,
                            Key::Up => -1,
                            _ => 0,
                        }
                    } else {
                        match key {
                            Key::Right => 1,
                            Key::Left => -1,
                            _ => 0,
                        }
                    };
                    let (min, max, initial, step) = {
                        let s = me.st.borrow();
                        (s.min, s.max, s.initial, s.step)
                    };
                    let target = match key {
                        Key::Home => min,
                        Key::End => max,
                        Key::Enter | Key::KPEnter => initial,
                        _ if screen != 0 => me.size.get() + screen * step * me.grow_sign(column),
                        _ => return false,
                    };
                    me.apply(target, true);
                    true
                }
                Event::Focus | Event::Unfocus => {
                    w.redraw();
                    true
                }
                _ => false,
            }
        });
        let me = self.clone();
        self.inner.clone().draw(move |w| {
            let (hot, dragging, thickness) = {
                let s = me.st.borrow();
                (s.hot, s.drag.is_some(), s.thickness)
            };
            theme::with(|t| {
                let under = t.palette.background;
                let ring = crate::focus::show_ring(w);
                let (color, width) = if dragging || ring {
                    (t.palette.accent, 2)
                } else if hot {
                    (t.palette.foreground.over(under, 0.45), 2)
                } else {
                    (t.palette.foreground.over(under, 0.12), 1)
                };
                // A line along the split, centred in the grip.
                if column {
                    let y = w.y() + (thickness - width) / 2;
                    paint::fill(w.x(), y, w.w(), width, 0, color);
                } else {
                    let x = w.x() + (thickness - width) / 2;
                    paint::fill(x, w.y(), width, w.h(), 0, color);
                }
            });
        });
        let me = self.clone();
        a11y::describe(&self.inner, move |n| {
            let s = me.st.borrow();
            n.role(Role::Splitter)
                .numeric(me.size.get() as f64, s.min as f64, s.max as f64, s.step as f64)
                .focusable()
                .action(Action::Increment)
                .action(Action::Decrement)
                .action(Action::SetValue);
        });
        let me = self.clone();
        a11y::on_action(&self.inner, move |r| {
            let step = me.st.borrow().step;
            match r.action {
                Action::Increment => me.apply(me.size.get() + step, true),
                Action::Decrement => me.apply(me.size.get() - step, true),
                Action::SetValue => match r.value {
                    Some(a11y::Value::Number(x)) => me.apply(x.round() as i32, true),
                    _ => return false,
                },
                _ => return false,
            }
            true
        });
    }
}
