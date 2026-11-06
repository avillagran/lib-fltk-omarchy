//! Display rows of a [`Patch`]: unified or side-by-side, with folded hunks,
//! collapsed files and hidden context. Rows are small copies (16 bytes)
//! rebuilt in one pass when the layout changes; their IDs are stable
//! (file, hunk, line) and sorted in display order, so an ID is found by
//! binary search.

use super::parse::{FileStatus, LineKind, Patch, NONE};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowKind {
    /// File header (path, status, counts).
    File,
    /// A note under the file header (binary, mode change, no content).
    Notice,
    /// `@@ … @@` header of hunk `hunk`.
    Hunk,
    /// Unified line `a` of hunk `hunk`.
    Line,
    /// Side-by-side row: left line `a`, right line `b` (either may be NONE).
    Pair,
    /// "Show more lines" after the last hunk `hunk`.
    More,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Row {
    pub kind: RowKind,
    pub file: u32,
    pub hunk: u32,
    pub a: u32,
    pub b: u32,
}

const HUNK_SHIFT: u32 = 24;
const FILE_SHIFT: u32 = 48;
const LINE_MASK: u64 = (1 << HUNK_SHIFT) - 1;

impl Row {
    /// Stable ID, increasing in display order.
    pub fn id(&self) -> u64 {
        let f = (self.file as u64) << FILE_SHIFT;
        let h = |slot: u64| f | (slot.min((1 << 24) - 1) << HUNK_SHIFT);
        let line = |i: u32| (i as u64 + 1).min(LINE_MASK);
        match self.kind {
            RowKind::File => f,
            RowKind::Notice => f | 1,
            RowKind::Hunk => h(self.hunk as u64 + 1),
            RowKind::Line => h(self.hunk as u64 + 1) | line(self.a),
            RowKind::Pair => h(self.hunk as u64 + 1) | line(self.a.min(self.b)),
            RowKind::More => h(self.hunk as u64 + 2),
        }
    }

    /// The hunk lines shown in this row (left first).
    pub fn lines(&self) -> [Option<u32>; 2] {
        match self.kind {
            RowKind::Line => [Some(self.a), None],
            RowKind::Pair => [(self.a != NONE).then_some(self.a), (self.b != NONE && self.b != self.a).then_some(self.b)],
            _ => [None, None],
        }
    }

    pub fn is_header(&self) -> bool {
        matches!(self.kind, RowKind::File | RowKind::Hunk)
    }
}

/// File of an ID.
pub(crate) fn file_of(id: u64) -> u32 {
    (id >> FILE_SHIFT) as u32
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Options {
    pub split: bool,
    pub hide_context: bool,
    /// A "Show more lines" row after the last hunk (an expand hook is set).
    pub more: bool,
    pub folded: HashSet<(u32, u32)>,
    pub collapsed: HashSet<u32>,
    /// Files whose end was reached by expanding below.
    pub at_end: HashSet<u32>,
}

/// The display rows of `patch`.
pub(crate) fn build(patch: &Patch, o: &Options) -> Vec<Row> {
    let mut rows = Vec::with_capacity(patch.line_count() + patch.files.len() * 3);
    let row = |kind, file, hunk, a, b| Row { kind, file, hunk, a, b };
    for (fi, f) in patch.files.iter().enumerate() {
        let fi = fi as u32;
        rows.push(row(RowKind::File, fi, 0, NONE, NONE));
        if o.collapsed.contains(&fi) {
            continue;
        }
        if f.binary || f.hunks.is_empty() || f.mode_changed() {
            rows.push(row(RowKind::Notice, fi, 0, NONE, NONE));
        }
        for (hi, h) in f.hunks.iter().enumerate() {
            let hi = hi as u32;
            rows.push(row(RowKind::Hunk, fi, hi, NONE, NONE));
            if o.folded.contains(&(fi, hi)) {
                continue;
            }
            let lines = &h.lines;
            if !o.split {
                for (i, l) in lines.iter().enumerate() {
                    if o.hide_context && l.kind == LineKind::Context {
                        continue;
                    }
                    rows.push(row(RowKind::Line, fi, hi, i as u32, NONE));
                }
                continue;
            }
            let mut i = 0;
            while i < lines.len() {
                match lines[i].kind {
                    LineKind::Context => {
                        if !o.hide_context {
                            rows.push(row(RowKind::Pair, fi, hi, i as u32, i as u32));
                        }
                        i += 1;
                    }
                    LineKind::Added => {
                        rows.push(row(RowKind::Pair, fi, hi, NONE, i as u32));
                        i += 1;
                    }
                    LineKind::Removed => {
                        let r0 = i;
                        while i < lines.len() && lines[i].kind == LineKind::Removed {
                            i += 1;
                        }
                        let a0 = i;
                        while i < lines.len() && lines[i].kind == LineKind::Added {
                            i += 1;
                        }
                        let (nr, na) = (a0 - r0, i - a0);
                        for k in 0..nr.max(na) {
                            let l = if k < nr { (r0 + k) as u32 } else { NONE };
                            let r = if k < na { (a0 + k) as u32 } else { NONE };
                            rows.push(row(RowKind::Pair, fi, hi, l, r));
                        }
                    }
                }
            }
        }
        if o.more && !f.hunks.is_empty() && !f.binary && f.status != FileStatus::Deleted && !o.at_end.contains(&fi) {
            rows.push(row(RowKind::More, fi, f.hunks.len() as u32 - 1, NONE, NONE));
        }
    }
    rows
}

// ------------------------------------------------------------ text cells

/// Columns of a character: tabs go to the next multiple of 4, East Asian
/// wide characters and emoji take two.
pub(crate) fn char_cols(c: char, col: usize) -> usize {
    match c {
        '\t' => 4 - col % 4,
        c if (c as u32) < 0x1100 => 1,
        c if is_wide(c) => 2,
        _ => 1,
    }
}

fn is_wide(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF |
        0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 |
        0xFFE0..=0xFFE6 | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD)
}

/// Display columns of a line.
pub(crate) fn cols(text: &str) -> usize {
    if text.is_ascii() && !text.contains('\t') {
        return text.len();
    }
    text.chars().fold(0, |col, c| col + char_cols(c, col))
}

/// Visual lines of `text` wrapped at `width` columns (at least 1).
pub(crate) fn visual_lines(text: &str, width: usize) -> usize {
    let width = width.max(1);
    if text.is_ascii() && !text.contains('\t') {
        return text.len().div_ceil(width).max(1);
    }
    // Same breaks as `Cells::wrap`: tab stops of the unwrapped line.
    let (mut lines, mut start, mut col) = (1, 0, 0);
    for c in text.chars() {
        let w = char_cols(c, col);
        if col + w - start > width && col > start {
            lines += 1;
            start = col;
        }
        col += w;
    }
    lines
}

/// A line laid out in cells: the text with tabs as spaces, and for each
/// character its byte in the source, its byte in `shown` and its column.
pub(crate) struct Cells {
    pub shown: String,
    /// (source byte, shown byte, column) per character, plus an end entry.
    pub at: Vec<(usize, usize, usize)>,
}

impl Cells {
    pub fn new(text: &str) -> Cells {
        let mut shown = String::with_capacity(text.len());
        let mut at = Vec::with_capacity(text.len() + 1);
        let mut col = 0;
        for (b, c) in text.char_indices() {
            at.push((b, shown.len(), col));
            let w = char_cols(c, col);
            if c == '\t' {
                shown.extend(std::iter::repeat(' ').take(w));
            } else if c.is_control() {
                shown.push('\u{fffd}');
            } else {
                shown.push(c);
            }
            col += w;
        }
        at.push((text.len(), shown.len(), col));
        Cells { shown, at }
    }

    pub fn cols(&self) -> usize {
        self.at.last().map_or(0, |a| a.2)
    }

    /// Column of a source byte offset.
    pub fn col_of(&self, byte: usize) -> usize {
        match self.at.binary_search_by_key(&byte, |a| a.0) {
            Ok(i) => self.at[i].2,
            Err(i) => self.at.get(i).map_or(self.cols(), |a| a.2),
        }
    }

    /// Shown text from column `c0` (rounded down to a character) to `c1`
    /// (rounded up), with the column where it starts.
    pub fn slice(&self, c0: usize, c1: usize) -> (&str, usize) {
        let i0 = self.at.partition_point(|a| a.2 <= c0).saturating_sub(1);
        let i1 = self.at.partition_point(|a| a.2 < c1).min(self.at.len() - 1);
        let (s0, s1) = (self.at[i0].1, self.at[i1.max(i0)].1);
        (&self.shown[s0..s1], self.at[i0].2)
    }

    /// Column ranges of the visual lines when wrapped at `width` columns
    /// (same breaks as [`visual_lines`]).
    pub fn wrap(&self, width: usize) -> Vec<(usize, usize)> {
        let width = width.max(1);
        let mut out = Vec::new();
        let mut start = 0;
        for w in self.at.windows(2) {
            let (c, next) = (w[0].2, w[1].2);
            if next - start > width && c > start {
                out.push((start, c));
                start = c;
            }
        }
        out.push((start, self.cols()));
        out
    }
}

/// The changed middle of two paired lines as byte ranges `(old, new)`,
/// widened to whole words. None when the lines share nothing at either
/// end (a full rewrite: highlighting all of it adds nothing).
pub(crate) fn changed_range(old: &str, new: &str) -> Option<((usize, usize), (usize, usize))> {
    let prefix = old.char_indices().zip(new.chars()).take_while(|((_, a), b)| a == b).last().map_or(0, |((i, a), _)| i + a.len_utf8());
    let (o_rest, n_rest) = (&old[prefix..], &new[prefix..]);
    let suffix = o_rest.chars().rev().zip(n_rest.chars().rev()).take_while(|(a, b)| a == b).map(|(a, _)| a.len_utf8()).sum::<usize>();
    if prefix == 0 && suffix == 0 {
        return None;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let widen = |s: &str, mut a: usize, mut b: usize| {
        while a > 0 {
            let c = s[..a].chars().next_back().unwrap();
            if !word(c) || !s[a..].chars().next().is_some_and(word) {
                break;
            }
            a -= c.len_utf8();
        }
        while b < s.len() {
            let c = s[b..].chars().next().unwrap();
            if !word(c) || b == 0 || !s[..b].chars().next_back().is_some_and(word) {
                break;
            }
            b += c.len_utf8();
        }
        (a, b)
    };
    let o = widen(old, prefix, old.len() - suffix.min(old.len() - prefix));
    let n = widen(new, prefix, new.len() - suffix.min(new.len() - prefix));
    if o.0 >= o.1 && n.0 >= n.1 {
        return None;
    }
    Some((o, n))
}

#[cfg(test)]
mod tests {
    use super::super::parse::parse;
    use super::*;

    const P: &str = "diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,6 +1,6 @@ fn x
 a
-b
-c
-d
+B
+C
 e
+f
diff --git a/i.png b/i.png
Binary files a/i.png and b/i.png differ
";

    fn kinds(rows: &[Row]) -> String {
        rows.iter().map(|r| match r.kind {
            RowKind::File => "F".to_string(),
            RowKind::Notice => "N".into(),
            RowKind::Hunk => "H".into(),
            RowKind::Line => format!("{}", r.a),
            RowKind::Pair => format!("({},{})", if r.a == NONE { "-".into() } else { r.a.to_string() }, if r.b == NONE { "-".into() } else { r.b.to_string() }),
            RowKind::More => "M".into(),
        }).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn unified_split_fold_context_and_ids() {
        let p = parse(P).unwrap();
        let mut o = Options::default();
        let u = build(&p, &o);
        assert_eq!(kinds(&u), "F H 0 1 2 3 4 5 6 7 F N");
        o.split = true;
        let s = build(&p, &o);
        assert_eq!(kinds(&s), "F H (0,0) (1,4) (2,5) (3,-) (6,6) (-,7) F N");
        for r in [&u, &s] {
            assert!(r.windows(2).all(|w| w[0].id() < w[1].id()), "IDs increase in display order");
        }
        o.hide_context = true;
        o.more = true;
        assert_eq!(kinds(&build(&p, &o)), "F H (1,4) (2,5) (3,-) (-,7) M F N");
        o.folded.insert((0, 0));
        o.collapsed.insert(1);
        assert_eq!(kinds(&build(&p, &o)), "F H M F");
        assert_eq!(file_of(s[9].id()), 1);
    }

    #[test]
    fn cells_tabs_wide_and_wrap() {
        let c = Cells::new("\tab日c");
        assert_eq!(c.shown, "    ab日c");
        assert_eq!(c.cols(), 9);
        assert_eq!((c.col_of(0), c.col_of(1), c.col_of(3), c.col_of(6)), (0, 4, 6, 8));
        assert_eq!(c.slice(5, 7), ("b日", 5));
        assert_eq!(c.wrap(4), vec![(0, 4), (4, 8), (8, 9)]);
        assert_eq!(visual_lines("\tab日c", 4), 3);
        assert_eq!(visual_lines("abcdefghij", 4), 3);
        assert_eq!(visual_lines("", 4), 1);
        assert_eq!(cols("a\tb"), 5);
    }

    #[test]
    fn word_ranges() {
        assert_eq!(changed_range("let x = foo(1);", "let x = bar(1);"), Some(((8, 11), (8, 11))));
        assert_eq!(changed_range("value = 10", "value = 1000"), Some(((10, 10), (8, 12))));
        assert_eq!(changed_range("abc", "xyz"), None);
        assert_eq!(changed_range("same", "same"), None);
        assert_eq!(changed_range("a.b", "a.b.c"), Some(((3, 3), (3, 5))));
        assert_eq!(changed_range("héllo wörld", "héllo world"), Some(((7, 13), (7, 12))));
    }
}
