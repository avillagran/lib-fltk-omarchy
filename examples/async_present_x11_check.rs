//! Async presentation wire probe, for a private X11 display only.
use fltk::prelude::*;
use fltk_omarchy::{
    native::{self, PresentOptions, PresentRequest},
    App,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn main() {
    let _app = App::without_theme_watch("dev.omarchy.fltk.async-x11-check");
    let root = PathBuf::from(std::env::args().nth(1).unwrap());
    let mut a = fltk::window::DoubleWindow::new(0, 0, 200, 150, "async-x11-a");
    a.end();
    a.show();
    let mut b = fltk::window::DoubleWindow::new(250, 0, 200, 150, "async-x11-b");
    b.end();
    b.show();
    let (mut old, mut current): (Option<PresentRequest>, Option<PresentRequest>) = (None, None);
    std::fs::write(
        root.join("ready.json"),
        json!({"a":a.raw_handle() as usize,"b":b.raw_handle() as usize}).to_string(),
    )
    .unwrap();
    let start = Instant::now();
    let mut last = 0;
    while start.elapsed() < Duration::from_secs(40) && !root.join("done").exists() {
        let _ = fltk::app::wait_for(0.01);
        if let Ok(bytes) = std::fs::read(root.join("command.json")) {
            if let Ok(cmd) = serde_json::from_slice::<Value>(&bytes) {
                let seq = cmd["seq"].as_u64().unwrap_or(0);
                if seq <= last {
                    continue;
                }
                let options = PresentOptions {
                    startup_id: Some("async-wire_TIME123"),
                    ..Default::default()
                };
                let answer = match cmd["action"].as_str().unwrap() {
                    "begin" => {
                        current = Some(native::begin_present(&mut b, options).unwrap());
                        "begun".to_string()
                    }
                    "cancel" => format!("{}", current.as_mut().unwrap().cancel()),
                    "drop" => {
                        current.take();
                        "dropped".into()
                    }
                    "poll" => format!("{:?}", current.as_mut().unwrap().poll()),
                    "remember" => {
                        old = current.take();
                        "remembered".into()
                    }
                    "drop_old" => {
                        old.take();
                        "dropped".into()
                    }
                    "busy" => format!("{:?}", native::begin_present(&mut a, options)),
                    _ => panic!("unknown action"),
                };
                std::fs::write(
                    root.join(format!("reply-{seq}.json")),
                    json!({"result":answer}).to_string(),
                )
                .unwrap();
                last = seq;
            }
        }
    }
    a.hide();
    b.hide();
}
