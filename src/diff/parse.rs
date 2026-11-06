//! Unified diff parser: `git diff` / `git show` / `git format-patch`
//! output and plain `diff -u` into a [`Patch`].
//!
//! Handles several files per patch, renames and copies (with or without
//! content changes), new and deleted files, mode changes, binary markers
//! (`Binary files … differ` and `GIT binary patch` blocks), `\ No newline
//! at end of file`, hunk headers with function context, C-quoted paths
//! (`"a/caf\303\251.txt"`), unquoted paths with spaces, CRLF line ends and
//! context lines whose single space was stripped by an editor. Mail
//! headers, commit messages and diffstats around the diffs are skipped.
//! Combined diffs of merges (`diff --cc`) are rejected.
//!
//! Line text is copied once into one buffer owned by the patch; a line is
//! an offset into it (about 24 bytes per line), so the raw patch can be
//! dropped after parsing.

use std::fmt;

/// Size limits checked while parsing. The defaults are o3-code's review
/// limits (`docs/DIFF_REVIEW.md`): 2 MiB of patch, 256 files, 12 000 hunk
/// lines and 16 KiB per line. Going over a limit is an error, never a
/// silently truncated patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_files: usize,
    pub max_lines: usize,
    pub max_line_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_bytes: 2 * 1024 * 1024, max_files: 256, max_lines: 12_000, max_line_bytes: 16 * 1024 }
    }
}

impl Limits {
    /// No limits (trusted input; memory and time grow with the patch).
    pub fn none() -> Limits {
        Limits { max_bytes: usize::MAX, max_files: usize::MAX, max_lines: usize::MAX, max_line_bytes: usize::MAX }
    }
}

/// Why a patch was not parsed. `Display` is translated (`tr!`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffError {
    /// The patch has more bytes than `Limits::max_bytes`.
    TooLarge { bytes: usize, max: usize },
    /// More files than `Limits::max_files`.
    TooManyFiles { max: usize },
    /// More hunk lines than `Limits::max_lines`.
    TooManyLines { max: usize },
    /// A line longer than `Limits::max_line_bytes` (1-based patch line).
    LineTooLong { line: usize, max: usize },
    /// Not a unified diff at this 1-based patch line.
    Malformed { line: usize, reason: &'static str },
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mib = |b: usize| format!("{:.1}", b as f64 / (1024.0 * 1024.0));
        let s = match self {
            DiffError::TooLarge { bytes, max } => crate::tr!("The patch is too large ({size} MiB, limit {max} MiB)", size = mib(*bytes), max = mib(*max)),
            DiffError::TooManyFiles { max } => crate::tr!("The patch changes more than {max} files", max = max),
            DiffError::TooManyLines { max } => crate::tr!("The patch has more than {max} lines", max = max),
            DiffError::LineTooLong { line, max } => crate::tr!("Line {line} of the patch is longer than {max} bytes", line = line, max = max),
            DiffError::Malformed { line, reason } => crate::tr!("Line {line} of the patch is not a valid diff ({reason})", line = line, reason = reason),
        };
        f.write_str(&s)
    }
}

impl std::error::Error for DiffError {}

/// What happened to a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
}

impl FileStatus {
    /// One-letter code as in `git status --short` (M, A, D, R, C).
    pub fn letter(self) -> &'static str {
        match self {
            FileStatus::Modified => "M",
            FileStatus::Added => "A",
            FileStatus::Deleted => "D",
            FileStatus::Renamed => "R",
            FileStatus::Copied => "C",
        }
    }
}

/// Kind of a hunk line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

impl LineKind {
    /// The patch marker: ' ', '+' or '-'.
    pub fn marker(self) -> char {
        match self {
            LineKind::Context => ' ',
            LineKind::Added => '+',
            LineKind::Removed => '-',
        }
    }
}

/// Marker for "no partner line".
pub(crate) const NONE: u32 = u32::MAX;

/// One line of a hunk. Its text is `patch.text(line)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    /// 1-based line in the old file (0 for added lines).
    pub old: u32,
    /// 1-based line in the new file (0 for removed lines).
    pub new: u32,
    /// Followed by `\ No newline at end of file`.
    pub no_newline: bool,
    start: u32,
    len: u32,
    /// Index (in the hunk) of the line this one replaces or is replaced
    /// by: the i-th removed line of a block pairs with the i-th added line
    /// that follows it. `NONE` when unpaired.
    pub(crate) pair: u32,
}

impl Line {
    /// Index in the hunk of the paired line (a removed line and the added
    /// line that replaces it), used for split rows and word highlights.
    pub fn pair(&self) -> Option<usize> {
        (self.pair != NONE).then_some(self.pair as usize)
    }
}

/// One `@@ -a,b +c,d @@ section` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    /// Function context after the second `@@` (may be empty).
    pub section: String,
    pub lines: Vec<Line>,
}

impl Hunk {
    /// `@@ -a,b +c,d @@` (without the section).
    pub fn range(&self) -> String {
        format!("@@ -{},{} +{},{} @@", self.old_start, self.old_count, self.new_start, self.new_count)
    }
    /// Last new-file line covered by the hunk (start − 1 when empty).
    pub fn new_end(&self) -> u32 {
        (self.new_start + self.new_count).saturating_sub(1)
    }
}

/// The changes to one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDiff {
    /// Path before the change; None for an added file.
    pub old_path: Option<String>,
    /// Path after the change; None for a deleted file.
    pub new_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    /// `similarity index` of a rename or copy, in percent.
    pub similarity: Option<u8>,
    pub hunks: Vec<Hunk>,
    pub additions: u32,
    pub deletions: u32,
}

impl FileDiff {
    fn empty() -> FileDiff {
        FileDiff {
            old_path: None,
            new_path: None,
            status: FileStatus::Modified,
            binary: false,
            old_mode: None,
            new_mode: None,
            similarity: None,
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
        }
    }

    /// The path to show: the new path, or the old one for a deleted file.
    pub fn path(&self) -> &str {
        self.new_path.as_deref().or(self.old_path.as_deref()).unwrap_or("")
    }

    /// `old → new` for renames and copies, else [`FileDiff::path`].
    pub fn display_path(&self) -> String {
        match (&self.old_path, &self.new_path, self.status) {
            (Some(o), Some(n), FileStatus::Renamed | FileStatus::Copied) if o != n => format!("{o} → {n}"),
            _ => self.path().to_string(),
        }
    }

    /// Lines in all hunks.
    pub fn line_count(&self) -> usize {
        self.hunks.iter().map(|h| h.lines.len()).sum()
    }

    /// Mode changed (`old mode`/`new mode`) without the file being added or deleted.
    pub fn mode_changed(&self) -> bool {
        matches!((&self.old_mode, &self.new_mode), (Some(a), Some(b)) if a != b)
    }
}

/// A parsed patch: its files and the text of every line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Patch {
    pub files: Vec<FileDiff>,
    text: String,
}

/// Parses with the default [`Limits`].
pub fn parse(patch: &str) -> Result<Patch, DiffError> {
    parse_with(patch, Limits::default())
}

/// Parses with explicit limits.
pub fn parse_with(patch: &str, limits: Limits) -> Result<Patch, DiffError> {
    if patch.len() > limits.max_bytes {
        return Err(DiffError::TooLarge { bytes: patch.len(), max: limits.max_bytes });
    }
    Parser { lines: patch.split('\n').collect(), i: 0, limits, out: Patch::default(), total: 0 }.run()
}

impl Patch {
    /// Text of a line (no marker, no line end).
    pub fn text(&self, line: &Line) -> &str {
        &self.text[line.start as usize..(line.start + line.len) as usize]
    }

    /// Total (additions, deletions).
    pub fn totals(&self) -> (u32, u32) {
        self.files.iter().fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions))
    }

    /// Lines in all hunks of all files.
    pub fn line_count(&self) -> usize {
        self.files.iter().map(|f| f.line_count()).sum()
    }

    /// Heap bytes held (line text and line records), for measurements.
    pub fn heap_bytes(&self) -> usize {
        self.text.capacity()
            + self
                .files
                .iter()
                .map(|f| std::mem::size_of::<FileDiff>() + f.hunks.iter().map(|h| std::mem::size_of::<Hunk>() + h.section.capacity() + h.lines.capacity() * std::mem::size_of::<Line>()).sum::<usize>())
                .sum::<usize>()
    }

    /// The patch text of a hunk line range as `git diff` writes it
    /// (marker + text, `\ No newline…` where needed).
    pub fn line_as_patch(&self, line: &Line, out: &mut String) {
        out.push(line.kind.marker());
        out.push_str(self.text(line));
        out.push('\n');
        if line.no_newline {
            out.push_str("\\ No newline at end of file\n");
        }
    }

    /// New-file lines hidden above hunk `h` of file `f`: `(first, last)`,
    /// 1-based and inclusive, between the previous hunk (or the start of
    /// the file) and this one. None when nothing is hidden.
    pub fn gap_above(&self, f: usize, h: usize) -> Option<(u32, u32)> {
        let hunks = &self.files.get(f)?.hunks;
        let hunk = hunks.get(h)?;
        let first = if h == 0 { 1 } else { hunks[h - 1].new_end() + 1 };
        let last = hunk.new_start.checked_sub(1)?;
        (last >= first).then_some((first, last))
    }

    /// Adds context lines `lines` (new-file text, starting at new-file line
    /// `first`) around hunk `h` of file `f`: right above it when they end
    /// at `new_start − 1`, or right below it when they start at its last
    /// line + 1. Hunks that touch afterwards are merged. Returns false when
    /// the lines do not border the hunk or overlap another one.
    pub fn insert_context(&mut self, f: usize, h: usize, first: u32, lines: &[String]) -> bool {
        let n = lines.len() as u32;
        let Some(file) = self.files.get(f) else { return false };
        let Some(hunk) = file.hunks.get(h) else { return false };
        if n == 0 || first == 0 {
            return false;
        }
        let above = first + n == hunk.new_start;
        let below = first == hunk.new_end() + 1;
        if !above && !below {
            return false;
        }
        // Never overlap a neighbour.
        if above && h > 0 && first <= file.hunks[h - 1].new_end() {
            return false;
        }
        if below && h + 1 < file.hunks.len() && first + n > file.hunks[h + 1].new_start {
            return false;
        }
        // Context lines are equal in both files: old = new + offset.
        let offset = hunk.old_start as i64 - hunk.new_start as i64;
        let mut fresh = Vec::with_capacity(lines.len());
        for (k, t) in lines.iter().enumerate() {
            let t = t.strip_suffix('\r').unwrap_or(t);
            let start = self.text.len() as u32;
            self.text.push_str(t);
            let new = first + k as u32;
            fresh.push(Line { kind: LineKind::Context, old: (new as i64 + offset).max(1) as u32, new, no_newline: false, start, len: t.len() as u32, pair: NONE });
        }
        let hunk = &mut self.files[f].hunks[h];
        if above {
            hunk.old_start = (hunk.old_start as i64 - n as i64).max(1) as u32;
            hunk.new_start = first;
            fresh.append(&mut hunk.lines);
            hunk.lines = fresh;
        } else {
            // A "no newline" marker on the old last line no longer applies.
            if let Some(l) = hunk.lines.iter_mut().rev().find(|l| l.kind != LineKind::Removed) {
                l.no_newline = false;
            }
            hunk.lines.extend(fresh);
        }
        hunk.old_count += n;
        hunk.new_count += n;
        pair_lines(&mut hunk.lines);
        // Merge with a neighbour that now touches.
        let hunks = &mut self.files[f].hunks;
        let merge_at = if above && h > 0 && hunks[h - 1].new_end() + 1 == hunks[h].new_start {
            Some(h - 1)
        } else if below && h + 1 < hunks.len() && hunks[h].new_end() + 1 == hunks[h + 1].new_start {
            Some(h)
        } else {
            None
        };
        if let Some(a) = merge_at {
            let next = hunks.remove(a + 1);
            let prev = &mut hunks[a];
            prev.old_count += next.old_count;
            prev.new_count += next.new_count;
            prev.lines.extend(next.lines);
            pair_lines(&mut prev.lines);
        }
        true
    }
}

/// Pairs the i-th line of each removed block with the i-th line of the
/// added block right after it.
pub(crate) fn pair_lines(lines: &mut [Line]) {
    for l in lines.iter_mut() {
        l.pair = NONE;
    }
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Removed {
            i += 1;
            continue;
        }
        let r0 = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let a0 = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }
        let n = (a0 - r0).min(i - a0);
        for k in 0..n {
            lines[r0 + k].pair = (a0 + k) as u32;
            lines[a0 + k].pair = (r0 + k) as u32;
        }
    }
}

struct Parser<'a> {
    lines: Vec<&'a str>,
    i: usize,
    limits: Limits,
    out: Patch,
    total: usize,
}

fn strip_cr(s: &str) -> &str {
    s.strip_suffix('\r').unwrap_or(s)
}

impl<'a> Parser<'a> {
    fn line(&self, i: usize) -> Option<&'a str> {
        self.lines.get(i).map(|l| strip_cr(l))
    }

    fn malformed(&self, reason: &'static str) -> DiffError {
        DiffError::Malformed { line: self.i + 1, reason }
    }

    fn run(mut self) -> Result<Patch, DiffError> {
        // A trailing '\n' leaves one empty last element.
        if self.lines.last() == Some(&"") {
            self.lines.pop();
        }
        while self.i < self.lines.len() {
            let l = self.line(self.i).unwrap_or("");
            if l.len() > self.limits.max_line_bytes {
                return Err(DiffError::LineTooLong { line: self.i + 1, max: self.limits.max_line_bytes });
            }
            if l.starts_with("diff --cc ") || l.starts_with("diff --combined ") {
                return Err(self.malformed("combined diff"));
            }
            if let Some(rest) = l.strip_prefix("diff --git ") {
                self.i += 1;
                let mut f = FileDiff::empty();
                if let Some((a, b)) = git_header_paths(rest) {
                    f.old_path = Some(a);
                    f.new_path = Some(b);
                }
                self.file(f, true)?;
            } else if l.starts_with("--- ") && self.line(self.i + 1).is_some_and(|n| n.starts_with("+++ ")) {
                self.file(FileDiff::empty(), false)?;
            } else {
                self.i += 1; // mail header, message, diffstat, signature
            }
        }
        self.out.text.shrink_to_fit();
        Ok(self.out)
    }

    fn push_file(&mut self, mut f: FileDiff) -> Result<(), DiffError> {
        if self.out.files.len() >= self.limits.max_files {
            return Err(DiffError::TooManyFiles { max: self.limits.max_files });
        }
        if f.status == FileStatus::Modified {
            if f.old_path.is_none() {
                f.status = FileStatus::Added;
            } else if f.new_path.is_none() {
                f.status = FileStatus::Deleted;
            }
        }
        self.out.files.push(f);
        Ok(())
    }

    /// Extended header lines, then hunks, of one file.
    fn file(&mut self, mut f: FileDiff, git: bool) -> Result<(), DiffError> {
        while let Some(l) = self.line(self.i) {
            if l.len() > self.limits.max_line_bytes {
                return Err(DiffError::LineTooLong { line: self.i + 1, max: self.limits.max_line_bytes });
            }
            if git && l.starts_with("diff ") {
                break;
            }
            if let Some(m) = l.strip_prefix("old mode ") {
                f.old_mode = Some(m.trim().into());
            } else if let Some(m) = l.strip_prefix("new mode ") {
                f.new_mode = Some(m.trim().into());
            } else if let Some(m) = l.strip_prefix("deleted file mode ") {
                f.status = FileStatus::Deleted;
                f.old_mode = Some(m.trim().into());
                f.new_path = None;
            } else if let Some(m) = l.strip_prefix("new file mode ") {
                f.status = FileStatus::Added;
                f.new_mode = Some(m.trim().into());
                f.old_path = None;
            } else if let Some(p) = l.strip_prefix("similarity index ") {
                f.similarity = p.trim().trim_end_matches('%').parse().ok();
            } else if l.starts_with("dissimilarity index ") || l.starts_with("index ") {
            } else if let Some(p) = l.strip_prefix("rename from ") {
                f.status = FileStatus::Renamed;
                f.old_path = Some(unquote(p));
            } else if let Some(p) = l.strip_prefix("rename to ") {
                f.status = FileStatus::Renamed;
                f.new_path = Some(unquote(p));
            } else if let Some(p) = l.strip_prefix("copy from ") {
                f.status = FileStatus::Copied;
                f.old_path = Some(unquote(p));
            } else if let Some(p) = l.strip_prefix("copy to ") {
                f.status = FileStatus::Copied;
                f.new_path = Some(unquote(p));
            } else if l.starts_with("Binary files ") && l.ends_with(" differ") {
                f.binary = true;
            } else if l == "GIT binary patch" {
                f.binary = true;
                // literal/delta blocks until a blank line pair or the next file.
                self.i += 1;
                while let Some(b) = self.line(self.i) {
                    if b.starts_with("diff ") {
                        break;
                    }
                    self.i += 1;
                }
                continue;
            } else if l.starts_with("--- ") && self.line(self.i + 1).is_some_and(|n| n.starts_with("+++ ")) {
                if !f.hunks.is_empty() {
                    break; // the next file of a plain (non-git) diff
                }
                let old = header_path(&l[4..]);
                let new = header_path(&self.line(self.i + 1).unwrap_or("+++ ")[4..]);
                let prefixed = old.as_deref().map_or(true, |p| p.starts_with("a/")) && new.as_deref().map_or(true, |p| p.starts_with("b/"));
                let strip = |p: Option<String>| p.map(|p| if prefixed { p[2..].to_string() } else { p });
                let (old, new) = (strip(old), strip(new));
                // `/dev/null` decides additions and deletions; renames and
                // copies keep their `rename from/to` names; otherwise these
                // names are exact (the git header is ambiguous with spaces).
                let renamed = matches!(f.status, FileStatus::Renamed | FileStatus::Copied);
                if old.is_none() {
                    f.old_path = None;
                } else if !renamed {
                    f.old_path = old;
                }
                if new.is_none() {
                    f.new_path = None;
                } else if !renamed {
                    f.new_path = new;
                }
                self.i += 2;
                continue;
            } else if l.starts_with("@@ ") {
                let h = self.hunk(l)?;
                f.hunks.push(h);
                continue;
            } else if l.starts_with("\\ ") {
                // "\ No newline at end of file" after the last hunk line.
                if let Some(last) = f.hunks.last_mut().and_then(|h| h.lines.last_mut()) {
                    last.no_newline = true;
                }
            } else if !git || !f.hunks.is_empty() {
                // Text after the last hunk (signature, next message): file ends.
                break;
            }
            self.i += 1;
        }
        for h in &f.hunks {
            for l in &h.lines {
                match l.kind {
                    LineKind::Added => f.additions += 1,
                    LineKind::Removed => f.deletions += 1,
                    LineKind::Context => {}
                }
            }
        }
        self.push_file(f)
    }

    /// A hunk from its header at `self.i`; leaves `self.i` after its last line.
    fn hunk(&mut self, header: &str) -> Result<Hunk, DiffError> {
        let (os, oc, ns, nc, section) = parse_hunk_header(header).ok_or_else(|| self.malformed("hunk header"))?;
        let mut h = Hunk { old_start: os, old_count: oc, new_start: ns, new_count: nc, section: section.to_string(), lines: Vec::new() };
        self.i += 1;
        let (mut old_left, mut new_left) = (oc, nc);
        let (mut old_no, mut new_no) = (os, ns);
        // An empty hunk side starts at the line before it (git writes `-0,0`).
        if oc == 0 {
            old_no = os + 1;
        }
        if nc == 0 {
            new_no = ns + 1;
        }
        while old_left > 0 || new_left > 0 {
            let Some(l) = self.line(self.i) else { break };
            if l.len() > self.limits.max_line_bytes {
                return Err(DiffError::LineTooLong { line: self.i + 1, max: self.limits.max_line_bytes });
            }
            let (kind, body) = match l.as_bytes().first() {
                Some(b' ') => (LineKind::Context, &l[1..]),
                Some(b'+') => (LineKind::Added, &l[1..]),
                Some(b'-') => (LineKind::Removed, &l[1..]),
                // An editor stripped the single space of an empty context line.
                None => (LineKind::Context, ""),
                Some(b'\\') => {
                    if let Some(last) = h.lines.last_mut() {
                        last.no_newline = true;
                    }
                    self.i += 1;
                    continue;
                }
                _ => break, // truncated hunk: keep what was read
            };
            match kind {
                LineKind::Context if old_left == 0 || new_left == 0 => break,
                LineKind::Removed if old_left == 0 => break,
                LineKind::Added if new_left == 0 => break,
                _ => {}
            }
            self.total += 1;
            if self.total > self.limits.max_lines {
                return Err(DiffError::TooManyLines { max: self.limits.max_lines });
            }
            let start = self.out.text.len() as u32;
            self.out.text.push_str(body);
            let (old, new) = match kind {
                LineKind::Context => {
                    old_left -= 1;
                    new_left -= 1;
                    old_no += 1;
                    new_no += 1;
                    (old_no - 1, new_no - 1)
                }
                LineKind::Removed => {
                    old_left -= 1;
                    old_no += 1;
                    (old_no - 1, 0)
                }
                LineKind::Added => {
                    new_left -= 1;
                    new_no += 1;
                    (0, new_no - 1)
                }
            };
            h.lines.push(Line { kind, old, new, no_newline: false, start, len: body.len() as u32, pair: NONE });
            self.i += 1;
        }
        // "\ No newline at end of file" right after the last counted line.
        while let Some(l) = self.line(self.i) {
            if !l.starts_with("\\ ") {
                break;
            }
            if let Some(last) = h.lines.last_mut() {
                last.no_newline = true;
            }
            self.i += 1;
        }
        pair_lines(&mut h.lines);
        Ok(h)
    }
}

/// `@@ -a[,b] +c[,d] @@[ section]`.
fn parse_hunk_header(l: &str) -> Option<(u32, u32, u32, u32, &str)> {
    let rest = l.strip_prefix("@@ -")?;
    let end = rest.find(" @@")?;
    let (ranges, tail) = (&rest[..end], &rest[end + 3..]);
    let (old, new) = ranges.split_once(" +")?;
    let range = |s: &str| -> Option<(u32, u32)> {
        match s.split_once(',') {
            Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
            None => Some((s.parse().ok()?, 1)),
        }
    };
    let (os, oc) = range(old)?;
    let (ns, nc) = range(new)?;
    Some((os, oc, ns, nc, tail.strip_prefix(' ').unwrap_or(tail)))
}

/// A `---`/`+++` path: unquoted, without a trailing tab + timestamp; None
/// for `/dev/null`.
fn header_path(s: &str) -> Option<String> {
    let s = s.trim_end();
    let p = if s.starts_with('"') {
        let (p, _) = take_quoted(s)?;
        p
    } else {
        s.split('\t').next().unwrap_or(s).to_string()
    };
    (p != "/dev/null").then_some(p)
}

/// Paths of `diff --git a/x b/y` (with their prefixes removed when both
/// have the usual `a/` and `b/`). Unquoted names with spaces are split
/// where both halves are equal (the common no-rename case); renames get
/// their exact names later from `rename from/to` or `---`/`+++`.
fn git_header_paths(rest: &str) -> Option<(String, String)> {
    let (a, b) = if rest.starts_with('"') {
        let (a, used) = take_quoted(rest)?;
        let tail = rest[used..].trim_start();
        let b = if tail.starts_with('"') { take_quoted(tail)?.0 } else { tail.to_string() };
        (a, b)
    } else if let Some(i) = rest.find(" \"") {
        (rest[..i].to_string(), take_quoted(&rest[i + 1..])?.0)
    } else {
        // "a/X b/X": equal halves when the name has spaces.
        let n = rest.len();
        let half = (n.saturating_sub(1)) / 2;
        if n % 2 == 1 && rest.is_char_boundary(half) && rest.as_bytes().get(half) == Some(&b' ') && rest[..half].get(2..) == rest[half + 1..].get(2..) {
            (rest[..half].to_string(), rest[half + 1..].to_string())
        } else {
            let i = rest.find(" b/").or_else(|| rest.find(' '))?;
            (rest[..i].to_string(), rest[i + 1..].to_string())
        }
    };
    if a.starts_with("a/") && b.starts_with("b/") {
        Some((a[2..].to_string(), b[2..].to_string()))
    } else {
        Some((a, b))
    }
}

/// Unquotes a C-quoted git path (`"tab\there"`), or returns it as is.
pub fn unquote(s: &str) -> String {
    let s = s.trim_end();
    if s.starts_with('"') {
        if let Some((p, _)) = take_quoted(s) {
            return p;
        }
    }
    s.to_string()
}

/// Reads a C-quoted string at the start of `s`: (value, bytes used).
fn take_quoted(s: &str) -> Option<(String, usize)> {
    let b = s.as_bytes();
    if b.first() != Some(&b'"') {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 1;
    while i < b.len() {
        match b[i] {
            b'"' => return Some((String::from_utf8_lossy(&out).into_owned(), i + 1)),
            b'\\' if i + 1 < b.len() => {
                i += 1;
                let c = b[i];
                match c {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'a' => out.push(7),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'v' => out.push(11),
                    b'0'..=b'7' => {
                        let mut v: u32 = 0;
                        let mut k = 0;
                        while k < 3 && i < b.len() && (b'0'..=b'7').contains(&b[i]) {
                            v = v * 8 + (b[i] - b'0') as u32;
                            i += 1;
                            k += 1;
                        }
                        out.push(v as u8);
                        continue;
                    }
                    other => out.push(other),
                }
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `git diff` output (git 2.47), trimmed.
    const MODIFIED: &str = "diff --git a/src/main.rs b/src/main.rs
index 3b18e51..a0423e1 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,6 @@ use std::io;
 fn main() {
-    println!(\"hello\");
+    println!(\"hello, world\");
+    run();
 }

 fn run() {}
";

    #[test]
    fn modified_file_lines_numbers_and_section() {
        let p = parse(MODIFIED).unwrap();
        assert_eq!(p.files.len(), 1);
        let f = &p.files[0];
        assert_eq!((f.old_path.as_deref(), f.new_path.as_deref(), f.status), (Some("src/main.rs"), Some("src/main.rs"), FileStatus::Modified));
        assert_eq!((f.additions, f.deletions), (2, 1));
        let h = &f.hunks[0];
        assert_eq!((h.old_start, h.old_count, h.new_start, h.new_count, h.section.as_str()), (1, 5, 1, 6, "use std::io;"));
        let kinds: Vec<_> = h.lines.iter().map(|l| (l.kind.marker(), l.old, l.new)).collect();
        assert_eq!(kinds, vec![(' ', 1, 1), ('-', 2, 0), ('+', 0, 2), ('+', 0, 3), (' ', 3, 4), (' ', 4, 5), (' ', 5, 6)]);
        assert_eq!(p.text(&h.lines[1]), "    println!(\"hello\");");
        assert_eq!(p.text(&h.lines[4]), "}");
        assert_eq!(p.text(&h.lines[5]), "", "empty context line");
        assert_eq!((h.lines[1].pair(), h.lines[2].pair(), h.lines[3].pair()), (Some(2), Some(1), None));
    }

    #[test]
    fn rename_copy_mode_binary_and_new_deleted() {
        let patch = "diff --git a/old name.txt b/new name.txt
similarity index 90%
rename from old name.txt
rename to new name.txt
index 1111111..2222222 100644
--- a/old name.txt
+++ b/new name.txt
@@ -1 +1 @@
-a
+b
diff --git a/pure.txt b/moved/pure.txt
similarity index 100%
rename from pure.txt
rename to moved/pure.txt
diff --git a/script.sh b/script.sh
old mode 100644
new mode 100755
diff --git a/logo.png b/logo.png
index 0000000..1234567 100644
Binary files a/logo.png and b/logo.png differ
diff --git a/new.txt b/new.txt
new file mode 100644
index 0000000..e69de29
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,2 @@
+one
+two
diff --git a/gone.txt b/gone.txt
deleted file mode 100644
index e69de29..0000000
--- a/gone.txt
+++ /dev/null
@@ -1 +0,0 @@
-bye
diff --git a/empty b/empty
new file mode 100644
index 0000000..e69de29
diff --git a/a.c b/b.c
similarity index 80%
copy from a.c
copy to b.c
";
        let p = parse(patch).unwrap();
        let s: Vec<_> = p.files.iter().map(|f| (f.status, f.old_path.clone(), f.new_path.clone(), f.binary)).collect();
        let o = |s: &str| Some(s.to_string());
        assert_eq!(s, vec![
            (FileStatus::Renamed, o("old name.txt"), o("new name.txt"), false),
            (FileStatus::Renamed, o("pure.txt"), o("moved/pure.txt"), false),
            (FileStatus::Modified, o("script.sh"), o("script.sh"), false),
            (FileStatus::Modified, o("logo.png"), o("logo.png"), true),
            (FileStatus::Added, None, o("new.txt"), false),
            (FileStatus::Deleted, o("gone.txt"), None, false),
            (FileStatus::Added, None, o("empty"), false),
            (FileStatus::Copied, o("a.c"), o("b.c"), false),
        ]);
        assert_eq!(p.files[0].similarity, Some(90));
        assert_eq!(p.files[0].display_path(), "old name.txt → new name.txt");
        assert!(p.files[2].mode_changed() && p.files[2].hunks.is_empty());
        let added = &p.files[4].hunks[0];
        assert_eq!((added.old_start, added.old_count, added.lines[0].new, added.lines[1].new), (0, 0, 1, 2));
        assert_eq!(p.files[5].hunks[0].lines[0].old, 1);
        assert_eq!(p.totals(), (3, 2));
    }

    #[test]
    fn no_newline_markers_and_crlf() {
        let patch = "diff --git a/f b/f\r
--- a/f\r
+++ b/f\r
@@ -1,2 +1,2 @@\r
 keep\r
-old\r
\\ No newline at end of file\r
+new\r
\\ No newline at end of file\r
";
        let p = parse(patch).unwrap();
        let l = &p.files[0].hunks[0].lines;
        assert_eq!(l.len(), 3);
        assert!(!l[0].no_newline && l[1].no_newline && l[2].no_newline);
        assert_eq!((p.text(&l[0]), p.text(&l[2])), ("keep", "new"));
        let mut out = String::new();
        p.line_as_patch(&l[2], &mut out);
        assert_eq!(out, "+new\n\\ No newline at end of file\n");
    }

    #[test]
    fn quoted_paths_with_octal_escapes() {
        let patch = "diff --git \"a/caf\\303\\251 \\\"x\\\".txt\" \"b/caf\\303\\251 \\\"x\\\".txt\"
index 1..2 100644
--- \"a/caf\\303\\251 \\\"x\\\".txt\"
+++ \"b/caf\\303\\251 \\\"x\\\".txt\"
@@ -1 +1 @@
-a
+b
diff --git a/tab \"b/t\\tb\"
--- a/tab
+++ \"b/t\\tb\"
@@ -1 +1 @@
-a
+b
";
        let p = parse(patch).unwrap();
        assert_eq!(p.files[0].path(), "café \"x\".txt");
        assert_eq!(p.files[1].new_path.as_deref(), Some("t\tb"));
        assert_eq!(unquote("\"\\001\\\\\""), "\u{1}\\");
    }

    #[test]
    fn hunk_text_that_looks_like_headers_follows_the_counts() {
        // Removed "-- x" and added "++ y" lines must not end the hunk.
        let patch = "diff --git a/m.sql b/m.sql
--- a/m.sql
+++ b/m.sql
@@ -1,2 +1,2 @@ fn x
--- comment
+++ y
 diff --git a/z b/z
";
        let p = parse(patch).unwrap();
        let l = &p.files[0].hunks[0].lines;
        assert_eq!(l.len(), 3);
        assert_eq!((p.text(&l[0]), p.text(&l[1]), p.text(&l[2])), ("-- comment", "++ y", "diff --git a/z b/z"));
        assert_eq!(p.files.len(), 1);
    }

    #[test]
    fn format_patch_mail_and_signature_are_skipped() {
        let patch = "From 1234 Mon Sep 17 00:00:00 2001
From: A <a@b.c>
Subject: [PATCH] Fix

Body text
---
 f | 2 +-
 1 file changed, 1 insertion(+), 1 deletion(-)

diff --git a/f b/f
index 1..2 100644
--- a/f
+++ b/f
@@ -1 +1 @@
-x
+y
--
2.47.0

";
        let p = parse(patch).unwrap();
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.files[0].hunks[0].lines.len(), 2);
    }

    #[test]
    fn plain_unified_diff_with_timestamps_and_multiple_files() {
        let patch = "--- a/x.txt\t2024-01-01 10:00:00.000000000 +0100
+++ b/x.txt\t2024-01-02 10:00:00.000000000 +0100
@@ -1 +1,2 @@
 a
+b
--- y.txt\t2024-01-01
+++ y.txt\t2024-01-02
@@ -3,0 +4 @@ section
+c
";
        let p = parse(patch).unwrap();
        assert_eq!(p.files.len(), 2);
        assert_eq!(p.files[0].path(), "x.txt");
        assert_eq!(p.files[1].path(), "y.txt");
        let h = &p.files[1].hunks[0];
        assert_eq!((h.old_start, h.old_count, h.new_start, h.new_count, h.lines[0].new), (3, 0, 4, 1, 4));
    }

    #[test]
    fn binary_patch_blocks_are_skipped() {
        let patch = "diff --git a/i.png b/i.png
new file mode 100644
index 0000000000000000000000000000000000000000..1d2e5d4a1b8a5f5e3c2f2b1a9b1e2c3d4e5f6a7b
GIT binary patch
literal 12
TcmZ?wbhEHbRA6vmVBi1%2Lb>B

literal 0
HcmV?d00001

diff --git a/t b/t
--- a/t
+++ b/t
@@ -1 +1 @@
-a
+b
";
        let p = parse(patch).unwrap();
        assert_eq!(p.files.len(), 2);
        assert!(p.files[0].binary && p.files[0].status == FileStatus::Added);
        assert_eq!(p.files[1].hunks[0].lines.len(), 2);
    }

    #[test]
    fn limits_and_errors() {
        let one = MODIFIED;
        assert!(matches!(parse_with(one, Limits { max_bytes: 10, ..Limits::default() }), Err(DiffError::TooLarge { .. })));
        assert!(matches!(parse_with(one, Limits { max_lines: 3, ..Limits::default() }), Err(DiffError::TooManyLines { max: 3 })));
        let two = format!("{one}{}", one.replace("main.rs", "lib.rs"));
        assert!(matches!(parse_with(&two, Limits { max_files: 1, ..Limits::default() }), Err(DiffError::TooManyFiles { max: 1 })));
        assert!(matches!(parse_with(one, Limits { max_line_bytes: 20, ..Limits::default() }), Err(DiffError::LineTooLong { .. })));
        assert!(matches!(parse("diff --git a/x b/x\n@@ -a +b @@\n"), Err(DiffError::Malformed { line: 2, .. })));
        assert!(matches!(parse("diff --cc f\n"), Err(DiffError::Malformed { line: 1, .. })));
        assert!(parse("").unwrap().files.is_empty());
        assert!(parse("just text\n").unwrap().files.is_empty());
        assert!(DiffError::TooManyFiles { max: 256 }.to_string().contains("256"));
    }

    #[test]
    fn insert_context_above_below_and_merge() {
        let patch = "diff --git a/f b/f
--- a/f
+++ b/f
@@ -5,2 +5,2 @@
 e
-f
+F
@@ -10,1 +10,1 @@
-j
+J
";
        let mut p = parse(patch).unwrap();
        assert_eq!(p.gap_above(0, 0), Some((1, 4)));
        assert_eq!(p.gap_above(0, 1), Some((7, 9)));
        assert!(!p.insert_context(0, 0, 2, &["b".into()]), "does not border the hunk");
        assert!(p.insert_context(0, 0, 3, &["c".into(), "d".into()]));
        let h = &p.files[0].hunks[0];
        assert_eq!((h.old_start, h.old_count, h.new_start, h.new_count), (3, 4, 3, 4));
        assert_eq!((h.lines[0].old, h.lines[0].new, p.text(&h.lines[0])), (3, 3, "c"));
        assert_eq!(h.lines[3].pair(), Some(4), "pairs recomputed after the shift");
        assert_eq!(p.gap_above(0, 0), Some((1, 2)));
        // Below the first hunk: 7, 8 → then 9 closes the gap and merges.
        assert!(p.insert_context(0, 0, 7, &["g".into(), "h".into()]));
        assert_eq!(p.files[0].hunks.len(), 2);
        assert!(!p.insert_context(0, 0, 9, &["i".into(), "x".into()]), "would overlap the next hunk");
        assert!(p.insert_context(0, 1, 9, &["i".into()]));
        assert_eq!(p.files[0].hunks.len(), 1, "touching hunks merge");
        let h = &p.files[0].hunks[0];
        assert_eq!((h.old_start, h.old_count, h.new_start, h.new_count, h.lines.len()), (3, 8, 3, 8, 10));
        assert_eq!(h.lines[9].pair(), Some(8));
    }

    #[test]
    fn header_split_with_spaces() {
        assert_eq!(git_header_paths("a/x y.txt b/x y.txt"), Some(("x y.txt".into(), "x y.txt".into())));
        assert_eq!(git_header_paths("a/x b/y"), Some(("x".into(), "y".into())));
        assert_eq!(git_header_paths("x y"), Some(("x".into(), "y".into())));
        assert_eq!(parse_hunk_header("@@ -1 +1 @@"), Some((1, 1, 1, 1, "")));
        assert_eq!(parse_hunk_header("@@ -0,0 +1,3 @@ impl X {"), Some((0, 0, 1, 3, "impl X {")));
    }
}
