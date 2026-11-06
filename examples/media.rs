//! Images and icons: thumbnails (cover, contain, letterbox), an animated
//! GIF and WebP, the error placeholder, Lucide icons tinted by theme role,
//! and the viewer (Enter or click on a thumbnail; +/-/0, arrows, Ctrl+C).
//!   cargo run --example media --features images
//!   cargo run --example media --features images -- photo.jpg other.webp
//!   cargo run --example media --features images -- --snapshot build/shots
use fltk_omarchy::icon::{lucide, Icon, Svg};
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::media::{self, Fit, Image, Source};
use fltk_omarchy::text::ColorRole;
use fltk_omarchy::widgets::*;
use fltk_omarchy::{App, AppWindow};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn fixture(dir: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(dir).join(name)
}

fn thumb(src: &Source, name: &str, fit: Fit) -> Image {
    let img = Image::new(132, 100).source(src).fit(fit).framed(true).label(name);
    let i2 = img.clone();
    img.on_activate(move || {
        if let Some(v) = i2.viewer() {
            v.on_copy(|b| println!("copy: {}×{} RGBA ({} bytes as PNG)", b.width, b.height, b.to_png().len())).open(&*i2);
        }
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let snapshot = args.iter().skip_while(|a| *a != "--snapshot").nth(1).cloned();
    let files: Vec<String> = args.iter().take_while(|a| *a != "--snapshot").cloned().collect();
    let app = App::new("dev.omarchy.fltk.media");
    let mut win = AppWindow::new("Images and icons", 640, 560);
    let mut body = win.body();

    let sources: Vec<(Source, String, Fit)> = if files.is_empty() {
        vec![
            (Source::file(fixture("media", "quad.png")), "quad.png".into(), Fit::Cover),
            (Source::file(fixture("media", "quad-rot6.jpg")), "quad-rot6.jpg (EXIF)".into(), Fit::Contain),
            (Source::file(fixture("media", "quad-alpha.webp")), "quad-alpha.webp".into(), Fit::Letterbox),
            (Source::file(fixture("media", "rgb.gif")), "rgb.gif".into(), Fit::Cover),
        ]
    } else {
        files.iter().map(|f| (Source::file(f), f.clone(), Fit::Cover)).collect()
    };

    body.fit(&*Label::section("Thumbnails"));
    let mut row = layout::row();
    let mut thumbs = Vec::new();
    for (src, name, fit) in sources.iter().take(4) {
        let t = thumb(src, name, *fit);
        row.fit(&*t);
        thumbs.push(t);
    }
    layout::spacer();
    row.close_fitted();
    body.fixed(&row, 100);

    body.fit(&*Label::section("Animation, WebP, errors"));
    let mut row = layout::row();
    let anim = Image::new(100, 100).source(&Source::file(fixture("media", "rg.webp"))).fit(Fit::Cover).label("rg.webp");
    let broken = Image::new(100, 100).source(&Source::bytes("not-an-image", b"plain text".to_vec())).framed(true).label("broken.png");
    let small = Image::new(100, 100).source(&Source::file(fixture("media", "quad.jpg"))).fit(Fit::Contain).rounded(false);
    row.fit(&*anim);
    row.fit(&*broken);
    row.fit(&*small);
    layout::spacer();
    row.close_fitted();
    body.fixed(&row, 100);

    body.fit(&*Label::section("Icons (Lucide, tinted by theme role)"));
    let mut row = layout::row();
    let roles = [ColorRole::Foreground, ColorRole::Dim, ColorRole::Accent, ColorRole::Urgent, ColorRole::Green, ColorRole::Blue];
    let builtins = [lucide::IMAGE, lucide::FOLDER, lucide::SEARCH, lucide::SETTINGS, lucide::COPY, lucide::CIRCLE_ALERT];
    for (src, role) in builtins.iter().zip(roles) {
        if let Ok(svg) = Svg::cached(src) {
            row.fit(&*Icon::new(&svg, 24).tint(role));
        }
    }
    for name in ["lucide-rocket.svg", "lucide-star.svg", "lucide-codepen.svg", "ui-t3-logo.svg"] {
        if let Ok(svg) = std::fs::read_to_string(fixture("icons", name)).map_err(|e| e.to_string()).and_then(|s| Svg::parse(&s).map_err(|e| e.to_string())) {
            row.fit(&*Icon::new(&svg, 24).tint(ColorRole::Accent).label(name));
        }
    }
    if let Ok(svg) = Svg::cached(lucide::ZOOM_IN) {
        let t0 = thumbs.first().cloned();
        row.fit(&*Icon::new(&svg, 16).tooltip("Open the first image").on_click(move || {
            if let Some(t) = &t0 {
                t.activate();
            }
        }));
    }
    layout::spacer();
    row.close_fitted();
    body.fixed(&row, 32);
    body.fit(&*Label::new("Tab to a thumbnail and press Enter to open the viewer.").tone(Tone::Dim));
    layout::spacer();
    win.end();

    if let Some(dir) = snapshot {
        let dir = PathBuf::from(dir);
        win.snapshot(dir.join("media.png")).expect("snapshot");
        let end = Instant::now() + Duration::from_secs(5);
        while media::cache_stats().pending > 0 && Instant::now() < end {
            App::pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        win.snapshot(dir.join("media.png")).expect("snapshot");
        if let Some(v) = thumbs[0].viewer() {
            let h = v.on_copy(|_| {}).open(&*thumbs[0]).expect("viewer");
            while !h.is_loaded() && Instant::now() < end {
                App::pump();
                std::thread::sleep(Duration::from_millis(10));
            }
            win.snapshot(dir.join("media-viewer.png")).expect("snapshot");
            h.set_zoom(4.0);
            win.snapshot(dir.join("media-viewer-zoom.png")).expect("snapshot");
            h.close();
        }
        println!("snapshots written to {}", dir.display());
        return;
    }
    win.show();
    app.run();
}
