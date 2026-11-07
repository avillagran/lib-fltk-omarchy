//! Lazy modal content with a bounded, closed-only retention period.
use super::*;
use crate::{
    layout::FlexExt,
    widgets::{ActionButton, Label},
};
use fltk::group::{Flex, Group};
use std::{
    rc::Weak,
    time::{Duration, Instant},
};

type Builder = dyn FnMut(&mut Flex, WeakSheet);
type DismissCallback = dyn FnMut(SheetDismiss);
type LayoutStamp = (i32, i32, u64, bool);
#[derive(Clone, Copy, PartialEq, Eq)]
enum OwnerVisibility {
    Normal,
    Hiding,
    Hidden,
}

/// Why a sheet stopped accepting input. Dismissal runs once per opening,
/// before the fade finishes; retained content may still exist afterwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetDismiss {
    Closed,
    Escape,
    ClickOutside,
    CloseButton,
    OwnerHidden,
}

/// A lazy, reusable modal sheet owned by one window. Widgets are built on
/// first `open`, kept hidden for 30 seconds after closing, then deleted.
/// Keep drafts and presentation choices in app-owned `State`s captured by
/// the builder so reconstruction preserves them. Dropping the final sheet
/// handle disposes its widgets; callbacks should capture `downgrade()`.
#[derive(Clone)]
pub struct Sheet {
    inner: Rc<Inner>,
}

/// A non-owning sheet reference for buttons inside its content.
#[derive(Clone)]
pub struct WeakSheet {
    inner: Weak<Inner>,
}
impl WeakSheet {
    pub fn upgrade(&self) -> Option<Sheet> {
        self.inner.upgrade().map(|inner| Sheet { inner })
    }
}
struct Inner {
    owner: Widget,
    size: (i32, i32),
    title: State<String>,
    title_binding: RefCell<Option<crate::Subscription>>,
    user_dismissible: Cell<bool>,
    dismissible_revision: Cell<u64>,
    dismissible_binding: RefCell<Option<(State<bool>, crate::Subscription)>>,
    builder: RefCell<Box<Builder>>,
    dismiss: RefCell<Option<Box<DismissCallback>>>,
    content: RefCell<Option<Content>>,
    building: Cell<bool>,
    opened: Cell<bool>,
    owner_visibility: Cell<OwnerVisibility>,
    owner_mapped: Cell<bool>,
    revision: Cell<u64>,
    fade_end: Cell<Option<Instant>>,
    closed_at: Cell<Option<Instant>>,
    retention: Cell<Option<Duration>>,
}
#[derive(Clone)]
struct Content {
    layer: Group,
    panel: Flex,
    header: Flex,
    close: ActionButton,
    body: Flex,
    layout_stamp: Rc<Cell<Option<LayoutStamp>>>,
}
thread_local! {
    // Only sheets waiting to hide or release content. Weak entries never
    // keep an owner alive. One native timer, no per-close closure allocation.
    static WAITING: RefCell<Vec<Weak<Inner>>> = const { RefCell::new(Vec::new()) };
    static RETIRING: RefCell<Vec<Widget>> = const { RefCell::new(Vec::new()) };
    static CHECK: Cell<bool> = const { Cell::new(false) };
    static POLLING: Cell<bool> = const { Cell::new(false) };
}

impl Sheet {
    /// `owner` must be a live widget in the owning top-level window. The
    /// builder receives the open content column and a weak sheet handle on
    /// each cold open. Move the weak handle into content callbacks to close
    /// the sheet without keeping its content alive through a cycle.
    /// No native widgets or timers are allocated until the first opening.
    pub fn new<A: WidgetExt>(
        owner: &A,
        title: &str,
        width: i32,
        height: i32,
        build: impl FnMut(&mut Flex, WeakSheet) + 'static,
    ) -> Self {
        assert!(app::is_ui_thread(), "Sheet requires the FLTK UI thread");
        let window = owner.top_window().expect("Sheet owner needs a window");
        // SAFETY: the owner is a live FLTK window; this is a deletion-tracked handle.
        let owner = unsafe { Widget::from_widget_ptr(window.as_widget_ptr()) };
        Self {
            inner: Rc::new(Inner {
                owner,
                size: (width.max(1), height.max(1)),
                title: State::new(title.into()),
                title_binding: RefCell::new(None),
                user_dismissible: Cell::new(true),
                dismissible_revision: Cell::new(0),
                dismissible_binding: RefCell::new(None),
                builder: RefCell::new(Box::new(build)),
                dismiss: RefCell::new(None),
                content: RefCell::new(None),
                building: Cell::new(false),
                opened: Cell::new(false),
                owner_visibility: Cell::new(OwnerVisibility::Normal),
                owner_mapped: Cell::new(false),
                revision: Cell::new(0),
                fade_end: Cell::new(None),
                closed_at: Cell::new(None),
                retention: Cell::new(Some(Duration::from_secs(30))),
            }),
        }
    }
    pub fn downgrade(&self) -> WeakSheet {
        WeakSheet {
            inner: Rc::downgrade(&self.inner),
        }
    }
    /// Live translated dialog/header title. The State survives content release.
    pub fn bind_title(self, title: &State<String>) -> Self {
        self.inner.title.set(title.get());
        let target = self.inner.title.clone();
        *self.inner.title_binding.borrow_mut() =
            Some(title.subscribe(move |v| target.set(v.clone())));
        self
    }
    /// Live UI-owned permission for Escape, outside-click and Close actions.
    /// False disables the close control. A rejected gesture leaves input,
    /// focus, motion and retention untouched and emits no dismissal callback.
    /// Owner `close()`, owner hiding/deletion and final handle drop bypass it.
    /// Bind `busy.map(|busy| !busy)` or a permission State updated by the app.
    pub fn bind_dismissible(self, allowed: &State<bool>) -> Self {
        let revision = self
            .inner
            .dismissible_revision
            .get()
            .checked_add(1)
            .expect("Sheet permission identities exhausted");
        self.inner.dismissible_revision.set(revision);
        let weak = Rc::downgrade(&self.inner);
        let next = allowed.subscribe(move |allowed| {
            if let Some(i) = weak.upgrade() {
                if i.dismissible_revision.get() == revision {
                    set_dismissible(&i, *allowed);
                }
            }
        });
        // Install before native activation changes can notify focus listeners.
        // No subscription/content borrow spans those application callbacks.
        let previous = self
            .inner
            .dismissible_binding
            .replace(Some((allowed.clone(), next)));
        drop(previous);
        if self.inner.dismissible_revision.get() == revision {
            set_dismissible(&self.inner, allowed.get());
        }
        self
    }
    /// Current permission for user dismissal, independent of open/built state.
    /// This does not restrict explicit owner lifecycle operations.
    pub fn is_dismissible(&self) -> bool {
        self.inner.user_dismissible.get()
    }
    /// Closed-only retention (default 30 seconds). Zero releases as soon as
    /// the closing fade finishes. Open content never expires.
    pub fn release_after(self, delay: Duration) -> Self {
        self.inner.retention.set(Some(delay));
        track(&self.inner);
        self
    }
    /// Keep stateful forms mounted until explicit `release` or handle drop.
    pub fn keep_content(self) -> Self {
        self.inner.retention.set(None);
        track(&self.inner);
        self
    }
    pub fn on_dismiss(self, f: impl FnMut(SheetDismiss) + 'static) -> Self {
        *self.inner.dismiss.borrow_mut() = Some(Box::new(f));
        self
    }
    pub fn is_open(&self) -> bool {
        !self.inner.owner.was_deleted() && self.inner.opened.get()
    }
    /// Includes open, fading and cached content; a deleted owner is unbuilt.
    pub fn is_built(&self) -> bool {
        self.inner
            .content
            .borrow()
            .as_ref()
            .is_some_and(|c| !c.layer.was_deleted())
    }
    /// Card rectangle in owning-window coordinates, including live resize.
    pub fn rect(&self) -> Option<(i32, i32, i32, i32)> {
        let c = self.inner.content.borrow().clone()?;
        if c.layer.was_deleted() {
            return None;
        }
        layout(&self.inner, &c);
        Some((c.panel.x(), c.panel.y(), c.panel.w(), c.panel.h()))
    }
    pub fn close_rect(&self) -> Option<(i32, i32, i32, i32)> {
        let c = self.inner.content.borrow().clone()?;
        if c.layer.was_deleted() {
            return None;
        }
        layout(&self.inner, &c);
        Some((c.close.x(), c.close.y(), c.close.w(), c.close.h()))
    }
    /// Opens or reverses a closing fade without rebuilding retained content.
    /// Returns false for a deleted owner or a recursive call from its builder.
    pub fn open(&self) -> bool {
        let i = &self.inner;
        if i.owner.was_deleted()
            || i.building.get()
            || i.owner_visibility.get() == OwnerVisibility::Hiding
        {
            return false;
        }
        if i.owner_visibility.get() == OwnerVisibility::Hidden {
            if !i.owner.as_window().is_some_and(|window| window.shown()) {
                return false;
            }
            i.owner_visibility.set(OwnerVisibility::Normal);
        }
        if i.owner.as_window().is_some_and(|window| window.shown()) {
            i.owner_mapped.set(true);
        }
        if i.opened.replace(true) {
            return true;
        }
        i.revision.set(i.revision.get().wrapping_add(1));
        if !self.is_built() {
            build(i);
        }
        let Some(mut c) = i.content.borrow().clone() else {
            i.opened.set(false);
            return false;
        };
        if !i.opened.get() {
            hidden(i);
            track(i);
            return false;
        } // builder dismissed it
        let phase = if c.layer.visible() {
            presence(&c.layer.as_base_widget())
        } else {
            0.0
        };
        i.fade_end.set(None);
        i.closed_at.set(None);
        forget_layer(&c.layer.as_base_widget());
        let previous = crate::focus::current();
        // A cached sheet returns above siblings created since it closed.
        if let Some(mut parent) = c.layer.parent() {
            parent.remove(&c.layer);
            parent.add(&c.layer);
        }
        layout(i, &c);
        c.layer.activate();
        c.layer.show();
        register_layer(&c.layer.as_base_widget(), true);
        fade_layer(&c.layer.as_base_widget(), phase, 1.0);
        if let Some(p) = previous {
            crate::focus::remember_for(&c.layer.as_base_widget(), p);
        }
        crate::focus::push_modal(&c.layer);
        if crate::focus::first_in(&c.body).is_none() && crate::focus::first(&c.layer).is_none() {
            app::set_focus(&c.layer);
        }
        c.layer.redraw();
        redraw_owner(i);
        track(i);
        true
    }
    /// Owner dismissal: bypasses `bind_dismissible`, stops input/restores focus,
    /// then fades and starts retention. Use after success/ACK or explicit abort.
    pub fn close(&self) -> bool {
        dismiss(&self.inner, SheetDismiss::Closed)
    }
    /// Deletes only hidden, closed content. Returns false while open/fading.
    pub fn release(&self) -> bool {
        // Native groups notify children before this layer's Hide handler.
        // A child may explicitly release here while the owner is unmapping.
        // Settle ownership first without remapping the hidden owner for focus.
        if self.inner.opened.get()
            && self.inner.owner_mapped.get()
            && !self.inner.owner.was_deleted()
            && !self.inner.owner.visible()
        {
            owner_hidden(&self.inner);
        }
        if self.inner.opened.get()
            || self.inner.fade_end.get().is_some()
            || self.inner.building.get()
            || self
                .inner
                .content
                .borrow()
                .as_ref()
                .is_some_and(|c| !c.layer.was_deleted() && c.layer.visible())
        {
            return false;
        }
        let released = dispose(&self.inner);
        track(&self.inner);
        released
    }
}

fn build(i: &Rc<Inner>) {
    struct Restore<'a> {
        inner: &'a Inner,
        current: Option<Group>,
    }
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            self.inner.building.set(false);
            Group::set_current(self.current.as_ref());
        }
    }
    i.building.set(true);
    let _restore = Restore {
        inner: i,
        current: Group::try_current(),
    };
    Group::set_current(None::<&Group>);
    let mut layer = Group::new(0, 0, i.owner.w(), i.owner.h(), None);
    layer.hide();
    layer.set_frame(FrameType::NoBox);
    let mut panel = Flex::default().column();
    panel.set_frame(FrameType::NoBox);
    panel.set_clip_children(true);
    let mut header = crate::layout::row();
    let title = Label::new("").bind(&i.title);
    header.add(&*title);
    let weak = Rc::downgrade(i);
    let close = ActionButton::new(glyph::CLOSE, "")
        .bind_tooltip(&crate::i18n::text("Close"))
        .on_click(move || {
            if let Some(i) = weak.upgrade() {
                dismiss_user(&i, SheetDismiss::CloseButton);
            }
        });
    header.fit(&*close);
    header.end();
    let mut body = crate::layout::column();
    (i.builder.borrow_mut())(
        &mut body,
        WeakSheet {
            inner: Rc::downgrade(i),
        },
    );
    body.end();
    panel.end();
    layer.end();
    i.owner.as_group().expect("live window group").add(&layer);
    panel.super_draw(false);
    panel.draw(|p| {
        theme::with(|t| {
            card(
                t,
                p.x(),
                p.y(),
                p.w(),
                p.h(),
                &t.popups,
                t.controls.normal.border_width.max(1),
            )
        });
        p.draw_children();
    });
    layer.super_draw(false);
    let weak = Rc::downgrade(i);
    layer.draw(move |l| {
        let Some(i) = weak.upgrade() else { return };
        let content = i.content.borrow().clone();
        if let Some(c) = content {
            layout(&i, &c);
        }
        paint_layer(l, &mut |l: &mut Group| {
            theme::with(|t| draw_scrim(&l.as_base_widget(), bg(t), 0.7));
            l.draw_children();
        });
    });
    let weak = Rc::downgrade(i);
    let outside = Cell::new(false);
    layer.handle(move |l, event| {
        let Some(i) = weak.upgrade() else {
            return false;
        };
        if event == Event::Show && i.owner.as_window().is_some_and(|window| window.shown()) {
            i.owner_mapped.set(true);
        }
        if event == Event::Hide && i.opened.get() {
            owner_hidden(&i);
            return false;
        }
        if !i.opened.get() {
            return false;
        }
        let inside = i
            .content
            .borrow()
            .as_ref()
            .is_some_and(|c| app::event_inside_widget(&c.panel));
        match event {
            Event::Focus => true,
            Event::KeyDown | Event::Shortcut if app::event_key() == Key::Escape => {
                dismiss_user(&i, SheetDismiss::Escape);
                true
            }
            Event::Push => {
                outside.set(
                    !inside
                        && i.user_dismissible.get()
                        && app::event_mouse_button() == app::MouseButton::Left,
                );
                true // FLTK already offered the event to the card children.
            }
            Event::Released if outside.replace(false) => {
                if !inside && app::event_mouse_button() == app::MouseButton::Left {
                    dismiss_user(&i, SheetDismiss::ClickOutside);
                }
                true
            }
            Event::MouseWheel | Event::Drag | Event::Move | Event::Released => true,
            _ => {
                let _ = l;
                false
            }
        }
    });
    let weak = Rc::downgrade(i);
    a11y::describe(&layer, move |n| {
        if let Some(i) = weak.upgrade() {
            n.role(Role::Dialog).modal(true).name(&i.title.get());
        }
    });
    *i.content.borrow_mut() = Some(Content {
        layer,
        panel,
        header,
        close,
        body,
        layout_stamp: Rc::new(Cell::new(None)),
    });
    set_dismissible(i, i.user_dismissible.get());
}
fn set_dismissible(i: &Inner, allowed: bool) {
    i.user_dismissible.set(allowed);
    let content = i.content.borrow().clone();
    if let Some(mut c) = content.filter(|c| !c.layer.was_deleted()) {
        if c.close.active() != allowed {
            if allowed {
                c.close.activate();
            } else {
                c.close.deactivate();
            }
            c.close.redraw();
        }
    }
}
fn dismiss_user(i: &Rc<Inner>, reason: SheetDismiss) -> bool {
    // Permission is a plain UI-owned value: no app/model borrow or callback
    // is needed before the first lifecycle/focus/animation mutation.
    i.user_dismissible.get() && dismiss(i, reason)
}
fn owner_hidden(i: &Rc<Inner>) {
    struct Hidden<'a>(&'a Cell<OwnerVisibility>);
    impl Drop for Hidden<'_> {
        fn drop(&mut self) {
            self.0.set(OwnerVisibility::Hidden);
        }
    }
    i.owner_visibility.set(OwnerVisibility::Hiding);
    let _hidden = Hidden(&i.owner_visibility);
    dismiss(i, SheetDismiss::OwnerHidden);
}
fn layout(i: &Inner, c: &Content) {
    if i.owner.was_deleted() || c.layer.was_deleted() {
        return;
    }
    let (ww, wh) = (i.owner.w(), i.owner.h());
    let stamp = (ww, wh, theme::generation(), crate::layout::is_rtl(&c.layer));
    if c.layout_stamp.replace(Some(stamp)) == Some(stamp) {
        return;
    }
    let (pad, gap, hh) = theme::with(|t| {
        (
            t.spacing.popup_padding,
            t.spacing.md,
            t.spacing.control_height,
        )
    });
    let margin = pad.min(ww.min(wh).max(0) / 4);
    let (w, h) = (
        i.size.0.min((ww - 2 * margin).max(1)),
        i.size.1.min((wh - 2 * margin).max(1)),
    );
    let rect = ((ww - w) / 2, (wh - h) / 2, w, h);
    if (c.layer.w(), c.layer.h()) != (ww, wh) {
        c.layer.clone().resize(0, 0, ww, wh);
    }
    let mut panel = c.panel.clone();
    panel.set_margin(pad.min(w.min(h).max(0) / 4));
    panel.set_spacing(gap);
    panel.fixed(&c.header, hh.min(h));
    if (panel.x(), panel.y(), panel.w(), panel.h()) != rect {
        panel.resize(rect.0, rect.1, rect.2, rect.3);
    }
    panel.layout();
}
fn redraw_owner(i: &Inner) {
    if !i.owner.was_deleted() {
        i.owner.clone().redraw();
    }
}
fn dismiss(i: &Rc<Inner>, reason: SheetDismiss) -> bool {
    if !i.opened.replace(false) {
        return false;
    }
    let revision = i.revision.get().wrapping_add(1);
    i.revision.set(revision);
    let content = i.content.borrow().clone();
    if let Some(mut c) = content.filter(|c| !c.layer.was_deleted()) {
        let lw = c.layer.as_base_widget();
        let phase = presence(&lw);
        crate::focus::pop_modal(&c.layer);
        if reason == SheetDismiss::OwnerHidden {
            crate::focus::forget_return(&lw);
        } else {
            crate::focus::give_back(&lw, (!i.owner.was_deleted()).then(|| i.owner.clone()));
        }
        // Focus listeners may synchronously reopen or dismiss this sheet.
        // An older close must not retire the replacement modal registration.
        if i.revision.get() == revision {
            c.layer.deactivate();
            LAYERS.with(|ls| {
                ls.borrow_mut()
                    .retain(|(w, _)| alive_ptr(w) != alive_ptr(&lw))
            });
            let delay = fade_layer(&lw, phase, 0.0);
            i.fade_end.set(Some(crate::anim::now() + delay));
            if delay.is_zero() || !c.layer.visible() || reason == SheetDismiss::OwnerHidden {
                hidden(i);
            } else {
                c.layer.redraw();
                redraw_owner(i);
            }
        }
    }
    track(i);
    let callback = i.dismiss.borrow_mut().take();
    if let Some(mut cb) = callback {
        cb(reason);
        if i.dismiss.borrow().is_none() {
            *i.dismiss.borrow_mut() = Some(cb);
        }
    }
    true
}
fn hidden(i: &Rc<Inner>) {
    i.fade_end.set(None);
    i.closed_at.set(Some(crate::anim::now()));
    let content = i.content.borrow().clone();
    if let Some(mut c) = content.filter(|c| !c.layer.was_deleted()) {
        forget_layer(&c.layer.as_base_widget());
        c.layer.hide();
        redraw_owner(i);
    }
}
fn dispose(i: &Inner) -> bool {
    i.closed_at.set(None);
    i.fade_end.set(None);
    let Some(mut c) = i.content.borrow_mut().take() else {
        return false;
    };
    if !c.layer.was_deleted() {
        let lw = c.layer.as_base_widget();
        forget_layer(&lw);
        crate::focus::pop_modal(&c.layer);
        // A child can release from Hide while FLTK walks the parent's
        // cached child array. Defer removal as well as destruction. A
        // deletion-tracked handle also handles an owner deleted meanwhile.
        c.layer.hide();
        RETIRING.with(|r| r.borrow_mut().push(lw));
        install_check();
        arm();
    }
    true
}
impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(c) = self
            .content
            .borrow()
            .as_ref()
            .filter(|c| !c.layer.was_deleted())
        {
            crate::focus::pop_modal(&c.layer);
            if self.owner_visibility.get() == OwnerVisibility::Normal {
                crate::focus::give_back(
                    &c.layer.as_base_widget(),
                    (!self.owner.was_deleted()).then(|| self.owner.clone()),
                );
            } else {
                crate::focus::forget_return(&c.layer.as_base_widget());
            }
        }
        dispose(self);
        if WAITING
            .try_with(|w| {
                w.borrow_mut()
                    .retain(|w| !std::ptr::eq(w.as_ptr(), self) && w.strong_count() > 0)
            })
            .is_ok()
        {
            arm();
        }
    }
}
fn deadline(i: &Inner) -> Option<Instant> {
    i.fade_end.get().or_else(|| {
        i.closed_at
            .get()
            .zip(i.retention.get())
            .map(|(at, d)| at + d)
    })
}
fn install_check() {
    if !CHECK.with(|c| c.replace(true)) {
        app::add_check(|_| poll());
    }
}
fn track(i: &Rc<Inner>) {
    if deadline(i).is_some() {
        install_check();
    }
    WAITING.with(|w| {
        let mut w = w.borrow_mut();
        w.retain(|x| x.strong_count() > 0 && x.as_ptr() != Rc::as_ptr(i));
        if deadline(i).is_some() {
            w.push(Rc::downgrade(i));
        }
    });
    arm();
}
fn poll() {
    if POLLING.with(|p| p.replace(true)) {
        return;
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            POLLING.with(|p| p.set(false));
        }
    }
    let _reset = Reset;
    let retired = RETIRING.with(|r| std::mem::take(&mut *r.borrow_mut()));
    for widget in retired {
        if !widget.was_deleted() {
            if let Some(mut parent) = widget.parent() {
                parent.remove(&widget);
                parent.redraw();
            }
            // SAFETY: detached live own widget, outside the callback that
            // retired it. FLTK defers final destruction to its event loop.
            unsafe { Fl_delete_widget(widget.as_widget_ptr().cast()) };
        }
    }
    let waiting = WAITING.with(|w| w.borrow().clone());
    for w in waiting {
        let Some(i) = w.upgrade() else { continue };
        if i.owner.was_deleted() {
            i.opened.set(false);
            dispose(&i);
            continue;
        }
        if i.fade_end
            .get()
            .is_some_and(|at| crate::anim::now() >= at || !crate::anim::enabled())
        {
            hidden(&i);
        }
        if !i.opened.get()
            && i.fade_end.get().is_none()
            && deadline(&i).is_some_and(|at| crate::anim::now() >= at)
        {
            dispose(&i);
        }
    }
    WAITING.with(|w| {
        w.borrow_mut()
            .retain(|w| w.upgrade().is_some_and(|i| deadline(&i).is_some()))
    });
    arm();
}
fn tick(_: *mut ()) {
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(poll));
}
fn arm() {
    app::remove_raw_timeout(tick, std::ptr::null_mut());
    if crate::anim::clock::frozen() {
        return;
    }
    let mut next = WAITING.with(|w| {
        w.borrow()
            .iter()
            .filter_map(|w| w.upgrade().and_then(|i| deadline(&i)))
            .min()
    });
    if RETIRING.with(|r| !r.borrow().is_empty()) {
        next = Some(crate::anim::now());
    }
    if let Some(at) = next {
        app::add_raw_timeout(
            at.saturating_duration_since(crate::anim::now())
                .as_secs_f64()
                .max(0.001),
            tick,
            std::ptr::null_mut(),
        );
    }
}
