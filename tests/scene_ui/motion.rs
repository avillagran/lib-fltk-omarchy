use super::*;
use fltk_omarchy::{
    anim::{self, clock, Ease},
    scene::{Layer, NodeId, NodeMotion, Surface},
    App, State,
};
use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

struct Motion;
impl Motion {
    fn on() -> Self {
        anim::set_enabled(true);
        clock::freeze();
        Self
    }
}
impl Drop for Motion {
    fn drop(&mut self) {
        clock::resume();
        anim::set_enabled(false);
    }
}
fn card() -> Layer {
    let (scene, ()) =
        Scene::record(|| theme::with(|t| paint::fill(0, 0, 70, 45, 8, t.palette.accent)));
    Layer::new(Rect::new(0, 0, 70, 45), scene).unwrap()
}
fn scene(card: &Layer, x: i32, y: i32, opacity: f32) -> Scene {
    Scene::record(|| {
        theme::with(|t| {
            paint::fill(0, 0, 320, 240, 0, t.palette.background);
            paint::fill(0, 70, 320, 90, 0, t.palette.dim);
        });
        card.draw_keyed(NodeId(1), x, y, opacity).unwrap();
    })
    .0
}
fn shell() -> (AppWindow, Surface) {
    let mut win = AppWindow::new("Node motion", 320, 240);
    let mut body = win.body();
    body.set_margin(0);
    let surface = Surface::new(280, 180);
    body.add(&*surface);
    win.end();
    (win, surface)
}
fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        App::pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn wait(mut ready: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        App::pump();
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

pub(super) fn checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "scene images: shared resampling is independent of destination and outer clipping",
        &|_| {
            let image = scene::Image::rgba(
                2,
                2,
                vec![
                    255, 0, 0, 128, 0, 255, 0, 255, 0, 0, 255, 0, 255, 255, 255, 64,
                ],
            )
            .unwrap();
            let surface = fltk::surface::ImageSurface::new(140, 170, false);
            fltk::surface::ImageSurface::push_current(&surface);
            paint::fill(0, 0, 140, 170, 0, theme::Rgb(0));
            image.draw(Rect::new(0, 0, 20, 20));
            image.draw(Rect::new(100, 130, 20, 20));
            let pixels = surface.image().unwrap().to_rgb_data();
            let mut equal = true;
            for y in 0..20 {
                for x in 0..20 {
                    let a = (y * 140 + x) * 3;
                    let b = ((y + 130) * 140 + x + 100) * 3;
                    equal &= pixels[a..a + 3] == pixels[b..b + 3];
                }
            }
            paint::fill(100, 130, 20, 20, 0, theme::Rgb(0));
            scene::clip(Rect::new(105, 135, 10, 10), || {
                image.draw(Rect::new(100, 130, 20, 20))
            });
            let clipped = surface.image().unwrap().to_rgb_data();
            for y in 5..15 {
                for x in 5..15 {
                    let a = (y * 140 + x) * 3;
                    let b = ((y + 130) * 140 + x + 100) * 3;
                    equal &= pixels[a..a + 3] == clipped[b..b + 3];
                }
            }
            fltk::surface::ImageSurface::pop_current();
            check(
                equal,
                "same RGBA samples at both origins and under a smaller clip",
            )
        },
    );
    run(
        "scene motion: frozen frames, retarget, painter invalidation and deferred completion",
        &|shots| {
            let _motion = Motion::on();
            let card = card();
            let target = State::new((20, 40, 1.0f32));
            let t = target.clone();
            let c = card.clone();
            let (mut win, view) = shell();
            let view = view.paint(move |_, _| {
                let (x, y, a) = t.get();
                c.draw_keyed(NodeId(1), x, y, a).unwrap();
            });
            let initial = shots.join("scene-motion-start.png");
            win.snapshot(&initial).map_err(|e| e.to_string())?;
            let old = Rc::new(Cell::new(0));
            let o = old.clone();
            target.set((180, 120, 0.2));
            view.animate_node(
                NodeId(1),
                NodeMotion::new()
                    .position(180, 120)
                    .opacity(0.2)
                    .duration_ms(200)
                    .ease(Ease::Linear)
                    .on_done(move |_, _| o.set(o.get() + 1)),
            )
            .map_err(|e| e.to_string())?;
            let first = shots.join("scene-motion-first.png");
            win.snapshot(&first).map_err(|e| e.to_string())?;
            check(
                png_rgb(&initial)?.raw == png_rgb(&first)?.raw,
                "first motion frame has no jump",
            )?;
            clock::advance(Duration::from_millis(100));
            view.invalidate();
            let middle = shots.join("scene-motion-middle.png");
            win.snapshot(&middle).map_err(|e| e.to_string())?;
            check(
                view.node_bounds(NodeId(1)).map_err(|e| e.to_string())?
                    == Rect::new(100, 80, 70, 45),
                "active transition survives painter rebuild",
            )?;
            check(
                (view.node_opacity(NodeId(1)).map_err(|e| e.to_string())? - 0.6).abs()
                    <= 1.0 / 255.0,
                "opacity interpolates with the same clock",
            )?;
            let done = Rc::new(Cell::new(0));
            let d = done.clone();
            target.set((40, 80, 0.8));
            view.animate_node(
                NodeId(1),
                NodeMotion::new()
                    .position(40, 80)
                    .opacity(0.8)
                    .duration_ms(100)
                    .ease(Ease::Linear)
                    .on_done(move |v, id| {
                        v.raise_node(id).unwrap();
                        d.set(d.get() + 1);
                    }),
            )
            .map_err(|e| e.to_string())?;
            let retarget = shots.join("scene-motion-retarget.png");
            win.snapshot(&retarget).map_err(|e| e.to_string())?;
            check(
                png_rgb(&middle)?.raw == png_rgb(&retarget)?.raw,
                "retarget starts from current pose",
            )?;
            clock::advance(Duration::from_millis(50));
            win.snapshot(shots.join("scene-motion-retarget-mid.png"))
                .map_err(|e| e.to_string())?;
            check(
                view.node_bounds(NodeId(1)).map_err(|e| e.to_string())?.x == 70,
                "retarget midpoint follows new destination",
            )?;
            clock::advance(Duration::from_millis(50));
            let final_shot = shots.join("scene-motion-final.png");
            win.snapshot(&final_shot).map_err(|e| e.to_string())?;
            check(
                !view.node_animating(NodeId(1)) && done.get() == 0,
                "completion waits for a later UI turn",
            )?;
            let (mut reference, r) = shell();
            r.set_scene(
                Scene::record(|| {
                    card.draw_keyed(NodeId(1), 40, 80, 0.8).unwrap();
                })
                .0,
            );
            let ref_shot = shots.join("scene-motion-reference.png");
            reference.snapshot(&ref_shot).map_err(|e| e.to_string())?;
            check(
                png_rgb(&final_shot)?.raw == png_rgb(&ref_shot)?.raw,
                "final pixels equal static reference",
            )?;
            pump(30);
            check(
                done.get() == 1 && old.get() == 0,
                "one completion; interrupted callback dropped; callback updates surface safely",
            )?;
            fltk::window::DoubleWindow::delete((*reference).clone());
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run(
        "scene motion: static CPU scene paces, settles on reduced motion/hide, then idles",
        &|shots| {
            let _motion = Motion::on();
            clock::resume();
            let (mut win, view) = shell();
            view.set_scene(scene(&card(), 20, 40, 1.0));
            win.show();
            check(wait(|| view.stats().cpu_frames > 0), "initial CPU frame")?;
            let mut signature = (view.w(), view.h(), view.stats().scene_builds);
            let mut stable = Instant::now();
            check(
                wait(|| {
                    let now = (view.w(), view.h(), view.stats().scene_builds);
                    if now != signature {
                        signature = now;
                        stable = Instant::now();
                    }
                    stable.elapsed() >= Duration::from_millis(120)
                }),
                "initial size/focus settles before motion sample",
            )?;
            let builds = view.stats().scene_builds;
            let before = view.stats().cpu_frames;
            let done = Rc::new(Cell::new(0));
            let d = done.clone();
            view.animate_node(
                NodeId(1),
                NodeMotion::new()
                    .position(180, 80)
                    .opacity(0.3)
                    .duration_ms(160)
                    .on_done(move |_, _| d.set(d.get() + 1)),
            )
            .map_err(|e| e.to_string())?;
            check(
                wait(|| !view.node_animating(NodeId(1)) && done.get() == 1),
                "managed CPU transition completes",
            )?;
            check(
                view.stats().cpu_frames >= before + 2 && view.stats().scene_builds == builds,
                "CPU motion paces without rebuilding static painter",
            )?;
            pump(120);
            let frames = view.stats().cpu_frames;
            pump(180);
            check(
                view.stats().cpu_frames == frames,
                "completed scene stops requesting frames",
            )?;
            view.animate_node(
                NodeId(1),
                NodeMotion::new().position(30, 110).duration_ms(1000),
            )
            .map_err(|e| e.to_string())?;
            anim::set_enabled(false);
            check(
                wait(|| !view.node_animating(NodeId(1))),
                "reduced motion settles immediately",
            )?;
            check(
                view.node_bounds(NodeId(1)).map_err(|e| e.to_string())?.x == 30,
                "reduced motion reaches target",
            )?;
            anim::set_enabled(true);
            let hidden = Rc::new(Cell::new(false));
            let h = hidden.clone();
            view.animate_node(
                NodeId(1),
                NodeMotion::new()
                    .position(90, 120)
                    .duration_ms(1000)
                    .on_done(move |_, _| h.set(true)),
            )
            .map_err(|e| e.to_string())?;
            win.hide();
            check(
                wait(|| hidden.get()),
                "hidden transition settles and completes",
            )?;
            let frames = view.stats().cpu_frames;
            pump(180);
            check(
                view.stats().cpu_frames == frames && !view.node_animating(NodeId(1)),
                "hidden static scene has no frame loop",
            )?;
            check(
                view.node_bounds(NodeId(1)).map_err(|e| e.to_string())?.x == 90,
                "hidden target retained",
            )?;
            let stale = Rc::new(Cell::new(false));
            let s = stale.clone();
            view.animate_node(
                NodeId(1),
                NodeMotion::new()
                    .duration_ms(0)
                    .on_done(move |_, _| s.set(true)),
            )
            .map_err(|e| e.to_string())?;
            view.move_node(NodeId(1), 40, 70)
                .map_err(|e| e.to_string())?;
            pump(30);
            check(
                !stale.get(),
                "immediate setter cancels already queued completion",
            )?;
            view.animate_node(
                NodeId(1),
                NodeMotion::new().position(110, 90).duration_ms(1000),
            )
            .map_err(|e| e.to_string())?;
            check(
                !view.node_animating(NodeId(1))
                    && view.node_bounds(NodeId(1)).map_err(|e| e.to_string())?.x == 110,
                "new transitions settle while already hidden",
            )?;
            win.snapshot(shots.join("scene-motion-hidden.png"))
                .map_err(|e| e.to_string())?;
            let replaced = Rc::new(Cell::new(false));
            let r = replaced.clone();
            view.animate_node(
                NodeId(1),
                NodeMotion::new()
                    .duration_ms(0)
                    .on_done(move |_, _| r.set(true)),
            )
            .map_err(|e| e.to_string())?;
            view.set_scene(scene(&card(), 20, 40, 1.0));
            pump(30);
            check(
                !replaced.get(),
                "replacing content drops already queued callbacks",
            )?;
            let deleted = Rc::new(Cell::new(false));
            let d = deleted.clone();
            view.animate_node(
                NodeId(1),
                NodeMotion::new()
                    .duration_ms(0)
                    .on_done(move |_, _| d.set(true)),
            )
            .map_err(|e| e.to_string())?;
            fltk::window::DoubleWindow::delete((*win).clone());
            pump(30);
            check(!deleted.get(), "deleted surface drops deferred callbacks")?;
            Ok(())
        },
    );
    #[cfg(feature = "gpu")]
    run(
        "scene motion GL: frozen parity, no uploads/parent redraws and completion idles",
        &|shots| {
            use fltk_omarchy::gfx::Backend;
            let _motion = Motion::on();
            let (mut win, view) = shell();
            let view = view.backend(Backend::Gpu);
            view.set_scene(scene(&card(), 20, 40, 1.0));
            win.show();
            check(wait(|| view.stats().gpu_frames > 0), "GL activated")?;
            let before = view.stats();
            view.animate_node(
                NodeId(1),
                NodeMotion::new()
                    .position(160, 100)
                    .opacity(0.4)
                    .duration_ms(200)
                    .ease(Ease::Linear),
            )
            .map_err(|e| e.to_string())?;
            clock::advance(Duration::from_millis(100));
            let result = Rc::new(Cell::new(None));
            let r = result.clone();
            check(
                view.read_gpu_pixel(120, 90, move |p| r.set(Some(p))),
                "read moving GL layer",
            )?;
            check(wait(|| result.get().is_some()), "GL midpoint drawn")?;
            let after = view.stats();
            check(
                after.scene_builds == before.scene_builds
                    && after.surface_draws == before.surface_draws
                    && after.gpu_preparations == before.gpu_preparations
                    && after.gpu_scene_uploads == before.gpu_scene_uploads,
                "GL transition only updates retained values",
            )?;
            let shot = shots.join("scene-motion-gl-mid.png");
            win.snapshot(&shot).map_err(|e| e.to_string())?;
            let cpu = png_rgb(&shot)?.at(view.x() + 120, view.y() + 90);
            let gpu = result.get().unwrap();
            check(
                (0..3).all(|i| cpu[i].abs_diff(gpu[i]) <= 2),
                &format!("GL/CPU moving midpoint: {cpu:?}/{gpu:?}"),
            )?;
            let midpoint = view.node_bounds(NodeId(1)).map_err(|e| e.to_string())?;
            let frames = view.stats().cpu_frames;
            let view = view.backend(Backend::Cpu);
            check(
                wait(|| view.stats().cpu_frames > frames),
                "moving scene switches to CPU",
            )?;
            check(
                view.node_animating(NodeId(1))
                    && view.node_bounds(NodeId(1)).map_err(|e| e.to_string())? == midpoint,
                "CPU policy preserves active transition phase",
            )?;
            let frames = view.stats().gpu_frames;
            let view = view.backend(Backend::Gpu);
            check(
                wait(|| view.stats().gpu_frames > frames),
                "moving scene recovers GL",
            )?;
            check(
                view.node_bounds(NodeId(1)).map_err(|e| e.to_string())? == midpoint,
                "GL recovery preserves frozen transition phase",
            )?;
            view.cancel_node_motion(NodeId(1))
                .map_err(|e| e.to_string())?;
            clock::resume();
            let done = Rc::new(Cell::new(false));
            let d = done.clone();
            let before = view.stats();
            view.animate_node(
                NodeId(1),
                NodeMotion::new()
                    .position(200, 40)
                    .opacity(0.8)
                    .duration_ms(160)
                    .on_done(move |v, id| {
                        v.set_node_opacity(id, 0.75).unwrap();
                        d.set(true);
                    }),
            )
            .map_err(|e| e.to_string())?;
            check(
                wait(|| done.get()),
                "GL completion can update its own node outside draw",
            )?;
            check(
                view.stats().gpu_frames >= before.gpu_frames + 2
                    && view.stats().surface_draws == before.surface_draws,
                "managed GL pacing does not repaint CPU parent",
            )?;
            pump(120);
            let frames = view.stats().gpu_frames;
            pump(180);
            check(
                view.stats().gpu_frames == frames,
                "finished GL motion stops pacing",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
}
