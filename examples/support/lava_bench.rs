//! Time-limited diagnostic. No synthetic desktop input or native focus.
use super::lava_ui::Core;
use fltk::{app, prelude::*};
use fltk_omarchy::{
    anim,
    gfx::Backend,
    scene::{Surface, SurfaceStats},
    AppWindow,
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Instant};

pub fn start_when_ready(view: Surface, core: Rc<Core>, window: AppWindow, path: String) {
    app::add_check(move |handle| {
        if view.was_deleted() {
            app::remove_check(handle);
            return;
        }
        if let Some(error) = view.backend_error() {
            eprintln!("{error}");
            app::remove_check(handle);
            app::quit();
            return;
        }
        if view.stats().gpu_frames + view.stats().cpu_frames > 0
            && (view.active_backend() == Backend::Gpu
                || std::env::args().any(|a| a == "--cpu")
                || !cfg!(feature = "gpu"))
        {
            app::remove_check(handle);
            start(view.clone(), core.clone(), window.clone(), path.clone());
        }
    });
}
fn cpu() -> f64 {
    fltk_omarchy::platform::process_cpu_time()
        .expect("process CPU time")
        .as_secs_f64()
}
type Sample = (f64, Instant, SurfaceStats);
fn sample(view: &Surface) -> Sample {
    (cpu(), Instant::now(), view.stats())
}
fn phase(
    report: &mut serde_json::Map<String, serde_json::Value>,
    name: &str,
    before: Sample,
    view: &Surface,
) {
    let (cpu_before, at, before) = before;
    let dt = at.elapsed().as_secs_f64();
    let after = view.stats();
    let frames = after.cpu_frames + after.gpu_frames - before.cpu_frames - before.gpu_frames;
    report.insert(
        name.into(),
        serde_json::json!({
            "cpu_percent_one_core": (cpu()-cpu_before)/dt*100.0,
            "seconds": dt, "frames": frames, "fps": frames as f64/dt,
            "parent_draws": after.surface_draws-before.surface_draws,
            "scene_builds": after.scene_builds-before.scene_builds,
            "preparations": after.gpu_preparations-before.gpu_preparations,
            "scene_uploads": after.gpu_scene_uploads-before.gpu_scene_uploads,
            "gpu_primitives": after.gpu_primitives-before.gpu_primitives,
            "gpu_draw_calls": after.gpu_draw_calls-before.gpu_draw_calls,
            "instance_upload_bytes": after.gpu_instance_upload_bytes-before.gpu_instance_upload_bytes,
            "instance_bytes": after.instance_bytes,
        "overlay_captures": after.overlay_captures-before.overlay_captures,
        "overlay_uploads": after.overlay_uploads-before.overlay_uploads,
        "overlay_texture_bytes": after.overlay_texture_bytes,
        }),
    );
}
fn allocator() -> serde_json::Value {
    fltk_omarchy::platform::memory::allocator_snapshot().map_or(serde_json::Value::Null, |s| serde_json::json!({
        "arena_bytes":s.arena_bytes,"used_arena_bytes":s.used_arena_bytes,
        "free_arena_bytes":s.free_arena_bytes,"mapped_bytes":s.mapped_bytes,
    }))
}
fn start(view: Surface, core: Rc<Core>, mut window: AppWindow, path: String) {
    anim::set_enabled(true);
    let overlay = std::env::args().any(|a| a == "--bench-overlay");
    if overlay {
        // A critical toast is non-modal, stays for the full diagnostic and
        // does not request focus on the user's hidden-workspace window.
        fltk_omarchy::overlay::Notification::show(
            &*view,
            "",
            &fltk_omarchy::tr!("Tasks"),
            &fltk_omarchy::tr!("Ready for review"),
            fltk_omarchy::overlay::Urgency::Critical,
        );
    }
    let phase_path = PathBuf::from(format!("{path}.phase"));
    std::fs::write(&phase_path, "warmup").expect("benchmark phase");
    let current = Rc::new(RefCell::new(sample(&view)));
    let report = Rc::new(RefCell::new(serde_json::Map::new()));
    let trim = std::env::args().any(|a| a == "--bench-trim");
    report.borrow_mut().insert("heap_trim_requested".into(), serde_json::json!(trim));
    let p = phase_path.clone();
    app::add_timeout3(0.12, move |_| { std::fs::write(&p, "before_trim").unwrap(); });
    let r = report.clone();
    app::add_timeout3(0.25, move |_| {
        // Diagnostic only: measure a post-compilation allocator operation
        // separately from steady-state rendering. No allocator tuning.
        let before = allocator();
        let (at, cpu_before) = (Instant::now(), cpu());
        let ran = trim && fltk_omarchy::platform::memory::trim_heap();
        let (elapsed_us, cpu_seconds) = (at.elapsed().as_micros(), cpu()-cpu_before);
        r.borrow_mut().insert("heap_trim".into(), serde_json::json!({
            "ran": ran, "elapsed_us": elapsed_us, "cpu_seconds": cpu_seconds,
            "allocator_before":before,"allocator_after":allocator(),
        }));
    });
    let (s, v, p) = (current.clone(), view.clone(), phase_path.clone());
    app::add_timeout3(0.5, move |_| {
        std::fs::write(&p, "idle").unwrap();
        *s.borrow_mut() = sample(&v);
    });
    let (s, v, r) = (current.clone(), view.clone(), report.clone());
    app::add_timeout3(2.5, move |_| {
        phase(&mut r.borrow_mut(), "idle", *s.borrow(), &v);
    });
    // Reorder every 350 ms, exercising the actual preview/reflow/settle
    // controller. GPU pulses overlap; CPU deliberately keeps lava static.
    for i in 0..11 {
        let c = core.clone();
        app::add_timeout3(2.5 + i as f64 * 0.35, move |_| c.benchmark_cycle(false));
    }
    let (s, v, p) = (current.clone(), view.clone(), phase_path.clone());
    app::add_timeout3(3.5, move |_| {
        std::fs::write(&p, "animation").unwrap();
        *s.borrow_mut() = sample(&v);
    });
    let (s, v, r, c) = (current.clone(), view.clone(), report.clone(), core.clone());
    app::add_timeout3(6.5, move |_| {
        phase(&mut r.borrow_mut(), "animation", *s.borrow(), &v);
        c.benchmark_cycle(true);
    });
    let (s, v, p) = (current.clone(), view.clone(), phase_path);
    app::add_timeout3(7.5, move |_| {
        std::fs::write(&p, "idle_again").unwrap();
        *s.borrow_mut() = sample(&v);
    });
    app::add_timeout3(9.5, move |_| {
        let mut r = report.borrow_mut();
        phase(&mut r, "idle_again", *current.borrow(), &view);
        r.insert(
            "backend".into(),
            serde_json::json!(format!("{:?}", view.active_backend())),
        );
        r.insert("overlay".into(), serde_json::json!(overlay));
        r.insert("renderer".into(), serde_json::json!(view.renderer_name()));
        r.insert(
            "surface_size".into(),
            serde_json::json!([view.w(), view.h()]),
        );
        r.insert(
            "texture_bytes".into(),
            serde_json::json!(view.stats().texture_bytes),
        );
        r.insert("remaining_keys".into(), serde_json::json!(core.keys()));
        r.insert("flight_finished".into(), serde_json::json!(!core.flying()));
        r.insert("scope".into(),serde_json::json!("five-card lava controller; 350 ms reorder cycles, then 5-to-4 deletion; GPU pulses, CPU static lava; no physical input, power, 5000-message or 4K/120 claim"));
        let data = serde_json::to_string(&*r).unwrap();
        println!("{data}");
        std::fs::write(&path, data).expect("benchmark output");
        window.hide();
    });
}
