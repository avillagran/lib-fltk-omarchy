//! Model-backed tab identities and app-owned requests.
use super::*;
use crate::model::ListModel;
use std::hash::Hash;
use std::ops::{Deref, DerefMut};

/// A tab context request, with coordinates in the containing window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabContext<K> {
    pub key: K,
    pub x: i32,
    pub y: i32,
}

/// The clickable part of a live tab (not its painted glyph bounds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabPart {
    Body,
    Close,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabHit<K> {
    pub key: K,
    pub part: TabPart,
}

/// Move `key` immediately before `before`, or to the end when None.
/// This is a request: it never selects a tab or mutates the app's model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabReorder<K> {
    pub key: K,
    pub before: Option<K>,
}
impl<K: Clone + Eq + Hash + 'static> TabReorder<K> {
    /// Apply against current model positions, preserving selection state.
    /// Missing IDs and a self-target are ignored, not treated as append.
    pub fn apply_to<T: Clone + PartialEq + 'static>(
        &self,
        model: &ListModel<K, T>,
    ) -> Option<usize> {
        let from = model.index_of_key(&self.key)?;
        let to = match &self.before {
            Some(key) if key == &self.key => return None,
            Some(key) => {
                let i = model.index_of_key(key)?;
                i - usize::from(from < i)
            }
            None => model.len().checked_sub(1)?,
        };
        model.move_key(&self.key, to)
    }
}

/// Keyed tab strip. Selection and close/reorder decisions remain app-owned.
/// Use `body.fit(&*tabs)` as with the index-based strip.
#[derive(Clone)]
pub struct KeyedTabBar<K: Clone + Eq + 'static> {
    inner: TabBar,
    selected: State<Option<K>>,
    key_at: Rc<dyn Fn(usize) -> Option<K>>,
    id_for: Rc<dyn Fn(&K) -> Option<u64>>,
}
impl<K: Clone + Eq + 'static> Deref for KeyedTabBar<K> {
    type Target = Widget;
    fn deref(&self) -> &Widget {
        &self.inner.inner
    }
}
impl<K: Clone + Eq + 'static> DerefMut for KeyedTabBar<K> {
    fn deref_mut(&mut self) -> &mut Widget {
        &mut self.inner.inner
    }
}

fn sync<K, T>(
    bar: &TabBar,
    model: &ListModel<K, T>,
    selected: &State<Option<K>>,
    title: &dyn Fn(&T) -> String,
) where
    K: Clone + Eq + Hash + 'static,
    T: Clone + PartialEq + 'static,
{
    let mut labels = Vec::with_capacity(model.len());
    let mut ids = Vec::with_capacity(model.len());
    for i in 0..model.len() {
        if let (Some(item), Some(id)) = (model.get(i), model.id_at(i)) {
            labels.push(title(&item));
            ids.push(id);
        }
    }
    *bar.tabs.borrow_mut() = labels;
    *bar.ids.borrow_mut() = ids;
    let index = selected
        .get()
        .as_ref()
        .and_then(|key| model.index_of_key(key))
        .unwrap_or(usize::MAX);
    bar.value.set(index);
    // Never change app selection when its key disappears.
    if bar
        .cursor
        .get()
        .is_some_and(|id| !bar.ids.borrow().contains(&id))
    {
        bar.cursor.set(None);
    }
    bar.geo.clamp(&bar.inner);
    bar.inner.clone().redraw();
}

impl TabBar {
    /// Follow granular model changes by stable ItemId, including title
    /// updates, moves and snapshot diffs. A missing selected key remains
    /// app-owned and has no highlight; no implicit replacement is chosen.
    pub fn from_model<K, T>(
        model: &ListModel<K, T>,
        selected: &State<Option<K>>,
        title: impl Fn(&T) -> String + 'static,
    ) -> KeyedTabBar<K>
    where
        K: Clone + Eq + Hash + 'static,
        T: Clone + PartialEq + 'static,
    {
        let value = State::new(usize::MAX);
        let bar = TabBar::new(&[], &value);
        let title: Rc<dyn Fn(&T) -> String> = Rc::new(title);
        sync(&bar, model, selected, &*title);
        let (ids, m) = (bar.ids.clone(), model.clone());
        let key_at: Rc<dyn Fn(usize) -> Option<K>> =
            Rc::new(move |i| ids.borrow().get(i).copied().and_then(|id| m.key_of(id)));
        let m = model.clone();
        let id_for: Rc<dyn Fn(&K) -> Option<u64>> =
            Rc::new(move |key| m.index_of_key(key).and_then(|i| m.id_at(i)));
        let (lookup, selection) = (key_at.clone(), selected.clone());
        *bar.select.borrow_mut() = Some(Box::new(move |i| {
            if let Some(key) = lookup(i) {
                selection.set(Some(key));
            }
        }));
        let (m, ids, v) = (model.clone(), bar.ids.clone(), bar.value.clone());
        crate::state::bind_widget(&bar.inner, selected, false, move |key| {
            let id = key
                .as_ref()
                .and_then(|key| m.index_of_key(key))
                .and_then(|i| m.id_at(i));
            let i = id
                .and_then(|id| ids.borrow().iter().position(|v| *v == id))
                .unwrap_or(usize::MAX);
            v.set(i);
        });
        // A model listener must not own a bar that in turn owns the model.
        let weak = Rc::downgrade(&bar.tabs);
        let index = bar.value.downgrade();
        let selection = selected.downgrade();
        let (mut widget, ids, cursor, geo, render) = (
            bar.inner.clone(),
            bar.ids.clone(),
            bar.cursor.clone(),
            bar.geo.clone(),
            title.clone(),
        );
        model.listen(move |m, _| {
            if widget.was_deleted() {
                return false;
            }
            let (Some(tabs), Some(value), Some(selected)) =
                (weak.upgrade(), index.upgrade(), selection.upgrade())
            else {
                return false;
            };
            let mut labels = Vec::with_capacity(m.len());
            let mut keys = Vec::with_capacity(m.len());
            for i in 0..m.len() {
                if let (Some(item), Some(id)) = (m.get(i), m.id_at(i)) {
                    labels.push(render(&item));
                    keys.push(id);
                }
            }
            *tabs.borrow_mut() = labels;
            *ids.borrow_mut() = keys;
            value.set(
                selected
                    .get()
                    .as_ref()
                    .and_then(|key| m.index_of_key(key))
                    .unwrap_or(usize::MAX),
            );
            if cursor.get().is_some_and(|id| !ids.borrow().contains(&id)) {
                cursor.set(None);
            }
            geo.clamp(&widget);
            widget.redraw();
            true
        });
        // Labels can call tr!; language changes re-evaluate them live.
        let (b, m, selection) = (bar.clone(), model.clone(), selected.downgrade());
        crate::i18n::bind(&bar.inner, move || {
            if let Some(s) = selection.upgrade() {
                sync(&b, &m, &s, &*title);
            }
        });
        KeyedTabBar {
            inner: bar,
            selected: selected.clone(),
            key_at,
            id_for,
        }
    }
}

impl<K: Clone + Eq + 'static> KeyedTabBar<K> {
    fn geometry_live(&self) -> bool {
        let w = &self.inner.inner;
        !w.was_deleted() && w.w() > 0 && w.h() > 0 && crate::focus::visible_in_window(w)
    }
    pub fn overflowing(&self) -> bool {
        self.inner.overflowing()
    }
    pub fn scroll_offset(&self) -> i32 {
        self.inner.scroll_offset()
    }
    pub fn selected(&self) -> Option<K> {
        self.selected.get()
    }
    pub fn select_key(&self, key: &K) -> bool {
        if self.inner.inner.was_deleted() {
            return false;
        }
        if (self.id_for)(key).is_none() {
            return false;
        }
        self.selected.set(Some(key.clone()));
        true
    }
    pub fn key_at(&self, index: usize) -> Option<K> {
        (self.key_at)(index)
    }
    /// Opaque ListModel ItemId; controllers should use the domain key K.
    pub fn id_for_key(&self, key: &K) -> Option<u64> {
        (self.id_for)(key)
    }
    /// Visible `(x,width)` in window coordinates; y/height are the bar's.
    pub fn rect_for_key(&self, key: &K) -> Option<(i32, i32)> {
        if !self.geometry_live() {
            return None;
        }
        let id = (self.id_for)(key)?;
        let i = self.inner.ids.borrow().iter().position(|v| *v == id)?;
        self.inner.tab_rect(i)
    }
    /// Visible close hit area `(x,y,width,height)` in window coordinates.
    /// Includes the clickable padding and respects scrolling/clipping/RTL.
    /// None when not closable, absent, disposed or completely scrolled out.
    pub fn close_rect_for_key(&self, key: &K) -> Option<(i32, i32, i32, i32)> {
        if !self.geometry_live() || !self.inner.closable.get() {
            return None;
        }
        let id = (self.id_for)(key)?;
        let i = self.inner.ids.borrow().iter().position(|v| *v == id)?;
        let w = &self.inner.inner;
        let g = &self.inner.geo;
        let (x, width) = *g.tabs_at(w).get(i)?;
        let (vx, vw) = g.view(w);
        let pad = theme::with(|t| t.spacing.control_padding_x + t.font.body);
        let a = (x + width - pad).max(vx);
        let b = (x + width).min(vx + vw);
        if b <= a {
            return None;
        }
        Some((
            crate::layout::Mirror::of(w).x(a, b - a),
            w.y(),
            b - a,
            w.h(),
        ))
    }
    /// Use the widget's actual hit testing, including close padding and
    /// RTL. Scroll arrows, gaps, stale keys and points outside return None.
    /// Middle-click routing uses the key regardless of which part is hit.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<TabHit<K>> {
        let w = &self.inner.inner;
        if !self.geometry_live()
            || x < w.x()
            || x >= w.x() + w.w()
            || y < w.y()
            || y >= w.y() + w.h()
        {
            return None;
        }
        match self.inner.geo.hit(w, x) {
            Hit::Tab(i, close) => Some(TabHit {
                key: (self.key_at)(i)?,
                part: if close && self.inner.closable.get() {
                    TabPart::Close
                } else {
                    TabPart::Body
                },
            }),
            _ => None,
        }
    }
    /// Move the keyboard cursor/focus without selecting or activating.
    pub fn focus_key(&self, key: &K) -> bool {
        if !self.geometry_live() {
            return false;
        }
        let Some(id) = (self.id_for)(key) else {
            return false;
        };
        let Some(i) = self.inner.ids.borrow().iter().position(|v| *v == id) else {
            return false;
        };
        self.inner.cursor.set(Some(id));
        self.inner.geo.reveal(&self.inner.inner, i);
        let mut w = self.inner.inner.clone();
        w.redraw();
        w.take_focus().is_ok()
    }
    pub fn focused_key(&self) -> Option<K> {
        let id = self.inner.cursor.get()?;
        let i = self.inner.ids.borrow().iter().position(|v| *v == id)?;
        (self.key_at)(i)
    }
    pub fn on_close(mut self, mut f: impl FnMut(K) + 'static) -> Self {
        let lookup = self.key_at.clone();
        self.inner = self.inner.on_close(move |i| {
            if let Some(key) = lookup(i) {
                f(key)
            }
        });
        self
    }
    pub fn request_close(&self, key: &K) {
        if let Some(id) = (self.id_for)(key) {
            let i = self.inner.ids.borrow().iter().position(|v| *v == id);
            if let Some(i) = i {
                self.inner.request_close(i);
            }
        }
    }
    pub fn on_context(mut self, mut f: impl FnMut(TabContext<K>) + 'static) -> Self {
        let lookup = self.key_at.clone();
        self.inner = self.inner.on_context(move |i, x, y| {
            if let Some(key) = lookup(i) {
                f(TabContext { key, x, y })
            }
        });
        self
    }
    /// Drag or Mod+Shift+Left/Right requests a stable insertion. Apply it
    /// in the app (`request.apply_to(&model)`) or forward it to a controller.
    /// Alt+Left/Right moves only the keyboard cursor; Enter activates it.
    pub fn reorderable(mut self, mut f: impl FnMut(TabReorder<K>) + 'static) -> Self {
        let lookup = self.key_at.clone();
        let ids = self.inner.ids.clone();
        self.inner = self.inner.reorderable(move |from, to| {
            let Some(key) = lookup(from) else { return };
            let remaining: Vec<_> = (0..ids.borrow().len()).filter(|i| *i != from).collect();
            let before = if let Some(&i) = remaining.get(to) {
                let Some(target) = lookup(i) else { return };
                Some(target)
            } else {
                None
            };
            f(TabReorder { key, before });
        });
        self
    }
}
