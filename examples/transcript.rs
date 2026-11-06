//! An o3-style conversation in one `RichText`: keyed messages, an
//! activity card drawn by the app (embed), an approval card hosting a
//! real button, a code block with app actions, a table with copy actions,
//! an image resolved later, find, and a reply streamed by re-sending the
//! whole row (as o3's protocol does) so only the new text is parsed.
//!   cargo run --example transcript
//!   cargo run --example transcript -- --snapshot build/shots/transcript.png
//!
//! Keys: Tab walks links, actions and cards; Enter/Space runs them; F3 /
//! Shift+F3 move between find matches; End follows the conversation again.
use fltk::{app, prelude::*};
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::text::{BlockAction, ImageData, Message, MarkdownOptions, RichText};
use fltk_omarchy::widgets::{glyph, Button, Label, TextField};
use fltk_omarchy::{paint, theme, tr, App, AppWindow, State};
use std::cell::Cell;
use std::rc::Rc;

const REPLY: &str = "The streaming parser keeps an **open tail**: each push re-parses only the last block, \
so a long answer costs O(tail). Rows that o3 re-sends whole are compared with the text set before \
and only the new part is parsed.\n\n- committed blocks never change\n- the view stays at the end \
while it grows, until you scroll up\n\nSee `src/text/markdown.rs:140` for the details.";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let snapshot = args.iter().skip_while(|a| *a != "--snapshot").nth(1).cloned();
    let app = App::new("dev.omarchy.fltk.transcript");
    fltk_omarchy::anim::set_enabled(snapshot.is_none());
    let mut win = AppWindow::new("Transcript", 680, 820);
    let _body = win.body();
    let query = State::new(String::new());
    let count = State::new(String::new());
    let mut bar = layout::row();
    let find = TextField::new().placeholder(&tr!("Search…")).bind(&query);
    bar.fit(&*find);
    bar.fit(&*Label::new("").bind(&count));
    bar.close_fitted();
    let view = RichText::new()
        .stick_to_bottom(true)
        .markdown_options(MarkdownOptions::default().with_path_links())
        .on_link(|url| println!("open {url}"))
        .code_actions(|lang, code| {
            let mut a = Vec::new();
            if lang == "sh" && !code.trim().is_empty() {
                a.push(BlockAction::new("run", &tr!("Run")));
            }
            if lang == "mermaid" {
                a.push(BlockAction::new("diagram", &tr!("Open")));
            }
            a
        })
        .table_actions(true)
        .on_block_action(|e| println!("action {} on block {} of {}: {:?}", e.action, e.index, e.message, e.text))
        .on_embed_activate(|id, at| println!("card {id} activated at {at:?}"))
        .on_embed_visibility(|id, on| println!("card {id} {}", if on { "in view" } else { "out of view" }))
        .embed_painter(|p| {
            let t = p.theme;
            let pal = &t.palette;
            let r = t.radius.min(8);
            paint::fill(p.x, p.y, p.w, p.h, r, pal.lighter_background);
            paint::stroke(p.x, p.y, p.w, p.h, r, 1, if p.focused { pal.accent } else { pal.foreground.over(p.under, 0.15) });
            let (title, detail) = p.text.split_once('\n').unwrap_or((p.text, ""));
            let pad = t.spacing.lg;
            paint::fill(p.x + pad, p.y + pad + 4, 8, 8, 4, if p.id.starts_with("approve") { pal.yellow } else { pal.green });
            paint::text(title, p.x + pad + 16, p.y + pad - 2, p.w - 2 * pad, t.font.body + 8, t.font.body, true, pal.foreground, fltk::enums::Align::Left);
            paint::text_elided(detail, p.x + pad + 16, p.y + pad + t.font.body + 6, p.w - 3 * pad - 16, t.font.caption + 8, t.font.caption, false, pal.foreground.darker(1.4), fltk::enums::Align::Left);
        });
    let img_view = view.clone();
    let view = view.on_image(move |src| {
        // A "decoder": a gradient of theme colors, delivered later.
        let v = img_view.clone();
        let src = src.to_string();
        app::add_timeout3(0.0, move |_| {
            let (w, h) = (360, 120);
            let (a, b) = theme::with(|t| (t.palette.green, t.palette.magenta));
            let mut px = Vec::with_capacity((w * h * 4) as usize);
            for _y in 0..h {
                for x in 0..w {
                    let c = b.over(a, x as f64 / w as f64);
                    px.extend_from_slice(&[c.r(), c.g(), c.b(), 255]);
                }
            }
            v.set_image(&src, ImageData::Rgba { width: w, height: h, pixels: px });
        });
    });
    // Not fitted: the view takes the rest of the body.
    win.end();

    // Find as you type.
    {
        let (v, c) = (view.clone(), count.clone());
        query
            .subscribe(move |q| {
                let n = v.find(q);
                c.set(if q.is_empty() { String::new() } else { format!("{n}") });
            })
            .forever();
    }
    win.shortcut("F3", {
        let v = view.clone();
        move || {
            v.find_next();
        }
    });
    win.header().action(glyph::CHEVRON_DOWN, &tr!("Forward"), {
        let v = view.clone();
        move || v.follow_end()
    });

    view.set_message("u1", Message::markdown("**You**\n\nHow does the transcript stay fast while an agent streams?"));
    view.set_message("act1", Message::embed("act-1", 64, "Ran 3 commands\ncargo test --offline · rg -n push_markdown · git diff --stat"));
    view.set_message("a1", Message::markdown(
        "**Agent**\n\nHere is the plan, with a diagram and the numbers:\n\n```sh\ncargo run --release --example richtext_bench\n```\n\n\
         ```mermaid\ngraph LR; push-->tail-->layout\n```\n\n| Case | push avg | CPU |\n|------|---------:|----:|\n| timer 60 Hz | 0.33 ms | 18 % |\n| sender | — | 19 % |\n\n\
         ![bench chart](demo://chart)\n\nLinks keep their scheme: [the earlier thread](t3-thread://abc123)."));
    view.set_message("approve1", Message::embed("approve-1", 56, "Allow the agent to publish the branch?\nThe button on the right is a real widget hosted over this card."));
    // A real widget over the card: a row whose spacer lets the card show.
    let mut approve = fltk::group::Flex::default().row();
    approve.set_margins(0, 14, 16, 14);
    fltk::frame::Frame::default();
    let button = Button::new(&tr!("Apply")).bordered().on_click(|| println!("approved"));
    approve.fixed(&*button, 90);
    approve.end();
    view.host_widget("approve-1", &approve);
    view.set_message("u2", Message::markdown("**You**\n\nAnd the reply below?"));

    if let Some(path) = snapshot {
        view.set_message("a2", Message::markdown(&format!("**Agent**\n\n{REPLY}")));
        win.snapshot(&path).expect("snapshot");
        // The image arrives on the next loop turn: snapshot again with it.
        for _ in 0..5 {
            app::wait_for(0.01).ok();
        }
        view.scroll_to(0);
        win.snapshot(&path).expect("snapshot");
        println!("snapshot written to {path}");
        return;
    }
    // o3 re-sends the whole row every 75 ms while it streams.
    let sent = Rc::new(Cell::new(0usize));
    let v = view.clone();
    app::add_timeout3(0.075, move |h| {
        let n = (sent.get() + 9).min(REPLY.len());
        let n = (n..=REPLY.len()).find(|&i| REPLY.is_char_boundary(i)).unwrap_or(REPLY.len());
        sent.set(n);
        v.set_message("a2", Message::markdown(&format!("**Agent**\n\n{}", &REPLY[..n])));
        if n < REPLY.len() {
            app::repeat_timeout3(0.075, h);
        } else {
            v.finish_message("a2");
        }
    });
    win.show();
    app.run();
}
