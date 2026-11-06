//! Compare complete native GL pixels after ordered instanced submission.
use super::{
    check,
    overlays::{parity, wait},
    Check,
};
use fltk::{enums::Align, prelude::*};
use fltk_omarchy::{
    gfx::{sl, Backend, Uniforms},
    paint,
    scene::{self, Layer, NodeId, Rect, Scene, Surface},
    theme, AppWindow,
};

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("scene atlas GPU: distinct filtered images share a batch without edge bleed", &|shots| {
        let mut win = AppWindow::new("Image page", 340, 280);
        let mut body = win.body();
        body.set_margin(0);
        let images: Vec<_> = (0..80u8).map(|i| scene::Image::rgba(2, 2, vec![
            255,i*3,0,128, 0,255,i*2,255, i*2,0,255,0, 255,255,i*3,190,
        ]).unwrap()).collect();
        let view = Surface::new(340,240).backend(Backend::Gpu).paint(move |r,t| {
            paint::fill(0,0,r.w,r.h,0,t.palette.background);
            for (i,image) in images.iter().enumerate() {
                image.draw(Rect::new(4+(i%20) as i32*16, 8+(i/20) as i32*18,14,16));
            }
            images[0].draw(Rect::new(22,100,14,16));
        });
        body.add(&*view); win.end(); win.show();
        check(wait(||view.stats().gpu_frames>0), &format!("atlas ready: {:?}",view.backend_error()))?;
        let before=view.stats();
        parity(&mut win,&view,shots,"scene-atlas-images")?;
        let after=view.stats();let frames=after.gpu_frames-before.gpu_frames;
        check(frames>0 && after.gpu_draw_calls-before.gpu_draw_calls<=frames*2,"81 distinct/reused image draws share one ordered image batch")?;
        check(after.instance_bytes>0,"image page uses retained UV instance records")?;
        std::fs::write(shots.join("scene-atlas-report.json"),serde_json::json!({"frames":frames,"primitives":after.gpu_primitives-before.gpu_primitives,"draw_calls":after.gpu_draw_calls-before.gpu_draw_calls,"instance_bytes":after.instance_bytes,"texture_bytes":after.texture_bytes,"renderer":view.renderer_name()}).to_string()).map_err(|e|e.to_string())?;
        fltk::window::DoubleWindow::delete((*win).clone());Ok(())
    });
    run("scene batching GPU: ordered solids/images, clip and shader barriers, retained keyed updates", &|shots| {
        let mut win = AppWindow::new("Ordered batches", 340, 300);
        let mut body = win.body();
        body.set_margin(0);
        let t = theme::current();
        let (image, ()) = Scene::record(|| {
            paint::fill(0, 0, 14, 14, 0, t.palette.accent);
            paint::text("A", 1, 0, 12, 14, 10, true, t.palette.background, Align::Center);
        });
        let image = Layer::new(Rect::new(0, 0, 14, 14), image).map_err(|e|e.to_string())?;
        let shader = sl::Shader::new(|p, u| sl::mix(u.background(), u.accent(), sl::clamp(p.x() / u.size().x() + u.time() * 0.1, 0.0, 1.0)));
        let view = Surface::new(340, 240).backend(Backend::Gpu).paint(move |r, t| {
            paint::fill(0, 0, r.w, r.h, 0, t.palette.background);
            for i in 0..130 {
                paint::fill(8 + (i % 26) * 11, 8 + (i / 26) * 7, 14, 11, 0,
                    if i % 2 == 0 { t.palette.accent } else { t.palette.foreground });
            }
            scene::shader(Rect::new(0, 50, r.w, 20), &shader, &Uniforms::from_theme(t, (r.w as f32, 20.0)));
            for i in 0..70 {
                paint::fill(8 + (i % 20) * 12, 78 + (i / 20) * 9, 16, 13, 0,
                    if i % 2 == 0 { t.palette.foreground } else { t.palette.accent });
            }
            for i in 0..100 {
                assert!(image.draw_keyed(NodeId(i + 1), 10 + (i % 20) as i32 * 12, 135 + (i / 20) as i32 * 10, 1.0).unwrap());
            }
            for i in 0..4 {
                scene::clip(Rect::new(10 + i * 65, 200, 50, 22), || {
                    paint::fill(0, 195, r.w, 30, 0, t.palette.accent);
                    paint::fill(15 + i * 65, 205, 15, 15, 0, t.palette.foreground);
                });
            }
        });
        body.add(&*view);
        win.end();
        view.set_time(Some(0.0));
        win.show();
        check(wait(|| view.stats().gpu_frames > 0), &format!("GPU activation: {:?}", view.backend_error()))?;
        let before = view.stats();
        parity(&mut win, &view, shots, "scene-batching-initial")?;
        let after = view.stats();
        let frames = after.gpu_frames - before.gpu_frames;
        let primitives = after.gpu_primitives - before.gpu_primitives;
        let calls = after.gpu_draw_calls - before.gpu_draw_calls;
        std::fs::write(shots.join("scene-batching-report.json"), serde_json::json!({
            "frames": frames, "submitted_primitives": primitives, "draw_calls": calls,
            "renderer": view.renderer_name(), "max_allowed_rgb_delta": 2, "instance_bytes": after.instance_bytes,
            "scope": "correctness and GL submission counts; not physical presentation or an efficiency benchmark"
        }).to_string()).map_err(|e|e.to_string())?;
        check(frames > 0 && primitives >= frames * 300, "capture submits the complete visible workload")?;
        check(calls <= frames * 16, &format!("over 300 ordered quads use at most 16 draws per frame: {primitives} quads / {calls} draws / {frames} frames"))?;
        view.set_time(Some(0.4));
        parity(&mut win, &view, shots, "scene-batching-shader-time")?;
        check(view.stats().gpu_instance_upload_bytes == after.gpu_instance_upload_bytes, "shader time reuses the retained instance buffer")?;
        view.move_node(NodeId(1), 21, 143).map_err(|e|e.to_string())?;
        view.set_node_opacity(NodeId(1), 0.5).map_err(|e|e.to_string())?;
        view.raise_node(NodeId(1)).map_err(|e|e.to_string())?;
        parity(&mut win, &view, shots, "scene-batching-raised")?;
        check(view.stats().gpu_scene_uploads == after.gpu_scene_uploads, "raising and opacity preserve shared textures/programs")?;
        check(view.stats().gpu_instance_upload_bytes > after.gpu_instance_upload_bytes, "keyed mutations update the retained instance records")?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}
