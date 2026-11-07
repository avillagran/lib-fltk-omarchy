//! Lazy modal form: app-owned drafts survive delayed widget release.
use fltk::prelude::*;
use fltk_omarchy::{
    i18n,
    layout::{self, FlexExt},
    overlay::Sheet,
    tr,
    widgets::*,
    App, AppWindow, State,
};
fn main() {
    let app = App::new("dev.omarchy.fltk.sheets");
    i18n::load(include_str!("support/sheets.json")).expect("catalog");
    let mut win = AppWindow::new(&tr!("Lazy sheets"), 640, 440);
    win.bind_title(&i18n::text("Lazy sheets"));
    let draft = State::new(String::new());
    let saved = State::new(None::<String>);
    let builds = State::new(0usize);
    let mut body = win.body();
    body.fit(&*Label::new("").bind(&i18n::text(
        "The form is built on demand and released 30 seconds after closing.",
    )));
    body.fit(&*Label::new("").bind(&i18n::map(&saved, |name| {
        name.as_ref()
            .map(|name| tr!("Saved: {name}", name = name))
            .unwrap_or_default()
    })));
    body.fit(&*Label::new("").bind(&i18n::map(&builds, |n| {
        tr!("Content builds: {count}", count = n)
    })));
    win.end();
    let (d, s, b) = (draft.clone(), saved.clone(), builds.clone());
    let sheet = Sheet::new(&*win, "", 460, 260, move |col, owner| {
        b.update(|n| *n += 1);
        col.fit(&*Label::new("").bind(&i18n::text("Name")));
        let field = TextField::new().bind(&d);
        let live = field.clone();
        i18n::bind(&*field, move || {
            live.clone().placeholder(&tr!("Name"));
        });
        col.fit(&*field);
        col.fit(&*Label::new("").bind(&i18n::text("Draft state survives content reconstruction.")));
        let mut row = layout::row();
        let cancel = owner.clone();
        row.fit(
            &*Button::new("")
                .bind_text(&i18n::text("Cancel"))
                .on_click(move || {
                    if let Some(s) = cancel.upgrade() {
                        s.close();
                    }
                }),
        );
        let (draft, saved) = (d.clone(), s.clone());
        row.fit(
            &*Button::new("")
                .bind_text(&i18n::text("Save"))
                .shortcut("Mod+Enter")
                .on_click(move || {
                    saved.set(Some(draft.get()));
                    if let Some(s) = owner.upgrade() {
                        s.close();
                    }
                }),
        );
        row.close_fitted();
    })
    .bind_title(&i18n::text("Edit details"));
    let show = sheet.clone();
    win.header()
        .action(glyph::FILE, "", move || {
            show.open();
        })
        .bind_tooltip(&i18n::text("Edit details"));
    let show = sheet.clone();
    win.shortcut("Mod+I", move || {
        show.open();
    });
    let help = (*win).clone();
    win.shortcut("F1", move || {
        fltk_omarchy::popups::ShortcutHelp::open(
            &help,
            &tr!("Form"),
            vec![(
                tr!("Form"),
                vec![
                    ("Mod+I".into(), tr!("Edit details")),
                    ("Mod+Enter".into(), tr!("Save")),
                    ("Escape".into(), tr!("Close")),
                ],
            )],
        );
    });
    let args: Vec<String> = std::env::args().collect();
    if let Some(index) = args.iter().position(|a| a == "--snapshot") {
        fltk_omarchy::anim::set_enabled(false);
        sheet.open();
        win.snapshot(args.get(index + 1).expect("snapshot path"))
            .expect("snapshot");
        fltk::window::DoubleWindow::delete((*win).clone());
        return;
    }
    win.show();
    app.run();
}
