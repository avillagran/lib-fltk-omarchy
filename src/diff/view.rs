//! [`DiffView`]: a virtualized canvas for a [`Patch`]. One widget, no
//! widget per line; each frame draws only the rows in view.

use super::parse::{DiffError, FileStatus, LineKind, Patch, NONE};
use super::review::{self, Annotation, AnnotationPaint, LineRange, LineRef, Side};
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
/// Tallest annotation (clip regions stay within what GDI accepts).
const MAX_NOTE_H: i32 = 8_000;
/// Accessibility keys of a line row's children: the comment button and
/// the painted annotations (`NOTE_KEY + index`).
const COMMENT_KEY: u64 = 1;
const NOTE_KEY: u64 = 1 << 20;

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
type Painter = Rc<RefCell<dyn FnMut(&mut AnnotationPaint)>>;

struct St {
    patch: Patch,
    rows: Vec<Row>,
    ids: Rc<RefCell<Vec<u64>>>,
    ys: Vec<i64>,
    /// Height of each row's own content (its line part, without the
    /// annotations under it); `ys` adds the annotations.
    base: Vec<i32>,
    heights_key: Option<(i32, i32, u64, bool, bool, u64)>,
    heights_gen: u64,
    ys_key: Option<(u64, u64, u64)>,
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
    /// The cursor moved: scroll it into view at the next frame.
    follow: bool,
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
    // ---- review: annotations, line ranges, comments
    notes: Vec<Annotation>,
    /// Annotations or their heights changed (ys again).
    notes_gen: u64,
    /// Annotations changed (their rows again).
    notes_map_gen: u64,
    /// (row, annotation) of the annotations shown, in display order.
    row_notes: Vec<(u32, u32)>,
    row_notes_key: Option<(u64, u64)>,
    painter: Option<Painter>,
    hosted: Vec<(String, Widget)>,
    host: Option<fltk::group::Group>,
    range: Option<LineRange>,
    /// The line a Shift+click or a drag extends the range from.
    range_anchor: Option<u32>,
    /// The panel selection is being set from `range` (not by the user).
    syncing: u32,
    on_select: Vec<Box<dyn FnMut(Option<LineRange>)>>,
    on_comment: Option<Box<dyn FnMut(LineRange)>>,
    /// A press on the line numbers: (file, side, anchor line).
    gutter_drag: Option<(u32, Side, u32)>,
    /// Line row under the pointer and its half (split: 1 = right).
    hover_line: Option<(usize, u8)>,
}

/// A virtualized diff: unified or side by side, line numbers, add/delete
/// tints, word highlights, folding hunks and files, wrap or horizontal
/// scrolling, selection and copy, find, line ranges by side and number,
/// line comments and annotations under lines. See the [module docs](super) and
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
            base: Vec::new(),
            heights_key: None,
            heights_gen: 0,
            ys_key: None,
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
            follow: false,
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
            notes: Vec::new(),
            notes_gen: 0,
            notes_map_gen: 0,
            row_notes: Vec::new(),
            row_notes_key: None,
            painter: None,
            hosted: Vec::new(),
            host: None,
            range: None,
            range_anchor: None,
            syncing: 0,
            on_select: Vec::new(),
            on_comment: None,
            gutter_drag: None,
            hover_line: None,
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
        // The panel follows the rows; the line range is the selection that
        // survives (selected again on the new rows below).
        self.st.borrow_mut().syncing += 1;
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
        self.st.borrow_mut().syncing -= 1;
        if self.st.borrow().range.is_some() {
            self.project(None);
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
        self.derive_range();
        let cursor = self.panel().cursor();
        let file = self.current_file();
        let f = {
            let Ok(mut s) = self.st.try_borrow_mut() else { return };
            if cursor != s.last_cursor {
                s.last_cursor = cursor;
                s.follow = cursor.is_some();
            }
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

    // ------------------------------------------------------------ review: line ranges

    /// The selected lines by file, side and number: set by a click on a
    /// line number (Shift+click extends, a drag down the numbers selects a
    /// range) or derived from the selected rows (keys, clicks on the code:
    /// the new side, or the old side when only removed lines are
    /// selected). It survives folding, more context, split and wrap.
    pub fn selected_range(&self) -> Option<LineRange> {
        self.st.borrow().range
    }

    /// Selects lines (None clears) and moves the cursor to the range's
    /// last line. Does not call `on_select`.
    pub fn set_selected_range(&self, range: Option<LineRange>) {
        {
            let mut s = self.st.borrow_mut();
            s.range = range;
            s.range_anchor = range.map(|r| r.start);
        }
        match range {
            Some(r) => {
                self.ensure_layout();
                let end = self.st.borrow().row_of_ref(r.file as u32, r.last());
                self.project(end);
            }
            None => {
                self.sync(|p| p.clear_selection());
            }
        }
        self.widget.clone().redraw();
    }

    /// Called when [`DiffView::selected_range`] changes by the user
    /// (clicks, drags, keys). Listeners add up.
    pub fn on_select(self, f: impl FnMut(Option<LineRange>) + 'static) -> Self {
        self.st.borrow_mut().on_select.push(Box::new(f));
        self
    }

    /// Lets the user comment on lines: a "+" in the gutter of the line
    /// under the pointer and of the cursor line; clicking it, or Enter on a
    /// line, calls `f` with the selected range when the line is in it, or
    /// with that line alone.
    pub fn on_comment(self, f: impl FnMut(LineRange) + 'static) -> Self {
        {
            let mut s = self.st.borrow_mut();
            s.on_comment = Some(Box::new(f));
            // The gutter makes room for the button.
            s.heights_key = None;
        }
        self.widget.clone().redraw();
        self
    }

    /// The display row showing line `at` of file `file` (None while it is
    /// hidden: folded, collapsed, context hidden or not in the patch).
    pub fn row_of_line_ref(&self, file: usize, at: LineRef) -> Option<usize> {
        self.st.borrow().row_of_ref(file as u32, at)
    }

    /// Runs `f` on the panel while its changes are not the user's.
    fn sync(&self, f: impl FnOnce(&KeyboardPanel)) {
        self.st.borrow_mut().syncing += 1;
        let panel = self.panel();
        f(&panel);
        self.st.borrow_mut().syncing -= 1;
    }

    /// Selects the rows of the range (from the row of its first line on
    /// its side to the row of its last); the cursor goes to `cursor` (a
    /// row index), or stays.
    fn project(&self, cursor: Option<usize>) {
        let span = {
            let s = self.st.borrow();
            let Some(r) = s.range else { return };
            let anchor = s.range_anchor.and_then(|a| s.row_of_ref(r.file as u32, LineRef::new(r.side, a)));
            s.span_of(&r).map(|(a, b)| (s.rows[a].id(), s.rows[b].id(), anchor.map(|i| s.rows[i].id()), cursor.and_then(|i| s.rows.get(i)).map(|r| r.id())))
        };
        let Some((first, last, anchor, cursor)) = span else {
            self.sync(|p| p.clear_selection());
            return;
        };
        self.sync(|p| {
            let keep = p.cursor();
            let anchor = anchor.unwrap_or(first);
            p.select_only(anchor);
            p.select_range(first, last, false);
            p.set_cursor(cursor.or(keep).or(Some(last)));
        });
        let c = self.panel().cursor();
        self.st.borrow_mut().last_cursor = c;
    }

    /// The user changed the range: remember it and tell the listeners.
    fn set_range(&self, range: Option<LineRange>, anchor: Option<u32>, cursor: Option<usize>) {
        let changed = {
            let mut s = self.st.borrow_mut();
            let changed = s.range != range;
            s.range = range;
            s.range_anchor = anchor;
            changed
        };
        if range.is_some() {
            self.project(cursor);
        }
        if changed {
            self.notify_select(range);
        }
        self.widget.clone().redraw();
    }

    fn notify_select(&self, range: Option<LineRange>) {
        let mut ls = std::mem::take(&mut self.st.borrow_mut().on_select);
        for f in ls.iter_mut() {
            f(range);
        }
        let mut s = self.st.borrow_mut();
        ls.append(&mut s.on_select);
        s.on_select = ls;
    }

    /// The panel's selection changed by the user: the range of its lines.
    fn derive_range(&self) {
        let (range, anchor) = {
            let Ok(s) = self.st.try_borrow() else { return };
            if s.syncing > 0 {
                return;
            }
            let panel = &s.panel;
            let mut ids = panel.selection();
            if ids.is_empty() {
                ids.extend(panel.cursor());
            }
            s.range_of_rows(&ids, panel.anchor())
        };
        let changed = {
            let Ok(mut s) = self.st.try_borrow_mut() else { return };
            let changed = s.range != range;
            s.range = range;
            s.range_anchor = anchor;
            changed
        };
        if changed {
            self.notify_select(range);
        }
    }

    /// A press on a line number: a new range, or with Shift one extended
    /// from the anchor line.
    fn press_number(&self, i: usize, at: LineRef, shift: bool) {
        let (file, prev, anchor) = {
            let s = self.st.borrow();
            (s.rows[i].file, s.range, s.range_anchor)
        };
        let (range, anchor) = match (prev, anchor) {
            (Some(r), Some(a)) if shift && r.file == file as usize => {
                // Extend on the range's side; a row without a number there
                // keeps the range.
                let n = self.st.borrow().number_of_row(i, r.side);
                match n {
                    Some(n) => (LineRange::between(file as usize, r.side, a, n), a),
                    None => (r, a),
                }
            }
            _ => (LineRange::line(file as usize, at), at.line),
        };
        self.st.borrow_mut().gutter_drag = Some((file, range.side, anchor));
        self.set_range(Some(range), Some(anchor), Some(i));
    }

    /// Dragging from a line number reached row `i`.
    fn drag_number(&self, i: usize) {
        let Some((file, side, anchor)) = self.st.borrow().gutter_drag else { return };
        let n = {
            let s = self.st.borrow();
            if !s.rows.get(i).is_some_and(|r| r.file == file) {
                return;
            }
            s.number_of_row(i, side)
        };
        if let Some(n) = n {
            let range = LineRange::between(file as usize, side, anchor, n);
            self.set_range(Some(range), Some(anchor), Some(i));
        }
    }

    /// Comment on line `at` of row `i`: the selected range when the row is
    /// in it, else that line (which becomes the selection).
    fn comment_at(&self, i: usize, at: LineRef) -> bool {
        if self.st.borrow().on_comment.is_none() {
            return false;
        }
        let range = {
            let s = self.st.borrow();
            let file = s.rows[i].file;
            match s.range {
                Some(r) if r.file == file as usize && s.span_of(&r).is_some_and(|(a, b)| (a..=b).contains(&i)) => Some(r),
                _ => None,
            }
        };
        let range = match range {
            Some(r) => r,
            None => {
                let file = self.st.borrow().rows[i].file as usize;
                let r = LineRange::line(file, at);
                self.set_range(Some(r), Some(at.line), Some(i));
                r
            }
        };
        let f = self.st.borrow_mut().on_comment.take();
        if let Some(mut f) = f {
            f(range);
            let mut s = self.st.borrow_mut();
            if s.on_comment.is_none() {
                s.on_comment = Some(f);
            }
        }
        true
    }

    // ------------------------------------------------------------ review: annotations

    /// Shows these annotations under their lines (several on one line in
    /// this order). The top of the view stays on the same content.
    pub fn set_annotations(&self, notes: Vec<Annotation>) {
        let anchor = self.top_anchor();
        {
            let mut s = self.st.borrow_mut();
            s.notes = notes;
            s.notes_gen += 1;
            s.notes_map_gen += 1;
            s.pending_top = anchor;
        }
        self.widget.clone().redraw();
    }

    pub fn annotations(&self) -> Vec<Annotation> {
        self.st.borrow().notes.clone()
    }

    /// Changes the height of annotation `id` (a thread got a reply, a
    /// composer grew); what is at the top of the view stays still. False
    /// when there is no such annotation.
    pub fn set_annotation_height(&self, id: &str, height: i32) -> bool {
        let anchor = self.top_anchor();
        let mut s = self.st.borrow_mut();
        let Some(a) = s.notes.iter_mut().find(|a| a.id == id) else { return false };
        if a.height == height {
            return true;
        }
        a.height = height;
        s.notes_gen += 1;
        s.pending_top = anchor;
        drop(s);
        self.widget.clone().redraw();
        true
    }

    /// Paints annotations that have no hosted widget, in the rectangle the
    /// view reserved (clipped). It only paints: it must not call the view.
    pub fn annotation_painter(self, f: impl FnMut(&mut AnnotationPaint) + 'static) -> Self {
        self.st.borrow_mut().painter = Some(Rc::new(RefCell::new(f)));
        self.widget.clone().redraw();
        self
    }

    /// Hosts `widget` in annotation `id`'s rectangle: the view moves it
    /// with scrolling and hides it while its line is out of view or
    /// hidden (a focused widget stays alive, clipped, until focus leaves
    /// it). It is moved into an overlay group of the view's window; Tab
    /// reaches it right after the diff, Shift+Tab goes back, and Escape
    /// inside it returns to the diff on its line.
    pub fn host_annotation(&self, id: &str, widget: &impl WidgetExt) {
        let mut wd = widget.as_base_widget();
        wd.hide();
        {
            let mut s = self.st.borrow_mut();
            s.hosted.retain(|(k, w)| k != id && !w.was_deleted());
            s.hosted.push((id.to_string(), wd));
        }
        self.widget.clone().redraw();
    }

    /// Window rectangle of annotation `id` at the current scroll offset
    /// (it may be outside the view); None while its line is hidden.
    pub fn annotation_rect(&self, id: &str) -> Option<(i32, i32, i32, i32)> {
        self.ensure_layout();
        let s = self.st.borrow();
        let k = s.notes.iter().position(|a| a.id == id)?;
        let (_, y, h) = s.note_y(k)?;
        let g = s.geo(self.widget.w());
        let (x, w) = note_x(&g, &s.metrics);
        Some((self.widget.x() + x, self.widget.y() + (y - s.top) as i32, w, h))
    }

    /// Scrolls annotation `id` (and its line) into view. False while its
    /// line is hidden.
    pub fn reveal_annotation(&self, id: &str) -> bool {
        self.ensure_layout();
        let mut s = self.st.borrow_mut();
        let Some(k) = s.notes.iter().position(|a| a.id == id) else { return false };
        let Some((row, y, h)) = s.note_y(k) else { return false };
        let view_h = self.widget.h() as i64;
        let y0 = s.ys[row];
        let y1 = y + h as i64;
        if y0 < s.top {
            s.top = y0;
        } else if y1 > s.top + view_h {
            s.top = (y1 - view_h).min(y0);
        }
        let t = s.top;
        s.motion.jump(t);
        s.pending_top = None;
        s.follow = false;
        drop(s);
        self.widget.clone().redraw();
        true
    }

    /// Escape in a hosted widget, or its line went away while it had
    /// focus: focus back to the diff, the cursor on its line.
    fn focus_back(&self, id: Option<String>) {
        let row = id.and_then(|id| {
            self.ensure_layout();
            let s = self.st.borrow();
            let k = s.notes.iter().position(|a| a.id == id)?;
            let a = &s.notes[k];
            s.row_of_ref(a.file as u32, a.at).map(|i| s.rows[i].id())
        });
        let panel = self.panel();
        panel.focus();
        if let Some(id) = row {
            panel.set_cursor(Some(id));
        }
        self.widget.clone().redraw();
    }

    /// The hosted annotation that has focus.
    fn focused_note(&self) -> Option<String> {
        let f = crate::focus::current()?;
        let s = self.st.borrow();
        s.hosted.iter().find(|(_, w)| !w.was_deleted() && contains(w, &f)).map(|(id, _)| id.clone())
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
        s.follow = false;
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
    /// (it may be outside the widget), without the annotations under it.
    pub fn row_rect(&self, row: usize) -> Option<(i32, i32, i32, i32)> {
        self.ensure_layout();
        let s = self.st.borrow();
        let (y0, h) = (*s.ys.get(row)?, *s.base.get(row)?);
        Some((self.widget.x(), self.widget.y() + (y0 - s.top) as i32, self.widget.w() - BAR, h))
    }

    /// Window rectangle of the line number on `side` in row `row` (where a
    /// click selects that line); None when the row has no number there.
    pub fn number_rect(&self, row: usize, side: Side) -> Option<(i32, i32, i32, i32)> {
        let (x, y, _, h) = self.row_rect(row)?;
        let s = self.st.borrow();
        s.number_of_row(row, side)?;
        let g = s.geo(self.widget.w());
        Some(match (s.opts.split, side) {
            (false, Side::Old) => (x, y, g.gutter / 2, h),
            (false, Side::New) => (x + g.gutter / 2, y, g.gutter / 2, h),
            (true, Side::Old) => (x, y, g.gutter, h),
            (true, Side::New) => (x + g.half, y, g.gutter, h),
        })
    }

    /// Window rectangle of the comment button of row `row` (split: of the
    /// half of `side`), drawn while the pointer or the cursor is on the
    /// line; None without `on_comment` or a line there.
    pub fn comment_button_rect(&self, row: usize, side: Side) -> Option<(i32, i32, i32, i32)> {
        let (x, y, _, _) = self.row_rect(row)?;
        let s = self.st.borrow();
        s.on_comment.as_ref()?;
        let half = (s.opts.split && side == Side::New) as u8;
        s.line_ref_of(row, half)?;
        let g = s.geo(self.widget.w());
        let m = &s.metrics;
        let (px, pw) = plus_x(m);
        let ph = pw.min(m.rh);
        Some((x + half as i32 * g.half + px, y + (m.rh - ph) / 2, pw, ph))
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
        // Enter on a line: comment on it (or on the selected range).
        if matches!(k, Key::Enter | Key::KPEnter) && !shift && row.is_some_and(|r| matches!(r.kind, RowKind::Line | RowKind::Pair)) {
            let at = {
                let s = self.st.borrow();
                s.panel.cursor_index().and_then(|i| s.line_ref_of(i, 1).or_else(|| s.line_ref_of(i, 0)).map(|at| (i, at)))
            };
            if let Some((i, at)) = at {
                if self.comment_at(i, at) {
                    return true;
                }
            }
        }
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
        // Rows may have changed since the last frame (a fold, new context).
        if matches!(ev, Event::Push | Event::Drag | Event::Move | Event::Enter | Event::MouseWheel) {
            match self.st.try_borrow_mut() {
                Ok(mut s) => s.layout(w.w()),
                Err(_) => return true,
            }
        }
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
            Event::Enter | Event::Move => {
                // Hosted widgets shown or moved while drawing send these.
                let Ok(mut s) = self.st.try_borrow_mut() else { return true };
                let row = s.row_at(ey);
                let h = row.filter(|i| s.rows[*i].is_header() || s.rows[*i].kind == RowKind::More);
                let line = row.and_then(|i| match s.spot(i, ex, ey, w.w()) {
                    Spot::Plus(_, half) | Spot::Number(_, half) | Spot::Row(half) if matches!(s.rows[i].kind, RowKind::Line | RowKind::Pair) => Some((i, half)),
                    _ => None,
                });
                if s.hover != h || s.hover_line != line {
                    s.hover = h;
                    s.hover_line = line;
                    drop(s);
                    w.redraw();
                }
                true
            }
            Event::Leave => {
                let Ok(mut s) = self.st.try_borrow_mut() else { return true };
                s.hover = None;
                s.hover_line = None;
                drop(s);
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
                        s.follow = false;
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
                    s.row_at(ey).map(|i| (i, s.rows[i], s.geo(w.w()), s.spot(i, ex, ey, w.w())))
                };
                let Some((i, r, geo, spot)) = row else { return true };
                let panel = self.panel();
                let id = r.id();
                match spot {
                    // The annotations' own area (a hosted widget takes its clicks).
                    Spot::Note => return true,
                    Spot::Plus(at, _) => {
                        self.comment_at(i, at);
                        return true;
                    }
                    Spot::Number(at, _) => {
                        self.press_number(i, at, app::is_event_shift());
                        return true;
                    }
                    Spot::Row(_) => {}
                }
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
                let (anchor, numbers) = {
                    let s = self.st.borrow();
                    (s.drag, s.gutter_drag.is_some())
                };
                if numbers {
                    let target = {
                        let mut s = self.st.borrow_mut();
                        let rh = s.metrics.rh.max(1) as i64;
                        if ey < 0 || ey > w.h() {
                            let max = (*s.ys.last().unwrap_or(&0) - w.h() as i64).max(0);
                            s.top = (s.top + if ey < 0 { -rh } else { rh }).clamp(0, max);
                            let t = s.top;
                            s.motion.jump(t);
                        }
                        s.row_at(ey.clamp(0, w.h() - 1))
                    };
                    if let Some(i) = target {
                        self.drag_number(i);
                    }
                    w.redraw();
                    return true;
                }
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
                let mut s = self.st.borrow_mut();
                s.drag = None;
                s.gutter_drag = None;
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
                let h = s.base[i];
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
                    // The comment button and the painted annotations.
                    if let Some((_, label)) = s.comment_target(i).filter(|_| s.on_comment.is_some()) {
                        c.child(COMMENT_KEY, |b| {
                            b.role(Role::Button).name(&label).bounds(me.x(), y, s.geo(me.w()).gutter, h).action(Action::Click);
                        });
                    }
                    let mut ny = y + h;
                    let g = s.geo(me.w());
                    let (nx, nw) = note_x(&g, &s.metrics);
                    for k in s.notes_of(i) {
                        let a = &s.notes[s.row_notes[k].1 as usize];
                        let nh = note_h(a);
                        let hosted = s.hosted.iter().any(|(hid, w)| *hid == a.id && !w.was_deleted());
                        if !hosted && nh > 0 {
                            let name = if a.label.is_empty() { tr!("Note under line {line}", line = a.at.line) } else { a.label.clone() };
                            c.child(NOTE_KEY + k as u64, |b| {
                                b.role(Role::Group).name(&name).bounds(me.x() + nx, ny, nw, nh).action(Action::ScrollIntoView);
                            });
                        }
                        ny += nh;
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
            match r.path.get(1).copied() {
                Some(COMMENT_KEY) if r.action == Action::Click => {
                    let at = v.st.borrow().comment_target(i).map(|(at, _)| at);
                    return at.is_some_and(|at| v.comment_at(i, at));
                }
                Some(k) if k >= NOTE_KEY && r.action == Action::ScrollIntoView => {
                    let id = v.st.borrow().row_notes.get((k - NOTE_KEY) as usize).map(|(_, n)| v.st.borrow().notes[*n as usize].id.clone());
                    return id.is_some_and(|id| v.reveal_annotation(&id));
                }
                Some(_) => return false,
                None => {}
            }
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

fn note_h(a: &Annotation) -> i32 {
    a.height.clamp(0, MAX_NOTE_H)
}

/// What a pointer is on, in a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spot {
    /// The annotations under the row.
    Note,
    /// The comment button of a half (split: 1 = right) for that line.
    Plus(LineRef, u8),
    /// A line number.
    Number(LineRef, u8),
    /// Anywhere else (code, markers, headers).
    Row(u8),
}

/// Left (relative to a gutter) and width of the comment button.
fn plus_x(m: &Metrics) -> (i32, i32) {
    (2, (m.rh - 4).max(10))
}

/// Left (relative to the view) and width of annotations: from the code
/// marker column of the first gutter to the right edge.
fn note_x(g: &Geo, m: &Metrics) -> (i32, i32) {
    (g.gutter, (g.w - g.gutter - m.pad).max(0))
}

/// True when `f` is `w` or inside it.
fn contains(w: &Widget, f: &Widget) -> bool {
    let id = w.as_widget_ptr();
    if f.as_widget_ptr() == id {
        return true;
    }
    let mut p = f.parent();
    while let Some(g) = p {
        if g.as_widget_ptr() == id {
            return true;
        }
        p = g.parent();
    }
    false
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
        // With comments, room for the "+" button left of the numbers.
        let lead = if self.on_comment.is_some() { plus_x(m).0 + plus_x(m).1 } else { 0 };
        let num_w = (digits(self.max_line_no).max(3) as f64 * m.cw).ceil() as i32 + 2 * m.pad + lead;
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
    /// Row contents are measured again only when the rows, width, font or
    /// wrap change; annotations only add their heights to the offsets
    /// (one pass of additions).
    fn layout(&mut self, width: i32) {
        self.metrics_now();
        let m = self.metrics;
        let key = (width, m.rh, m.cw.to_bits(), self.wrap, self.opts.split, self.rows_gen);
        if self.heights_key != Some(key) {
            self.heights_key = Some(key);
            self.heights_gen += 1;
            let g = self.geo(width);
            let cols = ((g.code_w as f64) / m.cw).floor().max(8.0) as usize;
            let p = &self.patch;
            let wrap = self.wrap;
            self.base.clear();
            self.base.extend(self.rows.iter().map(|r| match r.kind {
                RowKind::File => m.head_h,
                RowKind::Notice => m.note_h,
                RowKind::Hunk | RowKind::More => m.hunk_h,
                RowKind::Line | RowKind::Pair => {
                    if wrap {
                        let hk = &p.files[r.file as usize].hunks[r.hunk as usize];
                        let n = r.lines().into_iter().flatten().map(|l| rows::visual_lines(p.text(&hk.lines[l as usize]), cols)).max().unwrap_or(1);
                        m.rh * n as i32
                    } else {
                        m.rh
                    }
                }
            }));
        }
        let rk = (self.rows_gen, self.notes_map_gen);
        if self.row_notes_key != Some(rk) {
            self.row_notes_key = Some(rk);
            self.map_notes();
        }
        let yk = (self.heights_gen, self.notes_gen, self.rows_gen);
        if self.ys_key == Some(yk) {
            return;
        }
        self.ys_key = Some(yk);
        let mut ys = std::mem::take(&mut self.ys);
        ys.clear();
        ys.reserve(self.rows.len() + 1);
        let mut y = 0i64;
        ys.push(0);
        let mut k = 0;
        for (i, h) in self.base.iter().enumerate() {
            y += *h as i64;
            while k < self.row_notes.len() && self.row_notes[k].0 as usize == i {
                y += note_h(&self.notes[self.row_notes[k].1 as usize]) as i64;
                k += 1;
            }
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

    /// The rows of the annotations (those whose line is shown).
    fn map_notes(&mut self) {
        let mut out = Vec::new();
        for (k, a) in self.notes.iter().enumerate() {
            if let Some(i) = self.row_of_ref(a.file as u32, a.at) {
                out.push((i as u32, k as u32));
            }
        }
        out.sort_unstable();
        self.row_notes = out;
    }

    /// The row showing line `at` of `file`, when it is shown.
    fn row_of_ref(&self, file: u32, at: LineRef) -> Option<usize> {
        let (hunk, line) = review::find_line(&self.patch, file as usize, at.side, at.line)?;
        let i = self.locate(Anchor::Line { file, hunk, line })?;
        let r = self.rows[i];
        (matches!(r.kind, RowKind::Line | RowKind::Pair) && r.file == file && r.hunk == hunk && (r.a == line || r.b == line)).then_some(i)
    }

    /// The annotations under row `i`: a range of `row_notes`.
    fn notes_of(&self, i: usize) -> std::ops::Range<usize> {
        let a = self.row_notes.partition_point(|(r, _)| (*r as usize) < i);
        let b = a + self.row_notes[a..].iter().take_while(|(r, _)| *r as usize == i).count();
        a..b
    }

    /// Content y and height of annotation `k` when its line is shown.
    fn note_y(&self, k: usize) -> Option<(usize, i64, i32)> {
        let pos = self.row_notes.iter().position(|(_, n)| *n as usize == k)?;
        let row = self.row_notes[pos].0 as usize;
        let range = self.notes_of(row);
        let mut y = self.ys.get(row)? + *self.base.get(row)? as i64;
        for j in range.start..pos {
            y += note_h(&self.notes[self.row_notes[j].1 as usize]) as i64;
        }
        Some((row, y, note_h(&self.notes[k])))
    }

    /// The line Enter or the comment button of row `i` comments on (split:
    /// the right half when it has a line) and the button's accessible
    /// name ("Comment on lines 12–14 of src/x.rs" when the row is in the
    /// selected range).
    fn comment_target(&self, i: usize) -> Option<(LineRef, String)> {
        let at = self.line_ref_of(i, 1).or_else(|| self.line_ref_of(i, 0))?;
        let file = self.rows[i].file as usize;
        let path = self.patch.files.get(file)?.path().to_string();
        let range = self.range.filter(|r| r.file == file && self.span_of(r).is_some_and(|(a, b)| (a..=b).contains(&i)));
        let label = match range {
            Some(r) if r.start != r.end => tr!("Comment on lines {start}–{end} of {path}", start = r.start, end = r.end, path = path),
            _ => tr!("Comment on line {line} of {path}", line = at.line, path = path),
        };
        Some((at, label))
    }

    /// What is at `(ex, ey)` (view coordinates) of row `i`.
    fn spot(&self, i: usize, ex: i32, ey: i32, width: i32) -> Spot {
        let (Some(y0), Some(lh)) = (self.ys.get(i), self.base.get(i)) else { return Spot::Row(0) };
        if self.top + ey as i64 >= y0 + *lh as i64 {
            return Spot::Note;
        }
        let r = self.rows[i];
        if !matches!(r.kind, RowKind::Line | RowKind::Pair) {
            return Spot::Row(0);
        }
        let g = self.geo(width);
        let half = (r.kind == RowKind::Pair && ex >= g.half) as u8;
        let rx = ex - half as i32 * g.half;
        if !(0..g.gutter).contains(&rx) {
            return Spot::Row(half);
        }
        let (px, pw) = plus_x(&self.metrics);
        if self.on_comment.is_some() && (px..px + pw).contains(&rx) {
            if let Some(at) = self.line_ref_of(i, half) {
                return Spot::Plus(at, half);
            }
        }
        let at = if r.kind == RowKind::Line {
            let side = if rx < g.gutter / 2 { Side::Old } else { Side::New };
            self.number_of_row(i, side).map(|n| LineRef::new(side, n)).or_else(|| self.line_ref_of(i, 0))
        } else {
            self.line_ref_of(i, half)
        };
        at.map_or(Spot::Row(half), |at| Spot::Number(at, half))
    }

    /// Row `i`'s line number on `side` (unified: of its line; split: of
    /// the half of that side).
    fn number_of_row(&self, i: usize, side: Side) -> Option<u32> {
        let r = self.rows.get(i)?;
        let li = match (r.kind, side) {
            (RowKind::Line, _) => r.a,
            (RowKind::Pair, Side::Old) => r.a,
            (RowKind::Pair, Side::New) => r.b,
            _ => return None,
        };
        if li == NONE {
            return None;
        }
        let l = self.patch.files.get(r.file as usize)?.hunks.get(r.hunk as usize)?.lines.get(li as usize)?;
        review::number(l, side)
    }

    /// The line a click on half `half` of row `i` refers to (unified: the
    /// new side when the line has it).
    fn line_ref_of(&self, i: usize, half: u8) -> Option<LineRef> {
        let r = self.rows.get(i)?;
        let order = match (r.kind, half) {
            (RowKind::Pair, 0) => [Side::Old, Side::New],
            _ => [Side::New, Side::Old],
        };
        if r.kind == RowKind::Pair {
            // A half without a line refers to nothing.
            let side = order[0];
            return self.number_of_row(i, side).map(|n| LineRef::new(side, n));
        }
        order.into_iter().find_map(|side| self.number_of_row(i, side).map(|n| LineRef::new(side, n)))
    }

    /// First and last rows of a range: from the first row whose number on
    /// the range's side is in it to the last (rows between them that have
    /// no number on that side included).
    fn span_of(&self, r: &LineRange) -> Option<(usize, usize)> {
        let head = Row { kind: RowKind::File, file: r.file as u32, hunk: 0, a: NONE, b: NONE }.id();
        let start = self.ids.borrow().binary_search(&head).ok()?;
        let mut span: Option<(usize, usize)> = None;
        for i in start + 1..self.rows.len() {
            if self.rows[i].file != r.file as u32 {
                break;
            }
            if self.number_of_row(i, r.side).is_some_and(|n| (r.start..=r.end).contains(&n)) {
                span = Some((span.map_or(i, |s| s.0), i));
            }
        }
        span
    }

    /// The range of the lines of rows `ids` (one file), and its anchor line
    /// (of row `anchor`).
    fn range_of_rows(&self, ids: &[u64], anchor: Option<u64>) -> (Option<LineRange>, Option<u32>) {
        let index = |id: u64| self.ids.borrow().binary_search(&id).ok();
        let mut file = None;
        let mut rows = Vec::new();
        for &id in ids {
            let Some(i) = index(id) else { continue };
            let r = self.rows[i];
            if !matches!(r.kind, RowKind::Line | RowKind::Pair) {
                continue;
            }
            if file.is_some_and(|f| f != r.file) {
                return (None, None);
            }
            file = Some(r.file);
            rows.push(i);
        }
        let Some(file) = file else { return (None, None) };
        let has = |side| rows.iter().any(|&i| self.number_of_row(i, side).is_some());
        let side = if has(Side::New) { Side::New } else { Side::Old };
        let nums = rows.iter().filter_map(|&i| self.number_of_row(i, side));
        let (lo, hi) = nums.fold((u32::MAX, 0), |(lo, hi), n| (lo.min(n), hi.max(n)));
        if lo > hi {
            return (None, None);
        }
        let a = anchor.and_then(index).and_then(|i| self.number_of_row(i, side)).unwrap_or(lo);
        (Some(LineRange::between(file as usize, side, lo, hi)), Some(a.clamp(lo, hi)))
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
    // The view's clip already bounds the height; GDI rejects a clip region
    // past 16-bit coordinates (i32::MAX / 4 drew nothing on Windows).
    draw::push_clip(x, y, w, 16_000);
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
    if std::mem::take(&mut s.follow) {
        if let Some(i) = s.panel.cursor_index() {
            let (y0, y1) = (s.ys[i], s.ys[i + 1]);
            if y0 < s.top {
                s.top = y0;
            } else if y1 > s.top + view_h as i64 {
                // Its annotations too, but never past the line itself.
                s.top = (y1 - view_h as i64).min(y0);
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
            let lh = s.base[i];
            if draw::not_clipped(w.x(), ry, w.w(), lh) {
                draw_row(s, t, &ink, &m, &geo, w.x(), ry, lh, i, ring, wrap_cols);
                drawn += 1;
            }
            if rh > lh && draw::not_clipped(w.x(), ry + lh, w.w(), rh - lh) {
                draw_notes(s, t, &ink, &m, &geo, w.x(), ry + lh, i);
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
    let me = WeakView { widget: w.clone(), st: Rc::downgrade(st) };
    if let Some(id) = place_hosted(w, s, &geo, &me) {
        // Its line went away while it had focus: back to the diff (later:
        // not while drawing).
        app::add_timeout3(0.0, move |_| {
            if let Some(v) = me.upgrade() {
                v.focus_back(Some(id.clone()));
            }
        });
    }
}

/// The annotations under row `i`, from `y` down: a band tied to the line
/// (the gutter shaded) and what the painter draws (hosted widgets cover
/// theirs).
#[allow(clippy::too_many_arguments)]
fn draw_notes(s: &St, t: &Theme, ink: &Ink, m: &Metrics, g: &Geo, x: i32, mut y: i32, i: usize) {
    let current = s.panel.cursor() == s.rows.get(i).map(|r| r.id());
    let (nx, nw) = note_x(g, m);
    for k in s.notes_of(i) {
        let a = &s.notes[s.row_notes[k].1 as usize];
        let h = note_h(a);
        if h > 0 && draw::not_clipped(x, y, g.w, h) {
            paint::fill(x, y, g.w, h, 0, ink.bg);
            paint::fill(x, y, nx, h, 0, ink.empty);
            paint::fill(x + nx - 2, y, 2, h, 0, ink.rule);
            let hosted = s.hosted.iter().any(|(id, w)| *id == a.id && !w.was_deleted());
            if let (false, Some(p)) = (hosted, &s.painter) {
                draw::push_clip(x + nx, y, nw, h);
                (p.borrow_mut())(&mut AnnotationPaint { annotation: a, x: x + nx, y, w: nw, h, current, theme: t, under: ink.bg });
                draw::pop_clip();
            }
        }
        y += h;
    }
}

/// Moves hosted widgets over their annotations in view and hides the
/// others (a focused one stays, clipped, while its line is shown).
/// Returns the annotation whose widget lost its line while focused.
fn place_hosted(w: &Widget, s: &mut St, geo: &Geo, me: &WeakView) -> Option<String> {
    if s.hosted.is_empty() {
        return None;
    }
    if s.host.is_none() {
        let win = w.window()?;
        let mut win = fltk::group::Group::from_dyn_widget_ptr(win.as_widget_ptr())?;
        let cur = fltk::group::Group::try_current();
        fltk::group::Group::set_current(None::<&fltk::group::Group>);
        let mut g = fltk::group::Group::new(w.x(), w.y(), w.w(), w.h(), None);
        g.end();
        g.set_frame(FrameType::NoBox);
        g.set_clip_children(true);
        if let Some(c) = cur {
            fltk::group::Group::set_current(Some(&c));
        }
        win.add(&g);
        // Tab goes from the diff into its annotations, then on.
        crate::focus::follow(w, &g);
        let me = me.clone();
        g.handle(move |_, ev| {
            if ev == Event::KeyDown && app::event_key() == Key::Escape && !app::event_state().intersects(Shortcut::Ctrl | Shortcut::Alt | Shortcut::Shift | Shortcut::Meta) {
                if let Some(v) = me.upgrade() {
                    let id = v.focused_note();
                    if id.is_some() {
                        v.focus_back(id);
                        return true;
                    }
                }
            }
            false
        });
        s.host = Some(g);
    }
    let mut g = s.host.clone()?;
    let view_h = w.h();
    if (g.x(), g.y(), g.w(), g.h()) != (w.x(), w.y(), w.w() - BAR, view_h) {
        g.widget_resize(w.x(), w.y(), w.w() - BAR, view_h);
    }
    let focus = crate::focus::current();
    let (nx, nw) = note_x(geo, &s.metrics);
    let mut lost = None;
    let mut order: Vec<(i64, Widget)> = Vec::new();
    let hosted = s.hosted.clone();
    for (id, mut wid) in hosted {
        if wid.was_deleted() {
            continue;
        }
        if wid.parent().map(|p| p.as_widget_ptr()) != Some(g.as_widget_ptr()) {
            g.add(&wid);
        }
        let focused = focus.as_ref().is_some_and(|f| contains(&wid, f));
        let pos = s.notes.iter().position(|a| a.id == id).and_then(|k| s.note_y(k)).filter(|p| p.2 > 0);
        match pos {
            Some((_, y, h)) => {
                let sy = w.y() + (y - s.top) as i32;
                let in_view = sy < w.y() + view_h && sy + h > w.y();
                if in_view || focused {
                    // Out of view (focused): just outside, clipped away.
                    let sy = if in_view { sy } else if sy < w.y() { w.y() - h - 1 } else { w.y() + view_h + 1 };
                    if (wid.x(), wid.y(), wid.w(), wid.h()) != (w.x() + nx, sy, nw, h) {
                        wid.resize(w.x() + nx, sy, nw, h);
                    }
                    if !wid.visible() {
                        wid.show();
                    }
                    order.push((y, wid.clone()));
                    continue;
                }
                if wid.visible() {
                    wid.hide();
                }
            }
            None => {
                if focused {
                    lost = Some(id.clone());
                }
                if wid.visible() {
                    wid.hide();
                }
            }
        }
    }
    // Tab order follows the display order.
    order.sort_by_key(|(y, _)| *y);
    for (k, (_, wid)) in order.iter().enumerate() {
        if g.find(wid) != k as i32 {
            g.insert(wid, k as i32);
        }
    }
    g.redraw();
    lost
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
                let marks = line_marks(s, ink, r.file, r.hunk, li, true);
                draw_code(&Cells::new(p.text(l)), hx0 + g.code_x, y, g.code_w, m, ink.fg, wrap, s.hx, &marks);
            }
            paint::fill(x + g.half, y, 1, h, 0, ink.rule);
        }
    }
    // The comment button: on the line under the pointer and the cursor line.
    if s.on_comment.is_some() && matches!(r.kind, RowKind::Line | RowKind::Pair) {
        let focused = cursor && s.panel.has_focus();
        let pair = r.kind == RowKind::Pair;
        let preferred = (pair && r.b != NONE) as u8;
        for half in 0..=pair as u8 {
            let show = s.hover_line == Some((i, half)) || (focused && half == preferred);
            if show && s.line_ref_of(i, half).is_some() {
                draw_plus(t, ink, m, x + half as i32 * g.half, y);
            }
        }
    }
    if cursor && ring {
        paint::stroke(x, y, g.w, h, 0, t.controls.focus.border_width.max(1), t.border(&t.controls.focus, ink.bg));
    }
}

/// The "+" comment button at the left of a gutter starting at `x`.
fn draw_plus(t: &Theme, ink: &Ink, m: &Metrics, x: i32, y: i32) {
    let (px, pw) = plus_x(m);
    let ph = pw.min(m.rh);
    let by = y + (m.rh - ph) / 2;
    paint::fill(x + px, by, pw, ph, t.radius.min(4), ink.accent);
    paint::glyph(glyph::PLUS, x + px, by, pw, ph, t.font.caption, ink.bg);
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
