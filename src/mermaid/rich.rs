//! Native diagrams inside a [`RichText`](crate::text::RichText): closed
//! ```` ```mermaid ```` code blocks are drawn as diagrams under their code
//! header (language, copy button, "Show source"), cached by source.
//! Turned on with `RichText::native_mermaid(true)`.

use super::{render, Diagram, Error, Picture};
use crate::text::layout::{Layout, Typo};
use crate::text::BlockAction;
use crate::theme::{self, Rgb};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// The code header action that shows a diagram's source.
pub const SHOW_SOURCE: &str = "mermaid-source";
/// The code header action that shows the diagram again.
pub const SHOW_DIAGRAM: &str = "mermaid-diagram";

/// Most diagrams kept laid out per view (least recently used go first).
const MAX_ENTRIES: usize = 48;

/// Whether a code block's language names a Mermaid diagram.
pub fn is_mermaid(lang: &str) -> bool {
    lang.eq_ignore_ascii_case("mermaid")
}

struct Entry {
    /// (width, theme generation) it was laid out for.
    laid: (i32, u64),
    result: Result<Diagram, Error>,
    source: bool,
    pic: Picture,
    used: u64,
}

/// Per-view state: diagrams by source hash.
#[derive(Default)]
pub(crate) struct Native {
    entries: HashMap<u64, Entry>,
    clock: u64,
    /// Blocks laid out while their fence was still open (block, source hash).
    open: Vec<(usize, u64)>,
    pub(crate) layouts: u64,
}

fn hash(src: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    h.finish()
}

impl Native {
    fn entry(&mut self, src: &str, width: i32) -> &mut Entry {
        self.clock += 1;
        let key = hash(src);
        let laid = (width, theme::generation());
        if self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(&key) {
            if let Some(old) = self.entries.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| *k) {
                self.entries.remove(&old);
            }
        }
        let clock = self.clock;
        let mut fresh = false;
        let e = self.entries.entry(key).or_insert_with(|| {
            fresh = true;
            Entry { laid, result: render(src, width as f32), source: false, pic: Picture::new(), used: clock }
        });
        if fresh {
            self.layouts += 1;
        } else if e.laid != laid {
            e.result = render(src, width as f32);
            e.laid = laid;
            self.layouts += 1;
        }
        e.used = clock;
        e
    }

    /// Lays out code block `block` (source `src`): a closed fence that
    /// renders becomes a diagram under the code header. Returns the header
    /// action to add ("Show source" or "Show diagram").
    pub(crate) fn layout(&mut self, block: usize, src: &str, open: bool, ty: &Typo, l: &mut Layout) -> Option<BlockAction> {
        self.open.retain(|(b, _)| *b != block);
        if open {
            if self.open.len() < 16 {
                self.open.push((block, hash(src)));
            }
            return None;
        }
        let width = (l.box_w - 2 * ty.code_pad).max(60);
        let e = self.entry(src, width);
        let d = e.result.as_ref().ok()?;
        if e.source {
            return Some(BlockAction::new(SHOW_DIAGRAM, &crate::tr!("Show diagram")));
        }
        l.lines.clear();
        l.frags.clear();
        l.height = ty.code_header + d.height.ceil() as i32 + ty.code_pad;
        Some(BlockAction::new(SHOW_SOURCE, &crate::tr!("Show source")))
    }

    /// Blocks laid out with an open fence whose fence is now closed
    /// (`still_open(block)` says which are not).
    pub(crate) fn closed(&mut self, mut still_open: impl FnMut(usize) -> bool) -> Vec<usize> {
        let mut out = Vec::new();
        self.open.retain(|&(b, _)| {
            let open = still_open(b);
            if !open {
                out.push(b);
            }
            open
        });
        out
    }

    /// Flips between the diagram and its source.
    pub(crate) fn toggle(&mut self, src: &str) {
        if let Some(e) = self.entries.get_mut(&hash(src)) {
            e.source = !e.source;
        }
    }

    /// Paints the diagram of `src` for a code block whose surface is at
    /// (x, y), `w` wide.
    pub(crate) fn paint(&mut self, src: &str, x: i32, y: i32, w: i32, ty: &Typo, unit: f32, under: Rgb) {
        let Some(e) = self.entries.get_mut(&hash(src)) else { return };
        let Ok(d) = &e.result else { return };
        let inner = w - 2 * ty.code_pad;
        let dx = (inner as f32 - d.width).max(0.0) as i32 / 2;
        e.pic.draw(d, x + ty.code_pad + dx, y + ty.code_header + ty.code_pad / 2, unit, under);
    }

    /// The diagram shown for `src`, if it rendered.
    pub(crate) fn diagram(&self, src: &str) -> Option<&Diagram> {
        self.entries.get(&hash(src)).and_then(|e| e.result.as_ref().ok())
    }
}
