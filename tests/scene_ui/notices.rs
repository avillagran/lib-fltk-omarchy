//! Actual GPU overlay pixels, native-child routing and retained resources.
use super::super::check;
use super::overlays::{parity, wait};
use fltk::{enums::Event, prelude::*};
use fltk_omarchy::{
    anim, focus,
    gfx::{self, Backend},
    overlay::{Notice, NoticeAction},
    scene::{self, Surface},
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
        "scene notice GPU: app-owned actions, live text, retained overlay and full release",
        &|shots| {
            struct Restore;
            impl Drop for Restore {
                fn drop(&mut self) {
                    anim::set_enabled(false);
                }
            }
            let _restore = Restore;
            anim::set_enabled(false);
            let mut win = AppWindow::new("GPU notices", 640, 460);
            let mut body = win.body();
            body.set_margin(0);
            let scene_clicks = Rc::new(Cell::new(0));
            let behind = scene_clicks.clone();
            let shader = gfx::shaders::gradient();
            let surface = Surface::new(640, 419)
                .backend(Backend::Gpu)
                .on_event(move |e, _, _| {
                    if e == Event::Push {
                        behind.set(behind.get() + 1);
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
                wait(|| surface.stats().gpu_frames > 0 && surface.active_backend() == Backend::Gpu),
                "GL ready",
            )?;
            let text = State::new("A persistent notice above the scene".to_owned());
            let updated = text.clone();
            let calls = Rc::new(Cell::new(0));
            let out = calls.clone();
            let notice = Notice::new("Choose an action", "")
                .bind_body(&text)
                .action(NoticeAction::new("retry", "Retry"))
                .on_action(move |_, id| {
                    assert_eq!(id, "retry");
                    out.set(out.get() + 1);
                    updated.set("The action updated this live text".into());
                })
                .show(&*surface)
                .ok_or("notice")?;
            parity(&mut win, &surface, shots, "scene-notice-initial")?;
            let (x, y, w, h) = notice.action_rect("retry").ok_or("action geometry")?;
            let child = surface.child(0).ok_or("native child")?;
            let gl = fltk::window::GlWindow::from_dyn_widget_ptr(child.as_widget_ptr())
                .ok_or("GL child")?;
            let (x, y) = (x + w / 2 - surface.x(), y + h / 2 - surface.y());
            for button in [2, 3] {
                focus::click_button_at(&gl, x, y, button);
            }
            check(calls.get()==0&&scene_clicks.get()==0&&notice.is_open(),"native-child auxiliary events stay with notice without primary action or scene click")?;
            focus::click_at(&gl, x, y);
            App::pump();
            check(
                calls.get() == 1 && scene_clicks.get() == 0 && notice.is_open(),
                "native-child primary action reaches app once without implicit dismissal",
            )?;
            parity(&mut win, &surface, shots, "scene-notice-updated")?;
            focus::hover_at(&gl, x, y);
            pump(40);
            parity(&mut win, &surface, shots, "scene-notice-hover")?;
            // Offscreen window rendering lays out its children; let that
            // pending native repaint finish before measuring pointer-only work.
            pump(50);
            let hover = surface.stats();
            for _ in 0..8 {
                focus::hover_at(&gl, x, y);
                App::pump();
            }
            check(
                surface.stats().overlay_captures == hover.overlay_captures
                    && surface.stats().overlay_uploads == hover.overlay_uploads
                    && surface.stats().surface_draws == hover.surface_draws,
                &format!(
                    "same action hit: recaptures={}, uploads={}, parent_draws={}",
                    surface.stats().overlay_captures - hover.overlay_captures,
                    surface.stats().overlay_uploads - hover.overlay_uploads,
                    surface.stats().surface_draws - hover.surface_draws
                ),
            )?;
            focus::hover_at(&gl, x, y - 20);
            pump(40);
            parity(&mut win, &surface, shots, "scene-notice-hover-cleared")?;
            anim::set_enabled(true);
            surface.clone().animate(Some(anim::Pace::Display));
            pump(150);
            let before = surface.stats();
            pump(240);
            let after = surface.stats();
            check(
                after.gpu_frames > before.gpu_frames
                    && after.overlay_captures == before.overlay_captures
                    && after.overlay_uploads == before.overlay_uploads
                    && after.surface_draws == before.surface_draws,
                "shader frames retain notice texture without recapture/upload/parent draw",
            )?;
            surface.clone().animate(None);
            pump(50);
            let idle = surface.stats();
            pump(80);
            let idle_frames = surface.stats().gpu_frames - idle.gpu_frames;
            check(
                idle_frames == 0,
                "persistent notice schedules no idle frames",
            )?;
            notice.close();
            check(
                wait(|| surface.stats().overlay_texture_bytes == 0),
                "notice close releases its overlay texture",
            )?;
            check(
                surface.active_backend() == Backend::Gpu && calls.get() == 1,
                "closing notice preserves surface backend and action count",
            )?;
            println!(
                "notice GPU {:?}: shader_frames={}, recaptures={}, uploads={}, parent_draws={}, idle_frames={}, remaining_overlay_bytes={}",
                surface.renderer_name(), after.gpu_frames-before.gpu_frames,
                after.overlay_captures-before.overlay_captures,
                after.overlay_uploads-before.overlay_uploads,
                after.surface_draws-before.surface_draws,
                idle_frames,
                surface.stats().overlay_texture_bytes,
            );
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
}
