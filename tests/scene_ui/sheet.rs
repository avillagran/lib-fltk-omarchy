//! Lazy sheets compose over native GL children and release cached GPU bytes.
use super::super::check;
use super::overlays::{parity, wait};
use fltk::prelude::*;
use fltk_omarchy::{
    anim, focus,
    gfx::{self, Backend},
    layout::FlexExt,
    overlay::Sheet,
    scene::{self, Surface},
    widgets::{Button, TextField},
    App, AppWindow, State,
};
use std::{
    cell::Cell,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};
type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;
fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        App::pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}
pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "scene sheet GPU: retained modal input, parity, warm reopening and hidden resource release",
        &|shots| {
            struct Restore;
            impl Drop for Restore {
                fn drop(&mut self) {
                    anim::clock::resume();
                    anim::set_enabled(false);
                }
            }
            let _restore = Restore;
            anim::set_enabled(false);
            let mut win = AppWindow::new("Sheet over GL", 620, 420);
            let mut body = win.body();
            let clicks = Rc::new(Cell::new(0));
            let out = clicks.clone();
            let shader = gfx::shaders::gradient();
            let surface = Surface::new(620, 360)
                .backend(Backend::Gpu)
                .on_event(move |e, _, _| {
                    if e == fltk::enums::Event::Push {
                        out.set(out.get() + 1);
                    }
                    false
                })
                .paint(move |r, t| {
                    scene::shader(
                        r,
                        &shader,
                        &gfx::Uniforms::from_theme(t, (r.w as f32, r.h as f32)),
                    );
                });
            body.add(&*surface);
            win.end();
            win.set_pos(73, 37);
            win.show();
            check(
                wait(|| surface.active_backend() == Backend::Gpu && surface.stats().gpu_frames > 0),
                "GL ready",
            )?;
            let draft = State::new("Retained draft".to_owned());
            let d = draft.clone();
            let calls = Rc::new(Cell::new(0));
            let out = calls.clone();
            let builds = Rc::new(Cell::new(0));
            let b = builds.clone();
            let sheet = Sheet::new(&*win, "Details above GL", 440, 250, move |col, _sheet| {
                b.set(b.get() + 1);
                col.fit(&*TextField::new().bind(&d));
                let n = out.clone();
                col.fit(&*Button::new("Apply").on_click(move || n.set(n.get() + 1)));
            });
            sheet.open();
            parity(&mut win, &surface, shots, "scene-sheet-open")?;
            let child = surface.child(0).ok_or("GL child")?;
            let gl = fltk::window::GlWindow::from_dyn_widget_ptr(child.as_widget_ptr())
                .ok_or("GL window")?;
            let (x, y, w, h) = sheet.close_rect().ok_or("close rect")?;
            let (x, y) = (x + w / 2 - surface.x(), y + h / 2 - surface.y());
            for button in [2, 3] {
                focus::click_button_at(&gl, x, y, button);
            }
            check(
                sheet.is_open() && calls.get() == 0 && clicks.get() == 0,
                "auxiliary event ownership above native child",
            )?;
            draft.set("Updated retained draft".into());
            parity(&mut win, &surface, shots, "scene-sheet-update")?;
            anim::set_enabled(true);
            surface.clone().animate(Some(anim::Pace::Display));
            pump(180);
            let before = surface.stats();
            pump(220);
            let after = surface.stats();
            check(
                after.gpu_frames > before.gpu_frames
                    && after.overlay_captures == before.overlay_captures
                    && after.overlay_uploads == before.overlay_uploads
                    && after.surface_draws == before.surface_draws,
                &format!(
                    "static sheet recaptured during shader: before={before:?}, after={after:?}"
                ),
            )?;
            surface.clone().animate(None);
            anim::set_enabled(false);
            focus::click_at(&gl, x, y);
            check(
                !sheet.is_open() && sheet.is_built() && clicks.get() == 0,
                "native-child primary close without scene click",
            )?;
            check(
                wait(|| surface.stats().overlay_texture_bytes == 0),
                "hidden sheet retained GPU texture",
            )?;
            pump(50);
            let idle = surface.stats();
            pump(80);
            let cached_idle_frames = surface.stats().gpu_frames - idle.gpu_frames;
            check(cached_idle_frames == 0, "cached sheet caused idle frames")?;
            sheet.open();
            check(builds.get() == 1, "warm reopen rebuilt content")?;
            parity(&mut win, &surface, shots, "scene-sheet-reopened")?;
            anim::set_enabled(true);
            anim::clock::freeze();
            sheet.close();
            anim::clock::advance(Duration::from_millis(30));
            parity(&mut win, &surface, shots, "scene-sheet-closing")?;
            sheet.open();
            parity(&mut win, &surface, shots, "scene-sheet-reversal")?;
            anim::clock::advance(Duration::from_millis(250));
            parity(&mut win, &surface, shots, "scene-sheet-reversal-finished")?;
            anim::set_enabled(false);
            anim::clock::resume();
            sheet.close();
            sheet.release();
            check(
                wait(|| surface.stats().overlay_texture_bytes == 0),
                "release retained overlay bytes",
            )?;
            println!("sheet GPU {:?}: shader_frames={}, captures={}, uploads={}, parent_draws={}, cached_idle_frames={}, remaining_overlay_bytes={}",surface.renderer_name(),after.gpu_frames-before.gpu_frames,after.overlay_captures-before.overlay_captures,after.overlay_uploads-before.overlay_uploads,after.surface_draws-before.surface_draws,cached_idle_frames,surface.stats().overlay_texture_bytes);
            win.hide();
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
}
