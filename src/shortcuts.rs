//! Command shortcut tables: keys bound to command ids, with conditions,
//! rebound at run time (o3-code's 82 editable keybindings and their
//! "when" clauses).
//!
//! Each [`AppWindow`](crate::AppWindow) has a table ([`AppWindow::shortcuts`](crate::AppWindow::shortcuts)).
//! A key the focused control does not use reaches the table; the first
//! binding whose keys match and whose condition holds runs the command
//! handler with its id. Bindings that should not fire again while the key
//! is held set `repeat(false)` (the default; next/previous style commands
//! set `repeat(true)`).
//!
//! ```no_run
//! use fltk_omarchy::{AppWindow, State};
//! # let _app = fltk_omarchy::App::new("dev.example.shortcuts");
//! let win = AppWindow::new("App", 800, 600);
//! let composer_open = State::new(false);
//! let table = win.shortcuts();
//! table.on_command(|cmd| println!("run {cmd}"));
//! table.bind("thread.next", "Mod+Down").unwrap().repeat(true);
//! let open = composer_open.clone();
//! table.bind("chat.new", "Mod+n").unwrap().when(move || !open.get());
//! // The user rebinds a command: replace its keys, keep the handler.
//! table.rebind("thread.next", &["Mod+j", "Alt+Down"]).unwrap();
//! ```
//!
//! Conditions are plain closures: read app state in them (a ported
//! when-clause evaluator, focus, the open view) and they stay current
//! without rebuilding the table. While [`keys::record`](crate::keys::record)
//! or a [`keys::Suspend`](crate::keys::Suspend) guard is active no
//! shortcut fires.

use crate::keys::{self, Keys};
use std::cell::RefCell;
use std::rc::Rc;

type When = Rc<dyn Fn() -> bool>;
type Handler = Rc<RefCell<Box<dyn FnMut(&str)>>>;

struct Binding {
    id: u64,
    command: String,
    spec: String,
    keys: Keys,
    when: Option<When>,
    repeat: bool,
}

#[derive(Default)]
struct Table {
    bindings: Vec<Binding>,
    handler: Option<Handler>,
    next_id: u64,
}

/// A window's command shortcut table. Cheap to clone (shared).
#[derive(Clone, Default)]
pub struct Shortcuts {
    t: Rc<RefCell<Table>>,
}

/// One binding, to set its condition and repeat after [`Shortcuts::bind`].
pub struct BindingRef {
    t: Rc<RefCell<Table>>,
    id: u64,
}

impl BindingRef {
    fn edit(&self, f: impl FnOnce(&mut Binding)) {
        if let Some(b) = self.t.borrow_mut().bindings.iter_mut().find(|b| b.id == self.id) {
            f(b);
        }
    }

    /// Fires only while `cond` returns true.
    pub fn when(self, cond: impl Fn() -> bool + 'static) -> Self {
        let c: When = Rc::new(cond);
        self.edit(|b| b.when = Some(c));
        self
    }

    /// Fires again on the automatic repeat of a held key (default false).
    pub fn repeat(self, on: bool) -> Self {
        self.edit(|b| b.repeat = on);
        self
    }
}

impl Shortcuts {
    pub fn new() -> Shortcuts {
        Shortcuts::default()
    }

    /// The handler that runs a command by id (replaces the previous one).
    pub fn on_command(&self, f: impl FnMut(&str) + 'static) {
        self.t.borrow_mut().handler = Some(Rc::new(RefCell::new(Box::new(f))));
    }

    /// Adds `spec` (see [`keys::parse`]) for `command`. A command can have
    /// several bindings; earlier bindings win when two match.
    pub fn bind(&self, command: &str, spec: &str) -> Result<BindingRef, String> {
        let k = keys::parse(spec)?;
        let mut t = self.t.borrow_mut();
        t.next_id += 1;
        let id = t.next_id;
        t.bindings.push(Binding { id, command: command.into(), spec: spec.into(), keys: k, when: None, repeat: false });
        drop(t);
        Ok(BindingRef { t: self.t.clone(), id })
    }

    /// Replaces the keys of `command` with `specs`, keeping the condition
    /// and repeat of its first binding. Nothing changes when a spec does
    /// not parse.
    pub fn rebind(&self, command: &str, specs: &[&str]) -> Result<(), String> {
        let parsed = specs.iter().map(|s| keys::parse(s).map(|k| (s.to_string(), k))).collect::<Result<Vec<_>, _>>()?;
        let mut t = self.t.borrow_mut();
        let (when, repeat) = t
            .bindings
            .iter()
            .find(|b| b.command == command)
            .map(|b| (b.when.clone(), b.repeat))
            .unwrap_or((None, false));
        let at = t.bindings.iter().position(|b| b.command == command).unwrap_or(t.bindings.len());
        t.bindings.retain(|b| b.command != command);
        for (i, (spec, k)) in parsed.into_iter().enumerate() {
            t.next_id += 1;
            let id = t.next_id;
            let at = (at + i).min(t.bindings.len());
            t.bindings.insert(at, Binding { id, command: command.into(), spec, keys: k, when: when.clone(), repeat });
        }
        Ok(())
    }

    /// Removes every binding of `command`.
    pub fn unbind(&self, command: &str) {
        self.t.borrow_mut().bindings.retain(|b| b.command != command);
    }

    /// Removes every binding (the handler stays).
    pub fn clear(&self) {
        self.t.borrow_mut().bindings.clear();
    }

    /// The specs bound to `command`, in order (for menus and the F1 help;
    /// show them with [`keys::label`]).
    pub fn specs(&self, command: &str) -> Vec<String> {
        self.t.borrow().bindings.iter().filter(|b| b.command == command).map(|b| b.spec.clone()).collect()
    }

    /// Commands other than `except` already bound to the same keys as
    /// `spec` (a keybinding editor's conflict warning). Conditions are
    /// not considered.
    pub fn conflicts(&self, spec: &str, except: &str) -> Vec<String> {
        let Ok(k) = keys::parse(spec) else { return Vec::new() };
        let mut out: Vec<String> = Vec::new();
        for b in self.t.borrow().bindings.iter() {
            if b.keys == k && b.command != except && !out.contains(&b.command) {
                out.push(b.command.clone());
            }
        }
        out
    }

    /// The command the current key event would run, if any.
    pub fn lookup(&self) -> Option<String> {
        if keys::suspended() {
            return None;
        }
        let repeat = keys::is_repeat();
        let candidates: Vec<(String, Option<When>, bool)> = self
            .t
            .borrow()
            .bindings
            .iter()
            .filter(|b| b.keys.matches_event())
            .map(|b| (b.command.clone(), b.when.clone(), b.repeat))
            .collect();
        // Conditions run without the table borrowed: they may read anything.
        for (cmd, when, rep) in candidates {
            if repeat && !rep {
                continue;
            }
            if when.as_ref().map_or(true, |w| w()) {
                return Some(cmd);
            }
        }
        None
    }

    /// Runs the command for the current key event; true when one ran.
    /// [`AppWindow`](crate::AppWindow) calls it for keys nothing else used.
    pub fn dispatch(&self) -> bool {
        let Some(cmd) = self.lookup() else { return false };
        self.run(&cmd)
    }

    /// Runs `command` through the handler (a menu or palette entry does the
    /// same as its shortcut); false without a handler.
    pub fn run(&self, command: &str) -> bool {
        let h = self.t.borrow().handler.clone();
        match h {
            Some(h) => {
                if let Ok(mut f) = h.try_borrow_mut() {
                    f(command);
                }
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_rebind_conflicts() {
        let t = Shortcuts::new();
        t.bind("a", "Mod+k").unwrap();
        t.bind("b", "Mod+k").unwrap();
        t.bind("c", "F5").unwrap();
        assert_eq!(t.conflicts("Mod+k", "a"), vec!["b".to_string()]);
        assert!(t.conflicts("Mod+x", "a").is_empty());
        t.rebind("a", &["Mod+j", "Alt+Down"]).unwrap();
        assert_eq!(t.specs("a"), vec!["Mod+j".to_string(), "Alt+Down".to_string()]);
        assert!(t.rebind("a", &["Hyper+q"]).is_err());
        assert_eq!(t.specs("a").len(), 2, "a bad spec changes nothing");
        t.unbind("b");
        assert!(t.specs("b").is_empty());
        assert!(t.bind("x", "Nope+z").is_err());
        let ran = Rc::new(RefCell::new(Vec::new()));
        let r = ran.clone();
        t.on_command(move |c| r.borrow_mut().push(c.to_string()));
        assert!(t.run("c"));
        assert_eq!(*ran.borrow(), vec!["c".to_string()]);
    }
}
