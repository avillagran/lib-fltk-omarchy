//! Decode time and memory of `media` on real files, as JSON lines.
//!   cargo run --release --example media_bench --features images -- a.jpg b.png c.gif d.webp
//! For each file: median decode time (5 runs) at the full size and for a
//! 640 × 480 thumbnail (cover), and decoded bytes. Then 24 thumbnails are
//! shown in an offscreen window through the cache and the process PSS is
//! reported before and after (Linux `/proc/self/smaps_rollup`; absent on
//! other systems). `--memory` skips the timing runs so the PSS numbers
//! only see the UI path (the full-size timing decodes grow the heap).
use fltk_omarchy::layout::FlexExt;
use fltk_omarchy::media::{self, Fit, Image, Limits, Source, Target};
use fltk_omarchy::{App, AppWindow};
use std::time::{Duration, Instant};

fn pss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/smaps_rollup").ok()?;
    s.lines().find(|l| l.starts_with("Pss:"))?.split_whitespace().nth(1)?.parse().ok()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // --memory: skip the timing runs, so the PSS numbers only see the UI path.
    let memory_only = args.iter().any(|a| a == "--memory");
    let files: Vec<String> = args.into_iter().filter(|a| a != "--memory").collect();
    if files.is_empty() {
        eprintln!("usage: media_bench <image files>...");
        std::process::exit(2);
    }
    let limits = Limits::default();
    for f in files.iter().filter(|_| !memory_only) {
        let bytes = std::fs::read(f).expect("read");
        for (label, target) in [("full", Target::original()), ("thumb640", Target::new(640, 480, Fit::Cover))] {
            let mut times = Vec::new();
            let mut info = None;
            for _ in 0..5 {
                let t = Instant::now();
                let d = media::decode(&bytes, target, &limits, &|| false);
                times.push(t.elapsed().as_secs_f64() * 1000.0);
                info = Some(d);
            }
            match info.expect("ran") {
                Ok(d) => println!(
                    "{{\"file\":{:?},\"target\":\"{label}\",\"format\":\"{:?}\",\"source\":[{},{}],\"decoded\":[{},{}],\"frames\":{},\"decoded_bytes\":{},\"input_bytes\":{},\"median_ms\":{:.2}}}",
                    f, d.format, d.source_size.0, d.source_size.1, d.size().0, d.size().1, d.frames.len(), d.bytes(), bytes.len(), median(times)
                ),
                Err(e) => println!("{{\"file\":{f:?},\"target\":\"{label}\",\"error\":{:?}}}", e.to_string()),
            }
        }
    }
    let app = App::new("dev.omarchy.fltk.media-bench");
    let _ = &app;
    let mut win = AppWindow::new("bench", 900, 700);
    let mut body = win.body();
    let mut row = fltk_omarchy::layout::row();
    let mut imgs = Vec::new();
    for i in 0..24 {
        let f = &files[i % files.len()];
        let img = Image::new(140, 105).fit(Fit::Cover);
        let _ = f;
        if i % 6 == 0 && i > 0 {
            row.close_fitted();
            body.fixed(&row, 105);
            row = fltk_omarchy::layout::row();
        }
        row.fit(&*img);
        imgs.push(img);
    }
    row.close_fitted();
    body.fixed(&row, 105);
    win.end();
    let shot = std::env::temp_dir().join(format!("media-bench-{}.png", std::process::id()));
    // Baseline: the window drawn once with empty images (fonts, theme,
    // placeholders and icon rasters loaded).
    win.snapshot(&shot).expect("snapshot");
    let before = pss_kib();
    for (i, img) in imgs.iter_mut().enumerate() {
        img.set_source(Some(&Source::file(&files[i % files.len()])));
    }
    let t = Instant::now();
    win.snapshot(&shot).expect("snapshot");
    let end = Instant::now() + Duration::from_secs(30);
    while media::cache_stats().pending > 0 && Instant::now() < end {
        App::pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    win.snapshot(&shot).expect("snapshot");
    let shown_ms = t.elapsed().as_secs_f64() * 1000.0;
    let _ = std::fs::remove_file(&shot);
    let after = pss_kib();
    let s = media::cache_stats();
    println!(
        "{{\"thumbnails\":24,\"files\":{},\"load_and_draw_ms\":{shown_ms:.1},\"cache_entries\":{},\"cache_bytes\":{},\"decodes\":{},\"pss_kib_before\":{},\"pss_kib_after\":{}}}",
        files.len(),
        s.entries,
        s.bytes,
        s.decodes,
        before.map_or("null".into(), |v| v.to_string()),
        after.map_or("null".into(), |v| v.to_string())
    );
}
