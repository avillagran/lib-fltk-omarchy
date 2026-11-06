//! `RichText`: a virtualized, selectable, keyboard-first view of a rich
//! text document, made for chat transcripts that stream.

use super::doc::*;
use super::layout::{self, estimate_of, layout_block, FltkMetrics, Layout, Metrics, Typo};
use super::markdown::{MarkdownOptions, MarkdownStream};
use super::store::{ShapeKind, Store};
use super::transcript::*;
use crate::paint;
use crate::state::State;
use crate::theme::{self, Rgb, Theme};
use crate::widgets::glyph;
use fltk::{app, draw, enums::*, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use unicode_segmentation::UnicodeSegmentation;

#[cfg(feature = "a11y")]
#[path = "access.rs"]
mod access;

#[path = "view_api.rs"]
mod api;

const BAR: i32 = 4;

/// A place in the document: block index and byte offset in its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct TextPos {
    pub block: usize,
    pub offset: usize,
}

/// Something reachable with Tab inside the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Link { block: usize, run: usize },
    /// The copy button of a code block.
    Copy { block: usize },
    /// A code block header action or a table footer action.
    Action { block: usize, slot: usize },
    /// An embed (when the app activates embeds).
    Embed { block: usize },
}

impl Target {
    fn block(self) -> usize {
        match self {
            Target::Link { block, .. }
            | Target::Copy { block }
            | Target::Action { block, .. }
            | Target::Embed { block } => block,
        }
    }

    fn moved(self, delta: i64) -> Target {
        let m = |b: usize| (b as i64 + delta).max(0) as usize;
        match self {
            Target::Link { block, run } => Target::Link { block: m(block), run },
            Target::Copy { block } => Target::Copy { block: m(block) },
            Target::Action { block, slot } => Target::Action { block: m(block), slot },
            Target::Embed { block } => Target::Embed { block: m(block) },
        }
    }
}

// ----------------------------------------------------------- prefix sums

/// Fenwick tree of block heights: y of a block and the block at a y in
/// O(log n), appends and tail truncation in O(log n).
#[derive(Debug, Default, Clone)]
struct Heights {
    t: Vec<i64>,
}

impl Heights {
    fn len(&self) -> usize {
        self.t.len()
    }
    fn prefix(&self, i: usize) -> i64 {
        let mut s = 0;
        let mut j = i.min(self.t.len());
        while j > 0 {
            s += self.t[j - 1];
            j &= j - 1;
        }
        s
    }
    fn add(&mut self, i: usize, d: i64) {
        let mut j = i + 1;
        while j <= self.t.len() {
            self.t[j - 1] += d;
            j += j & j.wrapping_neg();
        }
    }
    fn push(&mut self, v: i64) {
        let n = self.t.len();
        let j = n + 1;
        let lb = j & j.wrapping_neg();
        let below = self.prefix(n) - self.prefix(n + 1 - lb);
        self.t.push(v + below);
    }
    fn truncate(&mut self, n: usize) {
        self.t.truncate(n);
    }
    /// Replaces values `at..at + remove` with `vals`: O((n - at) log n).
    fn splice(&mut self, at: usize, remove: usize, vals: &[i64]) {
        let n = self.t.len();
        let end = (at + remove).min(n);
        let tail: Vec<i64> = (end..n).map(|i| self.get(i)).collect();
        self.t.truncate(at.min(n));
        for v in vals.iter().chain(&tail) {
            self.push(*v);
        }
    }
    /// The value at `i`.
    fn get(&self, i: usize) -> i64 {
        self.prefix(i + 1) - self.prefix(i)
    }
    /// Index of the block that holds y (the last one past the end).
    fn find(&self, y: i64) -> usize {
        let n = self.t.len();
        let (mut pos, mut rem) = (0usize, y);
        let mut step = if n == 0 {
            0
        } else {
            1usize << (usize::BITS - 1 - n.leading_zeros())
        };
        while step > 0 {
            if pos + step <= n && self.t[pos + step - 1] <= rem {
                pos += step;
                rem -= self.t[pos - 1];
            }
            step >>= 1;
        }
        pos.min(n.saturating_sub(1))
    }
}

// ---------------------------------------------------------------- state

/// Hashes block indices (keys of the hot map) with one multiply.
#[derive(Default, Clone, Copy)]
struct BuildIndexHasher;

#[derive(Default)]
struct IndexHasher(u64);

impl std::hash::Hasher for IndexHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0.rotate_left(5) ^ *b as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }
    fn write_usize(&mut self, n: usize) {
        self.0 = (n as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

impl std::hash::BuildHasher for BuildIndexHasher {
    type Hasher = IndexHasher;
    fn build_hasher(&self) -> IndexHasher {
        IndexHasher::default()
    }
}

/// A block near the view: materialized from the store, with its layout.
struct Hot {
    block: Block,
    layout: Option<Layout>,
    /// An earlier version of this block and its layout (incremental relayout).
    prev: Option<(Block, Layout)>,
    /// Header actions of a code block / footer actions of a table, laid
    /// out with the block.
    actions: Vec<ActionSlot>,
}

impl Hot {
    fn new(block: Block) -> Box<Hot> {
        Box::new(Hot {
            block,
            layout: None,
            prev: None,
            actions: Vec::new(),
        })
    }
}

/// An action button in a code block header or a table footer (block
/// coordinates: x and width, on the header / footer row).
#[derive(Debug, Clone)]
pub(super) struct ActionSlot {
    pub id: String,
    pub label: String,
    pub x: i32,
    pub w: i32,
}

/// Anchor offset of an untouched view: the very top, padding included.
const TOP: i64 = -(1 << 40);

/// Blocks far from the view drop their layout (their height stays) when
/// more than this many are hot.
const LIVE_MAX: usize = 600;

struct Press {
    x: i32,
    y: i32,
    link: Option<Arc<str>>,
    moved: bool,
}

/// A message key: the app's, or a number for sections added without one
/// (shown as `"\u{1}N"`; no allocation per message for long legacy
/// transcripts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MsgKey {
    Auto(u64),
    Named(Arc<str>),
}

impl MsgKey {
    fn auto_of(s: &str) -> Option<u64> {
        s.strip_prefix('\u{1}').and_then(|n| n.parse().ok())
    }

    pub fn is(&self, s: &str) -> bool {
        match self {
            MsgKey::Named(k) => **k == *s,
            MsgKey::Auto(n) => MsgKey::auto_of(s) == Some(*n),
        }
    }
}

impl From<&str> for MsgKey {
    fn from(s: &str) -> MsgKey {
        match MsgKey::auto_of(s) {
            Some(n) => MsgKey::Auto(n),
            None => MsgKey::Named(Arc::from(s)),
        }
    }
}

impl std::fmt::Display for MsgKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MsgKey::Named(k) => f.write_str(k),
            MsgKey::Auto(n) => write!(f, "\u{1}{n}"),
        }
    }
}

/// One message (a row of a transcript): a run of consecutive blocks.
pub(super) struct Msg {
    pub key: MsgKey,
    /// Open Markdown stream: pushes re-parse only its tail.
    pub stream: Option<Box<MarkdownStream>>,
    /// Length and hash of the Markdown it was set from: a later value that
    /// extends it streams only the new text.
    pub src: Option<(usize, u64)>,
}

/// What the view tells the app after a paint or an event (delivered with
/// the view's state released, so callbacks may change the view).
pub(super) enum Note {
    Shown(Arc<str>),
    Hidden(Arc<str>),
    Image(Arc<str>),
    Follow(bool),
    Far(bool),
    Selection,
    Action(BlockActionEvent),
    Embed(String, Option<(i32, i32)>),
}

/// State of an image source.
pub(super) enum ImageState {
    /// Asked for (`on_image`), no answer yet.
    Requested,
    Size(i32, i32),
    Pixels(i32, i32, fltk::image::RgbImage),
    /// Drawn by the embed painter, this tall.
    Painted(i32),
    Failed,
}

/// Find state: every match (block, byte range) in document order.
#[derive(Default)]
pub(super) struct Find {
    pub query: String,
    pub matches: Vec<(usize, Range<usize>)>,
    pub current: Option<usize>,
}

pub(super) type Painter = Rc<RefCell<dyn FnMut(&mut EmbedPaint)>>;
pub(super) type ActionsFn = Rc<dyn Fn(&str, &str) -> Vec<BlockAction>>;
pub(super) type TableActionsFn = Rc<dyn Fn(&Block) -> Vec<BlockAction>>;

pub(super) struct Inner {
    /// Every block, packed. A block laid out once keeps its height in
    /// `heights` (flagged `measured`) after its layout is dropped.
    store: Store,
    /// Blocks near the view (laid out, or about to be): at most about
    /// `LIVE_MAX`, keyed by block index.
    hot: HashMap<usize, Box<Hot>, BuildIndexHasher>,
    /// Gap above + height of each block.
    heights: Heights,
    /// Messages in order and their block counts (prefix sums).
    msgs: Vec<Msg>,
    counts: Heights,
    keys: HashMap<Arc<str>, usize>,
    keys_dirty: bool,
    auto_key: u64,
    /// The section `push_markdown` streams into.
    open: Option<MsgKey>,
    width: i32,
    typo: Typo,
    avg_char: f32,
    measured_avg: bool,
    top: i64,
    /// What the top of the view shows: block and offset from its slot.
    /// Height changes (layout, embeds, images, inserts above) keep it in
    /// place unless the view follows the end.
    anchor: (usize, i64),
    motion: crate::scroll::Scroll,
    stick: bool,
    stuck: bool,
    opts: MarkdownOptions,
    sel: Option<(TextPos, TextPos)>,
    selecting: bool,
    press: Option<Press>,
    pub hover: Option<Target>,
    pub focus: Option<Target>,
    copied: Option<(Target, Instant)>,
    thumb: Option<(i32, i64)>,
    pad: i32,
    /// Space under the content (a composer overlaid on the view).
    inset: i32,
    surface: Option<ColorRole>,
    laid_out: usize,
    bound: Option<(usize, u64)>,
    view_h: i32,
    frames: u64,
    paint_time: std::time::Duration,
    /// A new message fading in: its first block and the motion.
    reveal: Option<(usize, crate::anim::Tween)>,
    /// Embed ids and image sources → their blocks (rebuilt when stale).
    embeds: Option<HashMap<Arc<str>, Vec<usize>>>,
    /// Embeds and images in view at the last paint.
    shown: Vec<Arc<str>>,
    /// Someone listens to embed visibility.
    track: bool,
    /// Someone resolves images.
    want_images: bool,
    images: HashMap<Arc<str>, ImageState>,
    image_max_h: i32,
    painter: Option<Painter>,
    code_actions: Option<ActionsFn>,
    table_actions: bool,
    extra_table_actions: Option<TableActionsFn>,
    /// Embeds are Tab stops (an activate callback is set).
    embed_focus: bool,
    hosted: Vec<(Arc<str>, Widget)>,
    /// Scroll offset and geometry of the last frame drawn (scroll blits).
    drawn: Option<(i64, i32, i32, i32, i32, i32)>,
    /// While the pixels move: draw with this exact offset (no motion step).
    hold_motion: bool,
    /// Paint only the background, scrollbar and ring (the scrollbar column
    /// after a scroll blit): the text never reaches it.
    chrome_only: bool,
    /// Frames drawn by moving pixels (`fl_scroll`) instead of repainting.
    blits: u64,
    /// First block changed since the last frame by a streamed append (a
    /// frame that only grew the last message moves pixels up and repaints
    /// from here down).
    changed_from: Option<usize>,
    /// Frame rate of streamed appends (`stream_pace`); None: at once.
    stream_pace: Option<crate::anim::Pace>,
    /// Room kept under the last block after the bottom inset shrank while
    /// following (Rune's slack): new content fills it; what is left goes
    /// 120 ms after the content stopped growing.
    slack: i64,
    /// Content height at the last frame and when it last grew.
    grown: (i64, Option<Instant>),
    host: Option<fltk::group::Group>,
    pub(super) find: Option<Find>,
    notes: Vec<Note>,
    seen_follow: bool,
    /// `on_far_from_bottom`: (threshold px, last state reported).
    far: Option<(i64, bool)>,
    /// `pace_words`: text from a sender waits for the end of its last word.
    pace_words: bool,
    seen_sel: Option<(TextPos, TextPos)>,
    /// `native_mermaid`: closed Mermaid fences drawn as diagrams.
    #[cfg(feature = "mermaid")]
    pub(super) mermaid: Option<Box<crate::mermaid::rich::Native>>,
}

impl Inner {
    fn new() -> Inner {
        Inner {
            store: Store::default(),
            hot: HashMap::default(),
            heights: Heights::default(),
            msgs: Vec::new(),
            counts: Heights::default(),
            keys: HashMap::new(),
            keys_dirty: false,
            auto_key: 0,
            open: None,
            width: 0,
            typo: theme::with(Typo::from_theme),
            avg_char: 0.0,
            measured_avg: false,
            top: 0,
            anchor: (0, TOP),
            motion: crate::scroll::Scroll::new(),
            stick: false,
            stuck: false,
            opts: MarkdownOptions::default(),
            sel: None,
            selecting: false,
            press: None,
            hover: None,
            focus: None,
            copied: None,
            thumb: None,
            pad: theme::with(|t| t.spacing.xl),
            inset: 0,
            surface: None,
            laid_out: 0,
            bound: None,
            view_h: 0,
            frames: 0,
            paint_time: std::time::Duration::ZERO,
            reveal: None,
            embeds: None,
            shown: Vec::new(),
            track: false,
            want_images: false,
            images: HashMap::new(),
            image_max_h: 320,
            painter: None,
            code_actions: None,
            table_actions: false,
            extra_table_actions: None,
            embed_focus: false,
            hosted: Vec::new(),
            drawn: None,
            hold_motion: false,
            chrome_only: false,
            blits: 0,
            changed_from: None,
            stream_pace: None,
            slack: 0,
            grown: (0, None),
            host: None,
            find: None,
            notes: Vec::new(),
            seen_follow: false,
            far: None,
            pace_words: false,
            seen_sel: None,
            #[cfg(feature = "mermaid")]
            mermaid: None,
        }
    }

    fn avg(&self) -> f32 {
        if self.avg_char > 0.0 {
            self.avg_char
        } else {
            self.typo.body as f32 * 0.6
        }
    }

    fn gap(&self, i: usize) -> i32 {
        if i == 0 {
            0
        } else {
            self.typo
                .gap_of(Some(self.store.shape(i - 1)), self.store.shape(i))
        }
    }

    /// The laid out block `i` (blocks near the view).
    fn laid(&self, i: usize) -> Option<(&Block, &Layout)> {
        let h = self.hot.get(&i)?;
        Some((&h.block, h.layout.as_ref()?))
    }

    fn actions_of(&self, i: usize) -> &[ActionSlot] {
        self.hot.get(&i).map_or(&[], |h| &h.actions)
    }

    /// Height of a table's action footer.
    fn foot_h(&self) -> i32 {
        if self.table_actions {
            self.typo.code_header
        } else {
            0
        }
    }

    /// Display size of image `src` in a column `avail` wide, when known.
    fn image_size(&self, src: &str, avail: i32) -> Option<(i32, i32)> {
        let (w, h) = match self.images.get(src)? {
            ImageState::Size(w, h) | ImageState::Pixels(w, h, _) => (*w, *h),
            ImageState::Painted(h) => return Some((avail, (*h).max(0))),
            _ => return None,
        };
        if w <= 0 || h <= 0 {
            return None;
        }
        let s = (avail as f64 / w as f64)
            .min(self.image_max_h as f64 / h as f64)
            .min(1.0);
        Some((((w as f64 * s).round() as i32).max(1), ((h as f64 * s).round() as i32).max(1)))
    }

    fn image_h(&self, i: usize) -> Option<i32> {
        if !self.store.is_image(i) {
            return None;
        }
        let src = self.store.embed_id(i)?;
        let left = self.typo.left_of(self.store.shape(i)) - self.typo.list_w;
        self.image_size(src, self.width.max(40) - left).map(|s| s.1)
    }

    fn body_h(&self, i: usize) -> i32 {
        if let Some((_, l)) = self.laid(i) {
            return l.height;
        }
        if self.store.measured(i) && i < self.heights.len() {
            return (self.heights.get(i) - self.gap(i) as i64) as i32;
        }
        if let Some(h) = self.image_h(i) {
            return h;
        }
        let shape = self.store.shape(i);
        let foot = if matches!(shape.kind, ShapeKind::Table(_)) {
            self.foot_h()
        } else {
            0
        };
        estimate_of(shape, self.store.text(i), self.width.max(40), &self.typo, self.avg()) + foot
    }

    fn refresh_h(&mut self, i: usize) -> bool {
        let h = self.gap(i) + self.body_h(i);
        let d = h as i64 - self.heights.get(i);
        if d != 0 {
            self.heights.add(i, d);
        }
        d != 0
    }

    fn content_h(&self) -> i64 {
        self.heights.prefix(self.heights.len()) + 2 * self.pad as i64 + self.inset as i64 + self.slack
    }

    /// New content uses up the slack; notes when the content last grew.
    fn track_growth(&mut self) {
        let h = self.heights.prefix(self.heights.len());
        if h > self.grown.0 && self.grown.0 > 0 {
            self.slack = (self.slack - (h - self.grown.0)).max(0);
            self.grown.1 = Some(Instant::now());
        }
        self.grown.0 = h;
    }

    fn max_top(&self) -> i64 {
        (self.content_h() - self.view_h as i64).max(0)
    }

    fn at_bottom(&self) -> bool {
        self.top >= self.max_top() - 1
    }

    /// Height of the view that is not under the bottom inset.
    fn usable_h(&self) -> i32 {
        (self.view_h - self.inset).max(self.typo.line_h(self.typo.body))
    }

    // ------------------------------------------------------------ anchor

    /// Block and offset (from its slot) at content y `top`.
    fn locate(&self, top: i64) -> (usize, i64) {
        let y = top - self.pad as i64;
        if self.store.len() == 0 {
            return (0, y);
        }
        let k = self.heights.find(y.max(0));
        (k, y - self.heights.prefix(k))
    }

    /// A scroll by the user or the app: the anchor follows.
    fn set_top(&mut self, t: i64) {
        let max = self.max_top();
        self.top = t.clamp(0, max);
        self.motion.jump(self.top);
        self.anchor = self.locate(self.top);
    }

    /// Heights changed: put the anchor back where it was on screen (or the
    /// end, when following).
    fn reanchor(&mut self) {
        let n = self.store.len();
        if n == 0 {
            self.top = 0;
            self.anchor = (0, TOP);
            return;
        }
        let max = self.max_top();
        if self.stuck && !self.motion.running() {
            self.top = max;
            return;
        }
        let (k, off) = self.anchor;
        let k = k.min(n - 1);
        let new = (self.heights.prefix(k) + off + self.pad as i64).clamp(0, max);
        let d = new - self.top;
        if d != 0 {
            self.top = new;
            self.motion.shift(d);
        }
    }

    // ------------------------------------------------------------ splice

    /// Replaces blocks `at..at + remove` with `blocks`; unchanged ones
    /// keep their layout, a block that grew keeps its old layout for
    /// reuse. Costs O(blocks after the edit).
    fn splice(&mut self, at: usize, remove: usize, blocks: Vec<Block>) {
        let old_n = self.store.len();
        let at = at.min(old_n);
        let remove = remove.min(old_n - at);
        let end = at + remove;
        let add = blocks.len();
        let delta = add as i64 - remove as i64;
        // Compare with the old blocks before their bytes are dropped:
        // `Some(measured height)` for an unchanged block.
        let same: Vec<Option<Option<i32>>> = blocks
            .iter()
            .enumerate()
            .map(|(j, b)| {
                let k = at + j;
                let eq = j < remove
                    && match self.hot.get(&k) {
                        Some(h) => h.block == *b,
                        None => self.store.same(k, b),
                    };
                eq.then(|| {
                    (self.store.measured(k) && self.laid(k).is_none())
                        .then(|| (self.heights.get(k) - self.gap(k) as i64) as i32)
                })
            })
            .collect();
        // The first block after the edit keeps its body; its gap may change.
        let next_body = (end < old_n).then(|| self.heights.get(end) - self.gap(end) as i64);
        let mut old_hot: HashMap<usize, Box<Hot>> = HashMap::new();
        for k in at..end {
            if let Some(h) = self.hot.remove(&k) {
                old_hot.insert(k - at, h);
            }
        }
        if delta != 0 && end < old_n {
            let moved: Vec<usize> = self.hot.keys().copied().filter(|k| *k >= end).collect();
            let mut boxes: Vec<(usize, Box<Hot>)> =
                moved.into_iter().filter_map(|k| self.hot.remove(&k).map(|h| (k, h))).collect();
            for (k, h) in boxes.drain(..) {
                self.hot.insert((k as i64 + delta) as usize, h);
            }
        }
        let tail: Vec<i64> = (end..old_n).map(|k| self.heights.get(k)).collect();
        if let Some(f) = self.find.as_mut() {
            f.matches.retain(|(k, _)| *k < at || *k >= end);
            if delta != 0 {
                for m in f.matches.iter_mut().filter(|m| m.0 >= end) {
                    m.0 = (m.0 as i64 + delta) as usize;
                }
            }
        }
        self.update_embed_index(at, end, delta, &blocks);
        self.store.splice(at, remove, &blocks);
        self.heights.truncate(at);
        for (j, b) in blocks.into_iter().enumerate() {
            let k = at + j;
            let mut body = None;
            match same[j] {
                Some(m) => {
                    let h = old_hot.remove(&j);
                    if h.as_ref().is_some_and(|h| h.layout.is_some()) {
                        self.store.set_measured(k, true);
                    } else if let Some(m) = m {
                        self.store.set_measured(k, true);
                        body = Some(m);
                    }
                    if let Some(h) = h {
                        self.hot.insert(k, h);
                    }
                }
                None => {
                    if let Some(h) = old_hot.remove(&j) {
                        let prev = match h.layout {
                            Some(l) => Some((h.block, l)),
                            None => h.prev,
                        };
                        if prev.is_some() {
                            self.hot.insert(
                                k,
                                Box::new(Hot {
                                    block: b,
                                    layout: None,
                                    prev,
                                    actions: Vec::new(),
                                }),
                            );
                        }
                    }
                }
            }
            let h = self.gap(k) + body.unwrap_or_else(|| self.body_h(k));
            self.heights.push(h as i64);
        }
        for v in tail {
            self.heights.push(v);
        }
        if let Some(b) = next_body {
            let k = at + add;
            let d = self.gap(k) as i64 + b - self.heights.get(k);
            if d != 0 {
                self.heights.add(k, d);
            }
        }
        // New matches of the find query.
        if let Some(f) = self.find.as_mut() {
            let mut found = Vec::new();
            for k in at..at + add {
                for r in find_in(self.store.text(k), &f.query) {
                    found.push((k, r));
                }
            }
            if !found.is_empty() {
                let p = f.matches.partition_point(|(k, _)| *k < at);
                f.matches.splice(p..p, found);
            }
            if f.current.is_some_and(|c| c >= f.matches.len()) {
                f.current = f.matches.len().checked_sub(1);
            }
        }
        // Positions after the edit move with it; inside it they stay, clamped.
        let n = self.store.len();
        let map = |k: usize| -> usize {
            if k < at {
                k
            } else if k >= end {
                (k as i64 + delta) as usize
            } else {
                k.min((at + add).max(1) - 1)
            }
        };
        let clamp = |p: TextPos| TextPos {
            block: map(p.block).min(n.saturating_sub(1)),
            offset: p.offset,
        };
        if let Some((a, b)) = self.sel {
            self.sel = if n == 0 {
                None
            } else {
                Some((clamp(a), clamp(b)))
            };
        }
        if self.focus.is_some_and(|t| t.block() >= at) {
            self.focus = match self.focus {
                Some(t) if t.block() >= end => Some(t.moved(delta)),
                _ => None,
            };
        }
        if let Some((t, _)) = self.copied.as_mut() {
            if t.block() >= end {
                *t = t.moved(delta);
            }
        }
        if let Some((s, _)) = self.reveal.as_mut() {
            if *s >= end {
                *s = (*s as i64 + delta) as usize;
            }
        }
        let (k, off) = self.anchor;
        self.anchor = if k >= old_n {
            // Nothing laid out at the anchor yet (an empty view): the top.
            (k.min(n.saturating_sub(1)), off)
        } else if k >= at && k < end && add == 0 {
            (at.min(n.saturating_sub(1)), 0)
        } else {
            let m = map(k);
            // An untouched view keeps showing what it showed.
            (m, if off == TOP && m != k { -(self.pad as i64) } else { off })
        };
        self.hover = None;
    }

    /// Keeps the embed index valid across a splice (dropped when absent).
    fn update_embed_index(&mut self, at: usize, end: usize, delta: i64, blocks: &[Block]) {
        let Some(map) = self.embeds.as_mut() else { return };
        for v in map.values_mut() {
            v.retain(|k| *k < at || *k >= end);
            for k in v.iter_mut().filter(|k| **k >= end) {
                *k = (*k as i64 + delta) as usize;
            }
        }
        map.retain(|_, v| !v.is_empty());
        for (j, b) in blocks.iter().enumerate() {
            let id = match &b.kind {
                BlockKind::Embed(e) => e.id.as_str(),
                BlockKind::Image { src } => src.as_str(),
                _ => continue,
            };
            let v = map.entry(Arc::from(id)).or_default();
            v.push(at + j);
            v.sort_unstable();
        }
    }

    /// Blocks of embed `id` (or of image source `id`).
    fn embed_blocks(&mut self, id: &str) -> Vec<usize> {
        if self.embeds.is_none() {
            let mut map: HashMap<Arc<str>, Vec<usize>> = HashMap::new();
            for k in 0..self.store.len() {
                if let Some(id) = self.store.embed_id(k) {
                    map.entry(Arc::from(id)).or_default().push(k);
                }
            }
            self.embeds = Some(map);
        }
        self.embeds
            .as_ref()
            .and_then(|m| m.get(id).cloned())
            .unwrap_or_default()
    }

    /// Block `k`'s layout is stale (embed height, image state): measure
    /// it again, keeping the view still.
    fn invalidate(&mut self, k: usize) {
        if k >= self.store.len() {
            return;
        }
        if let Some(h) = self.hot.get_mut(&k) {
            h.layout = None;
            h.prev = None;
            h.block = self.store.block(k);
        }
        self.store.set_measured(k, false);
        self.refresh_h(k);
    }

    // ---------------------------------------------------------- messages

    fn key_index(&mut self, key: &str) -> Option<usize> {
        if let Some(n) = MsgKey::auto_of(key) {
            return self.index_of(&MsgKey::Auto(n));
        }
        if self.keys_dirty {
            self.keys = self
                .msgs
                .iter()
                .enumerate()
                .filter_map(|(j, m)| match &m.key {
                    MsgKey::Named(k) => Some((k.clone(), j)),
                    MsgKey::Auto(_) => None,
                })
                .collect();
            self.keys_dirty = false;
        }
        self.keys.get(key).copied()
    }

    fn index_of(&mut self, key: &MsgKey) -> Option<usize> {
        match key {
            // Sections without a key are nearly always the newest.
            MsgKey::Auto(_) => self.msgs.iter().rposition(|m| m.key == *key),
            MsgKey::Named(k) => {
                let k = k.clone();
                self.key_index(&k)
            }
        }
    }

    fn msg_start(&self, j: usize) -> usize {
        self.counts.prefix(j) as usize
    }

    fn msg_len(&self, j: usize) -> usize {
        self.counts.get(j) as usize
    }

    /// The message that holds block `k`.
    fn msg_of(&self, k: usize) -> Option<usize> {
        if self.msgs.is_empty() || k >= self.store.len() {
            return None;
        }
        Some(self.counts.find(k as i64))
    }

    fn new_key(&mut self) -> MsgKey {
        self.auto_key += 1;
        MsgKey::Auto(self.auto_key)
    }

    /// Inserts message `key` at message index `j` with these blocks.
    fn insert_msg(&mut self, j: usize, key: impl Into<MsgKey>, blocks: Vec<Block>, stream: Option<Box<MarkdownStream>>) {
        let key: MsgKey = key.into();
        let j = j.min(self.msgs.len());
        let start = self.msg_start(j);
        let n = blocks.len();
        self.splice(start, 0, blocks);
        self.msgs.insert(
            j,
            Msg {
                key: key.clone(),
                stream,
                src: None,
            },
        );
        if j == self.counts.len() {
            self.counts.push(n as i64);
        } else {
            self.counts.splice(j, 0, &[n as i64]);
        }
        if j + 1 == self.msgs.len() {
            if let (MsgKey::Named(k), false) = (key, self.keys_dirty) {
                self.keys.insert(k, j);
            }
        } else {
            self.keys_dirty = true;
        }
    }

    /// Replaces blocks `keep..` of message `j`.
    fn set_msg_blocks(&mut self, j: usize, keep: usize, blocks: Vec<Block>) {
        let start = self.msg_start(j);
        let len = self.msg_len(j);
        let keep = keep.min(len);
        let n = keep + blocks.len();
        self.splice(start + keep, len - keep, blocks);
        let from = start + keep;
        self.changed_from = Some(self.changed_from.map_or(from, |c| c.min(from)));
        let d = n as i64 - len as i64;
        if d != 0 {
            self.counts.add(j, d);
        }
        #[cfg(feature = "mermaid")]
        self.mermaid_closed();
    }

    /// Whether block `k` is a code block whose fence is still streaming
    /// (the last block of a message whose open tail has an odd number of
    /// fence lines).
    #[cfg(feature = "mermaid")]
    fn fence_open(&self, k: usize) -> bool {
        let Some(j) = self.msg_of(k) else { return false };
        let Some(s) = self.msgs[j].stream.as_ref() else { return false };
        if k + 1 != self.msg_start(j) + self.msg_len(j) {
            return false;
        }
        let fences = s
            .tail()
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                t.starts_with("```") || t.starts_with("~~~")
            })
            .count();
        fences % 2 == 1
    }

    /// Relays out Mermaid blocks laid out while streaming whose fence closed.
    #[cfg(feature = "mermaid")]
    fn mermaid_closed(&mut self) {
        let Some(mut n) = self.mermaid.take() else { return };
        let done = n.closed(|b| b < self.store.len() && self.store.shape(b).is_code() && self.fence_open(b));
        self.mermaid = Some(n);
        for b in done {
            self.invalidate(b);
        }
    }

    /// True when a push into message `j` can be painted by moving pixels:
    /// following the end, `j` is the last message, no fade in progress.
    pub(super) fn tail_push(&self, j: usize) -> bool {
        self.stuck && self.reveal.is_none() && j + 1 == self.msgs.len()
    }

    fn remove_msg(&mut self, j: usize) {
        if j >= self.msgs.len() {
            return;
        }
        let start = self.msg_start(j);
        let len = self.msg_len(j);
        self.splice(start, len, Vec::new());
        let last = j + 1 == self.msgs.len();
        let m = self.msgs.remove(j);
        self.counts.splice(j, 1, &[]);
        match (&m.key, last) {
            (MsgKey::Named(k), true) => {
                self.keys.remove(k);
            }
            (MsgKey::Auto(_), true) => {}
            _ => self.keys_dirty = true,
        }
        if self.open.as_ref() == Some(&m.key) {
            self.open = None;
        }
    }

    /// Streams `more` into message `j` (opening its stream if needed).
    fn push_msg(&mut self, j: usize, more: &str) {
        if more.is_empty() {
            return;
        }
        let opts = self.opts.clone();
        let s = self.msgs[j]
            .stream
            .get_or_insert_with(|| Box::new(MarkdownStream::with_options(opts)));
        let u = s.push(more);
        self.set_msg_blocks(j, u.keep, u.blocks);
    }

    /// Replaces message `j` with Markdown; streams when it extends the
    /// Markdown it was set from.
    fn set_msg_markdown(&mut self, j: usize, md: &str) {
        let extends = self.msgs[j].stream.is_some()
            && self.msgs[j].src.is_some_and(|(len, h)| {
                md.len() >= len && md.is_char_boundary(len) && fnv(FNV0, &md.as_bytes()[..len]) == h
            });
        if extends {
            let (len, h) = self.msgs[j].src.unwrap_or((0, FNV0));
            self.push_msg(j, &md[len..]);
            self.msgs[j].src = Some((md.len(), fnv(h, &md.as_bytes()[len..])));
            return;
        }
        let mut s = MarkdownStream::with_options(self.opts.clone());
        let u = s.push(md);
        self.set_msg_blocks(j, 0, u.blocks);
        self.msgs[j].stream = Some(Box::new(s));
        self.msgs[j].src = Some((md.len(), fnv(FNV0, md.as_bytes())));
    }

    /// The legacy open section: streams into it, or a new message.
    fn push_markdown(&mut self, more: &str) {
        if more.is_empty() {
            return;
        }
        let j = match self.open.clone().and_then(|k| self.index_of(&k)) {
            Some(j) => j,
            None => {
                let key = self.new_key();
                let j = self.msgs.len();
                self.insert_msg(j, key.clone(), Vec::new(), None);
                self.open = Some(key);
                j
            }
        };
        self.push_msg(j, more);
    }

    /// Opens a new section at the end (its stream starts empty).
    fn open_section(&mut self) -> usize {
        self.close_open();
        let key = self.new_key();
        let j = self.msgs.len();
        let s = Box::new(MarkdownStream::with_options(self.opts.clone()));
        self.insert_msg(j, key.clone(), Vec::new(), Some(s));
        self.open = Some(key);
        j
    }

    /// Closes the open section (frees its parser state).
    fn close_open(&mut self) {
        if let Some(k) = self.open.take() {
            if let Some(j) = self.index_of(&k) {
                self.end_stream(j);
            }
        }
    }

    /// Ends message `j`'s stream: a tail shown settled gets its exact parse.
    pub(super) fn end_stream(&mut self, j: usize) {
        if let Some(mut s) = self.msgs[j].stream.take() {
            if s.settles() {
                let u = s.finish();
                self.set_msg_blocks(j, u.keep, u.blocks);
            }
        }
        self.msgs[j].src = None;
        #[cfg(feature = "mermaid")]
        self.mermaid_closed();
    }

    fn clear(&mut self) {
        self.reveal = None;
        self.store.clear();
        self.hot.clear();
        self.heights = Heights::default();
        self.msgs.clear();
        self.counts = Heights::default();
        self.keys.clear();
        self.keys_dirty = false;
        self.open = None;
        self.sel = None;
        self.focus = None;
        self.hover = None;
        self.top = 0;
        self.anchor = (0, TOP);
        self.motion.jump(0);
        self.stuck = self.stick;
        self.bound = None;
        self.embeds = None;
        if let Some(f) = self.find.as_mut() {
            f.matches.clear();
            f.current = None;
        }
    }

    /// Re-estimates every block (width, font or theme change).
    fn relayout_all(&mut self) {
        let n = self.store.len();
        let (k, off) = self.anchor;
        let keep = (n > 0).then(|| (k.min(n - 1), off.min(self.heights.get(k.min(n - 1)))));
        self.hot.clear();
        let mut h = Heights::default();
        for i in 0..n {
            self.store.set_measured(i, false);
        }
        for i in 0..n {
            h.push((self.gap(i) + self.body_h(i)) as i64);
        }
        self.heights = h;
        if let Some(a) = keep {
            self.anchor = a;
            self.reanchor();
            self.motion.jump(self.top);
        }
    }

    fn ensure_layout(&mut self, i: usize, m: &mut dyn Metrics) -> bool {
        if self.laid(i).is_some() {
            return false;
        }
        if !self.hot.contains_key(&i) {
            let b = self.store.block(i);
            self.hot.insert(i, Hot::new(b));
        }
        let prev = self.hot.get_mut(&i).and_then(|h| h.prev.take());
        let Some(hot) = self.hot.get(&i) else { return false };
        let block = &hot.block;
        let mut l = layout_block(
            block,
            self.width,
            &self.typo,
            m,
            prev.as_ref().map(|p| (&p.0, &p.1)),
        );
        let small = layout::FontKey {
            size: self.typo.small,
            ..Default::default()
        };
        let pad = self.typo.code_pad / 2;
        // Right-aligned, in order, ending at `right`.
        let mut measure = |acts: Vec<BlockAction>, right: i32| -> Vec<ActionSlot> {
            let ws: Vec<i32> = acts.iter().map(|a| m.width(small, &a.label).ceil() as i32 + 2 * pad).collect();
            let mut x = right - ws.iter().sum::<i32>();
            acts.into_iter()
                .zip(ws)
                .map(|(a, w)| {
                    let s = ActionSlot {
                        id: a.id,
                        label: a.label,
                        x,
                        w,
                    };
                    x += w;
                    s
                })
                .collect()
        };
        let mut actions = Vec::new();
        match &block.kind {
            BlockKind::Code { lang } => {
                let mut acts = Vec::new();
                #[cfg(feature = "mermaid")]
                if crate::mermaid::rich::is_mermaid(lang) && self.mermaid.is_some() {
                    let open = self.fence_open(i);
                    if let Some(n) = self.mermaid.as_mut() {
                        acts.extend(n.layout(i, block.text(), open, &self.typo, &mut l));
                    }
                }
                if let Some(f) = &self.code_actions {
                    acts.extend(f(lang, block.text()));
                }
                if !acts.is_empty() {
                    let right = l.left + l.box_w - self.typo.code_header - 2;
                    actions = measure(acts, right);
                }
            }
            BlockKind::Table(_) if self.table_actions => {
                let mut acts = vec![
                    BlockAction::new(BlockAction::COPY_MARKDOWN, &crate::i18n::tr("Copy as Markdown")),
                    BlockAction::new(BlockAction::COPY_CSV, &crate::i18n::tr("Copy as CSV")),
                ];
                if let Some(f) = &self.extra_table_actions {
                    acts.extend(f(block));
                }
                let right = l.left + l.box_w;
                actions = measure(acts, right);
                l.height += self.typo.code_header;
            }
            BlockKind::Image { src } => {
                let avail = l.box_w + self.typo.list_w;
                if let Some((_, h)) = self.image_size(src, avail) {
                    l.lines.clear();
                    l.frags.clear();
                    l.height = h;
                }
            }
            _ => {}
        }
        if let Some(h) = self.hot.get_mut(&i) {
            h.actions = actions;
            h.layout = Some(l);
        }
        self.store.set_measured(i, true);
        self.laid_out += 1;
        self.refresh_h(i)
    }

    /// Visible block range for the current top.
    fn visible(&self) -> Range<usize> {
        let n = self.store.len();
        if n == 0 {
            return 0..0;
        }
        let y0 = (self.top - self.pad as i64).max(0);
        let a = self.heights.find(y0);
        let b = self.heights.find(y0 + self.view_h as i64);
        a..(b + 1).min(n)
    }

    /// Lays out what is visible (and half a view around), keeping the
    /// anchor where it was on screen.
    fn ensure_visible(&mut self, m: &mut dyn Metrics) {
        let n = self.store.len();
        if n == 0 {
            self.top = 0;
            return;
        }
        for _ in 0..8 {
            self.reanchor();
            let y0 = self.top - self.pad as i64;
            let over = self.view_h as i64 / 2;
            let first = self.heights.find((y0 - over).max(0));
            let last = self.heights.find((y0 + self.view_h as i64 + over).max(0));
            let mut changed = false;
            for k in first..=last {
                changed |= self.ensure_layout(k, m);
            }
            if !changed {
                break;
            }
        }
        self.reanchor();
        self.evict();
    }

    /// Drops the layouts of blocks far from the view when there are many.
    fn evict(&mut self) {
        if self.hot.len() <= LIVE_MAX {
            return;
        }
        let vis = self.visible();
        let margin = 64;
        let keep = vis.start.saturating_sub(margin)..vis.end + margin;
        self.hot.retain(|k, _| keep.contains(k));
    }

    /// A message appended after others fades in (o3 row arrival, 360 ms);
    /// streaming into it later costs nothing more.
    fn reveal_from(&mut self, start: usize) {
        self.reveal = (start > 0 && self.store.len() > start && crate::anim::enabled())
            .then(|| (start, crate::anim::Tween::start(crate::anim::ms::ROW_ARRIVE, crate::anim::Ease::OutCubic)));
    }

    fn screen_y(&self, i: usize, wy: i32) -> i32 {
        (wy as i64 + self.pad as i64 + self.heights.prefix(i) - self.top) as i32 + self.gap(i)
    }

    /// Block under window y (clamped), with its screen top.
    fn block_at(&self, wy: i32, ey: i32) -> Option<(usize, i32)> {
        if self.store.len() == 0 {
            return None;
        }
        let y = (ey as i64 - wy as i64 - self.pad as i64 + self.top).max(0);
        let i = self.heights.find(y);
        Some((i, self.screen_y(i, wy)))
    }

    fn pos_at(
        &mut self,
        wx: i32,
        wy: i32,
        ex: i32,
        ey: i32,
        m: &mut dyn Metrics,
    ) -> Option<TextPos> {
        let (i, by) = self.block_at(wy, ey)?;
        self.ensure_layout(i, m);
        let (b, l) = self.laid(i)?;
        // Above the first block or below the last: the ends.
        let rel_y = ey - by;
        if i + 1 == self.store.len() && rel_y >= l.height {
            return Some(TextPos {
                block: i,
                offset: b.text().len(),
            });
        }
        // Embeds and resolved images are one unit.
        if l.lines.is_empty() && !b.text().is_empty() {
            let end = if rel_y >= l.height / 2 { b.text().len() } else { 0 };
            return Some(TextPos { block: i, offset: end });
        }
        let off = l.hit(b.text(), ex - (wx + self.pad), rel_y, m);
        Some(TextPos {
            block: i,
            offset: off,
        })
    }

    fn target_at(
        &mut self,
        wx: i32,
        wy: i32,
        ex: i32,
        ey: i32,
        m: &mut dyn Metrics,
    ) -> Option<Target> {
        let (i, by) = self.block_at(wy, ey)?;
        self.ensure_layout(i, m);
        let bx = wx + self.pad;
        let hit = |x: i32, y: i32, w: i32, h: i32| ex >= x && ex < x + w && ey >= y && ey < y + h;
        let (b, l) = self.laid(i)?;
        if b.is_code() {
            let (x, y, w, h) = copy_rect(l, &self.typo, bx, by);
            if hit(x, y, w, h) {
                return Some(Target::Copy { block: i });
            }
            let hh = self.typo.code_header;
            return self
                .actions_of(i)
                .iter()
                .position(|a| hit(bx + a.x, by + 1, a.w, hh - 2))
                .map(|slot| Target::Action { block: i, slot });
        }
        if matches!(b.kind, BlockKind::Table(_)) && self.table_actions {
            let hh = self.typo.code_header;
            let fy = by + l.height - hh;
            if ey >= fy {
                return self
                    .actions_of(i)
                    .iter()
                    .position(|a| hit(bx + a.x, fy + 1, a.w, hh - 2))
                    .map(|slot| Target::Action { block: i, slot });
            }
        }
        if let BlockKind::Embed(_) = b.kind {
            return (self.embed_focus && hit(bx + l.left, by, l.box_w, l.height)).then_some(Target::Embed { block: i });
        }
        if let BlockKind::Image { src } = &b.kind {
            if l.lines.is_empty() {
                return hit(bx + l.left - self.typo.list_w, by, l.box_w + self.typo.list_w, l.height)
                    .then_some(Target::Link { block: i, run: 0 })
                    .filter(|_| !src.is_empty());
            }
        }
        let (rx, ry) = (ex - bx, ey - by);
        for ln in &l.lines {
            if ry < ln.y || ry >= ln.y + ln.h {
                continue;
            }
            for f in l.frags_of(ln) {
                let fx = ln.x as f32 + f.x;
                if (rx as f32) >= fx && (rx as f32) < fx + f.w {
                    let run = f.run as usize;
                    return b
                        .content
                        .runs()
                        .get(run)
                        .and_then(|r| r.style.link.as_ref())
                        .map(|_| Target::Link { block: i, run });
                }
            }
        }
        None
    }

    fn link_of(&self, t: Target) -> Option<Arc<str>> {
        match t {
            Target::Link { block, run } => match self.hot.get(&block) {
                Some(h) => h.block.content.runs().get(run)?.style.link.clone(),
                None => self.store.link(block, run).map(Arc::from),
            },
            _ => None,
        }
    }

    /// Links, block actions and copy buttons of the blocks in view, in
    /// reading order.
    fn targets(&self) -> Vec<Target> {
        let mut out = Vec::new();
        for i in self.visible() {
            let Some((b, l)) = self.laid(i) else { continue };
            if b.is_code() {
                for slot in 0..self.actions_of(i).len() {
                    out.push(Target::Action { block: i, slot });
                }
                out.push(Target::Copy { block: i });
                continue;
            }
            if matches!(b.kind, BlockKind::Embed(_)) {
                if self.embed_focus {
                    out.push(Target::Embed { block: i });
                }
                continue;
            }
            if matches!(b.kind, BlockKind::Image { .. }) && l.lines.is_empty() {
                out.push(Target::Link { block: i, run: 0 });
                continue;
            }
            for ln in &l.lines {
                for f in l.frags_of(ln) {
                    let run = f.run as usize;
                    if b.content
                        .runs()
                        .get(run)
                        .is_some_and(|r| r.style.link.is_some())
                    {
                        let t = Target::Link { block: i, run };
                        if !out.contains(&t) {
                            out.push(t);
                        }
                    }
                }
            }
            if matches!(b.kind, BlockKind::Table(_)) {
                for slot in 0..self.actions_of(i).len() {
                    out.push(Target::Action { block: i, slot });
                }
            }
        }
        out
    }

    fn sel_range(&self) -> Option<(TextPos, TextPos)> {
        let (a, b) = self.sel?;
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        (a != b).then_some((a, b))
    }

    fn selected_text(&self) -> String {
        let Some((a, b)) = self.sel_range() else {
            return String::new();
        };
        let mut out = String::new();
        for i in a.block..=b.block.min(self.store.len().saturating_sub(1)) {
            let t = self.store.text(i);
            let s = if i == a.block {
                floor_char(t, a.offset)
            } else {
                0
            };
            let e = if i == b.block {
                floor_char(t, b.offset)
            } else {
                t.len()
            };
            if s < e {
                out.push_str(&t[s..e]);
            }
            if i < b.block {
                out.push('\n');
            }
        }
        out
    }

    fn select_all(&mut self) {
        if let Some(last) = self.store.len().checked_sub(1) {
            self.sel = Some((
                TextPos::default(),
                TextPos {
                    block: last,
                    offset: self.store.text(last).len(),
                },
            ));
        }
    }

    fn scroll_by(&mut self, d: i64) -> bool {
        let max = self.max_top();
        let old = self.top;
        self.set_top(self.top + d);
        self.stuck = self.stick && self.top >= max;
        self.top != old
    }

    /// The event for action `slot` of block `k` (code or table).
    fn action_event(&self, k: usize, action: &str) -> BlockActionEvent {
        let b = match self.hot.get(&k) {
            Some(h) => h.block.clone(),
            None => self.store.block(k),
        };
        let (message, index) = match self.msg_of(k) {
            Some(j) => (self.msgs[j].key.to_string(), k - self.msg_start(j)),
            None => (String::new(), k),
        };
        let (lang, text) = match &b.kind {
            BlockKind::Code { lang } => (Some(lang.clone()), b.text().to_string()),
            BlockKind::Table(_) if action == BlockAction::COPY_CSV => (None, table_csv(&b).unwrap_or_default()),
            BlockKind::Table(_) => (None, table_markdown(&b).unwrap_or_default()),
            _ => (None, b.text().to_string()),
        };
        BlockActionEvent {
            action: action.to_string(),
            block: k,
            message,
            index,
            lang,
            text,
        }
    }

    /// Runs target `t` (Enter, Space, click): copies built-in actions and
    /// queues the app's callbacks. Returns the URL of a link to open.
    fn activate(&mut self, t: Target, at: Option<(i32, i32)>) -> Option<Arc<str>> {
        match t {
            Target::Link { .. } => return self.link_of(t),
            Target::Copy { block } => {
                let ev = self.action_event(block, BlockAction::COPY);
                app::copy(&ev.text);
                self.copied = Some((t, Instant::now()));
                self.notes.push(Note::Action(ev));
            }
            Target::Action { block, slot } => {
                let id = self.actions_of(block).get(slot).map(|a| a.id.clone())?;
                let ev = self.action_event(block, &id);
                #[cfg(feature = "mermaid")]
                if id == crate::mermaid::rich::SHOW_SOURCE || id == crate::mermaid::rich::SHOW_DIAGRAM {
                    if let Some(n) = self.mermaid.as_mut() {
                        n.toggle(&ev.text);
                        self.invalidate(block);
                        self.reanchor();
                    }
                }
                let builtin = matches!(self.store.shape(block).kind, ShapeKind::Table(_))
                    && (id == BlockAction::COPY_MARKDOWN || id == BlockAction::COPY_CSV);
                if builtin {
                    app::copy(&ev.text);
                    self.copied = Some((t, Instant::now()));
                }
                self.notes.push(Note::Action(ev));
            }
            Target::Embed { block } => {
                if let Some(id) = self.store.embed_id(block) {
                    self.notes.push(Note::Embed(id.to_string(), at));
                }
            }
        }
        None
    }
}

/// Case-insensitive matches of `q` in `t` (byte ranges, not overlapping).
pub(super) fn find_in(t: &str, q: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    if q.is_empty() {
        return out;
    }
    if q.is_ascii() {
        let (tb, qb) = (t.as_bytes(), q.as_bytes());
        let mut i = 0;
        while i + qb.len() <= tb.len() {
            if tb[i..i + qb.len()].eq_ignore_ascii_case(qb) && t.is_char_boundary(i) {
                out.push(i..i + qb.len());
                i += qb.len();
            } else {
                i += 1;
            }
        }
        return out;
    }
    let ql: Vec<char> = q.chars().flat_map(char::to_lowercase).collect();
    let mut starts = t.char_indices().peekable();
    while let Some((i, _)) = starts.next() {
        let mut want = ql.iter();
        let mut end = i;
        let mut ok = true;
        let mut chars = t[i..].char_indices();
        let mut pending: Vec<char> = Vec::new();
        loop {
            if pending.is_empty() {
                if want.len() == 0 {
                    break;
                }
                match chars.next() {
                    Some((j, c)) => {
                        pending = c.to_lowercase().collect();
                        pending.reverse();
                        end = i + j + c.len_utf8();
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            let Some(w) = want.next() else {
                // The query ended inside a folded char.
                ok = pending.is_empty();
                break;
            };
            if pending.pop() != Some(*w) {
                ok = false;
                break;
            }
        }
        if ok && pending.is_empty() {
            out.push(i..end);
            while starts.peek().is_some_and(|(j, _)| *j < end) {
                starts.next();
            }
        }
    }
    out
}

fn floor_char(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn copy_rect(l: &Layout, ty: &Typo, bx: i32, by: i32) -> (i32, i32, i32, i32) {
    let s = ty.code_header;
    (bx + l.left + l.box_w - s - 2, by + 1, s, s - 2)
}

// ----------------------------------------------------------------- widget

type Hook<F> = RefCell<Option<Box<F>>>;

/// App callbacks. They run with the view's state released (after a paint
/// or an event), so they may change the view.
#[derive(Default)]
pub(super) struct Hooks {
    pub link: Hook<dyn FnMut(&str)>,
    pub visibility: Hook<dyn FnMut(&str, bool)>,
    pub image: Hook<dyn FnMut(&str)>,
    pub action: Hook<dyn FnMut(&BlockActionEvent)>,
    pub embed: Hook<dyn FnMut(&str, Option<(i32, i32)>)>,
    pub follow: Hook<dyn FnMut(bool)>,
    pub far: Hook<dyn FnMut(bool)>,
    pub selection: Hook<dyn FnMut()>,
}

/// Delivers what the view queued for the app.
pub(super) fn dispatch(inn: &Rc<RefCell<Inner>>, hooks: &Hooks) {
    for _ in 0..4 {
        let notes = {
            let Ok(mut i) = inn.try_borrow_mut() else { return };
            let f = i.stuck;
            if f != i.seen_follow {
                i.seen_follow = f;
                i.notes.push(Note::Follow(f));
            }
            if let Some((min, was)) = i.far {
                // Following the end is never far (heights still settling
                // while content arrives do not count).
                let now = !i.stuck && i.max_top() - i.top > min;
                if now != was {
                    i.far = Some((min, now));
                    i.notes.push(Note::Far(now));
                }
            }
            let s = i.sel_range();
            if s != i.seen_sel {
                i.seen_sel = s;
                i.notes.push(Note::Selection);
            }
            std::mem::take(&mut i.notes)
        };
        if notes.is_empty() {
            return;
        }
        for n in notes {
            match n {
                Note::Shown(id) => call(&hooks.visibility, |f| f(&id, true)),
                Note::Hidden(id) => call(&hooks.visibility, |f| f(&id, false)),
                Note::Image(src) => call(&hooks.image, |f| f(&src)),
                Note::Follow(on) => call(&hooks.follow, |f| f(on)),
                Note::Far(on) => call(&hooks.far, |f| f(on)),
                Note::Selection => call(&hooks.selection, |f| f()),
                Note::Action(ev) => call(&hooks.action, |f| f(&ev)),
                Note::Embed(id, at) => call(&hooks.embed, |f| f(&id, at)),
            }
        }
    }
}

fn call<F: ?Sized>(h: &Hook<F>, run: impl FnOnce(&mut F)) {
    // A callback that replaces itself while running is skipped, not a panic.
    if let Ok(mut g) = h.try_borrow_mut() {
        if let Some(f) = g.as_mut() {
            run(f);
        }
    }
}

/// Rich text view: Markdown or [`Block`]s, word-wrapped, virtualized
/// (only the blocks in view are laid out and drawn), selectable, with
/// links, code blocks and tables. Built for chat transcripts: append
/// messages, stream tokens into the last one, and keep the view at the
/// bottom while it grows.
///
/// ```no_run
/// use fltk_omarchy::text::RichText;
/// let chat = RichText::new().stick_to_bottom(true).on_link(|url| println!("open {url}"));
/// chat.append_markdown("**You**\n\nWhat is a monad?");
/// chat.append_markdown("");                      // a new message for the reply…
/// chat.push_markdown("A monad is ");             // …streamed token by token
/// chat.push_markdown("a *monoid* in the category of endofunctors.");
/// ```
///
/// As a transcript host (o3): messages addressed by key
/// ([`set_message`](Self::set_message), [`insert_message`](Self::insert_message),
/// [`remove_message`](Self::remove_message), [`bind_model`](Self::bind_model)),
/// app-drawn cards ([`Block::embed`], [`embed_painter`](Self::embed_painter),
/// [`host_widget`](Self::host_widget)), scroll anchors by message key, a
/// bottom inset for an overlaid composer, code and table actions, images
/// resolved by the app, find and a selection rectangle.
///
/// Keys (focused): Up / Down, PageUp / PageDown, Home / End scroll; Tab and
/// Shift+Tab go through the links, block actions, code copy buttons and
/// activatable embeds in view, then on to the next control; Enter or Space
/// opens the focused link (or runs the action); Ctrl+A selects everything,
/// Ctrl+C copies the selection (Cmd on macOS); F3 / Shift+F3 (and Mod+G /
/// Shift+Mod+G) go to the next / previous find match; Esc clears the
/// selection.
#[derive(Clone)]
pub struct RichText {
    widget: Widget,
    inner: Rc<RefCell<Inner>>,
    hooks: Rc<Hooks>,
}
fltk::widget_extends!(RichText, Widget, widget);

impl Default for RichText {
    fn default() -> Self {
        Self::new()
    }
}

impl RichText {
    pub fn new() -> RichText {
        let mut widget = Widget::default().with_size(320, 240);
        widget.set_frame(FrameType::NoBox);
        widget.set_visible_focus();
        let rt = RichText {
            widget,
            inner: Rc::new(RefCell::new(Inner::new())),
            hooks: Rc::default(),
        };
        rt.install();
        crate::window::register_blitter(&rt.widget);
        rt
    }

    fn install(&self) {
        let (inn, hooks) = (self.inner.clone(), self.hooks.clone());
        let mut w = self.widget.clone();
        w.draw(move |w| {
            let t0 = Instant::now();
            {
                let mut i = inn.borrow_mut();
                // Only scrolled since the last frame: move the pixels.
                let only_scroll = w.damage_type() == Damage::Scroll;
                theme::with(|t| {
                    if !(only_scroll && blit_scroll(w, &mut i, t)) {
                        draw_view(w, &mut i, t);
                    }
                });
                i.frames += 1;
                i.paint_time += t0.elapsed();
            }
            dispatch(&inn, &hooks);
        });
        let (inn, hooks) = (self.inner.clone(), self.hooks.clone());
        w.handle(move |w, ev| {
            let r = handle(w, &inn, &hooks, ev);
            dispatch(&inn, &hooks);
            r
        });
        // Action labels are translated: lay out again in a new language.
        let (inn, mut ww) = (Rc::downgrade(&self.inner), self.widget.clone());
        let mut first = true;
        crate::i18n::bind(&self.widget, move || {
            if std::mem::take(&mut first) {
                return;
            }
            if let Some(inn) = inn.upgrade() {
                if let Ok(mut i) = inn.try_borrow_mut() {
                    i.relayout_all();
                }
                ww.redraw();
            }
        });
        #[cfg(feature = "a11y")]
        access::install(self);
        #[cfg(not(feature = "a11y"))]
        if crate::testkit::armed() {
            api::testkit_rows(self);
        }
    }

    // ------------------------------------------------------------ builders

    /// Follow the end while new text arrives, until the user scrolls up;
    /// End or scrolling back to the bottom follows again.
    pub fn stick_to_bottom(self, on: bool) -> Self {
        {
            let mut i = self.inner.borrow_mut();
            i.stick = on;
            i.stuck = on;
        }
        self
    }

    /// Called with the URL of a clicked link (or Enter on a focused one).
    /// Any scheme arrives as written (`t3-thread://…`, `file://…`).
    pub fn on_link(self, f: impl FnMut(&str) + 'static) -> Self {
        *self.hooks.link.borrow_mut() = Some(Box::new(f));
        self
    }

    /// Markdown conversion options (syntax highlighting, images, inline
    /// code path links).
    pub fn markdown_options(self, opts: MarkdownOptions) -> Self {
        self.inner.borrow_mut().opts = opts;
        self
    }

    /// Padding around the text (default: the theme's `xl` spacing).
    pub fn padding(self, px: i32) -> Self {
        self.inner.borrow_mut().pad = px.max(0);
        self
    }

    /// Background role (default: the window background).
    pub fn surface(self, role: ColorRole) -> Self {
        self.inner.borrow_mut().surface = Some(role);
        self
    }

    /// Shows the Markdown in `s` and follows it: when the new value extends
    /// the old one (streaming into a `State`), only the new text is parsed;
    /// any other change re-parses it all. `State::push_str` / `text_feed`
    /// say so directly (`Edit::Appended`), so the old text is not even
    /// re-hashed; plain `set` compares the prefix. For high-rate streams prefer
    /// [`push_markdown`](Self::push_markdown) or [`sender`](Self::sender).
    pub fn bind_markdown(self, s: &State<String>) -> Self {
        let (inn, mut w) = (self.inner.clone(), self.widget.clone());
        crate::state::bind_widget_edit(&self.widget, s, move |v: &String, edit| {
            let mut i = inn.borrow_mut();
            let (old_len, old_h) = i.bound.unwrap_or((0, FNV0));
            // `push_str` says it only appended: no need to hash the prefix.
            let extends = match edit {
                crate::state::Edit::Appended { from } if i.bound.is_some() && from >= old_len && v.len() >= old_len => true,
                _ => i.bound.is_some_and(|(len, h)| {
                    v.len() >= len && v.is_char_boundary(len) && fnv(FNV0, &v.as_bytes()[..len]) == h
                }),
            };
            if extends && i.open.is_some() {
                i.push_markdown(&v[old_len..]);
                i.bound = Some((v.len(), fnv(old_h, &v.as_bytes()[old_len..])));
            } else {
                i.clear();
                i.open_section();
                i.push_markdown(v);
                i.bound = Some((v.len(), fnv(FNV0, v.as_bytes())));
            }
            drop(i);
            w.redraw();
        });
        self
    }

    // ------------------------------------------------------------- content

    /// Replaces everything with this Markdown.
    pub fn set_markdown(&self, md: &str) {
        self.inner.borrow_mut().clear();
        self.append_markdown(md);
    }

    /// Appends a new section (a chat message) with this Markdown. It stays
    /// open: [`push_markdown`](Self::push_markdown) continues it.
    pub fn append_markdown(&self, md: &str) {
        {
            let mut i = self.inner.borrow_mut();
            let j = i.open_section();
            let start = i.msg_start(j);
            i.push_markdown(md);
            i.reveal_from(start);
        }
        self.changed();
    }

    /// Streams more Markdown into the open section (a new one if none):
    /// only its open tail is parsed again and only changed blocks are laid
    /// out again.
    pub fn push_markdown(&self, more: &str) {
        if more.is_empty() {
            return;
        }
        let tail = {
            let mut i = self.inner.borrow_mut();
            i.push_markdown(more);
            open_is_tail(&mut i)
        };
        damage_push(&mut self.widget.clone(), tail);
    }

    /// Closes the open section (frees its parser state).
    pub fn finish(&self) {
        self.inner.borrow_mut().close_open();
        self.changed();
    }

    /// Appends blocks built in code (closes the open section).
    pub fn append_blocks(&self, blocks: impl IntoIterator<Item = Block>) {
        {
            let mut i = self.inner.borrow_mut();
            i.close_open();
            let start = i.store.len();
            let key = i.new_key();
            let j = i.msgs.len();
            i.insert_msg(j, key, blocks.into_iter().collect(), None);
            i.reveal_from(start);
        }
        self.changed();
    }

    pub fn append_block(&self, b: Block) {
        self.append_blocks([b]);
    }

    pub fn set_document(&self, doc: Document) {
        self.inner.borrow_mut().clear();
        self.append_blocks(doc.blocks);
    }

    pub fn clear(&self) {
        self.inner.borrow_mut().clear();
        self.changed();
    }

    fn changed(&self) {
        self.widget.clone().redraw();
    }

    /// Frames for text streamed into the last message while following the
    /// end come at `pace` (e.g. [`Pace::Low`](crate::anim::Pace::Low), 30
    /// fps) instead of one per push: pushes in between are drawn together.
    pub fn stream_pace(self, pace: crate::anim::Pace) -> Self {
        self.inner.borrow_mut().stream_pace = Some(pace);
        self
    }

    /// Text from [`RichText::sender`] waits for the end of its last word
    /// (a partial word of up to 24 characters is held for at most 80 ms):
    /// no half-words reflow the line while a reply streams in.
    pub fn pace_words(self, on: bool) -> Self {
        self.inner.borrow_mut().pace_words = on;
        self
    }

    /// A `Send` handle that streams Markdown from any thread: text pushed
    /// between two frames is joined and parsed once on the UI thread.
    pub fn sender(&self) -> TextSender {
        let id = SENDER_ID.with(|n| {
            let v = n.get() + 1;
            n.set(v);
            v
        });
        // SAFETY: a live widget; the copy only tracks its deletion.
        let guard = unsafe { Widget::from_widget_ptr(self.widget.as_widget_ptr()) };
        SENDERS.with(|s| {
            let mut s = s.borrow_mut();
            s.retain(|(_, w, _)| !w.was_deleted());
            s.push((id, guard, Rc::downgrade(&self.inner)));
        });
        TextSender {
            id,
            buf: Arc::new(Mutex::new(Pending::default())),
        }
    }

    // ------------------------------------------------------------- queries

    pub fn block_count(&self) -> usize {
        self.inner.borrow().store.len()
    }

    pub fn block(&self, i: usize) -> Option<Block> {
        let inner = self.inner.borrow();
        if i >= inner.store.len() {
            return None;
        }
        Some(match inner.hot.get(&i) {
            Some(h) => h.block.clone(),
            None => inner.store.block(i),
        })
    }

    /// The whole text, one block per line (for accessibility and search);
    /// [`block_ranges`](Self::block_ranges) maps blocks into it.
    pub fn plain_text(&self) -> String {
        let i = self.inner.borrow();
        let n = i.store.len();
        let mut out = String::with_capacity((0..n).map(|k| i.store.text(k).len() + 1).sum());
        for k in 0..n {
            if k > 0 {
                out.push('\n');
            }
            out.push_str(i.store.text(k));
        }
        out
    }

    pub fn block_ranges(&self) -> Vec<Range<usize>> {
        let i = self.inner.borrow();
        let mut at = 0;
        (0..i.store.len())
            .map(|k| {
                let r = at..at + i.store.text(k).len();
                at = r.end + 1;
                r
            })
            .collect()
    }

    /// Selection as (start, end), ordered; None when nothing is selected.
    pub fn selection(&self) -> Option<(TextPos, TextPos)> {
        self.inner.borrow().sel_range()
    }

    pub fn set_selection(&self, a: TextPos, b: TextPos) {
        self.inner.borrow_mut().sel = Some((a, b));
        self.changed();
    }

    pub fn select_all(&self) {
        self.inner.borrow_mut().select_all();
        self.changed();
    }

    pub fn selected_text(&self) -> String {
        self.inner.borrow().selected_text()
    }

    /// Copies the selection to the clipboard; false when nothing is selected.
    pub fn copy_selection(&self) -> bool {
        let t = self.selected_text();
        if t.is_empty() {
            return false;
        }
        app::copy(&t);
        true
    }

    /// Scroll offset in pixels.
    pub fn scroll_offset(&self) -> i64 {
        self.inner.borrow().top
    }

    pub fn scroll_to(&self, top: i64) {
        {
            let mut i = self.inner.borrow_mut();
            let max = i.max_top();
            i.set_top(top);
            i.stuck = i.stick && i.top >= max;
        }
        // Only the offset changed: the next frame may move pixels.
        let mut w = self.widget.clone();
        w.set_damage_type(w.damage_type() | Damage::Scroll);
    }

    /// Frames drawn by moving the pixels already on screen instead of
    /// repainting the view (benchmarks, tests).
    pub fn scroll_blits(&self) -> u64 {
        self.inner.borrow().blits
    }

    pub fn scroll_to_bottom(&self) {
        {
            let mut i = self.inner.borrow_mut();
            i.stuck = i.stick;
            let top = i.max_top();
            i.set_top(top);
        }
        self.changed();
    }

    /// True when the view shows the end.
    pub fn is_at_bottom(&self) -> bool {
        self.inner.borrow().at_bottom()
    }

    /// Content height (estimates for blocks never laid out).
    pub fn content_height(&self) -> i64 {
        self.inner.borrow().content_h()
    }

    /// Frames painted and the time spent in them (layout included).
    pub fn paint_stats(&self) -> (u64, std::time::Duration) {
        let i = self.inner.borrow();
        (i.frames, i.paint_time)
    }

    /// Blocks laid out so far (proves virtualization in tests).
    pub fn blocks_laid_out(&self) -> usize {
        self.inner.borrow().laid_out
    }

    /// URL of the link focused with Tab.
    pub fn focused_link(&self) -> Option<String> {
        let i = self.inner.borrow();
        i.focus.and_then(|t| i.link_of(t)).map(|s| s.to_string())
    }

    /// Window rectangle (x, y, w, h) of the first occurrence of `needle`
    /// in the blocks in view, on one line (tests, tours, popups anchored
    /// to a word). Only blocks that have been drawn are searched.
    pub fn text_rect(&self, needle: &str) -> Option<(i32, i32, i32, i32)> {
        let i = self.inner.borrow();
        let mut m = FltkMetrics::default();
        let (wx, wy) = (self.widget.x(), self.widget.y());
        for k in i.visible() {
            let Some((b, l)) = i.laid(k) else { continue };
            let text = b.text();
            let Some(off) = text.find(needle) else {
                continue;
            };
            let Some(li) = l
                .lines
                .iter()
                .position(|ln| (ln.start as usize) <= off && off < ln.end as usize)
            else {
                continue;
            };
            let ln = &l.lines[li];
            let end = (off + needle.len()).min(ln.end as usize);
            // All its pieces in mixed-direction text.
            let (x0, x1) = l.span_x(text, li, off..end, &mut m);
            let by = i.screen_y(k, wy);
            return Some((
                wx + i.pad + x0.round() as i32,
                by + ln.y,
                (x1 - x0).round() as i32,
                ln.h,
            ));
        }
        None
    }
}

// ------------------------------------------------------------ cross-thread

#[derive(Default)]
struct Pending {
    text: String,
    finish: bool,
    posted: bool,
    last: Option<Instant>,
}

/// Streams Markdown into a [`RichText`] from any thread (see
/// [`RichText::sender`]). Text arrives on the UI thread in order, at most
/// 60 times a second: what accumulates in between is applied as one push
/// (one parse of the tail, one layout, one paint).
#[derive(Clone)]
pub struct TextSender {
    id: u64,
    buf: Arc<Mutex<Pending>>,
}

/// Live senders: id, the view's widget (deletion guard), its state.
type SenderEntry = (u64, Widget, Weak<RefCell<Inner>>);

thread_local! {
    static SENDER_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SENDERS: RefCell<Vec<SenderEntry>> = const { RefCell::new(Vec::new()) };
}

impl TextSender {
    pub fn push(&self, more: &str) {
        self.post(|p| p.text.push_str(more));
    }

    /// Closes the open section after the text pushed so far.
    pub fn finish(&self) {
        self.post(|p| p.finish = true);
    }

    fn post(&self, f: impl FnOnce(&mut Pending)) {
        let mut p = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut p);
        if p.posted {
            return;
        }
        p.posted = true;
        drop(p);
        let (id, buf) = (self.id, self.buf.clone());
        crate::task::post(move || schedule(id, buf));
    }
}

/// On the UI thread: delivers now, or at the next 60 Hz frame.
fn schedule(id: u64, buf: Arc<Mutex<Pending>>) {
    const FRAME: f64 = 1.0 / 60.0;
    let last = buf.lock().unwrap_or_else(|e| e.into_inner()).last;
    let wait = last.map_or(0.0, |t| FRAME - t.elapsed().as_secs_f64());
    if wait <= 0.0 {
        deliver(id, &buf);
    } else {
        app::add_timeout3(wait, move |_| deliver(id, &buf));
    }
}

/// With word pacing: the part of `text` that waits for the next delivery,
/// a trailing partial word of at most 24 characters (Rune's stream pacing:
/// no half-words reflowing the line).
fn held_word(text: &str) -> usize {
    if text.ends_with(|c: char| c.is_whitespace() || c.is_ascii_punctuation()) {
        return 0;
    }
    match text.rfind(|c: char| c.is_whitespace()) {
        Some(ws) if text[ws + 1..].chars().count() <= 24 => text.len() - (ws + 1),
        _ => 0,
    }
}

fn deliver(id: u64, buf: &Arc<Mutex<Pending>>) {
    let target = SENDERS.with(|s| {
        s.borrow()
            .iter()
            .find(|(i, w, _)| *i == id && !w.was_deleted())
            .map(|(_, w, inn)| (w.clone(), inn.clone()))
    });
    let paced = target
        .as_ref()
        .and_then(|(_, inn)| inn.upgrade())
        .is_some_and(|inn| inn.try_borrow().is_ok_and(|i| i.pace_words));
    let (text, finish) = {
        let mut p = buf.lock().unwrap_or_else(|e| e.into_inner());
        p.posted = false;
        p.last = Some(Instant::now());
        let finish = std::mem::replace(&mut p.finish, false);
        let mut text = std::mem::take(&mut p.text);
        let hold = if paced && !finish { held_word(&text) } else { 0 };
        if hold > 0 {
            // The partial word goes back; a later delivery takes it (one is
            // scheduled so it never waits for more than 80 ms).
            p.text = text.split_off(text.len() - hold);
            p.posted = true;
            let (id, b) = (id, buf.clone());
            app::add_timeout3(0.08, move |_| deliver_all(id, &b));
        }
        (text, finish)
    };
    let Some((mut w, inn)) = target else { return };
    let Some(inn) = inn.upgrade() else { return };
    let tail = {
        let mut i = inn.borrow_mut();
        i.push_markdown(&text);
        if finish {
            i.close_open();
        }
        if finish { None } else { open_is_tail(&mut i) }
    };
    damage_push(&mut w, tail);
}

/// Delivers everything pending, a held word included.
fn deliver_all(id: u64, buf: &Arc<Mutex<Pending>>) {
    let (text, finish) = {
        let mut p = buf.lock().unwrap_or_else(|e| e.into_inner());
        if !p.posted {
            return; // delivered meanwhile
        }
        p.posted = false;
        p.last = Some(Instant::now());
        (std::mem::take(&mut p.text), std::mem::replace(&mut p.finish, false))
    };
    let target = SENDERS.with(|s| {
        s.borrow()
            .iter()
            .find(|(i, w, _)| *i == id && !w.was_deleted())
            .map(|(_, w, inn)| (w.clone(), inn.clone()))
    });
    let Some((mut w, inn)) = target else { return };
    let Some(inn) = inn.upgrade() else { return };
    let tail = {
        let mut i = inn.borrow_mut();
        i.push_markdown(&text);
        if finish {
            i.close_open();
        }
        if finish { None } else { open_is_tail(&mut i) }
    };
    damage_push(&mut w, tail);
}

/// After a push: a frame that only grew the last message while following
/// the end moves pixels (`Damage::Scroll`); anything else repaints.
pub(super) fn damage_push(w: &mut Widget, tail: Option<Option<crate::anim::Pace>>) {
    match tail {
        // Paced: the frame comes on the shared clock (pushes in between
        // join it); else at once.
        Some(Some(p)) => crate::anim::request_scroll_frame_at(w, p),
        Some(None) => w.set_damage_type(w.damage_type() | Damage::Scroll),
        None => w.redraw(),
    }
}

/// Whether the open section is the last message and the view follows it.
fn open_is_tail(i: &mut Inner) -> Option<Option<crate::anim::Pace>> {
    let j = i.open.clone().and_then(|k| i.index_of(&k));
    j.is_some_and(|j| i.tail_push(j)).then_some(i.stream_pace)
}

const FNV0: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

// ---------------------------------------------------------------- drawing

fn sel_color(t: &Theme, under: Rgb) -> Rgb {
    t.palette
        .foreground
        .over(under, t.controls.selection_fill_alpha)
}

fn draw_view(w: &mut Widget, i: &mut Inner, t: &Theme) {
    let mut m = FltkMetrics::default();
    let ty = Typo::from_theme(t);
    if ty != i.typo || !i.measured_avg {
        i.typo = ty;
        let sample = "The quick brown fox jumps over the lazy dog";
        i.avg_char = m.width(
            layout::FontKey {
                size: i.typo.body,
                ..Default::default()
            },
            sample,
        ) / sample.chars().count() as f32;
        i.measured_avg = true;
        i.relayout_all();
    }
    let width = (w.w() - 2 * i.pad - BAR - 2).max(40);
    i.view_h = w.h();
    if width != i.width {
        i.width = width;
        i.relayout_all();
    }
    if !i.hold_motion {
        let max = i.max_top();
        let top = i.top;
        let moved = i.motion.position(top, max);
        if moved != top {
            i.top = moved;
            i.anchor = i.locate(moved);
        }
    }
    i.ensure_visible(&mut m);
    i.track_growth();
    if i.motion.running() && !i.hold_motion {
        // Wheel glide: frames that only move the content.
        crate::anim::request_scroll_frame(w);
    }
    let under = i.surface.map_or(t.palette.background, |r| r.rgb(t));
    draw::push_clip(w.x(), w.y(), w.w(), w.h());
    crate::glass::fill(w.x(), w.y(), w.w(), w.h(), 0, under);
    let sel = i.sel_range();
    let ring = crate::focus::show_ring(w);
    let focused = ring || crate::focus::is_focused(w);
    let bx = w.x() + i.pad;
    let clip = (w.y(), w.y() + w.h());
    let vis = i.visible();
    let painter = i.painter.clone();
    let mut pictures: Vec<(Arc<str>, i32, i32, i32, i32)> = Vec::new();
    let foot = i.foot_h();
    for k in vis.clone() {
        if i.chrome_only {
            break;
        }
        let by = i.screen_y(k, w.y());
        // Strips painted after a scroll blit: skip blocks outside the clip.
        let next = if k + 1 < i.store.len() { i.screen_y(k + 1, w.y()) } else { w.y() + w.h() };
        if !draw::not_clipped(w.x(), by - 16, w.w(), (next - by).max(1) + 32) {
            continue;
        }
        let Some((b, l)) = i.laid(k) else { continue };
        let range = sel.and_then(|(a, b)| {
            if k < a.block || k > b.block {
                return None;
            }
            let s0 = if k == a.block { a.offset } else { 0 };
            let e0 = if k == b.block { b.offset } else { usize::MAX };
            Some(s0..e0)
        });
        let hover = i.hover.filter(|h| h.block() == k);
        let focus = if focused { i.focus.filter(|h| h.block() == k) } else { None };
        let marks: Vec<(Range<usize>, bool)> = match &i.find {
            Some(f) => {
                let p = f.matches.partition_point(|(b, _)| *b < k);
                f.matches[p..]
                    .iter()
                    .take_while(|(b, _)| *b == k)
                    .enumerate()
                    .map(|(n, (_, r))| (r.clone(), f.current == Some(p + n)))
                    .collect()
            }
            None => Vec::new(),
        };
        // Embeds and resolved images: one box, drawn by the app or here.
        let unit = match &b.kind {
            BlockKind::Embed(_) => true,
            BlockKind::Image { .. } => l.lines.is_empty(),
            _ => false,
        };
        if unit {
            let (x, y, ww, hh) = (bx + l.left, by, l.box_w, l.height);
            let text = b.text();
            let whole = range.as_ref().is_some_and(|r| r.start == 0 && r.end >= text.len().max(1));
            let id = i.store.embed_id(k).unwrap_or("");
            let mut boxed = (x, ww);
            let state = match &b.kind {
                BlockKind::Image { src } => {
                    let left = l.left - i.typo.list_w;
                    let avail = l.box_w + i.typo.list_w;
                    let (dw, _) = i.image_size(src, avail).unwrap_or((avail, hh));
                    boxed = (bx + left, dw);
                    i.images.get(src.as_str())
                }
                _ => None,
            };
            let (x, ww) = boxed;
            draw::push_clip(x, y.max(clip.0), ww, (y + hh).min(clip.1) - y.max(clip.0));
            match state {
                Some(ImageState::Pixels(..)) => pictures.push((Arc::from(id), x, y, ww, hh)),
                Some(ImageState::Painted(_)) | None if painter.is_some() => {
                    if let Some(p) = &painter {
                        let mut ep = EmbedPaint {
                            id,
                            text,
                            x,
                            y,
                            w: ww,
                            h: hh,
                            hovered: hover.is_some(),
                            focused: focus.is_some(),
                            selected: whole,
                            image: state.is_some(),
                            theme: t,
                            under,
                        };
                        if let Ok(mut f) = p.try_borrow_mut() {
                            f(&mut ep);
                        }
                    }
                }
                _ => {
                    // No painter, or a size without pixels: a placeholder.
                    let surface = t.palette.lighter_background;
                    crate::glass::fill(x, y, ww, hh, t.radius.min(8), surface);
                    paint::stroke(x, y, ww, hh, t.radius.min(8), 1, t.palette.foreground.over(under, 0.12));
                    let dim = t.palette.foreground.darker(1.4);
                    if state.is_some() {
                        paint::glyph(IMAGE, x, y, ww, hh / 2 + i.typo.body, i.typo.body * 2, dim);
                        paint::text_elided(text, x + 4, y + hh / 2 + i.typo.body, ww - 8, i.typo.line_h(i.typo.small), i.typo.small, false, dim, Align::Center);
                    } else {
                        paint::text_elided(text, x + i.typo.code_pad, y, ww - 2 * i.typo.code_pad, hh.min(i.typo.line_h(i.typo.body) * 2), i.typo.body, false, dim, Align::Left);
                    }
                }
            }
            if whole {
                paint::alpha_fill(x, y, ww, hh, t.palette.foreground, t.controls.selection_fill_alpha);
            }
            if let Some((_, cur)) = marks.first() {
                paint::stroke(x, y, ww, hh, t.radius.min(8), 2, if *cur { t.palette.accent } else { t.palette.yellow });
            }
            if focus.is_some() {
                paint::stroke(x, y, ww, hh, t.radius.min(8), t.controls.focus.border_width.max(1), t.palette.accent);
            }
            draw::pop_clip();
            continue;
        }
        let prev_quote = if k > 0 { i.store.shape(k - 1).quote } else { 0 };
        let fresh = |at: &Instant| at.elapsed().as_secs_f32() < 1.5;
        let ctx = BlockCtx {
            t,
            ty: &i.typo,
            under,
            sel: range,
            hover,
            focus,
            copied: i
                .copied
                .is_some_and(|(c, at)| c == Target::Copy { block: k } && fresh(&at)),
            prev_quote,
            gap: i.gap(k),
            clip,
            color: None,
            actions: i.actions_of(k),
            copied_slot: match i.copied {
                Some((Target::Action { block, slot }, at)) if block == k && fresh(&at) => Some(slot),
                _ => None,
            },
            foot: if matches!(b.kind, BlockKind::Table(_)) { foot } else { 0 },
            marks: &marks,
        };
        #[cfg(feature = "mermaid")]
        let native = match &b.kind {
            BlockKind::Code { lang } if l.lines.is_empty() && crate::mermaid::rich::is_mermaid(lang) => {
                Some((b.text().to_string(), bx + l.left, by, l.box_w))
            }
            _ => None,
        };
        draw_block(&ctx, b, l, bx, by, &mut m);
        #[cfg(feature = "mermaid")]
        if let Some((src, x, y, ww)) = native {
            let unit = crate::native::pixels_per_unit(w);
            if let Some(n) = i.mermaid.as_mut() {
                n.paint(&src, x, y, ww, &i.typo, unit, t.palette.lighter_background);
            }
        }
    }
    for (src, x, y, ww, hh) in pictures {
        if let Some(ImageState::Pixels(_, _, img)) = i.images.get_mut(&src) {
            draw::push_clip(x, y.max(clip.0), ww, (y + hh).min(clip.1) - y.max(clip.0));
            img.scale(ww, hh, false, true);
            img.draw(x, y, ww, hh);
            draw::pop_clip();
        }
    }
    if !i.chrome_only {
        place_hosted(w, i, &vis);
        track_visible(i, &vis);
    }
    // A new message fading in over what is under it.
    if let Some((start, tw)) = i.reveal {
        if tw.done() || !crate::anim::enabled() || start >= i.store.len() {
            i.reveal = None;
        } else {
            let y0 = i.screen_y(start, w.y()).max(w.y());
            let bottom = w.y() + w.h();
            if y0 < bottom {
                paint::alpha_fill(w.x(), y0, w.w() - BAR - 2, bottom - y0, under, 1.0 - tw.value());
            }
            crate::anim::request_redraw(w);
        }
    }
    // Scrollbar (above the bottom inset).
    let content = i.content_h();
    if content > w.h() as i64 {
        let track = (w.h() - i.inset).max(24);
        let thumb = ((w.h() as f64 / content as f64) * track as f64).max(24.0) as i32;
        let maxt = i.max_top().max(1);
        let tyy = w.y() + ((i.top as f64 / maxt as f64) * (track - thumb).max(0) as f64) as i32;
        paint::fill(
            w.x() + w.w() - BAR,
            tyy,
            BAR,
            thumb,
            BAR / 2,
            t.palette.foreground.over(under, 0.25),
        );
    }
    draw::pop_clip();
    if ring && i.focus.is_none() {
        let c = t.border(&t.controls.focus, under);
        paint::stroke(
            w.x(),
            w.y(),
            w.w(),
            w.h(),
            t.radius,
            t.controls.focus.border_width.max(1),
            c,
        );
    }
    i.drawn = Some((i.top, w.x(), w.y(), w.w(), w.h(), i.width));
    if !i.hold_motion && !i.chrome_only {
        i.changed_from = None;
    }
}

/// Whether scroll-only frames move pixels (`OMARCHY_FLTK_SCROLL_BLIT=on|off`,
/// read once; default from [`crate::platform::scroll_blit_default`]).
fn scroll_blit_enabled() -> bool {
    thread_local! {
        static ON: bool = match std::env::var("OMARCHY_FLTK_SCROLL_BLIT").ok().as_deref() {
            Some("on" | "1") => true,
            Some("off" | "0") => false,
            _ => crate::platform::scroll_blit_default(),
        };
    }
    ON.with(|v| *v)
}

/// A frame that only scrolled: moves the pixels already on screen
/// (`fl_scroll`) and paints the strip it exposed, the frame around the
/// moved area and the scrollbar. False when a full paint is needed (first
/// frame, new size, hosted widgets or a fade in view, a jump of about a
/// screen, or a backend without `fl_scroll`).
fn blit_scroll(w: &mut Widget, i: &mut Inner, t: &Theme) -> bool {
    if !scroll_blit_enabled() {
        return false;
    }
    let Some((top0, x, y, ww, hh, width)) = i.drawn else { return false };
    if (x, y, ww, hh) != (w.x(), w.y(), w.w(), w.h()) || width != i.width || i.reveal.is_some() {
        return false;
    }
    if i.hosted.iter().any(|(_, h)| !h.was_deleted() && h.visible()) {
        return false;
    }
    // A streamed append: lay out the changed blocks first (their heights
    // grow, and the view follows the end), then move what is above them.
    let changed = i.changed_from;
    if changed.is_some() {
        let mut m = FltkMetrics::default();
        i.ensure_visible(&mut m);
    }
    let max = i.max_top();
    let moved = i.motion.position(i.top, max);
    if moved != i.top {
        i.top = moved;
        i.anchor = i.locate(moved);
    }
    if i.motion.running() {
        crate::anim::request_scroll_frame(w);
    }
    let dy = i.top - top0;
    // The moved area: the text column (the text never reaches the
    // scrollbar column), inside 2 px rows at the top and bottom (focus
    // ring), above the bottom inset (a composer drawn over the view).
    let (cx, cy) = (w.x(), w.y() + 2);
    let (cw, ch) = (w.w() - BAR - 2, w.h() - 4 - i.inset);
    // Where the changed blocks start on screen after the move.
    let repaint_from = match changed {
        Some(k) if k < i.store.len() => {
            let y = i.screen_y(k, w.y()) - 4;
            if y < cy {
                return false; // the change starts above the view: repaint all
            }
            Some(y)
        }
        Some(_) => Some(cy + ch),
        None => None,
    };
    if dy == 0 && repaint_from.is_none() {
        return true;
    }
    if cw <= 0 || ch <= 8 || dy.abs() >= (ch - 8) as i64 || dy < 0 && repaint_from.is_some() {
        return false;
    }
    i.hold_motion = true;
    let inset = i.inset;
    let wc = w.clone();
    let mut strip = |sx: i32, sy: i32, sw: i32, sh: i32| {
        if sw <= 0 || sh <= 0 {
            return;
        }
        draw::push_clip(sx, sy, sw, sh);
        let mut w2 = wc.clone();
        draw_view(&mut w2, i, t);
        draw::pop_clip();
    };
    let moved_pixels = if dy == 0 { true } else { crate::platform::fl_abi::scroll_pixels(cx, cy, cw, ch, 0, -(dy as i32), &mut strip) };
    if moved_pixels {
        if let Some(y) = repaint_from {
            strip(cx, y, cw, cy + ch - y);
        }
        // Around the moved area: the ring rows, the inset, and the
        // scrollbar column (background, thumb and ring only).
        strip(w.x(), w.y(), w.w(), 2);
        strip(w.x(), w.y() + w.h() - 2 - inset, w.w(), 2 + inset);
        i.chrome_only = true;
        draw::push_clip(cx + cw, w.y(), w.x() + w.w() - (cx + cw), w.h());
        let mut w2 = w.clone();
        draw_view(&mut w2, i, t);
        draw::pop_clip();
        i.chrome_only = false;
    }
    i.hold_motion = false;
    i.changed_from = None;
    if moved_pixels {
        i.blits += 1;
        i.drawn = Some((i.top, w.x(), w.y(), w.w(), w.h(), i.width));
    }
    moved_pixels
}

/// Moves hosted widgets over their embeds in view and hides the others.
fn place_hosted(w: &Widget, i: &mut Inner, vis: &Range<usize>) {
    if i.hosted.is_empty() {
        return;
    }
    if i.host.is_none() {
        let Some(win) = w.window() else { return };
        let Some(mut win) = fltk::group::Group::from_dyn_widget_ptr(win.as_widget_ptr()) else { return };
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
        i.host = Some(g);
    }
    let Some(mut g) = i.host.clone() else { return };
    let h = (w.h() - i.inset).max(0);
    if (g.x(), g.y(), g.w(), g.h()) != (w.x(), w.y(), w.w() - BAR - 2, h) {
        g.widget_resize(w.x(), w.y(), w.w() - BAR - 2, h);
    }
    let hosted = i.hosted.clone();
    for (id, mut wid) in hosted {
        if wid.was_deleted() {
            continue;
        }
        if wid.parent().map(|p| p.as_widget_ptr()) != Some(g.as_widget_ptr()) {
            g.add(&wid);
        }
        let rect = i
            .embed_blocks(&id)
            .into_iter()
            .find(|k| vis.contains(k))
            .and_then(|k| i.laid(k).map(|(_, l)| (k, l.left, l.box_w, l.height)))
            .map(|(k, left, bw, bh)| (w.x() + i.pad + left, i.screen_y(k, w.y()), bw, bh));
        match rect {
            Some((x, y, ww, hh)) if y < w.y() + h && y + hh > w.y() => {
                if (wid.x(), wid.y(), wid.w(), wid.h()) != (x, y, ww, hh) {
                    wid.resize(x, y, ww, hh);
                }
                if !wid.visible() {
                    wid.show();
                }
            }
            _ => {
                if wid.visible() {
                    wid.hide();
                }
            }
        }
    }
    g.redraw();
}

/// Queues "entered / left the view" for embeds and images, and the first
/// request of an image source.
fn track_visible(i: &mut Inner, vis: &Range<usize>) {
    if !i.track && !i.want_images {
        return;
    }
    let y0 = i.top - i.pad as i64;
    let y1 = y0 + (i.view_h - i.inset) as i64;
    let mut now: Vec<Arc<str>> = Vec::new();
    for k in vis.clone() {
        let Some(id) = i.store.embed_id(k) else { continue };
        let top = i.heights.prefix(k) + i.gap(k) as i64;
        let bottom = i.heights.prefix(k + 1);
        if bottom <= y0 || top >= y1 {
            continue;
        }
        if i.want_images && i.store.is_image(k) && !i.images.contains_key(id) {
            let src: Arc<str> = Arc::from(id);
            i.images.insert(src.clone(), ImageState::Requested);
            i.notes.push(Note::Image(src));
        }
        if i.track && !now.iter().any(|n| **n == *id) {
            now.push(Arc::from(id));
        }
    }
    if !i.track {
        return;
    }
    let old = std::mem::take(&mut i.shown);
    for id in &old {
        if !now.contains(id) {
            i.notes.push(Note::Hidden(id.clone()));
        }
    }
    for id in &now {
        if !old.contains(id) {
            i.notes.push(Note::Shown(id.clone()));
        }
    }
    i.shown = now;
}

pub(super) struct BlockCtx<'a> {
    pub t: &'a Theme,
    pub ty: &'a Typo,
    pub under: Rgb,
    pub sel: Option<Range<usize>>,
    pub hover: Option<Target>,
    pub focus: Option<Target>,
    pub copied: bool,
    pub prev_quote: u8,
    pub gap: i32,
    pub clip: (i32, i32),
    /// Text color for runs without a color role (labels).
    pub color: Option<Rgb>,
    /// Code header / table footer actions of the block.
    pub actions: &'a [ActionSlot],
    /// The action that just copied (a check mark for a moment).
    pub copied_slot: Option<usize>,
    /// Height of the table action footer (inside `l.height`).
    pub foot: i32,
    /// Find matches in the block (byte ranges; true for the current one).
    pub marks: &'a [(Range<usize>, bool)],
}

/// Draws a row of small action buttons (code header, table footer).
fn draw_actions(c: &BlockCtx, bx: i32, y: i32, h: i32, surface: Rgb) {
    let t = c.t;
    let p = &t.palette;
    for (slot, a) in c.actions.iter().enumerate() {
        let me = |o: Option<Target>| matches!(o, Some(Target::Action { slot: s, .. }) if s == slot);
        let (x, w) = (bx + a.x, a.w);
        if me(c.hover) || me(c.focus) {
            paint::fill(x, y + 1, w, h - 2, t.radius.min(4), t.fill(&t.controls.hover, surface));
        }
        let (label, color) = if c.copied_slot == Some(slot) {
            (glyph::CHECKMARK, p.green)
        } else {
            (a.label.as_str(), if me(c.hover) { p.accent } else { p.foreground.darker(1.4) })
        };
        paint::text(label, x, y, w, h, c.ty.small, false, color, Align::Center);
        if me(c.focus) {
            paint::stroke(x, y + 1, w, h - 2, t.radius.min(4), 1, p.accent);
        }
    }
}

pub(super) fn draw_block(
    c: &BlockCtx,
    b: &Block,
    l: &Layout,
    bx: i32,
    by: i32,
    m: &mut FltkMetrics,
) {
    let t = c.t;
    let p = &t.palette;
    let ty = c.ty;
    let text = b.text();
    let fg = p.foreground;
    let hair = fg.over(c.under, 0.12);
    // Quote bars (joined to the block above when it is quoted too).
    for q in 0..b.quote {
        // Right-to-left blocks: bars on the right.
        let x = bx + l.mirror_x(q as i32 * ty.quote_w, 3);
        let up = if c.prev_quote > q { c.gap } else { 0 };
        let color = b
            .quote_tone
            .filter(|_| q + 1 == b.quote)
            .map_or(fg.over(c.under, 0.3), |r| r.rgb(t));
        paint::fill(x, by - up, 3, l.height + up, 1, color);
    }
    let surface = p.lighter_background;
    match &b.kind {
        BlockKind::Code { lang } => {
            let (x, w) = (bx + l.left, l.box_w);
            crate::glass::fill(x, by, w, l.height, t.radius.min(8), surface);
            paint::stroke(x, by, w, l.height, t.radius.min(8), 1, hair);
            if !lang.is_empty() {
                paint::text(
                    lang,
                    x + ty.code_pad,
                    by,
                    w / 2,
                    ty.code_header,
                    ty.small,
                    false,
                    fg.darker(1.4),
                    Align::Left,
                );
            }
            draw_actions(c, bx, by, ty.code_header, surface);
            let (cx, cy, cw, ch) = copy_rect(l, ty, bx, by);
            let on_copy = |o: Option<Target>| matches!(o, Some(Target::Copy { .. }));
            if on_copy(c.hover) || on_copy(c.focus) {
                paint::fill(
                    cx,
                    cy,
                    cw,
                    ch,
                    t.radius.min(4),
                    t.fill(&t.controls.hover, surface),
                );
            }
            let g = if c.copied { glyph::CHECK } else { COPY };
            paint::glyph(
                g,
                cx,
                cy,
                cw,
                ch,
                ty.small + 2,
                if c.copied { p.green } else { fg.darker(1.4) },
            );
            if on_copy(c.focus) {
                paint::stroke(cx, cy, cw, ch, t.radius.min(4), 1, p.accent);
            }
        }
        BlockKind::Rule => {
            paint::fill(
                bx + l.left,
                by + l.height / 2,
                l.box_w,
                1,
                0,
                fg.over(c.under, 0.2),
            );
        }
        BlockKind::Table(_) => {
            if c.foot > 0 {
                draw_actions(c, bx, by + l.height - c.foot, c.foot, c.under);
            }
            let th = l.height - c.foot;
            if let Some(g) = &l.table {
                let x0 = bx + l.left;
                if g.header_rows > 0 {
                    let hy = g.rows.get(g.header_rows - 1).map_or(0, |(y, h)| y + h);
                    crate::glass::fill(x0, by, l.box_w, hy, 0, surface);
                }
                for (ri, (ry, _)) in g.rows.iter().enumerate().skip(1) {
                    let _ = ri;
                    paint::fill(x0, by + ry, l.box_w, 1, 0, hair);
                }
                // Every column edge but the table's left one (columns run
                // right to left in a right-to-left table).
                for (cx, _) in g.cols.iter().filter(|(cx, _)| *cx > l.left) {
                    paint::fill(bx + cx, by, 1, th - 1, 0, hair);
                }
                paint::stroke(x0, by, l.box_w, th, 0, 1, hair);
            }
        }
        BlockKind::Heading(lv) if *lv <= 2 => {
            paint::fill(
                bx + l.mirror_x(l.left, l.box_w),
                by + l.height - ty.item_gap - 1,
                l.box_w,
                1,
                0,
                hair,
            );
        }
        _ => {}
    }
    // List marker on the first line.
    if let (Some(mk), Some(first)) = (b.marker, l.lines.first()) {
        let base = by + first.y + first.base;
        let size = ty.body;
        let mx = bx + l.left - ty.list_w;
        let dim = fg.darker(1.4);
        layout::FltkMetrics::set(layout::FontKey {
            size,
            ..Default::default()
        });
        match mk {
            ListMarker::Bullet => {
                let d = (size / 3).max(4);
                let cx = bx + l.mirror_x(mx - bx + ty.list_w / 2 - d / 2, d);
                let cy = by + first.y + first.h / 2 - d / 2;
                draw::set_draw_color(dim.fl());
                match b.indent {
                    0 | 1 => draw::draw_pie(cx, cy, d, d, 0.0, 360.0),
                    2 => draw::draw_arc(cx, cy, d, d, 0.0, 360.0),
                    _ => draw::draw_rectf(cx, cy, d, d),
                }
            }
            ListMarker::Ordered(n) => {
                let s = format!("{n}.");
                let wn = m.width(
                    layout::FontKey {
                        size,
                        ..Default::default()
                    },
                    &s,
                );
                draw::set_draw_color(dim.fl());
                let wn = wn.ceil() as i32;
                draw::draw_text_n(&s, bx + l.mirror_x(l.left - 6 - wn, wn), base);
            }
            ListMarker::Task(done) => {
                let d = (size as f32 * 1.05).round() as i32;
                let cx = bx + l.mirror_x(mx - bx + ty.list_w / 2 - d / 2, d);
                let cy = by + first.y + (first.h - d) / 2;
                if done {
                    paint::fill(cx, cy, d, d, 3, p.accent);
                    paint::glyph(glyph::CHECKMARK, cx, cy, d, d, size, p.background);
                } else {
                    paint::stroke(cx, cy, d, d, 3, 1, dim);
                }
            }
        }
    }
    // An image not resolved yet: its glyph before the alt text.
    if let (BlockKind::Image { .. }, Some(first)) = (&b.kind, l.lines.first()) {
        let gx = bx + l.mirror_x(l.left - ty.list_w, ty.list_w);
        paint::glyph(IMAGE, gx, by + first.y, ty.list_w, first.h, ty.body + 2, fg.darker(1.4));
    }
    // Find matches under the text.
    for (r, current) in c.marks {
        let r = r.start.min(text.len())..r.end.min(text.len());
        let color = if *current { p.accent } else { p.yellow };
        let a = if *current { 0.55 } else { 0.3 };
        for (x, y, w, h) in l.rects(text, r, m) {
            paint::alpha_fill(bx + x, by + y, w, h, color, a);
        }
    }
    // Selection under the text.
    if let Some(r) = &c.sel {
        let r = r.start.min(text.len())..r.end.min(text.len() + 1);
        let sc = sel_color(t, c.under);
        for (x, y, w, h) in l.rects(text, r, m) {
            paint::fill(bx + x, by + y, w, h, 0, sc);
        }
    }
    // Text.
    let base_color = match &b.kind {
        _ if c.color.is_some() => c.color.unwrap_or(fg),
        BlockKind::Heading(_) => p.bright_foreground,
        _ if b.quote > 0 => fg.darker(1.25),
        _ => fg,
    };
    let runs = b.content.runs();
    let first_line = if l.table.is_some() {
        0
    } else {
        l.lines.partition_point(|ln| by + ln.y + ln.h < c.clip.0)
    };
    for ln in &l.lines[first_line..] {
        let ly = by + ln.y;
        if ly > c.clip.1 {
            if l.table.is_some() {
                continue;
            }
            break;
        }
        if ly + ln.h < c.clip.0 {
            continue;
        }
        let base_y = ly + ln.base;
        for f in l.frags_of(ln) {
            if f.tab {
                continue;
            }
            let Some(r) = runs.get(f.run as usize) else {
                continue;
            };
            let st = &r.style;
            let x = bx + ln.x + f.x.round() as i32;
            let fw = f.w.ceil() as i32;
            if st.code && !b.is_code() {
                paint::fill(x - 2, ly + 1, fw + 4, ln.h - 2, 3, surface);
            }
            if st.chip {
                // The Composer's chip: accent-tinted pill, no underline.
                paint::fill(x - 4, ly + 1, fw + 8, ln.h - 2, (ln.h - 2) / 2, p.accent.over(c.under, 0.16));
            }
            let link = st.link.is_some() && !st.chip;
            let color = match st.color {
                Some(role) => role.rgb(t),
                None if link => p.accent,
                None => base_color,
            };
            FltkMetrics::set(f.font);
            draw::set_draw_color(color.fl());
            draw::draw_text_n(&text[f.start as usize..f.end as usize], x, base_y);
            if link || st.strike {
                draw::set_draw_color(color.fl());
                if link {
                    draw::draw_rectf(x, base_y + 2, fw, 1);
                }
                if st.strike {
                    draw::draw_rectf(x, base_y - (f.font.size * 3 / 10).max(2), fw, 1);
                }
            }
            let target = Target::Link {
                block: 0,
                run: f.run as usize,
            };
            let is = |o: Option<Target>| {
                o.is_some_and(|o| matches!((o, target), (Target::Link { run: a, .. }, Target::Link { run: b, .. }) if a == b))
            };
            if link && is(c.focus) {
                paint::stroke(x - 2, ly, fw + 4, ln.h, 3, 1, p.accent);
            } else if link && is(c.hover) {
                draw::set_draw_color(color.fl());
                draw::draw_rectf(x, base_y + 3, fw, 1);
            }
        }
    }
}

/// Nerd Font "content copy".
const COPY: &str = "\u{f018f}";
/// Nerd Font "image".
const IMAGE: &str = "\u{f02e9}";

// --------------------------------------------------------------- events

fn handle(w: &mut Widget, inn: &Rc<RefCell<Inner>>, hooks: &Hooks, ev: Event) -> bool {
    let mut m = FltkMetrics::default();
    let (wx, wy) = (w.x(), w.y());
    match ev {
        Event::Focus => {
            inn.borrow_mut().focus = None;
            w.redraw();
            true
        }
        Event::Unfocus => {
            inn.borrow_mut().focus = None;
            w.redraw();
            true
        }
        Event::Enter => true,
        Event::Move => {
            let (ex, ey) = (app::event_x(), app::event_y());
            let mut i = inn.borrow_mut();
            let over_bar = ex >= wx + w.w() - BAR - 4;
            let h = if over_bar {
                None
            } else {
                i.target_at(wx, wy, ex, ey, &mut m)
            };
            if h != i.hover {
                i.hover = h;
                drop(i);
                set_cursor(
                    w,
                    if h.is_some() {
                        Cursor::Hand
                    } else {
                        Cursor::Default
                    },
                );
                w.redraw();
            }
            true
        }
        Event::Leave => {
            let had = inn.borrow_mut().hover.take().is_some();
            set_cursor(w, Cursor::Default);
            if had {
                w.redraw();
            }
            true
        }
        Event::Push => {
            let _ = w.take_focus();
            let (ex, ey) = (app::event_x(), app::event_y());
            let mut i = inn.borrow_mut();
            let top = i.top;
            i.motion.jump(top);
            if ex >= wx + w.w() - BAR - 4 {
                i.thumb = Some((ey, top));
                return true;
            }
            if let Some(t @ (Target::Copy { .. } | Target::Action { .. } | Target::Embed { .. })) =
                i.target_at(wx, wy, ex, ey, &mut m)
            {
                let at = match t {
                    Target::Embed { block } => {
                        let l = i.laid(block).map_or(0, |(_, l)| l.left);
                        Some((ex - (wx + i.pad + l), ey - i.screen_y(block, wy)))
                    }
                    _ => None,
                };
                i.focus = None;
                i.activate(t, at);
                drop(i);
                redraw_soon(w);
                w.redraw();
                return true;
            }
            let target = i.target_at(wx, wy, ex, ey, &mut m);
            let link = target.and_then(|t| i.link_of(t));
            let Some(pos) = i.pos_at(wx, wy, ex, ey, &mut m) else {
                return true;
            };
            let clicks = app::event_clicks_num();
            let shift = app::event_state().contains(Shortcut::Shift);
            if shift && i.sel.is_some() {
                if let Some((a, _)) = i.sel {
                    i.sel = Some((a, pos));
                }
            } else if clicks == 1 {
                let t = i.store.text(pos.block);
                let (a, b) = word_at(t, pos.offset);
                i.sel = Some((
                    TextPos {
                        block: pos.block,
                        offset: a,
                    },
                    TextPos {
                        block: pos.block,
                        offset: b,
                    },
                ));
            } else if clicks >= 2 {
                let n = i.store.text(pos.block).len();
                i.sel = Some((
                    TextPos {
                        block: pos.block,
                        offset: 0,
                    },
                    TextPos {
                        block: pos.block,
                        offset: n,
                    },
                ));
            } else {
                i.sel = Some((pos, pos));
            }
            i.focus = None;
            i.selecting = true;
            i.press = Some(Press {
                x: ex,
                y: ey,
                link,
                moved: false,
            });
            drop(i);
            w.redraw();
            true
        }
        Event::Drag => {
            let (ex, ey) = (app::event_x(), app::event_y());
            let mut i = inn.borrow_mut();
            if let Some((y0, top0)) = i.thumb {
                let content = i.content_h() as f64;
                let max = i.max_top();
                i.set_top(top0 + ((ey - y0) as f64 * content / w.h().max(1) as f64) as i64);
                let top = i.top;
                i.stuck = i.stick && top >= max;
                drop(i);
                w.redraw();
                return true;
            }
            if !i.selecting {
                return true;
            }
            if let Some(p) = i.press.as_mut() {
                if (ex - p.x).abs() > 3 || (ey - p.y).abs() > 3 {
                    p.moved = true;
                    p.link = None;
                }
            }
            // Drag past an edge scrolls.
            if ey < wy {
                i.scroll_by((ey - wy) as i64);
            } else if ey > wy + w.h() {
                i.scroll_by((ey - wy - w.h()) as i64);
            }
            let cy = ey.clamp(wy, wy + w.h() - 1);
            if let Some(pos) = i.pos_at(wx, wy, ex, cy, &mut m) {
                if let Some((a, _)) = i.sel {
                    let moved = i.press.as_ref().is_some_and(|p| p.moved);
                    if moved {
                        i.sel = Some((a, pos));
                    }
                }
            }
            drop(i);
            w.redraw();
            true
        }
        Event::Released => {
            let mut i = inn.borrow_mut();
            i.thumb = None;
            i.selecting = false;
            let press = i.press.take();
            if i.sel_range().is_none() {
                i.sel = None;
            } else {
                let t = i.selected_text();
                if !t.is_empty() {
                    app::copy2(&t);
                }
            }
            drop(i);
            if let Some(url) = press.filter(|p| !p.moved).and_then(|p| p.link) {
                call(&hooks.link, |f| f(&url));
            }
            w.redraw();
            true
        }
        Event::MouseWheel => {
            let mut i = inn.borrow_mut();
            let max = i.max_top();
            let line = i.typo.line_h(i.typo.body) as f64;
            let delta = app::event_dy_value() as f64 * 3.0 * line;
            let top = i.top;
            // At an edge the wheel goes to the scroll area around the view.
            if i.motion.passes(top, delta, max) {
                return false;
            }
            i.top = i.motion.wheel(top, delta, max);
            i.anchor = i.locate(i.top);
            if delta < 0.0 {
                i.stuck = false;
            } else if i.stick && top as f64 + delta >= max as f64 - 1.0 {
                i.stuck = true;
            }
            drop(i);
            w.redraw();
            true
        }
        Event::KeyDown => key(w, inn, hooks),
        _ => false,
    }
}

fn set_cursor(w: &Widget, c: Cursor) {
    if let Some(mut win) = w.window() {
        win.set_cursor(c);
    }
}

/// Word (or run of blanks / one punctuation mark) around `at`.
fn word_at(t: &str, at: usize) -> (usize, usize) {
    let at = floor_char(t, at);
    for (i, w) in t.split_word_bound_indices() {
        if at >= i && at < i + w.len() {
            return (i, i + w.len());
        }
    }
    (at, at)
}

fn key(w: &mut Widget, inn: &Rc<RefCell<Inner>>, hooks: &Hooks) -> bool {
    let k = app::event_key();
    let state = app::event_state();
    let command = app::is_event_command();
    let alt = state.contains(Shortcut::Alt);
    let shift = state.contains(Shortcut::Shift);
    let mut i = inn.borrow_mut();
    {
        let top = i.top;
        i.motion.jump(top);
    }
    // Find: F3 / Shift+F3, Mod+G / Shift+Mod+G.
    if i.find.as_ref().is_some_and(|f| !f.matches.is_empty())
        && ((k == Key::F3 && !command && !alt) || (command && !alt && k == Key::from_char('g')))
    {
        i.find_step(if shift { -1 } else { 1 }, &mut FltkMetrics::default());
        drop(i);
        w.redraw();
        return true;
    }
    if command && !alt {
        if k == Key::from_char('c') {
            let text = match i.focus {
                Some(Target::Copy { block }) if i.sel_range().is_none() => {
                    i.store.text(block).to_string()
                }
                _ => i.selected_text(),
            };
            if text.is_empty() {
                return false;
            }
            drop(i);
            app::copy(&text);
            return true;
        }
        if k == Key::from_char('a') {
            i.select_all();
            drop(i);
            w.redraw();
            return true;
        }
        if k == Key::Home || k == Key::End {
            // Ctrl+Home / Ctrl+End as Home / End.
        } else {
            return false;
        }
    }
    let line = i.typo.line_h(i.typo.body) as i64;
    let page = (i.usable_h() as i64 - line).max(line);
    let used = match k {
        Key::Up => i.scroll_by(-line),
        Key::Down => i.scroll_by(line),
        Key::PageUp => i.scroll_by(-page),
        Key::PageDown => i.scroll_by(page),
        Key::Home => {
            let d = -i.top;
            i.scroll_by(d);
            true
        }
        Key::End => {
            let max = i.max_top();
            i.set_top(max);
            i.stuck = i.stick;
            true
        }
        Key::Tab if !alt && !command => {
            let targets = i.targets();
            let cur = i.focus.and_then(|f| targets.iter().position(|t| *t == f));
            let next = match (cur, shift) {
                (None, false) => targets.first().copied(),
                (None, true) => {
                    // Coming back into the view from below with Shift+Tab
                    // lands on the view itself; one more goes to the last link.
                    targets.last().copied()
                }
                (Some(c), false) => targets.get(c + 1).copied(),
                (Some(c), true) => c.checked_sub(1).and_then(|c| targets.get(c).copied()),
            };
            i.focus = next;
            if let Some(t) = next {
                reveal(&mut i, t, w);
            }
            let used = next.is_some();
            drop(i);
            w.redraw();
            return used;
        }
        Key::Enter | Key::KPEnter => {
            if i.focus.is_none() {
                return false;
            }
            drop(i);
            return activate_focus(w, inn, hooks);
        }
        Key::Escape => {
            let had = i.sel.take().is_some() | i.focus.take().is_some();
            drop(i);
            if had {
                w.redraw();
            }
            return had;
        }
        _ if k == Key::from_char(' ') && i.focus.is_some() => {
            drop(i);
            // Space activates like Enter.
            return activate_focus(w, inn, hooks);
        }
        _ => return false,
    };
    drop(i);
    if used {
        w.redraw();
    }
    used
}

fn activate_focus(w: &mut Widget, inn: &Rc<RefCell<Inner>>, hooks: &Hooks) -> bool {
    let url = {
        let mut i = inn.borrow_mut();
        let Some(t) = i.focus else { return false };
        i.activate(t, None)
    };
    if let Some(url) = url {
        call(&hooks.link, |f| f(&url));
    } else {
        redraw_soon(w);
    }
    w.redraw();
    true
}

/// Redraws after the "copied" check mark has had its moment.
fn redraw_soon(w: &Widget) {
    let mut ww = w.clone();
    app::add_timeout3(1.6, move |_| {
        if !ww.was_deleted() {
            ww.redraw()
        }
    });
}

/// Scrolls so the target's block line is fully in view.
fn reveal(i: &mut Inner, t: Target, w: &Widget) {
    let k = t.block();
    let Some((_, l)) = i.laid(k) else {
        return;
    };
    let (y, h) = match t {
        Target::Copy { .. } => (0, i.typo.code_header),
        Target::Action { .. } if l.table.is_some() => (l.height - i.typo.code_header, i.typo.code_header),
        Target::Action { .. } => (0, i.typo.code_header),
        Target::Embed { .. } => (0, l.height.min(i.usable_h())),
        Target::Link { run, .. } => l
            .lines
            .iter()
            .find(|ln| l.frags_of(ln).iter().any(|f| f.run as usize == run))
            .map_or((0, l.height), |ln| (ln.y, ln.h)),
    };
    let _ = w;
    let top_y = i.heights.prefix(k) + i.gap(k) as i64 + y as i64 + i.pad as i64;
    let view = i.usable_h() as i64;
    let mut top = i.top;
    if top_y < top {
        top = top_y - i.pad as i64;
    } else if top_y + h as i64 > top + view {
        top = top_y + h as i64 - view + i.pad as i64;
    }
    let max = i.max_top();
    i.set_top(top);
    i.stuck = i.stick && i.top >= max;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_pacing_holds_a_partial_word() {
        assert_eq!(super::held_word("Hello wor"), 3);
        assert_eq!(super::held_word("Hello world "), 0);
        assert_eq!(super::held_word("Hello world."), 0);
        assert_eq!(super::held_word("single"), 0, "no space: nothing to cut at");
        assert_eq!(super::held_word(&format!("a {}", "x".repeat(30))), 0, "long tokens pass");
        assert_eq!(super::held_word("día añ"), "añ".len());
    }

    #[test]
    fn heights_prefix_find_push_truncate() {
        let mut h = Heights::default();
        let v = [10i64, 0, 5, 20, 7, 3, 9, 1, 4];
        for x in v {
            h.push(x);
        }
        for i in 0..=v.len() {
            assert_eq!(h.prefix(i), v[..i].iter().sum::<i64>());
        }
        assert_eq!(h.find(0), 0);
        assert_eq!(h.find(9), 0);
        assert_eq!(h.find(10), 2);
        assert_eq!(h.find(15), 3);
        assert_eq!(h.find(1000), v.len() - 1);
        h.add(3, -15);
        assert_eq!(h.prefix(4), 20);
        h.truncate(5);
        h.push(100);
        assert_eq!(h.prefix(6), 20 + 7 + 100);
    }

    #[test]
    fn word_bounds() {
        assert_eq!(word_at("hello brave world", 8), (6, 11));
        assert_eq!(word_at("añob", 2), (0, 5));
    }
}
