//! Single-context scene composition/read-back check. Run in private Xvfb:
//! env -u WAYLAND_DISPLAY xvfb-run -a cargo run --features gpu --example scene_gpu_check
//! Writes CPU/GPU PNGs and JSON in target/scene-gpu-check; exits nonzero on failure.
use fltk::{app, enums::*, prelude::*, window::GlWindow};
use fltk_omarchy::{gfx::{self, sl}, paint, scene::{self, gpu::{PreparedScene, Renderer}, NodeId, Rect, Scene}, theme, tr, App};
use glow::HasContext;
use std::{cell::RefCell, rc::Rc};

const W: i32 = 256;
const H: i32 = 192;
fn main() {
    let application = App::without_theme_watch("dev.omarchy.fltk.scene-gpu-check");
    fltk_omarchy::anim::set_enabled(false);
    let output = std::path::Path::new("target/scene-gpu-check");
    std::fs::create_dir_all(output).unwrap();
    let t = theme::current();
    let shader = sl::Shader::new(|p, u| sl::mix(u.background(), u.accent(), sl::clamp(p.x() / u.size().x() + u.time() * 0.1, 0.0, 1.0)));
    let mut uniforms = gfx::Uniforms::from_theme(&t, (W as f32, H as f32));
    uniforms.time = 0.2;
    let (card, ()) = Scene::record(|| {
        paint::fill(0, 0, 110, 65, 10, t.palette.lighter_background);
        paint::text(&tr!("GPU scene"), 8, 4, 95, 24, 14, true, t.palette.foreground, Align::Left);
        paint::stroke(0, 0, 110, 65, 10, 1, t.palette.accent);
    });
    let card = scene::Layer::new(Rect::new(0, 0, 110, 65), card).unwrap();
    let (scene, ()) = Scene::record(|| {
        // Start with a shader, before any instanced draw initializes its
        // per-batch offsets. The opaque shader supplies the background.
        assert!(scene::shader(Rect::new(0, 0, W, H), &shader, &uniforms));
        assert!(card.draw_keyed(NodeId(7),14,12,0.7).unwrap());
        assert!(card.draw_keyed(NodeId(91),-200,-200,0.0).unwrap());
        scene::clip(Rect::new(90, 60, 150, 115), || {
        scene::clip(Rect::new(100, 65, 120, 105), || {
            assert!(card.draw_keyed(NodeId(33), 110, 80, 1.0).unwrap());
            paint::fill(90, 65, 140, 48, 9, t.palette.accent);
            paint::text(&tr!("@layer café"), 106, 73, 110, 30, 13, false, t.palette.background, Align::Left);
            paint::alpha_fill(140, 110, 100, 60, t.palette.foreground, 0.3);
            let image = scene::Image::rgba(2, 2, vec![255, 0, 0, 128, 0, 255, 0, 255, 0, 0, 255, 0, 255, 255, 255, 64]).unwrap();
            image.draw(Rect::new(100, 130, 20, 20));
            image.draw(Rect::new(124, 132, 2, 2));
        });
        });
    });
    let render_cpu = |scene: &Scene| {
        let surface = fltk::surface::ImageSurface::new(W, H, false);
        fltk::surface::ImageSurface::push_current(&surface);
        scene.draw();
        let image = surface.image().unwrap();
        fltk::surface::ImageSurface::pop_current();
        image.to_rgb_data()
    };
    let cpu = render_cpu(&scene);
    let mut updated_scene = scene.clone();
    updated_scene.move_node(NodeId(7), 36, 25).unwrap();
    updated_scene.set_node_opacity(NodeId(7), 0.4).unwrap();
    updated_scene.move_node(NodeId(91), 80, 36).unwrap();
    updated_scene.set_node_opacity(NodeId(91), 0.6).unwrap();
    updated_scene.raise_node(NodeId(7)).unwrap();
    updated_scene.raise_node(NodeId(91)).unwrap();
    updated_scene.raise_node(NodeId(7)).unwrap();
    updated_scene.move_node(NodeId(33), 105, 100).unwrap();
    updated_scene.raise_node(NodeId(33)).unwrap();
    let updated_cpu = render_cpu(&updated_scene);
    let mut deferred = PreparedScene::new(&scene, Rect::new(0, 0, W, H)).unwrap();
    deferred.move_node(NodeId(7), 36, 25).unwrap();
    deferred.set_node_opacity(NodeId(7), 0.4).unwrap();
    deferred.move_node(NodeId(91), 80, 36).unwrap();
    deferred.set_node_opacity(NodeId(91), 0.6).unwrap();
    deferred.raise_node(NodeId(7)).unwrap();
    deferred.raise_node(NodeId(91)).unwrap();
    deferred.raise_node(NodeId(7)).unwrap();
    deferred.move_node(NodeId(33), 105, 100).unwrap();
    deferred.raise_node(NodeId(33)).unwrap();
    let mut deferred = Some(deferred);
    fltk_omarchy::png::write(&output.join("cpu.png"), W as u32, H as u32, 3, &cpu).unwrap();
    let prepared = PreparedScene::new(&scene, Rect::new(0, 0, W, H)).unwrap();
    let count = prepared.primitive_count();
    let mut prepared = Some(prepared);
    let result: Rc<RefCell<Option<Result<(), String>>>> = Rc::default();
    let r = result.clone();
    let title = tr!("Scene compositor check");
    let mut win = GlWindow::new(0, 0, W, H, title.as_str());
    win.set_mode(Mode::Opengl3 | Mode::Double | Mode::Rgb8);
    win.end();
    win.super_draw(false);
    win.draw(move |window| {
        let Some(prepared) = prepared.take() else { return };
        let execute = || -> Result<(), String> {
            // SAFETY: GL draw callback, current context; renderer and FBO
            // are destroyed before leaving this callback.
            unsafe {
                let gl = Rc::new(glow::Context::from_loader_function(|name| window.get_proc_address(name) as *const _));
                let mut renderer = Renderer::new(gl.clone(), prepared)?;
                let texture = gl.create_texture()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, W, H, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
                let fbo = gl.create_framebuffer()?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(texture), 0);
                assert_eq!(gl.check_framebuffer_status(glow::FRAMEBUFFER), glow::FRAMEBUFFER_COMPLETE);
                renderer.draw((W, H), (W, H), None);
                let mut rgba = vec![0; (W * H * 4) as usize];
                gl.read_pixels(0, 0, W, H, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut rgba)));
                let mut rgb = Vec::with_capacity((W * H * 3) as usize);
                for row in rgba.chunks_exact((W * 4) as usize).rev() { for pixel in row.chunks_exact(4) { rgb.extend_from_slice(&pixel[..3]); } }
                let max_delta = rgb.iter().zip(&cpu).map(|(a,b)| a.abs_diff(*b)).max().unwrap();
                let changed = rgb.chunks_exact(3).zip(cpu.chunks_exact(3)).filter(|(a,b)| a != b).count();
                fltk_omarchy::png::write(&output.join("gpu.png"), W as u32, H as u32, 3, &rgb).unwrap();
                renderer.draw((W, H), (W, H), Some(2.0));
                let mut animated = vec![0; rgba.len()];
                gl.read_pixels(0, 0, W, H, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut animated)));
                let time_changed = animated != rgba;
                let bytes = renderer.texture_bytes();
                renderer.move_node(NodeId(7), 36, 25).unwrap();
                renderer.set_node_opacity(NodeId(7), 0.4).unwrap();
                renderer.move_node(NodeId(91), 80, 36).unwrap();
                renderer.set_node_opacity(NodeId(91), 0.6).unwrap();
                renderer.raise_node(NodeId(7)).unwrap();
                renderer.raise_node(NodeId(91)).unwrap();
                renderer.raise_node(NodeId(7)).unwrap();
                renderer.move_node(NodeId(33), 105, 100).unwrap();
                renderer.raise_node(NodeId(33)).unwrap();
                renderer.draw((W, H), (W, H), None);
                let mut moved = vec![0; rgba.len()];
                gl.read_pixels(0, 0, W, H, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut moved)));
                let card_changed_without_upload = moved != animated && bytes == renderer.texture_bytes();
                let rgb_of = |rgba: &[u8]| -> Vec<u8> { rgba.chunks_exact((W * 4) as usize).rev().flat_map(|row| row.chunks_exact(4).flat_map(|p|p[..3].iter().copied())).collect() };
                let updated_rgb = rgb_of(&moved);
                let keyed_delta = updated_rgb.iter().zip(&updated_cpu).map(|(a,b)|a.abs_diff(*b)).max().unwrap();
                fltk_omarchy::png::write(&output.join("keyed-gpu.png"), W as u32, H as u32, 3, &updated_rgb).unwrap();
                fltk_omarchy::png::write(&output.join("keyed-cpu.png"), W as u32, H as u32, 3, &updated_cpu).unwrap();
                let deferred_renderer = Renderer::new(gl.clone(), deferred.take().unwrap())?;
                deferred_renderer.draw((W,H),(W,H),None);
                let mut deferred_pixels = vec![0;rgba.len()];
                gl.read_pixels(0,0,W,H,glow::RGBA,glow::UNSIGNED_BYTE,glow::PixelPackData::Slice(Some(&mut deferred_pixels)));
                let deferred_delta = rgb_of(&deferred_pixels).iter().zip(&updated_cpu).map(|(a,b)|a.abs_diff(*b)).max().unwrap();
                drop(deferred_renderer);
                let error = gl.get_error();
                let report = serde_json::json!({"max_channel_delta":max_delta,"keyed_update_max_delta":keyed_delta,"deferred_update_max_delta":deferred_delta,"different_pixels":changed,"shader_time_changed":time_changed,"card_changed_without_upload":card_changed_without_upload,"gl_error":error,"primitives":count,"texture_bytes":renderer.texture_bytes(),"renderer":gl.get_parameter_string(glow::RENDERER),"correctness_check_not_an_efficiency_benchmark":true});
                println!("{report}");
                std::fs::write(output.join("report.json"), report.to_string()).unwrap();
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                gl.delete_framebuffer(fbo);
                gl.delete_texture(texture);
                drop(renderer);
                if max_delta > 2 || keyed_delta > 2 || deferred_delta > 2 || !time_changed || !card_changed_without_upload || error != glow::NO_ERROR { return Err("scene compositor parity/time/GL check failed".into()); }
            }
            Ok(())
        };
        *r.borrow_mut() = Some(execute());
        app::quit();
    });
    win.show();
    let timeout = result.clone();
    app::add_timeout3(5.0, move |_| { if timeout.borrow().is_none() { *timeout.borrow_mut() = Some(Err("GL draw timed out".into())); } app::quit(); });
    application.run();
    let result = result.borrow_mut().take().unwrap_or_else(|| Err("no GL result".into()));
    if let Err(error) = result { eprintln!("{error}"); std::process::exit(1); }
}
