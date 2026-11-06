//! Managed local-coordinate scene surface. No GL calls in application code.
use super::{Rect, Scene};
use crate::{
    anim::{self, Pace},
    gfx::Backend,
    i18n, paint, theme,
};
use fltk::{app, enums::*, group::Group, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

type Painter = Box<dyn FnMut(Rect, &theme::Theme)>;
type Events = Box<dyn FnMut(Event, i32, i32) -> bool>;
thread_local! { static CPU_SCOPE: Cell<bool> = const { Cell::new(false) }; }
struct Reset(bool);
impl Drop for Reset {
    fn drop(&mut self) {
        CPU_SCOPE.with(|s| s.set(self.0));
    }
}
/// AppWindow snapshots render scene surfaces through the CPU, including
/// surfaces already mapped on GL. The scope restores state on unwind.
pub(crate) fn cpu_scope<T>(f: impl FnOnce() -> T) -> T {
    let _reset = Reset(CPU_SCOPE.with(|s| s.replace(true)));
    f()
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SurfaceStats {
    pub scene_builds: u64,
    pub surface_draws: u64,
    pub cpu_frames: u64,
    pub gpu_frames: u64,
    pub texture_bytes: usize,
}
struct State {
    scene: Rc<Scene>,
    painter: Option<Painter>,
    events: Option<Events>,
    last_event: Option<(u64, Event, bool)>,
    dirty: bool,
    revision: u64,
    size: (i32, i32),
    ring: bool,
    backend: Backend,
    error: Option<String>,
    gpu_renderer: Option<String>,
    time: Option<f32>,
    pace: Option<Pace>,
    stats: SurfaceStats,
    started: std::time::Instant,
    activation_pending: bool,
    ready: bool,
    #[cfg(feature = "gpu")]
    native: Option<native::Native>,
}

/// A scene in local coordinates. `paint` records a retained scene on size,
/// theme, language, focus or explicit invalidation; shader time updates do
/// not re-run that painter. CPU by default, even with feature `gpu`.
/// Use one surface for a composed region, not one GL surface per row.
///
/// ```no_run
/// use fltk_omarchy::{scene::Surface, paint, theme};
/// let canvas = Surface::new(400, 240).paint(|r, t| {
///     paint::fill(0, 0, r.w, r.h, 0, t.palette.background);
/// });
/// ```
/// `on_event` receives local pointer coordinates; use `KeyboardPanel` and
/// `focus::host` for virtual collections and region keyboard/accessibility
/// behavior. This surface does not turn drawn labels into controls.
#[derive(Clone)]
pub struct Surface {
    group: Group,
    st: Rc<RefCell<State>>,
}
fltk::widget_extends!(Surface, Group, group);
impl Surface {
    pub fn new(w: i32, h: i32) -> Self {
        let mut group = Group::default().with_size(w, h);
        group.set_frame(FrameType::NoBox);
        group.end();
        group.super_draw(false);
        group.super_handle_first(false);
        crate::focus::control_group(&group);
        let st = Rc::new(RefCell::new(State {
            scene: Rc::new(Scene::default()),
            painter: None,
            events: None,
            last_event: None,
            dirty: true,
            revision: 0,
            size: (0, 0),
            ring: false,
            backend: Backend::Cpu,
            error: None,
            gpu_renderer: None,
            time: None,
            pace: None,
            stats: SurfaceStats::default(),
            started: anim::now(),
            activation_pending: false,
            ready: false,
            #[cfg(feature = "gpu")]
            native: None,
        }));
        let s = st.clone();
        group.draw(move |g| draw(g, &s));
        let s = st.clone();
        group.handle(move |g, event| {
            if event == Event::Hide {
                s.borrow_mut().ready = false;
                #[cfg(feature = "gpu")]
                native::hide(&s);
            }
            if matches!(event, Event::Focus | Event::Unfocus) {
                g.redraw();
            }
            let (x, y) = (app::event_x() - g.x(), app::event_y() - g.y());
            dispatch(g, &s, event, x, y)
        });
        let (s, mut g) = (st.clone(), group.clone());
        theme::on_change_while(&group, move |_| {
            invalidate(&s);
            g.redraw();
        });
        let (s, mut g) = (st.clone(), group.clone());
        i18n::bind(&group, move || {
            invalidate(&s);
            g.redraw();
        });
        // Checks run only when the application loop is awake. The system
        // motion watcher wakes it, so an enabled setting can resume a
        // previously stopped surface without a polling timer.
        let (s, mut g) = (Rc::downgrade(&st), group.clone());
        let enabled = Cell::new(anim::enabled());
        app::add_check(move |handle| {
            let Some(s) = s.upgrade() else {
                app::remove_check(handle);
                return;
            };
            if g.was_deleted() {
                app::remove_check(handle);
                return;
            }
            let now = anim::enabled();
            if enabled.replace(now) != now && s.borrow().pace.is_some() {
                g.redraw();
            }
        });
        Self { group, st }
    }
    pub fn paint(self, painter: impl FnMut(Rect, &theme::Theme) + 'static) -> Self {
        self.st.borrow_mut().painter = Some(Box::new(painter));
        self.invalidate();
        self
    }
    pub fn set_scene(&self, scene: Scene) {
        let mut s = self.st.borrow_mut();
        s.painter = None;
        s.scene = Rc::new(scene);
        drop(s);
        self.invalidate();
    }
    pub fn invalidate(&self) {
        invalidate(&self.st);
        if !self.group.was_deleted() {
            self.group.clone().redraw();
        }
    }
    /// Explicit rendering policy. Auto stays on CPU until the measured
    /// whole-window policy is ready; Gpu opts into the current compositor.
    pub fn backend(self, backend: Backend) -> Self {
        {
            let mut s = self.st.borrow_mut();
            s.backend = backend;
            s.ready = false;
        }
        self.invalidate();
        self
    }
    pub fn active_backend(&self) -> Backend {
        #[cfg(feature = "gpu")]
        if self
            .st
            .borrow()
            .native
            .as_ref()
            .is_some_and(|n| n.visible())
            && self.st.borrow().error.is_none()
        {
            return Backend::Gpu;
        }
        Backend::Cpu
    }
    pub fn backend_error(&self) -> Option<String> {
        self.st.borrow().error.clone()
    }
    /// Last initialized GL device/driver name, for diagnostics.
    pub fn renderer_name(&self) -> Option<String> {
        self.st.borrow().gpu_renderer.clone()
    }
    pub fn stats(&self) -> SurfaceStats {
        self.st.borrow().stats
    }
    /// None uses the time recorded in the shader commands. A frozen
    /// override updates only uniforms on GL and portable shaders on CPU.
    pub fn set_time(&self, time: Option<f32>) {
        self.st.borrow_mut().time = time;
        if !self.group.was_deleted() {
            self.group.clone().redraw();
        }
    }
    /// Continuous shader pacing through the toolkit's shared clock.
    /// Hidden surfaces and reduced motion stop re-queuing frames. Time
    /// advances on the animation clock unless set_time freezes it. Reduced
    /// motion draws phase zero; animate does not rebuild a painter per frame.
    pub fn animate(self, pace: Option<Pace>) -> Self {
        self.st.borrow_mut().pace = pace;
        self.group.clone().redraw();
        self
    }
    /// Queue a logical local pixel read after the next GL frame. Returns
    /// false on CPU, before activation, or outside the surface. Snapshots
    /// use the CPU path; this method checks the actual GL output.
    pub fn read_gpu_pixel(&self, x: i32, y: i32, f: impl FnOnce([u8; 4]) + 'static) -> bool {
        if x < 0 || y < 0 || x >= self.group.w() || y >= self.group.h() {
            return false;
        }
        #[cfg(feature = "gpu")]
        {
            let win = {
                let s = self.st.borrow();
                let Some(n) = s.native.as_ref() else {
                    return false;
                };
                if !n.visible() || s.error.is_some() {
                    return false;
                }
                if !n.queue_pixel(x, y, Box::new(f)) {
                    return false;
                }
                n.win.clone()
            };
            win.clone().redraw();
            true
        }
        #[cfg(not(feature = "gpu"))]
        {
            let _ = (x, y, f);
            false
        }
    }
    pub fn on_event(self, events: impl FnMut(Event, i32, i32) -> bool + 'static) -> Self {
        self.st.borrow_mut().events = Some(Box::new(events));
        self
    }
}
fn invalidate(st: &Rc<RefCell<State>>) {
    let mut s = st.borrow_mut();
    s.dirty = true;
    s.revision = s.revision.wrapping_add(1);
    s.error = None;
}
fn scene_time(s: &State) -> Option<f32> {
    s.time.or_else(|| {
        s.pace.map(|_| {
            if anim::enabled() {
                anim::now()
                    .saturating_duration_since(s.started)
                    .as_secs_f32()
            } else {
                0.0
            }
        })
    })
}
fn dispatch(g: &mut Group, st: &Rc<RefCell<State>>, ev: Event, x: i32, y: i32) -> bool {
    let serial = crate::focus::dispatch_serial();
    if serial != 0 {
        if let Some((previous, event, used)) = st.borrow().last_event {
            if previous == serial && event == ev {
                return used;
            }
        }
    }
    if ev == Event::Push {
        let _ = g.take_focus();
    }
    let mut callback = st.borrow_mut().events.take();
    let used = callback.as_mut().is_some_and(|f| f(ev, x, y));
    if st.borrow().events.is_none() {
        st.borrow_mut().events = callback;
    }
    let used = used || ev == Event::Focus;
    if serial != 0 {
        st.borrow_mut().last_event = Some((serial, ev, used));
    }
    used
}
fn draw(g: &mut Group, st: &Rc<RefCell<State>>) {
    st.borrow_mut().stats.surface_draws += 1;
    if g.w() <= 0 || g.h() <= 0 {
        return;
    }
    let shown = g.visible_r() && g.top_window().is_some_and(|w| w.shown());
    let offscreen = CPU_SCOPE.with(Cell::get);
    let ring = crate::focus::show_ring(g);
    let rebuild = {
        let s = st.borrow();
        s.dirty || s.size != (g.w(), g.h()) || s.ring != ring
    };
    if rebuild {
        let (mut painter, revision) = {
            let mut s = st.borrow_mut();
            (s.painter.take(), s.revision)
        };
        let scene = painter
            .as_mut()
            .map(|p| Scene::record(|| p(Rect::new(0, 0, g.w(), g.h()), &theme::current())).0);
        let mut s = st.borrow_mut();
        if s.revision == revision {
            if let Some(scene) = scene {
                s.scene = Rc::new(scene);
            }
            s.size = (g.w(), g.h());
            s.ring = ring;
            s.dirty = false;
            s.stats.scene_builds += 1;
        }
        if s.painter.is_none() {
            s.painter = painter;
        }
        drop(s);
        #[cfg(feature = "gpu")]
        native::prepare(g, st);
    }
    #[cfg(feature = "gpu")]
    let mut gpu = false;
    #[cfg(not(feature = "gpu"))]
    let gpu = false;
    #[cfg(feature = "gpu")]
    if !offscreen {
        if st.borrow().backend != Backend::Gpu || st.borrow().error.is_some() {
            native::hide(st);
        } else if st.borrow().error.is_none() {
            let win = {
                let s = st.borrow();
                if s.ready {
                    s.native.as_ref().map(|n| n.win.clone())
                } else {
                    None
                }
            };
            if let Some(mut win) = win {
                if !win.was_deleted() {
                    win.resize(g.x(), g.y(), g.w(), g.h());
                    if !win.visible() {
                        win.show();
                    }
                    win.redraw();
                    gpu = true;
                }
            }
        }
    }
    if gpu {
        g.draw_children();
    } else {
        theme::with(|t| paint::fill(g.x(), g.y(), g.w(), g.h(), 0, t.palette.background));
        let (scene, time) = {
            let s = st.borrow();
            (s.scene.clone(), scene_time(&s))
        };
        super::clip(Rect::new(g.x(), g.y(), g.w(), g.h()), || {
            scene.draw_at(g.x(), g.y(), time);
        });
        if ring {
            theme::with(|t| {
                paint::stroke(
                    g.x() + 1,
                    g.y() + 1,
                    g.w() - 2,
                    g.h() - 2,
                    0,
                    2,
                    t.palette.accent,
                )
            });
        }
        st.borrow_mut().stats.cpu_frames += 1;
    }
    #[cfg(feature = "gpu")]
    if shown
        && !offscreen
        && !gpu
        && st.borrow().backend == Backend::Gpu
        && st.borrow().error.is_none()
    {
        native::schedule(g, st);
    }
    #[cfg(not(feature = "gpu"))]
    if st.borrow().backend == Backend::Gpu {
        st.borrow_mut().error = Some("built without the gpu feature".into());
    }
    if !gpu && shown && !offscreen && anim::enabled() {
        let s = st.borrow();
        if let Some(pace) = s.pace {
            if !s.activation_pending
                && s.time.is_none()
                && s.scene
                    .commands()
                    .iter()
                    .any(|c| matches!(c, super::Command::Shader { .. }))
            {
                anim::request_redraw_at(g, pace.min(Pace::Low));
            }
        }
    }
}

#[cfg(feature = "gpu")]
mod native {
    use super::super::gpu::{PreparedScene, Renderer};
    use super::*;
    use fltk::window::GlWindow;
    use glow::HasContext;
    struct Gl {
        renderer: Option<Renderer>,
        win: GlWindow,
        ctx: Option<Rc<glow::Context>>,
        context: usize,
        prepared: Option<PreparedScene>,
        owner: std::rc::Weak<RefCell<State>>,
        pending_scene: bool,
        readbacks: Vec<(i32, i32, Box<dyn FnOnce([u8; 4])>)>,
    }
    impl Drop for Gl {
        fn drop(&mut self) {
            if let Some(owner) = self.owner.upgrade() {
                owner.borrow_mut().stats.texture_bytes = 0;
            }
            if let Some(renderer) = self.renderer.take() {
                if !self.win.was_deleted()
                    && self.win.context_valid()
                    && self
                        .win
                        .context()
                        .is_some_and(|c| c as usize == self.context)
                {
                    self.win.make_current();
                    drop(renderer);
                } else {
                    renderer.abandon_context();
                }
            }
        }
    }
    pub(super) struct Native {
        pub win: GlWindow,
        gl: std::rc::Weak<RefCell<Gl>>,
    }
    impl Native {
        pub fn queue_pixel(&self, x: i32, y: i32, f: Box<dyn FnOnce([u8; 4])>) -> bool {
            let Some(gl) = self.gl.upgrade() else {
                return false;
            };
            gl.borrow_mut().readbacks.push((x, y, f));
            true
        }
        pub fn visible(&self) -> bool {
            !self.win.was_deleted() && self.win.visible() && self.win.shown()
        }
    }
    pub fn hide(st: &Rc<RefCell<State>>) {
        let native = st
            .borrow()
            .native
            .as_ref()
            .and_then(|n| n.gl.upgrade().map(|gl| (n.win.clone(), gl)));
        if let Some((mut win, gl)) = native {
            if !win.was_deleted() && win.visible() {
                let mut gl = gl.borrow_mut();
                if let Some(renderer) = gl.renderer.take() {
                    if win.context_valid()
                        && win.context().is_some_and(|c| c as usize == gl.context)
                    {
                        win.make_current();
                        drop(renderer);
                    } else {
                        renderer.abandon_context();
                    }
                }
                gl.ctx = None;
                gl.context = 0;
                gl.pending_scene = true;
                drop(gl);
                {
                    let mut s = st.borrow_mut();
                    s.dirty = true;
                    s.stats.texture_bytes = 0;
                    s.revision = s.revision.wrapping_add(1);
                }
                win.hide();
            }
        }
    }
    pub fn prepare(g: &Group, st: &Rc<RefCell<State>>) {
        if st.borrow().backend != Backend::Gpu || st.borrow().native.is_none() {
            return;
        }
        let scene = {
            let s = st.borrow();
            with_ring(&s.scene, s.ring, g.w(), g.h())
        };
        let result = PreparedScene::new(&scene, Rect::new(0, 0, g.w(), g.h()));
        let mut s = st.borrow_mut();
        match result {
            Ok(prepared) => {
                if let Some(n) = &s.native {
                    if let Some(gl) = n.gl.upgrade() {
                        let mut gl = gl.borrow_mut();
                        gl.prepared = Some(prepared);
                        gl.pending_scene = true;
                    }
                }
            }
            Err(error) => s.error = Some(error),
        }
    }
    fn with_ring(scene: &Scene, ring: bool, w: i32, h: i32) -> Scene {
        let mut scene = scene.clone();
        if ring {
            let (outline, ()) = Scene::record(|| {
                theme::with(|t| paint::stroke(1, 1, w - 2, h - 2, 0, 2, t.palette.accent))
            });
            scene.append(&outline);
        }
        scene
    }
    pub fn schedule(g: &Group, st: &Rc<RefCell<State>>) {
        if std::mem::replace(&mut st.borrow_mut().activation_pending, true) {
            return;
        }
        let (mut group, owner) = (g.clone(), Rc::downgrade(st));
        let ticks = Cell::new(0);
        app::add_timeout3(0.05, move |timer| {
            let Some(st) = owner.upgrade() else { return };
            if group.was_deleted() {
                return;
            }
            if !group.visible_r()
                || !group.top_window().is_some_and(|w| w.shown())
                || st.borrow().backend != Backend::Gpu
            {
                st.borrow_mut().activation_pending = false;
                return;
            }
            ticks.set(ticks.get() + 1);
            if ticks.get() < 3 {
                app::repeat_timeout3(0.05, timer);
                return;
            }
            st.borrow_mut().activation_pending = false;
            if st.borrow().error.is_some() {
                return;
            }
            let existing = st.borrow().native.as_ref().map(|n| n.win.clone());
            if let Some(mut win) = existing {
                st.borrow_mut().ready = true;
                if !win.was_deleted() {
                    win.show();
                    group.redraw();
                }
                return;
            }
            let scene = {
                let s = st.borrow();
                with_ring(&s.scene, s.ring, group.w(), group.h())
            };
            let prepared = match PreparedScene::new(&scene, Rect::new(0, 0, group.w(), group.h())) {
                Ok(p) => p,
                Err(e) => {
                    st.borrow_mut().error = Some(e);
                    group.redraw();
                    return;
                }
            };
            group.begin();
            let mut win = GlWindow::new(group.x(), group.y(), group.w(), group.h(), None);
            win.set_mode(Mode::Opengl3 | Mode::Double | Mode::Rgb8);
            win.end();
            group.end();
            win.super_draw(false);
            win.super_handle_first(false);
            let gl = Rc::new(RefCell::new(Gl {
                renderer: None,
                win: win.clone(),
                ctx: None,
                context: 0,
                prepared: Some(prepared),
                owner: Rc::downgrade(&st),
                pending_scene: true,
                readbacks: Vec::new(),
            }));
            let s = gl.clone();
            win.draw(move |w| render(w, &s));
            let (s, mut g) = (Rc::downgrade(&st), group.clone());
            win.handle(move |_, event| {
                let Some(st) = s.upgrade() else { return false };
                if matches!(event, Event::Hide | Event::Show) {
                    return false;
                }
                // The group is the single Tab stop; the GL child is an
                // implementation detail, never the logical keyboard owner.
                if event == Event::Focus {
                    let _ = g.take_focus();
                    return false;
                }
                if event == Event::KeyDown && !crate::focus::is_focused(&g) {
                    return false;
                }
                // Subwindow events are already relative to its origin.
                dispatch(&mut g, &st, event, app::event_x(), app::event_y())
            });
            {
                let mut s = st.borrow_mut();
                s.native = Some(Native {
                    win: win.clone(),
                    gl: Rc::downgrade(&gl),
                });
                s.ready = true;
            }
            win.show();
            group.redraw();
        });
    }
    fn render(win: &mut GlWindow, state: &Rc<RefCell<Gl>>) {
        let mut gl = state.borrow_mut();
        let Some(owner) = gl.owner.upgrade() else {
            return;
        };
        let context = win.context().map_or(0, |c| c as usize);
        let fresh_context = !win.context_valid() || gl.context != context;
        if fresh_context {
            if let Some(r) = gl.renderer.take() {
                r.abandon_context();
            }
            gl.context = context;
            gl.pending_scene = true;
        }
        if gl.pending_scene {
            // Preparation uses CPU surfaces outside this callback. After a
            // context loss, regenerate it on a deferred UI turn instead.
            if gl.prepared.is_none() {
                drop(gl);
                let s = Rc::downgrade(&owner);
                let parent = win.parent();
                app::add_timeout3(0.0, move |_| {
                    if let Some(s) = s.upgrade() {
                        invalidate(&s);
                        if let Some(mut p) = parent.clone() {
                            if !p.was_deleted() {
                                p.redraw();
                            }
                        }
                    }
                });
                return;
            }
            drop(gl.renderer.take()); // our current context, safe cleanup
            let prepared = gl.prepared.take().unwrap();
            let loader = win.clone();
            // SAFETY: current FLTK GL draw context. Destruction happens in
            // this callback, or names are abandoned after native context loss.
            let ctx = Rc::new(unsafe {
                glow::Context::from_loader_function(|name| {
                    loader.get_proc_address(name) as *const _
                })
            });
            win.set_swap_interval(0);
            if fresh_context {
                owner.borrow_mut().gpu_renderer =
                    Some(unsafe { ctx.get_parameter_string(glow::RENDERER) });
            }
            gl.ctx = Some(ctx.clone());
            let result = unsafe { Renderer::new(ctx, prepared) };
            match result {
                Ok(renderer) => {
                    gl.renderer = Some(renderer);
                    gl.pending_scene = false;
                    win.set_context_valid(true);
                }
                Err(error) => {
                    owner.borrow_mut().error = Some(error);
                    drop(gl);
                    let (mut w, s, parent) = (win.clone(), Rc::downgrade(&owner), win.parent());
                    app::add_timeout3(0.0, move |_| {
                        if !w.was_deleted() {
                            w.hide();
                        }
                        if let Some(s) = s.upgrade() {
                            s.borrow_mut().dirty = true;
                        }
                        if let Some(mut p) = parent.clone() {
                            if !p.was_deleted() {
                                p.redraw();
                            }
                        }
                    });
                    return;
                }
            }
        }
        if let Some(renderer) = &gl.renderer {
            if let Some(ctx) = &gl.ctx {
                let bg = theme::with(|t| t.palette.background);
                unsafe {
                    ctx.disable(glow::SCISSOR_TEST);
                    ctx.clear_color(
                        bg.r() as f32 / 255.0,
                        bg.g() as f32 / 255.0,
                        bg.b() as f32 / 255.0,
                        1.0,
                    );
                    ctx.clear(glow::COLOR_BUFFER_BIT);
                }
            }
            let time = scene_time(&owner.borrow());
            unsafe {
                renderer.draw((win.w(), win.h()), (win.pixel_w(), win.pixel_h()), time);
            }
            let mut s = owner.borrow_mut();
            s.stats.gpu_frames += 1;
            s.stats.texture_bytes = renderer.texture_bytes();
        }
        let ctx = gl.ctx.clone();
        let mut results = Vec::new();
        if let Some(ctx) = ctx {
            for (x, y, f) in gl.readbacks.drain(..) {
                let px = ((x as f64 + 0.5) * win.pixel_w() as f64 / win.w().max(1) as f64).floor()
                    as i32;
                let py = win.pixel_h()
                    - 1
                    - ((y as f64 + 0.5) * win.pixel_h() as f64 / win.h().max(1) as f64).floor()
                        as i32;
                let mut value = [0; 4];
                unsafe {
                    ctx.read_pixels(
                        px,
                        py,
                        1,
                        1,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelPackData::Slice(Some(&mut value)),
                    );
                }
                results.push((f, value));
            }
        }
        drop(gl);
        // Animate the native child, not its CPU parent. Otherwise every
        // shader frame would also repaint AppWindow's full background.
        if win.visible_r() && win.shown() && anim::enabled() {
            let s = owner.borrow();
            if let Some(pace) = s.pace {
                if s.time.is_none()
                    && s.scene
                        .commands()
                        .iter()
                        .any(|c| matches!(c, super::super::Command::Shader { .. }))
                {
                    anim::request_redraw_at(win, pace);
                }
            }
        }
        for (f, value) in results {
            f(value);
        }
    }
}
