//! The hover preview card of a [`NavTree`] row (o3's thread hover card):
//! it opens after the pointer rests on a row for
//! [`OPEN_MS`] (500 ms), stays while the pointer is on the row or on the
//! card, and closes [`CLOSE_MS`] (140 ms) after it left both. Scrolling,
//! a click, a key, a context menu or the row going away close it at once.
//! The card is not modal: it takes neither focus nor clicks outside it.

use super::{window_origin, NavKind, NavTree};
use crate::a11y::{self, Role};
use crate::panel::ItemId;
use crate::theme;
use fltk::{app, enums::*, group::Flex, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::rc::Rc;

/// Hover time before the card opens (o3: 500 ms).
pub const OPEN_MS: u32 = 500;
/// Time the card stays after the pointer left the row and the card.
pub const CLOSE_MS: u32 = crate::anim::ms::HOVER_CLOSE;

pub(crate) type BuildFn = dyn FnMut(&str, &mut Flex) -> bool;

/// Preview state kept in the tree.
#[derive(Default)]
pub(crate) struct Pv {
    /// The row under the pointer that would get the card.
    pub want: Option<ItemId>,
    pub gen: u64,
    /// The open card: its row and layer.
    pub open: Option<(ItemId, Widget)>,
    pub card_hovered: bool,
    pub width: i32,
    pub build: Option<Rc<RefCell<Box<BuildFn>>>>,
}

impl NavTree {
    /// A card with app-built content next to a row the pointer rests on
    /// (500 ms; o3's thread hover card): `build(key, column)` fills the
    /// card's column like a [`PopupCard`](crate::popups::PopupCard)
    /// (`col.fit(&*Label::new(..))`) and returns false for rows without
    /// a preview. The card is `width` wide and as tall as its content; it
    /// stays while the pointer is on the row or on the card, and closes
    /// 140 ms after it left both, or at once on scroll, click, key or
    /// context menu. It never takes focus: keyboard users get the same
    /// information from the row's context menu or its accessible name.
    pub fn preview(self, width: i32, build: impl FnMut(&str, &mut Flex) -> bool + 'static) -> Self {
        {
            let mut s = self.st.borrow_mut();
            s.pv.width = width.max(80);
            s.pv.build = Some(Rc::new(RefCell::new(Box::new(build))));
        }
        self
    }

    /// The key of the row whose preview card is open.
    pub fn preview_key(&self) -> Option<String> {
        let id = self.preview_open()?;
        self.key_of(id)
    }

    /// The open preview card's rectangle (window coordinates).
    pub fn preview_rect(&self) -> Option<(i32, i32, i32, i32)> {
        let s = self.st.borrow();
        let (_, l) = s.pv.open.as_ref()?;
        let (ox, oy) = window_origin(&self.canvas);
        Some((l.x() - ox, l.y() - oy, l.w(), l.h()))
    }

    /// Opens the preview of `key` now (without the hover delay).
    pub fn show_preview(&self, key: &str) {
        if let Some(id) = self.model.id_of(&key.to_string()) {
            self.open_preview(id);
        }
    }

    /// Closes the preview card.
    pub fn hide_preview(&self) {
        let open = {
            let mut s = self.st.borrow_mut();
            s.pv.gen += 1;
            s.pv.want = None;
            s.pv.card_hovered = false;
            s.pv.open.take()
        };
        if let Some((_, l)) = open {
            close_later(l);
        }
    }

    pub(crate) fn preview_open(&self) -> Option<ItemId> {
        let s = self.st.borrow();
        s.pv.open.as_ref().filter(|(_, l)| !l.was_deleted()).map(|(id, _)| *id)
    }

    /// The pointer is on `row` (None: on no row, or on a row's action).
    pub(crate) fn preview_hover(&self, row: Option<ItemId>) {
        if self.st.borrow().pv.build.is_none() {
            return;
        }
        let gen = {
            let mut s = self.st.borrow_mut();
            if s.pv.want == row {
                return;
            }
            s.pv.want = row;
            s.pv.gen += 1;
            s.pv.gen
        };
        let open = self.preview_open();
        if open.is_some() && open != row {
            self.schedule_close(gen);
        }
        let Some(id) = row else { return };
        if open == Some(id) {
            return;
        }
        let me = self.clone();
        app::add_timeout3(OPEN_MS as f64 / 1000.0, move |_| {
            if me.canvas.was_deleted() {
                return;
            }
            let go = {
                let s = me.st.borrow();
                s.pv.gen == gen && s.pv.want == Some(id) && s.drag.is_none() && s.resize.is_none()
            };
            if go && me.is_expanded() && !crate::overlay::modal_open_over(&me.canvas) {
                me.open_preview(id);
            }
        });
    }

    fn schedule_close(&self, gen: u64) {
        let me = self.clone();
        app::add_timeout3(CLOSE_MS as f64 / 1000.0, move |_| {
            if me.canvas.was_deleted() {
                return;
            }
            let stay = {
                let s = me.st.borrow();
                let open = s.pv.open.as_ref().map(|(id, _)| *id);
                s.pv.gen != gen || s.pv.card_hovered || (open.is_some() && s.pv.want == open)
            };
            if !stay {
                me.close_preview_now();
            }
        });
    }

    /// Closes the card without touching the hover intent.
    pub(crate) fn close_preview_now(&self) {
        let open = self.st.borrow_mut().pv.open.take();
        self.st.borrow_mut().pv.card_hovered = false;
        if let Some((_, l)) = open {
            close_later(l);
        }
    }

    /// Scroll, click, key: closes it and forgets the hovered row until
    /// the pointer moves to another one.
    pub(crate) fn dismiss_preview(&self) {
        {
            let mut s = self.st.borrow_mut();
            s.pv.gen += 1;
        }
        self.close_preview_now();
    }

    fn open_preview(&self, id: ItemId) {
        let Some(build) = self.st.borrow().pv.build.clone() else { return };
        let Some(it) = self.item(id) else { return };
        if it.kind != NavKind::Row || it.disabled {
            return;
        }
        let Some(rect) = self.item_rect(&it.key) else { return };
        if self.preview_open() == Some(id) {
            return;
        }
        self.close_preview_now();
        let Some(win) = self.canvas.top_window() else { return };
        let (ox, oy) = window_origin(&self.canvas);
        let (rx, ry, rw, _) = rect;
        let (rx, ry) = (rx + ox, ry + oy);
        let w = self.st.borrow().pv.width.min(win.w() - 8).max(40);
        let (gap, pad, spacing) = theme::with(|t| (t.spacing.sm, t.spacing.popup_padding, t.spacing.md));
        // Beside the sidebar on its content side (o3: right of the row).
        let rs = self.right_side();
        let x = if rs { rx - gap - w } else { rx + rw + gap };
        let x = x.clamp(2, (win.w() - w - 2).max(2));
        let Some(mut g) = crate::overlay::open_group_layer(&self.canvas, (x, ry, w, 40)) else {
            return;
        };
        let prev = fltk::group::Group::try_current();
        g.begin();
        let mut col = Flex::new(x + pad, ry + pad, w - 2 * pad, 40, None).column();
        col.set_spacing(spacing);
        let keep = (build.borrow_mut())(&it.key, &mut col);
        col.end();
        g.end();
        fltk::group::Group::set_current(None::<&fltk::group::Group>);
        if let Some(p) = prev {
            p.begin();
        }
        let layer = g.as_base_widget();
        if !keep || col.children() == 0 {
            close_later(layer);
            return;
        }
        let kids: Vec<i32> = (0..col.children())
            .filter_map(|i| col.child(i))
            .filter(|c| c.visible())
            .map(|c| c.h())
            .collect();
        let inner = kids.iter().sum::<i32>() + spacing * (kids.len() as i32 - 1).max(0);
        let h = (inner + 2 * pad).min(win.h() - 4).max(1);
        let y = ry.min(win.h() - h - 2).max(2);
        g.resize(x, y, w, h);
        col.resize(x + pad, y + pad, w - 2 * pad, h - 2 * pad);
        col.layout();
        let lw = layer.clone();
        g.draw(move |g| {
            let p = crate::overlay::presence(&lw);
            if p <= 0.0 {
                return;
            }
            theme::with(|t| {
                crate::overlay::card(t, g.x(), g.y(), g.w(), g.h(), &t.popups, t.controls.normal.border_width.max(1));
            });
        });
        g.super_draw_first(false);
        let me = self.clone();
        g.handle(move |_, ev| match ev {
            Event::Enter => {
                me.st.borrow_mut().pv.card_hovered = true;
                false
            }
            Event::Leave => {
                let gen = {
                    let mut s = me.st.borrow_mut();
                    s.pv.card_hovered = false;
                    s.pv.gen += 1;
                    s.pv.gen
                };
                me.schedule_close(gen);
                false
            }
            Event::MouseWheel => {
                me.dismiss_preview();
                false
            }
            _ => false,
        });
        let title = it.title.clone();
        a11y::describe(&layer, move |n| {
            n.role(Role::Group).name(&title);
        });
        {
            let mut s = self.st.borrow_mut();
            s.pv.open = Some((id, layer.clone()));
            s.pv.card_hovered = false;
        }
        g.redraw();
    }
}

/// Closes a card on the next loop turn: it may be closed from inside an
/// event its window is still dispatching to its children (a wheel on the
/// rows), which must not see the child list change under it.
fn close_later(l: Widget) {
    app::add_timeout3(0.0, move |_| crate::overlay::close_layer(&l));
}
