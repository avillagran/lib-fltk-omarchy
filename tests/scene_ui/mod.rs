//! Shared-painter parity and damage: compare every pixel, including clips,
//! rounded borders, literal '@', alpha over varied backgrounds and text.
use super::{check, png_rgb};
use fltk::{enums::Align, prelude::*};
use fltk_omarchy::{paint, scene::{self, Rect, Scene}, theme, AppWindow};
use std::path::Path;

mod nodes;
mod motion;
#[cfg(feature="gpu")]
mod overlays;
#[cfg(feature="gpu")]
mod notices;
#[cfg(feature="gpu")]
mod sheet;
#[cfg(feature="gpu")]
mod lifecycle;
#[cfg(feature="gpu")]
mod batching;
#[cfg(feature="gpu")]
mod residency;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn content(x: i32, y: i32) {
    let t = theme::current();
    paint::fill(x, y, 280, 150, 0, t.palette.background);
    paint::fill(x, y + 60, 280, 90, 0, t.palette.accent);
    scene::clip(Rect::new(x + 12, y + 8, 246, 128), || {
        paint::surface(x + 8, y + 5, 260, 110, 18, Some(t.palette.background), Some((t.palette.foreground, 2)));
        paint::alpha_fill(x + 18, y + 58, 235, 88, t.palette.foreground, 0.25);
        scene::clip(Rect::new(x + 24, y + 16, 190, 100), || {
            paint::text("@review: café 日本語", x + 28, y + 24, 230, 28, 15, true, t.palette.foreground, Align::Left);
            paint::text_weight("Semibold card", x + 28, y + 52, 190, 24, 13, paint::Weight::SemiBold, t.palette.accent, Align::Left);
            paint::text_spaced("LAYER", x + 28, y + 82, 190, 20, 11, paint::Weight::Medium, 2.0, t.palette.foreground, Align::Left);
        });
    });
    let image = scene::Image::rgba(2, 2, vec![
        230, 110, 20, 128, 0, 0, 0, 0,
        20, 170, 240, 255, 80, 30, 100, 64,
    ]).unwrap();
    scene::clip(Rect::new(x + 170, y + 70, 55, 50), || {
        image.draw(Rect::new(x + 160, y + 60, 80, 70));
    });
    let uniforms = fltk_omarchy::gfx::Uniforms::from_theme(&t, (60.0, 18.0));
    assert!(scene::shader(Rect::new(x + 20, y + 112, 60, 18), &fltk_omarchy::gfx::shaders::gradient(), &uniforms));
    let (child, ()) = Scene::record(|| paint::fill(-10, -5, 20, 20, 4, t.palette.accent));
    let child = scene::Layer::new(Rect::new(-10, -5, 20, 20), child).unwrap();
    assert!(child.draw_at(x + 235, y + 20, 0.5));
    // Outside the previous clips: catches unbalanced replay stacks.
    paint::fill(x + 262, y + 130, 12, 12, 3, t.palette.foreground);
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    nodes::checks(run);
    motion::checks(run);
    #[cfg(feature="gpu")]
    overlays::checks(run);
    #[cfg(feature="gpu")]
    notices::checks(run);
    #[cfg(feature="gpu")]
    sheet::checks(run);
    #[cfg(feature="gpu")]
    lifecycle::checks(run);
    #[cfg(feature="gpu")]
    batching::checks(run);
    #[cfg(feature="gpu")]
    residency::checks(run);
    run("scene surface bounded shader: final phase, restart, explicit time and reduced motion", &|shots| {
        use fltk_omarchy::{anim::{self, clock, Pace}, gfx::{sl, Uniforms}};
        use std::time::Duration;
        struct Restore;
        impl Drop for Restore {fn drop(&mut self){clock::resume();anim::set_enabled(false);}}
        let _restore=Restore;
        anim::set_enabled(true);clock::freeze();
        let mut win=AppWindow::new("Bounded shader",220,160);
        let mut body=win.body();body.set_margin(0);
        let shader=sl::Shader::new(|_,u|sl::mix(u.background(),u.accent(),sl::clamp(u.time(),0.0,1.0)));
        let view=scene::Surface::new(150,90).paint(move |r,t|{scene::shader(r,&shader,&Uniforms::from_theme(t,(r.w as f32,r.h as f32)));});
        body.add(&*view);win.end();
        let capture=|win:&mut AppWindow, name:&str|->Result<Vec<u8>,String>{let p=shots.join(name);win.snapshot(&p).map_err(|e|e.to_string())?;Ok(png_rgb(&p)?.raw)};
        view.play_shader(Pace::Display,200);
        let initial=capture(&mut win,"scene-pulse-zero.png")?;
        clock::advance(Duration::from_millis(100));
        let middle=capture(&mut win,"scene-pulse-middle.png")?;
        check(initial!=middle,"bounded shader advances on frozen test clock")?;
        clock::advance(Duration::from_millis(300));
        let final_image=capture(&mut win,"scene-pulse-final.png")?;
        view.set_time(Some(0.2));
        check(final_image==capture(&mut win,"scene-pulse-reference.png")?,"end phase clamps to requested duration")?;
        view.play_shader(Pace::Display,200);
        check(initial==capture(&mut win,"scene-pulse-restart.png")?,"restart clears explicit time and starts at zero")?;
        clock::advance(Duration::from_millis(100));anim::set_enabled(false);
        check(initial==capture(&mut win,"scene-pulse-reduced.png")?,"reduced motion draws phase zero")?;
        fltk::window::DoubleWindow::delete((*win).clone());Ok(())
    });
    run("scene surface CPU: retained painter, coordinates, invalidation, snapshots and keyboard", &|shots| {
        use fltk_omarchy::{gfx::Backend, focus, State};
        let color=State::new(theme::with(|t|t.palette.accent));
        let c=color.clone();
        let events=std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let e=events.clone();
        let mut win=AppWindow::new("Scene surface",320,220);
        let mut body=win.body();body.set_margin(0);
        let view=scene::Surface::new(200,120).paint(move |r,_|paint::fill(0,0,r.w,r.h,0,c.get()))
            .on_event(move |ev,x,y|{e.borrow_mut().push((ev,x,y));false});
        body.add(&*view);win.end();
        let first=shots.join("scene-surface-cpu.png");win.snapshot(&first).map_err(|e|e.to_string())?;
        let p=png_rgb(&first)?;
        check(close(p.at(view.x()+30,view.y()+30),color.get()),"scene uses local coordinates inside window body")?;
        let builds=view.stats().scene_builds;
        win.snapshot(shots.join("scene-surface-repeat.png")).map_err(|e|e.to_string())?;
        check(view.stats().scene_builds==builds,"static snapshots retain painter")?;
        color.set(theme::with(|t|t.palette.foreground));view.invalidate();
        win.snapshot(&first).map_err(|e|e.to_string())?;
        check(close(png_rgb(&first)?.at(view.x()+30,view.y()+30),color.get()),"explicit state invalidation reaches pixels")?;
        check(view.active_backend()==Backend::Cpu && view.stats().gpu_frames==0,"CPU is default and snapshots create no GL surface")?;
        let locale=fltk_omarchy::i18n::locale().get();let before=view.stats().scene_builds;
        fltk_omarchy::i18n::set_locale("de");let result=win.snapshot(&first);
        fltk_omarchy::i18n::set_locale(&locale);result.map_err(|e|e.to_string())?;
        check(view.stats().scene_builds>before,"live language change invalidates the retained painter")?;
        view.clone().take_focus().map_err(|e|e.to_string())?;focus::press("Enter");
        check(events.borrow().iter().any(|(ev,_,_)|*ev==fltk::enums::Event::KeyDown),"focused surface gets real router keys")?;
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
    run("scene surface clock: paced frames stop when hidden or motion is disabled", &|shots| {
        use fltk_omarchy::{anim, gfx::sl};
        use std::time::{Duration,Instant};
        struct Restore;
        impl Drop for Restore{fn drop(&mut self){anim::set_enabled(false);}}
        let _restore=Restore;
        let pump_for=|ms:u64|{let end=Instant::now()+Duration::from_millis(ms);while Instant::now()<end{fltk_omarchy::App::pump();std::thread::sleep(Duration::from_millis(5));}};
        anim::set_enabled(true);
        let mut win=AppWindow::new("Scene clock",220,160);let mut body=win.body();body.set_margin(0);
        let shader=sl::Shader::new(|_,u|sl::mix(u.background(),u.accent(),sl::clamp(u.time(),0.0,1.0)));
        let view=scene::Surface::new(150,90).paint(move|r,t|{
            // A tiled compositor may expand the window: this clock test
            // needs a small shader, not an accidental fullscreen CPU bench.
            let area=Rect::new(0,0,r.w.min(150),r.h.min(90));
            scene::shader(area,&shader,&fltk_omarchy::gfx::Uniforms::from_theme(t,(area.w as f32,area.h as f32)));
        }).animate(Some(anim::Pace::Saver));
        body.add(&*view);win.end();win.show();
        // Wayland configures the final tiled size asynchronously. Do not
        // count that valid layout/focus invalidation as shader rebuilding.
        let deadline=Instant::now()+Duration::from_secs(5);
        let mut stable_since=Instant::now();
        let mut signature=(view.w(),view.h(),view.stats().scene_builds);
        let mut settled=false;
        while Instant::now()<deadline {
            pump_for(10);
            let next=(view.w(),view.h(),view.stats().scene_builds);
            if signature!=next {signature=next;stable_since=Instant::now();}
            if view.stats().cpu_frames>=3 && stable_since.elapsed()>=Duration::from_millis(120) {settled=true;break;}
        }
        check(settled,"initial layout and focus settle before pacing sample")?;
        let before=view.stats();
        pump_for(280);check(view.stats().cpu_frames>before.cpu_frames,"shared clock drives visible shader surface")?;
        check(view.stats().scene_builds==before.scene_builds,"paced frames retain scene painter")?;
        anim::set_enabled(false);view.set_time(None);pump_for(180);let frames=view.stats().cpu_frames;
        pump_for(180);check(view.stats().cpu_frames==frames,"reduced motion stops requeueing frames")?;
        anim::set_enabled(true);pump_for(180);
        check(view.stats().cpu_frames>frames,"motion-setting wake resumes a stopped visible surface")?;
        win.hide();pump_for(180);let frames=view.stats().cpu_frames;
        pump_for(180);check(view.stats().cpu_frames==frames,"hidden surface has no drawing loop")?;
        win.snapshot(shots.join("scene-surface-clock-hidden.png")).map_err(|e|e.to_string())?;
        fltk::window::DoubleWindow::delete((*win).clone());Ok(())
    });
    #[cfg(feature="gpu")]
    run("scene surface GPU: managed lifecycle, snapshots and retained shader time", &|shots| {
        use fltk_omarchy::gfx::Backend;
        use std::time::{Duration,Instant};
        let wait=|test:&mut dyn FnMut()->bool|->bool {let deadline=Instant::now()+Duration::from_secs(5);while Instant::now()<deadline{fltk_omarchy::App::pump();if test(){return true;}std::thread::sleep(Duration::from_millis(5));}false};
        let mut win=AppWindow::new("Managed scene GL",340,240);
        let mut body=win.body();body.set_margin(0);
        let shader=fltk_omarchy::gfx::sl::Shader::new(|p,u| fltk_omarchy::gfx::sl::mix(u.background(),u.accent(),fltk_omarchy::gfx::sl::clamp(p.x()/u.size().x()+u.time()*0.1,0.0,1.0)));
        let pointer=std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));let p=pointer.clone();
        let keys=std::rc::Rc::new(std::cell::Cell::new(false));let k=keys.clone();
        let view=scene::Surface::new(300,180).backend(Backend::Gpu).on_event(move|ev,x,y|{
            if ev==fltk::enums::Event::Push{p.borrow_mut().push((x,y));return false;}
            if ev==fltk::enums::Event::KeyDown{k.set(true);}false
        }).paint(move |r,t|{
            paint::fill(0,0,r.w,r.h,0,t.palette.background);
            scene::shader(r,&shader,&fltk_omarchy::gfx::Uniforms::from_theme(t,(r.w as f32,r.h as f32)));
            paint::text("Scene",8,8,90,24,14,true,t.palette.foreground,Align::Left);
        });body.add(&*view);win.end();view.set_time(Some(0.2));
        win.snapshot(shots.join("scene-surface-before-map.png")).map_err(|e|e.to_string())?;
        check(view.stats().gpu_frames==0,"snapshot before map creates no GL context")?;
        win.show();
        check(wait(&mut ||view.stats().gpu_frames>0),&format!("GL activates: {:?}",view.backend_error()))?;
        check(view.active_backend()==Backend::Gpu,"active backend reports GL")?;
        let result=std::rc::Rc::new(std::cell::Cell::new(None));let r=result.clone();
        check(view.read_gpu_pixel(200,80,move |p|r.set(Some(p))),"queue GL read")?;
        check(wait(&mut ||result.get().is_some()),"GL read completes")?;
        let first=result.get().unwrap();let builds=view.stats().scene_builds;
        view.set_time(Some(2.0));let result2=result.clone();result.set(None);
        check(view.read_gpu_pixel(200,80,move |p|result2.set(Some(p))),"queue changed GL read")?;
        check(wait(&mut ||result.get().is_some()),"changed read completes")?;
        check(first!=result.get().unwrap() && builds==view.stats().scene_builds,"shader time changes without rebuilding scene")?;
        let shot=shots.join("scene-surface-mapped.png");win.snapshot(&shot).map_err(|e|e.to_string())?;
        let cpu=png_rgb(&shot)?.at(view.x()+200,view.y()+80);let gpu=result.get().unwrap();
        check((0..3).all(|i|cpu[i].abs_diff(gpu[i])<=2),&format!("mapped snapshot uses CPU with same time: {cpu:?} / {gpu:?}"))?;
        fltk_omarchy::focus::click_at(&*win,view.x()+150,view.y()+120);
        check(pointer.borrow().as_slice()==[(150,120)],&format!("GL surface pointer is local and dispatched once: {:?}",pointer.borrow()))?;
        view.clone().take_focus().map_err(|e|e.to_string())?;fltk_omarchy::focus::press("Enter");
        check(keys.get(),"mapped GL surface receives keyboard router input")?;
        win.hide();fltk_omarchy::App::pump();
        let frames=view.stats().gpu_frames;win.show();
        check(wait(&mut ||view.stats().gpu_frames>frames),&format!("context recovers on show: {:?}",view.backend_error()))?;
        let view=view.backend(Backend::Cpu);
        let cpu_frames=view.stats().cpu_frames;
        check(wait(&mut ||view.stats().cpu_frames>cpu_frames),"switch to CPU draws fallback")?;
        check(view.active_backend()==Backend::Cpu,"CPU policy hides GL child")?;
        let view=view.backend(Backend::Gpu);let frames=view.stats().gpu_frames;
        check(wait(&mut ||view.stats().gpu_frames>frames),"GPU resumes after explicit CPU policy")?;
        struct MotionRestore;impl Drop for MotionRestore{fn drop(&mut self){fltk_omarchy::anim::set_enabled(false);}}
        let _motion=MotionRestore;fltk_omarchy::anim::set_enabled(true);
        let frames=view.stats().gpu_frames;view.set_time(None);view.clone().animate(Some(fltk_omarchy::anim::Pace::Low));
        check(wait(&mut ||view.stats().gpu_frames>=frames+2),"managed GL clock produces frames")?;
        let before=view.stats();
        check(wait(&mut ||view.stats().gpu_frames>=before.gpu_frames+3),"GL clock keeps producing frames")?;
        check(view.stats().surface_draws==before.surface_draws,"GL shader frames do not redraw the CPU parent surface")?;
        check(view.stats().scene_builds==before.scene_builds,"GL clock retains CPU painter/resources")?;
        view.clone().animate(None);
        let (oversized,())=Scene::record(||{
            // Reject before allocating a fallback texture (>64 MiB).
            paint::fill(0,0,8193,8193,10,theme::current().palette.accent);
        });
        let cpu_frames=view.stats().cpu_frames;
        view.set_scene(oversized);
        check(wait(&mut ||view.stats().cpu_frames>cpu_frames && view.backend_error().is_some()),"preparation failure returns to CPU")?;
        check(view.child(0).is_some_and(|child|!child.visible()),"failed preparation hides the previous GL child")?;
        check(view.stats().texture_bytes==0,"failed preparation releases previous GL textures")?;
        let (recovered,())=Scene::record(||paint::fill(0,0,340,240,0,theme::current().palette.accent));
        let frames=view.stats().gpu_frames;
        view.set_scene(recovered);
        check(wait(&mut ||view.stats().gpu_frames>frames),"valid scene recovers GL after preparation failure")?;
        fltk::window::DoubleWindow::delete((*win).clone());
        drop(view);fltk_omarchy::App::pump();
        Ok(())
    });
    run("scene: isolated group opacity, translation, nested layers and live backdrop", &|shots| {
        let t = theme::current();
        let (a, b, bg, other) = (t.palette.accent, t.palette.foreground, t.palette.background, t.palette.muted);
        let (child, ()) = Scene::record(|| {
            paint::fill(30, 40, 70, 60, 8, a);
            paint::fill(60, 50, 60, 60, 0, b);
        });
        let layer = scene::Layer::new(Rect::new(30, 40, 100, 80), child).map_err(|e| e.to_string())?;
        let (nested, ()) = Scene::record(|| { assert!(layer.draw_at(-20, -10, 0.5)); });
        let nested = scene::Layer::new(Rect::new(-20, -10, 100, 80), nested).map_err(|e| e.to_string())?;
        let mode = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut win = AppWindow::new("Isolated scene layers", 260, 240);
        let mut body = win.body();
        body.set_margin(0);
        let mut frame = fltk::frame::Frame::default();
        body.add(&frame);
        let (m, l, n) = (mode.clone(), layer.clone(), nested.clone());
        frame.draw(move |w| {
            let (x, y) = (w.x(), w.y());
            let base = if m.get() == 1 { other } else { bg };
            paint::fill(x, y, w.w(), w.h(), 0, base);
            // Lower half has a different backdrop, including under the
            // overlap: proves group alpha is not a flat-parent-color veil.
            paint::fill(x, y + 60, w.w(), w.h() - 60, 0, a);
            let (s, ()) = Scene::record(|| {
                if m.get() == 2 { assert!(n.draw_at(x + 20, y + 20, 0.5)); }
                else { assert!(l.draw_at(x + 20, y + 20, 0.5)); }
            });
            s.draw();
        });
        win.end();
        for i in 0..3 {
            mode.set(i);
            let p = shots.join(format!("scene-layer-{i}.png"));
            win.snapshot(&p).map_err(|e| e.to_string())?;
            let image = png_rgb(&p)?;
            let (x, y) = (frame.x(), frame.y());
            let base = if i == 1 { other } else { bg };
            let opacity = if i == 2 { (128.0 / 255.0) * (128.0 / 255.0) } else { 128.0 / 255.0 };
            check(close(image.at(x + 20, y + 20), base), "rounded transparent corner preserves current background")?;
            check(close(image.at(x + 30, y + 30), a.over(base, opacity)), "translated child over current background")?;
            check(close(image.at(x + 65, y + 45), b.over(base, opacity)), "overlap uses only top child, group alpha applied once")?;
            check(close(image.at(x + 65, y + 75), b.over(a, opacity)), "same layer over second background stripe")?;
            check(close(image.at(x + 110, y + 95), a), "transparent padding leaves background untouched")?;
        }
        nested.clear_raster();
        layer.clear_raster();
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
    run("scene: immediate/replay pixel parity and clipped damage", &|shots| {
        let mode = std::rc::Rc::new(std::cell::Cell::new(0));
        let mut win = AppWindow::new("Scene parity", 320, 240);
        let mut body = win.body();
        body.set_margin(0);
        let mut canvas = fltk::frame::Frame::default().with_size(280, 150);
        body.add(&canvas);
        let m = mode.clone();
        canvas.draw(move |w| {
            let (x, y) = (w.x(), w.y());
            match m.get() {
                0 => content(x, y),
                1 => Scene::record(|| content(x, y)).0.draw(),
                _ => {
                    let bg = theme::with(|t| t.palette.background);
                    paint::fill(x, y, w.w(), w.h(), 0, bg);
                    let (s, ()) = Scene::record(|| content(x, y));
                    // Tile the complete image through disjoint damage regions.
                    for yy in (0..150).step_by(17) {
                        for xx in (0..280).step_by(23) {
                            assert!(s.draw_damage(Rect::new(x + xx, y + yy, 23.min(280 - xx), 17.min(150 - yy))));
                        }
                    }
                }
            }
        });
        win.end();
        let direct = shots.join("scene-direct.png");
        win.snapshot(&direct).map_err(|e| e.to_string())?;
        for i in 1..=2 {
            mode.set(i);
            let p = shots.join(format!("scene-replay-{i}.png"));
            win.snapshot(&p).map_err(|e| e.to_string())?;
            let expected = png_rgb(&direct)?;
            let actual = png_rgb(&p)?;
            let mut mismatches = Vec::new();
            let mut count = 0usize;
            let mut max_delta = 0u8;
            let height = expected.raw.len() / (expected.w * expected.depth + 1);
            for yy in 0..height as i32 {
                for xx in 0..expected.w as i32 {
                    let a = expected.at(xx, yy);
                    let b = actual.at(xx, yy);
                    if a != b {
                        count += 1;
                        max_delta = max_delta.max((0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap());
                        if mismatches.len() < 12 {
                            mismatches.push((xx - canvas.x(), yy - canvas.y(), a, b));
                        }
                    }
                }
            }
            check(count == 0, &format!("whole image parity, mode {i}: {count} differing pixels, max channel delta {max_delta}, first local (x,y,expected,actual): {mismatches:?}"))?;
        }
        fltk::window::DoubleWindow::delete((*win).clone());
        Ok(())
    });
}

fn close(actual: [u8; 3], expected: theme::Rgb) -> bool {
    actual[0].abs_diff(expected.r()) <= 2 && actual[1].abs_diff(expected.g()) <= 2 && actual[2].abs_diff(expected.b()) <= 2
}
