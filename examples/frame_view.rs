//! A `FrameView` showing `examples/frame_producer.rs` running as a child
//! process: shared memory by default, or the PNG/BMP file path that
//! o3-code's helpers use today. Click and type into the frame (the
//! producer echoes input), Mod+= / Mod+- / Mod+0 zoom, Mod+C copies the
//! producer's answer.
//!
//!   cargo build --release --example frame_producer --example frame_view
//!   target/release/examples/frame_view [--files png|bmp] [--fps 30]
//!
//! Benchmark (prints one JSON line): latency present → drawn, CPU of both
//! processes, memory (Linux PSS), for `--size` frames at `--fps`:
//!
//!   target/release/examples/frame_view --bench 10 --size 1280x800 [--files png|bmp] [--pattern page|photo] [--json out.jsonl]
use fltk::app;
use fltk::prelude::*;
use fltk_omarchy::frame::{FrameView, Message};
use fltk_omarchy::layout::FlexExt;
use fltk_omarchy::widgets::*;
use fltk_omarchy::{tr, App, AppWindow};
use serde_json::json;
use std::cell::RefCell;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

/// PSS in KiB of a process (Linux; None elsewhere).
fn pss_kib(pid: &str) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    s.lines().find(|l| l.starts_with("Pss:"))?.split_whitespace().nth(1)?.parse().ok()
}

fn producer_path() -> PathBuf {
    let exe = std::env::current_exe().expect("exe");
    exe.with_file_name(format!("frame_producer{}", std::env::consts::EXE_SUFFIX))
}

thread_local! {
    static VIEW: RefCell<Option<FrameView>> = const { RefCell::new(None) };
}

fn main() {
    let bench: Option<f64> = arg("--bench").and_then(|s| s.parse().ok());
    let files = arg("--files");
    let fps = arg("--fps").unwrap_or_else(|| "30".into());
    let (w, h) = arg("--size")
        .and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1280), b.parse().unwrap_or(800))))
        .unwrap_or((960, 600));

    let app = App::new("dev.omarchy.fltk.frame-view");
    let mut win = AppWindow::new(&tr!("Frame view"), w, h + if bench.is_some() { 0 } else { 80 });
    let mut body = win.body();
    if bench.is_some() {
        body.set_margin(0);
    }
    let view = FrameView::new(w, h).name(&tr!("Producer output"));
    body.add(&*view);
    let status = Label::new("");
    if bench.is_some() {
        body.remove(&*status);
    } else {
        body.fit(&*status);
        let v = view.clone();
        win.header().action("\u{f06ed}", &tr!("Zoom out"), move || v.zoom_out());
        let v = view.clone();
        win.header().action("\u{f06ec}", &tr!("Zoom in"), move || v.zoom_in());
        for (spec, f) in [("Mod+=", 1), ("Mod+-", -1), ("Mod+0", 0)] {
            let v = view.clone();
            win.shortcut(spec, move || match f {
                1 => v.zoom_in(),
                -1 => v.zoom_out(),
                _ => v.reset_zoom(),
            });
        }
    }
    win.end();
    win.show();
    for _ in 0..20 {
        App::pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    if bench.is_some() && view.h() != h {
        // Make the frame area exactly the requested size (below the header).
        let extra = h - view.h();
        let (x, y, ww, wh) = (win.x(), win.y(), win.w(), win.h());
        win.resize(x, y, ww, wh + extra);
        App::pump();
    }
    VIEW.with(|v| *v.borrow_mut() = Some(view.clone()));

    // The producer's stdout: frame announcements (files) and final stats.
    let child_stats: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
    let mut cmd = Command::new(producer_path());
    cmd.args(["--fps", &fps]).stdout(Stdio::piped());
    let pattern = arg("--pattern").unwrap_or_else(|| "page".into());
    cmd.args(["--pattern", &pattern]);
    let bench_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("build/frame-bench");
    let mut child = if let Some(format) = &files {
        std::fs::create_dir_all(&bench_dir).expect("bench dir");
        cmd.args(["--files", bench_dir.to_str().expect("utf-8"), "--format", format]).stdin(Stdio::piped());
        let mut child = cmd.spawn().expect("start frame_producer (build it first)");
        // Input as JSON lines on the helper's stdin, as o3-code does today.
        let stdin = child.stdin.take().expect("stdin");
        let v = view.clone().events_to(stdin);
        // The size, as the shared surface would have reported it.
        v.send(&fltk_omarchy::frame::Input::Resize { width: w as u32, height: h as u32, scale: 1.0 });
        child
    } else {
        view.spawn(cmd).expect("start frame_producer (build it first)")
    };
    let pid = child.id().to_string();
    {
        let out = child.stdout.take().expect("stdout");
        let stats = child_stats.clone();
        std::thread::spawn(move || {
            for l in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                let v: serde_json::Value = serde_json::from_str(&l).unwrap_or_default();
                if let Some(p) = v["frame"].as_str() {
                    let p = PathBuf::from(p);
                    fltk_omarchy::task::post(move || VIEW.with(|v| v.borrow().as_ref().map(|v| v.show_file(p))).unwrap_or(()));
                } else if v.get("stats").is_some() {
                    *stats.lock().unwrap() = Some(v["stats"].clone());
                }
            }
        });
    }
    let echoes = Rc::new(RefCell::new(0u32));
    let e2 = echoes.clone();
    let view = view.on_message(move |m| {
        if let Message::Other(v) = m {
            if v.get("echo").is_some() {
                *e2.borrow_mut() += 1;
            }
        }
    });

    match bench {
        None => {
            let (v, mut st) = (view.clone(), status.clone());
            app::add_timeout3(0.5, move |h| {
                let s = v.stats();
                let lat = s.mean_latency.map_or(0.0, |d| d.as_secs_f64() * 1000.0);
                st.set_label(&tr!(
                    "{frames} frames, {w}×{h}, latency {lat} ms, {echo} echoes, zoom {zoom}%",
                    frames = s.drawn,
                    w = s.frame_size.0,
                    h = s.frame_size.1,
                    lat = format!("{lat:.1}"),
                    echo = *echoes.borrow(),
                    zoom = (v.zoom() * 100.0).round()
                ));
                app::repeat_timeout3(0.5, h);
            });
            app.run();
            view.send_json(&json!({"type": "quit"}));
            let _ = child.wait();
        }
        Some(secs) => {
            // Warm up 1 s, then measure `secs`.
            let t_start = Instant::now();
            while t_start.elapsed() < Duration::from_secs(1) {
                let _ = app::wait_for(0.05);
                fltk_omarchy::task::drain();
            }
            let s0 = view.stats();
            let cpu0 = fltk_omarchy::platform::process_cpu_time().unwrap_or_default();
            let t0 = Instant::now();
            let mut max_lat = Duration::ZERO;
            let mut lat_sum = Duration::ZERO;
            let mut last_drawn = s0.drawn;
            let mut draw_sum = Duration::ZERO;
            let mut samples = 0u32;
            while t0.elapsed() < Duration::from_secs_f64(secs) {
                let _ = app::wait_for(0.05);
                fltk_omarchy::task::drain();
                let s = view.stats();
                if s.drawn > last_drawn {
                    last_drawn = s.drawn;
                    if let Some(l) = s.last_latency {
                        max_lat = max_lat.max(l);
                        lat_sum += l;
                        draw_sum += s.last_draw;
                        samples += 1;
                    }
                }
            }
            let wall = t0.elapsed().as_secs_f64();
            let cpu = fltk_omarchy::platform::process_cpu_time().unwrap_or_default() - cpu0;
            let s1 = view.stats();
            let (pss_ui, pss_child) = (pss_kib("self"), pss_kib(&pid));
            view.send_json(&json!({"type": "quit"}));
            let end = Instant::now();
            while child_stats.lock().unwrap().is_none() && end.elapsed() < Duration::from_secs(3) {
                App::pump();
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.wait();
            let cs = child_stats.lock().unwrap().clone().unwrap_or_default();
            let child_cpu_pct = cs["cpu_ms"].as_f64().zip(cs["seconds"].as_f64()).map(|(c, s)| c / 10.0 / s);
            let n = samples.max(1);
            let line = json!({
                    "mode": files.as_deref().map_or("shm".to_string(), |f| format!("file-{f}")),
                    "pattern": pattern, "wayland": fltk_omarchy::platform::wayland(),
                    "size": [w, h], "fps_target": fps.parse::<f64>().unwrap_or(30.0),
                    "seconds": wall,
                    "frames_drawn": s1.drawn - s0.drawn,
                    "fps_drawn": (s1.drawn - s0.drawn) as f64 / wall,
                    "frames_skipped": s1.skipped - s0.skipped,
                    "latency_ms_mean": lat_sum.as_secs_f64() * 1000.0 / n as f64,
                    "latency_ms_max": max_lat.as_secs_f64() * 1000.0,
                    "draw_ms_mean": draw_sum.as_secs_f64() * 1000.0 / n as f64,
                    "zero_copy": s1.zero_copy, "frame_size": [s1.frame_size.0, s1.frame_size.1], "view_size": [view.w(), view.h()],
                    "ui_cpu_pct": cpu.as_secs_f64() * 100.0 / wall,
                    "producer_cpu_pct_lifetime": child_cpu_pct,
                    "producer": cs,
                    "ui_pss_kib": pss_ui, "producer_pss_kib": pss_child,
                });
            println!("{line}");
            if let Some(path) = arg("--json") {
                let mut prev = std::fs::read_to_string(&path).unwrap_or_default();
                prev.push_str(&format!("{line}\n"));
                let _ = std::fs::write(&path, prev);
            }
        }
    }
}
