//! Commands for agents and scripts, as Omarchy's apps have them (Hype's
//! `hype check`, `hype render`): no window, `--json` for machine-readable
//! output, exit status 0 on success and 1 with the error on stderr.

use crate::backend::{self, Tally};
use crate::{NAME, VERSION};
use fltk::prelude::WidgetExt;
use serde_json::json;
use std::path::{Path, PathBuf};

/// The agent skill bundled in the binary (`{{name}} skill`).
pub const SKILL: &str = include_str!("../skill/SKILL.md");

pub fn usage() -> String {
    format!(
        "Usage: {NAME} [command] [options]\n\n\
         Window:\n  \
           open                      Open the window (the default)\n\n\
         Commands that need no window:\n  \
           check [file] [--json]     Report problems in the saved tally\n  \
           render [--out f.png] [--count n] [--lang code] [--json]\n                            \
         Draw the window to a PNG (needs a display, e.g. xvfb-run)\n  \
           skill [install]           Print the agent skill, or install it\n  \
           version                   Print the version\n  \
           help                      This help"
    )
}

fn fail(msg: impl std::fmt::Display) -> i32 {
    eprintln!("{NAME}: {msg}");
    1
}

fn print_json(v: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

/// Runs `args` (without the program name); returns the exit status.
pub fn run(args: &[String]) -> i32 {
    let cmd = args.first().map(String::as_str).unwrap_or("open");
    let rest = args.get(1..).unwrap_or_default();
    match cmd {
        "open" => crate::ui::open(),
        "check" => check(rest),
        "render" => render(rest),
        "skill" => skill(rest, &fltk_omarchy::platform::dirs::home()),
        "version" | "--version" => {
            println!("{NAME} {VERSION}");
            0
        }
        "help" | "--help" | "-h" => {
            println!("{}", usage());
            0
        }
        other => {
            eprintln!("{NAME}: unknown command {other:?}\n\n{}", usage());
            1
        }
    }
}

/// Splits `--json` and `--key value` options from positional words.
struct Opts {
    json: bool,
    values: Vec<(String, String)>,
    words: Vec<String>,
}

fn opts(args: &[String], with_value: &[&str]) -> Result<Opts, String> {
    let mut o = Opts { json: false, values: Vec::new(), words: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--json" {
            o.json = true;
        } else if with_value.contains(&a.as_str()) {
            let v = it.next().ok_or(format!("{a} needs a value"))?;
            o.values.push((a.clone(), v.clone()));
        } else if a.starts_with("--") {
            return Err(format!("unknown option {a}"));
        } else {
            o.words.push(a.clone());
        }
    }
    Ok(o)
}

impl Opts {
    fn value(&self, key: &str) -> Option<&str> {
        self.values.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// `check [file] [--json]`: problems in the saved tally (default: the
/// app's own file).
pub fn check(args: &[String]) -> i32 {
    let o = match opts(args, &[]) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let file = o.words.first().map(PathBuf::from).unwrap_or_else(backend::data_file);
    let problems = backend::check(&file);
    let count = backend::load(&file).ok();
    let errors = problems.iter().any(|p| p.error);
    if o.json {
        print_json(&json!({
            "file": file,
            "count": count,
            "ok": !errors,
            "problems": problems.iter().map(|p| json!({
                "severity": if p.error { "error" } else { "warning" },
                "message": p.message,
            })).collect::<Vec<_>>(),
        }));
    } else if problems.is_empty() {
        println!("{}: ok, count {}", file.display(), count.unwrap_or(0));
    }
    for p in problems.iter().filter(|_| !o.json) {
        let sev = if p.error { "error" } else { "warning" };
        eprintln!("{}: {sev}: {}", file.display(), p.message);
    }
    i32::from(errors)
}

/// FLTK draws offscreen through the display connection: on Linux that is
/// Wayland or X11 (`xvfb-run` in CI). macOS and Windows always have one.
fn display_available() -> bool {
    let set = |v: &str| std::env::var_os(v).is_some_and(|x| !x.is_empty());
    std::env::consts::OS != "linux" || set("WAYLAND_DISPLAY") || set("DISPLAY")
}

/// `render [--out f.png] [--count n] [--lang code] [--json]`: the window as
/// the user would see it, without showing it.
pub fn render(args: &[String]) -> i32 {
    let o = match opts(args, &["--out", "--count", "--lang"]) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let out = PathBuf::from(o.value("--out").unwrap_or(&format!("{NAME}.png")));
    let count = match o.value("--count") {
        Some(c) => match c.parse::<i64>() {
            Ok(n) => n,
            Err(_) => return fail(format!("--count {c:?} is not a whole number")),
        },
        None => backend::load(&backend::data_file()).unwrap_or(0),
    };
    if !display_available() {
        return fail("render needs a display: run it in a session or under xvfb-run");
    }
    let app = fltk_omarchy::App::without_theme_watch(NAME);
    if let Err(e) = crate::load_catalogs() {
        return fail(format!("translations: {}", e.0));
    }
    if let Some(lang) = o.value("--lang") {
        fltk_omarchy::i18n::set_locale(lang);
    }
    let mut ui = crate::ui::build(Tally::new(count), None);
    let (w, h) = (ui.win.w(), ui.win.h());
    let r = ui.win.snapshot(&out);
    drop(app);
    if let Err(e) = r {
        return fail(format!("{}: {e}", out.display()));
    }
    let abs = std::fs::canonicalize(&out).unwrap_or(out);
    if o.json {
        print_json(&json!({ "png": abs, "width": w, "height": h, "count": count }));
    } else {
        println!("Rendered {}", abs.display());
    }
    0
}

/// `skill` prints the bundled agent skill; `skill install` copies it to
/// `~/.agents/skills/{{name}}/SKILL.md` and links it into
/// `~/.claude/skills`, as Hype does (one shared copy, found directly by
/// Codex, linked for Claude Code).
pub fn skill(args: &[String], home: &Path) -> i32 {
    match args.first().map(String::as_str) {
        None => {
            print!("{SKILL}");
            0
        }
        Some("install") => match install_skill(home) {
            Ok(lines) => {
                lines.iter().for_each(|l| println!("{l}"));
                0
            }
            Err(e) => fail(e),
        },
        Some(_) => fail(format!("use `{NAME} skill` to print the skill, or `{NAME} skill install`")),
    }
}

fn install_skill(home: &Path) -> Result<Vec<String>, String> {
    let dir = home.join(".agents/skills").join(NAME);
    let file = dir.join("SKILL.md");
    backend::write_atomic(&file, SKILL.as_bytes()).map_err(|e| format!("could not write {}: {e}", file.display()))?;
    let mut out = vec![format!("Installed {}", file.display())];
    if !home.join(".claude").is_dir() {
        return Ok(out);
    }
    let link = home.join(".claude/skills").join(NAME);
    match std::fs::symlink_metadata(&link) {
        Ok(m) if m.file_type().is_symlink() => {
            std::fs::remove_file(&link).map_err(|e| format!("{}: {e}", link.display()))?;
        }
        Ok(_) => return Err(format!("{} exists and is not a link; leaving it alone", link.display())),
        Err(_) => {}
    }
    std::fs::create_dir_all(home.join(".claude/skills")).map_err(|e| e.to_string())?;
    link_dir(&format!("../../.agents/skills/{NAME}"), &link).map_err(|e| format!("could not link {}: {e}", link.display()))?;
    out.push(format!("Linked {}", link.display()));
    Ok(out)
}

#[cfg(unix)]
fn link_dir(target: &str, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Without symlinks (Windows needs a privilege for them), a copy.
#[cfg(not(unix))]
fn link_dir(_: &str, link: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(link)?;
    std::fs::write(link.join("SKILL.md"), SKILL)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/test-tmp")
            .join(format!("cli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn check_exit_codes() {
        let d = temp("check");
        let f = d.join("tally.json");
        assert_eq!(check(&s(&[f.to_str().unwrap(), "--json"])), 0); // missing: new tally
        std::fs::write(&f, "{\"count\": 3}").unwrap();
        assert_eq!(check(&s(&[f.to_str().unwrap()])), 0);
        std::fs::write(&f, "oops").unwrap();
        assert_eq!(check(&s(&[f.to_str().unwrap(), "--json"])), 1);
        assert_eq!(check(&s(&["--bogus"])), 1);
    }

    #[test]
    fn commands_and_usage() {
        assert_eq!(run(&s(&["version"])), 0);
        assert_eq!(run(&s(&["help"])), 0);
        assert_eq!(run(&s(&["frobnicate"])), 1);
        assert_eq!(render(&s(&["--count", "many"])), 1);
        assert!(SKILL.starts_with("---\nname: "));
    }

    #[test]
    fn skill_installs_like_hype() {
        let home = temp("home");
        assert_eq!(skill(&s(&["install"]), &home), 0);
        let file = home.join(".agents/skills").join(NAME).join("SKILL.md");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), SKILL);
        assert!(!home.join(".claude").exists()); // no Claude Code, no link

        std::fs::create_dir_all(home.join(".claude")).unwrap();
        assert_eq!(skill(&s(&["install"]), &home), 0);
        let link = home.join(".claude/skills").join(NAME);
        assert_eq!(std::fs::read_to_string(link.join("SKILL.md")).unwrap(), SKILL);
        assert_eq!(skill(&s(&["install"]), &home), 0); // again: replaces its own link
        assert_eq!(skill(&s(&["nope"]), &home), 1);
    }
}
