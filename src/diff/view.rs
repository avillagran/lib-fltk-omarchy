//! [`DiffView`]: a virtualized canvas for a [`Patch`]. One widget, no
//! widget per line; each frame draws only the rows in view.

use super::parse::{DiffError, FileStatus, LineKind, Patch, NONE};
use super::rows::{self, Cells, Options, Row, RowKind};
use crate::a11y::{self, Action, Role};
use crate::panel::{ItemId, KeyboardPanel};
use crate::state::State;
use crate::theme::{self, Rgb, Theme};
use crate::widgets::glyph;
use crate::{paint, tr, trn};
use fltk::{app, draw, enums::*, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

/// Scrollbar thickness.
const BAR: i32 = 4;
/// Context lines asked for by one expand.
const EXPAND_STEP: u32 = 20;

/// The app is asked for more context lines: new-file lines `first..=last`
/// of `path` (they are equal in the old file). Answer with
/// [`DiffView::provide_context`]; for a request `below` the end of the
/// file, fewer lines (or none) mean the file ends there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandRequest {
    pub file: usize,
    pub path: String,
    pub hunk: usize,
    pub first: u32,
    pub last: u32,
    pub below: bool,
}

/// A find match: line `line` of hunk `hunk` of file `file`, bytes `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Match {
    file: u32,
    hunk: u32,
    line: u32,
    start: u32,
    end: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Metrics {
    font: i32,
    cw: f64,
    rh: i32,
    head_h: i32,
    hunk_h: i32,
    note_h: i32,
    mark_w: i32,
    pad: i32,
    /// Baseline offset of mono text inside a line of `rh`.
    base: i32,
}

/// Columns and widths for a widget width.
#[derive(Debug, Clone, Copy)]
struct Geo {
    /// Width of the rows (without the scrollbar).
    w: i32,
    /// Unified: gutter (numbers) width; split: per half.
    gutter: i32,
    /// Code area: left of the (first) code column and its width.
    code_x: i32,
    code_w: i32,
    /// Split: left of the right half.
    half: i32,
}

/// Where the cursor or the top of the view is, by content (survives a
/// rebuild that changes row IDs: layout switch, expanded context).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Row(u64),
    Line { file: u32, hunk: u32, line: u32 },
}

type Listeners = Vec<Box<dyn FnMut()>>;

struct St {
    patch: Patch,
    rows: Vec<Row>,
    ids: Rc<RefCell<Vec<u64>>>,
    ys: Vec<i64>,
    heights_key: Option<(i32, i32, u64, bool, bool, u64)>,
    rows_gen: u64,
    opts: Options,
    wrap: bool,
    word_diff: bool,
    max_cols: usize,
    max_line_no: u32,
    top: i64,
    motion: crate::scroll::Scroll,
    hx: i32,
    panel: KeyboardPanel,
    last_cursor: Option<ItemId>,
    hover: Option<usize>,
    drag: Option<ItemId>,
    pending_top: Option<(Anchor, i64)>,
    metrics: Metrics,
    metrics_key: Option<(i32, String)>,
    find: Vec<Match>,
    find_query: String,
    find_current: Option<usize>,
    expand: Option<Box<dyn FnMut(ExpandRequest)>>,
    on_file: Vec<Box<dyn FnMut(usize)>>,
    current_file: Option<usize>,
    patch_listeners: Listeners,
    split_state: Option<State<bool>>,
    wrap_state: Option<State<bool>>,
    context_state: Option<State<bool>>,
    drawn: usize,
}

/// A virtualized diff: unified or side by side, line numbers, add/delete
/// tints, word highlights, folding hunks and files, wrap or horizontal
/// scrolling, selection and copy, find. See the [module docs](super) and
/// docs/DIFF.md.
#[derive(Clone)]
pub struct DiffView {
    widget: Widget,
    st: Rc<RefCell<St>>,
}
fltk::widget_extends!(DiffView, Widget, widget);

impl Default for DiffView {
    fn default() -> Self {
        DiffView::new()
    }
}

impl DiffView {
    pub fn new() -> DiffView {
        let mut widget = Widget::default().with_size(640, 400);
        widget.set_frame(FrameType::NoBox);
        widget.set_visible_focus();
        let panel = KeyboardPanel::new("diff");
        let st = Rc::new(RefCell::new(St {
            patch: Patch::default(),
            rows: Vec::new(),
            ids: Rc::default(),
            ys: vec![0],
            heights_key: None,
            rows_gen: 0,
            opts: Options::default(),
            wrap: false,
            word_diff: true,
            max_cols: 0,
            max_line_no: 0,
            top: 0,
            motion: crate::scroll::Scroll::new(),
            hx: 0,
            panel: panel.clone(),
            last_cursor: None,
            hover: None,
            drag: None,
            pending_top: None,
            metrics: Metrics::default(),
            metrics_key: None,
            find: Vec::new(),
            find_query: String::new(),
            find_current: None,
            expand: None,
            on_file: Vec::new(),
            current_file: None,
            patch_listeners: Vec::new(),
            split_state: None,
            wrap_state: None,
            context_state: None,
            drawn: 0,
        }));
        let view = DiffView { widget, st };
        let s2 = view.st.clone();
        view.widget.clone().draw(move |w| draw_view(w, &s2));
        let me = view.downgrade();
        view.widget.clone().handle(move |w, ev| me.upgrade().is_some_and(|v| v.handle(w, ev)));
        // Code stays left to right in every language (AGENTS.md rule 12).
        crate::layout::set_direction(&view.widget, Some(crate::layout::Direction::Ltr));
        crate::focus::host(&view.widget, &[&panel]);
        let me = view.downgrade();
        panel.on_key(move |k, mods| me.upgrade().is_some_and(|v| v.key(k, mods)));
        let me = view.downgrade();
        panel.on_activate(move |id| {
            if let Some(v) = me.upgrade() {
                v.activate(id);
            }
        });
        let me = view.downgrade();
        panel.on_change(move || {
            if let Some(v) = me.upgrade() {
                v.cursor_moved();
            }
        });
        view.describe();
        view
    }

    pub(crate) fn downgrade(&self) -> WeakView {
        WeakView { widget: self.widget.clone(), st: Rc::downgrade(&self.st) }
    }

    // ------------------------------------------------------------ content

    /// Parses `text` with the default limits and shows it. On error the
    /// view keeps its patch.
    pub fn set_text(&self, text: &str) -> Result<(), DiffError> {
        self.set_patch(super::parse(text)?);
        Ok(())
    }

    /// Shows a patch: folds, find and scroll position start over.
    pub fn set_patch(&self, patch: Patch) {
        {
            let mut s = self.st.borrow_mut();
            s.max_cols = patch.files.iter().flat_map(|f| f.hunks.iter()).flat_map(|h| h.lines.iter()).map(|l| rows::cols(patch.text(l))).max().unwrap_or(0);
            s.max_line_no = max_line_no(&patch);
            s.patch = patch;
            s.opts.folded.clear();
            s.opts.collapsed.clear();
            s.opts.at_end.clear();
            s.find.clear();
            s.find_current = None;
            s.top = 0;
            s.motion.jump(0);
            s.hx = 0;
            s.last_cursor = None;
            s.current_file = None;
            s.pending_top = None;
        }
        let panel = self.panel();
        panel.set_cursor(None);
        panel.clear_selection();
        self.rebuild(false);
        let q = self.st.borrow().find_query.clone();
        if !q.is_empty() {
            self.find(&q);
        }
        self.notify_patch();
    }

    /// Reads the patch.
    pub fn with_patch<R>(&self, f: impl FnOnce(&Patch) -> R) -> R {
        f(&self.st.borrow().patch)
    }

    /// The keyboard panel of the rows (cursor, selection by row ID).
    pub fn panel(&self) -> KeyboardPanel {
        self.st.borrow().panel.clone()
    }

    fn notify_patch(&self) {
        let mut ls = std::mem::take(&mut self.st.borrow_mut().patch_listeners);
        for f in ls.iter_mut() {
            f();
        }
        let mut s = self.st.borrow_mut();
        ls.append(&mut s.patch_listeners);
        s.patch_listeners = ls;
    }

    pub(crate) fn on_patch(&self, f: impl FnMut() + 'static) {
        self.st.borrow_mut().patch_listeners.push(Box::new(f));
    }

    // ------------------------------------------------------------ options

    /// Side by side (true) or unified (false, the default).
    pub fn split(self, on: bool) -> Self {
        self.set_split(on);
        self
    }

    pub fn set_split(&self, on: bool) {
        if self.st.borrow().opts.split == on {
            return;
        }
        self.st.borrow_mut().opts.split = on;
        self.rebuild(true);
        let st = self.st.borrow().split_state.clone();
        if let Some(s) = st {
            s.set(on);
        }
    }

    pub fn is_split(&self) -> bool {
        self.st.borrow().opts.split
    }

    /// Two-way binding of the layout (true = side by side).
    pub fn bind_split(self, state: &State<bool>) -> Self {
        self.st.borrow_mut().split_state = Some(state.clone());
        let me = self.downgrade();
        crate::state::bind_widget(&self.widget, state, true, move |v| {
            if let Some(m) = me.upgrade() {
                m.set_split(*v);
            }
        });
        self
    }

    /// Wraps long lines (true) or scrolls them horizontally (false, the
    /// default; every row shares one horizontal offset).
    pub fn wrap(self, on: bool) -> Self {
        self.set_wrap(on);
        self
    }

    pub fn set_wrap(&self, on: bool) {
        if self.st.borrow().wrap == on {
            return;
        }
        let anchor = self.top_anchor();
        {
            let mut s = self.st.borrow_mut();
            s.wrap = on;
            s.hx = 0;
            s.heights_key = None;
            s.pending_top = anchor;
        }
        self.widget.clone().redraw();
        let st = self.st.borrow().wrap_state.clone();
        if let Some(s) = st {
            s.set(on);
        }
    }

    pub fn is_wrap(&self) -> bool {
        self.st.borrow().wrap
    }

    /// Two-way binding of [`DiffView::set_wrap`].
    pub fn bind_wrap(self, state: &State<bool>) -> Self {
        self.st.borrow_mut().wrap_state = Some(state.clone());
        let me = self.downgrade();
        crate::state::bind_widget(&self.widget, state, true, move |v| {
            if let Some(m) = me.upgrade() {
                m.set_wrap(*v);
            }
        });
        self
    }

    /// Shows (true, the default) or hides unchanged context lines.
    pub fn set_show_context(&self, on: bool) {
        if self.st.borrow().opts.hide_context == !on {
            return;
        }
        self.st.borrow_mut().opts.hide_context = !on;
        self.rebuild(true);
        let st = self.st.borrow().context_state.clone();
        if let Some(s) = st {
            s.set(on);
        }
    }

    pub fn shows_context(&self) -> bool {
        !self.st.borrow().opts.hide_context
    }

    /// Two-way binding of [`DiffView::set_show_context`].
    pub fn bind_show_context(self, state: &State<bool>) -> Self {
        self.st.borrow_mut().context_state = Some(state.clone());
        let me = self.downgrade();
        crate::state::bind_widget(&self.widget, state, true, move |v| {
            if let Some(m) = me.upgrade() {
                m.set_show_context(*v);
            }
        });
        self
    }

    /// Highlights the changed words of paired removed/added lines (on by
    /// default; computed only for the rows in view).
    pub fn word_diff(self, on: bool) -> Self {
        self.st.borrow_mut().word_diff = on;
        self.widget.clone().redraw();
        self
    }

    // ------------------------------------------------------------ folding

    pub fn is_folded(&self, file: usize, hunk: usize) -> bool {
        self.st.borrow().opts.folded.contains(&(file as u32, hunk as u32))
    }

    /// Folds (hides the lines of) or unfolds a hunk.
    pub fn set_folded(&self, file: usize, hunk: usize, folded: bool) {
        let key = (file as u32, hunk as u32);
        {
            let mut s = self.st.borrow_mut();
            if s.opts.folded.contains(&key) == folded {
                return;
            }
            if folded {
                s.opts.folded.insert(key);
            } else {
                s.opts.folded.remove(&key);
            }
        }
        self.rebuild(true);
    }

    pub fn is_collapsed(&self, file: usize) -> bool {
        self.st.borrow().opts.collapsed.contains(&(file as u32))
    }

    /// Collapses a file to its header row, or opens it.
    pub fn set_collapsed(&self, file: usize, collapsed: bool) {
        {
            let mut s = self.st.borrow_mut();
            let f = file as u32;
            if s.opts.collapsed.contains(&f) == collapsed {
                return;
            }
            if collapsed {
                s.opts.collapsed.insert(f);
            } else {
                s.opts.collapsed.remove(&f);
            }
        }
        self.rebuild(true);
    }

    /// Folds or unfolds every hunk.
    pub fn fold_all(&self, folded: bool) {
        {
            let mut s = self.st.borrow_mut();
            s.opts.folded.clear();
            if folded {
                let keys: Vec<(u32, u32)> = s.patch.files.iter().enumerate().flat_map(|(f, d)| (0..d.hunks.len()).map(move |h| (f as u32, h as u32))).collect();
                s.opts.folded.extend(keys);
            }
        }
        self.rebuild(true);
    }

    // ------------------------------------------------------------ rows

    /// Rebuilds the rows; `keep` keeps the cursor and the top row on the
    /// same content.
    fn rebuild(&self, keep: bool) {
        let cursor = if keep { self.cursor_anchor() } else { None };
        let top = if keep { self.top_anchor() } else { None };
        let (len, ids) = {
            let mut g = self.st.borrow_mut();
            let s = &mut *g;
            s.opts.more = s.expand.is_some();
            s.rows = rows::build(&s.patch, &s.opts);
            *s.ids.borrow_mut() = s.rows.iter().map(|r| r.id()).collect();
            s.rows_gen += 1;
            s.heights_key = None;
            s.hover = None;
            if top.is_some() {
                s.pending_top = top;
            }
            (s.rows.len(), s.ids.clone())
        };
        let (a, b) = (ids.clone(), ids);
        let panel = self.panel();
        panel.set_items_with(len, move |i| a.borrow().get(i).copied().unwrap_or(u64::MAX), move |id| b.borrow().binary_search(&id).ok(), true);
        if let Some(c) = cursor {
            let id = self.resolve(c);
            if id != panel.cursor() {
                self.st.borrow_mut().last_cursor = id;
                panel.set_cursor(id);
                if let Some(id) = id {
                    if panel.selection_len() == 0 {
                        panel.select_only(id);
                    }
                }
            }
        }
        self.widget.clone().redraw();
    }

    fn cursor_anchor(&self) -> Option<Anchor> {
        let id = self.panel().cursor()?;
        Some(self.anchor_of(id))
    }

    fn anchor_of(&self, id: u64) -> Anchor {
        let s = self.st.borrow();
        let Ok(i) = s.ids.borrow().binary_search(&id) else { return Anchor::Row(id) };
        let r = s.rows[i];
        match r.lines() {
            [Some(l), _] | [None, Some(l)] => Anchor::Line { file: r.file, hunk: r.hunk, line: l },
            _ => Anchor::Row(id),
        }
    }

    /// The row ID showing an anchor now (a hidden line resolves to its hunk).
    fn resolve(&self, a: Anchor) -> Option<u64> {
        let s = self.st.borrow();
        let i = s.locate(a)?;
        Some(s.rows[i].id())
    }

    /// The anchor of the top row and the pixels scrolled into it.
    fn top_anchor(&self) -> Option<(Anchor, i64)> {
        let (id, off) = {
            let s = self.st.borrow();
            if s.heights_key.is_none() || s.rows.is_empty() {
                return s.pending_top;
            }
            let i = s.ys.partition_point(|y| *y <= s.top).saturating_sub(1).min(s.rows.len() - 1);
            (s.rows[i].id(), s.top - s.ys[i])
        };
        Some((self.anchor_of(id), off))
    }

    /// Computes metrics and row heights for the current width (lazy).
    fn ensure_layout(&self) {
        let w = self.widget.w();
        let mut s = self.st.borrow_mut();
        s.layout(w);
    }

    // ------------------------------------------------------------ navigation

    /// Moves the cursor to the next (or previous) hunk header and scrolls
    /// it to the top. False when there is none.
    pub fn next_hunk(&self, forward: bool) -> bool {
        self.jump_to(forward, |r| r.kind == RowKind::Hunk)
    }

    /// Moves to the next (or previous) file header.
    pub fn next_file(&self, forward: bool) -> bool {
        self.jump_to(forward, |r| r.kind == RowKind::File)
    }

    fn jump_to(&self, forward: bool, pick: impl Fn(&Row) -> bool) -> bool {
        self.ensure_layout();
        let panel = self.panel();
        let target = {
            let s = self.st.borrow();
            let from = panel.cursor_index().map(|i| i as i64).unwrap_or_else(|| {
                let i = s.ys.partition_point(|y| *y <= s.top) as i64 - 1;
                if forward { i - 1 } else { i + 1 }
            });
            if forward {
                (from + 1..s.rows.len() as i64).find(|i| pick(&s.rows[*i as usize]))
            } else {
                (0..from.min(s.rows.len() as i64)).rev().find(|i| pick(&s.rows[*i as usize]))
            }
        };
        let Some(i) = target else { return false };
        self.reveal_row(i as usize, true);
        true
    }

    /// Selects row `i` and scrolls it into view (to the top when `top`).
    fn reveal_row(&self, i: usize, top: bool) {
        let id = {
            let mut s = self.st.borrow_mut();
            let Some(id) = s.rows.get(i).map(|r| r.id()) else { return };
            if top {
                s.top = s.ys[i];
            }
            s.last_cursor = None; // follow it in draw
            let t = s.top;
            s.motion.jump(t);
            id
        };
        self.panel().select_only(id);
        self.widget.clone().redraw();
    }

    /// Scrolls file `file` to the top and puts the cursor on its header.
    pub fn show_file(&self, file: usize) -> bool {
        self.ensure_layout();
        let id = Row { kind: RowKind::File, file: file as u32, hunk: 0, a: NONE, b: NONE }.id();
        let i = self.st.borrow().ids.borrow().binary_search(&id).ok();
        match i {
            Some(i) => {
                self.reveal_row(i, true);
                true
            }
            None => false,
        }
    }

    /// File of the cursor row.
    pub fn current_file(&self) -> Option<usize> {
        self.panel().cursor().map(|id| rows::file_of(id) as usize)
    }

    /// Called with the file index when the cursor moves to another file
    /// (keys, clicks, [`DiffView::show_file`]). Listeners add up.
    pub fn on_file_change(self, f: impl FnMut(usize) + 'static) -> Self {
        self.st.borrow_mut().on_file.push(Box::new(f));
        self
    }

    fn cursor_moved(&self) {
        let file = self.current_file();
        let f = {
            let Ok(mut s) = self.st.try_borrow_mut() else { return };
            if file == s.current_file || file.is_none() {
                return;
            }
            s.current_file = file;
            std::mem::take(&mut s.on_file)
        };
        let mut f = f;
        if let Some(file) = file {
            for cb in f.iter_mut() {
                cb(file);
            }
        }
        let mut s = self.st.borrow_mut();
        f.append(&mut s.on_file);
        s.on_file = f;
    }

    // ------------------------------------------------------------ expand

    /// Lets the user ask for more context: a "Show more lines" row after
    /// the last hunk of each file and an expand button on hunk headers
    /// with hidden lines above them (`e` / Shift+E on the keyboard).
    pub fn on_expand(self, f: impl FnMut(ExpandRequest) + 'static) -> Self {
        self.st.borrow_mut().expand = Some(Box::new(f));
        self.rebuild(true);
        self
    }

    /// Adds the lines the app read for `req` (new-file text of lines
    /// `req.first..`). False when the patch changed since the request.
    pub fn provide_context(&self, req: &ExpandRequest, lines: Vec<String>) -> bool {
        let cursor = self.cursor_anchor().map(|a| self.line_numbers(a));
        let ok = {
            let mut s = self.st.borrow_mut();
            if req.below && (lines.len() as u32) < (req.last + 1).saturating_sub(req.first) {
                s.opts.at_end.insert(req.file as u32);
            }
            let ok = !lines.is_empty() && s.patch.insert_context(req.file, req.hunk, req.first, &lines);
            if ok {
                // Hunks may have merged: their folds no longer line up.
                let f = req.file as u32;
                s.opts.folded.retain(|(ff, _)| *ff != f);
                let extra = lines.iter().map(|l| rows::cols(l)).max().unwrap_or(0);
                s.max_cols = s.max_cols.max(extra);
                s.max_line_no = max_line_no(&s.patch);
                s.find.clear();
            }
            ok
        };
        self.rebuild(true);
        // Line indexes moved: put the cursor back on the same numbers.
        if let Some(Some((file, old, new))) = cursor.filter(|_| ok) {
            if let Some(id) = self.row_of_numbers(file, old, new) {
                self.panel().select_only(id);
            }
        }
        if ok {
            let q = self.st.borrow().find_query.clone();
            if !q.is_empty() {
                self.find(&q);
            }
        }
        ok
    }

    fn line_numbers(&self, a: Anchor) -> Option<(u32, u32, u32)> {
        let Anchor::Line { file, hunk, line } = a else { return None };
        let s = self.st.borrow();
        let l = s.patch.files.get(file as usize)?.hunks.get(hunk as usize)?.lines.get(line as usize)?;
        Some((file, l.old, l.new))
    }

    fn row_of_numbers(&self, file: u32, old: u32, new: u32) -> Option<u64> {
        let (hunk, line) = {
            let s = self.st.borrow();
            let f = s.patch.files.get(file as usize)?;
            f.hunks.iter().enumerate().find_map(|(h, hk)| hk.lines.iter().position(|l| l.old == old && l.new == new).map(|i| (h as u32, i as u32)))?
        };
        self.resolve(Anchor::Line { file, hunk, line })
    }

    fn request_expand(&self, file: u32, hunk: u32, below: bool) {
        let req = {
            let s = self.st.borrow();
            let Some(f) = s.patch.files.get(file as usize) else { return };
            let Some(h) = f.hunks.get(hunk as usize) else { return };
            let (first, last) = if below {
                let first = h.new_end() + 1;
                (first, first + EXPAND_STEP - 1)
            } else {
                let Some((a, b)) = s.patch.gap_above(file as usize, hunk as usize) else { return };
                (a.max(b.saturating_sub(EXPAND_STEP - 1)), b)
            };
            ExpandRequest { file: file as usize, path: f.path().to_string(), hunk: hunk as usize, first, last, below }
        };
        let f = self.st.borrow_mut().expand.take();
        if let Some(mut f) = f {
            f(req);
            let mut s = self.st.borrow_mut();
            if s.expand.is_none() {
                s.expand = Some(f);
            }
        }
    }

    fn can_expand_above(&self, file: u32, hunk: u32) -> bool {
        let s = self.st.borrow();
        s.expand.is_some() && s.patch.gap_above(file as usize, hunk as usize).is_some()
    }

    // ------------------------------------------------------------ find

    /// Finds `query` in every line (folded ones too); case-insensitive
    /// unless the query has capitals. Returns the number of matches and
    /// highlights them; an empty query clears the find.
    pub fn find(&self, query: &str) -> usize {
        let mut s = self.st.borrow_mut();
        s.find.clear();
        s.find_current = None;
        s.find_query = query.to_string();
        if !query.is_empty() {
            let fold = !query.chars().any(|c| c.is_uppercase());
            let q = if fold { query.to_lowercase() } else { query.to_string() };
            let mut found = Vec::new();
            for (fi, f) in s.patch.files.iter().enumerate() {
                for (hi, h) in f.hunks.iter().enumerate() {
                    for (li, l) in h.lines.iter().enumerate() {
                        let text = s.patch.text(l);
                        // ASCII folding keeps byte offsets.
                        let hay: std::borrow::Cow<str> = if fold { text.to_ascii_lowercase().into() } else { text.into() };
                        for (b, _) in hay.match_indices(q.as_str()) {
                            found.push(Match { file: fi as u32, hunk: hi as u32, line: li as u32, start: b as u32, end: (b + q.len()) as u32 });
                            if found.len() >= 100_000 {
                                break;
                            }
                        }
                    }
                }
            }
            s.find = found;
        }
        let n = s.find.len();
        drop(s);
        self.widget.clone().redraw();
        n
    }

    /// Matches of the last [`DiffView::find`].
    pub fn find_count(&self) -> usize {
        self.st.borrow().find.len()
    }

    /// Moves to the next (or previous) match after the cursor, unfolding
    /// its hunk or file. Returns (1-based match number, total).
    pub fn find_next(&self, forward: bool) -> Option<(usize, usize)> {
        let cursor = self.cursor_anchor();
        let (idx, m, total) = {
            let s = self.st.borrow();
            let n = s.find.len();
            if n == 0 {
                return None;
            }
            let key = |m: &Match| (m.file, m.hunk, m.line, m.start);
            let idx = match (s.find_current, cursor) {
                (Some(c), _) => if forward { (c + 1) % n } else { (c + n - 1) % n },
                (None, Some(Anchor::Line { file, hunk, line })) => {
                    let p = s.find.partition_point(|m| key(m) < (file, hunk, line, 0));
                    if forward { p % n } else { (p + n - 1) % n }
                }
                (None, Some(Anchor::Row(id))) => {
                    let f = rows::file_of(id);
                    let p = s.find.partition_point(|m| m.file < f);
                    if forward { p % n } else { (p + n - 1) % n }
                }
                (None, None) => if forward { 0 } else { n - 1 },
            };
            (idx, s.find[idx], n)
        };
        {
            let mut s = self.st.borrow_mut();
            s.find_current = Some(idx);
        }
        if self.is_collapsed(m.file as usize) {
            self.set_collapsed(m.file as usize, false);
        }
        if self.is_folded(m.file as usize, m.hunk as usize) {
            self.set_folded(m.file as usize, m.hunk as usize, false);
        }
        if self.st.borrow().opts.hide_context {
            let ctx = self.with_patch(|p| p.files[m.file as usize].hunks[m.hunk as usize].lines[m.line as usize].kind == LineKind::Context);
            if ctx {
                self.set_show_context(true);
            }
        }
        if let Some(id) = self.resolve(Anchor::Line { file: m.file, hunk: m.hunk, line: m.line }) {
            self.ensure_layout();
            let i = self.st.borrow().ids.borrow().binary_search(&id).ok();
            if let Some(i) = i {
                self.reveal_row(i, false);
                // Bring the match column into view when lines do not wrap.
                let mut s = self.st.borrow_mut();
                if !s.wrap {
                    let col = {
                        let l = &s.patch.files[m.file as usize].hunks[m.hunk as usize].lines[m.line as usize];
                        Cells::new(s.patch.text(l)).col_of(m.start as usize)
                    };
                    let geo = s.geo(self.widget.w());
                    let x = (col as f64 * s.metrics.cw) as i32;
                    if x < s.hx || x > s.hx + geo.code_w - 4 * s.metrics.cw as i32 {
                        s.hx = (x - geo.code_w / 3).max(0);
                    }
                }
            }
        }
        self.widget.clone().redraw();
        Some((idx + 1, total))
    }

    /// Clears the find highlights.
    pub fn clear_find(&self) {
        self.find("");
    }

    // ------------------------------------------------------------ copy

    /// Text of the selected rows (or the cursor row): line text only, or
    /// `as_patch` with `+`/`-`/` ` markers and hunk headers (a selected
    /// folded hunk brings all its lines).
    pub fn selected_text(&self, as_patch: bool) -> String {
        let panel = self.panel();
        let mut ids = panel.selection();
        if ids.is_empty() {
            ids.extend(panel.cursor());
        }
        let s = self.st.borrow();
        let p = &s.patch;
        let mut out = String::new();
        let mut headers = String::new();
        for id in ids {
            let Ok(i) = s.ids.borrow().binary_search(&id) else { continue };
            let r = s.rows[i];
            let Some(f) = p.files.get(r.file as usize) else { continue };
            match r.kind {
                RowKind::File => {
                    if as_patch {
                        let q = |o: &Option<String>, pre: &str| o.as_ref().map_or("/dev/null".to_string(), |x| format!("{pre}{x}"));
                        out.push_str(&format!("--- {}\n+++ {}\n", q(&f.old_path, "a/"), q(&f.new_path, "b/")));
                    } else {
                        headers.push_str(&f.display_path());
                        headers.push('\n');
                    }
                }
                RowKind::Hunk => {
                    let h = &f.hunks[r.hunk as usize];
                    let head = if h.section.is_empty() { h.range() } else { format!("{} {}", h.range(), h.section) };
                    if as_patch {
                        out.push_str(&head);
                        out.push('\n');
                        if s.opts.folded.contains(&(r.file, r.hunk)) {
                            for l in &h.lines {
                                p.line_as_patch(l, &mut out);
                            }
                        }
                    } else {
                        headers.push_str(&head);
                        headers.push('\n');
                    }
                }
                RowKind::Line | RowKind::Pair => {
                    let h = &f.hunks[r.hunk as usize];
                    for l in r.lines().into_iter().flatten() {
                        let l = &h.lines[l as usize];
                        if as_patch {
                            p.line_as_patch(l, &mut out);
                        } else {
                            out.push_str(p.text(l));
                            out.push('\n');
                        }
                    }
                }
                RowKind::Notice | RowKind::More => {}
            }
        }
        if out.is_empty() {
            out = headers;
        }
        out
    }

    /// Copies [`DiffView::selected_text`] to the clipboard (Mod+C, or
    /// Mod+Shift+C as a patch). False when nothing is selected.
    pub fn copy(&self, as_patch: bool) -> bool {
        let t = self.selected_text(as_patch);
        if t.is_empty() {
            return false;
        }
        app::copy(&t);
        true
    }

    // ------------------------------------------------------------ scroll, tests

    pub fn scroll_offset(&self) -> i64 {
        self.st.borrow().top
    }

    /// Scrolls to a vertical offset in pixels (clamped when drawn).
    pub fn set_scroll_offset(&self, px: i64) {
        let mut s = self.st.borrow_mut();
        s.top = px.max(0);
        let t = s.top;
        s.motion.jump(t);
        s.pending_top = None;
        drop(s);
        self.widget.clone().redraw();
    }

    /// Shared horizontal offset of the code columns (no-wrap mode).
    pub fn h_offset(&self) -> i32 {
        self.st.borrow().hx
    }

    pub fn set_h_offset(&self, px: i32) {
        self.st.borrow_mut().hx = px.max(0);
        self.widget.clone().redraw();
    }

    /// Total content height in pixels at the current width.
    pub fn content_height(&self) -> i64 {
        self.ensure_layout();
        *self.st.borrow().ys.last().unwrap_or(&0)
    }

    /// Display rows (all, not only those in view).
    pub fn row_count(&self) -> usize {
        self.st.borrow().rows.len()
    }

    /// Rows painted by the last frame (proves virtualization in tests).
    pub fn rows_drawn(&self) -> usize {
        self.st.borrow().drawn
    }

    /// The display row showing line `line` of hunk `hunk` of file `file`.
    pub fn row_of_line(&self, file: usize, hunk: usize, line: usize) -> Option<usize> {
        let id = self.resolve(Anchor::Line { file: file as u32, hunk: hunk as u32, line: line as u32 })?;
        self.st.borrow().ids.borrow().binary_search(&id).ok()
    }

    /// The display row of the header of hunk `hunk` of file `file`.
    pub fn row_of_hunk(&self, file: usize, hunk: usize) -> Option<usize> {
        let id = Row { kind: RowKind::Hunk, file: file as u32, hunk: hunk as u32, a: NONE, b: NONE }.id();
        self.st.borrow().ids.borrow().binary_search(&id).ok()
    }

    /// Window rectangle of display row `row` at the current scroll offset
    /// (it may be outside the widget).
    pub fn row_rect(&self, row: usize) -> Option<(i32, i32, i32, i32)> {
        self.ensure_layout();
        let s = self.st.borrow();
        let (y0, y1) = (*s.ys.get(row)?, *s.ys.get(row + 1)?);
        Some((self.widget.x(), self.widget.y() + (y0 - s.top) as i32, self.widget.w() - BAR, (y1 - y0) as i32))
    }

    /// Code columns: (x of the first code column, x of the right code
    /// column in split mode or 0, code width), in window coordinates.
    pub fn code_columns(&self) -> (i32, i32, i32) {
        self.ensure_layout();
        let s = self.st.borrow();
        let g = s.geo(self.widget.w());
        let x = self.widget.x();
        (x + g.code_x, if s.opts.split { x + g.half + g.code_x } else { 0 }, g.code_w)
    }

    // ------------------------------------------------------------ keys and mouse

    fn activate(&self, id: u64) {
        let row = {
            let s = self.st.borrow();
            let i = s.ids.borrow().binary_search(&id).ok();
            i.map(|i| s.rows[i])
        };
        if let Some(r) = row {
            self.toggle_row(r);
        }
    }

    fn toggle_row(&self, r: Row) -> bool {
        match r.kind {
            RowKind::Hunk => self.set_folded(r.file as usize, r.hunk as usize, !self.is_folded(r.file as usize, r.hunk as usize)),
            RowKind::File => self.set_collapsed(r.file as usize, !self.is_collapsed(r.file as usize)),
            RowKind::More => self.request_expand(r.file, r.hunk, true),
            _ => return false,
        }
        true
    }

    fn cursor_row(&self) -> Option<Row> {
        let id = self.panel().cursor()?;
        let s = self.st.borrow();
        let i = s.ids.borrow().binary_search(&id).ok()?;
        Some(s.rows[i])
    }

    fn key(&self, k: Key, mods: Shortcut) -> bool {
        let primary = crate::keys::PRIMARY;
        let other = (Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta).difference(primary);
        let ctrl = mods.contains(primary) && !mods.intersects(other);
        let alt = mods.contains(Shortcut::Alt) && !mods.intersects(Shortcut::Ctrl | Shortcut::Meta);
        let shift = mods.contains(Shortcut::Shift);
        let plain = !mods.intersects(Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta);
        let ch = |c: char| k == Key::from_char(c);
        if ctrl && ch('c') {
            return self.copy(shift);
        }
        if ctrl && !shift && (k == Key::Left || k == Key::Right) {
            self.next_file(k == Key::Right);
            return true;
        }
        if (alt && !shift && (k == Key::Down || k == Key::Up)) || (plain && !shift && (ch('n') || ch('p'))) {
            self.next_hunk(k == Key::Down || ch('n'));
            return true;
        }
        if (plain && k == Key::F3) || (ctrl && ch('g')) {
            self.find_next(!shift);
            return true;
        }
        if plain && k == Key::Escape && !self.st.borrow().find.is_empty() {
            self.clear_find();
            return true;
        }
        if !plain {
            return false;
        }
        let row = self.cursor_row();
        let header = row.filter(|r| r.is_header() || r.kind == RowKind::More);
        match k {
            Key::Enter | Key::KPEnter if !shift => header.is_some_and(|r| self.toggle_row(r)),
            _ if ch(' ') && !shift => header.is_some_and(|r| self.toggle_row(r)),
            Key::Left | Key::Right if !shift => {
                let open = k == Key::Right;
                match header {
                    Some(r) if r.kind == RowKind::Hunk && self.is_folded(r.file as usize, r.hunk as usize) == open => {
                        self.set_folded(r.file as usize, r.hunk as usize, !open);
                        true
                    }
                    Some(r) if r.kind == RowKind::File && self.is_collapsed(r.file as usize) == open => {
                        self.set_collapsed(r.file as usize, !open);
                        true
                    }
                    _ if !self.is_wrap() => {
                        let step = (4.0 * self.st.borrow().metrics.cw.max(1.0)) as i32;
                        let hx = self.h_offset();
                        self.set_h_offset(if open { hx + step } else { (hx - step).max(0) });
                        true
                    }
                    _ => false,
                }
            }
            _ if ch('e') => {
                let Some(r) = row else { return false };
                if shift {
                    let last = self.with_patch(|p| p.files.get(r.file as usize).map_or(0, |f| f.hunks.len()));
                    if last > 0 && self.st.borrow().expand.is_some() {
                        self.request_expand(r.file, last as u32 - 1, true);
                        return true;
                    }
                    false
                } else if self.can_expand_above(r.file, r.hunk) && r.kind != RowKind::File {
                    self.request_expand(r.file, r.hunk, false);
                    true
                } else {
                    false
                }
            }
            _ if ch('s') && !shift => {
                self.set_split(!self.is_split());
                true
            }
            _ if ch('w') && !shift => {
                self.set_wrap(!self.is_wrap());
                true
            }
            _ if ch('c') && !shift => {
                self.set_show_context(!self.shows_context());
                true
            }
            _ => false,
        }
    }

    fn handle(&self, w: &mut Widget, ev: Event) -> bool {
        let (ex, ey) = (app::event_x() - w.x(), app::event_y() - w.y());
        if matches!(ev, Event::Push | Event::KeyDown) {
            let mut s = self.st.borrow_mut();
            let t = s.top;
            s.motion.jump(t);
        }
        match ev {
            Event::Focus | Event::Unfocus => {
                w.redraw();
                true
            }
            Event::Enter => true,
            Event::Move => {
                let mut s = self.st.borrow_mut();
                let h = s.row_at(ey).filter(|i| s.rows[*i].is_header() || s.rows[*i].kind == RowKind::More);
                if s.hover != h {
                    s.hover = h;
                    drop(s);
                    w.redraw();
                }
                true
            }
            Event::Leave => {
                self.st.borrow_mut().hover = None;
                w.redraw();
                true
            }
            Event::MouseWheel => {
                let (mut dx, mut dy) = (app::event_dx_value(), app::event_dy_value());
                if dx == 0 && app::is_event_shift() {
                    (dx, dy) = (dy, 0);
                }
                let mut s = self.st.borrow_mut();
                let mut used = false;
                if dx != 0 && !s.wrap {
                    let geo = s.geo(w.w());
                    let max = s.max_hx(&geo);
                    let hx = (s.hx + dx * 3 * s.metrics.cw.max(1.0) as i32).clamp(0, max);
                    used = hx != s.hx || used;
                    s.hx = hx;
                }
                if dy != 0 {
                    let max = (*s.ys.last().unwrap_or(&0) - w.h() as i64).max(0);
                    let top = s.top;
                    let delta = dy as f64 * 3.0 * s.metrics.rh.max(1) as f64;
                    if !s.motion.passes(top, delta, max) {
                        s.top = s.motion.wheel(top, delta, max);
                        s.pending_top = None;
                        used = true;
                    }
                }
                drop(s);
                if used {
                    w.redraw();
                }
                used
            }
            Event::Push => {
                let _ = w.take_focus();
                let row = {
                    let s = self.st.borrow();
                    s.row_at(ey).map(|i| (s.rows[i], s.geo(w.w())))
                };
                let Some((r, geo)) = row else { return true };
                let panel = self.panel();
                let id = r.id();
                match r.kind {
                    RowKind::Hunk if ex < geo.gutter && self.can_expand_above(r.file, r.hunk) => {
                        panel.click(id, false, false);
                        self.request_expand(r.file, r.hunk, false);
                    }
                    RowKind::Hunk | RowKind::File | RowKind::More => {
                        panel.click(id, false, false);
                        self.toggle_row(r);
                    }
                    _ => {
                        panel.click(id, crate::keys::primary_held(), app::is_event_shift());
                        self.st.borrow_mut().drag = Some(panel.anchor().unwrap_or(id));
                    }
                }
                true
            }
            Event::Drag => {
                let anchor = self.st.borrow().drag;
                let Some(anchor) = anchor else { return true };
                let target = {
                    let mut s = self.st.borrow_mut();
                    // Past the edges: scroll a row and extend there.
                    let rh = s.metrics.rh.max(1) as i64;
                    if ey < 0 || ey > w.h() {
                        let max = (*s.ys.last().unwrap_or(&0) - w.h() as i64).max(0);
                        s.top = (s.top + if ey < 0 { -rh } else { rh }).clamp(0, max);
                        let t = s.top;
                        s.motion.jump(t);
                    }
                    s.row_at(ey.clamp(0, w.h() - 1)).map(|i| s.rows[i].id())
                };
                if let Some(id) = target {
                    let panel = self.panel();
                    panel.select_range(anchor, id, false);
                    panel.set_cursor(Some(id));
                    self.st.borrow_mut().last_cursor = Some(id);
                }
                w.redraw();
                true
            }
            Event::Released => {
                self.st.borrow_mut().drag = None;
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------ a11y

    fn describe(&self) {
        let (st, me) = (self.st.clone(), self.widget.clone());
        a11y::describe(&self.widget, move |n| {
            let Ok(s) = st.try_borrow() else { return };
            n.role(Role::List).name(&tr!("Diff")).focusable().multiselectable(true);
            let len = s.rows.len();
            if len == 0 || s.heights_key.is_none() {
                return;
            }
            let first = s.ys.partition_point(|y| *y <= s.top).saturating_sub(1);
            let last = s.ys.partition_point(|y| *y < s.top + me.h() as i64).min(len);
            let mut idx: Vec<usize> = (first..last.max(first)).collect();
            if let Some(c) = s.panel.cursor_index().filter(|c| !idx.contains(c)) {
                idx.push(c);
                idx.sort_unstable();
            }
            let cursor = s.panel.cursor();
            for i in idx {
                let r = s.rows[i];
                let id = r.id();
                let y = me.y() + (s.ys[i] - s.top) as i32;
                let h = (s.ys[i + 1] - s.ys[i]) as i32;
                n.child(id, |c| {
                    c.role(Role::ListItem).name(&s.row_label(&r)).position(i + 1, len).selected(s.panel.is_selected(id)).bounds(me.x(), y, me.w() - BAR, h).action(Action::Click).action(Action::Focus).action(Action::ScrollIntoView);
                    match r.kind {
                        RowKind::Hunk => {
                            let open = !s.opts.folded.contains(&(r.file, r.hunk));
                            c.expanded(open).action(if open { Action::Collapse } else { Action::Expand });
                        }
                        RowKind::File => {
                            let open = !s.opts.collapsed.contains(&r.file);
                            c.expanded(open).action(if open { Action::Collapse } else { Action::Expand });
                        }
                        _ => {}
                    }
                    if cursor == Some(id) {
                        c.focused();
                    }
                });
            }
        });
        let me = self.downgrade();
        a11y::on_action(&self.widget, move |r| {
            let Some(v) = me.upgrade() else { return false };
            let Some(&id) = r.path.first() else { return false };
            let row = {
                let s = v.st.borrow();
                let i = s.ids.borrow().binary_search(&id).ok();
                i.map(|i| (i, s.rows[i]))
            };
            let Some((i, row)) = row else { return false };
            let panel = v.panel();
            match r.action {
                Action::Click => {
                    panel.click(id, false, false);
                    v.toggle_row(row);
                }
                Action::Focus => {
                    panel.focus();
                    panel.set_cursor(Some(id));
                }
                Action::ScrollIntoView => v.reveal_row(i, false),
                Action::Expand | Action::Collapse => {
                    let open = r.action == Action::Expand;
                    match row.kind {
                        RowKind::Hunk => v.set_folded(row.file as usize, row.hunk as usize, !open),
                        RowKind::File => v.set_collapsed(row.file as usize, !open),
                        _ => return false,
                    }
                }
                _ => return false,
            }
            v.widget.clone().redraw();
            true
        });
    }
}

/// A view reference that does not keep it alive (callbacks of other widgets).
#[derive(Clone)]
pub(crate) struct WeakView {
    widget: Widget,
    st: Weak<RefCell<St>>,
}

impl WeakView {
    pub(crate) fn upgrade(&self) -> Option<DiffView> {
        Some(DiffView { widget: self.widget.clone(), st: self.st.upgrade()? }).filter(|v| !v.widget.was_deleted())
    }
}

fn max_line_no(p: &Patch) -> u32 {
    p.files.iter().flat_map(|f| f.hunks.iter()).map(|h| (h.old_start + h.old_count).max(h.new_start + h.new_count)).max().unwrap_or(0)
}

fn digits(n: u32) -> i32 {
    (n.max(1) as f64).log10().floor() as i32 + 1
}

// =================================================================== layout

impl St {
    fn metrics_now(&mut self) {
        let (size, family) = theme::with(|t| (t.font.body_small.max(8), t.font_family.clone()));
        let key = (size, family);
        if self.metrics_key.as_ref() == Some(&key) {
            return;
        }
        let (pad, sp) = theme::with(|t| (t.spacing.md, t.spacing.sm));
        draw::set_font(Font::Courier, size);
        let cw = (draw::width(&"0".repeat(100)) / 100.0).max(1.0);
        let (fh, fd) = (draw::height(), draw::descent());
        let rh = ((size as f64) * 1.55).round() as i32;
        self.metrics = Metrics {
            font: size,
            cw,
            rh,
            head_h: rh + 2 * pad,
            hunk_h: rh + sp,
            note_h: rh + sp,
            mark_w: (cw * 2.0).ceil() as i32,
            pad,
            base: (rh - fh) / 2 + fh - fd,
        };
        self.metrics_key = Some(key);
        self.heights_key = None;
    }

    fn geo(&self, width: i32) -> Geo {
        let m = &self.metrics;
        let num_w = (digits(self.max_line_no).max(3) as f64 * m.cw).ceil() as i32 + 2 * m.pad;
        let w = (width - BAR).max(0);
        if self.opts.split {
            let half = w / 2;
            let code_x = num_w + m.mark_w;
            Geo { w, gutter: num_w, code_x, code_w: (half - code_x - m.pad).max(8), half }
        } else {
            let code_x = 2 * num_w + m.mark_w;
            Geo { w, gutter: 2 * num_w, code_x, code_w: (w - code_x - m.pad).max(8), half: 0 }
        }
    }

    fn max_hx(&self, g: &Geo) -> i32 {
        ((self.max_cols as f64 * self.metrics.cw).ceil() as i32 - g.code_w + self.metrics.cw as i32 * 2).max(0)
    }

    /// Metrics and row heights for `width` (only when something changed).
    fn layout(&mut self, width: i32) {
        self.metrics_now();
        let m = self.metrics;
        let key = (width, m.rh, m.cw.to_bits(), self.wrap, self.opts.split, self.rows_gen);
        if self.heights_key == Some(key) {
            return;
        }
        self.heights_key = Some(key);
        let g = self.geo(width);
        let cols = ((g.code_w as f64) / m.cw).floor().max(8.0) as usize;
        let mut ys = Vec::with_capacity(self.rows.len() + 1);
        let mut y = 0i64;
        ys.push(0);
        let p = &self.patch;
        for r in &self.rows {
            let h = match r.kind {
                RowKind::File => m.head_h,
                RowKind::Notice => m.note_h,
                RowKind::Hunk | RowKind::More => m.hunk_h,
                RowKind::Line | RowKind::Pair => {
                    if self.wrap {
                        let hk = &p.files[r.file as usize].hunks[r.hunk as usize];
                        let n = r.lines().into_iter().flatten().map(|l| rows::visual_lines(p.text(&hk.lines[l as usize]), cols)).max().unwrap_or(1);
                        m.rh * n as i32
                    } else {
                        m.rh
                    }
                }
            };
            y += h as i64;
            ys.push(y);
        }
        self.ys = ys;
        // Keep the same content at the top after a rebuild or resize.
        if let Some((a, off)) = self.pending_top.take() {
            if let Some(i) = self.locate(a) {
                self.top = self.ys[i] + off.min(self.ys[i + 1] - self.ys[i] - 1).max(0);
                let t = self.top;
                self.motion.jump(t);
            }
        }
    }

    /// The display row of an anchor: the exact row, else (a hidden line)
    /// its hunk header, else its file header, else the nearest row before.
    fn locate(&self, a: Anchor) -> Option<usize> {
        let ids = self.ids.borrow();
        if ids.is_empty() {
            return None;
        }
        let before = |id: u64| ids.partition_point(|x| *x <= id).saturating_sub(1);
        match a {
            Anchor::Row(id) => Some(before(id)),
            Anchor::Line { file, hunk, line } => {
                let head = Row { kind: RowKind::Hunk, file, hunk, a: NONE, b: NONE }.id();
                if let Ok(h) = ids.binary_search(&head) {
                    let mut i = h + 1;
                    while i < self.rows.len() {
                        let r = self.rows[i];
                        if r.file != file || r.hunk != hunk || !matches!(r.kind, RowKind::Line | RowKind::Pair) {
                            break;
                        }
                        if r.a == line || r.b == line {
                            return Some(i);
                        }
                        i += 1;
                    }
                    return Some(h);
                }
                Some(before(Row { kind: RowKind::File, file, hunk: 0, a: NONE, b: NONE }.id()))
            }
        }
    }

    fn row_at(&self, y: i32) -> Option<usize> {
        if self.heights_key.is_none() || self.rows.is_empty() {
            return None;
        }
        let at = self.top + y as i64;
        let i = self.ys.partition_point(|v| *v <= at).checked_sub(1)?;
        (i < self.rows.len()).then_some(i)
    }

    /// What a screen reader reads for a row.
    fn row_label(&self, r: &Row) -> String {
        let p = &self.patch;
        let f = &p.files[r.file as usize];
        match r.kind {
            RowKind::File => tr!("{path}: {added} added, {removed} removed", path = f.display_path(), added = f.additions, removed = f.deletions),
            RowKind::Notice => notice_text(f),
            RowKind::More => tr!("Show more lines"),
            RowKind::Hunk => {
                let h = &f.hunks[r.hunk as usize];
                if h.section.is_empty() {
                    tr!("Hunk at line {line}", line = h.new_start)
                } else {
                    tr!("Hunk at line {line}: {section}", line = h.new_start, section = h.section)
                }
            }
            RowKind::Line | RowKind::Pair => {
                let h = &f.hunks[r.hunk as usize];
                let one = |i: u32| {
                    let l = &h.lines[i as usize];
                    let text = p.text(l);
                    match l.kind {
                        LineKind::Added => tr!("Added line {n}: {text}", n = l.new, text = text),
                        LineKind::Removed => tr!("Removed line {n}: {text}", n = l.old, text = text),
                        LineKind::Context => tr!("Line {n}: {text}", n = l.new, text = text),
                    }
                };
                r.lines().into_iter().flatten().map(one).collect::<Vec<_>>().join("\n")
            }
        }
    }
}

fn notice_text(f: &super::FileDiff) -> String {
    let mut parts = Vec::new();
    if f.binary {
        parts.push(tr!("Binary file — metadata only"));
    }
    if let (true, Some(a), Some(b)) = (f.mode_changed(), &f.old_mode, &f.new_mode) {
        parts.push(tr!("Mode changed from {old} to {new}", old = a, new = b));
    }
    if parts.is_empty() {
        parts.push(tr!("No content changes"));
    }
    parts.join(" · ")
}

// =================================================================== drawing

/// Colors of one frame (from the theme, never hard-coded).
struct Ink {
    bg: Rgb,
    fg: Rgb,
    dim: Rgb,
    add: Rgb,
    add_gutter: Rgb,
    add_word: Rgb,
    del: Rgb,
    del_gutter: Rgb,
    del_word: Rgb,
    green: Rgb,
    red: Rgb,
    accent: Rgb,
    hunk: Rgb,
    head: Rgb,
    empty: Rgb,
    rule: Rgb,
    find: Rgb,
    find_current: Rgb,
}

impl Ink {
    fn new(t: &Theme) -> Ink {
        let p = &t.palette;
        let bg = p.background;
        Ink {
            bg,
            fg: p.foreground,
            dim: p.foreground.over(bg, 0.5),
            add: p.green.over(bg, 0.13),
            add_gutter: p.green.over(bg, 0.2),
            add_word: p.green.over(bg, 0.36),
            del: p.red.over(bg, 0.13),
            del_gutter: p.red.over(bg, 0.2),
            del_word: p.red.over(bg, 0.36),
            green: p.green,
            red: p.red,
            accent: p.accent,
            hunk: p.accent.over(bg, 0.08),
            head: p.foreground.over(bg, 0.06),
            empty: p.foreground.over(bg, 0.03),
            rule: p.foreground.over(bg, 0.12),
            find: p.yellow.over(bg, 0.35),
            find_current: p.yellow.over(bg, 0.7),
        }
    }

    fn tint(&self, k: LineKind) -> Option<(Rgb, Rgb, Rgb)> {
        match k {
            LineKind::Added => Some((self.add, self.add_gutter, self.add_word)),
            LineKind::Removed => Some((self.del, self.del_gutter, self.del_word)),
            LineKind::Context => None,
        }
    }
}

fn mono(size: i32, color: Rgb) {
    draw::set_font(Font::Courier, size);
    draw::set_draw_color(color.fl());
}

/// One line of code in a code column `x..x+w`: wrapped at `wrap` columns
/// or shifted left by `hx`, with column highlights `(c0, c1, color)`.
#[allow(clippy::too_many_arguments)]
fn draw_code(cells: &Cells, x: i32, y: i32, w: i32, m: &Metrics, fg: Rgb, wrap: Option<usize>, hx: i32, marks: &[(usize, usize, Rgb)]) {
    let cw = m.cw;
    let cx = |c: usize| (c as f64 * cw).round() as i32;
    draw::push_clip(x, y, w, i32::MAX / 4);
    match wrap {
        Some(cols) => {
            for (k, (a, b)) in cells.wrap(cols).into_iter().enumerate() {
                let ly = y + k as i32 * m.rh;
                for &(c0, c1, color) in marks {
                    let (s, e) = (c0.max(a), c1.min(b));
                    if e > s {
                        paint::fill(x + cx(s - a), ly + 1, cx(e - a) - cx(s - a), m.rh - 2, 2, color);
                    }
                }
                let (txt, start) = cells.slice(a, b);
                mono(m.font, fg);
                draw::draw_text(txt, x + cx(start - a), ly + m.base);
            }
        }
        None => {
            let c0 = (hx as f64 / cw).floor() as usize;
            let c1 = ((hx + w) as f64 / cw).ceil() as usize + 1;
            for &(a, b, color) in marks {
                if b > a {
                    paint::fill(x + cx(a) - hx, y + 1, cx(b) - cx(a), m.rh - 2, 2, color);
                }
            }
            let (txt, start) = cells.slice(c0, c1);
            mono(m.font, fg);
            draw::draw_text(txt, x + cx(start) - hx, y + m.base);
        }
    }
    draw::pop_clip();
}

fn draw_view(w: &mut Widget, st: &Rc<RefCell<St>>) {
    let mut g = st.borrow_mut();
    let s = &mut *g;
    s.layout(w.w());
    let m = s.metrics;
    let geo = s.geo(w.w());
    let view_h = w.h();
    s.panel.set_page_rows((view_h / m.rh.max(1)).max(1) as usize);
    // Keep the cursor row in view when it moved.
    let cur = s.panel.cursor();
    if cur != s.last_cursor {
        s.last_cursor = cur;
        if let Some(i) = s.panel.cursor_index() {
            let (y0, y1) = (s.ys[i], s.ys[i + 1]);
            if y0 < s.top {
                s.top = y0;
            } else if y1 > s.top + view_h as i64 {
                s.top = y1 - view_h as i64;
            }
            let t = s.top;
            s.motion.jump(t);
        }
    }
    let content = *s.ys.last().unwrap_or(&0);
    s.top = s.motion.position(s.top, (content - view_h as i64).max(0));
    if s.motion.running() {
        crate::anim::request_redraw(w);
    }
    let max_hx = s.max_hx(&geo);
    s.hx = s.hx.clamp(0, max_hx);
    let ring = s.panel.show_ring();
    let wrap_cols = s.wrap.then(|| ((geo.code_w as f64) / m.cw).floor().max(8.0) as usize);
    theme::with(|t| {
        let ink = Ink::new(t);
        draw::push_clip(w.x(), w.y(), w.w(), w.h());
        paint::fill(w.x(), w.y(), w.w(), w.h(), 0, ink.bg);
        let first = s.ys.partition_point(|y| *y <= s.top).saturating_sub(1);
        let mut drawn = 0;
        let mut i = first;
        while i < s.rows.len() && s.ys[i] < s.top + view_h as i64 {
            let ry = w.y() + (s.ys[i] - s.top) as i32;
            let rh = (s.ys[i + 1] - s.ys[i]) as i32;
            if draw::not_clipped(w.x(), ry, w.w(), rh) {
                draw_row(s, t, &ink, &m, &geo, w.x(), ry, rh, i, ring, wrap_cols);
                drawn += 1;
            }
            i += 1;
        }
        s.drawn = drawn;
        // Scrollbars.
        if content > view_h as i64 {
            let max = content - view_h as i64;
            let thumb = ((view_h as f64 / content as f64) * view_h as f64).max(24.0) as i32;
            let ty = w.y() + ((s.top as f64 / max as f64) * (view_h - thumb) as f64) as i32;
            paint::fill(w.x() + w.w() - BAR, ty, BAR, thumb, BAR / 2, ink.fg.over(ink.bg, 0.25));
        }
        if !s.wrap && max_hx > 0 {
            let track = geo.code_w;
            let total = track + max_hx;
            let thumb = ((track as f64 / total as f64) * track as f64).max(24.0) as i32;
            let tx = (s.hx as f64 / max_hx as f64 * (track - thumb) as f64) as i32;
            let bx = w.x() + if s.opts.split { geo.half + geo.code_x } else { geo.code_x };
            paint::fill(bx + tx, w.y() + view_h - BAR, thumb, BAR, BAR / 2, ink.fg.over(ink.bg, 0.25));
        }
        if crate::focus::show_ring(w) && s.panel.cursor().is_none() {
            paint::stroke(w.x(), w.y(), w.w(), w.h(), t.radius, 1, t.border(&t.controls.focus, ink.bg));
        }
        draw::pop_clip();
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_row(s: &St, t: &Theme, ink: &Ink, m: &Metrics, g: &Geo, x: i32, y: i32, h: i32, i: usize, ring: bool, wrap: Option<usize>) {
    let r = s.rows[i];
    let id = r.id();
    let selected = s.panel.is_selected(id);
    let cursor = s.panel.cursor() == Some(id);
    let hot = s.hover == Some(i);
    let p = &s.patch;
    let f = &p.files[r.file as usize];
    let sel = |base: Rgb| if selected { t.fill(&t.controls.selected, base) } else { base };
    let fs = m.font;
    match r.kind {
        RowKind::File => {
            let mut bg = sel(ink.head);
            if hot {
                bg = t.fill(&t.controls.hover, bg);
            }
            paint::fill(x, y, g.w, h, 0, bg);
            paint::fill(x, y + h - 1, g.w, 1, 0, ink.rule);
            let open = !s.opts.collapsed.contains(&r.file);
            let gs = t.font.body;
            let mut cx = x + m.pad;
            paint::glyph(if open { glyph::CHEVRON_DOWN } else { glyph::CHEVRON_RIGHT }, cx, y, gs + 4, h, gs, ink.dim);
            cx += gs + 4 + m.pad / 2;
            let tone = match f.status {
                FileStatus::Added => ink.green,
                FileStatus::Deleted => ink.red,
                FileStatus::Renamed | FileStatus::Copied => ink.accent,
                FileStatus::Modified => t.palette.yellow,
            };
            let bw = t.font.caption + 8;
            paint::fill(cx, y + (h - bw) / 2, bw, bw, t.radius.min(4), tone.over(bg, 0.18));
            paint::text(f.status.letter(), cx, y, bw, h, t.font.caption, true, tone, Align::Center);
            cx += bw + m.pad;
            // Counts on the right, path in the rest.
            let add = format!("+{}", f.additions);
            let del = format!("−{}", f.deletions);
            let (aw, dw) = (paint::measure(&add, t.font.caption, true), paint::measure(&del, t.font.caption, true));
            let right = x + g.w - m.pad;
            paint::text(&del, right - dw, y, dw, h, t.font.caption, true, ink.red, Align::Right);
            paint::text(&add, right - dw - m.pad - aw, y, aw, h, t.font.caption, true, ink.green, Align::Right);
            let pw = right - dw - aw - 3 * m.pad - cx;
            paint::text_elided(&f.display_path(), cx, y, pw.max(0), h, t.font.body, true, ink.fg, Align::Left);
        }
        RowKind::Notice => {
            paint::fill(x, y, g.w, h, 0, sel(ink.bg));
            paint::text_elided(&notice_text(f), x + g.code_x, y, g.w - g.code_x - m.pad, h, t.font.caption, false, ink.dim, Align::Left);
        }
        RowKind::Hunk | RowKind::More => {
            let mut bg = sel(ink.hunk);
            if hot {
                bg = t.fill(&t.controls.hover, bg);
            }
            paint::fill(x, y, g.w, h, 0, bg);
            let gs = t.font.caption;
            if r.kind == RowKind::More {
                paint::glyph(glyph::CHEVRON_DOWN, x, y, g.gutter, h, gs, ink.accent);
                paint::text(&tr!("Show more lines"), x + g.code_x, y, g.w - g.code_x, h, t.font.caption, false, ink.dim, Align::Left);
            } else {
                let hk = &f.hunks[r.hunk as usize];
                let folded = s.opts.folded.contains(&(r.file, r.hunk));
                if s.expand.is_some() && p.gap_above(r.file as usize, r.hunk as usize).is_some() {
                    paint::glyph(glyph::CHEVRON_UP, x, y, g.gutter, h, gs, ink.accent);
                }
                let cx = x + g.gutter;
                paint::glyph(if folded { glyph::CHEVRON_RIGHT } else { glyph::CHEVRON_DOWN }, cx, y, m.mark_w, h, gs, ink.dim);
                let range = hk.range();
                mono(fs, ink.accent);
                let tx = x + g.code_x;
                let baseline = y + (h - m.rh) / 2 + m.base;
                draw::push_clip(tx, y, g.w - g.code_x, h);
                draw::draw_text(&range, tx, baseline);
                let mut ex = tx + (range.chars().count() as f64 * m.cw).round() as i32 + m.cw as i32;
                if !hk.section.is_empty() {
                    mono(fs, ink.dim);
                    draw::draw_text(&hk.section, ex, baseline);
                    ex += draw::width(&hk.section) as i32 + 2 * m.cw as i32;
                }
                if folded {
                    let n = hk.lines.len();
                    paint::text(&trn!("{count} hidden lines", n), ex, y, g.w - (ex - x), h, t.font.caption, false, ink.dim, Align::Left);
                }
                draw::pop_clip();
            }
        }
        RowKind::Line => {
            let hk = &f.hunks[r.hunk as usize];
            let l = &hk.lines[r.a as usize];
            let tint = ink.tint(l.kind);
            paint::fill(x, y, g.w, h, 0, sel(tint.map_or(ink.bg, |t| t.0)));
            if let Some((_, gut, _)) = tint {
                paint::fill(x, y, g.gutter + m.mark_w, h, 0, sel(gut));
            }
            draw_numbers(m, x, y, g.gutter / 2, l.old, ink.dim);
            draw_numbers(m, x + g.gutter / 2, y, g.gutter / 2, l.new, ink.dim);
            draw_marker(m, x + g.gutter, y, l.kind, ink);
            let marks = line_marks(s, ink, r.file, r.hunk, r.a, true);
            draw_code(&Cells::new(p.text(l)), x + g.code_x, y, g.code_w, m, ink.fg, wrap, s.hx, &marks);
        }
        RowKind::Pair => {
            let hk = &f.hunks[r.hunk as usize];
            for (side, li) in [(0, r.a), (1, r.b)] {
                let hx0 = x + side * g.half;
                let hw = if side == 0 { g.half } else { g.w - g.half };
                if li == NONE {
                    paint::fill(hx0, y, hw, h, 0, sel(ink.empty));
                    continue;
                }
                let l = &hk.lines[li as usize];
                let tint = ink.tint(l.kind);
                paint::fill(hx0, y, hw, h, 0, sel(tint.map_or(ink.bg, |t| t.0)));
                if let Some((_, gut, _)) = tint {
                    paint::fill(hx0, y, g.gutter + m.mark_w, h, 0, sel(gut));
                }
                draw_numbers(m, hx0, y, g.gutter, if side == 0 { l.old } else { l.new }, ink.dim);
                draw_marker(m, hx0 + g.gutter, y, l.kind, ink);
                let marks = line_marks(s, ink, r.file, r.hunk, li, side == 0 || r.a != r.b);
                draw_code(&Cells::new(p.text(l)), hx0 + g.code_x, y, g.code_w, m, ink.fg, wrap, s.hx, &marks);
            }
            paint::fill(x + g.half, y, 1, h, 0, ink.rule);
        }
    }
    if cursor && ring {
        paint::stroke(x, y, g.w, h, 0, t.controls.focus.border_width.max(1), t.border(&t.controls.focus, ink.bg));
    }
}

fn draw_numbers(m: &Metrics, x: i32, y: i32, w: i32, n: u32, color: Rgb) {
    if n == 0 {
        return;
    }
    let s = n.to_string();
    mono(m.font, color);
    let tw = (s.len() as f64 * m.cw).round() as i32;
    draw::draw_text(&s, x + w - m.pad - tw, y + m.base);
}

fn draw_marker(m: &Metrics, x: i32, y: i32, k: LineKind, ink: &Ink) {
    let (c, color) = match k {
        LineKind::Added => ("+", ink.green),
        LineKind::Removed => ("−", ink.red),
        LineKind::Context => return,
    };
    mono(m.font, color);
    draw::draw_text(c, x + (m.mark_w - m.cw as i32) / 2, y + m.base);
}

/// Column highlights of a line: the changed words against its pair and
/// the find matches.
fn line_marks(s: &St, ink: &Ink, file: u32, hunk: u32, li: u32, with_find: bool) -> Vec<(usize, usize, Rgb)> {
    let p = &s.patch;
    let hk = &p.files[file as usize].hunks[hunk as usize];
    let l = &hk.lines[li as usize];
    let text = p.text(l);
    let mut out = Vec::new();
    let mut cells: Option<Cells> = None;
    let mut col = |b: usize| -> usize { cells.get_or_insert_with(|| Cells::new(text)).col_of(b) };
    if s.word_diff {
        if let (Some(pair), Some((_, _, word))) = (l.pair(), ink.tint(l.kind)) {
            let other = p.text(&hk.lines[pair]);
            let r = if l.kind == LineKind::Removed { rows::changed_range(text, other).map(|(o, _)| o) } else { rows::changed_range(other, text).map(|(_, n)| n) };
            if let Some((a, b)) = r.filter(|(a, b)| b > a) {
                out.push((col(a), col(b), word));
            }
        }
    }
    if with_find && !s.find.is_empty() {
        let key = (file, hunk, li);
        let start = s.find.partition_point(|m| (m.file, m.hunk, m.line) < key);
        for (k, m) in s.find[start..].iter().enumerate().take_while(|(_, m)| (m.file, m.hunk, m.line) == key) {
            let color = if s.find_current == Some(start + k) { ink.find_current } else { ink.find };
            out.push((col(m.start as usize), col(m.end as usize), color));
        }
    }
    out
}
