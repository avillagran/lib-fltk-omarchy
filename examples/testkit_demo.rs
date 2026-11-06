//! A small app for the testkit's env runner (docs/TESTKIT.md):
//!
//! ```sh
//! env -u WAYLAND_DISPLAY xvfb-run -a \
//!   env OMARCHY_FLTK_SCRIPT=steps.json OMARCHY_FLTK_TREE=tree.json \
//!   cargo run --example testkit_demo
//! ```
//!
//! Without the variables it opens a normal window.
use fltk_omarchy::layout::FlexExt;
use fltk_omarchy::widgets::*;
use fltk_omarchy::{tr, App, AppWindow, State};

fn main() {
    let app = App::new("dev.omarchy.fltk.testkit-demo");
    let mut win = AppWindow::new("Testkit demo", 420, 300);
    win.header().action(glyph::SETTINGS, &tr!("Preferences"), || {});
    let mut body = win.body();
    let name = State::new(String::new());
    body.fit(&*TextField::new().placeholder(&tr!("Your name")).bind(&name));
    let count = State::new(0usize);
    let c2 = count.clone();
    body.fit(&*Button::new(&tr!("Add")).bordered().on_click(move || c2.update(|n| *n += 1)));
    body.fit(&*Label::new("").bind(&count.map(|n| tr!("Count {n}", n = n))));
    let palette = State::new(tr!("Palette closed"));
    body.fit(&*Label::new("").bind(&palette));
    win.end();
    let p2 = palette.clone();
    win.shortcut("Mod+K", move || p2.set(tr!("Palette open")));
    win.show();
    app.run();
}
