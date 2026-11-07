//! Native GL residency/eviction/readback gate. Run on a private display:
//! env -u WAYLAND_DISPLAY xvfb-run -a cargo run --features gpu --example scene_residency_check
use fltk::{enums::*, prelude::*, surface::ImageSurface, window::GlWindow};
use fltk_omarchy::{
    gfx::{sl, Uniforms},
    paint,
    scene::{
        self,
        gpu::{PreparedScene, Renderer},
        Image, Layer, Rect, Scene,
    },
    theme, tr, App,
};
use glow::HasContext;
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

const W: i32 = 256;
const H: i32 = 128;
struct Case {
    prepared: PreparedScene,
    reference: Vec<u8>,
}
fn prepare(scene: &Scene) -> Case {
    let surface = ImageSurface::new(W, H, false);
    ImageSurface::push_current(&surface);
    scene.draw();
    let reference = surface.image().unwrap().to_rgb_data();
    ImageSurface::pop_current();
    Case {
        prepared: PreparedScene::new(scene, Rect::new(0, 0, W, H)).unwrap(),
        reference,
    }
}
unsafe fn read(gl: &glow::Context) -> Vec<u8> {
    let mut rgba = vec![0; (W * H * 4) as usize];
    gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
    gl.pixel_store_i32(glow::PACK_ROW_LENGTH, 0);
    gl.read_pixels(
        0,
        0,
        W,
        H,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        glow::PixelPackData::Slice(Some(&mut rgba)),
    );
    let mut rgb = Vec::with_capacity((W * H * 3) as usize);
    for row in rgba.chunks_exact((W * 4) as usize).rev() {
        for pixel in row.chunks_exact(4) {
            rgb.extend_from_slice(&pixel[..3]);
        }
    }
    rgb
}
fn parity(actual: &[u8], expected: &[u8]) -> Result<u8, String> {
    let delta = actual
        .iter()
        .zip(expected)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    if delta > 2 {
        Err(format!("residency readback delta {delta} exceeds 2"))
    } else {
        Ok(delta)
    }
}
fn rgb(r: u8, g: u8, b: u8) -> theme::Rgb {
    theme::Rgb((r as u32) << 16 | (g as u32) << 8 | b as u32)
}
unsafe fn execute(
    window: &GlWindow,
    cases: Vec<Case>,
    pressure: Vec<Case>,
    rejected: PreparedScene,
    empty: Case,
    output: &std::path::Path,
    recreate: bool,
) -> Result<serde_json::Value, String> {
    let gl = Rc::new(glow::Context::from_loader_function(|name| {
        window.get_proc_address(name) as *const _
    }));
    let texture = gl.create_texture()?;
    gl.bind_texture(glow::TEXTURE_2D, Some(texture));
    gl.tex_image_2d(
        glow::TEXTURE_2D,
        0,
        glow::RGBA8 as i32,
        W,
        H,
        0,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        glow::PixelUnpackData::Slice(None),
    );
    let fbo = gl.create_framebuffer()?;
    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
    gl.framebuffer_texture_2d(
        glow::FRAMEBUFFER,
        glow::COLOR_ATTACHMENT0,
        glow::TEXTURE_2D,
        Some(texture),
        0,
    );
    assert_eq!(
        gl.check_framebuffer_status(glow::FRAMEBUFFER),
        glow::FRAMEBUFFER_COMPLETE
    );
    let mut iter = cases.into_iter();
    let first = iter.next().unwrap();
    let mut renderer = Some(Renderer::new(gl.clone(), first.prepared)?);
    renderer.as_ref().unwrap().draw((W, H), (W, H), None);
    let mut max_delta = parity(&read(&gl), &first.reference)?;
    let mut uploads = 0;
    let mut reused = 0;
    let mut peak = renderer.as_ref().unwrap().texture_bytes();
    let mut revision_program_links = 0;
    let mut texture_upload_bytes = 0;
    let started = Instant::now();
    let residency_bound = peak + 24 * 24 * 4; // one live replacement tile, old page retained
    for (revision, case) in iter.enumerate() {
        if recreate {
            drop(renderer.take());
            renderer = Some(Renderer::new(gl.clone(), case.prepared)?);
        } else {
            renderer.as_mut().unwrap().replace_scene(case.prepared)?;
        }
        let renderer = renderer.as_ref().unwrap();
        let stats = renderer.resource_stats();
        let expected = if recreate { (2, 3, 0) } else { (0, 1, 2) };
        if (
            stats.program_links,
            stats.texture_uploads,
            stats.reused_images,
        ) != expected
            || stats.resident_images != 3
            || stats.texture_objects > 2
        {
            return Err(format!("revision {revision} unexpected work: {stats:?}"));
        }
        uploads += stats.texture_uploads;
        reused += stats.reused_images;
        revision_program_links += stats.program_links;
        texture_upload_bytes += stats.texture_upload_bytes;
        peak = peak.max(renderer.texture_bytes());
        renderer.draw((W, H), (W, H), None);
        let rgb = read(&gl);
        max_delta = max_delta.max(parity(&rgb, &case.reference)?);
        if revision == 254 {
            fltk_omarchy::png::write(
                &output.join(if recreate {
                    "recreate-last-gpu.png"
                } else {
                    "last-gpu.png"
                }),
                W as u32,
                H as u32,
                3,
                &rgb,
            )
            .map_err(|e| e.to_string())?;
        }
        if gl.get_error() != glow::NO_ERROR {
            return Err(format!("GL error in revision {revision}"));
        }
    }
    if peak > residency_bound {
        return Err(format!("small-scene residency grew to {peak}"));
    }
    let revision_wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut renderer = renderer.take().unwrap();
    let mut repacks = 0;
    for case in pressure {
        renderer.replace_scene(case.prepared)?;
        repacks += renderer.resource_stats().repacks;
        renderer.draw((W, H), (W, H), None);
        max_delta = max_delta.max(parity(&read(&gl), &case.reference)?);
        if renderer.texture_bytes() + renderer.instance_bytes() > 64 * 1024 * 1024
            || gl.get_error() != glow::NO_ERROR
        {
            return Err("pressure budget/GL failure".into());
        }
    }
    if repacks != 1 {
        return Err(format!("expected one pressure compaction, got {repacks}"));
    }
    let error = renderer
        .replace_scene(rejected)
        .expect_err("64 MiB of pixels plus instance data must not fit");
    if !error.contains("budget")
        || renderer.texture_bytes() != 0
        || renderer.instance_bytes() != 0
        || renderer.resource_stats().resident_images != 0
    {
        return Err(format!("failed replacement retained allocations: {error}"));
    }
    renderer.replace_scene(empty.prepared)?;
    renderer.draw((W, H), (W, H), None);
    max_delta = max_delta.max(parity(&read(&gl), &empty.reference)?);
    let resources = renderer.resource_stats();
    if renderer.texture_bytes() != 0
        || renderer.instance_bytes() != 0
        || resources.resident_images != 0
        || resources.shader_programs != 1
    {
        return Err(format!("empty scene retained resources: {resources:?}"));
    }
    let report = serde_json::json!({"renderer":gl.get_parameter_string(glow::RENDERER),"mode":if recreate {"recreate"} else {"retain"},"revision_wall_ms_including_readback":revision_wall_ms,"changed_texture_upload_bytes":texture_upload_bytes,"revisions":256,"changed_image_uploads":uploads,"common_image_reuses":reused,"revision_program_links":revision_program_links,"budget_rejection_recovers":true,"small_scene_peak_texture_bytes":peak,"small_scene_bound_bytes":residency_bound,"pressure_repacks":repacks,"max_rgb_delta":max_delta,"empty_texture_bytes":renderer.texture_bytes(),"empty_instance_bytes":renderer.instance_bytes(),"scope":"resource ownership, uploads, GL errors and full readback parity; not PSS, power or presentation rate"});
    drop(renderer);
    gl.delete_framebuffer(fbo);
    gl.delete_texture(texture);
    Ok(report)
}
fn main() {
    let _app = App::without_theme_watch("dev.omarchy.fltk.scene-residency-check");
    fltk_omarchy::anim::set_enabled(false);
    let output = std::path::Path::new("target/scene-residency-check");
    std::fs::create_dir_all(output).unwrap();
    let t = theme::current();
    let shader = sl::Shader::new(|_, u| u.background());
    let uniforms = Uniforms::from_theme(&t, (W as f32, H as f32));
    let common = Image::rgba(
        2,
        2,
        vec![
            255, 0, 0, 128, 0, 255, 0, 255, 0, 0, 255, 0, 255, 255, 255, 190,
        ],
    )
    .unwrap();
    let (text, ()) = Scene::record(|| {
        paint::text(
            "café 日本語 😀",
            0,
            0,
            190,
            28,
            14,
            false,
            t.palette.foreground,
            Align::Left,
        )
    });
    let text = Layer::new(Rect::new(0, 0, 190, 28), text).unwrap();
    let mut cases = Vec::new();
    for revision in 0..256 {
        let variable = Image::rgba(2, 2, [revision as u8, 64, 180, 200].repeat(4)).unwrap();
        let (scene, ()) = Scene::record(|| {
            scene::shader(Rect::new(0, 0, W, H), &shader, &uniforms);
            common.draw(Rect::new(10, 52, 24, 24));
            variable.draw(Rect::new(25, 64, 24, 24));
            text.draw_at(8 + revision % 7, 8, 0.8);
            // Alternate instance counts as well as content: subdata must
            // not write past a buffer allocated for a smaller revision.
            for x in 0..revision % 19 {
                paint::fill(x * 9, 105, 12, 12, 0, t.palette.accent);
            }
        });
        cases.push(prepare(&scene));
    }
    // Force page fragmentation to consume the remaining live GPU budget.
    // The replacement fits as individual textures (63 MiB), but cannot
    // coexist with the previous partially live page (>64 MiB).
    let big_a = Image::rgba(4096, 2048, [10, 20, 30, 255].repeat(4096 * 2048)).unwrap();
    let big_b = Image::rgba(4096, 1664, [30, 40, 50, 255].repeat(4096 * 1664)).unwrap();
    let small_a = Image::rgba(512, 512, [120, 80, 30, 255].repeat(512 * 512)).unwrap();
    let small_b = Image::rgba(512, 512, [50, 70, 90, 255].repeat(512 * 512)).unwrap();
    let replacement = Image::rgba(1024, 1024, [200, 30, 60, 255].repeat(1024 * 1024)).unwrap();
    let mut pressure = Vec::new();
    for (variable, color) in [
        (&small_b, rgb(50, 70, 90)),
        (&replacement, rgb(200, 30, 60)),
    ] {
        let (scene, ()) = Scene::record(|| {
            scene::shader(Rect::new(0, 0, W, H), &shader, &uniforms);
            big_a.draw(Rect::new(-4000, -2000, 4096, 2048));
            big_b.draw(Rect::new(-4000, -1600, 4096, 1664));
            small_a.draw(Rect::new(120, 0, 512, 512));
            variable.draw(Rect::new(
                0,
                80,
                variable.width() as i32,
                variable.height() as i32,
            ));
        });
        let prepared = PreparedScene::new(&scene, Rect::new(0, 0, W, H)).unwrap();
        // The large test images are constant color: use the exact equivalent
        // CPU fills for reference, avoiding native caches of 63 MiB sources.
        let (reference, ()) = Scene::record(|| {
            paint::fill(0, 0, W, H, 0, t.palette.background);
            paint::fill(0, 0, 96, 48, 0, rgb(10, 20, 30));
            paint::fill(0, 0, 96, 64, 0, rgb(30, 40, 50));
            paint::fill(120, 0, W - 120, H, 0, rgb(120, 80, 30));
            paint::fill(0, 80, W, H - 80, 0, color);
        });
        pressure.push(Case {
            prepared,
            reference: prepare(&reference).reference,
        });
    }
    let (empty, ()) = Scene::record(|| paint::fill(0, 0, W, H, 0, t.palette.background));
    let empty = prepare(&empty);
    let oversized = Image::rgba(4096, 4096, vec![0; 64 * 1024 * 1024]).unwrap();
    let (rejected, ()) = Scene::record(|| oversized.draw(Rect::new(0, 0, 4096, 4096)));
    let rejected = PreparedScene::new(&rejected, Rect::new(0, 0, W, H)).unwrap();
    drop(oversized);
    let recreate = std::env::args().any(|arg| arg == "--recreate");
    let mut work = Some((cases, pressure, rejected, empty));
    let result: Rc<RefCell<Option<Result<serde_json::Value, String>>>> = Rc::default();
    let back = result.clone();
    let title = tr!("Scene residency check");
    let mut win = GlWindow::new(0, 0, W, H, title.as_str());
    win.set_mode(Mode::Opengl3 | Mode::Double | Mode::Rgb8);
    win.end();
    win.super_draw(false);
    win.draw(move |window| {
        let Some((cases, pressure, rejected, empty)) = work.take() else {
            return;
        };
        *back.borrow_mut() =
            Some(unsafe { execute(window, cases, pressure, rejected, empty, output, recreate) });
    });
    win.show();
    let until = Instant::now() + Duration::from_secs(20);
    while result.borrow().is_none() && Instant::now() < until {
        App::pump();
        std::thread::sleep(Duration::from_millis(5));
    }
    win.hide();
    let report = result
        .borrow_mut()
        .take()
        .expect("native GL callback did not run")
        .expect("residency gate failed");
    std::fs::write(
        output.join(if recreate {
            "recreate-report.json"
        } else {
            "report.json"
        }),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("{report}");
}
