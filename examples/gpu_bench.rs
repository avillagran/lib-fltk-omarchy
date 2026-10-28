//! GPU shader check and cost (needs `--features gpu`). Run by
//! script/gpu-bench on a hidden Hyprland workspace:
//!   gpu_bench check <report.json>          pixel check + compile error check
//!   gpu_bench <static|animated> <seconds> <report.json>
use fltk::prelude::*;
use fltk_omarchy::gfx::gpu::GpuView;
use fltk_omarchy::{App, AppWindow};
use std::cell::RefCell;
use std::rc::Rc;

const NOISE: &str = r#"
float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float noise(vec2 p) {
    vec2 i = floor(p), f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1, 0)), u.x), mix(hash(i + vec2(0, 1)), hash(i + vec2(1, 1)), u.x), u.y);
}
vec4 shade(vec2 p) {
    vec2 q = p / 48.0 + vec2(u_time * 0.15, -u_time * 0.1);
    float n = 0.5 * noise(q) + 0.25 * noise(q * 2.0) + 0.125 * noise(q * 4.0);
    return mix(u_background, u_foreground, n / 0.875 * 0.35);
}
"#;

fn pss_tree_kb() -> u64 {
    std::fs::read_to_string("/proc/self/smaps_rollup").unwrap_or_default().lines()
        .filter(|l| l.starts_with("Pss:") || l.starts_with("SwapPss:"))
        .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).sum()
}

fn cpu_s() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mode = a.get(1).cloned().unwrap_or_else(|| "check".into());
    let app = App::without_theme_watch("dev.omarchy.fltk.gpubench");
    let mut win = AppWindow::new("gpu-bench", 520, 400);
    let mut body = win.body();
    if mode == "check" {
        let out = a.get(2).cloned().unwrap_or_else(|| "build/gpu-check.json".into());
        let solid = GpuView::new(200, 120, "vec4 shade(vec2 p) { return p.x < u_size.x * 0.5 ? u_accent : u_background; }");
        let broken = GpuView::new(100, 60, "vec4 shade(vec2 p) { return nope; }");
        fltk_omarchy::layout::FlexExt::fit(&mut body, &*solid);
        fltk_omarchy::layout::FlexExt::fit(&mut body, &*broken);
        win.end();
        win.set_mode(fltk_omarchy::Mode::Floating);
        win.show();
        let acc = fltk_omarchy::theme::current().palette.accent;
        let bg = fltk_omarchy::theme::current().palette.background;
        let result: Rc<RefCell<Vec<String>>> = Rc::default();
        let (r1, s1) = (result.clone(), solid.clone());
        fltk::app::add_timeout3(0.8, move |_| {
            let (r2, r3) = (r1.clone(), r1.clone());
            let right_x = s1.w() - 10;
            s1.read_pixel(10, 60, move |px| r2.borrow_mut().push(format!("\"left\":[{},{},{}]", px[0], px[1], px[2])));
            s1.read_pixel(right_x, 60, move |px| r3.borrow_mut().push(format!("\"right\":[{},{},{}]", px[0], px[1], px[2])));
        });
        let (r4, b4, mut w) = (result.clone(), broken.clone(), win.clone());
        let s4 = solid.clone();
        fltk::app::add_timeout3(1.6, move |_| {
            let mut r = r4.borrow_mut();
            r.push(format!("\"accent\":[{},{},{}],\"background\":[{},{},{}]", acc.r(), acc.g(), acc.b(), bg.r(), bg.g(), bg.b()));
            r.push(format!("\"compile_error\":{}", serde_json::to_string(&b4.last_error()).unwrap()));
            r.push(format!("\"solid_error\":{}", serde_json::to_string(&s4.last_error()).unwrap()));
            r.push(format!("\"stats\":\"{:?}\",\"live\":{}", s4.stats(), s4.is_live()));
            std::fs::write(&out, format!("{{{}}}\n", r.join(","))).unwrap();
            w.hide();
        });
        app.run();
        return;
    }
    if mode == "demo" {
        // A few GPU shaders side by side for a screenshot (script captures it).
        let mut row = fltk_omarchy::layout::row();
        let glow = GpuView::new(160, 300, "vec4 shade(vec2 p) { float d = length(p - u_size * 0.5) / min(u_size.x, u_size.y); return mix(u_accent, u_background, smoothstep(0.0, 0.5 + 0.05 * sin(u_time * 2.0), d)); }").animate(30);
        let noise = GpuView::new(160, 300, NOISE).animate(30);
        let rings = GpuView::new(160, 300, "vec4 shade(vec2 p) { float d = length(p - u_size * 0.5); return mix(u_background, u_accent, (0.5 + 0.5 * sin(d * 0.15 - u_time * 3.0)) * 0.6); }").animate(30);
        for v in [&glow, &noise, &rings] { fltk_omarchy::layout::FlexExt::fit(&mut row, &**v); }
        fltk_omarchy::layout::FlexExt::close_fitted(&mut row);
        win.end();
        win.set_mode(fltk_omarchy::Mode::Floating);
        win.show();
        let mut w = win.clone();
        fltk::app::add_timeout3(a.get(2).and_then(|s| s.parse().ok()).unwrap_or(4.0), move |_| w.hide());
        app.run();
        return;
    }
    let secs: f64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(5.0);
    let out = a.get(3).cloned().unwrap_or_else(|| "build/gpu-bench.json".into());
    let mut view = GpuView::new(400, 300, NOISE);
    if mode == "animated" {
        view = view.animate(30);
    }
    fltk_omarchy::layout::FlexExt::fit(&mut body, &*view);
    win.end();
    win.set_mode(fltk_omarchy::Mode::Floating);
    win.show();
    let sample: Rc<RefCell<Option<(f64, u64, std::time::Instant)>>> = Rc::default();
    let (s2, v2) = (sample.clone(), view.clone());
    fltk::app::add_timeout3(1.0, move |_| { *s2.borrow_mut() = Some((cpu_s(), v2.stats().frames, std::time::Instant::now())); });
    let (s3, v3, m, mut w) = (sample.clone(), view.clone(), mode.clone(), win.clone());
    fltk::app::add_timeout3(1.0 + secs, move |_| {
        let (c0, f0, t0) = s3.borrow().expect("sample");
        let (cpu, frames, wall) = (cpu_s() - c0, v3.stats().frames - f0, t0.elapsed().as_secs_f64());
        let line = format!(
            "{{\"backend\":\"gpu\",\"mode\":\"{m}\",\"seconds\":{wall:.2},\"frames\":{frames},\"fps\":{:.1},\"cpu_pct\":{:.1},\"cpu_ms_per_frame\":{},\"last_draw_ms\":{:.2},\"error\":{},\"pss_tree_kb\":{}}}",
            frames as f64 / wall, 100.0 * cpu / wall,
            if frames > 0 { format!("{:.2}", cpu * 1000.0 / frames as f64) } else { "null".into() },
            v3.stats().last_draw_ms, serde_json::to_string(&v3.last_error()).unwrap(), pss_tree_kb());
        std::fs::write(&out, format!("{line}\n")).unwrap();
        w.hide();
    });
    app.run();
}
