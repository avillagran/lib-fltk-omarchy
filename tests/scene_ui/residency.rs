//! Immutable resources survive managed painter revisions; changed content
//! and invalidated layers get new identities, while hide releases the cache.
use super::{
    check,
    overlays::{parity, wait},
    Check,
};
use fltk::{enums::Align, prelude::*};
use fltk_omarchy::{
    gfx::{sl, Backend, Uniforms},
    paint,
    scene::{self, Image, Layer, Rect, Scene, Surface},
    theme, AppWindow, State,
};

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("scene residency GPU: painter revisions reuse immutable images and programs; content and hide invalidate", &|shots| {
        let mut win = AppWindow::new("Scene residency", 340, 240);
        let mut body = win.body(); body.set_margin(0);
        let shared = Image::rgba(2, 2, vec![255,0,0,128, 0,255,0,255, 0,0,255,0, 255,255,255,190]).unwrap();
        let (text, ()) = Scene::record(|| paint::text("café 日本語", 0, 0, 180, 28, 14, false, theme::current().palette.foreground, Align::Left));
        let text = Layer::new(Rect::new(0, 0, 180, 28), text).unwrap();
        let changed = State::new(0usize);
        let variants = [shared.clone(), Image::rgba(1, 1, vec![128,32,220,190]).unwrap()];
        let value = changed.clone();
        let layer = text.clone();
        let shader = sl::Shader::new(|_, u| u.background());
        let view = Surface::new(340, 190).backend(Backend::Gpu).paint(move |r, t| {
            scene::shader(r, &shader, &Uniforms::from_theme(t, (r.w as f32, r.h as f32)));
            shared.draw(Rect::new(10, 60, 24, 24));
            variants[value.get()].draw(Rect::new(44, 60, 24, 24));
            layer.draw_at(10, 12, 0.8);
        });
        body.add(&*view); win.end(); view.set_time(Some(0.0)); win.show();
        check(wait(|| view.stats().gpu_frames > 0), &format!("residency ready: {:?}", view.backend_error()))?;
        parity(&mut win, &view, shots, "scene-residency-initial")?;
        for revision in 0..8 {
            let before = view.stats();
            view.invalidate();
            check(wait(|| view.stats().gpu_scene_uploads > before.gpu_scene_uploads), "painter revision installed")?;
            let after = view.stats();
            check(after.gpu_texture_uploads == before.gpu_texture_uploads && after.gpu_program_links == before.gpu_program_links,
                &format!("same resources do not upload/link in revision {revision}: {before:?} -> {after:?}"))?;
            check(after.gpu_image_reuses >= before.gpu_image_reuses + 2, "deduplicated image and text layer retained")?;
        }
        let before = view.stats();
        changed.set(1); view.invalidate();
        parity(&mut win, &view, shots, "scene-residency-changed")?;
        let after = view.stats();
        check(after.gpu_texture_uploads == before.gpu_texture_uploads + 1, &format!("only changed image uploads: {before:?} -> {after:?}"))?;
        check(after.gpu_program_links == before.gpu_program_links && after.gpu_image_reuses >= before.gpu_image_reuses + 2, "other images and program survive content replacement")?;
        let before = view.stats(); text.clear_raster(); view.invalidate();
        parity(&mut win, &view, shots, "scene-residency-raster-invalidated")?;
        check(view.stats().gpu_texture_uploads == before.gpu_texture_uploads + 1, "clearing a native layer raster invalidates its GPU identity")?;
        let before = view.stats(); view.clone().hide();
        check(wait(|| view.stats().texture_bytes == 0), "hide releases resident pages")?;
        view.clone().show(); win.redraw();
        check(wait(|| view.stats().gpu_frames > before.gpu_frames), "show recreates the native renderer")?;
        check(view.stats().gpu_program_links > before.gpu_program_links && view.stats().gpu_texture_uploads > before.gpu_texture_uploads, "context recreation never reuses retired GL names")?;
        parity(&mut win, &view, shots, "scene-residency-recovered")?;
        fltk::window::DoubleWindow::delete((*win).clone()); Ok(())
    });
}
