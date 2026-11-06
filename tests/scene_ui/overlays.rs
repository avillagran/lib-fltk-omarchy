//! Real native-child read-back: CPU snapshots cannot prove GL stacking.
use super::super::{check, png_rgb};
use fltk::{enums::Align, prelude::*};
use fltk_omarchy::{
    anim,
    gfx::{self, Backend},
    paint,
    popups::{PopupCard, ShortcutHelp},
    scene::{self, Surface},
    App, AppWindow,
};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};
type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;
pub(super) fn wait(mut condition: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(6);
    while Instant::now() < end {
        App::pump();
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}
fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        App::pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn pixel(surface: &Surface, x: i32, y: i32) -> Result<[u8; 4], String> {
    let value = Rc::new(Cell::new(None));
    let reply = value.clone();
    check(
        surface.read_gpu_pixel(x, y, move |p| reply.set(Some(p))),
        "queue live pixel",
    )?;
    check(wait(|| value.get().is_some()), "live pixel completes")?;
    Ok(value.get().unwrap())
}
pub(super) fn parity(win: &mut AppWindow, surface: &Surface, shots: &Path, name: &str) -> Result<(), String> {
    let image = Rc::new(RefCell::new(None));
    let reply = image.clone();
    check(
        surface.read_gpu_image(move |r| *reply.borrow_mut() = Some(r)),
        "queue real GL capture",
    )?;
    check(wait(|| image.borrow().is_some()), "capture completes")?;
    let image = image.borrow_mut().take().unwrap()?;
    fltk_omarchy::png::write(
        &shots.join(format!("{name}-gpu.png")),
        image.width(),
        image.height(),
        4,
        image.pixels(),
    )
    .map_err(|e| e.to_string())?;
    let path = shots.join(format!("{name}-cpu.png"));
    let gpu_frames = surface.stats().gpu_frames;
    win.snapshot(&path).map_err(|e| e.to_string())?;
    check(
        surface.stats().gpu_frames == gpu_frames,
        "CPU snapshot never prints native GL children a second time",
    )?;
    let cpu = png_rgb(&path)?;
    let mut delta = 0;
    for y in 0..surface.h() {
        for x in 0..surface.w() {
            let rgb = cpu.at(surface.x() + x, surface.y() + y);
            let i = ((y * surface.w() + x) * 4) as usize;
            for (c, v) in rgb.iter().enumerate() {
                delta = delta.max(v.abs_diff(image.pixels()[i + c]));
            }
        }
    }
    check(
        delta <= 2,
        &format!("{name}: complete real GL/CPU max RGB delta {delta}"),
    )
}
pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("scene capture GPU: mapped portable View uses a fresh CPU reference without native printing", &|shots|{
        anim::set_enabled(false);
        let mut win=AppWindow::new("Portable capture",220,180);let mut body=win.body();body.set_margin(0);
        let view=gfx::View::new(180,120).shader(gfx::shaders::gradient()).backend(Backend::Gpu);
        body.add(&*view);win.end();view.set_time(Some(0.2));win.show();
        check(wait(||view.active_backend()==Backend::Gpu && view.stats().frames>0),"portable View GL starts")?;
        view.set_time(Some(2.0));App::pump();
        let value=Rc::new(Cell::new(None));let reply=value.clone();
        check(view.read_gpu_pixel(40,40,move|p|reply.set(Some(p))),"read portable View GPU")?;
        check(wait(||value.get().is_some()),"portable read completes")?;
        let before=view.stats().frames;
        let path=shots.join("scene-capture-portable-view.png");win.snapshot(&path).map_err(|e|e.to_string())?;
        let cpu=png_rgb(&path)?.at(view.x()+40,view.y()+40);let gpu=value.get().unwrap();
        check(cpu.iter().zip(gpu).all(|(a,b)|a.abs_diff(b)<=2),"mapped View CPU fallback samples its latest shader phase")?;
        check(view.stats().frames==before && view.active_backend()==Backend::Gpu,"CPU reference neither prints nor hides the live GL child")?;
        fltk::window::DoubleWindow::delete((*win).clone());Ok(())
    });
    run(
        "scene overlay GPU: unsupported native descendants fall back and closing them restores GPU",
        &|_| {
            anim::set_enabled(false);
            let mut win = AppWindow::new("Overlay fallback", 480, 360);
            let mut body = win.body();
            body.set_margin(0);
            let surface = Surface::new(480, 319)
                .backend(Backend::Gpu)
                .paint(|r, t| paint::fill(0, 0, r.w, r.h, 0, t.palette.accent));
            body.add(&*surface);
            win.end();
            win.show();
            check(
                wait(|| surface.active_backend() == Backend::Gpu),
                "GL ready",
            )?;
            let mut nested = None;
            let popup = PopupCard::open_at(&*surface, 40, 60, 220, 130, |col| {
                let child = fltk::window::Window::new(col.x(), col.y(), 100, 45, "Native child");
                child.end();
                nested = Some(child);
            });
            let mut nested = nested.unwrap();
            nested.show();
            check(
                wait(|| {
                    surface.backend_error().is_some() && surface.active_backend() == Backend::Cpu
                }),
                "native-window subtree produces an explicit CPU fallback",
            )?;
            check(
                surface.stats().texture_bytes == 0,
                "fallback releases old scene and overlay GPU resources",
            )?;
            popup.close();
            check(
                wait(|| {
                    surface.active_backend() == Backend::Gpu && surface.backend_error().is_none()
                }),
                "removing incompatible overlay recovers GPU without app invalidation",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run(
        "scene overlay GPU: frozen enter/exit phases, stacked layers and full removal",
        &|shots| {
            use fltk_omarchy::{
                focus,
                popups::{PopupItem, PopupMenu},
                theme,
            };
            struct Restore(theme::Theme);
            impl Drop for Restore {
                fn drop(&mut self) {
                    anim::clock::resume();
                    anim::set_enabled(false);
                    theme::set(self.0.clone());
                }
            }
            let original = theme::current();
            let _restore = Restore(original.clone());
            anim::set_enabled(true);
            anim::clock::freeze();
            let mut win = AppWindow::new("Overlay phases", 640, 480);
            let mut body = win.body();
            body.set_margin(0);
            let surface = Surface::new(640, 439)
                .backend(Backend::Gpu)
                .paint(|r, t| paint::fill(0, 0, r.w, r.h, 0, t.palette.accent));
            body.add(&*surface);
            win.end();
            win.show();
            check(
                wait(|| surface.stats().gpu_frames > 0),
                "GL ready with frozen clock",
            )?;
            let popup = PopupMenu::open_at(
                &*surface,
                80,
                120,
                vec![
                    PopupItem::new("", "First item"),
                    PopupItem::new("", "Second item"),
                ],
                |_| {},
            );
            App::pump();
            anim::clock::advance(Duration::from_millis(90));
            win.redraw();
            App::pump();
            parity(&mut win, &surface, shots, "scene-overlay-half-enter")?;
            anim::clock::advance(Duration::from_millis(180));
            win.redraw();
            App::pump();
            parity(&mut win, &surface, shots, "scene-overlay-entered")?;
            let mut changed = original.clone();
            changed.palette.accent = original.palette.foreground;
            changed.popups.text = original.palette.accent;
            theme::set(changed);
            win.redraw();
            App::pump();
            parity(&mut win, &surface, shots, "scene-overlay-theme")?;
            win.set_size(560, 440);
            win.redraw();
            App::pump();
            parity(&mut win, &surface, shots, "scene-overlay-resized")?;
            let _help = ShortcutHelp::open(&*surface, "Stacked help", vec![]);
            anim::clock::advance(Duration::from_millis(190));
            win.redraw();
            App::pump();
            parity(&mut win, &surface, shots, "scene-overlay-stacked")?;
            focus::press("Esc");
            anim::clock::advance(Duration::from_millis(60));
            win.redraw();
            App::pump();
            parity(&mut win, &surface, shots, "scene-overlay-half-exit")?;
            popup.close();
            anim::set_enabled(false);
            pump(180);
            win.redraw();
            App::pump();
            check(
                wait(|| surface.stats().overlay_texture_bytes == 0),
                "all open and closing overlays release their texture",
            )?;
            check(
                surface.active_backend() == Backend::Gpu,
                "fades and stacked layers keep the GPU context",
            )?;
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
    run("scene overlay GPU: CPU widgets receive native-child clicks; static texture is retained while shader animates", &|shots|{
        use fltk_omarchy::{focus,layout::FlexExt,widgets::Button};
        struct Restore;
        impl Drop for Restore {fn drop(&mut self){anim::clock::resume();anim::set_enabled(false);}}
        let _restore=Restore;
        anim::set_enabled(false);
        let mut win=AppWindow::new("Widget bridge",640,480);let mut body=win.body();body.set_margin(0);
        let clicks=Rc::new(Cell::new(0));let hit=clicks.clone();
        let shader=gfx::shaders::gradient();
        let surface=Surface::new(640,439).backend(Backend::Gpu).on_event(move |e,_,_|{if e==fltk::enums::Event::Push{hit.set(hit.get()+1);}false}).paint(move |r,t|{scene::shader(r,&shader,&gfx::Uniforms::from_theme(t,(r.w as f32,r.h as f32)));});
        body.add(&*surface);win.end();win.show();
        check(wait(||surface.stats().gpu_frames>0),"GL ready")?;
        let picked=Rc::new(Cell::new(0));let callback=picked.clone();let mut button=None;
        let popup=PopupCard::open_at(&*surface,70,90,260,150,|col|{
            let b=Button::new("Click the CPU widget").on_click(move ||callback.set(callback.get()+1));
            col.fit(&*b);button=Some(b);
            // Native FLTK drawing bypasses the toolkit's shared painter.
            let mut raw=fltk::frame::Frame::default().with_size(180,28).with_label("Native FLTK label");
            raw.set_frame(fltk::enums::FrameType::FlatBox);
            raw.set_color(fltk_omarchy::theme::with(|t|t.palette.lighter_background.fl()));
            raw.set_label_color(fltk_omarchy::theme::with(|t|t.palette.foreground.fl()));
            col.fit(&raw);
        });
        App::pump();parity(&mut win,&surface,shots,"scene-widget-popup")?;
        let button=button.unwrap();
        let child=surface.child(0).ok_or("native child")?;
        let gl=fltk::window::GlWindow::from_dyn_widget_ptr(child.as_widget_ptr()).ok_or("GL child type")?;
        let point=(button.x()+button.w()/2-surface.x(),button.y()+button.h()/2-surface.y());
        focus::click_at(&gl,point.0,point.1);
        check(fltk::app::event_coords()==point,"native event coordinates restore after routing")?;
        App::pump();
        check(picked.get()==1 && clicks.get()==0,"native-child click reaches the popup control once, never the scene")?;
        anim::set_enabled(true);surface.clone().animate(Some(anim::Pace::Display));
        pump(150);let before=surface.stats();pump(240);let after=surface.stats();
        check(after.gpu_frames>before.gpu_frames && after.overlay_captures==before.overlay_captures && after.overlay_uploads==before.overlay_uploads && after.gpu_instance_upload_bytes==before.gpu_instance_upload_bytes && after.gpu_preparations==before.gpu_preparations && after.gpu_scene_uploads==before.gpu_scene_uploads && after.surface_draws==before.surface_draws,"live shader reuses the static CPU overlay without capture, upload or parent draws")?;
        check(after.overlay_texture_bytes>0 && surface.active_backend()==Backend::Gpu,"widget popup is composed in the active GPU framebuffer")?;
        check(after.instance_bytes==0 && after.gpu_instance_upload_bytes==0,"singleton shader/overlay scene never allocates or uploads an instance store")?;
        surface.set_time(Some(0.0));let a=pixel(&surface,10,300)?;
        surface.set_time(Some(1.0));let b=pixel(&surface,10,300)?;
        check(a!=b,"shader pixels change behind the retained popup")?;
        check(surface.stats().overlay_uploads==after.overlay_uploads,"shader phase changes never upload the popup again")?;
        anim::set_enabled(false);surface.clone().animate(None);popup.close();
        check(wait(||surface.stats().overlay_texture_bytes==0),"closing the last overlay releases its GPU texture")?;
        pump(100);let frames=surface.stats().gpu_frames;pump(160);
        check(surface.stats().gpu_frames==frames,"finished scene has no idle redraw loop")?;
        fltk::window::DoubleWindow::delete((*win).clone());Ok(())
    });
    run(
        "scene overlay GPU: actual help/scrim pixels match CPU with GPU still active",
        &|shots| {
            anim::set_enabled(false);
            let mut win = AppWindow::new("Overlay bridge", 640, 480);
            let mut body = win.body();
            body.set_margin(0);
            let shader = gfx::shaders::gradient();
            let surface = Surface::new(640, 439)
                .backend(Backend::Gpu)
                .paint(move |r, t| {
                    scene::shader(
                        r,
                        &shader,
                        &gfx::Uniforms::from_theme(t, (r.w as f32, r.h as f32)),
                    );
                    paint::text(
                        "Live GPU background",
                        20,
                        20,
                        r.w - 40,
                        30,
                        16,
                        true,
                        t.palette.foreground,
                        Align::Left,
                    );
                });
            body.add(&*surface);
            win.end();
            win.show();
            check(wait(|| surface.stats().gpu_frames > 0), "GL activation")?;
            let _help = ShortcutHelp::open(
                &*surface,
                "GPU help",
                vec![(
                    "Scene".into(),
                    vec![
                        ("Mod+S".into(), "Save the scene".into()),
                        ("Esc".into(), "Close help".into()),
                    ],
                )],
            );
            App::pump();
            let image = Rc::new(RefCell::new(None));
            let reply = image.clone();
            check(
                surface.read_gpu_image(move |r| *reply.borrow_mut() = Some(r)),
                "queue actual overlay frame",
            )?;
            check(wait(|| image.borrow().is_some()), "GPU capture")?;
            let image = image.borrow_mut().take().unwrap()?;
            fltk_omarchy::png::write(
                &shots.join("scene-overlay-gpu.png"),
                image.width(),
                image.height(),
                4,
                image.pixels(),
            )
            .map_err(|e| e.to_string())?;
            let cpu = shots.join("scene-overlay-cpu.png");
            win.snapshot(&cpu).map_err(|e| e.to_string())?;
            let cpu = png_rgb(&cpu)?;
            let mut delta = 0;
            for y in 0..surface.h() {
                for x in 0..surface.w() {
                    let rgb = cpu.at(surface.x() + x, surface.y() + y);
                    let i = ((y * surface.w() + x) * 4) as usize;
                    for (c, v) in rgb.iter().enumerate() {
                        delta = delta.max(v.abs_diff(image.pixels()[i + c]));
                    }
                }
            }
            check(
                delta <= 2,
                &format!("actual GPU help and scrim agree with CPU: max RGB delta {delta}"),
            )?;
            check(
                surface.active_backend() == Backend::Gpu,
                "overlay never hides the native GL child",
            )?;
            fltk_omarchy::focus::press("Esc");
            fltk::window::DoubleWindow::delete((*win).clone());
            Ok(())
        },
    );
}
