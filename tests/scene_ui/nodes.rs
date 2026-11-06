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
fn ordered(raised: bool) -> Scene {
    let colors = theme::with(|t| {
        [
            t.palette.accent,
            t.palette.foreground,
            t.palette.dim,
            t.palette.yellow,
        ]
    });
    let layers: Vec<_> = colors
        .into_iter()
        .map(|color| {
            let (content, ()) = Scene::record(|| paint::fill(0, 0, 70, 45, 8, color));
            Layer::new(Rect::new(0, 0, 70, 45), content).unwrap()
        })
        .collect();
    Scene::record(|| {
        theme::with(|t| paint::fill(0, 0, 320, 240, 0, t.palette.background));
        if !raised {
            layers[3].draw_keyed(NodeId(4), 55, 48, 0.6).unwrap();
        }
        scene::clip(Rect::new(10, 20, 140, 100), || {
            if !raised {
                layers[0].draw_keyed(NodeId(1), 20, 35, 1.0).unwrap();
            }
            scene::clip(Rect::new(35, 35, 50, 50), || {
                layers[1].draw_keyed(NodeId(2), 30, 30, 1.0).unwrap();
            });
            layers[2].draw_keyed(NodeId(3), 50, 45, 1.0).unwrap();
            if raised {
                layers[0].draw_keyed(NodeId(1), 20, 35, 1.0).unwrap();
            }
        });
        if raised {
            layers[3].draw_keyed(NodeId(4), 55, 48, 0.6).unwrap();
        }
    })
    .0
}
pub(super) fn checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "scene nodes order: raising and hits respect nested scopes; callbacks update their surface",
        &|shots| {
            let mut win = AppWindow::new("Node order", 320, 240);
            let mut body = win.body();
            body.set_margin(0);
            let picked = std::rc::Rc::new(std::cell::Cell::new(None));
            let p = picked.clone();
            let view = Surface::new(280, 180).on_event_with(move |view, event, x, y| {
                if event != fltk::enums::Event::Push {
                    return false;
                }
                if let Some(id) = view.hit_node(x, y).unwrap() {
                    p.set(Some(id));
                    view.raise_node(id).unwrap();
                    view.set_node_opacity(id, 0.0).unwrap();
                    return true;
                }
                false
            });
            view.set_scene(ordered(false));
            body.add(&*view);
            win.end();
            win.snapshot(shots.join("node-order-before.png"))
                .map_err(|e| e.to_string())?;
            check(
                view.hit_node(60, 55) == Ok(Some(NodeId(3))),
                "hit follows original paint order",
            )?;
            let builds = view.stats().scene_builds;
            view.raise_node(NodeId(1)).map_err(|e| e.to_string())?;
            view.raise_node(NodeId(2)).map_err(|e| e.to_string())?;
            check(
                view.hit_node(60, 55) == Ok(Some(NodeId(1))),
                "raised child stays below later content outside its own scope",
            )?;
            view.raise_node(NodeId(4)).map_err(|e| e.to_string())?;
            view.raise_node(NodeId(1)).map_err(|e| e.to_string())?;
            check(
                view.hit_node(60, 55) == Ok(Some(NodeId(4))),
                "root node remains above later child raise",
            )?;
            check(
                view.hit_node(-1, 40) == Ok(None) && view.hit_node(view.w(), 40) == Ok(None),
                "surface viewport excludes outside hits",
            )?;
            let actual = shots.join("node-order-raised.png");
            win.snapshot(&actual).map_err(|e| e.to_string())?;
            check(
                view.stats().scene_builds == builds,
                "raising preserves the CPU painter",
            )?;
            view.set_scene(ordered(true));
            let reference = shots.join("node-order-reference.png");
            win.snapshot(&reference).map_err(|e| e.to_string())?;
            check(
                png_rgb(&actual)?.raw == png_rgb(&reference)?.raw,
                "raised CPU pixels match independent fresh paint order",
            )?;
            fltk_omarchy::focus::click_at(&*win, view.x() + 60, view.y() + 55);
            check(
                picked.get() == Some(NodeId(4)) && view.hit_node(60, 55) == Ok(Some(NodeId(1))),
                "callback safely hits, raises and hides its own surface node",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    #[cfg(feature = "gpu")]
    run(
        "scene nodes GL order: scoped raises retain uploads and focus chrome",
        &|shots| {
            use fltk_omarchy::{focus, gfx::Backend, App};
            use std::{
                cell::Cell,
                rc::Rc,
                time::{Duration, Instant},
            };
            let wait = |test: &mut dyn FnMut() -> bool| {
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
            let mut win = AppWindow::new("GL node order", 320, 240);
            let mut body = win.body();
            body.set_margin(0);
            let view = Surface::new(280, 180).backend(Backend::Gpu);
            view.set_scene(ordered(false));
            body.add(&*view);
            win.end();
            win.show();
            check(
                wait(&mut || view.stats().gpu_frames > 0),
                "node order GL activates",
            )?;
            App::pump();
            let before = view.stats();
            view.raise_node(NodeId(1)).map_err(|e| e.to_string())?;
            view.raise_node(NodeId(2)).map_err(|e| e.to_string())?;
            view.raise_node(NodeId(4)).map_err(|e| e.to_string())?;
            view.raise_node(NodeId(1)).map_err(|e| e.to_string())?;
            let pixel = Rc::new(Cell::new(None));
            let p = pixel.clone();
            check(
                view.read_gpu_pixel(60, 55, move |v| p.set(Some(v))),
                "queue raised overlap pixel",
            )?;
            check(wait(&mut || pixel.get().is_some()), "raised GL draw")?;
            let after = view.stats();
            check(
                after.scene_builds == before.scene_builds
                    && after.gpu_preparations == before.gpu_preparations
                    && after.gpu_scene_uploads == before.gpu_scene_uploads
                    && after.surface_draws == before.surface_draws,
                "raising only changes retained GL order",
            )?;
            let shot = shots.join("node-order-gl.png");
            win.snapshot(&shot).map_err(|e| e.to_string())?;
            let cpu = png_rgb(&shot)?.at(view.x() + 60, view.y() + 55);
            let gpu = pixel.get().unwrap();
            check(
                (0..3).all(|i| cpu[i].abs_diff(gpu[i]) <= 2),
                &format!("scoped GL order matches CPU overlap: {cpu:?}/{gpu:?}"),
            )?;
            // Root nodes must not rise over managed focus chrome appended by
            // the host after the recorded scene's own ordering boundary.
            let (fill, ()) = Scene::record(|| {
                theme::with(|t| paint::fill(0, 0, 320, 240, 0, t.palette.foreground))
            });
            let layer = Layer::new(Rect::new(0, 0, 320, 240), fill).unwrap();
            let (scene, ()) = Scene::record(|| {
                layer.draw_keyed(NodeId(10), 0, 0, 1.0).unwrap();
            });
            view.set_scene(scene);
            view.clone().take_focus().map_err(|e| e.to_string())?;
            focus::press("Enter");
            let frames = view.stats().gpu_frames;
            check(
                wait(&mut || view.stats().gpu_frames > frames),
                "focus chrome prepared",
            )?;
            check(focus::show_ring(&*view), "keyboard focus ring is active")?;
            view.raise_node(NodeId(10)).map_err(|e| e.to_string())?;
            let edge = Rc::new(Cell::new(None));
            let p = edge.clone();
            check(
                view.read_gpu_pixel(1, 30, move |v| p.set(Some(v))),
                "read focus edge",
            )?;
            check(wait(&mut || edge.get().is_some()), "focused raise rendered")?;
            let expected = theme::with(|t| t.palette.accent);
            let rgb = [expected.r(), expected.g(), expected.b()];
            let actual = edge.get().unwrap();
            check(
                (0..3).all(|i| actual[i].abs_diff(rgb[i]) <= 2),
                &format!("raised root stays under focus chrome: {actual:?}/{rgb:?}"),
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
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
