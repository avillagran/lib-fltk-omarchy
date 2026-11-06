//! Parses and rasterizes whole icon sets with `icon::Svg` and reports
//! failures and timings. Input: JSON objects of `name → SVG source`
//! (o3-code's `ui/assets/lucide-project-icons.json`, for example).
//!   cargo run --release --example icons_check -- icons.json [more.json] [--sheet sheet.png]
use fltk_omarchy::gfx::{Canvas, Color};
use fltk_omarchy::icon::Svg;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let sheet = args.iter().skip_while(|a| *a != "--sheet").nth(1).cloned();
    let files: Vec<&String> = args.iter().take_while(|a| *a != "--sheet").collect();
    if files.is_empty() {
        eprintln!("usage: icons_check <name→svg JSON>... [--sheet out.png]");
        std::process::exit(2);
    }
    let mut icons = Vec::new();
    for f in files {
        let text = std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{f}: {e}"));
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{f}: {e}"));
        for (name, svg) in v.as_object().expect("a JSON object") {
            if let Some(s) = svg.as_str() {
                icons.push((name.clone(), s.to_string()));
            }
        }
    }
    let t0 = Instant::now();
    let mut parsed = Vec::new();
    let mut failed = Vec::new();
    for (name, src) in &icons {
        match Svg::parse(src) {
            Ok(s) => parsed.push((name.clone(), s)),
            Err(e) => failed.push(format!("{name}: {e}")),
        }
    }
    let parse_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let mut out = Vec::new();
    let mut raster = |side: usize| {
        let t = Instant::now();
        let mut ink = 0usize;
        for (_, s) in &parsed {
            out.clear();
            out = s.rasterize(side, side, Color::rgba(1.0, 1.0, 1.0, 1.0));
            ink += out.chunks_exact(4).filter(|p| p[3] > 0).count();
        }
        (t.elapsed().as_secs_f64() * 1000.0, ink)
    };
    let (r16, _) = raster(16);
    let (r24, ink) = raster(24);
    let (r48, _) = raster(48);
    let empty: Vec<&String> = parsed
        .iter()
        .filter(|(_, s)| s.rasterize(24, 24, Color::rgba(1.0, 1.0, 1.0, 1.0)).chunks_exact(4).all(|p| p[3] == 0))
        .map(|(n, _)| n)
        .collect();
    let n = parsed.len().max(1) as f64;
    println!(
        "{{\"icons\":{},\"parsed\":{},\"failed\":{},\"empty_at_24px\":{},\"parse_ms\":{parse_ms:.1},\"parse_us_per_icon\":{:.1},\"raster_us_per_icon\":{{\"16\":{:.1},\"24\":{:.1},\"48\":{:.1}}},\"ink_px_24\":{ink}}}",
        icons.len(),
        parsed.len(),
        failed.len(),
        empty.len(),
        parse_ms * 1000.0 / n,
        r16 * 1000.0 / n,
        r24 * 1000.0 / n,
        r48 * 1000.0 / n
    );
    for f in &failed {
        eprintln!("FAIL {f}");
    }
    for e in &empty {
        eprintln!("EMPTY {e}");
    }
    if let Some(path) = sheet {
        let (cols, cell) = (40usize, 28usize);
        let rows = parsed.len().min(cols * 50).div_ceil(cols);
        let mut c = Canvas::new(cols * cell, rows * cell);
        c.clear(Color::rgba(0.07, 0.07, 0.07, 1.0));
        for (i, (_, s)) in parsed.iter().take(cols * rows).enumerate() {
            let (x, y) = ((i % cols * cell) as f32 + 2.0, (i / cols * cell) as f32 + 2.0);
            s.paint(&mut c, x, y, 24.0, 24.0, Color::rgba(0.75, 0.75, 0.75, 1.0));
        }
        let mut px = Vec::new();
        c.to_rgba8(&mut px);
        fltk_omarchy::png::write(std::path::Path::new(&path), c.width() as u32, c.height() as u32, 4, &px).expect("write sheet");
        eprintln!("sheet written to {path}");
    }
    if !failed.is_empty() {
        std::process::exit(1);
    }
}
