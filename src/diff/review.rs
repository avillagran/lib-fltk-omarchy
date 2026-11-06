//! Review hooks of a [`DiffView`](super::DiffView): lines referred to by
//! file, side and number (they survive folding, expanding, split and wrap),
//! line-range selections, and annotations under lines.

use super::parse::{Line, Patch};
use crate::theme::{Rgb, Theme};

/// A side of a diff: the old file (removed lines) or the new file (added
/// lines). Unchanged lines are on both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Side {
    Old,
    New,
}

/// A line of one side of a file: what annotations and selections point at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineRef {
    pub side: Side,
    /// 1-based line number on that side.
    pub line: u32,
}

impl LineRef {
    pub fn new(side: Side, line: u32) -> LineRef {
        LineRef { side, line }
    }

    pub fn old(line: u32) -> LineRef {
        LineRef::new(Side::Old, line)
    }

    pub fn new_side(line: u32) -> LineRef {
        LineRef::new(Side::New, line)
    }
}

/// Lines `start..=end` (numbers on `side`) of file `file`: what a click,
/// Shift+click or drag down the line numbers selects, and what a comment
/// is about. Unified rows between them that have no number on `side`
/// (removed lines inside a new-side range) are shown selected too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineRange {
    pub file: usize,
    pub side: Side,
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    /// One line.
    pub fn line(file: usize, at: LineRef) -> LineRange {
        LineRange { file, side: at.side, start: at.line, end: at.line }
    }

    /// From `a` to `b` on `side`, either way round.
    pub fn between(file: usize, side: Side, a: u32, b: u32) -> LineRange {
        LineRange { file, side, start: a.min(b), end: a.max(b) }
    }

    /// Number of lines on its side.
    pub fn len(&self) -> u32 {
        self.end - self.start + 1
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// True when line `at` of `file` is in the range (same side).
    pub fn contains(&self, file: usize, at: LineRef) -> bool {
        file == self.file && at.side == self.side && (self.start..=self.end).contains(&at.line)
    }

    pub fn first(&self) -> LineRef {
        LineRef::new(self.side, self.start)
    }

    pub fn last(&self) -> LineRef {
        LineRef::new(self.side, self.end)
    }
}

/// Something the app shows under a line: a review comment, a thread, a
/// composer. The view reserves `height` pixels under line `at` of file
/// `file`; the app paints them ([`DiffView::annotation_painter`]) or hosts a
/// widget there ([`DiffView::host_annotation`]) by `id`.
///
/// [`DiffView::annotation_painter`]: super::DiffView::annotation_painter
/// [`DiffView::host_annotation`]: super::DiffView::host_annotation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Annotation {
    pub file: usize,
    pub at: LineRef,
    pub id: String,
    /// Reserved height in pixels (0 hides it).
    pub height: i32,
    /// What a screen reader reads for a painted annotation (hosted widgets
    /// speak for themselves). Empty: "Note under line N".
    pub label: String,
}

impl Annotation {
    pub fn new(file: usize, at: LineRef, id: impl Into<String>, height: i32) -> Annotation {
        Annotation { file, at, id: id.into(), height, label: String::new() }
    }

    /// Name for screen readers (painted annotations).
    pub fn label(mut self, text: impl Into<String>) -> Annotation {
        self.label = text.into();
        self
    }
}

/// What [`DiffView::annotation_painter`](super::DiffView::annotation_painter)
/// paints: annotation `annotation` in `x, y, w, h` (window coordinates,
/// clipped to the view). Use theme colors (`theme`, `under`).
pub struct AnnotationPaint<'a> {
    pub annotation: &'a Annotation,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// The diff's cursor is on the annotated line.
    pub current: bool,
    pub theme: &'a Theme,
    /// The color under the annotation (the view's background).
    pub under: Rgb,
}

/// A line's number on `side` (None when the line is not on that side).
pub(crate) fn number(l: &Line, side: Side) -> Option<u32> {
    let n = match side {
        Side::Old => l.old,
        Side::New => l.new,
    };
    (n > 0).then_some(n)
}

/// Hunk and line index of line `n` on `side` of file `file`.
pub(crate) fn find_line(p: &Patch, file: usize, side: Side, n: u32) -> Option<(u32, u32)> {
    let f = p.files.get(file)?;
    let start = |h: &super::Hunk| match side {
        Side::Old => h.old_start,
        Side::New => h.new_start,
    };
    // Hunks are in file order: the line is in the last one starting at or
    // before `n` (an empty side starts one line early: look at two).
    let h = f.hunks.partition_point(|h| start(h) <= n);
    for hi in (h.saturating_sub(2)..h.min(f.hunks.len())).rev() {
        if let Some(li) = f.hunks[hi].lines.iter().position(|l| number(l, side) == Some(n)) {
            return Some((hi as u32, li as u32));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::parse::parse;
    use super::*;

    const P: &str = "diff --git a/f b/f
--- a/f
+++ b/f
@@ -1,4 +1,4 @@
 a
-b
+B
 c
 d
@@ -20,2 +20,3 @@
 x
+y
 z
";

    #[test]
    fn find_lines_by_side() {
        let p = parse(P).unwrap();
        assert_eq!(find_line(&p, 0, Side::Old, 2), Some((0, 1)));
        assert_eq!(find_line(&p, 0, Side::New, 2), Some((0, 2)));
        assert_eq!(find_line(&p, 0, Side::New, 21), Some((1, 1)));
        assert_eq!(find_line(&p, 0, Side::Old, 21), Some((1, 2)));
        assert_eq!(find_line(&p, 0, Side::New, 10), None);
        assert_eq!(find_line(&p, 1, Side::New, 1), None);
    }

    #[test]
    fn ranges() {
        let r = LineRange::between(0, Side::New, 14, 12);
        assert_eq!((r.start, r.end, r.len()), (12, 14, 3));
        assert!(r.contains(0, LineRef::new_side(13)));
        assert!(!r.contains(0, LineRef::old(13)));
        assert!(!r.contains(1, LineRef::new_side(13)));
        assert_eq!(LineRange::line(2, LineRef::old(7)), LineRange { file: 2, side: Side::Old, start: 7, end: 7 });
    }
}
