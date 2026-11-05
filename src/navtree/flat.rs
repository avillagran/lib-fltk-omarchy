//! The rows a `NavTree` shows, flattened from the model's parent keys:
//! folded branches left out, search filtering (ancestors of matches kept,
//! siblings ordered by their best match), and the cumulative heights at
//! rail width and expanded (a row's height in between is interpolated,
//! so the y of any row is O(1) and the first row in view a binary search).

use super::item::{Match, NavItem, NavKind};
use crate::model::ListModel;
use crate::panel::ItemId;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::ops::Range;

/// A row in view order.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FRow {
    pub id: ItemId,
    /// Tree depth (a11y level - 1) and visual indentation steps (sections
    /// do not indent their rows).
    pub depth: u16,
    pub indent: u16,
    pub kind: NavKind,
    pub parent: Option<ItemId>,
    /// Folds (has children, or a collapsible section) and is open.
    pub folds: bool,
    pub open: bool,
    /// Position among its shown siblings (1-based) and their count.
    pub pos: u32,
    pub of: u32,
    /// Height at rail width and expanded.
    pub h0: i32,
    pub h1: i32,
    pub focusable: bool,
}

#[derive(Default)]
pub(crate) struct Flat {
    /// Scrolling rows, then the footer rows (bottom, fixed).
    pub top: Vec<FRow>,
    pub foot: Vec<FRow>,
    y0: Vec<i32>,
    y1: Vec<i32>,
    f0: Vec<i32>,
    f1: Vec<i32>,
    /// Where each shown row is: (footer, index).
    pub at: HashMap<ItemId, (bool, usize)>,
    /// What decides the shape of each item's row (see `signature`).
    pub sig: HashMap<ItemId, u64>,
    /// Title ranges to highlight (search).
    pub hits: HashMap<ItemId, Vec<Range<usize>>>,
    /// Items the search matched (not their ancestors).
    pub matched: HashSet<ItemId>,
    pub searching: bool,
}

fn lerp(a: i32, b: i32, r: f64) -> i32 {
    (a as f64 + (b - a) as f64 * r).round() as i32
}

impl Flat {
    /// y of top row `i` (content coordinates) at reveal `r`; `i == len`
    /// is the content height.
    pub fn top_y(&self, i: usize, r: f64) -> i32 {
        lerp(self.y0[i], self.y1[i], r)
    }
    pub fn content(&self, r: f64) -> i32 {
        self.top_y(self.top.len(), r)
    }
    /// y of footer row `i` from the start of the footer block.
    pub fn foot_y(&self, i: usize, r: f64) -> i32 {
        lerp(self.f0[i], self.f1[i], r)
    }
    pub fn foot_h(&self, r: f64) -> i32 {
        self.foot_y(self.foot.len(), r)
    }
    /// First top row whose bottom is below `y` (content coordinates).
    pub fn first_at(&self, y: i32, r: f64) -> usize {
        let (mut lo, mut hi) = (0usize, self.top.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.top_y(mid + 1, r) <= y {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
    pub fn row(&self, foot: bool, i: usize) -> &FRow {
        if foot {
            &self.foot[i]
        } else {
            &self.top[i]
        }
    }
    pub fn get(&self, id: ItemId) -> Option<(bool, usize, &FRow)> {
        let &(f, i) = self.at.get(&id)?;
        Some((f, i, self.row(f, i)))
    }
    /// Rows in screen order (top, then footer).
    pub fn order(&self) -> impl Iterator<Item = &FRow> {
        self.top.iter().chain(self.foot.iter())
    }
}

/// What makes a row change place or size: an update that keeps it only
/// needs a repaint of that row.
pub(crate) fn signature(it: &NavItem, searching: bool) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (
        &it.parent,
        it.kind as u8,
        it.footer,
        it.collapsible,
        it.disabled,
        it.detail.is_empty(),
        it.usage.is_some(),
        &it.renderer,
    )
        .hash(&mut h);
    if searching {
        (&it.title, &it.detail).hash(&mut h);
    }
    h.finish()
}

pub(crate) type Matcher = dyn Fn(&NavItem, &str) -> Option<Match>;

/// Flattens `model`. `collapsed` holds folded keys (items with
/// `collapsed: true` are added the first time `seen` meets them);
/// `height` gives (rail, expanded) heights.
pub(crate) fn flatten(
    model: &ListModel<String, NavItem>,
    collapsed: &mut HashSet<String>,
    seen: &mut HashSet<String>,
    query: &str,
    matcher: &Matcher,
    height: &dyn Fn(&NavItem) -> (i32, i32),
) -> Flat {
    struct Info {
        key: String,
        parent: String,
        kind: NavKind,
        footer: bool,
        collapsible: bool,
        disabled: bool,
        h: (i32, i32),
    }
    let n = model.len();
    let searching = !query.trim().is_empty();
    let ids: Vec<ItemId> = (0..n).map(|i| model.id_at(i).unwrap_or(0)).collect();
    let mut info = Vec::with_capacity(n);
    let mut sig = HashMap::with_capacity(n);
    let mut score: Vec<Option<i32>> = Vec::new();
    let mut hits = HashMap::new();
    let mut matched = HashSet::new();
    model.for_each(|i, it| {
        if !seen.contains(&it.key) {
            seen.insert(it.key.clone());
            if it.collapsed {
                collapsed.insert(it.key.clone());
            }
        }
        sig.insert(ids[i], signature(it, searching));
        if searching {
            let m = matcher(it, query);
            if m.is_some() {
                matched.insert(ids[i]);
            }
            if let Some(m) = &m {
                if !m.ranges.is_empty() {
                    hits.insert(ids[i], m.ranges.clone());
                }
            }
            score.push(m.map(|m| m.score));
        }
        info.push(Info {
            key: it.key.clone(),
            parent: it.parent.clone(),
            kind: it.kind,
            footer: it.footer,
            collapsible: it.collapsible,
            disabled: it.disabled,
            h: height(it),
        });
    });
    let index: HashMap<&str, usize> = info
        .iter()
        .enumerate()
        .map(|(i, x)| (x.key.as_str(), i))
        .collect();
    let parent_of: Vec<Option<usize>> = info
        .iter()
        .enumerate()
        .map(|(i, x)| {
            if x.parent.is_empty() {
                return None;
            }
            index.get(x.parent.as_str()).copied().filter(|&p| p != i)
        })
        .collect();
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots = Vec::new();
    for (i, p) in parent_of.iter().enumerate() {
        match *p {
            Some(p) => kids[p].push(i),
            None => roots.push(i),
        }
    }
    // Search: keep matches and their ancestors; best score of each subtree.
    let mut best: Vec<Option<i32>> = vec![None; n];
    if searching {
        for (i, s) in score.iter().enumerate() {
            let Some(s) = *s else { continue };
            let mut cur = Some(i);
            let mut guard = 0;
            while let Some(c) = cur {
                if best[c].is_some_and(|b| b >= s) || guard > 64 {
                    break;
                }
                best[c] = Some(s);
                cur = parent_of[c];
                guard += 1;
            }
        }
        let by_best = |v: &mut Vec<usize>| {
            v.retain(|&i| best[i].is_some());
            v.sort_by_key(|&i| std::cmp::Reverse(best[i].unwrap_or(0)));
        };
        by_best(&mut roots);
        for k in kids.iter_mut() {
            by_best(k);
        }
    }
    let mut flat = Flat {
        searching,
        ..Default::default()
    };
    let mut visited = vec![false; n];
    #[allow(clippy::too_many_arguments)]
    fn visit(
        i: usize,
        depth: u16,
        indent: u16,
        parent: Option<ItemId>,
        (pos, of): (u32, u32),
        out: &mut Vec<FRow>,
        ctx: &Ctx,
        visited: &mut Vec<bool>,
    ) {
        if visited[i] || depth > 64 {
            return;
        }
        visited[i] = true;
        let x = &ctx.info[i];
        if ctx.searching && x.kind == NavKind::Separator {
            return;
        }
        let kids = &ctx.kids[i];
        let folds = match x.kind {
            NavKind::Row => !kids.is_empty(),
            NavKind::Section => x.collapsible,
            NavKind::Separator => false,
        };
        let open = ctx.searching || !folds || !ctx.collapsed.contains(&x.key);
        out.push(FRow {
            id: ctx.ids[i],
            depth,
            indent,
            kind: x.kind,
            parent,
            folds,
            open,
            pos,
            of,
            h0: x.h.0,
            h1: x.h.1,
            focusable: match x.kind {
                NavKind::Row => !x.disabled,
                NavKind::Section => folds,
                NavKind::Separator => false,
            },
        });
        if open {
            let step = if x.kind == NavKind::Row { 1 } else { 0 };
            let shown: Vec<usize> = kids
                .iter()
                .copied()
                .filter(|&k| !(ctx.searching && ctx.info[k].kind == NavKind::Separator))
                .collect();
            let of = shown.len() as u32;
            for (p, k) in shown.into_iter().enumerate() {
                visit(
                    k,
                    depth + 1,
                    indent + step,
                    Some(ctx.ids[i]),
                    (p as u32 + 1, of),
                    out,
                    ctx,
                    visited,
                );
            }
        }
    }
    struct Ctx<'a> {
        info: &'a [Info],
        kids: &'a [Vec<usize>],
        ids: &'a [ItemId],
        collapsed: &'a HashSet<String>,
        searching: bool,
    }
    let ctx = Ctx {
        info: &info,
        kids: &kids,
        ids: &ids,
        collapsed,
        searching,
    };
    for footer in [false, true] {
        let list: Vec<usize> = roots
            .iter()
            .copied()
            .filter(|&i| info[i].footer == footer)
            .filter(|&i| !(searching && info[i].kind == NavKind::Separator))
            .collect();
        let of = list.len() as u32;
        let mut out = Vec::new();
        for (p, i) in list.into_iter().enumerate() {
            visit(
                i,
                0,
                0,
                None,
                (p as u32 + 1, of),
                &mut out,
                &ctx,
                &mut visited,
            );
        }
        if footer {
            flat.foot = out;
        } else {
            flat.top = out;
        }
    }
    let sums = |rows: &[FRow]| {
        let (mut a, mut b) = (vec![0], vec![0]);
        for r in rows {
            a.push(a.last().unwrap() + r.h0);
            b.push(b.last().unwrap() + r.h1);
        }
        (a, b)
    };
    (flat.y0, flat.y1) = sums(&flat.top);
    (flat.f0, flat.f1) = sums(&flat.foot);
    for (i, r) in flat.top.iter().enumerate() {
        flat.at.insert(r.id, (false, i));
    }
    for (i, r) in flat.foot.iter().enumerate() {
        flat.at.insert(r.id, (true, i));
    }
    flat.sig = sig;
    flat.hits = hits;
    flat.matched = matched;
    flat
}

#[cfg(test)]
mod tests {
    use super::super::item::default_match;
    use super::*;

    fn model() -> ListModel<String, NavItem> {
        let m = ListModel::new(|t: &NavItem| t.key.clone());
        m.extend([
            NavItem::section("s", "Projects").collapsible(),
            NavItem::row("a", "", "Alpha").parent("s"),
            NavItem::row("a1", "", "Alpha child").parent("a"),
            NavItem::row("a2", "", "Child beta").parent("a"),
            NavItem::row("b", "", "Beta").parent("s"),
            NavItem::row("f", "", "Settings").footer(),
            NavItem::row("o", "", "Orphan").parent("missing"),
        ]);
        m
    }

    fn keys(m: &ListModel<String, NavItem>, f: &Flat) -> Vec<String> {
        f.order()
            .map(|r| m.with_id(r.id, |t| t.key.clone()).unwrap())
            .collect()
    }

    #[test]
    fn tree_order_folds_footer_and_orphans() {
        let m = model();
        let (mut c, mut seen) = (HashSet::new(), HashSet::new());
        let h = |_: &NavItem| (10, 20);
        let f = flatten(&m, &mut c, &mut seen, "", &default_match, &h);
        assert_eq!(keys(&m, &f), ["s", "a", "a1", "a2", "b", "o", "f"]);
        let a = f.top[1];
        assert!(a.folds && a.open && a.depth == 1 && a.indent == 0);
        assert_eq!(
            (f.top[2].depth, f.top[2].indent, f.top[2].pos, f.top[2].of),
            (2, 1, 1, 2)
        );
        assert_eq!(f.content(0.0), 60);
        assert_eq!(f.content(1.0), 120);
        assert_eq!(f.first_at(25, 1.0), 1);
        c.insert("a".into());
        let f = flatten(&m, &mut c, &mut seen, "", &default_match, &h);
        assert_eq!(keys(&m, &f), ["s", "a", "b", "o", "f"]);
        c.insert("s".into());
        let f = flatten(&m, &mut c, &mut seen, "", &default_match, &h);
        assert_eq!(keys(&m, &f), ["s", "o", "f"]);
    }

    #[test]
    fn search_keeps_ancestors_and_orders_by_score() {
        let m = model();
        let (mut c, mut seen) = (HashSet::new(), HashSet::new());
        c.insert("s".into());
        let h = |_: &NavItem| (10, 20);
        let f = flatten(&m, &mut c, &mut seen, "beta", &default_match, &h);
        // "Beta" (start of the title) ranks above "Child beta" (a word start).
        assert_eq!(keys(&m, &f), ["s", "b", "a", "a2"]);
        assert!(f.searching);
        let b = m.id_of(&"b".to_string()).unwrap();
        let want: Vec<Range<usize>> = std::iter::once(0..4).collect();
        assert_eq!(f.hits.get(&b), Some(&want));
    }
}
