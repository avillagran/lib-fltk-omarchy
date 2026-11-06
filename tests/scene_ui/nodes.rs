use super::*;
use fltk_omarchy::scene::{Layer, NodeError, NodeId, Surface};

fn card() -> Layer {
    let (content, ()) = Scene::record(|| {
        theme::with(|t| {
            paint::fill(0, 0, 70, 45, 8, t.palette.accent);
            paint::text(
                "Node",
                8,
                4,
                55,
                28,
                13,
                true,
                t.palette.foreground,
                Align::Left,
            );
        });
    });
    Layer::new(Rect::new(0, 0, 70, 45), content).unwrap()
}
fn content(card: &Layer, moved: bool) -> Scene {
    Scene::record(|| {
        theme::with(|t| {
            paint::fill(0, 0, 320, 240, 0, t.palette.background);
            paint::fill(0, 70, 320, 100, 0, t.palette.dim);
            // This unkeyed primitive is culled by GL preparation.
            paint::fill(-200, -200, 20, 20, 0, t.palette.accent);
        });
        scene::clip(Rect::new(20, 10, 200, 150), || {
            card.draw_keyed(
                NodeId(91),
                if moved { 180 } else { -100 },
                35,
                if moved { 0.4 } else { 0.0 },
            )
            .unwrap();
            card.draw_keyed(
                NodeId(7),
                if moved { 60 } else { 30 },
                90,
                if moved { 0.6 } else { 1.0 },
            )
            .unwrap();
        });
    })
    .0
}
pub(super) fn checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "scene nodes CPU: stable updates match fresh recording and clear damage caches",
        &|shots| {
            let card = card();
            let mut win = AppWindow::new("Nodes", 320, 240);
            let mut body = win.body();
            body.set_margin(0);
            let view = Surface::new(280, 180);
            view.set_scene(content(&card, false));
            body.add(&*view);
            win.end();
            win.snapshot(shots.join("nodes-before.png"))
                .map_err(|e| e.to_string())?;
            let builds = view.stats().scene_builds;
            view.move_node(NodeId(91), 180, 35)
                .map_err(|e| e.to_string())?;
            view.set_node_opacity(NodeId(91), 0.4)
                .map_err(|e| e.to_string())?;
            view.move_node(NodeId(7), 60, 90)
                .map_err(|e| e.to_string())?;
            view.set_node_opacity(NodeId(7), 0.6)
                .map_err(|e| e.to_string())?;
            let updated = shots.join("nodes-updated.png");
            win.snapshot(&updated).map_err(|e| e.to_string())?;
            check(
                view.stats().scene_builds == builds,
                "CPU node updates retain scene",
            )?;
            check(
                view.move_node(NodeId(8), 0, 0) == Err(NodeError::NotFound(NodeId(8))),
                "missing ID is an error",
            )?;
            view.set_scene(content(&card, true));
            let reference = shots.join("nodes-reference.png");
            win.snapshot(&reference).map_err(|e| e.to_string())?;
            check(
                png_rgb(&updated)?.raw == png_rgb(&reference)?.raw,
                "all updated CPU pixels equal fresh recording, including ancestor clip",
            )?;
            // Populate an independent whole-scene damage raster, then move
            // its nodes. Reusing the old backing would leave stale pixels.
            let mut retained = content(&card, false);
            let surf = fltk::surface::ImageSurface::new(320, 240, false);
            fltk::surface::ImageSurface::push_current(&surf);
            check(
                retained.draw_damage(Rect::new(0, 0, 320, 240)),
                "initial damage cache",
            )?;
            retained
                .move_node(NodeId(91), 180, 35)
                .map_err(|e| e.to_string())?;
            retained
                .set_node_opacity(NodeId(91), 0.4)
                .map_err(|e| e.to_string())?;
            retained
                .move_node(NodeId(7), 60, 90)
                .map_err(|e| e.to_string())?;
            retained
                .set_node_opacity(NodeId(7), 0.6)
                .map_err(|e| e.to_string())?;
            check(
                retained.draw_damage(Rect::new(0, 0, 320, 240)),
                "updated damage cache",
            )?;
            let damaged = surf.image().unwrap().to_rgb_data();
            content(&card, true).draw();
            let expected = surf.image().unwrap().to_rgb_data();
            fltk::surface::ImageSurface::pop_current();
            check(
                damaged == expected,
                "node updates invalidate whole-scene damage backing",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    #[cfg(feature = "gpu")]
    run(
        "scene nodes GPU: culled and zero-alpha keys move without preparation or upload",
        &|shots| {
            use fltk_omarchy::{gfx::Backend, App};
            use std::{
                cell::Cell,
                rc::Rc,
                time::{Duration, Instant},
            };
            let wait = |test: &mut dyn FnMut() -> bool| -> bool {
                let end = Instant::now() + Duration::from_secs(5);
                while Instant::now() < end {
                    App::pump();
                    if test() {
                        return true;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                false
            };
            let card = card();
            let mut win = AppWindow::new("GL nodes", 320, 240);
            let mut body = win.body();
            body.set_margin(0);
            let view = Surface::new(280, 180).backend(Backend::Gpu);
            view.set_scene(content(&card, false));
            body.add(&*view);
            win.end();
            win.show();
            check(
                wait(&mut || view.stats().gpu_frames > 0),
                &format!("activate GL: {:?}", view.backend_error()),
            )?;
            App::pump();
            let before = view.stats();
            view.move_node(NodeId(91), 180, 35)
                .map_err(|e| e.to_string())?;
            view.set_node_opacity(NodeId(91), 0.4)
                .map_err(|e| e.to_string())?;
            view.move_node(NodeId(7), 60, 90)
                .map_err(|e| e.to_string())?;
            view.set_node_opacity(NodeId(7), 0.6)
                .map_err(|e| e.to_string())?;
            let pixel = Rc::new(Cell::new(None));
            let result = pixel.clone();
            check(
                view.read_gpu_pixel(195, 45, move |p| result.set(Some(p))),
                "read updated GL node",
            )?;
            check(wait(&mut || pixel.get().is_some()), "node update rendered")?;
            let after = view.stats();
            check(
                after.scene_builds == before.scene_builds
                    && after.gpu_preparations == before.gpu_preparations
                    && after.gpu_scene_uploads == before.gpu_scene_uploads,
                "keyed GL updates do not record, prepare or upload",
            )?;
            check(
                after.surface_draws == before.surface_draws,
                "keyed updates redraw GL child only",
            )?;
            let shot = shots.join("nodes-gl-snapshot.png");
            win.snapshot(&shot).map_err(|e| e.to_string())?;
            let cpu = png_rgb(&shot)?.at(view.x() + 195, view.y() + 45);
            let gpu = pixel.get().unwrap();
            check(
                (0..3).all(|i| cpu[i].abs_diff(gpu[i]) <= 2),
                &format!("CPU snapshot agrees with moved/faded GL layer: {cpu:?}/{gpu:?}"),
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
}
