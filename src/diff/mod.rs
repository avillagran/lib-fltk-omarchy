//! Native diff viewer: a unified-diff parser and virtualized views.
//!
//! - [`parse`] turns `git diff` output into a [`Patch`] (files, hunks,
//!   lines with old/new numbers) within o3-code's limits.
//! - [`DiffView`] draws a patch, unified or side by side, one canvas and
//!   only the rows in view.
//! - [`DiffFiles`] is the file tree next to it (a toolkit `Tree`).
//! - Review: lines by [`LineRef`] (side and number), [`LineRange`]
//!   selections, `on_comment`, and [`Annotation`]s under lines (painted or
//!   hosting a widget).
//!
//! ```
//! use fltk_omarchy::diff::{self, FileStatus, LineKind};
//! let patch = diff::parse("diff --git a/a.txt b/a.txt
//! --- a/a.txt
//! +++ b/a.txt
//! @@ -1,2 +1,2 @@ intro
//!  keep
//! -old
//! +new
//! ").unwrap();
//! let f = &patch.files[0];
//! assert_eq!((f.path(), f.status, f.additions, f.deletions), ("a.txt", FileStatus::Modified, 1, 1));
//! let added = &f.hunks[0].lines[2];
//! assert_eq!((added.kind, added.new, patch.text(added)), (LineKind::Added, 2, "new"));
//! ```
//!
//! ```no_run
//! use fltk::prelude::*;
//! use fltk_omarchy::diff::{DiffFiles, DiffView};
//! use fltk_omarchy::{layout, split::Split};
//! # let _app = fltk_omarchy::App::new("dev.example.diff");
//! # let text = String::new();
//! let view = DiffView::new().split(true).on_expand(|req| println!("read {}:{}-{}", req.path, req.first, req.last));
//! view.set_text(&text).expect("a unified diff");
//! let mut row = layout::row();
//! let files = DiffFiles::new(&view);
//! row.add(&*files);
//! let _grip = Split::new(&mut row, &*files).range(160, 420).initial(240);
//! row.add(&*view);
//! row.end();
//! ```
//!
//! Guide: docs/DIFF.md.

mod files;
mod parse;
mod review;
mod rows;
mod view;

pub use files::DiffFiles;
pub use parse::{parse, parse_with, unquote, DiffError, FileDiff, FileStatus, Hunk, Line, LineKind, Limits, Patch};
pub use review::{Annotation, AnnotationPaint, LineRange, LineRef, Side};
pub use view::{DiffView, ExpandRequest};
