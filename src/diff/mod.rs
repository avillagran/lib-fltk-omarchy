//! Native diff viewer: a unified-diff parser and virtualized views.
//!
//! - [`parse`] turns `git diff` output into a [`Patch`] (files, hunks,
//!   lines with old/new numbers) within o3-code's limits.
//! - [`DiffView`] draws a patch, unified or side by side, one canvas and
//!   only the rows in view.
//! - [`DiffFiles`] is the file tree next to it (a toolkit `Tree`).
//!
//! Guide: docs/DIFF.md.

mod parse;
mod rows;

pub use parse::{parse, parse_with, unquote, DiffError, FileDiff, FileStatus, Hunk, Line, LineKind, Limits, Patch};
