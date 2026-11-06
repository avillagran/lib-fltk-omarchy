//! Interactive lava reference: --cpu, --snapshot PATH, --size WIDTHxHEIGHT.
//! --bench PATH runs the same controller without taking keyboard focus.
//! Add --bench-overlay for a retained non-modal notification.
//! Use a private display for scripted input; never the focused desktop.
#[path = "support/lava_bench.rs"]
mod lava_bench;
#[path = "support/lava_ui.rs"]
mod lava_ui;
use fltk::prelude::*;
use fltk_omarchy::{anim, App};
fn main() {
    let app = App::new("dev.omarchy.fltk.lava");
    let args: Vec<_> = std::env::args().collect();
    let benchmark = args.iter().position(|a| a == "--bench");
    let mut demo = lava_ui::Lava::new(args.iter().any(|a| a == "--cpu"));
    if let Some(i) = args.iter().position(|a| a == "--size") {
        let (w, h) = args
            .get(i + 1)
            .and_then(|s| s.split_once('x'))
            .expect("--size WIDTHxHEIGHT");
        let (w, h) = (
            w.parse::<i32>().expect("width"),
            h.parse::<i32>().expect("height"),
        );
        assert!(
            (560..=4096).contains(&w) && (620..=4096).contains(&h),
            "size out of bounds"
        );
        demo.win.set_size(w, h);
    }
    if let Some(i) = args.iter().position(|a| a == "--snapshot") {
        anim::set_enabled(false);
        demo.win
            .snapshot(args.get(i + 1).expect("snapshot path"))
            .expect("lava snapshot");
    } else {
        if benchmark.is_some() {
            demo.win.set_mode(fltk_omarchy::window::Mode::Floating);
        }
        demo.win.show();
        if let Some(i) = benchmark {
            lava_bench::start_when_ready(
                demo.surface.clone(),
                demo.core.clone(),
                demo.win.clone(),
                args.get(i + 1).expect("benchmark path").clone(),
            );
        }
        app.run();
    }
}
