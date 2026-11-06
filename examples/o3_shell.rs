//! An o3-code-shaped window to measure the toolkit on the real workload:
//! a sidebar of 231 threads (keyed model, cards), a 5 000-message
//! transcript (Markdown, code blocks) and the Composer, with a live reply
//! streaming into the transcript at 60 Hz while the sidebar receives
//! snapshots every 1.5 s, as o3's backend sends them.
//!
//! ```sh
//! cargo run --release --example o3_shell                 # a window
//! cargo run --release --example o3_shell -- --bench out.json [title]
//! ```
//!
//! `--bench` writes PSS and CPU after building, while idle and while
//! streaming, then quits (script it on a hidden workspace, like
//! script/richtext-bench).

use fltk::{app, prelude::*};
use fltk_omarchy::anim::Pace;
use fltk_omarchy::edit::Composer;
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::model::ListModel;
use fltk_omarchy::navtree::*;
use fltk_omarchy::text::{ColorRole, Message, RichText};
use fltk_omarchy::widgets::glyph;
use fltk_omarchy::{App, AppWindow, State};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

const AGENT: &str = "I looked at `src/live.rs` and the snapshot is rebuilt on every chunk.\n\n\
Here is a smaller **diff-based** path:\n\n```rust\nfn apply(model: &ListModel<String, Row>, rows: Vec<Row>) {\n    model.sync_from(rows);\n}\n```\n\n\
| Case | Before | After |\n|---|---|---|\n| Stream | 9 % | 2 % |\n\n- keeps the anchor\n- no rebuild\n";

fn proc_field(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn cpu_ticks() -> u64 {
    let s = proc_field("/proc/self/stat");
    let f: Vec<&str> = s.rsplit(") ").next().unwrap_or("").split_whitespace().collect();
    f.get(11).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0) + f.get(12).and_then(|x| x.parse::<u64>().ok()).unwrap_or(0)
}

fn pss_kb() -> u64 {
    proc_field("/proc/self/smaps_rollup")
        .lines()
        .filter(|l| l.starts_with("Pss:") || l.starts_with("SwapPss:"))
        .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        .sum()
}

#[derive(Clone)]
struct Thread {
    key: String,
    title: String,
    project: String,
    working: bool,
    minutes: u32,
}

fn threads() -> Vec<Thread> {
    (0..231)
        .map(|i| Thread {
            key: format!("t{i}"),
            title: format!("Thread {i}: {}", ["Fix the sidebar order", "Port the composer", "Measure memory", "Review the diff"][i % 4]),
            project: ["o3-code", "lib-fltk-omarchy", "files", "omarchy"][i % 4].to_string(),
            working: i % 9 == 0,
            minutes: (i as u32 * 7) % 600,
        })
        .collect()
}

fn item(t: &Thread) -> NavItem {
    let (icon, text, role) = if t.working { (glyph::LOADING, "Working".to_string(), ColorRole::Blue) } else { ("\u{f05e1}", format!("{} min", t.minutes), ColorRole::Dim) };
    NavItem::row(&t.key, glyph::FOLDER, &t.title).card().overline(&t.project).status(icon, &text, role)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bench = args.iter().position(|a| a == "--bench").and_then(|i| args.get(i + 1)).cloned();
    let title = args.last().filter(|a| !a.starts_with("--") && bench.as_deref() != Some(a.as_str()) && args.len() > 1).cloned().unwrap_or_else(|| "O3 shell".into());
    let app = App::new("dev.omarchy.fltk.o3-shell");
    let t_build = Instant::now();
    let mut win = AppWindow::new(&title, 1200, 860);
    let mut body = win.body();
    body.set_margin(0);
    let data = Rc::new(RefCell::new(threads()));
    let model: ListModel<String, NavItem> = ListModel::new(|t: &NavItem| t.key.clone());
    model.sync_from(data.borrow().iter().map(item).collect::<Vec<_>>());
    let selected = State::new("t3".to_string());
    let mut row = layout::row();
    let tree = NavTree::new(&model, &selected).widths(48, 320).search("Search threads");
    row.fit(&*tree);
    row.fixed(&*tree, 320);
    let mut col = layout::column();
    col.set_margin(0);
    // Streamed replies paint at 30 fps (pushes in between join one frame).
    let pace = if std::env::var("O3_SHELL_PACE").as_deref() == Ok("normal") { Pace::Normal } else { Pace::Low };
    let view = RichText::new().stick_to_bottom(true).bottom_inset(0).stream_pace(pace);
    col.add(&*view);
    let draft = State::new(String::new());
    let composer = Composer::new().placeholder("Ask anything").bind(&draft);
    col.fit(&*composer);
    col.end();
    row.end();
    win.end();
    tree.set_pinned(true);
    for k in 0..5000usize {
        let md = if k % 2 == 0 { format!("**You** — question {k}: how do I stream Markdown into a `RichText`?") } else { AGENT.to_string() };
        view.set_message(&format!("m{k}"), Message::markdown(&md));
    }
    view.set_message("live", Message::markdown("**Agent** — "));
    let build_ms = t_build.elapsed().as_secs_f64() * 1000.0;
    win.show();

    // Sidebar snapshots every 1.5 s, as o3's backend sends them.
    let (d, m) = (data.clone(), model.clone());
    let mut tick = 0usize;
    app::add_timeout3(1.5, move |h| {
        tick += 1;
        {
            let mut d = d.borrow_mut();
            let n = d.len();
            let i = (tick * 7) % n;
            d[i].working = !d[i].working;
            d[i].minutes += 1;
            m.sync_from(d.iter().map(item).collect::<Vec<_>>());
        }
        app::repeat_timeout3(1.5, h);
    });

    let Some(out) = bench else {
        app.run();
        return;
    };
    // 1.5 s settle, 3 s idle, 6 s streaming (~1 KB/s in 60 pushes a second), 3 s idle.
    let report = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
    report.borrow_mut().push(("build_ms".into(), format!("{build_ms:.0}")));
    let (r, v, o) = (report.clone(), view.clone(), out.clone());
    app::add_timeout3(1.5, move |_| {
        let (c0, t0) = (cpu_ticks(), Instant::now());
        r.borrow_mut().push(("pss_after_build_kb".into(), pss_kb().to_string()));
        let (r, v, o) = (r.clone(), v.clone(), o.clone());
        app::add_timeout3(3.0, move |_| {
            let idle = (cpu_ticks() - c0) as f64 / 100.0 / t0.elapsed().as_secs_f64() * 100.0;
            r.borrow_mut().push(("idle_cpu_pct".into(), format!("{idle:.1}")));
            r.borrow_mut().push(("pss_idle_kb".into(), pss_kb().to_string()));
            let (c1, t1) = (cpu_ticks(), Instant::now());
            let end = t1 + Duration::from_secs(6);
            let words = ["the ", "snapshot ", "is ", "now ", "a ", "diff, ", "keyed ", "by ", "id. "];
            let mut n = 0usize;
            let (v2, r2, o2) = (v.clone(), r.clone(), o.clone());
            app::add_timeout3(1.0 / 60.0, move |h| {
                if Instant::now() < end {
                    for _ in 0..2 {
                        v2.push_message_markdown("live", words[n % words.len()]);
                        n += 1;
                    }
                    app::repeat_timeout3(1.0 / 60.0, h);
                    return;
                }
                let stream = (cpu_ticks() - c1) as f64 / 100.0 / t1.elapsed().as_secs_f64() * 100.0;
                r2.borrow_mut().push(("stream_cpu_pct".into(), format!("{stream:.1}")));
                r2.borrow_mut().push(("pss_stream_kb".into(), pss_kb().to_string()));
                r2.borrow_mut().push(("at_bottom".into(), v2.is_at_bottom().to_string()));
                let (c2, t2) = (cpu_ticks(), Instant::now());
                let (r3, o3) = (r2.clone(), o2.clone());
                app::add_timeout3(3.0, move |_| {
                    let idle = (cpu_ticks() - c2) as f64 / 100.0 / t2.elapsed().as_secs_f64() * 100.0;
                    r3.borrow_mut().push(("idle_after_cpu_pct".into(), format!("{idle:.1}")));
                    let json = format!(
                        "{{{}}}",
                        r3.borrow().iter().map(|(k, v)| format!("\"{k}\":{}", if v.parse::<f64>().is_ok() || v == "true" || v == "false" { v.clone() } else { format!("\"{v}\"") })).collect::<Vec<_>>().join(",")
                    );
                    println!("{json}");
                    let _ = std::fs::write(&o3, json + "\n");
                    app::quit();
                });
            });
        });
    });
    app.run();
}
