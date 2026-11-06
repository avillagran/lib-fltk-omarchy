//! UI checks of layers: offscreen capture, real opacity, effects on the
//! picture, cross-fades.
use super::{check, pixel, pump_until};
use fltk::{enums::FrameType, prelude::*};
use fltk_omarchy::gfx::{Canvas, Color, Path};
use fltk_omarchy::layer::{self, Layer};
use fltk_omarchy::theme::{self, Rgb};
use fltk_omarchy::AppWindow;
use std::path::Path as FsPath;

type Check<'a> = &'a dyn Fn(&FsPath) -> Result<(), String>;

fn near(p: [u8; 3], c: Rgb) -> bool {
    p[0].abs_diff(c.r()) <= 3 && p[1].abs_diff(c.g()) <= 3 && p[2].abs_diff(c.b()) <= 3
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("layer: capture, real opacity, effect on the picture, cross-fade", &|shots| {
        let (accent, bg) = theme::with(|t| (t.palette.accent, t.palette.background));
        let mut win = AppWindow::new("Layer", 360, 240);
        let mut body = win.body();
        body.set_margin(0);
        let lay = Layer::new(0, 0, 300, 120);
        let mut block = fltk::frame::Frame::new(0, 0, 300, 120, None);
        block.set_frame(FrameType::FlatBox);
        block.set_color(accent.fl());
        lay.end();
        body.add(&*lay);
        win.end();
        win.show();
        let l2 = lay.clone();
        pump_until(|| l2.w() > 0);
        block.resize(lay.x(), lay.y(), lay.w(), lay.h());
        let (lx, ly) = (lay.x() + 50, lay.y() + 60);
        let rx = lay.x() + lay.w() - 50;
        // capture: the picture of a widget.
        let pic = layer::capture(&block).ok_or("capture")?;
        let c = pic.pixel(10, 10);
        check((c.r * 255.0).round() as u8 == accent.r() && (c.b * 255.0).round() as u8 == accent.b(), &format!("captured accent: {c:?}"))?;
        let shot = |name: &str| -> Result<std::path::PathBuf, String> {
            let p = shots.join(name);
            win.clone().snapshot(&p).map_err(|e| e.to_string())?;
            Ok(p)
        };
        let p = shot("layer-opaque.png")?;
        check(near(pixel(&p, lx, ly)?, accent), "opaque layer draws normally")?;
        lay.set_opacity(0.5);
        let p = shot("layer-half.png")?;
        let want = accent.over(bg, 0.5);
        let got = pixel(&p, lx, ly)?;
        check(near(got, want), &format!("half opacity blends with the background: {got:?} vs {want:?}"))?;
        lay.set_opacity(1.0);
        // An effect that covers the right half with the background (a reveal at 50 %).
        lay.set_params([0.5, 0.0, 0.0, 0.0]);
        lay.effect(|c: &mut Canvas, u| {
            let w = c.width() as f32 / c.unit();
            c.fill(&Path::rect(w * u.params[0], 0.0, w, c.height() as f32), u.background);
        });
        let p = shot("layer-effect.png")?;
        check(near(pixel(&p, lx, ly)?, accent) && near(pixel(&p, rx, ly)?, bg), "effect: left revealed, right covered")?;
        lay.clear_effect();
        let p = shot("layer-cleared.png")?;
        check(near(pixel(&p, rx, ly)?, accent), "clear_effect draws normally again")?;
        // cross_fade
        let mut a = Canvas::new(4, 4);
        a.clear(Color::rgba(1.0, 0.0, 0.0, 1.0));
        let mut b = Canvas::new(4, 4);
        b.clear(Color::rgba(0.0, 0.0, 1.0, 1.0));
        let m = layer::cross_fade(&a, &b, 0.5).pixel(1, 1);
        check((m.r - 0.5).abs() < 0.02 && (m.b - 0.5).abs() < 0.02, &format!("cross-fade halfway: {m:?}"))?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}
