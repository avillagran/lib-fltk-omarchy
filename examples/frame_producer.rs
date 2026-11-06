//! A frame producer for `examples/frame_view.rs` (and a template for real
//! helpers). Started by a `FrameView`, it opens the surface from
//! `OMARCHY_FRAME`, draws an animated page-like pattern at a fixed rate,
//! follows resize and zoom requests and answers input:
//!
//!   cargo run --example frame_producer -- [--fps 30] [--pattern page|photo] [--files DIR --format png|bmp]
//!
//! - Shared memory (default): draws straight into the surface and
//!   presents; clicks and keys are echoed back as messages.
//! - `--files DIR`: the old path, for comparison: encodes each frame as a
//!   PNG (feature `images`) or BMP file in DIR, replaced by rename, and
//!   announces it on stdout as `{"frame":"<path>"}` like o3-code's helpers.
//!   Input then arrives as JSON lines on stdin.
//!
//! On `{"type":"quit"}` it prints `{"stats":{..}}` on stdout (frames,
//! draw and encode time, CPU time) and exits.
use fltk_omarchy::frame::{Input, Message, Producer};
use serde_json::json;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

/// `--pattern photo`: smooth gradients with per-pixel noise (camera or
/// device-screen content; compresses badly, like real video frames).
fn draw_photo(px: &mut [u8], w: usize, h: usize, stride: usize, t: u64) {
    for y in 0..h {
        let row = &mut px[y * stride..y * stride + w * 4];
        for x in 0..w {
            let n = (x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663) ^ (t as u32).wrapping_mul(83_492_791);
            let n = (n >> 13) as u8 & 0x1f;
            let r = ((x * 255 / w.max(1)) as u8).wrapping_add(t as u8).wrapping_add(n);
            let g = ((y * 255 / h.max(1)) as u8).wrapping_add(n);
            let b = (((x + y) * 128 / (w + h).max(1)) as u8).wrapping_add(n);
            row[x * 4..x * 4 + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
}

fn photo() -> bool {
    arg("--pattern").is_some_and(|p| p == "photo")
}

/// A page-like frame: background, a moving accent bar, rows of "text".
fn draw(px: &mut [u8], w: usize, h: usize, stride: usize, t: u64, zoom: f32) {
    if photo() {
        return draw_photo(px, w, h, stride, t);
    }
    let bar = ((t * 6) % w.max(1) as u64) as usize;
    let line = (14.0 * zoom).max(4.0) as usize;
    for y in 0..h {
        let row = &mut px[y * stride..y * stride + w * 4];
        let text_row = (y / line) % 3 != 2 && (y % line) < line * 2 / 3;
        for x in 0..w {
            let mut c = [0x12u8, 0x12, 0x14, 255];
            if text_row && (x / 7 + y / line) % 11 < 8 && x > 24 && x < w.saturating_sub(24) {
                c = [0xbe, 0xbe, 0xbe, 255];
            }
            if x.abs_diff(bar) < 6 {
                c = [0xe6, 0x8e, 0x0d, 255];
            }
            row[x * 4..x * 4 + 4].copy_from_slice(&c);
        }
    }
}

#[cfg(feature = "images")]
fn encode_png(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        // Qt's PNG writer at quality 80 uses a fast zlib level.
        e.set_compression(png::Compression::Fast);
        let mut wr = e.write_header().expect("png header");
        wr.write_image_data(rgba).expect("png data");
    }
    out
}

#[cfg(not(feature = "images"))]
fn encode_png(_rgba: &[u8], _w: u32, _h: u32) -> Vec<u8> {
    eprintln!("frame_producer: --format png needs --features images (use --format bmp)");
    std::process::exit(2);
}

fn encode_bmp(rgba: &[u8], w: u32, h: u32) -> Vec<u8> {
    // 32-bit BI_RGB, top-down: what a fast writer to tmpfs produces.
    let size = 54 + rgba.len();
    let mut b = Vec::with_capacity(size);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(size as u32).to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(-(h as i32)).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(&[0; 24]);
    for p in rgba.chunks_exact(4) {
        b.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
    }
    b
}

fn stats_line(frames: u64, draw: Duration, encode: Duration, start: Instant) {
    let cpu = fltk_omarchy::platform::process_cpu_time().map_or(-1.0, |d| d.as_secs_f64() * 1000.0);
    let n = frames.max(1) as f64;
    println!(
        "{}",
        json!({"stats": {
            "frames": frames,
            "seconds": start.elapsed().as_secs_f64(),
            "draw_ms_per_frame": draw.as_secs_f64() * 1000.0 / n,
            "encode_ms_per_frame": encode.as_secs_f64() * 1000.0 / n,
            "cpu_ms": cpu,
        }})
    );
    let _ = std::io::stdout().flush();
}

fn main() {
    let fps: f64 = arg("--fps").and_then(|s| s.parse().ok()).unwrap_or(30.0);
    let period = Duration::from_secs_f64(1.0 / fps.max(1.0));
    let start = Instant::now();
    let (mut draw_t, mut frames) = (Duration::ZERO, 0u64);
    if let Some(dir) = arg("--files") {
        files_mode(PathBuf::from(dir), arg("--format").unwrap_or_else(|| "png".into()), period);
        return;
    }
    let mut p = Producer::from_env().expect("frame_producer is started by a FrameView (OMARCHY_FRAME)");
    let mut zoom = 1.0f32;
    let mut next = Instant::now();
    loop {
        p.wait(Some(next.saturating_duration_since(Instant::now())));
        for v in p.messages_raw() {
            if v["type"] == "quit" {
                stats_line(frames, draw_t, Duration::ZERO, start);
                return;
            }
            let Some(e) = Input::from_json(&v) else { continue };
            match &e {
                Input::Resize { width, height, scale } => {
                    p.set_size((*width as f32 * scale).round() as u32, (*height as f32 * scale).round() as u32);
                }
                Input::Zoom { value } => zoom = *value,
                Input::Copy => {
                    p.send(&Message::Clipboard(format!("frame {frames}")));
                }
                Input::Press { .. } | Input::KeyDown { .. } => {
                    p.send(&Message::Other(json!({"echo": e.to_json()})));
                }
                _ => {}
            }
        }
        if p.consumer_gone() {
            return;
        }
        if Instant::now() >= next {
            let (w, h) = p.size();
            let stride = p.stride();
            let t0 = Instant::now();
            draw(p.frame_mut(), w as usize, h as usize, stride, frames, zoom);
            draw_t += t0.elapsed();
            p.present();
            frames += 1;
            if frames == 1 {
                // Tell the view how tall the "page" is (inline auto height).
                p.send(&Message::ContentSize { width: w, height: h * 3 });
            }
            next += period;
            if next < Instant::now() {
                next = Instant::now() + period;
            }
        }
    }
}

/// The PNG/BMP file path: frames as files, input on stdin.
fn files_mode(dir: PathBuf, format: String, period: Duration) {
    let start = Instant::now();
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for l in std::io::stdin().lock().lines().map_while(Result::ok) {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    let (mut w, mut h) = (640u32, 400u32);
    let (mut draw_t, mut enc_t, mut frames) = (Duration::ZERO, Duration::ZERO, 0u64);
    let mut px = Vec::new();
    let mut next = Instant::now();
    loop {
        let left = next.saturating_duration_since(Instant::now());
        if let Ok(l) = rx.recv_timeout(left) {
            let v: serde_json::Value = serde_json::from_str(&l).unwrap_or_default();
            if v["type"] == "quit" {
                stats_line(frames, draw_t, enc_t, start);
                return;
            }
            if let Some(Input::Resize { width, height, scale }) = Input::from_json(&v) {
                w = (width as f32 * scale).round().max(1.0) as u32;
                h = (height as f32 * scale).round().max(1.0) as u32;
            }
            continue;
        }
        let t0 = Instant::now();
        px.resize(w as usize * h as usize * 4, 0);
        draw(&mut px, w as usize, h as usize, w as usize * 4, frames, 1.0);
        let t1 = Instant::now();
        let (bytes, ext) = if format == "bmp" { (encode_bmp(&px, w, h), "bmp") } else { (encode_png(&px, w, h), "png") };
        let path = dir.join(format!("frame-{}.{ext}", frames % 2));
        let tmp = dir.join(format!("frame-{}.{ext}.next", frames % 2));
        std::fs::write(&tmp, &bytes).expect("write frame");
        std::fs::rename(&tmp, &path).expect("rename frame");
        enc_t += t1.elapsed();
        draw_t += t1 - t0;
        frames += 1;
        println!("{}", json!({"frame": path, "revision": frames, "width": w, "height": h}));
        let _ = std::io::stdout().flush();
        next += period;
        if next < Instant::now() {
            next = Instant::now() + period;
        }
    }
}
