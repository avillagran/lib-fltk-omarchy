#[path = "../../examples/support/lava_ui.rs"]
mod demo;
use super::check;
#[cfg(feature = "gpu")]
use super::png_rgb;
use fltk::prelude::*;
use fltk_omarchy::{
    anim::{self, clock},
    focus, App,
};
#[cfg(feature = "gpu")]
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};
use std::{path::Path, time::Duration};
type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;
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
fn frame(demo: &mut demo::Lava, shots: &Path, name: &str, ms: u64) -> Result<(), String> {
    clock::advance(Duration::from_millis(ms));
    demo.win
        .snapshot(shots.join(name))
        .map_err(|e| e.to_string())?;
    App::pump();
    Ok(())
}
fn center(d: &demo::Lava, key: u64) -> (i32, i32) {
    let r = d.core.row_rect(key).unwrap();
    (d.surface.x() + r.x + r.w / 2, d.surface.y() + r.y + 32)
}
pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "lava reference: live language, rounded cards and mirrored completion target",
        &|shots| {
            use fltk_omarchy::{i18n, theme};
            struct Restore(String, theme::Corners);
            impl Drop for Restore {
                fn drop(&mut self) {
                    i18n::set_locale(&self.0);
                    theme::set_corners(self.1);
                }
            }
            let _restore = Restore(i18n::locale().get(), theme::corners());
            anim::set_enabled(false);
            i18n::set_locale("en");
            theme::set_corners(theme::Corners::Rounded);
            let mut d = demo::Lava::new(true);
            frame(&mut d, shots, "lava-rounded.png", 0)?;
            let before = d.surface.stats().scene_builds;
            i18n::set_locale("es");
            frame(&mut d, shots, "lava-spanish.png", 0)?;
            check(
                d.win.label() == "Tareas de lava" && d.surface.stats().scene_builds > before,
                "live language updates title and retained content",
            )?;
            check(
                d.core
                    .panel
                    .item_label(d.core.model.id_at(0).unwrap())
                    .as_deref()
                    == Some("Publicar las notas de la versión"),
                "virtual row names follow the language",
            )?;
            i18n::set_locale("ar");
            frame(&mut d, shots, "lava-rtl.png", 0)?;
            let r = d.core.row_rect(1).unwrap();
            focus::click_at(
                &*d.win,
                d.surface.x() + r.x + r.w - 24,
                d.surface.y() + r.y + 32,
            );
            App::pump();
            check(
                d.core.keys() == [2, 3, 4, 5],
                "mirrored right-hand completion target removes the intended stable ID",
            )?;
            fltk::window::DoubleWindow::delete((*d.win).clone());
            Ok(())
        },
    );
    run(
        "lava reference: pointer reorder, detached preview, falling removal and gap",
        &|shots| {
            let _motion = Motion::on();
            let mut d = demo::Lava::new(true);
            frame(&mut d, shots, "lava-initial.png", 0)?;
            check(d.core.keys() == [1, 2, 3, 4, 5], "initial five tasks")?;
            let from = center(&d, 2);
            let to = center(&d, 4);
            focus::mouse_drag(&*d.win, from, to, false);
            frame(&mut d, shots, "lava-drag.png", 100)?;
            check(
                d.core.keys() == [1, 3, 4, 2, 5] && d.core.flying(),
                &format!("reorder keeps stable IDs: {:?}", d.core.keys()),
            )?;
            focus::mouse_up(&*d.win, to.0, to.1);
            frame(&mut d, shots, "lava-drop.png", 200)?;
            check(
                !d.core.flying() && d.core.model.len() == 5,
                "reorder settles without removal",
            )?;
            let from = center(&d, 2);
            let to = (from.0, d.surface.y() + d.surface.h() - 20);
            focus::mouse_drag(&*d.win, from, to, true);
            frame(&mut d, shots, "lava-falling.png", 180)?;
            check(
                d.core.model.len() == 5 && d.core.flying(),
                "row remains in model until its flight completes",
            )?;
            frame(&mut d, shots, "lava-removed.png", 300)?;
            frame(&mut d, shots, "lava-gap-closed.png", 220)?;
            check(
                d.core.keys() == [1, 3, 4, 5] && !d.core.flying(),
                "5→4 removal completes exactly once and gap closes",
            )?;
            let first = d.core.row_rect(5).unwrap();
            check(
                first.y > d.core.row_rect(4).unwrap().y,
                "remaining rows have stable positions",
            )?;
            fltk::window::DoubleWindow::delete((*d.win).clone());
            Ok(())
        },
    );
    run(
        "lava reference: keyboard pick/drop, reorder, cancel and reset",
        &|shots| {
            let _motion = Motion::on();
            let mut d = demo::Lava::new(true);
            frame(&mut d, shots, "lava-key-initial.png", 0)?;
            d.core.panel.focus();
            focus::press("Down");
            focus::press("Space");
            focus::press("Down");
            check(
                d.core.keys() == [1, 3, 2, 4, 5] && d.core.flying(),
                "keyboard drag moves the cursor task",
            )?;
            focus::press("Esc");
            frame(&mut d, shots, "lava-key-cancel.png", 220)?;
            check(
                d.core.keys() == [1, 2, 3, 4, 5] && !d.core.flying(),
                "Escape restores original order",
            )?;
            focus::press("Mod+Shift+Up");
            frame(&mut d, shots, "lava-key-reorder.png", 220)?;
            check(
                d.core.keys() == [2, 1, 3, 4, 5],
                &format!("shortcut reorders by stable cursor: {:?}", d.core.keys()),
            )?;
            focus::press("Delete");
            frame(&mut d, shots, "lava-key-fall.png", 480)?;
            frame(&mut d, shots, "lava-key-four.png", 220)?;
            check(
                d.core.keys() == [1, 3, 4, 5],
                "Delete removes selected task after animation",
            )?;
            focus::press("Mod+R");
            frame(&mut d, shots, "lava-key-reset.png", 0)?;
            check(
                d.core.keys() == [1, 2, 3, 4, 5] && !d.core.flying(),
                "reset cancels old flights and restores model",
            )?;
            d.core.benchmark_cycle(true);
            focus::press("Mod+R");
            frame(&mut d, shots, "lava-reset-during-flight.png", 800)?;
            check(
                d.core.model.len() == 5 && !d.core.flying(),
                "reset cancels deferred removal of the old model",
            )?;
            for _ in 0..5 {
                d.core.benchmark_cycle(true);
                frame(&mut d, shots, "lava-emptying.png", 480)?;
                frame(&mut d, shots, "lava-empty.png", 220)?;
            }
            check(
                d.core.model.is_empty() && d.core.panel.cursor().is_none() && !d.core.flying(),
                "last removal leaves an empty model and no cursor",
            )?;
            let weak = d.surface.downgrade();
            fltk::window::DoubleWindow::delete((*d.win).clone());
            check(
                weak.upgrade().is_none(),
                "weak surface does not upgrade a deleted native widget",
            )?;
            Ok(())
        },
    );
    #[cfg(feature = "gpu")]
    run(
        "lava reference GPU: drag/fall runs without preparing static cards; effect stops",
        &|shots| {
            let _motion = Motion::on();
            clock::resume();
            let mut d = demo::Lava::new(false);
            d.win.show();
            let wait = |ready: &mut dyn FnMut() -> bool| {
                let end = Instant::now() + Duration::from_secs(8);
                while Instant::now() < end {
                    App::pump();
                    if ready() {
                        return true;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                false
            };
            check(
                wait(&mut || d.surface.stats().gpu_frames > 0),
                &format!("GL starts: {:?}", d.surface.backend_error()),
            )?;
            let from = center(&d, 3);
            let to = center(&d, 4);
            focus::mouse_drag(&*d.win, from, to, false);
            App::pump();
            let before = d.surface.stats();
            let end = Instant::now() + Duration::from_millis(150);
            while Instant::now() < end {
                App::pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            let after = d.surface.stats();
            check(
                after.gpu_frames > before.gpu_frames
                    && after.gpu_preparations == before.gpu_preparations
                    && after.gpu_scene_uploads == before.gpu_scene_uploads,
                "drag updates shader/time and poses without uploads",
            )?;
            clock::freeze();
            let captured = Rc::new(RefCell::new(None));
            let reply = captured.clone();
            check(
                d.surface
                    .read_gpu_image(move |image| *reply.borrow_mut() = Some(image)),
                "queue actual GPU frame",
            )?;
            check(
                wait(&mut || captured.borrow().is_some()),
                "actual GPU capture completes",
            )?;
            let image = captured
                .borrow_mut()
                .take()
                .unwrap()
                .map_err(|e| e.to_string())?;
            fltk_omarchy::png::write(
                &shots.join("lava-gpu-flight.png"),
                image.width(),
                image.height(),
                4,
                image.pixels(),
            )
            .map_err(|e| e.to_string())?;
            check(
                image.width() == d.surface.w() as u32 && image.height() == d.surface.h() as u32,
                "capture uses physical framebuffer dimensions",
            )?;
            check(
                image.pixels().chunks_exact(4).all(|p| p[3] == 255),
                "opaque surface read-back has straight RGBA",
            )?;
            let cpu_path = shots.join("lava-cpu-flight-reference.png");
            d.win.snapshot(&cpu_path).map_err(|e| e.to_string())?;
            let cpu = png_rgb(&cpu_path)?;
            let mut delta = 0;
            for y in 0..d.surface.h() {
                for x in 0..d.surface.w() {
                    let p = cpu.at(x + d.surface.x(), y + d.surface.y());
                    let i = ((y * d.surface.w() + x) * 4) as usize;
                    for (c, value) in p.iter().enumerate() {
                        delta = delta.max(value.abs_diff(image.pixels()[i + c]));
                    }
                }
            }
            check(
                delta <= 2,
                &format!("full GPU lava frame agrees with CPU replay: max RGB delta {delta}"),
            )?;
            clock::resume();
            focus::mouse_up(&*d.win, to.0, d.surface.y() + d.surface.h() - 20);
            check(
                wait(&mut || d.core.model.len() == 4 && !d.core.flying()),
                "GPU fall completes and removes row",
            )?;
            let end = Instant::now() + Duration::from_millis(350);
            while Instant::now() < end {
                App::pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            let frames = d.surface.stats().gpu_frames;
            let end = Instant::now() + Duration::from_millis(180);
            while Instant::now() < end {
                App::pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            check(
                d.surface.stats().gpu_frames == frames,
                "bounded lava effect and motion stop producing frames",
            )?;
            d.win
                .snapshot(shots.join("lava-gpu-after.png"))
                .map_err(|e| e.to_string())?;
            let _ = png_rgb(&shots.join("lava-gpu-after.png"))?;
            let closed = Rc::new(Cell::new(false));
            let stale = Rc::new(Cell::new(false));
            let (done, window) = (closed.clone(), (*d.win).clone());
            check(
                d.surface.read_gpu_image(move |result| {
                    assert!(result.is_ok());
                    fltk::window::DoubleWindow::delete(window);
                    done.set(true);
                }),
                "queue window-closing capture callback",
            )?;
            let called = stale.clone();
            check(
                d.surface.read_gpu_image(move |_| called.set(true)),
                "queue following capture",
            )?;
            check(
                wait(&mut || closed.get()),
                "capture callback can close its native window on a later UI turn",
            )?;
            App::pump();
            check(
                !stale.get() && d.surface.downgrade().upgrade().is_none(),
                "deletion suppresses following queued callbacks",
            )?;
            Ok(())
        },
    );
}
