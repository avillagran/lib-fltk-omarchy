//! Zero-area scenes must stop native rendering and release resources,
//! even when FLTK culls their parent draw callback.
use super::{check, Check};
use fltk::prelude::*;
use fltk_omarchy::{
    anim,
    gfx::{sl, Backend, Uniforms},
    scene, App, AppWindow,
};
use std::time::{Duration, Instant};

fn pump_for(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        App::pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn wait(mut condition: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        App::pump();
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("scene lifecycle GPU: zero width/height stops frames and releases resources; positive area recovers", &|_| {
        struct Restore;
        impl Drop for Restore { fn drop(&mut self) { anim::set_enabled(false); } }
        let _restore = Restore;
        anim::set_enabled(true);
        let mut win = AppWindow::new("Scene lifecycle", 340, 240);
        let mut body = win.body();
        // Keep the tested region inside a plain group: resizing a direct
        // Flex child is a layout request that Flex can legitimately undo.
        let host = fltk::group::Group::default();
        let shader = sl::Shader::new(|_, u| sl::mix(u.background(), u.accent(), sl::clamp(u.time(), 0.0, 1.0)));
        let mut view = scene::Surface::new(300, 180).backend(Backend::Gpu).paint(move |r, t| {
            scene::shader(r, &shader, &Uniforms::from_theme(t, (r.w as f32, r.h as f32)));
            fltk_omarchy::paint::text("Resource", 8, 8, 100, 24, 14, false, t.palette.foreground, fltk::enums::Align::Left);
        }).animate(Some(anim::Pace::Low));
        host.end();
        body.add(&host);
        win.end();
        win.show();
        check(wait(|| view.stats().gpu_frames >= 3), "native scene starts")?;
        let (x, y, w, h) = (view.x(), view.y(), view.w(), view.h());
        for (width, height) in [(0, h), (w, 0)] {
            view.resize(x, y, width, height);
            win.redraw();
            check(wait(|| view.stats().texture_bytes == 0 && view.child(0).is_some_and(|c| !c.visible())), "zero area hides native child and releases textures")?;
            check(view.stats().instance_bytes == 0, "zero area releases the instance buffer")?;
            pump_for(150);
            let before = view.stats();
            pump_for(180);
            check((view.w(), view.h()) == (width, height), "host preserves the zero-area geometry")?;
            check(view.stats().gpu_frames == before.gpu_frames, &format!("zero area has no GPU frames: {} to {}", before.gpu_frames, view.stats().gpu_frames))?;
            check(view.stats().cpu_frames == before.cpu_frames, "zero area has no CPU fallback loop")?;
            check(!view.read_gpu_pixel(0, 0, |_| {}), "zero area cannot queue native readback")?;
            check(view.backend_error().is_none(), "zero area is a lifecycle state, not a driver failure")?;
            view.resize(x, y, w, h);
            win.redraw();
            check(wait(|| view.stats().gpu_frames >= before.gpu_frames + 3), "positive area restarts native rendering")?;
            check(view.stats().texture_bytes > 0, "positive area restores scene textures")?;
        }
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}
