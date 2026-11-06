//! Managed scene composition: same local-coordinate painter on CPU or GL.
//! cargo run --example scene_surface -- --snapshot target/surface.png
//! cargo run --features gpu --example scene_surface
//! With --cpu, or without a working GL backend, the demo stays static.
//! --bench target/report.json samples idle/animation/idle process CPU;
//! --size WIDTHxHEIGHT changes the diagnostic window size.
use fltk::{app, enums::Align, prelude::*};
use fltk_omarchy::{
    a11y::AccessibleExt,
    anim::Pace,
    gfx::{self, Backend},
    i18n, paint,
    scene::{self, Rect, Scene, Surface},
    tr, App, AppWindow,
};

fn main() {
    let app = App::new("dev.omarchy.fltk.scene-surface");
    let args: Vec<_> = std::env::args().collect();
    let bench = args
        .iter()
        .position(|a| a == "--bench")
        .map(|i| args.get(i + 1).expect("--bench needs a JSON path").clone());
    let snapshot = args
        .iter()
        .position(|a| a == "--snapshot")
        .and_then(|i| args.get(i + 1));
    let cpu = args.iter().any(|a| a == "--cpu");
    let size = args
        .iter()
        .position(|a| a == "--size")
        .map(|i| {
            let (w, h) = args
                .get(i + 1)
                .and_then(|s| s.split_once('x'))
                .expect("--size WIDTHxHEIGHT");
            (
                w.parse::<i32>().expect("width"),
                h.parse::<i32>().expect("height"),
            )
        })
        .unwrap_or((420, 340));
    assert!(
        size.0 >= 240 && size.1 >= 160 && size.0 <= 8192 && size.1 <= 8192,
        "size out of bounds"
    );
    let mut win = AppWindow::new(&tr!("Scene composition"), size.0, size.1);
    win.bind_title(&i18n::text("Scene composition"));
    let mut body = win.body();
    body.set_margin(0);
    let shader = gfx::shaders::gradient();
    let view = Surface::new(380, 280)
        .backend(if cpu { Backend::Cpu } else { Backend::Gpu })
        .paint(move |r, t| {
            paint::fill(0, 0, r.w, r.h, 0, t.palette.background);
            let uniforms = gfx::Uniforms::from_theme(t, (r.w as f32, r.h as f32));
            scene::shader(r, &shader, &uniforms);
            let cw = (r.w - 48).max(1);
            let (card, ()) = Scene::record(|| {
                paint::surface(
                    0,
                    0,
                    cw,
                    84,
                    t.radius,
                    Some(t.palette.background),
                    Some((t.palette.accent, 1)),
                );
                paint::text(
                    &tr!("Retained scene"),
                    16,
                    12,
                    cw - 32,
                    26,
                    16,
                    true,
                    t.palette.foreground,
                    Align::Left,
                );
                paint::text(
                    &tr!("One surface · live theme"),
                    16,
                    43,
                    cw - 32,
                    22,
                    12,
                    false,
                    t.palette.foreground,
                    Align::Left,
                );
            });
            let card =
                scene::Layer::new(Rect::new(0, 0, cw, 84), card).expect("bounded card layer");
            assert!(card.draw_at(24, 24, 0.85));
            assert!(card.draw_at(24, 124, 0.5));
        })
        .accessible_name(&tr!("Scene composition"));
    let v = view.clone();
    i18n::bind(&*view, move || {
        v.clone().accessible_name(&tr!("Scene composition"));
    });
    body.add(&*view);
    win.end();
    if let Some(path) = snapshot {
        fltk_omarchy::anim::set_enabled(false);
        view.set_time(Some(0.7));
        win.snapshot(path).expect("scene surface snapshot");
    } else {
        win.show();
        // Event-driven activation check: CPU-only/error fallback stays
        // static, and a slow GL startup does not miss a fixed deadline.
        if let Some(path) = bench {
            let (v, w) = (view.clone(), win.clone());
            app::add_check(move |handle| {
                if v.was_deleted() {
                    app::remove_check(handle);
                    return;
                }
                let ready = if cpu {
                    v.stats().cpu_frames > 0
                } else {
                    v.active_backend() == Backend::Gpu && v.stats().gpu_frames > 0
                };
                if ready {
                    start_bench(v.clone(), w.clone(), path.clone());
                    app::remove_check(handle);
                } else if let Some(error) = v.backend_error() {
                    eprintln!("{error}");
                    app::remove_check(handle);
                    app::quit();
                }
            });
        } else if cfg!(feature = "gpu") && !cpu {
            let v = view.clone();
            app::add_check(move |handle| {
                if v.was_deleted() || v.backend_error().is_some() {
                    app::remove_check(handle);
                    return;
                }
                if v.active_backend() == Backend::Gpu && v.stats().gpu_frames > 0 {
                    v.clone().animate(Some(Pace::Display));
                    app::remove_check(handle);
                }
            });
        }
        app.run();
    }
}

fn start_bench(view: Surface, mut window: AppWindow, path: String) {
    fltk_omarchy::anim::set_enabled(true);
    use std::{cell::RefCell, rc::Rc, time::Instant};
    let cpu = || {
        fltk_omarchy::platform::process_cpu_time()
            .unwrap_or_default()
            .as_secs_f64()
    };
    let phase_path = std::path::PathBuf::from(format!("{path}.phase"));
    std::fs::write(&phase_path, "warmup").expect("benchmark phase");
    let sample = Rc::new(RefCell::new((cpu(), Instant::now(), view.stats())));
    let report = Rc::new(RefCell::new(serde_json::Map::new()));
    let (s, v, phase) = (sample.clone(), view.clone(), phase_path.clone());
    app::add_timeout3(0.5, move |_| {
        std::fs::write(&phase, "idle").expect("benchmark phase");
        *s.borrow_mut() = (cpu(), Instant::now(), v.stats());
    });
    let (s, v, r) = (sample.clone(), view.clone(), report.clone());
    app::add_timeout3(2.5, move |_| {
        let (c, at, stats) = *s.borrow();
        r.borrow_mut().insert(
            "idle_cpu_percent".into(),
            serde_json::json!((cpu() - c) / at.elapsed().as_secs_f64() * 100.0),
        );
        r.borrow_mut().insert(
            "idle_frames".into(),
            serde_json::json!(
                v.stats().gpu_frames + v.stats().cpu_frames - stats.gpu_frames - stats.cpu_frames
            ),
        );
        v.clone().animate(Some(Pace::Normal));
    });
    let (s, v, phase) = (sample.clone(), view.clone(), phase_path.clone());
    app::add_timeout3(3.5, move |_| {
        std::fs::write(&phase, "animation").expect("benchmark phase");
        *s.borrow_mut() = (cpu(), Instant::now(), v.stats());
    });
    let (s, v, r) = (sample.clone(), view.clone(), report.clone());
    app::add_timeout3(6.5, move |_| {
        let (c, at, before) = *s.borrow();
        let elapsed = at.elapsed().as_secs_f64();
        let after = v.stats();
        r.borrow_mut().insert(
            "animation_cpu_percent".into(),
            serde_json::json!((cpu() - c) / elapsed * 100.0),
        );
        r.borrow_mut().insert(
            "animation_fps".into(),
            serde_json::json!(
                (after.gpu_frames + after.cpu_frames - before.gpu_frames - before.cpu_frames)
                    as f64
                    / elapsed
            ),
        );
        r.borrow_mut().insert(
            "animation_parent_draws".into(),
            serde_json::json!(after.surface_draws - before.surface_draws),
        );
        r.borrow_mut().insert(
            "animation_scene_builds".into(),
            serde_json::json!(after.scene_builds - before.scene_builds),
        );
        r.borrow_mut().insert(
            "texture_bytes".into(),
            serde_json::json!(after.texture_bytes),
        );
        v.clone().animate(None);
    });
    let (s, v, phase) = (sample.clone(), view.clone(), phase_path.clone());
    app::add_timeout3(7.5, move |_| {
        std::fs::write(&phase, "idle_again").expect("benchmark phase");
        *s.borrow_mut() = (cpu(), Instant::now(), v.stats());
    });
    app::add_timeout3(9.5, move |_| {
        let (c, at, before) = *sample.borrow();
        let after = view.stats();
        let mut r = report.borrow_mut();
        r.insert(
            "idle_again_cpu_percent".into(),
            serde_json::json!((cpu() - c) / at.elapsed().as_secs_f64() * 100.0),
        );
        r.insert(
            "idle_again_frames".into(),
            serde_json::json!(
                after.gpu_frames + after.cpu_frames - before.gpu_frames - before.cpu_frames
            ),
        );
        r.insert(
            "backend".into(),
            serde_json::json!(format!("{:?}", view.active_backend())),
        );
        r.insert("size".into(), serde_json::json!([view.w(), view.h()]));
        r.insert("renderer".into(), serde_json::json!(view.renderer_name()));
        r.insert(
            "scope".into(),
            serde_json::json!(
                "two-card managed surface; not the 5000-message or lava efficiency gate"
            ),
        );
        let data = serde_json::to_string(&*r).unwrap();
        println!("{data}");
        std::fs::write(&path, data).expect("benchmark output");
        window.hide();
    });
}
