//! Starts a new Omarchy app from `templates/omarchy-app`, the Rust version
//! of Omarchy's omarchy-app skill (skills/omarchy-fltk-app/SKILL.md):
//!
//! ```sh
//! cargo run --example new_app -- <name> [dir] [--path] [--no-git] [--title "Display Name"]
//! ```
//!
//! `<name>` is short and lowercase (`tally`, `pomo`): the binary, package,
//! `.desktop` file, icon and Wayland app id. `dir` defaults to
//! `~/Work/<name>`. The app depends on this toolkit by git revision (this
//! checkout's HEAD), or by path with `--path`. Existing files are never
//! overwritten: they are listed and kept.

use std::path::{Path, PathBuf};
use std::process::Command;

const TEMPLATE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/templates/omarchy-app");
const REPO: &str = "https://github.com/avillagran/lib-fltk-omarchy";

fn usage() -> ! {
    eprintln!("usage: cargo run --example new_app -- <name> [dir] [--path] [--no-git] [--title \"Display Name\"]");
    std::process::exit(2);
}

fn valid_name(n: &str) -> bool {
    let b = n.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && !n.ends_with('-')
}

/// `tally` → `Tally`, `my-app` → `My App`.
fn display_name(n: &str) -> String {
    n.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

/// The civil year of a Unix time (Howard Hinnant's days-to-civil).
fn year(unix: u64) -> i64 {
    let z = (unix / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    yoe + era * 400 + i64::from(month <= 2)
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).expect("template directory").flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            files(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let mut words = Vec::new();
    let (mut by_path, mut init_git, mut title) = (false, true, None);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--path" => by_path = true,
            "--no-git" => init_git = false,
            "--title" => title = Some(args.next().unwrap_or_else(|| usage())),
            "-h" | "--help" => usage(),
            s if s.starts_with("--") => usage(),
            _ => words.push(a),
        }
    }
    let Some(name) = words.first().cloned() else { usage() };
    if !valid_name(&name) {
        eprintln!("{name:?}: use a short lowercase name (letters, digits, '-'), like tally or pomo");
        std::process::exit(2);
    }
    let dir = words
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| fltk_omarchy::platform::dirs::home().join("Work").join(&name));
    let title = title.unwrap_or_else(|| display_name(&name));
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let toolkit = if by_path {
        let p = manifest.canonicalize().unwrap_or_else(|_| manifest.to_path_buf());
        format!("path = {:?}", p.display().to_string())
    } else if let Some(rev) = git(manifest, &["rev-parse", "HEAD"]) {
        format!("git = \"{REPO}\", rev = \"{rev}\"")
    } else {
        format!("git = \"{REPO}\", tag = \"v{}\"", env!("CARGO_PKG_VERSION"))
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let author = git(manifest, &["config", "user.name"]).unwrap_or_else(|| format!("The {title} authors"));
    let vars: [(&str, String); 6] = [
        ("{{name}}", name.clone()),
        ("{{Name}}", title.clone()),
        ("{{crate}}", name.replace('-', "_")),
        ("{{toolkit}}", toolkit),
        ("{{year}}", year(now).to_string()),
        ("{{author}}", author),
    ];
    let fill = |s: &str| vars.iter().fold(s.to_string(), |acc, (k, v)| acc.replace(k, v));

    let template = Path::new(TEMPLATE);
    let mut list = Vec::new();
    files(template, &mut list);
    let (mut written, mut kept) = (0, Vec::new());
    for src in list {
        let rel = src.strip_prefix(template).unwrap().to_string_lossy().into_owned();
        let rel = fill(&rel);
        let rel = rel.strip_suffix(".in").unwrap_or(&rel).to_string();
        let dst = dir.join(&rel);
        if dst.exists() {
            kept.push(rel);
            continue;
        }
        std::fs::create_dir_all(dst.parent().unwrap()).expect("create directory");
        let bytes = std::fs::read(&src).expect("read template");
        match String::from_utf8(bytes) {
            Ok(text) => std::fs::write(&dst, fill(&text)),
            Err(e) => std::fs::write(&dst, e.into_bytes()),
        }
        .unwrap_or_else(|e| panic!("{}: {e}", dst.display()));
        // Keep the executable bits of bin/*.
        if let Ok(m) = std::fs::metadata(&src) {
            let _ = std::fs::set_permissions(&dst, m.permissions());
        }
        written += 1;
    }
    println!("{title} ({name}): {written} files in {}", dir.display());
    for k in &kept {
        println!("  kept existing {k}");
    }
    if init_git && !dir.join(".git").exists() && Command::new("git").arg("-C").arg(&dir).args(["init", "-q"]).status().is_ok_and(|s| s.success()) {
        println!("  git init");
    }
    println!("\nNext:\n  cd {}\n  bin/build && bin/test\n  cargo run              # try it\n  bin/install            # into the app launcher", dir.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("tally") && valid_name("pomo2") && valid_name("my-app"));
        assert!(!valid_name("Tally") && !valid_name("2x") && !valid_name("a_b") && !valid_name("x-") && !valid_name(""));
        assert_eq!(display_name("my-app"), "My App");
        assert_eq!(year(0), 1970);
        assert_eq!(year(1_798_761_600), 2027); // 2027-01-01
        assert_eq!(year(1_798_761_599), 2026);
    }
}
