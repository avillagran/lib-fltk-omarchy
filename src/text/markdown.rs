//! Markdown to [`Block`]s with `pulldown-cmark` (CommonMark plus the GFM
//! extensions: tables, task lists, strikethrough, alerts), bare-URL
//! autolinks, and a streaming parser for text that arrives in pieces.
//!
//! ## Streaming
//!
//! [`MarkdownStream::push`] appends text and re-parses only the *open
//! tail*: the source from the start of the last top-level block (or the
//! last item of a top-level list) that may still change. Everything before
//! it is committed: its blocks are final and its source is dropped. So a
//! push into a long message costs O(tail), not O(message).
//!
//! A boundary is committed when a later top-level block (or list item) has
//! started and its first line is complete: from then on, no text appended
//! can change the blocks before it in CommonMark. Ordered list numbering
//! continues across the boundary, and link reference definitions from the
//! committed part keep resolving in the tail.
//!
//! The streamed result equals a one-shot parse of the whole text, except
//! for a reference link (`[text][label]`) in an already committed block
//! whose definition comes later: it stays literal text.

use super::doc::*;
use pulldown_cmark::{
    Alignment, BlockQuoteKind, BrokenLink, CodeBlockKind, CowStr, Event, Options, Parser, Tag,
    TagEnd,
};
use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

/// Syntax highlighter for code blocks: `(language, code)` to colored
/// byte ranges (sorted, not overlapping).
pub type Highlight = Arc<dyn Fn(&str, &str) -> Vec<(Range<usize>, ColorRole)> + Send + Sync>;

/// Inline code to a link target (`None`: stays plain code). See
/// [`file_path_link`].
pub type CodeLink = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Conversion options.
#[derive(Clone)]
pub struct MarkdownOptions {
    /// Code block highlighter; the default is [`super::highlight::basic`]
    /// (diffs, comments, strings, numbers, keywords). `None`: plain code.
    pub highlight: Option<Highlight>,
    /// Image references become [`BlockKind::Image`] blocks (a paragraph
    /// with an image is split around it) that the view resolves through
    /// `RichText::on_image`. Off: an image is its alt text linked to its
    /// source, inline. Images in table cells and headings stay inline.
    pub images: bool,
    /// Inline code that this rule maps to a URL becomes a link (still
    /// drawn as code): `Some(Arc::new(file_path_link))` turns `src/lib.rs:12`
    /// into `file://src/lib.rs:12`, as T3 does.
    pub code_links: Option<CodeLink>,
    /// `$…$` and `$$…$$` are math, shown as text ([`super::math`]): Greek
    /// letters, operators, scripts, fractions (T3's `mathRenderingEnabled`).
    /// Off: dollars are plain text. Apps that also get `\(…\)` / `\[…\]`
    /// pass their Markdown through [`super::math::normalize_delimiters`].
    pub math: bool,
    /// While streaming, the open tail is shown as it is about to become
    /// (Rune's tail settling): an unclosed `**`, `*`, `` ` ``, `~~` or
    /// `[text](url` is drawn already closed, a trailing run of markers
    /// (`and **`) waits for the next character, and a `|` line waits until
    /// it is part of a table. Committed blocks and the final result
    /// ([`MarkdownStream::finish`]) are always the exact parse.
    pub settle_tail: bool,
    /// Links whose target this rule accepts are drawn as chips (mentions:
    /// `[@notes.md](file:notes.md)`), still activatable with `on_link`.
    pub chip_links: Option<ChipLink>,
    /// An ordered item (`4. `) on the line after paragraph text continues a
    /// list, as models write lists (CommonMark only lets `1.` interrupt a
    /// paragraph). Rune's rule; applied the same when streaming.
    pub model_lists: bool,
}

/// Rule for [`MarkdownOptions::chip_links`].
pub type ChipLink = Arc<dyn Fn(&str) -> bool + Send + Sync>;

impl Default for MarkdownOptions {
    fn default() -> Self {
        MarkdownOptions {
            highlight: Some(Arc::new(super::highlight::basic)),
            images: false,
            code_links: None,
            math: false,
            settle_tail: false,
            chip_links: None,
            model_lists: false,
        }
    }
}

impl MarkdownOptions {
    /// No syntax highlighting.
    pub fn plain() -> Self {
        MarkdownOptions {
            highlight: None,
            ..Default::default()
        }
    }

    /// Image references as image blocks (see [`MarkdownOptions::images`]).
    pub fn with_images(mut self, on: bool) -> Self {
        self.images = on;
        self
    }

    /// Inline code that looks like a file path links to `file://…`
    /// ([`file_path_link`]).
    pub fn with_path_links(mut self) -> Self {
        self.code_links = Some(Arc::new(file_path_link));
        self
    }

    /// See [`MarkdownOptions::model_lists`].
    pub fn with_model_lists(mut self, on: bool) -> Self {
        self.model_lists = on;
        self
    }

    /// Links accepted by `f` are drawn as chips (see [`MarkdownOptions::chip_links`]).
    pub fn with_chip_links(mut self, f: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        self.chip_links = Some(Arc::new(f));
        self
    }

    /// Streaming tail settling (see [`MarkdownOptions::settle_tail`]).
    pub fn with_settle_tail(mut self, on: bool) -> Self {
        self.settle_tail = on;
        self
    }

    /// `$…$` / `$$…$$` as math (see [`MarkdownOptions::math`]).
    pub fn with_math(mut self, on: bool) -> Self {
        self.math = on;
        self
    }

    /// Inline code mapped to links by `f`.
    pub fn with_code_links(mut self, f: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        self.code_links = Some(Arc::new(f));
        self
    }
}

impl fmt::Debug for MarkdownOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarkdownOptions")
            .field("highlight", &self.highlight.is_some())
            .field("images", &self.images)
            .field("code_links", &self.code_links.is_some())
            .field("math", &self.math)
            .field("settle_tail", &self.settle_tail)
            .field("chip_links", &self.chip_links.is_some())
            .field("model_lists", &self.model_lists)
            .finish()
    }
}

/// T3's rule for inline code that names a file or folder: absolute,
/// `~/`, `./` and `../` paths, `dir/file.ext`, and `file.ext:line[:col]`;
/// not URLs, commands with spaces, versions (`v1.2/3`) or host names
/// (`github.com/x/y`). Returns `file://` + the path (`\` as `/`, bytes
/// outside `A-Za-z0-9/._-~:` percent-encoded).
pub fn file_path_link(code: &str) -> Option<String> {
    let t = code.trim();
    if t.is_empty()
        || t.len() > 4096
        || t.chars().any(|c| c.is_whitespace() || c == '`' || c.is_control())
        || t.contains("://")
    {
        return None;
    }
    let c = t.replace('\\', "/");
    let digits = |n: &str| !n.is_empty() && n.bytes().all(|d| d.is_ascii_digit());
    let (body, position) = match c.rsplit_once(':') {
        Some((b, n)) if !b.is_empty() && digits(n) => {
            let b = b.rsplit_once(':').filter(|(x, n)| !x.is_empty() && digits(n)).map_or(b, |(x, _)| x);
            (b.to_string(), true)
        }
        _ => (c.clone(), false),
    };
    let href = |p: &str| -> String {
        let mut out = String::from("file://");
        for b in p.bytes() {
            if b.is_ascii_alphanumeric() || b"/._-~:".contains(&b) {
                out.push(b as char);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    };
    if !position && !body.contains('/') {
        return None;
    }
    if body.starts_with('/') || body.starts_with("~/") || body.starts_with("./") || body.starts_with("../") {
        return Some(href(&c));
    }
    if body.contains(':') {
        return None;
    }
    let first = body.split('/').next().unwrap_or("");
    let host = first.contains('.')
        && !first.starts_with('.')
        && body.contains('/')
        && first
            .rsplit('.')
            .next()
            .is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|ch| ch.is_ascii_alphabetic()));
    if host {
        return None;
    }
    let base = body.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let ext = base.rsplit_once('.').is_some_and(|(n, e)| {
        !n.is_empty() && !e.is_empty() && e.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    });
    let version = base
        .as_bytes()
        .windows(3)
        .any(|w| w[0].is_ascii_digit() && w[1] == b'.' && w[2].is_ascii_digit());
    if (version && !position) || (!position && !ext) {
        return None;
    }
    Some(href(&c))
}

fn cmark_options(math: bool) -> Options {
    let o = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM;
    if math { o | Options::ENABLE_MATH } else { o }
}

/// One-shot parse with the default options.
pub fn parse(md: &str) -> Vec<Block> {
    parse_with(md, &MarkdownOptions::default())
}

pub fn parse_with(md: &str, opts: &MarkdownOptions) -> Vec<Block> {
    let mut s = MarkdownStream::with_options(opts.clone());
    let mut out = Vec::new();
    s.push_into(&mut out, md);
    out
}

/// Inline Markdown of one paragraph (`**bold**`, `` `code` ``, links) as
/// one [`Inline`]; block syntax is flattened into it. For labels and toasts.
pub fn parse_inline(md: &str) -> Inline {
    let blocks = parse_with(md, &MarkdownOptions::plain());
    let mut out = Inline::new();
    for (i, b) in blocks.iter().enumerate() {
        if i > 0 {
            out.push(" ", &Style::plain());
        }
        for r in b.content.runs() {
            out.push(&b.content.text()[r.range.clone()], &r.style);
        }
    }
    out
}

// --------------------------------------------------------------- streaming

/// Incremental Markdown parser (see the module docs).
///
/// ```
/// use fltk_omarchy::text::{markdown::MarkdownStream, Document};
/// let mut s = MarkdownStream::new();
/// let mut blocks = Vec::new();
/// for piece in ["# Ti", "tle\n\nSome **bo", "ld** text\n\n- a\n- b\n"] {
///     let u = s.push(piece);
///     blocks.truncate(u.keep);       // the first `keep` blocks are final
///     blocks.extend(u.blocks);       // the rest is replaced
/// }
/// assert_eq!(blocks, Document::from_markdown("# Title\n\nSome **bold** text\n\n- a\n- b\n").blocks);
/// ```
#[derive(Debug, Default)]
pub struct MarkdownStream {
    tail: String,
    committed: usize,
    defs: HashMap<String, (String, String)>,
    cont: Option<u64>,
    opts: MarkdownOptions,
    total: usize,
}

/// Result of a push: the stream's blocks are its first `keep` blocks
/// (unchanged since the last push) followed by `blocks`.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamUpdate {
    pub keep: usize,
    pub blocks: Vec<Block>,
}

impl MarkdownStream {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_options(opts: MarkdownOptions) -> Self {
        MarkdownStream {
            opts,
            ..Default::default()
        }
    }

    /// Appends `more` and re-parses the open tail.
    pub fn push(&mut self, more: &str) -> StreamUpdate {
        self.tail.push_str(more);
        if self.opts.model_lists {
            if let Some(t) = loosen_lists(&self.tail) {
                self.tail = t;
            }
        }
        self.total += more.len();
        let parsed = convert(&self.tail, &self.defs, self.cont, &self.opts);
        let keep = self.committed;
        let mut committed_blocks = 0;
        // Commit up to the last boundary whose first line is complete.
        let cut = parsed
            .units
            .iter()
            .skip(1)
            .rev()
            .find(|u| u.offset.is_some_and(|p| self.tail[p..].contains('\n')));
        if let Some(u) = cut {
            let p = u.offset.unwrap_or(0);
            for (key, url, title, end) in &parsed.defs {
                if *end <= p {
                    self.defs
                        .entry(key.clone())
                        .or_insert_with(|| (url.clone(), title.clone()));
                }
            }
            self.committed += u.block;
            committed_blocks = u.block;
            self.cont = u.number;
            self.tail.drain(..p);
        }
        if self.opts.settle_tail {
            // The committed blocks are exact; the open tail is shown settled.
            let settled = settle(&self.tail);
            let mut blocks: Vec<Block> = parsed.blocks.into_iter().take(committed_blocks).collect();
            blocks.extend(convert(&settled, &self.defs, self.cont, &self.opts).blocks);
            return StreamUpdate { keep, blocks };
        }
        StreamUpdate {
            keep,
            blocks: parsed.blocks,
        }
    }

    /// The exact blocks of the open tail, for when the text is complete
    /// (matters with [`MarkdownOptions::settle_tail`]): replace the stream's
    /// blocks from `keep` on with `blocks`.
    pub fn finish(&mut self) -> StreamUpdate {
        StreamUpdate {
            keep: self.committed,
            blocks: convert(&self.tail, &self.defs, self.cont, &self.opts).blocks,
        }
    }

    /// [`push`](Self::push) applied to a list that holds exactly this
    /// stream's blocks.
    pub fn push_into(&mut self, blocks: &mut Vec<Block>, more: &str) {
        let u = self.push(more);
        blocks.truncate(u.keep);
        blocks.extend(u.blocks);
    }

    /// True when the open tail is shown settled ([`MarkdownOptions::settle_tail`]).
    pub fn settles(&self) -> bool {
        self.opts.settle_tail
    }

    /// Blocks that are final (never replaced by later pushes).
    pub fn committed(&self) -> usize {
        self.committed
    }

    /// The source still re-parsed on every push.
    pub fn tail(&self) -> &str {
        &self.tail
    }

    /// Bytes pushed so far.
    pub fn len(&self) -> usize {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

/// An ordered list line (`  12. ` / `3) `).
fn is_ordered_item(line: &str) -> bool {
    let t = line.strip_prefix("   ").or_else(|| line.strip_prefix("  ")).or_else(|| line.strip_prefix(' ')).unwrap_or(line);
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    (1..=9).contains(&digits) && matches!(t.as_bytes().get(digits), Some(b'.' | b')')) && matches!(t.as_bytes().get(digits + 1), Some(b' ' | b'\t'))
}

fn is_list_or_special(line: &str) -> bool {
    let t = line.trim_start();
    t.is_empty() || is_ordered_item(line) || t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") || t.starts_with('|') || t.starts_with('#') || t.starts_with('>')
}

/// Inserts a blank line before an ordered item that follows paragraph text
/// (see [`MarkdownOptions::model_lists`]); None when nothing changes.
/// Idempotent; code fences are left alone.
pub fn loosen_lists(src: &str) -> Option<String> {
    let mut out = String::with_capacity(src.len() + 8);
    let mut changed = false;
    let mut fence = false;
    let mut prev: Option<&str> = None;
    // Inside a list (since its first item, until a blank line) a following
    // item is already part of it.
    let mut in_list = false;
    for line in src.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
        }
        let item = !fence && is_ordered_item(line);
        if item && !in_list {
            if let Some(p) = prev {
                if !is_list_or_special(p.trim_end_matches(['\n', '\r'])) {
                    out.push('\n');
                    changed = true;
                }
            }
        }
        if t.trim().is_empty() {
            in_list = false;
        } else if item || t.starts_with("- ") || t.starts_with("* ") || t.starts_with("+ ") {
            in_list = true;
        }
        out.push_str(line);
        prev = Some(line);
    }
    changed.then_some(out)
}

/// The open tail as it is about to become (see
/// [`MarkdownOptions::settle_tail`]). Pure; the source is not changed.
pub fn settle(tail: &str) -> String {
    // Inside an open code fence nothing is inline Markdown.
    let fences = tail.lines().filter(|l| {
        let t = l.trim_start();
        t.starts_with("```") || t.starts_with("~~~")
    });
    if fences.count() % 2 == 1 {
        return tail.to_string();
    }
    let mut out = tail.to_string();
    // A `|` line that is not yet part of a table waits.
    if let Some(start) = out.rfind('\n').map(|i| i + 1).or(Some(0)) {
        let last = &out[start..];
        if last.trim_start().starts_with('|') {
            let prev = out[..start].trim_end_matches('\n').rsplit('\n').next().unwrap_or("");
            if !prev.trim_start().starts_with('|') {
                out.truncate(start);
            }
        }
    }
    // A trailing run of markers waits for the next character.
    let trimmed = out.trim_end_matches(['*', '_', '~', '`', '[']);
    if trimmed.len() < out.len() && trimmed.ends_with(|c: char| c.is_whitespace() || c == '(') {
        out.truncate(trimmed.len());
    }
    // Close what the paragraph being written left open.
    let para_start = out.rfind("\n\n").map_or(0, |i| i + 2);
    let para = &out[para_start..];
    let mut closers = String::new();
    // Inline code first: inside it nothing else counts.
    let ticks = para.matches('`').count();
    let outside: String = if ticks % 2 == 1 {
        closers.push('`');
        para[..para.rfind('`').unwrap_or(0)].to_string()
    } else {
        para.to_string()
    };
    let outside = strip_code_spans(&outside);
    // An unclosed link destination: `[text](http…`.
    if let Some(open) = outside.rfind("](") {
        if !outside[open..].contains(')') {
            closers.push(')');
        }
    }
    if outside.matches("~~").count() % 2 == 1 {
        closers.push_str("~~");
    }
    let bold = outside.matches("**").count();
    if bold % 2 == 1 {
        closers.push_str("**");
    }
    // Single `*` emphasis, not bullets at the start of a line.
    let singles = outside
        .lines()
        .map(|l| {
            let l = l.trim_start();
            let l = l.strip_prefix("* ").unwrap_or(l);
            l.replace("**", "").matches('*').count()
        })
        .sum::<usize>();
    if singles % 2 == 1 {
        closers.push('*');
    }
    if !closers.is_empty() {
        out.push_str(&closers);
    }
    out
}

/// `s` without its closed inline code spans (their content is literal).
fn strip_code_spans(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_code = false;
    for c in s.chars() {
        if c == '`' {
            in_code = !in_code;
            continue;
        }
        if !in_code {
            out.push(c);
        }
    }
    out
}

// --------------------------------------------------------------- converter

/// A place where the source can be cut: a top-level block, or an item of
/// a top-level list.
struct Unit {
    /// Line start of the unit in the parsed text; `None` when the unit does
    /// not start a line (never a cut).
    offset: Option<usize>,
    /// Blocks emitted before it.
    block: usize,
    /// Ordered-list number of the item that starts it.
    number: Option<u64>,
}

struct Parsed {
    blocks: Vec<Block>,
    units: Vec<Unit>,
    defs: Vec<(String, String, String, usize)>,
}

fn label_key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn convert(
    src: &str,
    defs: &HashMap<String, (String, String)>,
    cont: Option<u64>,
    opts: &MarkdownOptions,
) -> Parsed {
    let callback = |link: BrokenLink<'_>| {
        defs.get(&label_key(&link.reference))
            .map(|(u, t)| (CowStr::from(u.clone()), CowStr::from(t.clone())))
    };
    let mut it = Parser::new_with_broken_link_callback(src, cmark_options(opts.math), Some(callback))
        .into_offset_iter();
    let mut c = Conv::new(src, cont, opts);
    for (ev, range) in &mut it {
        c.event(ev, range);
    }
    c.close();
    let defs = it
        .reference_definitions()
        .iter()
        .map(|(k, d)| {
            (
                label_key(k),
                d.dest.to_string(),
                d.title.as_deref().unwrap_or("").to_string(),
                d.span.end,
            )
        })
        .collect();
    Parsed {
        blocks: c.blocks,
        units: c.units,
        defs,
    }
}

struct TableState {
    align: Vec<ColumnAlign>,
    rows: Vec<Vec<Range<usize>>>,
    row: Vec<Range<usize>>,
    cell: usize,
    header_rows: usize,
}

struct Conv<'s> {
    src: &'s str,
    opts: &'s MarkdownOptions,
    blocks: Vec<Block>,
    units: Vec<Unit>,
    cur: Option<Block>,
    quote: u8,
    tones: Vec<Option<ColorRole>>,
    lists: Vec<Option<u64>>,
    top_items: usize,
    pending: Option<ListMarker>,
    strong: u32,
    emph: u32,
    strike: u32,
    links: Vec<Arc<str>>,
    images: Vec<usize>,
    /// Inside an image that became its own block.
    image_block: u32,
    /// The text after an image block starts a paragraph without blanks.
    trim_next: bool,
    depth: u32,
    table: Option<TableState>,
    cont: Option<u64>,
}

impl<'s> Conv<'s> {
    fn new(src: &'s str, cont: Option<u64>, opts: &'s MarkdownOptions) -> Self {
        Conv {
            src,
            opts,
            blocks: Vec::new(),
            units: Vec::new(),
            cur: None,
            quote: 0,
            tones: Vec::new(),
            lists: Vec::new(),
            top_items: 0,
            pending: None,
            strong: 0,
            emph: 0,
            strike: 0,
            links: Vec::new(),
            images: Vec::new(),
            image_block: 0,
            trim_next: false,
            depth: 0,
            table: None,
            cont,
        }
    }

    fn style(&self) -> Style {
        Style {
            bold: self.strong > 0,
            italic: self.emph > 0,
            strike: self.strike > 0,
            code: false,
            link: self.links.last().cloned(),
            color: None,
            chip: self.links.last().is_some_and(|u| self.opts.chip_links.as_ref().is_some_and(|f| f(u))),
        }
    }

    /// Line start of `off` when only blanks precede it on its line.
    fn snap(&self, off: usize) -> Option<usize> {
        let ls = self.src[..off].rfind('\n').map_or(0, |i| i + 1);
        self.src[ls..off]
            .bytes()
            .all(|b| b == b' ' || b == b'\t')
            .then_some(ls)
    }

    fn unit(&mut self, off: usize, number: Option<u64>) {
        self.close();
        let offset = self.snap(off);
        self.units.push(Unit {
            offset,
            block: self.blocks.len(),
            number,
        });
    }

    /// A block-level tag starts.
    fn enter(&mut self, off: usize) {
        if self.depth == 0 {
            self.unit(off, None);
        }
        self.depth += 1;
    }

    fn open(&mut self, kind: BlockKind) {
        self.close();
        self.cur = Some(Block {
            kind,
            content: Inline::new(),
            indent: self.lists.len().min(255) as u8,
            quote: self.quote,
            quote_tone: self.tones.last().copied().flatten(),
            marker: self.pending.take(),
        });
    }

    fn close(&mut self) {
        let Some(mut b) = self.cur.take() else { return };
        match &b.kind {
            BlockKind::Code { lang } => {
                let t = b.content.text();
                if t.ends_with('\n') {
                    b.content.truncate(t.len() - 1);
                }
                if let Some(h) = &self.opts.highlight {
                    let spans = h(lang, b.content.text());
                    if !spans.is_empty() {
                        color_runs(&mut b.content, &spans);
                    }
                }
            }
            BlockKind::Rule => {}
            _ => {
                let t = b.content.text();
                let trimmed = t.trim_end_matches('\n').len();
                if trimmed < t.len() {
                    b.content.truncate(trimmed);
                }
                linkify(&mut b.content);
            }
        }
        if b.content.is_empty() && b.marker.is_none() && matches!(b.kind, BlockKind::Paragraph) {
            return;
        }
        b.content.shrink();
        self.blocks.push(b);
    }

    /// An item that has no text of its own still shows its marker.
    fn flush_marker(&mut self) {
        if self.pending.is_some() {
            self.open(BlockKind::Paragraph);
            self.close();
        }
    }

    fn text(&mut self, mut s: &str, style: &Style) {
        if self.trim_next && self.cur.is_none() {
            s = s.trim_start();
            if s.is_empty() {
                return;
            }
        }
        self.trim_next = false;
        if self.cur.is_none() {
            self.open(BlockKind::Paragraph);
        }
        if let Some(b) = self.cur.as_mut() {
            b.content.push(s, style);
        }
    }

    fn content_len(&self) -> usize {
        self.cur.as_ref().map_or(0, |b| b.content.len())
    }

    fn event(&mut self, ev: Event<'_>, range: Range<usize>) {
        match ev {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                let style = if self.cur.as_ref().is_some_and(|b| b.is_code()) {
                    Style::plain()
                } else {
                    self.style()
                };
                self.text(&t, &style);
            }
            Event::Code(t) => {
                let mut style = self.style().code();
                if style.link.is_none() {
                    if let Some(f) = &self.opts.code_links {
                        style.link = f(&t).map(Arc::from);
                    }
                }
                self.text(&t, &style);
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                let style = self.style().italic();
                self.text(&super::math::to_text(&t), &style);
            }
            Event::Html(t) | Event::InlineHtml(t) | Event::FootnoteReference(t) => {
                self.text(&t, &Style::plain());
            }
            Event::SoftBreak => {
                let style = self.style();
                self.text(" ", &style);
            }
            Event::HardBreak => self.text("\n", &Style::plain()),
            Event::Rule => {
                if self.depth == 0 {
                    self.unit(range.start, None);
                }
                self.open(BlockKind::Rule);
                self.close();
            }
            Event::TaskListMarker(done) => {
                if self.pending.is_some() {
                    self.pending = Some(ListMarker::Task(done));
                } else if let Some(b) = self
                    .cur
                    .as_mut()
                    .filter(|b| b.content.is_empty() && b.marker.is_some())
                {
                    b.marker = Some(ListMarker::Task(done));
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) {
        match tag {
            Tag::Paragraph => {
                self.enter(range.start);
                self.open(BlockKind::Paragraph);
            }
            Tag::Heading { level, .. } => {
                self.enter(range.start);
                self.open(BlockKind::Heading(level as u8));
            }
            Tag::BlockQuote(kind) => {
                self.enter(range.start);
                self.close();
                self.flush_marker();
                self.quote = self.quote.saturating_add(1);
                self.tones.push(kind.map(|k| match k {
                    BlockQuoteKind::Note => ColorRole::Blue,
                    BlockQuoteKind::Tip => ColorRole::Green,
                    BlockQuoteKind::Important => ColorRole::Magenta,
                    BlockQuoteKind::Warning => ColorRole::Yellow,
                    BlockQuoteKind::Caution => ColorRole::Red,
                }));
            }
            Tag::CodeBlock(kind) => {
                self.enter(range.start);
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split(|c: char| c.is_whitespace() || c == ',' || c == '{' || c == '}')
                        .find(|s| !s.is_empty())
                        .unwrap_or("")
                        .to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.open(BlockKind::Code { lang });
            }
            Tag::HtmlBlock => {
                self.enter(range.start);
                self.open(BlockKind::Paragraph);
            }
            Tag::List(start) => {
                self.enter(range.start);
                self.close();
                self.flush_marker();
                let first = if self.lists.is_empty()
                    && self.cont.is_some()
                    && start.is_some()
                    && self.src[..range.start].trim().is_empty()
                {
                    self.cont
                } else {
                    start
                };
                if self.lists.is_empty() {
                    self.top_items = 0;
                }
                self.lists.push(first);
            }
            Tag::Item => {
                let number = self.lists.last().copied().flatten();
                if self.depth == 1 && self.lists.len() == 1 {
                    self.top_items += 1;
                    if self.top_items > 1 {
                        self.unit(range.start, number);
                    }
                }
                self.depth += 1;
                self.close();
                self.flush_marker();
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = ListMarker::Ordered(*n);
                        *n += 1;
                        m
                    }
                    _ => ListMarker::Bullet,
                };
                self.pending = Some(marker);
            }
            Tag::Table(align) => {
                self.enter(range.start);
                self.open(BlockKind::Table(Box::default()));
                let align = align
                    .iter()
                    .map(|a| match a {
                        Alignment::Center => ColumnAlign::Center,
                        Alignment::Right => ColumnAlign::Right,
                        _ => ColumnAlign::Left,
                    })
                    .collect();
                self.table = Some(TableState {
                    align,
                    rows: Vec::new(),
                    row: Vec::new(),
                    cell: 0,
                    header_rows: 0,
                });
            }
            Tag::TableHead | Tag::TableRow => {
                self.depth += 1;
                if self.table.as_ref().is_some_and(|t| !t.rows.is_empty()) {
                    self.text("\n", &Style::plain());
                }
            }
            Tag::TableCell => {
                self.depth += 1;
                if self.table.as_ref().is_some_and(|t| !t.row.is_empty()) {
                    self.text("\t", &Style::plain());
                }
                let at = self.content_len();
                if let Some(t) = self.table.as_mut() {
                    t.cell = at;
                }
            }
            Tag::Emphasis => self.emph += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.links.push(Arc::from(dest_url.as_ref())),
            Tag::Image { dest_url, .. }
                if self.opts.images
                    && self.image_block == 0
                    && self.table.is_none()
                    && self.cur.as_ref().map_or(true, |b| b.kind == BlockKind::Paragraph) =>
            {
                // The paragraph so far ends here; an empty one hands its
                // list marker to the image.
                let marker = match self.cur.as_mut() {
                    Some(b) if b.content.is_empty() => b.marker.take(),
                    _ => None,
                };
                if let Some(b) = self.cur.as_mut() {
                    let t = b.content.text();
                    let trimmed = t.trim_end().len();
                    if trimmed < t.len() {
                        b.content.truncate(trimmed);
                    }
                }
                self.close();
                self.pending = marker.or(self.pending.take());
                self.open(BlockKind::Image {
                    src: dest_url.to_string(),
                });
                self.links.push(Arc::from(dest_url.as_ref()));
                self.images.push(0);
                self.image_block += 1;
            }
            Tag::Image { dest_url, .. } => {
                if self.cur.is_none() {
                    self.open(BlockKind::Paragraph);
                }
                self.links.push(Arc::from(dest_url.as_ref()));
                let at = self.content_len();
                self.images.push(at);
            }
            Tag::Superscript | Tag::Subscript => {}
            Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::MetadataBlock(_) => {
                self.enter(range.start);
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::HtmlBlock => {
                self.close();
                self.depth -= 1;
            }
            TagEnd::BlockQuote(_) => {
                self.close();
                self.quote = self.quote.saturating_sub(1);
                self.tones.pop();
                self.depth -= 1;
            }
            TagEnd::List(_) => {
                self.close();
                self.lists.pop();
                self.depth -= 1;
            }
            TagEnd::Item => {
                self.close();
                self.flush_marker();
                self.depth -= 1;
            }
            TagEnd::Table => {
                if let (Some(t), Some(b)) = (self.table.take(), self.cur.as_mut()) {
                    b.kind = BlockKind::Table(Box::new(Table {
                        align: t.align,
                        header_rows: t.header_rows,
                        rows: t.rows,
                    }));
                }
                self.close();
                self.depth -= 1;
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                    if tag == TagEnd::TableHead {
                        t.header_rows += 1;
                    }
                }
                self.depth -= 1;
            }
            TagEnd::TableCell => {
                let at = self.content_len();
                if let Some(t) = self.table.as_mut() {
                    t.row.push(t.cell..at);
                }
                self.depth -= 1;
            }
            TagEnd::Emphasis => self.emph = self.emph.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.links.pop();
            }
            TagEnd::Image
                if self.image_block > 0
                    && self.cur.as_ref().is_some_and(|b| matches!(b.kind, BlockKind::Image { .. })) =>
            {
                self.image_block -= 1;
                self.images.pop();
                if self.content_len() == 0 {
                    if let Some(url) = self.links.last().cloned() {
                        let style = self.style();
                        self.text(&url, &style);
                    }
                }
                self.links.pop();
                self.close();
                self.trim_next = true;
            }
            TagEnd::Image => {
                let start = self.images.pop().unwrap_or(0);
                if self.content_len() == start {
                    if let Some(url) = self.links.last().cloned() {
                        let style = self.style();
                        self.text(&url, &style);
                    }
                }
                self.links.pop();
            }
            TagEnd::Superscript | TagEnd::Subscript => {}
            TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::MetadataBlock(_) => {
                self.close();
                self.depth -= 1;
            }
        }
    }
}

/// Applies highlighter colors over the (plain) runs of a code block.
fn color_runs(content: &mut Inline, spans: &[(Range<usize>, ColorRole)]) {
    let len = content.len();
    let text = content.text();
    let mut runs: Vec<Run> = Vec::with_capacity(spans.len() * 2 + 1);
    let mut push = |range: Range<usize>, color: Option<ColorRole>| {
        if range.is_empty() {
            return;
        }
        let style = Style {
            color,
            ..Style::plain()
        };
        match runs.last_mut() {
            Some(r) if r.style == style && r.range.end == range.start => r.range.end = range.end,
            _ => runs.push(Run { range, style }),
        }
    };
    let mut at = 0;
    for (r, c) in spans {
        let (s, e) = (r.start.clamp(at, len), r.end.min(len));
        if s >= e || !text.is_char_boundary(s) || !text.is_char_boundary(e) {
            continue;
        }
        push(at..s, None);
        push(s..e, Some(*c));
        at = e;
    }
    push(at..len, None);
    content.set_runs(runs);
}

/// Bare URLs (`https://…`, `http://…`, `www.…`) in plain runs become links,
/// with GFM's rules for trailing punctuation.
fn linkify(content: &mut Inline) {
    let text = content.text();
    if !(text.contains("http") || text.contains("www.")) {
        return;
    }
    let mut out: Vec<Run> = Vec::new();
    let mut changed = false;
    for r in content.runs() {
        if r.style.link.is_some() || r.style.code {
            out.push(r.clone());
            continue;
        }
        let s = &text[r.range.clone()];
        let mut last = 0;
        for (span, url) in find_urls(s) {
            changed = true;
            if span.start > last {
                out.push(Run {
                    range: r.range.start + last..r.range.start + span.start,
                    style: r.style.clone(),
                });
            }
            let mut style = r.style.clone();
            style.link = Some(Arc::from(url.as_str()));
            out.push(Run {
                range: r.range.start + span.start..r.range.start + span.end,
                style,
            });
            last = span.end;
        }
        if last < s.len() {
            out.push(Run {
                range: r.range.start + last..r.range.end,
                style: r.style.clone(),
            });
        }
    }
    if changed {
        content.set_runs(out);
    }
}

pub(crate) fn find_urls(s: &str) -> Vec<(Range<usize>, String)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let rest = &s[i..];
        let scheme = if rest.starts_with("https://") {
            8
        } else if rest.starts_with("http://") {
            7
        } else if rest.starts_with("www.") {
            4
        } else {
            i += s[i..].chars().next().map_or(1, char::len_utf8);
            continue;
        };
        let ok_before = i == 0
            || s[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || "(*_~\"'".contains(c));
        let mut end = i + rest
            .find(|c: char| c.is_whitespace() || c == '<')
            .unwrap_or(rest.len());
        // Trailing punctuation is not part of the link; ")" only when unbalanced.
        while let Some(c) = s[i..end].chars().next_back() {
            let unbalanced =
                c == ')' && s[i..end].matches('(').count() < s[i..end].matches(')').count();
            if "?!.,:;*_~\"'".contains(c) || unbalanced {
                end -= c.len_utf8();
            } else {
                break;
            }
        }
        let body = &s[i + scheme..end];
        let valid = ok_before && !body.is_empty();
        if valid {
            let url = if scheme == 4 {
                format!("http://{}", &s[i..end])
            } else {
                s[i..end].to_string()
            };
            out.push((i..end, url));
            i = end;
        } else {
            i += scheme;
        }
    }
    out
}

#[cfg(test)]
mod tests {

    #[test]
    fn model_lists_continue_after_paragraph_text() {
        let src = "Steps:\n1. first\n\nThen more text\n2. second\n3. third";
        let plain = parse_with(src, &MarkdownOptions::plain());
        assert!(plain.iter().any(|b| b.content.text().contains("Then more text 2. second")), "CommonMark keeps it in the paragraph");
        let opts = MarkdownOptions::plain().with_model_lists(true);
        let b = parse_with(src, &opts);
        assert!(b.iter().any(|x| x.content.text() == "second" && x.marker.is_some()), "{:?}", b.iter().map(|x| x.content.text().to_string()).collect::<Vec<_>>());
        // Streamed == one-shot, cut anywhere.
        for cut in 1..src.len() {
            if !src.is_char_boundary(cut) { continue; }
            let mut s = MarkdownStream::with_options(opts.clone());
            let mut out = Vec::new();
            s.push_into(&mut out, &src[..cut]);
            s.push_into(&mut out, &src[cut..]);
            assert_eq!(out, b, "cut {cut}");
        }
        // Inside a list a continuation line is left alone; idempotent.
        assert_eq!(loosen_lists("1. a\ncontinued\n2. b"), None);
        let once = loosen_lists("text\n4. x").unwrap();
        assert_eq!(loosen_lists(&once), None);
        assert_eq!(loosen_lists("```\ncode\n4. x\n```"), None);
    }

    #[test]
    fn chip_links_are_chips() {
        let opts = MarkdownOptions::plain().with_chip_links(|u| u.starts_with("file:"));
        let b = parse_with("See [@notes.md](file:notes.md) and [site](https://x.dev).", &opts);
        let runs = b[0].content.runs();
        let chip = runs.iter().find(|r| r.style.chip).expect("a chip run");
        assert_eq!(&b[0].content.text()[chip.range.clone()], "@notes.md");
        assert_eq!(chip.style.link.as_deref(), Some("file:notes.md"));
        assert!(runs.iter().any(|r| r.style.link.as_deref() == Some("https://x.dev") && !r.style.chip));
    }

    #[test]
    fn settle_tail_closes_and_holds_back() {
        assert_eq!(settle("a **bold"), "a **bold**");
        assert_eq!(settle("an *it"), "an *it*");
        assert_eq!(settle("run `cargo"), "run `cargo`");
        assert_eq!(settle("see [docs](https://exa"), "see [docs](https://exa)");
        assert_eq!(settle("old ~~gone"), "old ~~gone~~");
        assert_eq!(settle("and **"), "and ");
        assert_eq!(settle("done **bold** and `x` ok"), "done **bold** and `x` ok");
        assert_eq!(settle("text\n| a"), "text\n");
        assert_eq!(settle("| a | b |\n|---|---|\n| 1"), "| a | b |\n|---|---|\n| 1");
        assert_eq!(settle("```rust\nlet x = **y"), "```rust\nlet x = **y");
        assert_eq!(settle("* item with **b"), "* item with **b**");
        assert_eq!(settle("`a*b` then *c"), "`a*b` then *c*");
    }

    #[test]
    fn settled_stream_shows_styles_and_finishes_exact() {
        let opts = MarkdownOptions::plain().with_settle_tail(true);
        let mut s = MarkdownStream::with_options(opts.clone());
        let mut blocks = Vec::new();
        s.push_into(&mut blocks, "Hello **wor");
        let runs = blocks.last().unwrap().content.runs();
        assert!(runs.iter().any(|r| r.style.bold), "the open bold is drawn bold: {:?}", blocks.last().unwrap().content.text());
        assert!(!blocks.last().unwrap().content.text().contains('*'));
        s.push_into(&mut blocks, "ld** done");
        let u = s.finish();
        blocks.truncate(u.keep);
        blocks.extend(u.blocks);
        assert_eq!(blocks, parse_with("Hello **world** done", &opts));
    }

    #[test]
    fn math_is_text_only_when_enabled() {
        let on = MarkdownOptions::plain().with_math(true);
        let b = parse_with("Energy $E = mc^2$ and $$\\sum_{i=1}^{n} i$$", &on);
        let text: String = b.iter().map(|x| x.content.text().to_string()).collect();
        assert!(text.contains("E = mc²") && text.contains("∑ᵢ₌₁ⁿ i"), "{text}");
        let off = parse_with("costs $5 and $6", &MarkdownOptions::plain());
        assert!(off[0].content.text().contains("$5 and $6"));
    }
    use super::*;

    fn kinds(blocks: &[Block]) -> Vec<String> {
        blocks
            .iter()
            .map(|b| {
                format!(
                    "{:?}/{}/{}/{:?}:{}",
                    b.kind,
                    b.indent,
                    b.quote,
                    b.marker,
                    b.text()
                )
            })
            .collect()
    }

    #[test]
    fn blocks_and_inline_styles() {
        let md = "# Title\n\nSome **bold**, *it*, ~~gone~~, `code` and [a link](https://x.dev).\n\n> quote\n\n---\n\n```rust\nfn main() {}\n```\n";
        let b = parse(md);
        assert_eq!(b.len(), 5);
        assert_eq!(b[0].kind, BlockKind::Heading(1));
        assert_eq!(b[1].text(), "Some bold, it, gone, code and a link.");
        let runs = b[1].content.runs();
        let at = |s: &str| {
            runs[b[1].content.run_at(b[1].text().find(s).unwrap()).unwrap()]
                .style
                .clone()
        };
        assert!(at("bold").bold && at("it,").italic && at("gone").strike && at("code").code);
        assert_eq!(at("a link").link.as_deref(), Some("https://x.dev"));
        assert_eq!(b[2].quote, 1);
        assert_eq!(b[3].kind, BlockKind::Rule);
        assert_eq!(
            b[4].kind,
            BlockKind::Code {
                lang: "rust".into()
            }
        );
        assert_eq!(b[4].text(), "fn main() {}");
        assert!(
            b[4].content
                .runs()
                .iter()
                .any(|r| r.style.color == Some(ColorRole::Magenta)),
            "fn is a keyword"
        );
    }

    #[test]
    fn lists_tasks_nesting_and_numbers() {
        let b = parse("1. one\n2. two\n   - [x] done\n   - [ ] todo\n3. three\n\n   more\n");
        assert_eq!(
            kinds(&b),
            vec![
                "Paragraph/1/0/Some(Ordered(1)):one",
                "Paragraph/1/0/Some(Ordered(2)):two",
                "Paragraph/2/0/Some(Task(true)):done",
                "Paragraph/2/0/Some(Task(false)):todo",
                "Paragraph/1/0/Some(Ordered(3)):three",
                "Paragraph/1/0/None:more",
            ]
        );
    }

    #[test]
    fn tables_and_autolinks() {
        let b = parse("| a | b |\n|:-|-:|\n| 1 | see www.x.org. |\n");
        let BlockKind::Table(t) = &b[0].kind else {
            panic!("{:?}", b[0].kind)
        };
        assert_eq!(t.align, vec![ColumnAlign::Left, ColumnAlign::Right]);
        assert_eq!(t.header_rows, 1);
        assert_eq!(b[0].text(), "a\tb\n1\tsee www.x.org.");
        assert_eq!(&b[0].text()[t.rows[1][1].clone()], "see www.x.org.");
        let link = b[0]
            .content
            .runs()
            .iter()
            .find_map(|r| r.style.link.clone())
            .unwrap();
        assert_eq!(&*link, "http://www.x.org");
        assert_eq!(
            find_urls("(see https://a.b/c_(d)), ok"),
            vec![(5..22, "https://a.b/c_(d)".to_string())]
        );
        assert!(find_urls("xhttps://no").is_empty());
    }

    #[test]
    fn alerts_and_html_blocks() {
        let b = parse("> [!WARNING]\n> careful\n\n<div>\nraw\n</div>\n");
        assert_eq!(b[0].quote_tone, Some(ColorRole::Yellow));
        assert_eq!(b[0].text(), "careful");
        assert_eq!(b[1].text(), "<div>\nraw\n</div>");
    }

    #[test]
    fn image_blocks_and_code_path_links() {
        let opts = MarkdownOptions::plain().with_images(true).with_path_links();
        let b = parse_with("Before ![a chart](https://x.dev/c.png \"t\") after.\n\n![](img.png)\n\n| ![i](t.png) |\n|-|\n| x |\n", &opts);
        let short: Vec<&str> = b
            .iter()
            .map(|b| match b.kind {
                BlockKind::Paragraph => "P",
                BlockKind::Image { .. } => "Image",
                BlockKind::Table(_) => "Table",
                _ => "?",
            })
            .collect();
        assert_eq!(short, ["P", "Image", "P", "Image", "Table"], "{b:#?}");
        assert_eq!(b[0].text(), "Before");
        assert_eq!(b[1].kind, BlockKind::Image { src: "https://x.dev/c.png".into() });
        assert_eq!(b[1].text(), "a chart");
        assert_eq!(b[1].content.runs()[0].style.link.as_deref(), Some("https://x.dev/c.png"));
        assert_eq!(b[2].text(), "after.");
        assert_eq!(b[3].text(), "img.png", "no alt: the source");
        assert!(b[4].text().contains('i'), "images in cells stay inline");
        // Without the option: inline alt text, as before.
        let plain = parse("Before ![a chart](https://x.dev/c.png) after.");
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].text(), "Before a chart after.");
        // Paths in inline code link to file://; other code stays plain.
        let b = parse_with("see `src/lib.rs:12`, `/home/a b`, `cargo build` and [t3](t3-thread://abc)", &opts);
        let link = |needle: &str| {
            let r = b[0].content.runs().iter().find(|r| b[0].text()[r.range.clone()] == *needle).unwrap();
            (r.style.code, r.style.link.as_deref().map(str::to_string))
        };
        assert_eq!(link("src/lib.rs:12"), (true, Some("file://src/lib.rs:12".into())));
        assert_eq!(link("/home/a b"), (true, None));
        assert_eq!(link("cargo build"), (true, None));
        assert_eq!(link("t3"), (false, Some("t3-thread://abc".into())), "custom schemes stay as written");
        for p in ["/home", "/home/node/report.html", "./src/main.rs", "~/notes.md", "src/lib.rs", "src/lib.rs:12:3", "Cargo.toml:4"] {
            assert!(file_path_link(p).is_some(), "{p}");
        }
        for p in ["rpc_outcome_unknown", "server.updateProvider", "cargo build", "github.com/x/y", "v1.2/3", "https://a.b/c", "a/b", "x: y"] {
            assert!(file_path_link(p).is_none(), "{p}");
        }
        assert_eq!(file_path_link("C:\\x\\y z.rs:3"), None, "blanks are never paths");
        assert_eq!(file_path_link("./a%b.rs").as_deref(), Some("file://./a%25b.rs"));
    }

    #[test]
    fn streamed_images_equal_one_shot() {
        let opts = MarkdownOptions::plain().with_images(true);
        let md = "Intro text.\n\n- ![first](a.png)\n- item ![second](b.png) tail\n\nEnd ![third](c.png)\n";
        let one = parse_with(md, &opts);
        for cut in 1..md.len() {
            if !md.is_char_boundary(cut) {
                continue;
            }
            let mut s = MarkdownStream::with_options(opts.clone());
            let mut blocks = Vec::new();
            s.push_into(&mut blocks, &md[..cut]);
            s.push_into(&mut blocks, &md[cut..]);
            assert_eq!(blocks, one, "cut at {cut}");
        }
        assert_eq!(one[1].marker, Some(ListMarker::Bullet), "{one:#?}");
        assert!(matches!(one[1].kind, BlockKind::Image { .. }));
    }

    #[test]
    fn inline_label_markdown() {
        let i = parse_inline("Saved **3** files to `~/x`");
        assert_eq!(i.text(), "Saved 3 files to ~/x");
        assert!(i.runs().iter().any(|r| r.style.bold) && i.runs().iter().any(|r| r.style.code));
    }

    const CORPUS: &[&str] = &[
        "# Heading\n\nParagraph with **bold** and *italic* text that goes on.\nSoft break here.\n\n## Second\n\nMore.\n",
        "- a\n- b\n  - nested\n  - nested 2\n- c\n\nAfter list.\n\n1. x\n1. y\n1. z\n\n3) p\n4) q\n",
        "```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\nText after code.\n\n~~~\nplain\n~~~\n",
        "> quote line\n> second\n\n> [!NOTE]\n> note\n\nPara\n\n---\n\n***\n\nend",
        "| h1 | h2 |\n|----|----|\n| a | b |\n| c | d |\n\nTable done. Visit https://example.com/path?x=1.\n",
        "[ref]: https://ref.example \"Title\"\n\nUse [ref] and [text][ref].\n\n- [ ] task\n- [x] done\n",
        "Setext\n======\n\nAnother\n------\n\nText\n#\nmore\n\n    indented code\n    line 2\n\nend\n",
        "1. one\n\n   para in item\n\n   ```\n   code in item\n   ```\n2. two\n10. ten\n\n- x\n\n  - y\n\n    deep\n",
        "<!-- comment\n\nstill comment -->\n\n<div>\nhtml\n</div>\n\nText with <span>inline</span> html & entities &amp; done\\\nhard break\n",
        "Para\n| a | b |\n|---|---|\n| 1 | 2 |\n\nx\n- item\n+ other\n* third\n\n> lazy\ncontinuation\n\nfin",
        "Emoji 👩‍👩‍👧 and CJK 日本語のテキスト and العربية text.\n\n```diff\n- old\n+ new\n@@ hunk\n```\n",
    ];

    fn streamed(md: &str, cuts: &[usize]) -> Vec<Block> {
        let mut s = MarkdownStream::new();
        let mut out = Vec::new();
        let mut last = 0;
        for &c in cuts.iter().chain(std::iter::once(&md.len())) {
            let mut c = c.min(md.len());
            while !md.is_char_boundary(c) {
                c += 1;
            }
            if c < last {
                continue;
            }
            let u = s.push(&md[last..c]);
            assert!(u.keep <= out.len());
            out.truncate(u.keep);
            out.extend(u.blocks);
            last = c;
        }
        out
    }

    #[test]
    fn streaming_every_split_equals_one_shot() {
        for md in CORPUS {
            let whole = parse(md);
            for cut in 0..=md.len() {
                assert_eq!(
                    kinds(&streamed(md, &[cut])),
                    kinds(&whole),
                    "cut at {cut} of {md:?}"
                );
            }
            // Byte by byte, like a token stream.
            let every: Vec<usize> = (0..md.len()).collect();
            let s = streamed(md, &every);
            assert_eq!(kinds(&s), kinds(&whole), "byte stream of {md:?}");
            assert_eq!(s, whole, "styles of {md:?}");
        }
    }

    #[test]
    fn streaming_random_chunks_of_a_long_text_equal_one_shot() {
        let mut md = String::new();
        for i in 0..40 {
            md.push_str(CORPUS[i % CORPUS.len()]);
            md.push_str("\n\n");
        }
        let whole = parse(&md);
        let mut seed = 0x9e3779b97f4a7c15u64;
        for _ in 0..20 {
            let mut cuts = Vec::new();
            let mut at = 0;
            while at < md.len() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                at += 1 + (seed % 40) as usize;
                cuts.push(at);
            }
            assert_eq!(streamed(&md, &cuts), whole);
        }
    }

    #[test]
    fn streaming_keeps_the_tail_small() {
        let mut s = MarkdownStream::new();
        let para = "A sentence of streamed text that keeps going.\n\n";
        let mut max_tail = 0;
        for _ in 0..2000 {
            s.push(para);
            max_tail = max_tail.max(s.tail().len());
        }
        assert!(max_tail <= 2 * para.len(), "tail grew to {max_tail}");
        assert!(s.committed() >= 1998);
        // A long list commits item by item too, numbering continues.
        let mut s = MarkdownStream::new();
        let mut blocks = Vec::new();
        for i in 0..500 {
            s.push_into(&mut blocks, &format!("1. item {i}\n"));
            max_tail = s.tail().len();
        }
        assert!(max_tail < 40);
        assert_eq!(blocks.len(), 500);
        assert_eq!(blocks[499].marker, Some(ListMarker::Ordered(500)));
    }
}
