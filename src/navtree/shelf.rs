//! [`ShelfResizer`]: the grip above a sidebar shelf (o3's
//! `T3ShelfResizer`) that sets the height of the shelf below it.

use crate::a11y::{self, Action, Role};
use crate::state::State;
use crate::theme;
use fltk::{app, enums::*, group::Flex, prelude::*, widget::Widget};
use std::cell::Cell;
use std::rc::Rc;

/// Keyboard step of [`ShelfResizer`] (o3: 20 px).
const STEP: i32 = 20;

/// A thin grip above a shelf of a sidebar (o3's Working, Snoozed and
/// Settled shelves): dragging it up grows the shelf below, Up/Down
/// change it by 20 px, a double click restores the default. The height
/// lives in a `State<i32>` (persist it with the app's view state);
/// [`ShelfResizer::shelf`] keeps the shelf's height in its column equal to it.
///
/// Several shelves with their own heights are a column of [`NavTree`]s
/// (each with its own model, pinned), each below a
/// resizer; the last one usually takes the rest of the column:
///
/// ```no_run
/// use fltk::prelude::*;
/// use fltk_omarchy::{layout::{self, FlexExt}, model::ListModel, navtree::*, State};
/// let mut col = layout::column();
/// let selected = State::new(String::new());
/// let active = ListModel::new(|t: &NavItem| t.key.clone());
/// let settled = ListModel::new(|t: &NavItem| t.key.clone());
/// let tree = NavTree::new(&active, &selected); // takes the rest
/// let h = State::new(220);
/// let grip = ShelfResizer::new("Settled threads height", &h, 220);
/// let shelf = NavTree::new(&settled, &selected);
/// col.fit(&*grip);
/// let grip = grip.shelf(&*shelf);
/// col.end();
/// tree.set_pinned(true);
/// shelf.set_pinned(true);
/// ```
///
/// A single `NavTree` with sections (`NavItem::section(..).collapsible()`)
/// is the other way: one scroll, one Tab stop, sections fold, but no
/// per-section height. Use shelves when a section must keep its rows in
/// view while the main list scrolls (o3's Working and Settled shelves).
///
/// It is a Tab stop with the accessible role Slider (name: `name`, value
/// in pixels); the accent line shows while hovered, dragged or focused.
///
/// [`NavTree`]: super::NavTree
#[derive(Clone)]
pub struct ShelfResizer {
    inner: Widget,
    height: State<i32>,
    range: Rc<Cell<(i32, i32)>>,
}

impl std::ops::Deref for ShelfResizer {
    type Target = Widget;
    fn deref(&self) -> &Widget {
        &self.inner
    }
}

impl std::ops::DerefMut for ShelfResizer {
    fn deref_mut(&mut self) -> &mut Widget {
        &mut self.inner
    }
}

impl ShelfResizer {
    /// A grip for `height` (pixels, 60..=1200 by default); `default` is
    /// what a double click restores. `name` is the accessible name and
    /// tooltip (from the app's catalog).
    pub fn new(name: &str, height: &State<i32>, default: i32) -> ShelfResizer {
        let h = theme::with(|t| t.spacing.sm + 2);
        let mut inner = Widget::default().with_size(120, h);
        inner.set_frame(FrameType::NoBox);
        inner.set_visible_focus();
        inner.set_tooltip(name);
        let range = Rc::new(Cell::new((60, 1200)));
        let hover = Rc::new(Cell::new(false));
        let drag: Rc<Cell<Option<(i32, i32)>>> = Rc::default();
        let (hv, dr) = (hover.clone(), drag.clone());
        inner.draw(move |w| {
            let focused = crate::focus::is_focused(w);
            // The sidebar's surface, so the shelves read as one panel.
            theme::with(|t| crate::glass::fill(w.x(), w.y(), w.w(), w.h(), 0, super::rows::surface(t)));
            if !(hv.get() || dr.get().is_some() || focused) {
                return;
            }
            theme::with(|t| {
                let lw = (w.w() - 24).max(0);
                let x = w.x() + (w.w() - lw) / 2;
                let thick = 2;
                crate::paint::fill(x, w.y() + (w.h() - thick) / 2, lw, thick, 0, t.palette.accent);
            });
        });
        let (hs, rg) = (height.clone(), range.clone());
        let set = move |v: i32| {
            let (lo, hi) = rg.get();
            let v = v.clamp(lo, hi.max(lo));
            if hs.get() != v {
                hs.set(v);
            }
        };
        let set2 = set.clone();
        let hs = height.clone();
        inner.handle(move |w, ev| match ev {
            Event::Enter => {
                hover.set(true);
                if let Some(mut win) = w.window() {
                    win.set_cursor(Cursor::NS);
                }
                w.redraw();
                true
            }
            Event::Leave => {
                hover.set(false);
                if drag.get().is_none() {
                    if let Some(mut win) = w.window() {
                        win.set_cursor(Cursor::Default);
                    }
                }
                w.redraw();
                true
            }
            Event::Move => true,
            Event::Push => {
                if app::event_clicks() {
                    set(default);
                    drag.set(None);
                } else {
                    drag.set(Some((app::event_y_root(), hs.get())));
                    if let Some(p) = w.parent() {
                        if crate::layout::animated(&p) {
                            crate::layout::set_animated(&p, false);
                        }
                    }
                }
                w.redraw();
                true
            }
            Event::Drag => {
                if let Some((y0, v0)) = drag.get() {
                    set(v0 - (app::event_y_root() - y0));
                }
                true
            }
            Event::Released => {
                if drag.take().is_some() {
                    if let Some(p) = w.parent() {
                        crate::layout::set_animated(&p, true);
                    }
                }
                if !hover.get() {
                    if let Some(mut win) = w.window() {
                        win.set_cursor(Cursor::Default);
                    }
                }
                w.redraw();
                true
            }
            Event::Focus | Event::Unfocus => {
                w.redraw();
                true
            }
            Event::KeyDown => {
                let mods = app::event_state();
                if mods.intersects(Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta | Shortcut::Shift) {
                    return false;
                }
                match app::event_key() {
                    Key::Up => set(hs.get() + STEP),
                    Key::Down => set(hs.get() - STEP),
                    Key::Home => set(default),
                    _ => return false,
                }
                true
            }
            _ => false,
        });
        let mut w2 = inner.clone();
        crate::state::bind_widget(&inner, height, false, move |_| {
            w2.redraw();
            a11y::changed(&w2);
        });
        let (hs, rg, name) = (height.clone(), range.clone(), name.to_string());
        a11y::describe(&inner, move |n| {
            let (lo, hi) = rg.get();
            let v = hs.get();
            n.role(Role::Slider)
                .name(&name)
                .numeric(v as f64, lo as f64, hi as f64, STEP as f64)
                .value(&a11y::number_text(v as f64))
                .focusable()
                .action(Action::Increment)
                .action(Action::Decrement)
                .action(Action::SetValue);
        });
        let hs = height.clone();
        a11y::on_action(&inner, move |r| {
            match (r.action, &r.value) {
                (Action::Increment, _) => set2(hs.get() + STEP),
                (Action::Decrement, _) => set2(hs.get() - STEP),
                (Action::SetValue, Some(a11y::Value::Number(x))) => set2(x.round() as i32),
                _ => return false,
            }
            true
        });
        ShelfResizer {
            inner,
            height: height.clone(),
            range,
        }
    }

    /// Height limits (default 60..=1200).
    pub fn range(self, min: i32, max: i32) -> Self {
        self.range.set((min, max.max(min)));
        let v = self.height.get().clamp(min, max.max(min));
        if v != self.height.get() {
            self.height.set(v);
        }
        self
    }

    /// Keeps `shelf`'s height in its parent column equal to the height
    /// State (now and whenever it changes). Fit or add the shelf to its
    /// column first.
    pub fn shelf<W: WidgetExt>(self, shelf: &W) -> Self {
        // SAFETY: a live widget; the copy is only resized and compared.
        let w = unsafe { Widget::from_widget_ptr(shelf.as_widget_ptr()) };
        crate::state::bind_widget(&self.inner, &self.height, true, move |h| {
            if w.was_deleted() {
                return;
            }
            if !crate::layout::refit(&w, (w.w(), *h)) {
                if let Some(mut p) = w.parent().and_then(|p| Flex::from_dyn_widget(&p)) {
                    p.fixed(&w, *h);
                    p.layout();
                    p.redraw();
                }
            }
        });
        self
    }

    /// The height it sets.
    pub fn height(&self) -> State<i32> {
        self.height.clone()
    }
}
