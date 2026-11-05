//! `NavTree`: a model-based navigation sidebar for file managers, chat and
//! coding apps — sections, trees of any depth, rich rows, trailing
//! actions, search, drag and drop, and the hover-expanding rail.
//!
//! The items live in a [`ListModel<String, NavItem>`](crate::model::ListModel)
//! keyed by [`NavItem::key`]; [`NavItem::parent`] makes the tree. Change
//! the model with diffs (`upsert`, `patch`, `remove_key`, `move_key`) or
//! `sync_from(snapshot)`; the tree keeps selection, cursor, folded
//! branches, scroll and focus by key, animates rows in and out, and only
//! repaints when a change is in view. [`NavTree::pause_updates`] holds
//! live updates (while the user drags, for example).
//!
//! It is virtual: one widget draws the rows in view, whatever the count.
//!
//! - **Rows** ([`NavItem::row`]): icon, title, dimmed detail line, usage
//!   bar, badges ([`Badge`]: text in a theme role, pill, status dot,
//!   spinner), shortcut hint, trailing actions ([`RowAction`], always or
//!   on hover), states (selected, `active`, `dimmed`, `disabled`, `busy`,
//!   `attention`), and a `summary` while a parent is folded.
//! - **Sections** ([`NavItem::section`]): caption, count, header actions,
//!   foldable. **Separators** ([`NavItem::separator`]).
//! - **Custom rows**: [`NavTree::renderer`] paints items whose
//!   [`NavItem::renderer`] names it (variable height).
//! - **Search**: [`NavTree::search`] adds a filter field (or call
//!   [`NavTree::set_query`]); [`NavTree::matcher`] scores items
//!   ([`default_match`] by default); matches are highlighted, ordered by
//!   score among their siblings and their ancestors stay.
//! - **Drag and drop**: rows reorder ([`NavTree::reorder`],
//!   [`NavTree::on_move`]) and take dropped files ([`NavTree::on_drop_files`]).
//! - **Shell**: rail of icons that expands on hover (350 ms intent) or
//!   focus, pinned, [`Presence::Hidden`]; push or overlay
//!   ([`SidebarMode`]); width dragged by the edge ([`NavTree::resizable`],
//!   [`NavTree::bind_width`]); [`Side`]; widgets above and below the rows
//!   ([`NavTree::header`], [`NavTree::footer_widget`]).
//!
//! Every key and callback works with item keys. Keys: docs/KEYBOARD.md.
//!
//! ```no_run
//! use fltk::prelude::*;
//! use fltk_omarchy::{layout::{self, FlexExt}, model::ListModel, navtree::*, widgets::glyph, State};
//! let model = ListModel::new(|t: &NavItem| t.key.clone());
//! model.extend([
//!     NavItem::section("projects", "Projects").collapsible(),
//!     NavItem::row("p1", glyph::FOLDER, "omarchy").parent("projects"),
//!     NavItem::row("t1", glyph::FILE, "Fix the bar").parent("p1")
//!         .badge(Badge::pill("PR #42", ColorRole::Green))
//!         .action(RowAction::new("archive", glyph::CLOSE, "Archive").on_hover()),
//! ]);
//! let selected = State::new(String::new());
//! let mut row = layout::row();
//! let tree = NavTree::new(&model, &selected).search("Search")
//!     .on_activate(|key| println!("open {key}"))
//!     .on_action(|key, action| println!("{action} {key}"));
//! row.fit(&*tree);
//! row.end();
//! ```

mod flat;
mod item;
mod rows;

pub use crate::sidebar::SidebarMode;
pub use crate::text::ColorRole;
pub use item::{
    default_match, Badge, Match, NavItem, NavKind, RowAction, RowPaint, RowState, Show,
};

use crate::a11y::{self, Action, Role};
use crate::anim::{self, Anim, Ease};
use crate::layout::Mirror;
use crate::model::ListModel;
use crate::panel::ItemId;
use crate::rowmotion::RowMotion;
use crate::state::{State, Subscription};
use crate::theme;
use flat::{FRow, Flat, Matcher};
use fltk::{app, enums::*, group::Flex, prelude::*, widget::Widget};
use rows::{metrics, Look, Part, Renderer};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Instant;

/// How rows can be dragged to another place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reorder {
    #[default]
    Off,
    /// Among their siblings.
    Siblings,
    /// Anywhere, into other parents too.
    Anywhere,
}

/// Which edge the sidebar sits on: where its accent bar and icons are,
/// where the overlay panel and the resize edge grow. `Start`/`End` follow
/// the language's direction; `Left`/`Right` are fixed. Place the widget in
/// its row on that side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Side {
    #[default]
    Start,
    End,
    Left,
    Right,
}

/// Shown at all, as a rail, or expanded (pinned).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Presence {
    /// Not shown (collapses through the layout motion).
    Hidden,
    /// The rail of icons; expands on hover or focus.
    #[default]
    Rail,
    /// Always expanded (pinned).
    Expanded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Surf {
    Rail,
    Layer,
}

/// What a row that leaves is drawn with while it fades.
#[derive(Clone)]
struct GhostRow {
    item: NavItem,
    row: FRow,
    h: i32,
}

/// Where a dragged row would go: the item it is dropped on and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Before,
    After,
    Into,
}

struct Drag {
    id: ItemId,
    x0: i32,
    y0: i32,
    active: bool,
}

struct Slot {
    w: Widget,
    footer: bool,
    h: i32,
}

type KeyCb = Option<Box<dyn FnMut(&str)>>;
type ActionFn = dyn FnMut(&str, &str);
type ContextFn = dyn FnMut(&str, i32, i32);
type ToggleFn = dyn FnMut(&str, bool);
type MoveFn = dyn FnMut(&str, &str, usize);
type MoveFilter = dyn Fn(&str, &str, usize) -> bool;
type DropFn = dyn FnMut(&str, String);
type FilesFn = dyn FnMut(&str, Vec<PathBuf>);
type KeyFilter = dyn Fn(&str) -> bool;

struct Nt {
    flat: Flat,
    dirty: bool,
    paused: bool,
    /// While paused: the items as shown (the model keeps changing).
    frozen: Option<HashMap<ItemId, NavItem>>,
    inserted: Vec<ItemId>,
    /// Rows came or went: the next rebuild starts row motion.
    moved: bool,
    collapsed: HashSet<String>,
    seen: HashSet<String>,
    query: String,
    matcher: Rc<Matcher>,
    renderers: HashMap<String, Rc<Renderer>>,
    hover: Option<(ItemId, Part)>,
    cursor: Option<ItemId>,
    cursor_pos: usize,
    hovered: bool,
    layer_hovered: bool,
    open: bool,
    gen: u64,
    hover_expand: bool,
    rail: i32,
    wide: i32,
    min_w: i32,
    max_w: i32,
    resizable: bool,
    mode: SidebarMode,
    side: Side,
    layer: Option<Widget>,
    layer_w: Anim<f64>,
    scroll: i32,
    wheel: crate::scroll::Scroll,
    last_reveal: f64,
    drop_target: Option<ItemId>,
    drop_pending: Option<ItemId>,
    drag: Option<Drag>,
    spot: Option<(ItemId, Place)>,
    resize: Option<(i32, i32)>,
    rm_rail: RowMotion<GhostRow>,
    rm_layer: RowMotion<GhostRow>,
    tip: [bool; 2],
    typed: String,
    typed_at: Option<Instant>,
    tabs: bool,
    name: String,
    drawn: HashSet<ItemId>,
    rows_drawn: usize,
    slots: Vec<Slot>,
    search_input: Option<Widget>,
    host: Option<Flex>,
    unfocus_gen: u64,
    on_activate: KeyCb,
    on_action: Option<Box<ActionFn>>,
    on_context: Option<Box<ContextFn>>,
    on_toggle: Option<Box<ToggleFn>>,
    on_move: Option<Box<MoveFn>>,
    accept_move: Option<Rc<MoveFilter>>,
    reorder: Reorder,
    on_drop: Option<Box<DropFn>>,
    on_drop_files: Option<Box<FilesFn>>,
    accept_drop: Option<Rc<KeyFilter>>,
    on_query: KeyCb,
    _sub: Option<Subscription>,
}

/// Runs a callback stored in `Nt` with the state released (it may call
/// back into the tree), then puts it back unless it was replaced.
macro_rules! fire {
    ($st:expr, $field:ident, $($arg:expr),*) => {{
        let f = $st.borrow_mut().$field.take();
        if let Some(mut f) = f {
            f($($arg),*);
            let mut s = $st.borrow_mut();
            if s.$field.is_none() {
                s.$field = Some(f);
            }
        }
    }};
}

/// Model-based navigation sidebar. See the [module docs](self).
#[derive(Clone)]
pub struct NavTree {
    root: Widget,
    canvas: Widget,
    st: Rc<RefCell<Nt>>,
    kick: Rc<Cell<bool>>,
    model: ListModel<String, NavItem>,
    selected: State<String>,
    pinned: State<bool>,
    expanded: State<bool>,
    width: State<i32>,
    query_state: State<String>,
}

impl std::ops::Deref for NavTree {
    type Target = Widget;
    fn deref(&self) -> &Widget {
        &self.root
    }
}

impl std::ops::DerefMut for NavTree {
    fn deref_mut(&mut self) -> &mut Widget {
        &mut self.root
    }
}

// ================================================================ geometry

/// Rows area of a surface and how far it is expanded.
#[derive(Clone, Copy)]
struct View {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    reveal: f64,
    /// Scrolling region, relative to `y`.
    top: i32,
    bottom: i32,
    max: i32,
    scroll: i32,
}

fn reveal_of(rail: i32, wide: i32, w: i32) -> f64 {
    if wide > rail {
        ((w - rail) as f64 / (wide - rail) as f64).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn view_of(s: &Nt, (x, y, w, h): (i32, i32, i32, i32), pad: i32) -> View {
    let reveal = reveal_of(s.rail, s.wide, w);
    let foot = s.flat.foot_h(reveal);
    let top = pad;
    let bottom = (h - pad - foot).max(top);
    let max = (s.flat.content(reveal) - (bottom - top)).max(0);
    View {
        x,
        y,
        w,
        h,
        reveal,
        top,
        bottom,
        max,
        scroll: s.scroll.clamp(0, max),
    }
}

impl View {
    /// y (relative to the view) and height of a shown row.
    fn row(&self, flat: &Flat, foot: bool, i: usize) -> (i32, i32) {
        let r = self.reveal;
        if foot {
            let y0 = self.h - (self.h - self.bottom) + flat.foot_y(i, r);
            (y0, flat.foot_y(i + 1, r) - flat.foot_y(i, r))
        } else {
            let y = self.top + flat.top_y(i, r) - self.scroll;
            (y, flat.top_y(i + 1, r) - flat.top_y(i, r))
        }
    }
    /// Top rows in view (index range).
    fn visible(&self, flat: &Flat) -> std::ops::Range<usize> {
        let a = flat.first_at(self.scroll, self.reveal);
        let b = flat.first_at(self.scroll + (self.bottom - self.top), self.reveal);
        a..(b + 1).min(flat.top.len())
    }
}

/// Offset of `w`'s window in its top-level window (subwindows).
fn window_origin(w: &Widget) -> (i32, i32) {
    let (mut ox, mut oy) = (0, 0);
    let mut win = w.window();
    while let Some(x) = win {
        if x.parent().is_none() {
            break;
        }
        ox += x.x();
        oy += x.y();
        win = x.window();
    }
    (ox, oy)
}

/// An item as shown: from the model, or held while updates are paused.
fn item_of(s: &Nt, model: &ListModel<String, NavItem>, id: ItemId) -> Option<NavItem> {
    match &s.frozen {
        Some(f) => f.get(&id).cloned(),
        None => model.with_id(id, |t| t.clone()),
    }
}

fn freeze(s: &mut Nt, model: &ListModel<String, NavItem>) {
    let held = s
        .flat
        .order()
        .filter_map(|r| model.with_id(r.id, |t| (r.id, t.clone())))
        .collect();
    s.frozen = Some(held);
}

fn rebuild(s: &mut Nt, model: &ListModel<String, NavItem>) {
    if !s.dirty {
        return;
    }
    s.dirty = false;
    // Scroll anchoring: the first row in view keeps its place.
    let r = s.last_reveal;
    let anchor = (s.scroll > 0)
        .then(|| {
            let i = s.flat.first_at(s.scroll, r);
            s.flat
                .top
                .get(i)
                .map(|row| (row.id, s.flat.top_y(i, r) - s.scroll))
        })
        .flatten();
    let renderers = s.renderers.clone();
    let matcher = s.matcher.clone();
    let query = s.query.clone();
    let flat = theme::with(|t| {
        let m = metrics(t);
        let height = |it: &NavItem| {
            let custom = (!it.renderer.is_empty())
                .then(|| renderers.get(&it.renderer))
                .flatten();
            rows::heights(it, &m, custom)
        };
        flat::flatten(
            model,
            &mut s.collapsed,
            &mut s.seen,
            &query,
            &*matcher,
            &height,
        )
    });
    s.flat = flat;
    if s.frozen.is_some() {
        // A fold or a search while paused shows the model as it is now.
        freeze(s, model);
    }
    if let Some((id, off)) = anchor {
        if let Some((false, i, _)) = s.flat.get(id) {
            let ny = s.flat.top_y(i, r) - off;
            let d = ny - s.scroll;
            if d != 0 {
                s.scroll = ny.max(0);
                s.wheel.shift(d as i64);
            }
        }
    }
    // The cursor stays on its item, or the row now at its place.
    let order: Vec<ItemId> = s.flat.order().map(|r| r.id).collect();
    match s.cursor.and_then(|c| order.iter().position(|&x| x == c)) {
        Some(p) => s.cursor_pos = p,
        None if !order.is_empty() => {
            let p = s.cursor_pos.min(order.len() - 1);
            s.cursor = s.flat.order().skip(p).find(|r| r.focusable).map(|r| r.id);
        }
        None => s.cursor = None,
    }
    if let Some((id, _)) = s.hover {
        if !s.flat.at.contains_key(&id) {
            s.hover = None;
        }
    }
    let ins = std::mem::take(&mut s.inserted);
    if std::mem::replace(&mut s.moved, false) || !ins.is_empty() {
        s.rm_rail.changed(&ins, &[]);
        s.rm_layer.changed(&ins, &[]);
    }
}

// ================================================================ painting

#[allow(clippy::too_many_arguments)]
fn paint_surface(
    s: &mut Nt,
    model: &ListModel<String, NavItem>,
    surf: Surf,
    sel: &str,
    rect: (i32, i32, i32, i32),
    right_side: bool,
    ring: bool,
    active: bool,
    widget: &Widget,
) {
    rebuild(s, model);
    let (x, y, w, h) = rect;
    let mut again = false;
    let mut rm = std::mem::take(match surf {
        Surf::Rail => &mut s.rm_rail,
        Surf::Layer => &mut s.rm_layer,
    });
    let sel_id = (!sel.is_empty())
        .then(|| model.id_of(&sel.to_string()))
        .flatten();
    theme::with(|t| {
        let m = metrics(t);
        let mir = Mirror::new(right_side, x, w);
        let bg = rows::surface(t);
        crate::glass::fill(x, y, w, h, 0, bg);
        let fg = t.palette.foreground;
        // Hairline on the edge facing the content.
        crate::paint::fill(mir.x(x + w - 1, 1), y, 1, h, 0, fg.over(bg, 0.12));
        let mut v = view_of(s, rect, m.pad);
        if active {
            s.scroll = (s.wheel.position(s.scroll as i64, v.max as i64) as i32).clamp(0, v.max);
            v.scroll = s.scroll;
            again |= s.wheel.running();
            s.last_reveal = v.reveal;
        }
        let range = v.visible(&s.flat);
        let mut shown: Vec<(bool, usize)> = range.map(|i| (false, i)).collect();
        shown.extend((0..s.flat.foot.len()).map(|i| (true, i)));
        let inview: Vec<(ItemId, i32)> = shown
            .iter()
            .map(|&(f, i)| (s.flat.row(f, i).id, v.row(&s.flat, f, i).0))
            .collect();
        let fl = &s.flat;
        rm.begin(&inview, |id| fl.at.contains_key(&id));
        let drift = (m.rh / 4) as f64;
        let phase = rows::spin_phase();
        let mut spun = false;
        fltk::draw::push_clip(x, y, w, h);
        for g in &rm.ghosts {
            let (dy, o) = g.sample(drift);
            let ry = y + g.y + dy;
            fltk::draw::push_clip(x, ry, w, g.row.h);
            rows::paint_row(
                t,
                &m,
                s.rail,
                &g.row.item,
                &g.row.row,
                (x, ry, w, g.row.h),
                v.reveal,
                Look::default(),
                &mir,
                bg,
                0.0,
                None,
                None,
            );
            crate::paint::alpha_fill(x + 1, ry, w - 2, g.row.h, bg, 1.0 - o);
            fltk::draw::pop_clip();
        }
        let mut drawn = Vec::new();
        if active {
            s.drawn.clear();
        }
        let mut count = 0;
        for &(foot, i) in &shown {
            let row = *s.flat.row(foot, i);
            let (ry0, rh) = v.row(&s.flat, foot, i);
            let Some(it) = item_of(s, model, row.id) else {
                continue;
            };
            let (dy, op, _) = rm.at(row.id, drift);
            let ry = y + ry0 + dy;
            if !foot {
                fltk::draw::push_clip(x, y + v.top, w, v.bottom - v.top);
            }
            fltk::draw::push_clip(x, ry, w, rh);
            let hovered = s.hover.filter(|(id, _)| *id == row.id).map(|(_, p)| p);
            let is_cursor = s.cursor == Some(row.id);
            let selected = sel_id == Some(row.id) && row.kind == NavKind::Row;
            let look = Look {
                selected,
                hover: hovered,
                ring: ring && is_cursor,
                drop: s.drop_target == Some(row.id) || s.spot == Some((row.id, Place::Into)),
                hot: hovered.is_some() || selected || (ring && is_cursor),
            };
            let custom = (!it.renderer.is_empty())
                .then(|| s.renderers.get(&it.renderer))
                .flatten();
            spun |= rows::paint_row(
                t,
                &m,
                s.rail,
                &it,
                &row,
                (x, ry, w, rh),
                v.reveal,
                look,
                &mir,
                bg,
                phase,
                s.flat.hits.get(&row.id).map(|v| v.as_slice()),
                custom,
            );
            if op < 1.0 {
                crate::paint::alpha_fill(x + 1, ry, w - 2, rh, bg, 1.0 - op);
            }
            fltk::draw::pop_clip();
            if !foot {
                fltk::draw::pop_clip();
            }
            count += 1;
            if active {
                s.drawn.insert(row.id);
            }
            // Recorded even with animations off: a change made right after
            // they are turned on still starts from what is on screen.
            drawn.push((
                row.id,
                ry - y,
                GhostRow {
                    item: it,
                    row,
                    h: rh,
                },
            ));
        }
        if active {
            s.rows_drawn = count;
        }
        // Reorder: a line where the row would go.
        if let Some((id, place)) = s.spot.filter(|(_, p)| *p != Place::Into) {
            if let Some((f, i, row)) = s.flat.get(id) {
                let (ry, rh) = v.row(&s.flat, f, i);
                let ly = y + if place == Place::Before { ry } else { ry + rh } - 1;
                let ind = (row.indent as f64 * m.ind as f64 * v.reveal) as i32;
                let lx = x + m.inset + ind;
                let lw = (w - 2 * m.inset - ind).max(0);
                crate::paint::fill(mir.x(lx, lw), ly, lw, 2, 0, t.palette.accent);
            }
        }
        if v.max > 0 {
            let track = v.bottom - v.top;
            let content = s.flat.content(v.reveal).max(1);
            let thumb =
                ((track as f64 * track as f64 / content as f64) as i32).clamp(16.min(track), track);
            let ty = y
                + v.top
                + ((v.scroll as f64 / v.max as f64) * (track - thumb) as f64).round() as i32;
            crate::paint::fill(mir.x(x + w - 5, 3), ty, 3, thumb, 1, fg.over(bg, 0.25));
        }
        fltk::draw::pop_clip();
        rm.end(drawn);
        again |= rm.running() || (spun && anim::enabled());
    });
    match surf {
        Surf::Rail => s.rm_rail = rm,
        Surf::Layer => s.rm_layer = rm,
    }
    if again {
        anim::request_redraw(widget);
    }
}

/// The row and part at window point `(ex, ey)` of a surface.
fn hit(
    s: &Nt,
    model: &ListModel<String, NavItem>,
    area: (i32, i32, i32, i32),
    right_side: bool,
    ex: i32,
    ey: i32,
) -> Option<(ItemId, Part)> {
    let (x, y, w, h) = area;
    if ex < x || ex >= x + w || ey < y || ey >= y + h {
        return None;
    }
    theme::with(|t| {
        let m = metrics(t);
        let v = view_of(s, area, m.pad);
        let ly = ey - y;
        let (foot, i) = if ly >= v.top && ly < v.bottom {
            let i = s.flat.first_at(ly - v.top + v.scroll, v.reveal);
            (false, i)
        } else {
            let i = (0..s.flat.foot.len()).find(|&i| {
                let (ry, rh) = v.row(&s.flat, true, i);
                ly >= ry && ly < ry + rh
            })?;
            (true, i)
        };
        let row = *s
            .flat
            .top
            .get(i)
            .filter(|_| !foot)
            .or_else(|| s.flat.foot.get(i).filter(|_| foot))?;
        let (ry, rh) = v.row(&s.flat, foot, i);
        if ly < ry || ly >= ry + rh {
            return None;
        }
        let part = item_of(s, model, row.id)
            .map(|it| {
                let it = &it;
                if v.reveal < 0.5 || it.disabled {
                    return Part::Row;
                }
                let l = rows::layout(t, &m, s.rail, it, &row, (x, y + ry, w, rh), v.reveal, true);
                let px = Mirror::new(right_side, x, w).point(ex);
                let inside = |(bx, by, bw, bh): (i32, i32, i32, i32)| {
                    px >= bx && px < bx + bw && ey >= by && ey < by + bh
                };
                for &(a, r) in &l.actions {
                    if inside(r) {
                        return Part::Action(a);
                    }
                }
                if l.chevron.is_some_and(inside) && it.kind == NavKind::Row {
                    return Part::Chevron;
                }
                Part::Row
            })
            .unwrap_or(Part::Row);
        Some((row.id, part))
    })
}

// ================================================================== widget

impl NavTree {
    /// A tree of `model`'s items; `selected` holds the selected key
    /// (empty: none).
    pub fn new(model: &ListModel<String, NavItem>, selected: &State<String>) -> NavTree {
        let (rail, wide) = theme::with(|t| {
            (
                t.spacing.control_height + 2 * t.spacing.lg,
                240.max(t.font.body * 20),
            )
        });
        let mut canvas = Widget::default().with_size(rail, 240);
        canvas.set_frame(FrameType::NoBox);
        canvas.set_visible_focus();
        let st = Rc::new(RefCell::new(Nt {
            flat: Flat::default(),
            dirty: true,
            paused: false,
            frozen: None,
            inserted: Vec::new(),
            moved: false,
            collapsed: HashSet::new(),
            seen: HashSet::new(),
            query: String::new(),
            matcher: Rc::new(default_match),
            renderers: HashMap::new(),
            hover: None,
            cursor: None,
            cursor_pos: 0,
            hovered: false,
            layer_hovered: false,
            open: false,
            gen: 0,
            hover_expand: true,
            rail,
            wide,
            min_w: rail,
            max_w: i32::MAX,
            resizable: false,
            mode: SidebarMode::Push,
            side: Side::Start,
            layer: None,
            layer_w: Anim::new(),
            scroll: 0,
            wheel: crate::scroll::Scroll::new(),
            last_reveal: 0.0,
            drop_target: None,
            drop_pending: None,
            drag: None,
            spot: None,
            resize: None,
            rm_rail: RowMotion::default(),
            rm_layer: RowMotion::default(),
            tip: [false; 2],
            typed: String::new(),
            typed_at: None,
            tabs: false,
            name: String::new(),
            drawn: HashSet::new(),
            rows_drawn: 0,
            slots: Vec::new(),
            search_input: None,
            host: None,
            unfocus_gen: 0,
            on_activate: None,
            on_action: None,
            on_context: None,
            on_toggle: None,
            on_move: None,
            accept_move: None,
            reorder: Reorder::Off,
            on_drop: None,
            on_drop_files: None,
            accept_drop: None,
            on_query: None,
            _sub: None,
        }));
        let t = NavTree {
            root: canvas.clone(),
            canvas,
            st,
            kick: Rc::new(Cell::new(false)),
            model: model.clone(),
            selected: selected.clone(),
            pinned: State::new(false),
            expanded: State::new(false),
            width: State::new(wide),
            query_state: State::new(String::new()),
        };
        t.install();
        t
    }

    /// The model it shows.
    pub fn model(&self) -> ListModel<String, NavItem> {
        self.model.clone()
    }

    /// The widget that draws the rows and takes the keyboard (the same as
    /// the tree itself unless it has header or footer widgets).
    pub fn rows_widget(&self) -> Widget {
        self.canvas.clone()
    }

    // ------------------------------------------------------------ builder

    /// Width collapsed (the rail) and expanded.
    pub fn widths(self, rail: i32, expanded: i32) -> Self {
        {
            let mut s = self.st.borrow_mut();
            s.rail = rail.max(1);
            s.min_w = s.min_w.max(s.rail);
            s.wide = expanded.max(s.rail);
        }
        self.width.set(expanded.max(rail));
        let mut w = self.root_w();
        let h = w.h();
        w.set_size(if self.expanded.get() { expanded } else { rail }, h);
        self
    }

    /// Push (default) or overlay, see [`SidebarMode`].
    pub fn mode(self, m: SidebarMode) -> Self {
        self.st.borrow_mut().mode = m;
        self
    }

    /// The edge it sits on (default: the start edge of the language).
    pub fn side(self, side: Side) -> Self {
        self.st.borrow_mut().side = side;
        self.canvas.clone().redraw();
        self
    }

    /// Whether resting the pointer expands the rail (default true).
    pub fn hover_expand(self, on: bool) -> Self {
        self.st.borrow_mut().hover_expand = on;
        self
    }

    /// The expanded width can be dragged by the edge facing the content,
    /// between `min` and `max` (see [`NavTree::bind_width`]).
    pub fn resizable(self, min: i32, max: i32) -> Self {
        {
            let mut s = self.st.borrow_mut();
            s.resizable = true;
            s.min_w = min.max(s.rail);
            s.max_w = max.max(s.min_w);
            s.wide = s.wide.clamp(s.min_w, s.max_w);
        }
        self
    }

    /// The expanded width, two-way: dragging the edge sets it when the drag
    /// ends; setting it resizes (persist it in the app's settings).
    pub fn bind_width(self, width: &State<i32>) -> Self {
        let me = self.clone();
        crate::state::bind_widget(&self.canvas, width, true, move |w| me.set_wide(*w, false));
        let app_w = width.clone();
        crate::state::bind_widget(&self.canvas, &self.width, false, move |w| {
            app_w.set_if_changed(*w);
        });
        self
    }

    /// Keeps it expanded while `pinned` is true (two-way: [`NavTree::set_pinned`]).
    pub fn bind_pinned(self, pinned: &State<bool>) -> Self {
        let (me, mine) = (self.clone(), self.pinned.clone());
        crate::state::bind_widget(&self.canvas, pinned, true, move |v| {
            mine.set_if_changed(*v);
            me.apply();
        });
        let theirs = pinned.clone();
        crate::state::bind_widget(&self.canvas, &self.pinned, false, move |v| {
            theirs.set_if_changed(*v);
        });
        self
    }

    /// An item was picked (click, Enter, Space), after `selected` changed.
    pub fn on_activate(self, f: impl FnMut(&str) + 'static) -> Self {
        self.st.borrow_mut().on_activate = Some(Box::new(f));
        self
    }

    /// A row or section action ([`RowAction`]) was clicked, or Delete was
    /// pressed (the cursor row's first enabled action): `(key, action id)`.
    pub fn on_action(self, f: impl FnMut(&str, &str) + 'static) -> Self {
        self.st.borrow_mut().on_action = Some(Box::new(f));
        self
    }

    /// Context menu of an item or section: right click (at the pointer) or
    /// the Menu key / Shift+F10 (at the row's bottom start corner), in
    /// top-level window coordinates. See [`NavTree::open_menu`].
    pub fn on_context(self, f: impl FnMut(&str, i32, i32) + 'static) -> Self {
        self.st.borrow_mut().on_context = Some(Box::new(f));
        self
    }

    /// A branch or section was folded (`true`) or opened by the user.
    pub fn on_toggle(self, f: impl FnMut(&str, bool) + 'static) -> Self {
        self.st.borrow_mut().on_toggle = Some(Box::new(f));
        self
    }

    /// Lets rows be dragged to another place (see [`Reorder`]).
    pub fn reorder(self, r: Reorder) -> Self {
        self.st.borrow_mut().reorder = r;
        self
    }

    /// A row was dragged: `(key, new parent key, index among the new
    /// parent's children without it)`. Apply it to the model (change
    /// `parent`, `move_key`); the rows slide.
    pub fn on_move(self, f: impl FnMut(&str, &str, usize) + 'static) -> Self {
        self.st.borrow_mut().on_move = Some(Box::new(f));
        self
    }

    /// Which moves are allowed (`key, new parent, index`).
    pub fn accept_move(self, f: impl Fn(&str, &str, usize) -> bool + 'static) -> Self {
        self.st.borrow_mut().accept_move = Some(Rc::new(f));
        self
    }

    /// Text dropped on an item (as FLTK delivers it: `file://` URIs, one
    /// per line, or paths). The row under the pointer is highlighted.
    pub fn on_drop(self, f: impl FnMut(&str, String) + 'static) -> Self {
        self.st.borrow_mut().on_drop = Some(Box::new(f));
        self
    }

    /// Files dropped on an item (the drop parsed by [`crate::edit::parse_files`]).
    pub fn on_drop_files(self, f: impl FnMut(&str, Vec<PathBuf>) + 'static) -> Self {
        self.st.borrow_mut().on_drop_files = Some(Box::new(f));
        self
    }

    /// Which items take drops (default: rows that are not disabled).
    pub fn accept_drop(self, f: impl Fn(&str) -> bool + 'static) -> Self {
        self.st.borrow_mut().accept_drop = Some(Rc::new(f));
        self
    }

    /// A custom row: items with `renderer == name` are `height(item)` tall
    /// when expanded and painted by `paint` (after the selection or hover
    /// fill and the focus ring; the rail shows a plain icon row).
    pub fn renderer(
        self,
        name: &str,
        height: impl Fn(&NavItem) -> i32 + 'static,
        paint: impl Fn(&RowPaint) + 'static,
    ) -> Self {
        {
            let mut s = self.st.borrow_mut();
            s.renderers.insert(
                name.into(),
                Rc::new(Renderer {
                    height: Box::new(height),
                    paint: Box::new(paint),
                }),
            );
            s.dirty = true;
        }
        self.redraw_all();
        self
    }

    /// How search scores items (default [`default_match`]).
    pub fn matcher(self, f: impl Fn(&NavItem, &str) -> Option<Match> + 'static) -> Self {
        {
            let mut s = self.st.borrow_mut();
            s.matcher = Rc::new(f);
            s.dirty = true;
        }
        self
    }

    /// Runs when the search text changes (built-in field or `set_query`).
    pub fn on_query(self, f: impl FnMut(&str) + 'static) -> Self {
        self.st.borrow_mut().on_query = Some(Box::new(f));
        self
    }

    /// Screen readers: present it as a flat tab list (a navigation rail,
    /// like `Sidebar`) instead of a tree. `name` is its accessible name
    /// (default "Sidebar", translated).
    pub fn accessible_as(self, tabs: bool, name: &str) -> Self {
        {
            let mut s = self.st.borrow_mut();
            s.tabs = tabs;
            s.name = name.into();
        }
        self
    }

    /// A built-in search field above the rows (`placeholder` from the
    /// app's catalog). Typing filters; Down goes to the results, Enter
    /// opens the first one.
    pub fn search(mut self, placeholder: &str) -> Self {
        let q = self.query_state.clone();
        let field = crate::widgets::TextField::new()
            .placeholder(placeholder)
            .bind(&q);
        let me = self.clone();
        let field = field.on_submit(move |_| {
            let first = {
                let s = me.st.borrow();
                // The best match (siblings are ordered by score), else the first row.
                let id = s
                    .flat
                    .order()
                    .find(|r| {
                        r.focusable && r.kind == NavKind::Row && s.flat.matched.contains(&r.id)
                    })
                    .or_else(|| {
                        s.flat
                            .order()
                            .find(|r| r.focusable && r.kind == NavKind::Row)
                    })
                    .map(|r| r.id);
                id
            };
            if let Some(id) = first {
                me.activate(id);
            }
        });
        let me = self.clone();
        crate::state::bind_widget(&self.canvas, &q, false, move |v| me.set_query(v));
        let me = self.clone();
        let field = field.on_key(move |k| {
            let mods = app::event_state()
                .intersects(Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta | Shortcut::Shift);
            if k == Key::Down && !mods {
                let first = me
                    .st
                    .borrow()
                    .flat
                    .order()
                    .find(|r| r.focusable)
                    .map(|r| r.id);
                me.st.borrow_mut().cursor = first;
                let _ = me.canvas.clone().take_focus();
                me.reveal_cursor();
                me.redraw_all();
                return true;
            }
            false
        });
        // The field sits on the sidebar's surface, inset like the rows.
        let pad = theme::with(|t| t.spacing.sm);
        let fh = field.h();
        let prev = fltk::group::Group::try_current();
        fltk::group::Group::set_current(None::<&fltk::group::Group>);
        let mut wrap = fltk::group::Group::new(0, 0, 200, fh + 2 * pad, None);
        wrap.end();
        if let Some(g) = prev {
            g.begin();
        }
        let mut f = field.clone();
        f.resize(pad, pad, 200 - 2 * pad, fh);
        wrap.add(&*f);
        wrap.resizable(&*f);
        wrap.set_frame(FrameType::NoBox);
        wrap.super_draw(false);
        wrap.draw(|g| {
            theme::with(|t| crate::glass::fill(g.x(), g.y(), g.w(), g.h(), 0, rows::surface(t)));
            g.draw_children();
        });
        if let Some(input) = field.child(0) {
            self.st.borrow_mut().search_input = Some(input);
        }
        let w = wrap.as_base_widget();
        self.add_slot(&w, false, true);
        self
    }

    /// A widget above the rows (a title, buttons, a filter), shown while
    /// expanded. It keeps its height.
    pub fn header<W: WidgetExt>(mut self, w: &W) -> Self {
        // SAFETY: a live widget; the copy only tracks it.
        let w = unsafe { Widget::from_widget_ptr(w.as_widget_ptr()) };
        self.add_slot(&w, false, false);
        self
    }

    /// A widget below the rows, shown while expanded.
    pub fn footer_widget<W: WidgetExt>(mut self, w: &W) -> Self {
        // SAFETY: as above.
        let w = unsafe { Widget::from_widget_ptr(w.as_widget_ptr()) };
        self.add_slot(&w, true, false);
        self
    }

    fn add_slot(&mut self, w: &Widget, footer: bool, first: bool) {
        let mut host = self.ensure_host();
        let h = w.h().max(theme::with(|t| t.spacing.control_height));
        let at = if footer {
            host.children()
        } else if first {
            0
        } else {
            host.find(&self.canvas)
        };
        host.insert(w, at);
        host.fixed(w, h);
        let mut w2 = w.clone();
        if !self.is_expanded() {
            w2.hide();
        }
        host.layout();
        self.st.borrow_mut().slots.push(Slot {
            w: w.clone(),
            footer,
            h,
        });
    }

    /// Wraps the rows in a column (once) so widgets can go above and below.
    fn ensure_host(&mut self) -> Flex {
        let host = self.st.borrow().host.clone();
        if let Some(h) = host {
            return h;
        }
        let parent = self.canvas.parent();
        let prev = fltk::group::Group::try_current();
        fltk::group::Group::set_current(None::<&fltk::group::Group>);
        let mut host = Flex::default().column();
        host.end();
        host.set_margin(0);
        host.set_pad(0);
        host.resize(
            self.canvas.x(),
            self.canvas.y(),
            self.canvas.w(),
            self.canvas.h(),
        );
        if let Some(mut p) = parent {
            let i = p.find(&self.canvas);
            p.insert(&host, i);
        }
        host.add(&self.canvas);
        crate::layout::set_animated(&host, false);
        if let Some(g) = prev {
            g.begin();
        }
        // Hover is the whole column's (the field, the rows).
        let me = self.clone();
        host.handle(move |_, ev| {
            match ev {
                Event::Enter => me.entered(),
                Event::Leave => me.left(),
                _ => {}
            }
            false
        });
        self.root = host.as_base_widget();
        self.st.borrow_mut().host = Some(host.clone());
        host
    }

    // ------------------------------------------------------------ state

    pub fn set_pinned(&self, on: bool) {
        self.pinned.set(on);
        self.apply();
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned.get()
    }

    /// True while expanded (pinned, hovered or focused).
    pub fn expanded(&self) -> State<bool> {
        self.expanded.clone()
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded.get()
    }

    /// Expands now (as hover would, without the delay).
    pub fn expand(&self) {
        self.st.borrow_mut().open = true;
        self.apply();
    }

    /// Collapses unless pinned.
    pub fn collapse(&self) {
        self.st.borrow_mut().open = false;
        self.apply();
    }

    /// Hidden, rail or expanded (pinned).
    pub fn set_presence(&self, p: Presence) {
        crate::layout::set_visible(&self.root_w(), p != Presence::Hidden);
        match p {
            Presence::Expanded => self.set_pinned(true),
            Presence::Rail => {
                self.set_pinned(false);
                self.collapse();
            }
            Presence::Hidden => {
                self.pinned.set(false);
                self.st.borrow_mut().open = false;
                self.apply();
            }
        }
    }

    pub fn presence(&self) -> Presence {
        if !self.root_w().visible() {
            Presence::Hidden
        } else if self.is_pinned() {
            Presence::Expanded
        } else {
            Presence::Rail
        }
    }

    /// Holds model changes (`true`): the rows stay as they are until
    /// `pause_updates(false)` applies everything at once.
    pub fn pause_updates(&self, on: bool) {
        let resume = {
            let mut s = self.st.borrow_mut();
            let was = std::mem::replace(&mut s.paused, on);
            if on && !was {
                rebuild(&mut s, &self.model);
                freeze(&mut s, &self.model);
            }
            was && !on
        };
        if resume {
            let mut s = self.st.borrow_mut();
            s.frozen = None;
            s.dirty = true;
            s.moved = true;
            drop(s);
            a11y::changed(&self.canvas);
            self.redraw_all();
        }
    }

    /// The search text.
    pub fn query(&self) -> String {
        self.st.borrow().query.clone()
    }

    /// Filters the rows (empty: all). The built-in field follows.
    pub fn set_query(&self, q: &str) {
        let changed = {
            let mut s = self.st.borrow_mut();
            if s.query == q {
                false
            } else {
                s.query = q.into();
                s.dirty = true;
                s.scroll = 0;
                s.wheel.jump(0);
                true
            }
        };
        if !changed {
            return;
        }
        if self.query_state.get() != q {
            self.query_state.set(q.to_string());
        }
        fire!(self.st, on_query, q);
        a11y::changed(&self.canvas);
        self.redraw_all();
    }

    /// Focuses the built-in search field (expanding the sidebar).
    pub fn focus_search(&self) {
        self.expand();
        let input = self.st.borrow().search_input.clone();
        if let Some(mut f) = input.filter(|f| !f.was_deleted()) {
            let _ = f.take_focus();
        }
    }

    /// True when `key` is folded.
    pub fn is_collapsed(&self, key: &str) -> bool {
        self.st.borrow().collapsed.contains(key)
    }

    /// Folds (`true`) or opens an item's branch or a section; the rows
    /// below slide (200 ms). Remembered by key.
    pub fn set_collapsed(&self, key: &str, on: bool) {
        self.fold(key, on, false);
    }

    /// Opens the ancestors of `key`, puts the cursor on it and scrolls it
    /// into view (animated).
    pub fn reveal(&self, key: &str) {
        let mut cur = self.model.with_key(&key.to_string(), |t| t.parent.clone());
        let mut opened = false;
        let mut guard = 0;
        while let Some(p) = cur.filter(|p| !p.is_empty() && guard < 64) {
            opened |= self.st.borrow_mut().collapsed.remove(&p);
            cur = self.model.with_key(&p, |t| t.parent.clone());
            guard += 1;
        }
        if opened {
            let mut s = self.st.borrow_mut();
            s.dirty = true;
            s.rm_rail.reflowed(anim::ms::REVEAL);
            s.rm_layer.reflowed(anim::ms::REVEAL);
        }
        if let Some(id) = self.model.id_of(&key.to_string()) {
            self.st.borrow_mut().cursor = Some(id);
        }
        self.scroll_to(key);
    }

    /// Scrolls `key`'s row into view (animated, 120 ms).
    pub fn scroll_to(&self, key: &str) {
        let Some(id) = self.model.id_of(&key.to_string()) else {
            return;
        };
        let (_, area) = self.visible();
        let pad = theme::with(|t| metrics(t).pad);
        {
            let mut s = self.st.borrow_mut();
            rebuild(&mut s, &self.model);
            let s = &mut *s;
            let mut v = view_of(s, area, pad);
            v.reveal = if self.expanded.get() { 1.0 } else { v.reveal };
            let Some((false, i, _)) = s.flat.get(id) else {
                return;
            };
            let (y0, y1) = (s.flat.top_y(i, v.reveal), s.flat.top_y(i + 1, v.reveal));
            let view = v.bottom - v.top;
            let sc = s.scroll.clamp(0, v.max);
            let target = if y0 < sc {
                y0
            } else if y1 > sc + view {
                (y1 - view).min(v.max)
            } else {
                sc
            };
            if target != sc {
                s.scroll = s.wheel.wheel(sc as i64, (target - sc) as f64, v.max as i64) as i32;
            }
        }
        self.redraw_all();
    }

    /// The scroll offset of the rows (pixels) and its maximum.
    pub fn scroll_position(&self) -> (i32, i32) {
        let (_, area) = self.visible();
        let pad = theme::with(|t| metrics(t).pad);
        let mut s = self.st.borrow_mut();
        rebuild(&mut s, &self.model);
        let v = view_of(&s, area, pad);
        (v.scroll, v.max)
    }

    /// The key under the keyboard cursor.
    pub fn cursor(&self) -> Option<String> {
        let id = self.st.borrow().cursor?;
        self.key_of(id)
    }

    pub fn set_cursor(&self, key: &str) {
        let id = self.model.id_of(&key.to_string());
        self.st.borrow_mut().cursor = id;
        self.reveal_cursor();
        self.redraw_all();
    }

    /// Keys of the rows as shown (screen order; folded ones left out).
    pub fn visible_keys(&self) -> Vec<String> {
        let mut s = self.st.borrow_mut();
        rebuild(&mut s, &self.model);
        s.flat
            .order()
            .filter_map(|r| item_of(&s, &self.model, r.id).map(|t| t.key))
            .collect()
    }

    /// Rows painted by the last draw (tests: it stays near the view size).
    pub fn rows_drawn(&self) -> usize {
        self.st.borrow().rows_drawn
    }

    /// The item being dragged over (file drop) or the row a reordered row
    /// would go into.
    pub fn drop_target(&self) -> Option<String> {
        let s = self.st.borrow();
        let id = s
            .drop_target
            .or(s.spot.filter(|(_, p)| *p == Place::Into).map(|(i, _)| i))?;
        drop(s);
        self.key_of(id)
    }

    /// Where a dragged row would go: `(key of the row it is dropped on,
    /// "before" | "after" | "into")`.
    pub fn drop_spot(&self) -> Option<(String, &'static str)> {
        let (id, p) = self.st.borrow().spot?;
        let k = self.key_of(id)?;
        Some((
            k,
            match p {
                Place::Before => "before",
                Place::After => "after",
                Place::Into => "into",
            },
        ))
    }

    /// Where `key`'s row is drawn now (row motion included), in the tree's
    /// window coordinates; None when folded away, scrolled out or unknown.
    pub fn item_rect(&self, key: &str) -> Option<(i32, i32, i32, i32)> {
        let id = self.model.id_of(&key.to_string())?;
        let (surf, area) = self.visible();
        let pad = theme::with(|t| metrics(t).pad);
        let mut s = self.st.borrow_mut();
        rebuild(&mut s, &self.model);
        let v = view_of(&s, area, pad);
        let (f, i, _) = s.flat.get(id)?;
        let (ry, rh) = v.row(&s.flat, f, i);
        let rm = match surf {
            Surf::Rail => &s.rm_rail,
            Surf::Layer => &s.rm_layer,
        };
        let drift = theme::with(|t| (metrics(t).rh / 4) as f64);
        let ry = ry + rm.at(id, drift).0;
        if !f && (ry + rh <= v.top || ry >= v.bottom) {
            return None;
        }
        let (ox, oy) = match surf {
            Surf::Rail => (0, 0),
            Surf::Layer => {
                let o = window_origin(&self.canvas);
                (-o.0, -o.1)
            }
        };
        Some((v.x + ox, v.y + ry + oy, v.w, rh))
    }

    /// Opens a [`PopupMenu`](crate::popups::PopupMenu) at `(x, y)` (the
    /// coordinates `on_context` got); `on_pick` gets the entry's index.
    pub fn open_menu(
        &self,
        x: i32,
        y: i32,
        items: Vec<crate::popups::PopupItem>,
        on_pick: impl FnOnce(usize) + 'static,
    ) -> crate::popups::PopupHandle {
        crate::popups::PopupMenu::open_at(&self.canvas, x, y, items, on_pick)
    }

    /// The overlay panel's current width (overlay mode; 0 when closed).
    pub fn overlay_width(&self) -> i32 {
        let s = self.st.borrow();
        match &s.layer {
            Some(l) if !l.was_deleted() => s.layer_w.target().map_or(0, |v| v as i32),
            _ => 0,
        }
    }

    /// Simulates the pointer resting on (`true`) or leaving it, through the
    /// same path as real Enter/Leave events, hover intent included.
    pub fn hover(&self, on: bool) {
        if on {
            self.entered()
        } else {
            self.left()
        }
    }

    // ------------------------------------------------------------ internals

    /// The outer widget: the rows, or the column holding them and the
    /// header widgets (closures keep an early copy of `self`).
    fn root_w(&self) -> Widget {
        self.st
            .borrow()
            .host
            .clone()
            .map(|h| h.as_base_widget())
            .unwrap_or_else(|| self.canvas.clone())
    }

    fn right_side(&self) -> bool {
        let rtl = crate::layout::is_rtl(&self.root_w());
        let side = self.st.borrow().side;
        match side {
            Side::Start => rtl,
            Side::End => !rtl,
            Side::Left => false,
            Side::Right => true,
        }
    }

    fn redraw_all(&self) {
        self.canvas.clone().redraw();
        let layer = self.st.borrow().layer.clone();
        if let Some(mut l) = layer.filter(|l| !l.was_deleted()) {
            l.redraw();
        }
    }

    /// Height of the slots in the overlay panel: (above, below).
    fn layer_slots(s: &Nt) -> (i32, i32) {
        let up = s.slots.iter().filter(|x| !x.footer).map(|x| x.h).sum();
        let down = s.slots.iter().filter(|x| x.footer).map(|x| x.h).sum();
        (up, down)
    }

    fn visible_of(s: &Nt, canvas: &Widget, right_side: bool) -> (Surf, (i32, i32, i32, i32)) {
        if s.mode == SidebarMode::Overlay {
            if let Some(l) = s.layer.as_ref().filter(|l| !l.was_deleted()) {
                let w = s.layer_w.target().unwrap_or(s.rail as f64).round() as i32;
                let x = if right_side { l.x() + l.w() - w } else { l.x() };
                let (up, down) = Self::layer_slots(s);
                return (Surf::Layer, (x, l.y() + up, w, (l.h() - up - down).max(0)));
            }
        }
        (Surf::Rail, (canvas.x(), canvas.y(), canvas.w(), canvas.h()))
    }

    fn visible(&self) -> (Surf, (i32, i32, i32, i32)) {
        let rs = self.right_side();
        Self::visible_of(&self.st.borrow(), &self.canvas, rs)
    }

    fn area(&self, surf: Surf) -> (i32, i32, i32, i32) {
        match self.visible() {
            (v, a) if v == surf => a,
            _ => (
                self.canvas.x(),
                self.canvas.y(),
                self.canvas.w(),
                self.canvas.h(),
            ),
        }
    }

    fn key_of(&self, id: ItemId) -> Option<String> {
        let s = self.st.borrow();
        match &s.frozen {
            Some(f) => f.get(&id).map(|t| t.key.clone()),
            None => self.model.key_of(id),
        }
    }

    fn item(&self, id: ItemId) -> Option<NavItem> {
        item_of(&self.st.borrow(), &self.model, id)
    }

    fn entered(&self) {
        if !self.st.borrow().hover_expand {
            self.st.borrow_mut().hovered = true;
            return;
        }
        let gen = {
            let mut s = self.st.borrow_mut();
            s.hovered = true;
            s.gen += 1;
            s.gen
        };
        let me = self.clone();
        app::add_timeout3(anim::ms::HOVER_OPEN as f64 / 1000.0, move |_| {
            let go = {
                let s = me.st.borrow();
                s.gen == gen && (s.hovered || s.layer_hovered)
            };
            if go && !me.canvas.was_deleted() {
                me.expand();
            }
        });
    }

    fn left(&self) {
        let gen = {
            let mut s = self.st.borrow_mut();
            s.hovered = false;
            s.hover = None;
            s.gen += 1;
            s.gen
        };
        self.canvas.clone().redraw();
        let me = self.clone();
        app::add_timeout3(anim::ms::HOVER_CLOSE as f64 / 1000.0, move |_| {
            if me.canvas.was_deleted() {
                return;
            }
            let stay = {
                let s = me.st.borrow();
                s.gen != gen || s.hovered || s.layer_hovered || s.resize.is_some()
            };
            if !stay && !me.holds_focus() && !crate::overlay::modal_open_over(&me.canvas) {
                me.collapse();
            }
        });
    }

    /// Focus is in the rows, a header widget or the overlay panel.
    fn holds_focus(&self) -> bool {
        let Some(f) = app::focus() else { return false };
        let fp = f.as_widget_ptr();
        let inside = |g: &Widget| {
            let mut cur = Some(unsafe { Widget::from_widget_ptr(fp) });
            while let Some(c) = cur {
                if c.as_widget_ptr() == g.as_widget_ptr() {
                    return true;
                }
                cur = c.parent().map(|p| p.as_base_widget());
            }
            false
        };
        let layer = self.st.borrow().layer.clone();
        inside(&self.root_w()) || layer.is_some_and(|l| !l.was_deleted() && inside(&l))
    }

    fn activate(&self, id: ItemId) {
        let Some(it) = self.item(id) else { return };
        if it.kind != NavKind::Row || it.disabled {
            return;
        }
        self.st.borrow_mut().cursor = Some(id);
        self.selected.set(it.key.clone());
        fire!(self.st, on_activate, &it.key);
        self.redraw_all();
    }

    fn action(&self, id: ItemId, a: usize) {
        let Some(it) = self.item(id) else { return };
        let Some(act) = it.actions.get(a) else { return };
        if !act.enabled || it.disabled || it.busy {
            return;
        }
        if it.kind == NavKind::Row || self.st.borrow().flat.get(id).is_some_and(|r| r.2.focusable) {
            self.st.borrow_mut().cursor = Some(id);
        }
        let aid = act.id.clone();
        fire!(self.st, on_action, &it.key, &aid);
        self.redraw_all();
    }

    fn context(&self, id: ItemId, x: i32, y: i32) -> bool {
        let Some(it) = self.item(id) else {
            return false;
        };
        if it.disabled || self.st.borrow().on_context.is_none() {
            return false;
        }
        fire!(self.st, on_context, &it.key, x, y);
        true
    }

    fn fold(&self, key: &str, on: bool, user: bool) {
        let changed = {
            let mut s = self.st.borrow_mut();
            let c = if on {
                s.collapsed.insert(key.to_string())
            } else {
                s.collapsed.remove(key)
            };
            s.seen.insert(key.to_string());
            if c {
                s.dirty = true;
                s.rm_rail.reflowed(anim::ms::REVEAL);
                s.rm_layer.reflowed(anim::ms::REVEAL);
                // The cursor inside a folded branch goes to its parent.
                if on {
                    if let Some(id) = self.model.id_of(&key.to_string()) {
                        let mut cur = s
                            .cursor
                            .and_then(|c| self.model.with_id(c, |t| t.parent.clone()));
                        let mut guard = 0;
                        while let Some(p) = cur.filter(|p| !p.is_empty() && guard < 64) {
                            if p == key {
                                s.cursor = Some(id);
                                break;
                            }
                            cur = self.model.with_key(&p, |t| t.parent.clone());
                            guard += 1;
                        }
                    }
                }
            }
            c
        };
        if !changed {
            return;
        }
        if user {
            fire!(self.st, on_toggle, key, on);
        }
        a11y::changed(&self.canvas);
        self.redraw_all();
    }

    fn toggle_id(&self, id: ItemId) {
        let Some(k) = self.key_of(id) else { return };
        let folded = self.is_collapsed(&k);
        let folds = self.st.borrow().flat.get(id).is_some_and(|r| r.2.folds);
        if folds {
            self.fold(&k, !folded, true);
        }
    }

    fn set_wide(&self, w: i32, store: bool) {
        let changed = {
            let mut s = self.st.borrow_mut();
            let w = w.clamp(s.min_w, s.max_w.max(s.min_w));
            let c = s.wide != w;
            s.wide = w;
            c
        };
        if store {
            let w = self.st.borrow().wide;
            if self.width.get() != w {
                self.width.set(w);
            }
        }
        if changed {
            self.apply();
        }
    }

    /// Pointer moved over a surface: hover, the action's tooltip, the edge.
    fn track(&self, surf: Surf, w: &mut Widget) {
        let area = self.area(surf);
        let rs = self.right_side();
        let (ex, ey) = app::event_coords();
        let h = {
            let mut s = self.st.borrow_mut();
            rebuild(&mut s, &self.model);
            hit(&s, &self.model, area, rs, ex, ey)
        };
        let (hover, tip) = match h {
            Some((id, part)) => {
                let it = self.item(id);
                match it {
                    Some(it) if it.disabled || it.kind == NavKind::Separator => (None, None),
                    Some(it)
                        if it.kind == NavKind::Section
                            && !self.st.borrow().flat.get(id).is_some_and(|r| r.2.folds)
                            && !matches!(part, Part::Action(_)) =>
                    {
                        (None, None)
                    }
                    Some(it) => {
                        let tip = match part {
                            Part::Action(a) => it.actions.get(a).map(|a| a.tooltip.clone()),
                            _ => None,
                        };
                        (Some((id, part)), tip)
                    }
                    None => (None, None),
                }
            }
            None => (None, None),
        };
        let changed = {
            let mut s = self.st.borrow_mut();
            std::mem::replace(&mut s.hover, hover) != hover
        };
        self.set_tip(surf, w, tip);
        self.edge_cursor(surf, w, ex);
        if changed {
            w.redraw();
        }
    }

    /// On the resize edge (expanded, resizable).
    fn on_edge(&self, surf: Surf, ex: i32) -> bool {
        if !self.st.borrow().resizable || !self.is_expanded() {
            return false;
        }
        let (x, _, w, _) = self.area(surf);
        let edge = if self.right_side() { x } else { x + w - 1 };
        (ex - edge).abs() <= 3
    }

    fn edge_cursor(&self, surf: Surf, w: &Widget, ex: i32) {
        let want = self.on_edge(surf, ex) || self.st.borrow().resize.is_some();
        if let Some(mut win) = w.window() {
            win.set_cursor(if want { Cursor::WE } else { Cursor::Default });
        }
    }

    fn set_tip(&self, surf: Surf, w: &mut Widget, tip: Option<String>) {
        let k = (surf == Surf::Layer) as usize;
        match tip {
            Some(t) if !t.is_empty() => {
                if w.tooltip().as_deref() != Some(t.as_str()) {
                    w.set_tooltip(&t);
                }
                self.st.borrow_mut().tip[k] = true;
            }
            _ => {
                if std::mem::replace(&mut self.st.borrow_mut().tip[k], false) {
                    w.set_tooltip("");
                }
            }
        }
    }

    fn push(&self, surf: Surf, w: &Widget) {
        let (ex, ey) = app::event_coords();
        if app::event_mouse_button() == app::MouseButton::Left && self.on_edge(surf, ex) {
            let wide = self.st.borrow().wide;
            self.st.borrow_mut().resize = Some((app::event_x_root(), wide));
            return;
        }
        let area = self.area(surf);
        let rs = self.right_side();
        let h = {
            let mut s = self.st.borrow_mut();
            rebuild(&mut s, &self.model);
            hit(&s, &self.model, area, rs, ex, ey)
        };
        let Some((id, part)) = h else { return };
        let Some(it) = self.item(id) else { return };
        let row = self.st.borrow().flat.get(id).map(|r| *r.2);
        if app::event_mouse_button() == app::MouseButton::Right {
            if row.is_some_and(|r| r.focusable) {
                self.st.borrow_mut().cursor = Some(id);
            }
            let (ox, oy) = window_origin(w);
            if self.context(id, ex + ox, ey + oy) {
                self.redraw_all();
            }
            return;
        }
        match part {
            Part::Action(a) => self.action(id, a),
            Part::Chevron => {
                self.st.borrow_mut().cursor = Some(id);
                self.toggle_id(id);
            }
            Part::Row => match it.kind {
                NavKind::Section => {
                    if row.is_some_and(|r| r.folds) {
                        self.st.borrow_mut().cursor = Some(id);
                        self.toggle_id(id);
                    }
                }
                NavKind::Separator => {}
                NavKind::Row => {
                    if it.disabled {
                        return;
                    }
                    if app::event_clicks() && row.is_some_and(|r| r.folds) {
                        self.toggle_id(id);
                        return;
                    }
                    self.activate(id);
                    if self.st.borrow().reorder != Reorder::Off {
                        self.st.borrow_mut().drag = Some(Drag {
                            id,
                            x0: ex,
                            y0: ey,
                            active: false,
                        });
                    }
                }
            },
        }
    }

    /// Dragging: resizing the edge, or moving a row.
    fn drag(&self, surf: Surf) -> bool {
        let resize = self.st.borrow().resize;
        if let Some((x0, w0)) = resize {
            let dx = app::event_x_root() - x0;
            let w = if self.right_side() { w0 - dx } else { w0 + dx };
            self.resize_live(w);
            return true;
        }
        let (ex, ey) = app::event_coords();
        let (id, active) = {
            let mut s = self.st.borrow_mut();
            let Some(d) = s.drag.as_mut() else {
                return false;
            };
            if !d.active && ((ex - d.x0).abs() > 5 || (ey - d.y0).abs() > 5) {
                d.active = true;
            }
            (d.id, d.active)
        };
        if !active {
            return true;
        }
        let area = self.area(surf);
        // Near the edges: scroll.
        let (_, ay, _, ah) = area;
        let step = theme::with(|t| metrics(t).rh / 3);
        if ey < ay + step || ey > ay + ah - step {
            let mut s = self.st.borrow_mut();
            let d = if ey < ay + step { -step } else { step };
            s.scroll = (s.scroll + d).max(0);
            let sc = s.scroll as i64;
            s.wheel.jump(sc);
        }
        let spot = self.spot_at(area, id, ey);
        let changed = std::mem::replace(&mut self.st.borrow_mut().spot, spot) != spot;
        if changed {
            self.redraw_all();
        }
        true
    }

    /// Where the dragged row `id` would go with the pointer at `ey`.
    fn spot_at(&self, area: (i32, i32, i32, i32), id: ItemId, ey: i32) -> Option<(ItemId, Place)> {
        let rs = self.right_side();
        let (tid, _) = {
            let s = self.st.borrow();
            hit(&s, &self.model, area, rs, area.0 + 1, ey)?
        };
        if tid == id {
            return None;
        }
        let rect = {
            let s = self.st.borrow();
            let pad = theme::with(|t| metrics(t).pad);
            let v = view_of(&s, area, pad);
            let (f, i, _) = s.flat.get(tid)?;
            v.row(&s.flat, f, i)
        };
        let rel = (ey - area.1 - rect.0) as f64 / rect.1.max(1) as f64;
        let anywhere = self.st.borrow().reorder == Reorder::Anywhere;
        let target = self.item(tid)?;
        let place = if anywhere && (0.3..0.7).contains(&rel) && target.kind != NavKind::Separator {
            Place::Into
        } else if rel < 0.5 {
            Place::Before
        } else {
            Place::After
        };
        let (parent, index) = self.destination(id, tid, place)?;
        let key = self.key_of(id)?;
        let ok = {
            let me = self.model.with_id(id, |t| t.parent.clone())?;
            let same = me == parent;
            (anywhere || same) && !self.is_ancestor(&key, &parent)
        };
        if !ok {
            return None;
        }
        let accept = self.st.borrow().accept_move.clone();
        if accept.is_some_and(|f| !f(&key, &parent, index)) {
            return None;
        }
        Some((tid, place))
    }

    /// `(new parent, index among its children without the moved row)`.
    fn destination(&self, id: ItemId, target: ItemId, place: Place) -> Option<(String, usize)> {
        let t = self.item(target)?;
        let key = self.key_of(id)?;
        let parent = if place == Place::Into {
            t.key.clone()
        } else {
            t.parent.clone()
        };
        let mut sibs = Vec::new();
        self.model.for_each(|_, it| {
            if it.parent == parent && it.key != key {
                sibs.push(it.key.clone());
            }
        });
        let index = match place {
            Place::Into => sibs.len(),
            Place::Before => sibs.iter().position(|k| *k == t.key)?,
            Place::After => sibs.iter().position(|k| *k == t.key)? + 1,
        };
        Some((parent, index))
    }

    /// True when `key` is `of` or one of its ancestors (no row goes into
    /// itself).
    fn is_ancestor(&self, key: &str, of: &str) -> bool {
        let mut cur = Some(of.to_string());
        let mut guard = 0;
        while let Some(c) = cur.filter(|c| !c.is_empty() && guard < 64) {
            if c == key {
                return true;
            }
            cur = self.model.with_key(&c, |t| t.parent.clone());
            guard += 1;
        }
        false
    }

    fn release(&self) -> bool {
        if self.st.borrow_mut().resize.take().is_some() {
            let w = self.st.borrow().wide;
            self.end_resize();
            self.set_wide(w, true);
            return true;
        }
        let drag = self.st.borrow_mut().drag.take();
        let spot = self.st.borrow_mut().spot.take();
        let Some(d) = drag.filter(|d| d.active) else {
            return false;
        };
        self.redraw_all();
        let Some((tid, place)) = spot else {
            return true;
        };
        let Some((parent, index)) = self.destination(d.id, tid, place) else {
            return true;
        };
        let Some(key) = self.key_of(d.id) else {
            return true;
        };
        fire!(self.st, on_move, &key, &parent, index);
        true
    }

    /// The width follows the pointer at once (no layout motion).
    fn resize_live(&self, w: i32) {
        {
            let mut s = self.st.borrow_mut();
            s.wide = w.clamp(s.min_w, s.max_w.max(s.min_w));
        }
        {
            // The overlay panel follows the pointer too.
            let mut s = self.st.borrow_mut();
            if s.layer.is_some() {
                let w = s.wide as f64;
                s.layer_w.jump(w);
            }
        }
        if let Some(p) = self.root_w().parent() {
            if crate::layout::animated(&p) {
                crate::layout::set_animated(&p, false);
                RESIZING.with(|r| r.borrow_mut().push(p.as_base_widget()));
            }
        }
        self.apply();
    }

    fn end_resize(&self) {
        RESIZING.with(|r| {
            for p in r.borrow_mut().drain(..) {
                if !p.was_deleted() {
                    crate::layout::set_animated(&p, true);
                }
            }
        });
    }

    fn wheel(&self, inside: bool) -> bool {
        let (_, area) = self.visible();
        let pad = theme::with(|t| metrics(t).pad);
        let max = {
            let mut s = self.st.borrow_mut();
            rebuild(&mut s, &self.model);
            view_of(&s, area, pad).max
        };
        if max == 0 {
            return inside;
        }
        let rh = theme::with(|t| metrics(t).rh);
        {
            let mut s = self.st.borrow_mut();
            let cur = s.scroll.clamp(0, max) as i64;
            let dy = app::event_dy_value() as f64 * rh as f64;
            s.scroll = s.wheel.wheel(cur, dy, max as i64) as i32;
        }
        self.redraw_all();
        true
    }

    fn can_drop(&self, id: ItemId) -> bool {
        let Some(it) = self.item(id) else {
            return false;
        };
        if it.kind != NavKind::Row || it.disabled {
            return false;
        }
        let f = self.st.borrow().accept_drop.clone();
        f.map_or(true, |f| f(&it.key))
    }

    /// External drag and drop (accept DndEnter/DndDrag/DndRelease; the text
    /// comes in Event::Paste).
    fn dnd(&self, surf: Surf, ev: Event) -> bool {
        {
            let s = self.st.borrow();
            if s.on_drop.is_none() && s.on_drop_files.is_none() {
                return false;
            }
        }
        match ev {
            Event::DndEnter | Event::DndDrag => {
                if ev == Event::DndEnter {
                    if surf == Surf::Rail {
                        self.entered();
                    } else {
                        self.st.borrow_mut().layer_hovered = true;
                    }
                }
                let area = self.area(surf);
                let rs = self.right_side();
                let (ex, ey) = app::event_coords();
                let row = {
                    let mut s = self.st.borrow_mut();
                    rebuild(&mut s, &self.model);
                    hit(&s, &self.model, area, rs, ex, ey).map(|h| h.0)
                };
                let target = row.filter(|&i| self.can_drop(i));
                let changed =
                    std::mem::replace(&mut self.st.borrow_mut().drop_target, target) != target;
                if changed {
                    self.redraw_all();
                }
                target.is_some()
            }
            Event::DndLeave => {
                {
                    let mut s = self.st.borrow_mut();
                    s.drop_target = None;
                    if surf == Surf::Layer {
                        s.layer_hovered = false;
                    }
                }
                self.left();
                self.redraw_all();
                true
            }
            Event::DndRelease => {
                let t = {
                    let mut s = self.st.borrow_mut();
                    let t = s.drop_target.take();
                    s.drop_pending = t;
                    t
                };
                self.redraw_all();
                t.is_some()
            }
            _ => false,
        }
    }

    fn paste(&self) -> bool {
        let Some(id) = self.st.borrow_mut().drop_pending.take() else {
            return false;
        };
        let Some(key) = self.key_of(id) else {
            return false;
        };
        let text = app::event_text();
        fire!(self.st, on_drop, &key, text.clone());
        if self.st.borrow().on_drop_files.is_some() {
            if let Some(paths) = crate::edit::parse_files(&text, true) {
                fire!(self.st, on_drop_files, &key, paths);
            }
        }
        self.redraw_all();
        true
    }

    /// Scrolls so the cursor row is in view (at once).
    fn reveal_cursor(&self) {
        let (_, area) = self.visible();
        let pad = theme::with(|t| metrics(t).pad);
        let expanded = self.is_expanded();
        let mut s = self.st.borrow_mut();
        rebuild(&mut s, &self.model);
        let Some(id) = s.cursor else { return };
        let mut v = view_of(&s, area, pad);
        v.reveal = if expanded { 1.0 } else { 0.0 };
        let content = s.flat.content(v.reveal);
        v.max = (content - (v.bottom - v.top)).max(0);
        let Some((false, i, _)) = s.flat.get(id) else {
            return;
        };
        let (mut y0, y1) = (s.flat.top_y(i, v.reveal), s.flat.top_y(i + 1, v.reveal));
        let view = v.bottom - v.top;
        // Headers and separators right above it come along when they fit.
        let mut j = i;
        while j > 0 && !s.flat.top[j - 1].focusable && y1 - s.flat.top_y(j - 1, v.reveal) <= view {
            j -= 1;
        }
        y0 = y0.min(s.flat.top_y(j, v.reveal));
        let sc = s.scroll.clamp(0, v.max);
        let t = if y0 < sc {
            y0
        } else if y1 > sc + view {
            (y1 - view).min(v.max)
        } else {
            sc
        };
        s.scroll = t;
        s.wheel.jump(t as i64);
    }

    fn key(&self, w: &mut Widget) -> bool {
        let raw = app::event_key();
        let k = crate::layout::logical_key(&self.root_w(), raw);
        let mods = app::event_state();
        let cmd = mods.intersects(Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta);
        let shift = mods.contains(Shortcut::Shift);
        let plain = !cmd && !shift;
        if k == Key::Escape {
            if self.is_expanded() && !self.is_pinned() {
                self.collapse();
                return true;
            }
            return false;
        }
        let (order, cur, pos) = {
            let mut s = self.st.borrow_mut();
            rebuild(&mut s, &self.model);
            let order: Vec<FRow> = s.flat.order().filter(|r| r.focusable).copied().collect();
            let cur = s.cursor;
            let pos = cur.and_then(|c| order.iter().position(|r| r.id == c));
            (order, cur, pos)
        };
        if order.is_empty() || cmd {
            return false;
        }
        let near = || {
            let s = self.st.borrow();
            let all: Vec<ItemId> = s.flat.order().map(|r| r.id).collect();
            let at = cur
                .and_then(|c| all.iter().position(|&x| x == c))
                .unwrap_or(0);
            order
                .iter()
                .position(|r| all.iter().position(|&x| x == r.id).unwrap_or(0) >= at)
                .unwrap_or(0)
        };
        let page = {
            let (_, (_, _, _, h)) = self.visible();
            theme::with(|t| ((h - 2 * metrics(t).pad) / metrics(t).rh).max(1)) as usize
        };
        let last = order.len() - 1;
        let row = pos.map(|p| order[p]);
        let next = match k {
            Key::Up if plain => Some(pos.map_or_else(near, |p| p.saturating_sub(1))),
            Key::Down if plain => Some(pos.map_or_else(near, |p| (p + 1).min(last))),
            Key::PageUp if plain => Some(pos.map_or_else(near, |p| p.saturating_sub(page))),
            Key::PageDown if plain => Some(pos.map_or_else(near, |p| (p + page).min(last))),
            Key::Home if plain => Some(0),
            Key::End if plain => Some(last),
            Key::Right if plain => match row {
                Some(r) if r.folds && !r.open => {
                    self.toggle_id(r.id);
                    return true;
                }
                Some(r) if r.folds && r.kind == NavKind::Row => {
                    let p = pos.unwrap_or(0);
                    order
                        .get(p + 1)
                        .filter(|n| n.parent == Some(r.id))
                        .map(|_| p + 1)
                }
                _ => None,
            },
            Key::Left if plain => match row {
                Some(r) if r.folds && r.open => {
                    self.toggle_id(r.id);
                    return true;
                }
                Some(r) => r.parent.and_then(|p| order.iter().position(|x| x.id == p)),
                None => None,
            },
            _ => None,
        };
        if let Some(p) = next {
            self.st.borrow_mut().cursor = Some(order[p].id);
            self.st.borrow_mut().cursor_pos = p;
            if !self.is_expanded() {
                self.expand();
            }
            self.reveal_cursor();
            w.redraw();
            self.redraw_all();
            return true;
        }
        if matches!(
            k,
            Key::Left
                | Key::Right
                | Key::Up
                | Key::Down
                | Key::Home
                | Key::End
                | Key::PageUp
                | Key::PageDown
        ) {
            return matches!(k, Key::Left | Key::Right)
                && row.is_some_and(|r| r.parent.is_some() || r.folds);
        }
        let Some(r) = row else {
            return false;
        };
        let Some(it) = self.item(r.id) else {
            return false;
        };
        let space = k == Key::from_char(' ');
        let typing = !self.st.borrow().typed.is_empty()
            && self
                .st
                .borrow()
                .typed_at
                .is_some_and(|t| t.elapsed().as_millis() < 800);
        match k {
            Key::Enter | Key::KPEnter if plain => {
                if it.kind == NavKind::Row {
                    self.activate(r.id);
                } else {
                    self.toggle_id(r.id);
                }
                true
            }
            _ if space && plain && !typing => {
                if it.kind == NavKind::Row {
                    self.activate(r.id);
                } else {
                    self.toggle_id(r.id);
                }
                true
            }
            Key::Delete if plain => match it.actions.iter().position(|a| a.enabled) {
                Some(a) if !it.busy => {
                    self.action(r.id, a);
                    true
                }
                _ => false,
            },
            _ if (k == Key::Menu && plain) || (k == Key::F10 && shift && !cmd) => {
                let Some((x, y, rw, rh)) = self.item_rect(&it.key) else {
                    return false;
                };
                let inset = theme::with(|t| metrics(t).inset);
                let px = if self.right_side() {
                    x + rw - inset
                } else {
                    x + inset
                };
                let (ox, oy) = window_origin(&self.canvas);
                self.context(r.id, px + ox, y + rh + oy)
            }
            _ if !cmd => {
                // Type-ahead: jump to the next row whose title starts with the typed text.
                let text = app::event_text();
                let Some(c) = text
                    .chars()
                    .next()
                    .filter(|c| !c.is_control() && (!c.is_whitespace() || typing))
                else {
                    return false;
                };
                let prefix = {
                    let mut s = self.st.borrow_mut();
                    if !s.typed_at.is_some_and(|t| t.elapsed().as_millis() < 800) {
                        s.typed.clear();
                    }
                    s.typed.extend(c.to_lowercase());
                    s.typed_at = Some(Instant::now());
                    s.typed.clone()
                };
                let start = pos.unwrap_or(0);
                let single = prefix.chars().count() == 1;
                let n = order.len();
                let found = (0..n)
                    .map(|d| (start + d + single as usize) % n)
                    .find(|&p| {
                        self.model
                            .with_id(order[p].id, |t| t.title.to_lowercase().starts_with(&prefix))
                            .unwrap_or(false)
                    });
                if let Some(p) = found {
                    self.st.borrow_mut().cursor = Some(order[p].id);
                    if !self.is_expanded() {
                        self.expand();
                    }
                    self.reveal_cursor();
                    self.redraw_all();
                }
                true
            }
            _ => false,
        }
    }

    /// Expanded = pinned or open; moves the width (push) or the layer (overlay).
    fn apply(&self) {
        let want = self.pinned.get() || self.st.borrow().open;
        if self.expanded.get() != want {
            self.expanded.set(want);
        }
        let (mode, rail, wide) = {
            let s = self.st.borrow();
            (s.mode, s.rail, s.wide)
        };
        match mode {
            SidebarMode::Push => {
                self.show_slots(want);
                let target = if want { wide } else { rail };
                let root = self.root_w();
                if root.w() != target && !crate::layout::refit(&root, (target, root.h())) {
                    let mut w = root.clone();
                    let h = w.h();
                    w.set_size(target, h);
                    if let Some(mut p) = w.parent() {
                        p.redraw();
                    }
                }
            }
            SidebarMode::Overlay => {
                if want {
                    self.open_layer();
                }
                let layer = self.st.borrow().layer.clone();
                if let Some(mut l) = layer {
                    let r = self.layer_rect();
                    if !l.was_deleted() && (l.w(), l.h()) != (r.2, r.3) {
                        l.resize(r.0, r.1, r.2, r.3);
                    }
                    l.redraw();
                }
            }
        }
        a11y::changed(&self.canvas);
        self.canvas.clone().redraw();
    }

    fn show_slots(&self, on: bool) {
        let host = self.st.borrow().host.clone();
        let Some(mut host) = host else { return };
        let slots: Vec<Widget> = self.st.borrow().slots.iter().map(|s| s.w.clone()).collect();
        let mut changed = false;
        for mut w in slots {
            if w.visible() != on {
                changed = true;
                if on {
                    w.show();
                } else {
                    w.hide();
                }
            }
        }
        if changed {
            host.layout();
            host.redraw();
        }
    }

    /// The overlay panel's rectangle, in top-level window coordinates.
    fn layer_rect(&self) -> (i32, i32, i32, i32) {
        let wide = self.st.borrow().wide;
        let (ox, oy) = window_origin(&self.canvas);
        let r = self.root_w();
        let x = if self.right_side() {
            r.x() + r.w() - wide
        } else {
            r.x()
        };
        (x + ox, r.y() + oy, wide, r.h())
    }

    fn open_layer(&self) {
        if self
            .st
            .borrow()
            .layer
            .as_ref()
            .is_some_and(|l| !l.was_deleted())
        {
            return;
        }
        let rail = self.st.borrow().rail;
        let has_slots = !self.st.borrow().slots.is_empty();
        let rect = self.layer_rect();
        let opened = if has_slots {
            crate::overlay::open_group_layer(&self.canvas, rect)
                .map(|g| (g.as_base_widget(), Some(g)))
        } else {
            crate::overlay::open_layer(&self.canvas, Some(rect), false).map(|l| (l, None))
        };
        let Some((layer, group)) = opened else { return };
        {
            let mut s = self.st.borrow_mut();
            s.layer_w = Anim::new();
            s.layer_w.jump(rail as f64);
            s.layer = Some(layer.clone());
            s.rm_layer = RowMotion::default();
            s.tip[1] = false;
        }
        match group {
            Some(mut g) => {
                // The header widgets ride in the panel while it is open.
                let slots: Vec<Widget> =
                    self.st.borrow().slots.iter().map(|s| s.w.clone()).collect();
                for mut w in slots {
                    g.add(&w);
                    w.show();
                }
                g.super_draw_first(false);
                self.install_layer(g);
            }
            None => {
                self.install_layer(layer);
            }
        }
    }

    /// Draw and events of the overlay panel.
    fn install_layer<L: WidgetBase + WidgetExt + Clone + 'static>(&self, mut layer: L) {
        let me = self.clone();
        layer.draw(move |l| {
            let l = &mut l.as_base_widget();
            let r = me.layer_rect();
            if (l.x(), l.y(), l.w(), l.h()) != r {
                l.resize(r.0, r.1, r.2, r.3);
            }
            let rs = me.right_side();
            let mut s = me.st.borrow_mut();
            let want = me.expanded.get();
            let (rail, wide) = (s.rail as f64, s.wide as f64);
            let w = s
                .layer_w
                .get(
                    if want { wide } else { rail },
                    anim::ms::LAYOUT,
                    Ease::OutCubic,
                    l,
                )
                .round() as i32;
            let closing = !want && !s.layer_w.running();
            let x = if rs { l.x() + l.w() - w } else { l.x() };
            // Slots: header widgets at the top, footer widgets at the bottom.
            let (mut top, mut bottom) = (l.y(), l.y() + l.h());
            for slot in s.slots.iter_mut() {
                let y = if slot.footer {
                    bottom -= slot.h;
                    bottom
                } else {
                    top += slot.h;
                    top - slot.h
                };
                if (slot.w.x(), slot.w.y(), slot.w.w(), slot.w.h()) != (x, y, w, slot.h) {
                    slot.w.resize(x, y, w, slot.h);
                }
            }
            if top > l.y() || bottom < l.y() + l.h() {
                theme::with(|t| {
                    let bg = rows::surface(t);
                    crate::glass::fill(x, l.y(), w, top - l.y(), 0, bg);
                    crate::glass::fill(x, bottom, w, l.y() + l.h() - bottom, 0, bg);
                });
            }
            let ring = crate::focus::show_ring(&me.canvas);
            let sel = me.selected.get();
            paint_surface(
                &mut s,
                &me.model,
                Surf::Layer,
                &sel,
                (x, top, w, (bottom - top).max(0)),
                rs,
                ring,
                true,
                l,
            );
            if closing {
                s.layer = None;
                let slots: Vec<(Widget, bool)> =
                    s.slots.iter().map(|x| (x.w.clone(), x.footer)).collect();
                drop(s);
                me.return_slots(slots);
                let l2 = l.clone();
                app::add_timeout3(0.0, move |_| crate::overlay::close_layer(&l2));
            }
        });
        let me = self.clone();
        layer.handle(move |l, ev| {
            let l = &mut l.as_base_widget();
            let (_, area) = me.visible();
            let width = me.st.borrow().layer_w.target().unwrap_or(0.0) as i32;
            let rs = me.right_side();
            let ex = app::event_x();
            let inside_panel = if rs {
                ex >= l.x() + l.w() - width
            } else {
                ex < l.x() + width
            };
            let ey = app::event_y();
            let in_rows = inside_panel && ey >= area.1 && ey < area.1 + area.3;
            match ev {
                Event::Enter | Event::Move => {
                    let was = me.st.borrow().layer_hovered;
                    me.st.borrow_mut().layer_hovered = inside_panel;
                    if in_rows {
                        me.track(Surf::Layer, l);
                    } else {
                        me.st.borrow_mut().hover = None;
                        me.set_tip(Surf::Layer, l, None);
                    }
                    if was && !inside_panel {
                        me.left();
                    }
                    l.redraw();
                    // Slots inside get their own events.
                    in_rows
                }
                Event::Leave => {
                    me.st.borrow_mut().layer_hovered = false;
                    me.set_tip(Surf::Layer, l, None);
                    me.left();
                    false
                }
                Event::Push if in_rows => {
                    me.push(Surf::Layer, l);
                    true
                }
                Event::Drag => me.drag(Surf::Layer),
                Event::Released => me.release(),
                Event::MouseWheel if in_rows => me.wheel(true),
                Event::DndEnter | Event::DndDrag if in_rows => me.dnd(Surf::Layer, ev),
                Event::DndLeave | Event::DndRelease => me.dnd(Surf::Layer, ev),
                Event::Paste => me.paste(),
                _ => false,
            }
        });
    }

    /// Puts the header widgets back in the column (hidden while collapsed).
    fn return_slots(&self, slots: Vec<(Widget, bool)>) {
        let host = self.st.borrow().host.clone();
        let Some(mut host) = host else { return };
        let mut above = 0;
        for (w, footer) in slots {
            if w.was_deleted() {
                continue;
            }
            let at = if footer { host.children() } else { above };
            host.insert(&w, at);
            if !footer {
                above += 1;
            }
            let h = w.h();
            host.fixed(&w, h);
        }
        self.show_slots(self.is_expanded());
        host.layout();
    }

    fn install(&self) {
        let me = self.clone();
        let mut canvas = self.canvas.clone();
        canvas.draw(move |w| {
            if me.kick.replace(false) {
                me.st.borrow_mut().dirty = true;
            }
            let rs = me.right_side();
            let mut s = me.st.borrow_mut();
            let overlay_open = s.mode == SidebarMode::Overlay
                && s.layer.as_ref().is_some_and(|l| !l.was_deleted());
            let ring = crate::focus::show_ring(w) && !overlay_open;
            let sel = me.selected.get();
            paint_surface(
                &mut s,
                &me.model,
                Surf::Rail,
                &sel,
                (w.x(), w.y(), w.w(), w.h()),
                rs,
                ring,
                !overlay_open,
                w,
            );
        });
        let me = self.clone();
        canvas.handle(move |w, ev| match ev {
            Event::Enter => {
                if me.st.borrow().host.is_none() {
                    me.entered();
                }
                me.track(Surf::Rail, w);
                true
            }
            Event::Leave => {
                if me.st.borrow().host.is_none() {
                    me.left();
                } else {
                    me.st.borrow_mut().hover = None;
                    w.redraw();
                }
                me.set_tip(Surf::Rail, w, None);
                if let Some(mut win) = w.window() {
                    if me.st.borrow().resize.is_none() {
                        win.set_cursor(Cursor::Default);
                    }
                }
                true
            }
            Event::Move => {
                me.track(Surf::Rail, w);
                true
            }
            Event::Push => {
                if app::event_mouse_button() != app::MouseButton::Right {
                    let _ = w.take_focus();
                }
                me.push(Surf::Rail, w);
                true
            }
            Event::Drag => me.drag(Surf::Rail),
            Event::Released => me.release(),
            Event::MouseWheel => me.wheel(false),
            Event::Focus => {
                me.st.borrow_mut().open = true;
                if me.st.borrow().cursor.is_none() {
                    let sel = me.selected.get();
                    let id = me
                        .model
                        .id_of(&sel)
                        .filter(|id| me.st.borrow().flat.get(*id).is_some());
                    let first = {
                        let mut s = me.st.borrow_mut();
                        rebuild(&mut s, &me.model);
                        let id = s.flat.order().find(|r| r.focusable).map(|r| r.id);
                        id
                    };
                    me.st.borrow_mut().cursor = id.or(first);
                }
                me.apply();
                true
            }
            Event::Unfocus => {
                // Collapses when focus left the tree (not into its own field).
                let gen = {
                    let mut s = me.st.borrow_mut();
                    s.unfocus_gen += 1;
                    s.unfocus_gen
                };
                let me2 = me.clone();
                app::add_timeout3(0.0, move |_| {
                    if me2.canvas.was_deleted() || me2.st.borrow().unfocus_gen != gen {
                        return;
                    }
                    let hovered = {
                        let s = me2.st.borrow();
                        s.hovered || s.layer_hovered
                    };
                    if !hovered && !me2.holds_focus() {
                        me2.collapse();
                    }
                });
                w.redraw();
                true
            }
            Event::KeyDown => me.key(w),
            Event::DndEnter | Event::DndDrag | Event::DndLeave | Event::DndRelease => {
                me.dnd(Surf::Rail, ev)
            }
            Event::Paste => me.paste(),
            _ => false,
        });
        let mut w2 = self.canvas.clone();
        let st2 = Rc::downgrade(&self.st);
        crate::state::bind_widget(&self.canvas, &self.selected, false, move |_| {
            w2.redraw();
            if let Some(st) = st2.upgrade() {
                if let Some(mut l) = st.borrow().layer.clone().filter(|l| !l.was_deleted()) {
                    l.redraw();
                }
            }
        });
        // Live model changes.
        let weak: Weak<RefCell<Nt>> = Rc::downgrade(&self.st);
        let kick = self.kick.clone();
        let canvas = self.canvas.clone();
        let sub = self.model.subscribe(move |m, ch| {
            let Some(st) = weak.upgrade() else { return };
            if canvas.was_deleted() {
                return;
            }
            let redraw = || {
                canvas.clone().redraw();
                if let Ok(s) = st.try_borrow() {
                    if let Some(mut l) = s.layer.clone().filter(|l| !l.was_deleted()) {
                        l.redraw();
                    }
                }
            };
            let Ok(mut s) = st.try_borrow_mut() else {
                kick.set(true);
                redraw();
                return;
            };
            if s.paused {
                return;
            }
            let mut structural =
                ch.is_structural() || !ch.inserted().is_empty() || !ch.removed().is_empty();
            if !structural {
                let searching = s.flat.searching;
                structural = ch.updated().iter().any(|id| {
                    m.with_id(*id, |t| flat::signature(t, searching)) != s.flat.sig.get(id).copied()
                });
            }
            if structural {
                s.dirty = true;
                s.moved |= ch.is_structural();
                s.inserted.extend_from_slice(ch.inserted());
                drop(s);
                a11y::changed(&canvas);
                redraw();
            } else if ch.updated().iter().any(|id| s.drawn.contains(id)) {
                drop(s);
                a11y::changed(&canvas);
                redraw();
            }
        });
        self.st.borrow_mut()._sub = Some(sub);
        // Row heights come from the theme's fonts and spacing.
        let (weak, kick, mut canvas) = (
            Rc::downgrade(&self.st),
            self.kick.clone(),
            self.canvas.clone(),
        );
        theme::on_change_while(&self.canvas, move |_| {
            match weak.upgrade().as_ref().map(|st| st.try_borrow_mut()) {
                Some(Ok(mut s)) => s.dirty = true,
                _ => kick.set(true),
            }
            canvas.redraw();
        });
        self.install_a11y();
    }

    fn install_a11y(&self) {
        let (sel, ex, me) = (self.selected.clone(), self.expanded.clone(), self.clone());
        a11y::describe(&self.canvas, move |n| {
            let Ok(s) = me.st.try_borrow() else { return };
            let rs = me.right_side();
            let (_, area) = Self::visible_of(&s, &me.canvas, rs);
            let name = if s.name.is_empty() {
                crate::i18n::tr("Sidebar")
            } else {
                s.name.clone()
            };
            n.role(if s.tabs { Role::TabList } else { Role::Tree })
                .name(&name)
                .expanded(ex.get())
                .focusable();
            let sel_key = sel.get();
            theme::with(|t| {
                let m = metrics(t);
                let v = view_of(&s, area, m.pad);
                let mir = Mirror::new(rs, v.x, v.w);
                // Rows near the view (and the footer) as virtual children.
                let r = v.visible(&s.flat);
                let lo = r.start.saturating_sub(20);
                let hi = (r.end + 20).min(s.flat.top.len());
                let rows: Vec<(bool, usize)> = (lo..hi)
                    .map(|i| (false, i))
                    .chain((0..s.flat.foot.len()).map(|i| (true, i)))
                    .collect();
                let tabs_total = s.flat.order().filter(|r| r.kind == NavKind::Row).count();
                let mut tab_pos = s.flat.top[..lo]
                    .iter()
                    .filter(|r| r.kind == NavKind::Row)
                    .count();
                for (foot, i) in rows {
                    let row = *s.flat.row(foot, i);
                    if row.kind == NavKind::Separator {
                        continue;
                    }
                    let Some(it) = item_of(&s, &me.model, row.id) else {
                        continue;
                    };
                    let (ry, rh) = v.row(&s.flat, foot, i);
                    let shown = foot || (ry + rh > v.top && ry < v.bottom);
                    if row.kind == NavKind::Row {
                        tab_pos += 1;
                    }
                    n.child(row.id, |c| {
                        if shown {
                            c.bounds(v.x, v.y + ry, v.w, rh);
                        }
                        let section = row.kind == NavKind::Section;
                        if s.tabs {
                            if section {
                                c.role(Role::Heading).name(&it.title).level(2);
                            } else {
                                c.role(Role::Tab).position(tab_pos, tabs_total);
                            }
                        } else {
                            c.role(Role::TreeItem)
                                .level(row.depth as usize + 1)
                                .position(row.pos as usize, row.of as usize);
                        }
                        if !section || !s.tabs {
                            c.name(&it.title);
                        }
                        if row.folds {
                            c.expanded(row.open).action(if row.open {
                                Action::Collapse
                            } else {
                                Action::Expand
                            });
                        }
                        if row.kind == NavKind::Row {
                            c.selected(sel_key == it.key).disabled(it.disabled);
                            if it.active {
                                c.current(true);
                            }
                        }
                        if row.focusable {
                            c.action(Action::Click).action(Action::Focus).focusable();
                        }
                        if s.on_context.is_some() && !it.disabled {
                            c.action(Action::ShowContextMenu);
                        }
                        let mut desc: Vec<&str> = vec![];
                        if !it.detail.is_empty() {
                            desc.push(&it.detail);
                        }
                        for b in &it.badges {
                            if let Badge::Text { text, .. } = b {
                                desc.push(text);
                            }
                        }
                        if !it.count.is_empty() {
                            desc.push(&it.count);
                        }
                        if row.folds && !row.open && !it.summary.is_empty() {
                            desc.push(&it.summary);
                        }
                        if !desc.is_empty() {
                            c.description(&desc.join(", "));
                        }
                        if s.cursor == Some(row.id) {
                            c.focused();
                        }
                        let l = shown.then(|| {
                            rows::layout(
                                t,
                                &m,
                                s.rail,
                                &it,
                                &row,
                                (v.x, v.y + ry, v.w, rh),
                                v.reveal,
                                true,
                            )
                        });
                        for (j, a) in it.actions.iter().enumerate() {
                            c.child(1000 + j as u64, |b| {
                                b.role(Role::Button)
                                    .name(&a.tooltip)
                                    .disabled(!a.enabled || it.busy || it.disabled);
                                if a.enabled && !it.busy && !it.disabled {
                                    b.action(Action::Click);
                                }
                                if let Some(&(_, (bx, by, bw, bh))) = l
                                    .as_ref()
                                    .and_then(|l| l.actions.iter().find(|x| x.0 == j))
                                    .filter(|_| v.reveal >= 0.5)
                                {
                                    b.bounds(mir.x(bx, bw), by, bw, bh);
                                }
                            });
                        }
                        if let Some(u) = it.usage {
                            c.child(999, |p| {
                                p.role(Role::ProgressIndicator)
                                    .name(if it.detail.is_empty() {
                                        &it.title
                                    } else {
                                        &it.detail
                                    })
                                    .numeric(u, 0.0, 1.0, 0.01);
                            });
                        }
                    });
                }
            });
        });
        let me = self.clone();
        a11y::on_action(&self.canvas, move |r| {
            let Some(&id) = r.path.first() else {
                return false;
            };
            let Some(it) = me.item(id) else { return false };
            if r.path.len() == 2 {
                let j = r.path[1];
                if (1000..1000 + it.actions.len() as u64).contains(&j) && r.action == Action::Click
                {
                    me.action(id, (j - 1000) as usize);
                    return true;
                }
                return false;
            }
            match r.action {
                Action::Click if it.kind == NavKind::Row => me.activate(id),
                Action::Click => me.toggle_id(id),
                Action::Expand => me.fold(&it.key, false, true),
                Action::Collapse => me.fold(&it.key, true, true),
                Action::Focus | Action::ScrollIntoView => {
                    me.st.borrow_mut().cursor = Some(id);
                    let _ = me.canvas.clone().take_focus();
                    me.reveal_cursor();
                    me.redraw_all();
                }
                Action::ShowContextMenu => {
                    let Some((x, y, _, rh)) = me.item_rect(&it.key) else {
                        return false;
                    };
                    let (ox, oy) = window_origin(&me.canvas);
                    return me.context(id, x + ox, y + rh + oy);
                }
                _ => return false,
            }
            true
        });
    }
}

/// Applies a move reported by [`NavTree::on_move`] to the model: `key`
/// becomes the `index`-th child of `parent` (in model order, the order
/// the tree shows), in one notification; the rows slide.
pub fn move_item(model: &ListModel<String, NavItem>, key: &str, parent: &str, index: usize) {
    let k = key.to_string();
    let Some(from) = model.index_of_key(&k) else {
        return;
    };
    let mut sibs = Vec::new();
    model.for_each(|i, it| {
        if it.parent == parent && it.key != key {
            sibs.push(i);
        }
    });
    // Positions in the list without the moved item.
    let p = |i: usize| if i > from { i - 1 } else { i };
    let to = match (sibs.get(index), sibs.last()) {
        (Some(&s), _) => p(s),
        (None, Some(&l)) => p(l) + 1,
        (None, None) => model
            .index_of_key(&parent.to_string())
            .map(|i| p(i) + 1)
            .unwrap_or(model.len().saturating_sub(1)),
    };
    model.transaction(|m| {
        m.patch(&k, |t| t.parent = parent.into());
        m.move_to(from, to);
    });
}

thread_local! {
    /// Containers whose layout motion is off while the user drags the edge.
    static RESIZING: RefCell<Vec<Widget>> = const { RefCell::new(Vec::new()) };
}
