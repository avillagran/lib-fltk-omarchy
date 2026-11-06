//! [`DiffFiles`]: the changed files of a [`DiffView`] as a toolkit
//! [`Tree`] (folders, then files with their status and +/− counts).

use super::view::DiffView;
use super::Patch;
use crate::panel::{ItemId, KeyboardPanel};
use crate::table::{Tree, TreeNode};
use crate::widgets::glyph;
use crate::{a11y, tr};
use fltk::{prelude::*, widget::Widget};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Folder IDs have this bit; file IDs are the file index.
const DIR: ItemId = 1 << 62;

type Nodes = HashMap<Option<ItemId>, Vec<TreeNode>>;

/// The file tree of a diff. Moving the cursor (arrows, click) shows that
/// file in the view; Enter moves the focus into the diff. The cursor
/// follows the view's current file (Mod+Left/Right there).
#[derive(Clone)]
pub struct DiffFiles {
    widget: Widget,
    tree: Tree,
    panel: KeyboardPanel,
}
fltk::widget_extends!(DiffFiles, Widget, widget);

impl DiffFiles {
    pub fn new(view: &DiffView) -> DiffFiles {
        let nodes: Rc<RefCell<Nodes>> = Rc::default();
        let panel = KeyboardPanel::new("diff-files");
        let n2 = nodes.clone();
        let tree = Tree::new(&panel, move |parent| n2.borrow().get(&parent).cloned().unwrap_or_default());
        let widget = (*tree).clone();
        a11y::set_name(&widget, &tr!("Changed files"));
        let files = DiffFiles { widget, tree, panel: panel.clone() };
        let syncing = Rc::new(Cell::new(false));
        // Tree -> view. Callbacks hold the view weakly (no Rc cycle).
        let (v, sy) = (view.downgrade(), syncing.clone());
        let last = Cell::new(None::<ItemId>);
        let p2 = panel.downgrade();
        panel.on_change(move || {
            let (Some(p), Some(v)) = (p2.upgrade(), v.upgrade()) else { return };
            let cur = p.cursor();
            if sy.get() || cur == last.get() {
                return;
            }
            last.set(cur);
            if let Some(id) = cur.filter(|id| id & DIR == 0) {
                sy.set(true);
                v.show_file(id as usize);
                sy.set(false);
            }
        });
        let v = view.downgrade();
        panel.on_activate(move |id| {
            if let Some(v) = v.upgrade().filter(|_| id & DIR == 0) {
                v.show_file(id as usize);
                v.panel().focus();
            }
        });
        // View -> tree.
        let (p3, sy) = (panel.downgrade(), syncing.clone());
        let mut w = files.widget.clone();
        let _ = view.clone().on_file_change(move |file| {
            let Some(p) = p3.upgrade() else { return };
            if sy.get() || w.was_deleted() {
                return;
            }
            sy.set(true);
            p.select_only(file as ItemId);
            sy.set(false);
            w.redraw();
        });
        let (v, tree) = (view.downgrade(), files.tree.clone());
        let refresh = move || {
            let Some(v) = v.upgrade() else { return };
            let dirs = v.with_patch(|p| {
                let (map, dirs) = build(p);
                *nodes.borrow_mut() = map;
                dirs
            });
            tree.refresh();
            for d in dirs {
                tree.expand(d);
            }
        };
        refresh();
        view.on_patch(refresh);
        files
    }

    /// The tree (scroll offset, rows drawn, expand/collapse).
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    /// The tree's keyboard panel (cursor on the file index; folders have bit 62).
    pub fn panel(&self) -> KeyboardPanel {
        self.panel.clone()
    }
}

#[derive(Default)]
struct Dir {
    name: String,
    dirs: Vec<Dir>,
    files: Vec<usize>,
}

impl Dir {
    fn insert(&mut self, parts: &[&str], file: usize) {
        match parts {
            [] | [_] => self.files.push(file),
            [d, rest @ ..] => {
                let i = match self.dirs.iter().position(|x| x.name == *d) {
                    Some(i) => i,
                    None => {
                        self.dirs.push(Dir { name: d.to_string(), ..Default::default() });
                        self.dirs.len() - 1
                    }
                };
                self.dirs[i].insert(rest, file);
            }
        }
    }

    /// Folds folders that only hold one folder into "a/b".
    fn compress(&mut self) {
        for d in &mut self.dirs {
            d.compress();
            while d.files.is_empty() && d.dirs.len() == 1 {
                let only = d.dirs.pop().unwrap();
                d.name = format!("{}/{}", d.name, only.name);
                d.dirs = only.dirs;
                d.files = only.files;
            }
        }
    }
}

/// Tree nodes by parent and the folder IDs (to expand them).
fn build(p: &Patch) -> (Nodes, Vec<ItemId>) {
    let mut root = Dir::default();
    for (i, f) in p.files.iter().enumerate() {
        let parts: Vec<&str> = f.path().split('/').filter(|s| !s.is_empty()).collect();
        root.insert(&parts, i);
    }
    root.compress();
    let mut nodes = Nodes::new();
    let mut dirs = Vec::new();
    fn walk(d: &Dir, parent: Option<ItemId>, p: &Patch, nodes: &mut Nodes, dirs: &mut Vec<ItemId>) {
        let mut out = Vec::new();
        for sub in &d.dirs {
            let id = DIR | dirs.len() as ItemId;
            dirs.push(id);
            out.push(TreeNode::new(id, &sub.name).icon(glyph::FOLDER).branch());
            walk(sub, Some(id), p, nodes, dirs);
        }
        for &i in &d.files {
            let f = &p.files[i];
            let name = f.path().rsplit('/').next().unwrap_or("");
            let label = match (&f.old_path, f.status) {
                (Some(o), super::FileStatus::Renamed | super::FileStatus::Copied) if o != f.path() => format!("{name} ← {}", o.rsplit('/').next().unwrap_or(o)),
                _ => name.to_string(),
            };
            let detail = if f.binary { format!("{}  ·", f.status.letter()) } else { format!("{}  +{} −{}", f.status.letter(), f.additions, f.deletions) };
            out.push(TreeNode::new(i as ItemId, &label).icon(glyph::FILE).detail(&detail));
        }
        nodes.insert(parent, out);
    }
    walk(&root, None, p, &mut nodes, &mut dirs);
    (nodes, dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_folders_compress_and_order() {
        let patch = "diff --git a/src/diff/view.rs b/src/diff/view.rs
--- a/src/diff/view.rs
+++ b/src/diff/view.rs
@@ -1 +1 @@
-a
+b
diff --git a/src/diff/rows.rs b/src/diff/rows.rs
--- a/src/diff/rows.rs
+++ b/src/diff/rows.rs
@@ -1 +1,2 @@
 a
+b
diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-x
+y
";
        let p = super::super::parse(patch).unwrap();
        let (nodes, dirs) = build(&p);
        assert_eq!(dirs.len(), 1, "src/diff is one folder");
        let roots: Vec<_> = nodes[&None].iter().map(|n| (n.label.clone(), n.detail.clone())).collect();
        assert_eq!(roots, vec![("src/diff".to_string(), String::new()), ("README.md".into(), "M  +1 −1".into())]);
        let kids: Vec<_> = nodes[&Some(dirs[0])].iter().map(|n| (n.id, n.label.clone())).collect();
        assert_eq!(kids, vec![(0, "view.rs".to_string()), (1, "rows.rs".into())]);
    }
}
