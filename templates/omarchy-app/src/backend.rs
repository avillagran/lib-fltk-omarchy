//! The app's one job, without any UI: the tally, its undo history and the
//! file it lives in. Everything that can fail is here and returns errors
//! the window and the commands can show.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The count with its undo and redo history.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    count: i64,
    saved: i64,
    undo: Vec<i64>,
    redo: Vec<i64>,
}

impl Tally {
    /// A tally that starts (and counts as saved) at `count`.
    pub fn new(count: i64) -> Self {
        Tally { count, saved: count, ..Default::default() }
    }

    pub fn count(&self) -> i64 {
        self.count
    }

    /// The main action: one more.
    pub fn add(&mut self) {
        self.undo.push(self.count);
        self.redo.clear();
        self.count += 1;
    }

    /// Goes back one step; false when there is nothing to undo.
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo.pop() else { return false };
        self.redo.push(self.count);
        self.count = prev;
        true
    }

    /// Goes forward again; false when there is nothing to redo.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo.pop() else { return false };
        self.undo.push(self.count);
        self.count = next;
        true
    }

    /// The count differs from the one on disk.
    pub fn unsaved(&self) -> bool {
        self.count != self.saved
    }

    pub fn mark_saved(&mut self) {
        self.saved = self.count;
    }
}

/// Where the tally is kept: the app's data directory
/// (`~/.local/share/{{name}}/tally.json` on Linux).
pub fn data_file() -> PathBuf {
    fltk_omarchy::platform::dirs::app_data(crate::NAME).join("tally.json")
}

/// Reads a saved count; a missing file is a new tally (0).
pub fn load(path: &Path) -> Result<i64, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    v.get("count")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| format!("{}: no whole \"count\"", path.display()))
}

/// Saves the count atomically.
pub fn save(path: &Path, count: i64) -> std::io::Result<()> {
    let json = serde_json::json!({ "count": count }).to_string();
    write_atomic(path, json.as_bytes())
}

/// The exported text: one line, for other tools to read.
pub fn export_text(count: i64) -> String {
    format!("{count}\n")
}

/// Writes beside the target and renames over it, so a crash never leaves
/// half a file and readers see the old or the new contents only.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// One finding of `{{name}} check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub error: bool,
    pub message: String,
}

/// Everything wrong with the saved tally at `path` (empty: fine).
pub fn check(path: &Path) -> Vec<Problem> {
    let mut out = Vec::new();
    match load(path) {
        Ok(n) if n < 0 => out.push(Problem { error: false, message: format!("negative count {n}") }),
        Ok(_) => {}
        Err(e) => out.push(Problem { error: true, message: e }),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory under this crate's target/ (never the real data).
    pub(crate) fn temp(tag: &str) -> PathBuf {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-tmp")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn counts_undoes_and_redoes() {
        let mut t = Tally::new(5);
        assert!(!t.unsaved());
        t.add();
        t.add();
        assert_eq!(t.count(), 7);
        assert!(t.unsaved());
        assert!(t.undo());
        assert_eq!(t.count(), 6);
        assert!(t.redo());
        assert_eq!(t.count(), 7);
        assert!(!t.redo());
        t.undo();
        t.add(); // a new action drops the redo history
        assert!(!t.redo());
        t.mark_saved();
        assert!(!t.unsaved());
        while t.undo() {}
        assert_eq!(t.count(), 5);
    }

    #[test]
    fn saves_atomically_and_loads() {
        let d = temp("save");
        let f = d.join("sub/tally.json");
        assert_eq!(load(&f), Ok(0));
        save(&f, 42).unwrap();
        assert_eq!(load(&f), Ok(42));
        save(&f, 43).unwrap();
        assert_eq!(load(&f), Ok(43));
        // No temporary file is left behind.
        let names: Vec<_> = std::fs::read_dir(d.join("sub")).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("tally.json")]);
    }

    #[test]
    fn reports_bad_files() {
        let d = temp("check");
        let f = d.join("tally.json");
        assert!(check(&f).is_empty());
        std::fs::write(&f, "{\"count\": \"many\"}").unwrap();
        let p = check(&f);
        assert!(p[0].error && p[0].message.contains("count"), "{p:?}");
        std::fs::write(&f, "not json").unwrap();
        assert!(load(&f).is_err());
        std::fs::write(&f, "{\"count\": -1}").unwrap();
        assert!(!check(&f)[0].error);
    }
}
