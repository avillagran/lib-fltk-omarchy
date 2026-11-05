//! `Sidebar`: a navigation rail that expands on hover, with a flat list of
//! items addressed by index. It is a [`NavTree`](crate::navtree::NavTree)
//! underneath (same drawing, motion, keys); use `NavTree` directly for
//! trees, search, several actions per row, custom rows or live models.
//!
//! Collapsed it is a rail of icons; resting the pointer on it (hover intent
//! [`anim::ms::HOVER_OPEN`](crate::anim::ms::HOVER_OPEN), 350 ms) expands
//! it to show the labels, and it collapses 140 ms after the pointer
//! leaves. Keyboard focus inside expands it at once; Esc collapses it.
//! Pinned, it stays expanded. Selected = accent bar on the start edge +
//! selected fill.
//!
//! - [`SidebarMode::Push`] (default): the rail's width changes in its row
//!   and the content beside it makes room. The width animates through the
//!   toolkit's automatic layout motion (190 ms OutCubic), like o3's
//!   sidebar reveal and Omarchy's tray drawer, which push too.
//! - [`SidebarMode::Overlay`]: the rail keeps its width; the expanded
//!   panel slides over the content (a non-modal layer).
//!
//! Right to left, the rail is on the right (its row mirrors), the accent
//! bar on its right edge and an overlay panel grows to the left.
//!
//! A file manager's places list fits in it: section headers
//! ([`SidebarItem::header`], optionally [`collapsible`](SidebarItem::collapsible)),
//! a dimmed second line with a usage bar ([`SidebarItem::detail`],
//! [`SidebarItem::usage`]), a trailing action such as eject
//! ([`SidebarItem::secondary`], [`Sidebar::on_secondary`]), a busy spinner,
//! dimmed and disabled rows, context menus ([`Sidebar::on_context`]), file
//! drops ([`Sidebar::on_drop_files`]) and stable keys
//! ([`SidebarItem::key`]) so [`Sidebar::set_items`] keeps the selection
//! and animates rows in and out. The rail scrolls when its rows do not fit.
//!
//! **Indexes** given to every callback and taken by every method are
//! indexes into the `Vec<SidebarItem>` the app passed (headers included).
//!
//! ```no_run
//! use fltk::prelude::*;
//! use fltk_omarchy::{layout::{self, FlexExt}, widgets::*, State};
//! let view = State::new(0usize);
//! let mut row = layout::row();
//! let side = Sidebar::new(vec![
//!     SidebarItem::new(glyph::FOLDER, "Files"),
//!     SidebarItem::new(glyph::SEARCH, "Search"),
//!     SidebarItem::new(glyph::SETTINGS, "Settings").footer(),
//! ], &view);
//! row.fit(&*side);
//! let content = fltk::frame::Frame::default(); // takes the rest
//! row.end();
//! ```

use crate::model::ListModel;
use crate::navtree::{Badge, ColorRole, NavItem, NavTree, RowAction};
use crate::state::State;
use fltk::{prelude::*, widget::Widget};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

/// One entry of a [`Sidebar`]: a row, or a section header
/// ([`SidebarItem::header`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SidebarItem {
    pub icon: String,
    pub label: String,
    pub badge: String,
    pub footer: bool,
    /// Stable identity across [`Sidebar::set_items`] (empty: none).
    pub key: String,
    /// Dimmed second line shown when expanded.
    pub detail: String,
    /// Usage bar under the detail (0..1).
    pub usage: Option<f64>,
    /// Trailing action button: its glyph and tooltip (empty: none).
    pub secondary_icon: String,
    pub secondary_tooltip: String,
    /// A section header (not selectable).
    pub header: bool,
    /// A header that collapses its section when clicked.
    pub collapsible: bool,
    /// Drawn dimmed but still activatable.
    pub dimmed: bool,
    /// Dimmed, not activatable, skipped by the keyboard.
    pub disabled: bool,
    /// Shows a spinner.
    pub busy: bool,
}

impl SidebarItem {
    pub fn new(icon: &str, label: &str) -> SidebarItem {
        SidebarItem {
            icon: icon.into(),
            label: label.into(),
            ..Default::default()
        }
    }
    /// A section header: a dimmed caption over the rows that follow it (up
    /// to the next header), a hairline separator when the rail is
    /// collapsed. Not selectable; with [`collapsible`](Self::collapsible)
    /// a click, Enter or Space folds its rows.
    pub fn header(label: &str) -> SidebarItem {
        SidebarItem {
            label: label.into(),
            header: true,
            ..Default::default()
        }
    }
    /// A header that folds its section (click, Enter / Space, Left / Right).
    pub fn collapsible(mut self) -> Self {
        self.collapsible = true;
        self
    }
    /// A short count or mark at the end of the row (expanded).
    pub fn badge(mut self, b: &str) -> Self {
        self.badge = b.into();
        self
    }
    /// Kept at the bottom of the rail (settings, account).
    pub fn footer(mut self) -> Self {
        self.footer = true;
        self
    }
    /// A stable identity (a path, a device id): [`Sidebar::set_items`]
    /// keeps the selection, the cursor and folded sections on the item
    /// with the same key, and animates rows that arrive or leave.
    pub fn key(mut self, k: &str) -> Self {
        self.key = k.into();
        self
    }
    /// A dimmed second line ("12 GB free of 32 GB"); the row is two lines
    /// tall when expanded.
    pub fn detail(mut self, d: &str) -> Self {
        self.detail = d.into();
        self
    }
    /// A thin usage bar under the detail, `0..=1` (accent; the theme's
    /// urgent color above 0.9).
    pub fn usage(mut self, fraction: f64) -> Self {
        self.usage = fraction.is_finite().then(|| fraction.clamp(0.0, 1.0));
        self
    }
    /// A trailing icon button (eject, remove) shown when expanded, with its
    /// own hover and `tooltip` (also its accessible name). Clicked or
    /// Delete on the row: [`Sidebar::on_secondary`].
    pub fn secondary(mut self, icon: &str, tooltip: &str) -> Self {
        self.secondary_icon = icon.into();
        self.secondary_tooltip = tooltip.into();
        self
    }
    /// Drawn dimmed but still activatable (an unmounted device, a missing
    /// folder).
    pub fn dimmed(mut self) -> Self {
        self.dimmed = true;
        self
    }
    /// Dimmed, not activatable and skipped by the keyboard.
    pub fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
    /// A spinner in place of the badge and the secondary button (in place
    /// of the icon while collapsed); it turns only while animations are on.
    pub fn busy(mut self, on: bool) -> Self {
        self.busy = on;
        self
    }
}

/// How a [`Sidebar`] (or a [`NavTree`]) expands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarMode {
    /// The rail grows in its row; the content beside it makes room.
    #[default]
    Push,
    /// The rail keeps its width; the expanded panel covers the content.
    Overlay,
}

/// The id of the secondary button among a row's actions.
const SECONDARY: &str = "secondary";

/// Keys of the items: theirs, else one made from the icon and label (the
/// n-th equal one), so rows without keys still animate and fold by label.
fn keys_of(items: &[SidebarItem]) -> Vec<String> {
    let mut seen: HashMap<(bool, &str, &str), u32> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let mut k = if it.key.is_empty() {
                let n = seen
                    .entry((it.header, it.icon.as_str(), it.label.as_str()))
                    .or_default();
                *n += 1;
                format!(
                    "\u{1}{}\u{2}{}\u{2}{}\u{2}{}",
                    it.header as u8, it.icon, it.label, n
                )
            } else {
                it.key.clone()
            };
            if !used.insert(k.clone()) {
                k = format!("{k}\u{3}{i}");
                used.insert(k.clone());
            }
            k
        })
        .collect()
}

/// The tree items of a flat list: rows after a header are its children.
fn to_nav(items: &[SidebarItem], keys: &[String], selected: usize) -> Vec<NavItem> {
    let mut header: [Option<usize>; 2] = [None, None];
    items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let side = it.footer as usize;
            if it.header {
                header[side] = Some(i);
                let mut n = NavItem::section(&keys[i], &it.label);
                n.collapsible = it.collapsible;
                n.footer = it.footer;
                return n;
            }
            let mut n = NavItem::row(&keys[i], &it.icon, &it.label);
            if let Some(h) = header[side] {
                n.parent = keys[h].clone();
            }
            n.footer = it.footer;
            n.detail = it.detail.clone();
            n.usage = it.usage;
            if !it.badge.is_empty() && !it.busy {
                n.badges.push(Badge::text(&it.badge, ColorRole::Dim));
            }
            if !it.secondary_icon.is_empty() {
                n.actions.push(RowAction::new(
                    SECONDARY,
                    &it.secondary_icon,
                    &it.secondary_tooltip,
                ));
            }
            n.dimmed = it.dimmed;
            n.disabled = it.disabled;
            n.busy = it.busy;
            n.active = i == selected;
            n
        })
        .collect()
}

type Cb1 = Option<Box<dyn FnMut(usize)>>;
type ContextFn = dyn FnMut(usize, i32, i32);
type FilesFn = dyn FnMut(usize, Vec<PathBuf>);

#[derive(Default)]
struct Cbs {
    activate: Cb1,
    secondary: Cb1,
    context: Option<Box<ContextFn>>,
    drop: Option<Box<dyn FnMut(usize, String)>>,
    drop_files: Option<Box<FilesFn>>,
    accept: Option<Rc<dyn Fn(usize) -> bool>>,
    section: Option<Box<dyn FnMut(usize, bool)>>,
}

struct Data {
    items: Vec<SidebarItem>,
    keys: Vec<String>,
    /// The item drawn as active (accent bar).
    active: Option<String>,
}

macro_rules! call {
    ($cbs:expr, $field:ident, $($arg:expr),*) => {{
        let f = $cbs.borrow_mut().$field.take();
        if let Some(mut f) = f {
            f($($arg),*);
            let mut c = $cbs.borrow_mut();
            if c.$field.is_none() {
                c.$field = Some(f);
            }
        }
    }};
}

/// Navigation rail that expands on hover. See the [module docs](self).
#[derive(Clone)]
pub struct Sidebar {
    inner: Widget,
    tree: NavTree,
    model: ListModel<String, NavItem>,
    data: Rc<RefCell<Data>>,
    cbs: Rc<RefCell<Cbs>>,
    selected: State<usize>,
    key_sel: State<String>,
}
fltk::widget_extends!(Sidebar, Widget, inner);

impl Sidebar {
    /// A rail with `items`; `selected` is the selected item's index.
    pub fn new(items: Vec<SidebarItem>, selected: &State<usize>) -> Sidebar {
        let keys = keys_of(&items);
        let sel = selected.get();
        let model = ListModel::new(|t: &NavItem| t.key.clone());
        model.extend(to_nav(&items, &keys, sel));
        let key_sel = State::new(keys.get(sel).cloned().unwrap_or_default());
        let data = Rc::new(RefCell::new(Data {
            active: keys.get(sel).cloned(),
            items,
            keys,
        }));
        let cbs: Rc<RefCell<Cbs>> = Rc::default();
        let index = {
            let data = data.clone();
            move |k: &str| data.borrow().keys.iter().position(|x| x == k)
        };
        let (i1, c1) = (index.clone(), cbs.clone());
        let (i2, c2) = (index.clone(), cbs.clone());
        let (i3, c3) = (index.clone(), cbs.clone());
        let (i4, c4) = (index.clone(), cbs.clone());
        let (i5, c5) = (index.clone(), cbs.clone());
        let (i6, c6) = (index.clone(), cbs.clone());
        let (i7, c7) = (index, cbs.clone());
        let tree = NavTree::new(&model, &key_sel)
            .accessible_as(true, "")
            .on_activate(move |k| {
                if let Some(i) = i1(k) {
                    call!(c1, activate, i);
                }
            })
            .on_action(move |k, a| {
                if let (Some(i), true) = (i2(k), a == SECONDARY) {
                    call!(c2, secondary, i);
                }
            })
            .on_context(move |k, x, y| {
                if let Some(i) = i3(k) {
                    call!(c3, context, i, x, y);
                }
            })
            .on_drop(move |k, t| {
                if let Some(i) = i4(k) {
                    call!(c4, drop, i, t);
                }
            })
            .on_drop_files(move |k, p| {
                if let Some(i) = i5(k) {
                    call!(c5, drop_files, i, p);
                }
            })
            .accept_drop(move |k| {
                let f = c6.borrow().accept.clone();
                match (i6(k), f) {
                    (Some(i), Some(f)) => f(i),
                    _ => true,
                }
            })
            .on_toggle(move |k, on| {
                if let Some(i) = i7(k) {
                    call!(c7, section, i, on);
                }
            });
        let inner = tree.rows_widget();
        let sb = Sidebar {
            inner,
            tree,
            model,
            data,
            cbs,
            selected: selected.clone(),
            key_sel,
        };
        sb.link_selection();
        sb
    }

    /// The index state and the tree's key state follow each other; the
    /// selected row is drawn active (accent bar, as before).
    fn link_selection(&self) {
        let (data, sel) = (self.data.clone(), self.selected.clone());
        let me = self.clone();
        crate::state::bind_widget(&self.inner, &self.key_sel, false, move |k| {
            let i = data.borrow().keys.iter().position(|x| x == k);
            if let Some(i) = i {
                sel.set_if_changed(i);
            }
            me.mark_active();
        });
        let (data, ks) = (self.data.clone(), self.key_sel.clone());
        let me = self.clone();
        crate::state::bind_widget(&self.inner, &self.selected, false, move |i| {
            let k = data.borrow().keys.get(*i).cloned();
            if let Some(k) = k {
                ks.set_if_changed(k);
            }
            me.mark_active();
        });
    }

    fn mark_active(&self) {
        let (old, new) = {
            let d = self.data.borrow();
            (d.active.clone(), d.keys.get(self.selected.get()).cloned())
        };
        if old == new {
            return;
        }
        self.data.borrow_mut().active = new.clone();
        self.model.transaction(|m| {
            if let Some(k) = &old {
                m.patch(k, |t| t.active = false);
            }
            if let Some(k) = &new {
                m.patch(k, |t| t.active = true);
            }
        });
    }

    /// The tree it is built on (live model, more callbacks).
    pub fn tree(&self) -> NavTree {
        self.tree.clone()
    }

    /// Width collapsed (the rail) and expanded.
    pub fn widths(self, rail: i32, expanded: i32) -> Self {
        let _ = self.tree.clone().widths(rail, expanded);
        self
    }

    /// Push (default) or overlay, see [`SidebarMode`].
    pub fn mode(self, m: SidebarMode) -> Self {
        let _ = self.tree.clone().mode(m);
        self
    }

    /// Keeps it expanded while `pinned` is true (two-way: [`Sidebar::set_pinned`]).
    pub fn bind_pinned(self, pinned: &State<bool>) -> Self {
        let _ = self.tree.clone().bind_pinned(pinned);
        self
    }

    /// Runs when an item is picked (click, Enter, Space), after `selected` changed.
    pub fn on_activate(self, f: impl FnMut(usize) + 'static) -> Self {
        self.cbs.borrow_mut().activate = Some(Box::new(f));
        self
    }

    /// Runs when an item's secondary button ([`SidebarItem::secondary`]) is
    /// clicked, or Delete is pressed on it. Not while it is busy or
    /// disabled. The row is not activated.
    pub fn on_secondary(self, f: impl FnMut(usize) + 'static) -> Self {
        self.cbs.borrow_mut().secondary = Some(Box::new(f));
        self
    }

    /// Context menu for an item: right click (at the pointer), or the Menu
    /// key / Shift+F10 on the cursor row (at the row's bottom start
    /// corner). `x, y` are coordinates in the top-level window, ready for
    /// `PopupMenu::open_at(&*sidebar, x, y, ..)`. Headers get it too;
    /// disabled rows do not.
    pub fn on_context(self, f: impl FnMut(usize, i32, i32) + 'static) -> Self {
        self.cbs.borrow_mut().context = Some(Box::new(f));
        self
    }

    /// Something dropped on an item: the text as FLTK delivers it (one
    /// `file://` URI per line from file managers on Linux, paths on macOS
    /// and Windows). The row under the pointer is highlighted during the
    /// drag. See [`Sidebar::accept_drop`].
    pub fn on_drop(self, f: impl FnMut(usize, String) + 'static) -> Self {
        self.cbs.borrow_mut().drop = Some(Box::new(f));
        self
    }

    /// Files dropped on an item, as paths (the dropped text parsed by
    /// [`crate::edit::parse_files`]; not called when it is not a list of
    /// files).
    pub fn on_drop_files(self, f: impl FnMut(usize, Vec<PathBuf>) + 'static) -> Self {
        self.cbs.borrow_mut().drop_files = Some(Box::new(f));
        self
    }

    /// Which items take drops (default: every row that is not a header or
    /// disabled). E.g. no drops on read-only devices.
    pub fn accept_drop(self, f: impl Fn(usize) -> bool + 'static) -> Self {
        self.cbs.borrow_mut().accept = Some(Rc::new(f));
        self
    }

    /// Runs when the user folds (`true`) or unfolds a collapsible header's
    /// section (click, keys, screen reader), with the header's index.
    pub fn on_section_toggle(self, f: impl FnMut(usize, bool) + 'static) -> Self {
        self.cbs.borrow_mut().section = Some(Box::new(f));
        self
    }

    pub fn set_pinned(&self, on: bool) {
        self.tree.set_pinned(on);
    }

    pub fn is_pinned(&self) -> bool {
        self.tree.is_pinned()
    }

    /// True while expanded (pinned, hovered or focused). Bind it to follow.
    pub fn expanded(&self) -> State<bool> {
        self.tree.expanded()
    }

    pub fn is_expanded(&self) -> bool {
        self.tree.is_expanded()
    }

    /// Expands now (as hover would, without the delay).
    pub fn expand(&self) {
        self.tree.expand();
    }

    /// Collapses unless pinned.
    pub fn collapse(&self) {
        self.tree.collapse();
    }

    pub fn items(&self) -> Vec<SidebarItem> {
        self.data.borrow().items.clone()
    }

    /// Replaces the items. Expanded and pinned state and the scroll
    /// position stay. With keys ([`SidebarItem::key`]) the selection, the
    /// cursor, the hovered row and folded sections follow their item to
    /// its new index (`selected` is set to it; `on_activate` does not run),
    /// rows that arrive fade in, rows that leave fade out and the others
    /// slide (the list row motion; at once with animations off). Without
    /// keys the selected index stays.
    pub fn set_items(&mut self, items: Vec<SidebarItem>) {
        let keys = keys_of(&items);
        let sel = self.selected.get();
        let new_sel = {
            let d = self.data.borrow();
            d.items
                .get(sel)
                .filter(|it| !it.key.is_empty())
                .and_then(|it| items.iter().position(|n| n.key == it.key))
                .unwrap_or(sel)
        };
        let nav = to_nav(&items, &keys, new_sel);
        {
            let mut d = self.data.borrow_mut();
            d.active = keys.get(new_sel).cloned();
            d.items = items;
            d.keys = keys.clone();
        }
        self.model.sync_from(nav);
        if new_sel != sel {
            self.selected.set(new_sel);
        }
        if let Some(k) = keys.get(new_sel) {
            self.key_sel.set_if_changed(k.clone());
        }
    }

    fn key(&self, i: usize) -> Option<String> {
        self.data.borrow().keys.get(i).cloned()
    }

    fn index(&self, k: &str) -> Option<usize> {
        self.data.borrow().keys.iter().position(|x| x == k)
    }

    fn foldable(&self, index: usize) -> bool {
        self.data
            .borrow()
            .items
            .get(index)
            .is_some_and(|it| it.header && it.collapsible)
    }

    /// True when header `index` is collapsible and folded.
    pub fn section_collapsed(&self, index: usize) -> bool {
        self.foldable(index) && self.key(index).is_some_and(|k| self.tree.is_collapsed(&k))
    }

    /// Folds (`true`) or unfolds the section of collapsible header `index`;
    /// the rows below slide (200 ms, as a tree branch). Remembered by the
    /// header's key (or label) across [`Sidebar::set_items`].
    pub fn set_section_collapsed(&self, index: usize, collapsed: bool) {
        if let (true, Some(k)) = (self.foldable(index), self.key(index)) {
            self.tree.set_collapsed(&k, collapsed);
        }
    }

    /// The current scroll offset of the rows (pixels) and its maximum.
    pub fn scroll_position(&self) -> (i32, i32) {
        self.tree.scroll_position()
    }

    /// The row being dragged over (drop target), if any.
    pub fn drop_target(&self) -> Option<usize> {
        self.tree.drop_target().and_then(|k| self.index(&k))
    }

    /// Where item `index` is drawn now (row motion included), in the
    /// sidebar's window coordinates (like `x()`/`y()`); None when it is
    /// folded away, scrolled out of view or out of range.
    pub fn item_rect(&self, index: usize) -> Option<(i32, i32, i32, i32)> {
        self.tree.item_rect(&self.key(index)?)
    }

    /// Simulates the pointer resting on (`true`) or leaving it, through the
    /// same path as real Enter/Leave events, hover intent included.
    pub fn hover(&self, on: bool) {
        self.tree.hover(on);
    }

    /// The item under the keyboard cursor.
    pub fn cursor(&self) -> usize {
        self.tree
            .cursor()
            .and_then(|k| self.index(&k))
            .unwrap_or_else(|| self.selected.get())
    }

    /// The overlay panel's current width (overlay mode; 0 when closed).
    pub fn overlay_width(&self) -> i32 {
        self.tree.overlay_width()
    }
}
