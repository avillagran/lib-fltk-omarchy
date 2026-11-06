//! UI checks of `icon` (SVG icons, always built) and `media` (images,
//! feature `images`): offscreen snapshots with pixel assertions, keys
//! through `focus::press`, animation frames on the frozen clock.
use super::{check, pixel, png_rgb};
#[cfg(feature = "images")]
use super::pump_until;
use fltk::prelude::*;
use fltk_omarchy::icon::{self, Icon, Svg};
#[cfg(any(feature = "images", feature = "a11y"))]
use fltk_omarchy::layout::FlexExt;
use fltk_omarchy::text::ColorRole;
#[cfg(feature = "images")]
use fltk_omarchy::widgets::*;
#[cfg(any(feature = "images", feature = "a11y"))]
use fltk_omarchy::AppWindow;
use fltk_omarchy::theme;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn fixtures(dir: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(dir)
}

/// A plain window (no flex layout): children keep the exact geometry the
/// pixel checks assume.
fn plain(w: i32, h: i32) -> fltk::window::DoubleWindow {
    fltk::window::DoubleWindow::new(0, 0, w, h, None)
}

fn shot(win: &mut fltk::window::DoubleWindow, p: &Path) -> Result<(), String> {
    fltk_omarchy::window::snapshot(win, p).map_err(|e| e.to_string())
}

fn rgb(c: theme::Rgb) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

fn close(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

/// Any pixel of the rectangle within `tol` of `want`.
fn any_near(p: &Path, r: (i32, i32, i32, i32), want: [u8; 3], tol: i32) -> Result<bool, String> {
    let img = png_rgb(p)?;
    for y in r.1..r.1 + r.3 {
        for x in r.0..r.0 + r.2 {
            if close(img.at(x, y), want, tol) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("icons: every Lucide / o3 fixture parses; tinted pixels at the icon's place; theme restyles live; raster cache", &|shots| {
        let mut parsed = Vec::new();
        for e in std::fs::read_dir(fixtures("icons")).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "svg") {
                let src = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;
                let svg = Svg::parse(&src).map_err(|e| format!("{}: {e}", p.display()))?;
                check(!svg.shapes().is_empty(), &format!("{} has shapes", p.display()))?;
                parsed.push((p.file_name().unwrap().to_string_lossy().to_string(), svg));
            }
        }
        check(parsed.len() >= 15, &format!("{} fixtures", parsed.len()))?;
        let by = |n: &str| parsed.iter().find(|(f, _)| f == n).map(|(_, s)| s.clone()).ok_or(format!("fixture {n}"));
        let image = by("lucide-image.svg")?;
        let logo = by("ui-t3-logo.svg")?;
        let mut win = plain(240, 80);
        let mut big = Icon::new(&image, 48).tint(ColorRole::Accent);
        big.set_pos(10, 10);
        let mut filled = Icon::new(&logo, 48).tint(ColorRole::Foreground);
        filled.set_pos(70, 10);
        let clicks = Rc::new(Cell::new(0));
        let c2 = clicks.clone();
        let mut button = Icon::new(&image, 16).tooltip("Attach image").on_click(move || c2.set(c2.get() + 1));
        button.set_pos(130, 10);
        win.end();
        let p = shots.join("icons.png");
        shot(&mut win, &p)?;
        let t = theme::current();
        // Lucide image: the frame's left stroke is centered on x = 3 of 24
        // (6 at 48 px, 4 px wide).
        let (x, y) = (big.x() + 6, big.y() + 24);
        let px = pixel(&p, x, y)?;
        check(close(px, rgb(t.palette.accent), 40), &format!("accent stroke at {x},{y}: {px:?}"))?;
        let inside = pixel(&p, big.x() + 30, big.y() + 12)?;
        check(close(inside, rgb(t.palette.background), 8), &format!("empty inside the frame: {inside:?}"))?;
        // The filled logo (non-square viewBox) is centered vertically: ink in the middle, none at the top.
        check(any_near(&p, (filled.x(), filled.y() + 18, 48, 12), rgb(t.palette.foreground), 30)?, "logo ink")?;
        check(!any_near(&p, (filled.x(), filled.y(), 48, 6), rgb(t.palette.foreground), 60)?, "letterboxed above the logo")?;
        let (_, _, hits0, _) = icon::cache_stats();
        shot(&mut win, &p)?;
        let (n, bytes, hits1, _) = icon::cache_stats();
        check(hits1 >= hits0 + 3 && n >= 3 && bytes > 0, &format!("second draw hits the cache: {hits0} → {hits1}, {n} entries"))?;
        // omarchy-theme-set: a new accent re-tints without touching the widget.
        let mut b = t.clone();
        b.palette.accent = theme::Rgb(0x33ccff);
        theme::set(b);
        let p2 = shots.join("icons-theme-b.png");
        shot(&mut win, &p2)?;
        let px2 = pixel(&p2, x, y)?;
        theme::set(t);
        check(close(px2, [0x33, 0xcc, 0xff], 40), &format!("restyled stroke: {px2:?}"))?;
        // An icon button: Tab stop, Enter and click run it.
        button.click();
        let _ = button.clone().take_focus();
        fltk_omarchy::focus::press("Enter");
        check(clicks.get() == 2, &format!("clicks {}", clicks.get()))?;
        check(button.w() >= 22, "button keeps the ActionButton size")
    });

    #[cfg(feature = "images")]
    media_checks(run);
}

#[cfg(feature = "images")]
fn media_checks(run: &mut dyn FnMut(&str, Check)) {
    use fltk_omarchy::anim::{self, clock};
    use fltk_omarchy::media::{self, Error, Fit, Image, Source};
    use std::time::Duration;

    let quad = || Source::file(fixtures("media").join("quad.png"));

    run("media: contain, cover and letterbox pixels; rounded corners; placeholder and error state", &|shots| {
        let t = theme::current();
        let mut win = plain(440, 160);
        let mut contain = Image::new(128, 64).source(&quad()).fit(Fit::Contain).rounded(false);
        contain.set_pos(10, 10);
        let mut cover = Image::new(128, 64).source(&quad()).fit(Fit::Cover);
        cover.set_pos(150, 10);
        let mut letter = Image::new(128, 64).source(&quad()).fit(Fit::Letterbox).rounded(false);
        letter.set_pos(290, 10);
        let mut bad = Image::new(64, 64).source(&Source::bytes("ui-not-an-image", b"plain text".to_vec())).label("broken.png");
        bad.set_pos(10, 84);
        win.end();
        let p = shots.join("media-fit.png");
        shot(&mut win, &p)?;
        check(contain.is_loading() || contain.decoded().is_some(), "the first draw starts the decode")?;
        pump_until(|| contain.decoded().is_some() && cover.decoded().is_some() && letter.decoded().is_some() && bad.error().is_some());
        check(bad.error() == Some(Error::Unsupported), &format!("error value: {:?}", bad.error()))?;
        shot(&mut win, &p)?;
        let at = |w: &Image, dx: i32, dy: i32| pixel(&p, w.x() + dx, w.y() + dy);
        let (red, green, blue, white) = ([255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]);
        // Contain: 64 × 48 into 128 × 64 → 85 × 64 centered (x 21..106).
        check(close(at(&contain, 30, 10)?, red, 10) && close(at(&contain, 98, 10)?, green, 10), "contain top")?;
        check(close(at(&contain, 30, 54)?, blue, 10) && close(at(&contain, 98, 54)?, white, 10), "contain bottom")?;
        check(close(at(&contain, 4, 32)?, rgb(t.palette.background), 4), &format!("contain shows the window beside: {:?}", at(&contain, 4, 32)?))?;
        // Cover: scale 2, crop 64 × 32 from the middle: fills the box.
        check(close(at(&cover, 6, 20)?, red, 10) && close(at(&cover, 122, 20)?, green, 10), "cover top")?;
        check(close(at(&cover, 6, 44)?, blue, 10) && close(at(&cover, 122, 44)?, white, 10), "cover bottom")?;
        check(close(at(&cover, 0, 0)?, rgb(t.palette.background), 30), &format!("rounded corner: {:?}", at(&cover, 0, 0)?))?;
        check(close(at(&cover, 10, 0)?, red, 60), &format!("past the corner the image starts: {:?}", at(&cover, 10, 0)?))?;
        // Letterbox: bars in darker_background.
        check(close(at(&letter, 4, 32)?, rgb(t.palette.darker_background), 4), &format!("letterbox bar {:?}", at(&letter, 4, 32)?))?;
        check(close(at(&letter, 30, 10)?, red, 10), "letterbox image")?;
        // Error placeholder: the alert icon in the urgent color.
        check(any_near(&p, (bad.x() + 20, bad.y() + 20, 24, 24), rgb(t.palette.urgent), 50)?, "alert icon in urgent")?;
        let s = media::cache_stats();
        check(s.entries >= 2 && s.bytes > 0 && s.bytes <= s.budget, &format!("{s:?}"))
    });

    run("media: animated GIF frames follow the animation clock; first frame with animations off", &|shots| {
        let mut win = plain(60, 60);
        let mut gif = Image::new(32, 32).source(&Source::file(fixtures("media").join("rgb.gif"))).rounded(false);
        gif.set_pos(10, 10);
        win.end();
        let p = shots.join("media-gif.png");
        shot(&mut win, &p)?;
        pump_until(|| gif.decoded().is_some());
        check(gif.decoded().is_some_and(|d| d.frames.len() == 3), "three frames")?;
        let px = |p: &Path| pixel(p, gif.x() + 16, gif.y() + 16);
        anim::set_enabled(true);
        clock::freeze();
        shot(&mut win, &p)?;
        let f0 = (gif.frame_index(), px(&p)?);
        clock::advance(Duration::from_millis(150));
        shot(&mut win, &p)?;
        let f1 = (gif.frame_index(), px(&p)?);
        clock::advance(Duration::from_millis(300));
        shot(&mut win, &p)?;
        let f2 = (gif.frame_index(), px(&p)?);
        clock::advance(Duration::from_millis(200));
        shot(&mut win, &p)?;
        let looped = gif.frame_index();
        anim::set_enabled(false);
        clock::advance(Duration::from_millis(150));
        shot(&mut win, &p)?;
        let off = (gif.frame_index(), px(&p)?);
        clock::resume();
        check(f0.0 == 0 && close(f0.1, [255, 0, 0], 10), &format!("frame 0 red: {f0:?}"))?;
        check(f1.0 == 1 && close(f1.1, [0, 255, 0], 10), &format!("frame 1 green after 150 ms: {f1:?}"))?;
        check(f2.0 == 2 && close(f2.1, [0, 0, 255], 10), &format!("frame 2 blue after 450 ms: {f2:?}"))?;
        check(looped == 0, &format!("loops after 600 ms: {looped}"))?;
        check(off.0 == 0 && close(off.1, [255, 0, 0], 10), &format!("animations off: first frame {off:?}"))
    });

    run("media: viewer opens over the window, zooms, pans, copies; Esc closes and focus comes back", &|shots| {
        let mut win = AppWindow::new("t", 480, 360);
        let mut body = win.body();
        let before = Button::new("Before");
        body.fit(&*before);
        let opened = Rc::new(std::cell::RefCell::new(None));
        let copied = Rc::new(Cell::new((0u32, 0u32)));
        let thumb = Image::new(96, 72).source(&quad()).label("quad.png");
        let (o2, c2, t2) = (opened.clone(), copied.clone(), thumb.clone());
        let thumb = thumb.on_activate(move || {
            let c3 = c2.clone();
            let v = t2.viewer().expect("viewer").on_copy(move |b| c3.set((b.width, b.height)));
            *o2.borrow_mut() = v.open(&*t2);
        });
        body.fit(&*thumb);
        win.end();
        let p = shots.join("media-viewer.png");
        win.snapshot(&p).map_err(|e| e.to_string())?;
        pump_until(|| thumb.decoded().is_some());
        let _ = thumb.clone().take_focus();
        fltk_omarchy::focus::press("Enter");
        let h = opened.borrow().clone().ok_or("viewer opened")?;
        pump_until(|| h.is_loaded());
        check(h.error().is_none(), &format!("{:?}", h.error()))?;
        win.snapshot(&p).map_err(|e| e.to_string())?;
        let (ix, iy, iw, ih) = h.image_rect().ok_or("image rect")?;
        // Small file: shown at its natural size (64 × 48), centered under the toolbar.
        check((iw, ih) == (64, 48), &format!("natural size {iw}×{ih}"))?;
        check(close(pixel(&p, ix + 8, iy + 8)?, [255, 0, 0], 12) && close(pixel(&p, ix + iw - 8, iy + ih - 8)?, [255, 255, 255], 12), "image pixels")?;
        fltk_omarchy::focus::press("=");
        fltk_omarchy::focus::press("=");
        check((h.zoom() - 1.5625).abs() < 1e-3, &format!("zoom in twice: {}", h.zoom()))?;
        let (_, _, w2, _) = h.image_rect().ok_or("rect")?;
        check(w2 == 100, &format!("zoomed width {w2}"))?;
        fltk_omarchy::focus::press("-");
        fltk_omarchy::focus::press("0");
        check(h.zoom() == 1.0 && h.pan() == (0.0, 0.0), "0 resets")?;
        // Pan only where the image is larger than the view.
        h.set_zoom(10.0);
        fltk_omarchy::focus::press("Left");
        check(h.pan().0 > 0.0, &format!("Left pans: {:?}", h.pan()))?;
        for _ in 0..40 {
            fltk_omarchy::focus::press("Left");
        }
        let (rx, _, _, _) = h.image_rect().ok_or("rect")?;
        check(rx <= 12, &format!("pan stops at the edge: {rx}"))?;
        win.snapshot(shots.join("media-viewer-zoom.png")).map_err(|e| e.to_string())?;
        fltk_omarchy::focus::press("0");
        fltk_omarchy::focus::press("Ctrl+c");
        check(copied.get() == (64, 48), &format!("copy hook got {:?}", copied.get()))?;
        // Tab walks the toolbar; Enter presses (Zoom out first).
        fltk_omarchy::focus::press("Tab");
        fltk_omarchy::focus::press("Enter");
        check(h.zoom() < 1.0, &format!("toolbar zoom out: {}", h.zoom()))?;
        fltk_omarchy::focus::press("Escape");
        check(!h.is_open(), "Esc closes")?;
        pump_until(|| fltk_omarchy::focus::is_focused(&*thumb));
        check(fltk_omarchy::focus::is_focused(&*thumb), "focus back on the thumbnail")?;
        let _ = before;
        Ok(())
    });
}

#[cfg(feature = "a11y")]
pub fn a11y_checks(run: &mut dyn FnMut(&str, Check)) {
    use fltk_omarchy::a11y;
    run("a11y: icons are decorative unless named; icon buttons are named by their tooltip", &|_| {
        let svg = Svg::cached(icon::lucide::FOLDER).map_err(|e| e.to_string())?;
        let mut win = AppWindow::new("t", 300, 100);
        let _body = win.body();
        let mut row = fltk_omarchy::layout::row();
        row.fit(&*Icon::new(&svg, 16));
        row.fit(&*Icon::new(&svg, 16).label("Project folder"));
        row.fit(&*Icon::new(&svg, 16).tooltip("Open folder").on_click(|| {}));
        row.close_fitted();
        win.end();
        let snap = a11y::snapshot(&*win).ok_or("no tree")?;
        check(snap.contains("Image \"Project folder\""), &format!("named icon:\n{snap}"))?;
        check(snap.contains("Button \"Open folder\""), &format!("icon button:\n{snap}"))?;
        check(snap.matches("Image").count() == 1, &format!("the unnamed icon is hidden:\n{snap}"))
    });
    #[cfg(feature = "images")]
    run("a11y: images are named or decorative; the viewer is a modal dialog with named buttons", &|_| {
        use fltk_omarchy::media::{Image, ImageViewer, Source};
        let src = Source::file(fixtures("media").join("quad.png"));
        let mut win = AppWindow::new("t", 400, 300);
        let mut body = win.body();
        let named = Image::new(64, 48).source(&src).label("quad.png");
        body.fit(&*named);
        body.fit(&*Image::new(64, 48).source(&src));
        win.end();
        let snap = a11y::snapshot(&*win).ok_or("no tree")?;
        check(snap.contains("Image \"quad.png\"") && snap.matches("Image").count() == 1, &format!("images:\n{snap}"))?;
        let h = ImageViewer::new(&src).name("quad.png").on_copy(|_| {}).open(&*named).ok_or("open")?;
        let snap = a11y::snapshot(&*win).ok_or("no tree")?;
        h.close();
        for want in ["Dialog \"quad.png\"", "Button \"Zoom in\"", "Button \"Zoom out\"", "Button \"Fit to window\"", "Button \"Copy image\"", "Button \"Close\""] {
            check(snap.contains(want), &format!("has {want:?}:\n{snap}"))?;
        }
        Ok(())
    });
}
