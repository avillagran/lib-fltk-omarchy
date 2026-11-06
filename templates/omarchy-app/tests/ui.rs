//! Drives the real window offscreen, as an agent would: keys by name, the
//! element tree, a PNG to look at. Nothing is shown on the desktop; on
//! Linux it needs a display (`bin/test` runs it under xvfb-run) and is
//! skipped without one.

use fltk_omarchy::{anim, i18n, testkit, App};
use std::path::PathBuf;
use {{crate}}::{backend::Tally, ui};

fn main() {
    let set = |v: &str| std::env::var_os(v).is_some_and(|x| !x.is_empty());
    if std::env::consts::OS == "linux" && !set("WAYLAND_DISPLAY") && !set("DISPLAY") {
        println!("ui test skipped: no display (run bin/test, or xvfb-run cargo test)");
        return;
    }
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-ui");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    // Omarchy's default look regardless of the current theme, final
    // frames only, English first.
    std::env::set_var("OMARCHY_FLTK_STATE_DIR", out.join("no-theme"));
    std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
    anim::set_enabled(false);
    testkit::arm();
    let _app = App::without_theme_watch({{crate}}::NAME);
    {{crate}}::load_catalogs().unwrap();
    i18n::set_locale("en");

    let ui = ui::build(Tally::new(0), None);
    let shot = out.join("main.png");
    let script = serde_json::json!([
        {"press": "Space"},
        {"press": "Space"},
        {"expect": "2 taps", "visible": true},
        {"expect": "Unsaved changes", "visible": true},
        {"press": "Mod+Z"},
        {"expect": "1 tap", "visible": true},
        {"press": "Mod+Shift+Z"},
        {"expect": "2 taps", "visible": true},
        {"press": "Mod+S"},
        {"expect": "Saving is off: the saved tally could not be read", "visible": true},
        {"snapshot": shot},
    ]);
    let report = testkit::run_script(&*ui.win, &script.to_string());
    assert!(report.ok(), "{}", report.summary());
    assert_eq!(ui.tally.with(Tally::count), 2);
    let png = std::fs::metadata(&shot).map(|m| m.len()).unwrap_or(0);
    assert!(png > 1000, "snapshot {} is empty", shot.display());
    println!("ok   keys: Space counts, Mod+Z / Mod+Shift+Z undo and redo, Mod+S reports why it cannot save");

    // The language switches live: the same widgets re-label themselves.
    i18n::set_locale("da");
    let report = testkit::run_script(&*ui.win, r#"[{"expect": "2 tryk", "visible": true}]"#);
    assert!(report.ok(), "{}", report.summary());
    i18n::set_locale("en");
    println!("ok   language: Danish re-labels the window live");

    // ? opens the shortcut overlay.
    let report = testkit::run_script(&*ui.win, r#"[{"press": "?"}, {"expect": {"kind": "Dialog", "name": "Keyboard shortcuts"}, "visible": true}, {"expect": {"contains": "Count one more"}}]"#);
    assert!(report.ok(), "{}", report.summary());
    println!("ok   ? opens the keyboard shortcuts; picture at {}", shot.display());
}
