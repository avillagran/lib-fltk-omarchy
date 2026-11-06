//! Interactive composition acceptance case for the supplied lava video.
//! One surface, keyed model/panel, shared card resources and root previews.
use fltk::{
    enums::{Align, Event, Key, Shortcut},
    prelude::*,
};
use fltk_omarchy::{
    a11y::AccessibleExt,
    anim::{Ease, Pace},
    focus,
    gfx::{self, sl, Backend},
    i18n, keys,
    layout::Mirror,
    model::ListModel,
    paint,
    panel::KeyboardPanel,
    scene::{self, Layer, NodeId, NodeMotion, Rect, Scene, Surface, WeakSurface},
    state::Subscription,
    theme, tr, trn, AppWindow, State,
};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

const CURSOR: NodeId = NodeId(1000);
const SHADOW: NodeId = NodeId(1001);
const FALL_MS: u32 = 450;
fn row_node(key: u64) -> NodeId {
    NodeId(key * 2)
}
fn ghost_node(key: u64) -> NodeId {
    NodeId(key * 2 + 1)
}

#[derive(Clone, PartialEq)]
pub struct Card {
    pub key: u64,
    title: &'static str,
    tag: &'static str,
    color: usize,
}
fn cards() -> Vec<Card> {
    [
        ("Ship the release notes", "Engineering", 0),
        ("Review the layout cache", "Engineering", 0),
        ("Reply to the legal thread", "Operations", 1),
        ("Redesign the empty state", "Design", 2),
        ("Book flights for the offsite", "Personal", 3),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (title, tag, color))| Card {
        key: i as u64 + 1,
        title,
        tag,
        color,
    })
    .collect()
}
#[derive(Clone, Copy, PartialEq)]
struct Flight {
    key: u64,
    origin: usize,
    pos: (i32, i32),
    offset: (i32, i32),
    dragging: bool,
    keyboard: bool,
    deleting: bool,
}
#[derive(Clone, Default, PartialEq)]
struct Interaction {
    flight: Option<Flight>,
}
#[derive(Clone, Copy)]
struct Geometry {
    x: i32,
    width: i32,
    top: i32,
    pitch: i32,
    horizon: i32,
    height: i32,
}
impl Geometry {
    fn new(w: i32, h: i32) -> Self {
        let horizon = h - 86;
        Self {
            x: 40,
            width: (w - 80).max(120),
            top: 104,
            pitch: ((horizon - 40 - 104 - 64) / 4).clamp(68, 80),
            horizon,
            height: h,
        }
    }
    fn row(self, index: usize) -> (i32, i32) {
        (self.x, self.top + self.pitch * index as i32)
    }
    fn slot(self, y: i32, count: usize) -> usize {
        ((y - self.top + self.pitch / 2) / self.pitch).clamp(0, count.saturating_sub(1) as i32)
            as usize
    }
}
#[derive(Default)]
struct Resources {
    key: Option<(i32, i32, theme::Theme, String)>,
    cards: HashMap<u64, Layer>,
    shadow: Option<Layer>,
    cursor: Option<Layer>,
    static_lava: Option<Layer>,
}
pub struct Lava {
    pub win: AppWindow,
    pub surface: Surface,
    pub core: Rc<Core>,
}
pub struct Core {
    pub model: ListModel<u64, Card>,
    pub panel: KeyboardPanel,
    interaction: State<Interaction>,
    view: WeakSurface,
    cache: RefCell<Resources>,
    subscriptions: RefCell<Vec<Subscription>>,
    shader: sl::Shader,
    static_shader: sl::Shader,
    want_gpu: bool,
    me: RefCell<Weak<Core>>,
}
impl Lava {
    pub fn new(cpu: bool) -> Self {
        i18n::load(include_str!("lava.json")).expect("valid lava catalog");
        let mut win = AppWindow::new(&tr!("Lava tasks"), 720, 680);
        win.bind_title(&i18n::text("Lava tasks"));
        win.size_range(560, 620, 0, 0);
        let mut body = win.body();
        body.set_margin(0);
        let want_gpu = cfg!(feature = "gpu") && !cpu;
        let surface = Surface::new(720, 639)
            .backend(if want_gpu { Backend::Gpu } else { Backend::Cpu })
            .accessible_name(&tr!("Tasks"));
        body.add(&*surface);
        let model = ListModel::new(|c: &Card| c.key);
        for c in cards() {
            model.push(c);
        }
        let panel = KeyboardPanel::new(&tr!("Tasks"));
        model.bind_panel(&panel);
        panel.set_page_rows(5);
        let m = model.clone();
        panel.accessible_labels(move |id| m.with_id(id, |c| tr!(c.title)).unwrap_or_default());
        focus::host(&*surface, &[&panel]);
        let core = Rc::new(Core {
            model,
            panel,
            interaction: State::new(Interaction::default()),
            view: surface.downgrade(),
            cache: RefCell::new(Resources::default()),
            subscriptions: RefCell::new(Vec::new()),
            shader: lava_shader(true),
            static_shader: lava_shader(false),
            want_gpu,
            me: RefCell::new(Weak::new()),
        });
        *core.me.borrow_mut() = Rc::downgrade(&core);
        core.install_painter(&surface);
        let c = core.clone();
        let surface = surface.on_event_with(move |v, e, x, y| c.event(v, e, x, y));
        let weak = Rc::downgrade(&core);
        core.panel.on_change(move || {
            if let Some(c) = weak.upgrade() {
                c.cursor();
            }
        });
        let weak = Rc::downgrade(&core);
        core.panel.on_focus(move |_| {
            if let Some(c) = weak.upgrade() {
                c.cursor();
            }
        });
        let weak = Rc::downgrade(&core);
        core.panel
            .on_key(move |k, m| weak.upgrade().is_some_and(|c| c.key(k, m)));
        let weak = Rc::downgrade(&core);
        core.subscriptions
            .borrow_mut()
            .push(core.model.subscribe(move |_, change| {
                if !change.inserted().is_empty() || !change.removed().is_empty() {
                    if let Some(c) = weak.upgrade() {
                        if let Some(v) = c.view.upgrade() {
                            v.invalidate();
                        }
                    }
                }
            }));
        let c = core.clone();
        win.shortcut("Mod+R", move || c.reset());
        win.help_overlay(&tr!("Keyboard shortcuts"), || {
            vec![(
                tr!("Tasks"),
                vec![
                    (tr!("Up / Down"), tr!("Move the cursor")),
                    (tr!("Space"), tr!("Pick up or drop a task")),
                    (tr!("Enter / Delete"), tr!("Drop the task into the lava")),
                    (tr!("Esc"), tr!("Cancel the drag")),
                    ("Mod+Shift+Up / Down".into(), tr!("Reorder the task")),
                    ("Mod+R".into(), tr!("Reset the example")),
                ],
            )]
        });
        core.panel.set_cursor(core.model.id_at(0));
        win.quit_key(|| false);
        win.end();
        Self { win, surface, core }
    }
}
impl Core {
    fn install_painter(self: &Rc<Self>, v: &Surface) {
        let c = self.clone();
        v.clone().paint(move |r, t| c.paint(r, t));
    }
    pub fn keys(&self) -> Vec<u64> {
        (0..self.model.len())
            .filter_map(|i| self.model.key_at(i))
            .collect()
    }
    pub fn flying(&self) -> bool {
        self.interaction.get().flight.is_some()
    }
    pub fn row_rect(&self, key: u64) -> Option<Rect> {
        let v = self.view.upgrade()?;
        let g = Geometry::new(v.w(), v.h());
        let i = self.model.index_of_key(&key)?;
        let (x, y) = g.row(i);
        Some(Rect::new(x, y, g.width, 64))
    }
    fn geometry(&self) -> Option<Geometry> {
        let v = self.view.upgrade()?;
        Some(Geometry::new(v.w(), v.h()))
    }
    fn paint(&self, r: Rect, t: &theme::Theme) {
        let g = Geometry::new(r.w, r.h);
        let locale = i18n::locale().get();
        let mut cache = self.cache.borrow_mut();
        let key = (g.width, r.h, t.clone(), locale);
        if cache.key.as_ref() != Some(&key) {
            *cache = Resources {
                key: Some(key),
                ..Resources::default()
            };
        }
        let p = &t.palette;
        let mirror = Mirror::new(i18n::is_rtl(), 0, r.w);
        paint::fill(0, 0, r.w, r.h, 0, p.darker_background);
        paint::text(
            &tr!("Today"),
            40,
            22,
            r.w - 80,
            38,
            28,
            true,
            p.foreground,
            mirror.align(Align::Left),
        );
        paint::text(
            &trn!("{count} tasks", self.model.len()),
            40,
            65,
            r.w - 80,
            24,
            12,
            false,
            p.dim,
            mirror.align(Align::Left),
        );
        let mut uniforms = gfx::Uniforms::from_theme(t, (r.w as f32, 180.0));
        uniforms.background = p.darker_background.into();
        uniforms.accent = p.yellow.into();
        uniforms.urgent = p.red.into();
        let lava = Rect::new(0, r.h - 180, r.w, 180);
        let gpu = self.want_gpu
            && self
                .view
                .upgrade()
                .is_some_and(|v| v.backend_error().is_none());
        if gpu {
            scene::shader(lava, &self.shader, &uniforms);
        } else {
            let layer = cache.static_lava.get_or_insert_with(|| {
                let (s, ()) = Scene::record(|| {
                    scene::shader(Rect::new(0, 0, r.w, 180), &self.static_shader, &uniforms);
                });
                Layer::new(Rect::new(0, 0, r.w, 180), s).unwrap()
            });
            layer.draw_at(0, r.h - 180, 1.0);
        }
        paint::text(
            &tr!("Drop here to remove"),
            40,
            g.horizon + 20,
            r.w - 80,
            28,
            12,
            false,
            p.yellow,
            Align::Center,
        );
        let flight = self.interaction.get().flight;
        cache.cards.retain(|key, _| self.model.contains_key(key));
        scene::clip(
            Rect::new(24, g.top - 12, r.w - 48, g.horizon - g.top - 12),
            || {
                for i in 0..self.model.len() {
                    let card = self.model.get(i).unwrap();
                    let layer = cache
                        .cards
                        .entry(card.key)
                        .or_insert_with(|| card_layer(&card, g.width, t));
                    let (x, y) = g.row(i);
                    layer
                        .draw_keyed(
                            row_node(card.key),
                            x,
                            y,
                            if flight.is_some_and(|f| f.key == card.key) {
                                0.0
                            } else {
                                0.95
                            },
                        )
                        .unwrap();
                }
            },
        );
        let shadow = cache.shadow.get_or_insert_with(|| shadow_layer(g.width, t));
        let pos = flight.map(|f| f.pos).unwrap_or((0, 0));
        shadow
            .draw_keyed(
                SHADOW,
                pos.0 - 24,
                pos.1 - 18,
                if flight.is_some_and(|f| !f.deleting) {
                    1.0
                } else {
                    0.0
                },
            )
            .unwrap();
        for i in 0..self.model.len() {
            let card = self.model.get(i).unwrap();
            let layer = cache.cards.get(&card.key).unwrap();
            let f = flight.filter(|f| f.key == card.key);
            let pos = f.map(|f| f.pos).unwrap_or(g.row(i));
            layer
                .draw_keyed(
                    ghost_node(card.key),
                    pos.0,
                    pos.1,
                    if f.is_some() { 1.0 } else { 0.0 },
                )
                .unwrap();
        }
        let cursor = cache.cursor.get_or_insert_with(|| {
            let (s, ()) = Scene::record(|| paint::stroke(0, 0, g.width, 64, t.radius, 2, p.accent));
            Layer::new(Rect::new(0, 0, g.width, 64), s).unwrap()
        });
        let selected = self
            .panel
            .cursor()
            .and_then(|id| self.model.key_of(id))
            .and_then(|k| self.model.index_of_key(&k));
        let pos = selected.map(|i| g.row(i)).unwrap_or((0, 0));
        cursor
            .draw_keyed(
                CURSOR,
                pos.0,
                pos.1,
                if flight.is_none() && self.panel.show_ring() && selected.is_some() {
                    1.0
                } else {
                    0.0
                },
            )
            .unwrap();
    }
    fn pulse(&self, v: &Surface) {
        if v.active_backend() == Backend::Gpu {
            v.play_shader(Pace::Display, FALL_MS);
        }
    }
    fn cursor(&self) {
        let Some(v) = self.view.upgrade() else { return };
        let state = self.interaction.get();
        let show =
            state.flight.is_none() && self.panel.show_ring() && self.panel.cursor().is_some();
        if let Some(key) = self.panel.cursor().and_then(|id| self.model.key_of(id)) {
            if let Some(rect) = self.row_rect(key) {
                let _ = v.move_node(CURSOR, rect.x, rect.y);
            }
        }
        let _ = v.set_node_opacity(CURSOR, if show { 1.0 } else { 0.0 });
    }
    fn begin(&self, v: &Surface, key: u64, x: i32, y: i32, keyboard: bool) -> bool {
        if self.flying() {
            return false;
        }
        let Some(origin) = self.model.index_of_key(&key) else {
            return false;
        };
        let Ok(rect) = v.node_bounds(row_node(key)) else {
            return false;
        };
        if let Some(id) = self.model.id_of(&key) {
            self.panel.select_only(id);
        }
        self.interaction.set(Interaction {
            flight: Some(Flight {
                key,
                origin,
                pos: (rect.x, rect.y),
                offset: (x - rect.x, y - rect.y),
                dragging: true,
                keyboard,
                deleting: false,
            }),
        });
        let _ = v.cancel_node_motion(row_node(key));
        let _ = v.move_node(ghost_node(key), rect.x, rect.y);
        let _ = v.set_node_opacity(row_node(key), 0.0);
        let _ = v.set_node_opacity(ghost_node(key), 1.0);
        let _ = v.move_node(SHADOW, rect.x - 24, rect.y - 18);
        let _ = v.set_node_opacity(SHADOW, 1.0);
        self.cursor();
        self.pulse(v);
        true
    }
    fn reflow(&self, v: &Surface) {
        let Some(g) = self.geometry() else { return };
        let flight = self.interaction.get().flight;
        for i in 0..self.model.len() {
            let key = self.model.key_at(i).unwrap();
            if flight.is_some_and(|f| f.key == key) {
                continue;
            }
            let (x, y) = g.row(i);
            let _ = v.animate_node(row_node(key), NodeMotion::new().position(x, y));
        }
        self.cursor();
    }
    fn drag(&self, v: &Surface, x: i32, y: i32) {
        let Some(mut f) = self.interaction.get().flight.filter(|f| f.dragging) else {
            return;
        };
        let Some(g) = self.geometry() else { return };
        f.pos = (x - f.offset.0, y - f.offset.1);
        self.interaction.set(Interaction { flight: Some(f) });
        let _ = v.move_node(ghost_node(f.key), f.pos.0, f.pos.1);
        let _ = v.move_node(SHADOW, f.pos.0 - 24, f.pos.1 - 18);
        if y < g.horizon {
            let slot = g.slot(f.pos.1, self.model.len());
            if self.model.index_of_key(&f.key) != Some(slot) {
                self.model.move_key(&f.key, slot);
                self.reflow(v);
            }
        }
    }
    fn finish(&self, v: &Surface, remove: bool) {
        let Some(f) = self.interaction.get().flight else {
            return;
        };
        self.interaction.set(Interaction::default());
        let _ = v.set_node_opacity(ghost_node(f.key), 0.0);
        let _ = v.set_node_opacity(SHADOW, 0.0);
        if remove {
            let index = self.model.index_of_key(&f.key).unwrap_or(0);
            self.model.remove_key(&f.key);
            if self.panel.cursor().is_none() {
                if let Some(id) = self
                    .model
                    .id_at(index.min(self.model.len().saturating_sub(1)))
                {
                    self.panel.select_only(id);
                }
            }
            self.reflow(v);
            v.invalidate();
        } else {
            if let Some(rect) = self.row_rect(f.key) {
                let _ = v.move_node(row_node(f.key), rect.x, rect.y);
                let _ = v.set_node_opacity(row_node(f.key), 0.95);
            }
            self.cursor();
        }
    }
    fn drop_card(&self, v: &Surface, remove: bool, cancel: bool) {
        let Some(mut f) = self.interaction.get().flight.filter(|f| f.dragging) else {
            return;
        };
        let Some(g) = self.geometry() else { return };
        if cancel {
            self.model.move_key(&f.key, f.origin);
        }
        let destination = if remove {
            (f.pos.0, g.height + 30)
        } else {
            g.row(self.model.index_of_key(&f.key).unwrap_or(0))
        };
        f.dragging = false;
        f.deleting = remove;
        f.pos = destination;
        self.interaction.set(Interaction { flight: Some(f) });
        self.reflow(v);
        let weak = self.me.borrow().clone();
        let motion = NodeMotion::new()
            .position(destination.0, destination.1)
            .opacity(if remove { 0.0 } else { 0.95 })
            .duration_ms(if remove { FALL_MS } else { 190 })
            .ease(if remove {
                Ease::InCubic
            } else {
                Ease::OutCubic
            })
            .on_done(move |view, _| {
                if let Some(c) = weak.upgrade() {
                    c.finish(view, remove);
                }
            });
        let _ = v.animate_node(ghost_node(f.key), motion);
        let _ = v.animate_node(
            SHADOW,
            NodeMotion::new()
                .position(destination.0 - 24, destination.1 - 18)
                .opacity(0.0),
        );
        if remove {
            self.pulse(v);
        }
    }
    fn delete(&self, v: &Surface, key: u64) {
        if !self.flying() {
            if let Some(r) = self.row_rect(key) {
                self.begin(v, key, r.x + r.w / 2, r.y + 32, true);
            }
        }
        self.drop_card(v, true, false);
    }
    /// Exercises the same controller without taking native keyboard focus.
    /// Only the diagnostic driver calls this; real input uses `event`/`key`.
    pub fn benchmark_cycle(&self, remove: bool) {
        let Some(v) = self.view.upgrade() else { return };
        let Some(key) = self.model.key_at(0) else {
            return;
        };
        let Some(rect) = self.row_rect(key) else {
            return;
        };
        let (x, y) = (rect.x + rect.w / 2, rect.y + 32);
        if self.begin(&v, key, x, y, false) {
            if !remove {
                let last = self
                    .row_rect(self.model.key_at(self.model.len() - 1).unwrap())
                    .unwrap();
                self.drag(&v, x, last.y + 32);
            }
            self.drop_card(&v, remove, false);
        }
    }
    fn event(&self, v: &Surface, e: Event, x: i32, y: i32) -> bool {
        match e {
            Event::Push => {
                let Ok(Some(id)) = v.hit_node(x, y) else {
                    return false;
                };
                if id.0 >= 1000 || id.0 % 2 != 0 {
                    return false;
                }
                let key = id.0 / 2;
                if let Some(rect) = self.row_rect(key) {
                    if Mirror::new(i18n::is_rtl(), rect.x, rect.w).point(x) < rect.x + 44 {
                        self.delete(v, key);
                        return true;
                    }
                }
                self.begin(v, key, x, y, false)
            }
            Event::Drag => {
                if self
                    .interaction
                    .get()
                    .flight
                    .is_some_and(|f| f.dragging && !f.keyboard)
                {
                    self.drag(v, x, y);
                    true
                } else {
                    false
                }
            }
            Event::Released
                if self
                    .interaction
                    .get()
                    .flight
                    .is_some_and(|f| f.dragging && !f.keyboard) =>
            {
                self.drop_card(v, self.geometry().is_some_and(|g| y >= g.horizon), false);
                true
            }
            _ => false,
        }
    }
    fn key(&self, key: Key, mods: Shortcut) -> bool {
        let Some(v) = self.view.upgrade() else {
            return false;
        };
        let f = self.interaction.get().flight;
        if key == Key::Escape && f.is_some_and(|f| f.dragging) {
            self.drop_card(&v, false, true);
            return true;
        }
        if key == Key::from_char(' ') && mods.is_empty() {
            if f.is_some_and(|f| f.dragging) {
                self.drop_card(&v, false, false);
            } else if let Some(raw) = self.panel.cursor().and_then(|id| self.model.key_of(id)) {
                if let Some(r) = self.row_rect(raw) {
                    self.begin(&v, raw, r.x + r.w / 2, r.y + 32, true);
                }
            }
            return true;
        }
        if matches!(key, Key::Enter | Key::Delete | Key::BackSpace) && mods.is_empty() {
            if let Some(raw) = self.panel.cursor().and_then(|id| self.model.key_of(id)) {
                self.delete(&v, raw);
            }
            return true;
        }
        let reorder = mods.contains(keys::PRIMARY) && mods.contains(Shortcut::Shift);
        if matches!(key, Key::Up | Key::Down)
            && (reorder || f.is_some_and(|f| f.dragging && f.keyboard))
        {
            let Some(raw) = self.panel.cursor().and_then(|id| self.model.key_of(id)) else {
                return true;
            };
            if f.is_none() {
                let r = self.row_rect(raw).unwrap();
                self.begin(&v, raw, r.x + r.w / 2, r.y + 32, true);
            }
            if self.interaction.get().flight.is_some_and(|f| f.dragging) {
                let i = self.model.index_of_key(&raw).unwrap();
                let to = if key == Key::Up {
                    i.saturating_sub(1)
                } else {
                    (i + 1).min(self.model.len() - 1)
                };
                self.model.move_key(&raw, to);
                let (x, y) = self.geometry().unwrap().row(to);
                let offset = self.interaction.get().flight.unwrap().offset;
                self.drag(&v, x + offset.0, y + offset.1);
                self.reflow(&v);
                if reorder {
                    self.drop_card(&v, false, false);
                }
            }
            return true;
        }
        false
    }
    pub fn reset(self: &Rc<Self>) {
        let Some(v) = self.view.upgrade() else { return };
        self.interaction.set(Interaction::default());
        self.model.transaction(|m| {
            m.clear();
            for c in cards() {
                m.push(c);
            }
        });
        v.clone().animate(None);
        self.install_painter(&v);
        self.panel.set_cursor(self.model.id_at(0));
    }
}
fn card_layer(card: &Card, w: i32, t: &theme::Theme) -> Layer {
    let p = &t.palette;
    let m = Mirror::new(i18n::is_rtl(), 0, w);
    let color = [p.blue, p.yellow, p.magenta, p.green][card.color];
    let (s, ()) = Scene::record(|| {
        paint::surface(
            0,
            0,
            w,
            64,
            t.radius,
            Some(p.lighter_background),
            Some((p.foreground.over(p.lighter_background, 0.12), 1)),
        );
        paint::stroke(m.x(18, 14), 25, 14, 14, 7, 1, p.dim);
        scene::clip(
            Rect::new(m.x(52, (w - 215).max(1)), 6, (w - 215).max(1), 52),
            || {
                paint::text(
                    &tr!(card.title),
                    m.x(52, w - 215),
                    8,
                    w - 215,
                    24,
                    14,
                    true,
                    p.foreground,
                    m.align(Align::Left),
                );
                paint::text(
                    &tr!("Ready for review"),
                    m.x(52, w - 215),
                    35,
                    w - 215,
                    18,
                    11,
                    false,
                    p.dim,
                    m.align(Align::Left),
                );
            },
        );
        paint::fill(
            m.x(w - 143, 125),
            21,
            125,
            23,
            t.radius,
            color.over(p.lighter_background, 0.16),
        );
        paint::text(
            &tr!(card.tag),
            m.x(w - 143, 125),
            21,
            125,
            23,
            10,
            true,
            color,
            Align::Center,
        );
    });
    Layer::new(Rect::new(0, 0, w, 64), s).unwrap()
}
fn shadow_layer(w: i32, t: &theme::Theme) -> Layer {
    let mut canvas = gfx::Canvas::new((w + 48) as usize, 112);
    let path = gfx::Path::rounded_rect(24.0, 18.0, w as f32, 64.0, [t.radius as f32; 4]);
    let p = &t.palette;
    let dark = if p.background.r() as u32 + p.background.g() as u32 + p.background.b() as u32 > 384
    {
        p.foreground
    } else {
        p.background
    };
    canvas.shadow(&path, 0.0, 8.0, 8, gfx::Color::from(dark).with_alpha(0.65));
    let mut rgba = Vec::new();
    canvas.to_rgba8(&mut rgba);
    let image = scene::Image::rgba((w + 48) as u32, 112, rgba).unwrap();
    let (s, ()) = Scene::record(|| image.draw(Rect::new(0, 0, w + 48, 112)));
    Layer::new(Rect::new(0, 0, w + 48, 112), s).unwrap()
}
fn lava_shader(effects: bool) -> sl::Shader {
    sl::Shader::new(move |p, u| {
        let q = p / u.size();
        let y = q.y();
        let x = q.x();
        let ridge = 0.55 + sl::sin(x.clone() * 24.0) * 0.012;
        let glow = sl::exp(-sl::abs(y.clone() - ridge.clone()) * 18.0);
        let floor = sl::step(ridge.clone(), y.clone()) * 0.2;
        let mut heat = glow.clone() * 0.32 + floor;
        if effects {
            let t = u.time();
            let pulse = sl::sin(
                sl::clamp(t.clone() / (FALL_MS as f32 / 1000.0), 0.0, 1.0) * std::f32::consts::PI,
            );
            let noise = sl::fbm(
                sl::vec2(x.clone() * 12.0, y.clone() * 4.0 - t.clone() * 5.0),
                2,
            );
            let tongue = sl::max(
                noise.clone() - 0.35 - (ridge.clone() - y.clone()) * 2.8,
                0.0.into(),
            ) * (1.0 - sl::step(ridge, y.clone()));
            let sparks = sl::smoothstep(
                0.985,
                1.0,
                sl::noise(sl::vec2(x * 60.0, y * 24.0 - t * 8.0)),
            );
            heat = heat + pulse * (tongue * 2.0 + glow * 0.7 + sparks * 0.6);
        }
        let heat = sl::clamp(heat, 0.0, 1.0);
        sl::mix(
            sl::mix(u.background(), u.urgent(), heat.clone()),
            u.accent(),
            sl::smoothstep(0.45, 1.0, heat),
        )
    })
}
