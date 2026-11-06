//! `FrameView`: shows frames another process draws and sends it input.

use super::event::{Button, Input, Message, Mods};
use super::file;
use super::shm::{now_ns, Format};
use super::surface::{Consumer, Handle, Options, Watch};
use crate::a11y::{self, Role};
use crate::platform::frame::{Interrupter, Wait, POLL};
use crate::theme;
use fltk::{app, draw, enums::*, image::RgbImage, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How a frame fills the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Scaled to fit, aspect kept, bars around (the default). A frame of
    /// the size the view asked for shows 1:1.
    #[default]
    Contain,
    /// One frame pixel per device pixel, centred when smaller than the
    /// view, cropped at the right and bottom when larger.
    Actual,
}

/// What the view is for screen readers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrameRole {
    /// A page or document (browser, HTML, previews). The default.
    #[default]
    Document,
    /// A picture (device screen, diagram).
    Image,
}

/// Counters for benchmarks and diagnostics ([`FrameView::stats`]).
#[derive(Debug, Clone, Copy, Default)]
pub struct FrameStats {
    /// Frames taken from the surface (or decoded files).
    pub received: u64,
    /// Frames the producer presented but the view never took (it was
    /// slower than the producer; the newest frame always wins).
    pub skipped: u64,
    /// Draws that put a new frame on screen.
    pub drawn: u64,
    /// Present → drawn of the last frame (frame files: file written →
    /// drawn, so the producer's encoding is not included).
    pub last_latency: Option<Duration>,
    /// Mean and maximum present → drawn.
    pub mean_latency: Option<Duration>,
    pub max_latency: Option<Duration>,
    /// Time spent in the last draw (conversion, scaling, blit).
    pub last_draw: Duration,
    /// Size of the frame shown, in pixels.
    pub frame_size: (u32, u32),
    /// The last frame was blitted straight from shared memory (RGBA,
    /// opaque, no conversion on our side).
    pub zero_copy: bool,
    /// Input events sent.
    pub events: u64,
}

type FileInbox = Arc<Mutex<Option<(u64, Result<file::Decoded, String>)>>>;
type InputCb = Rc<dyn Fn(&Input)>;
type MessageCb = Rc<dyn Fn(&Message)>;
/// Where the frame was drawn (logical rectangle) and its pixel size.
type Placed = ((i32, i32, i32, i32), (u32, u32));

/// What is being shown.
enum Source {
    None,
    Shm { consumer: Consumer, stop: Arc<AtomicBool>, interrupt: Option<Interrupter> },
    Files { stop: Arc<AtomicBool> },
}

/// The image of the current frame, ready to blit.
struct Shown {
    /// Shared memory frame seq, or a counter for files.
    seq: u64,
    presented_ns: u64,
    size: (u32, u32),
    /// Converted pixels (BGRA, translucent, files); empty when the image
    /// borrows the shared slot.
    owned: Vec<u8>,
    image: Option<RgbImage>,
    counted: bool,
}

struct St {
    source: Source,
    shown: Option<Shown>,
    /// A decoded file waiting for the UI thread.
    file_inbox: FileInbox,
    file_seq: u64,
    /// Request number of the newest file asked for / shown.
    file_req: u64,
    file_shown_req: u64,
    file_sink: Option<crate::task::SinkId>,
    /// Paths for the decoding worker of `show_file` (newest wins).
    file_queue: Option<Arc<FileQueue>>,
    fit: Fit,
    name: String,
    role: FrameRole,
    zoom: f32,
    pass_escape: bool,
    owns_wheel: bool,
    reserve: Option<Rc<dyn Fn(Key, Mods) -> bool>>,
    on_event_dyn: Option<InputCb>,
    writer: Option<Box<dyn Write>>,
    on_message: Option<MessageCb>,
    on_content_size: Option<Rc<dyn Fn(u32, u32)>>,
    on_closed: Option<Rc<dyn Fn()>>,
    closed_told: bool,
    move_interval: Duration,
    last_move: Option<Instant>,
    pending_move: Option<Input>,
    move_timer: bool,
    resize_debounce: Duration,
    reported: Option<(i32, i32, f32)>,
    /// The size a debounce timer is waiting to report.
    resize_pending: Option<(i32, i32, f32)>,
    resize_gen: u64,
    copy_armed: Option<Instant>,
    buttons: u8,
    /// Where the frame was drawn last (logical) and its pixel size.
    placed: Option<Placed>,
    stats: FrameStats,
    latency_sum: Duration,
    latency_n: u64,
}

/// The watcher thread stops when the view goes away.
impl Drop for St {
    fn drop(&mut self) {
        stop_source(&mut self.source);
        if let Some(q) = &self.file_queue {
            q.stop.store(true, Ordering::Relaxed);
            q.cv.notify_all();
        }
    }
}

/// The newest path `show_file` asked for, for one decoding worker.
struct FileQueue {
    next: Mutex<Option<(u64, PathBuf)>>,
    cv: std::sync::Condvar,
    stop: AtomicBool,
}

/// Puts a decoded file in the inbox unless a newer one is there already.
fn deliver(inbox: &Mutex<Option<(u64, Result<file::Decoded, String>)>>, req: u64, d: Result<file::Decoded, String>) {
    let mut g = inbox.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().map_or(true, |(r, _)| *r < req) {
        *g = Some((req, d));
    }
}

fn stop_source(s: &mut Source) {
    match std::mem::replace(s, Source::None) {
        Source::Shm { stop, interrupt, consumer } => {
            stop.store(true, Ordering::Relaxed);
            if let Some(i) = interrupt {
                i.interrupt();
            }
            drop(consumer);
        }
        Source::Files { stop } => stop.store(true, Ordering::Relaxed),
        Source::None => {}
    }
}

/// Shows frames that another process draws (a browser engine, an HTML or
/// QML renderer, a device screen) and sends it the user's input.
///
/// - **Transport**: shared memory with three buffers ([`super::Consumer`]),
///   woken by the producer (no polling); or PNG/BMP frame files
///   ([`FrameView::watch_file`], [`FrameView::show_file`]) for helpers
///   that still write files.
/// - **Display**: blits only when a new frame arrives, straight from
///   shared memory when the frame is opaque RGBA; [`Fit::Contain`] or
///   [`Fit::Actual`]; device pixels follow `native::pixels_per_unit`.
/// - **Input**: [`Input`] events (pointer in frame pixels, hover moves at
///   most every 40 ms, keys, focus, resize debounced 160 ms, zoom, copy)
///   go into the surface's ring, to [`FrameView::on_event`] and to
///   [`FrameView::events_to`] as JSON lines.
/// - **Keyboard**: one Tab stop. While focused every key goes to the
///   producer except Tab/Shift+Tab (leave the view), Esc
///   ([`FrameView::pass_escape`]) and keys the app [reserves](FrameView::reserve).
/// - **Screen readers**: a Document (or Image) with the app's
///   [name](FrameView::name).
#[derive(Clone)]
pub struct FrameView {
    inner: Widget,
    st: Rc<RefCell<St>>,
}
fltk::widget_extends!(FrameView, Widget, inner);

impl FrameView {
    /// An empty view of `w × h` FLTK units (fit it in a flex to stretch).
    pub fn new(w: i32, h: i32) -> FrameView {
        let mut inner = Widget::default();
        inner.set_frame(FrameType::NoBox);
        inner.set_size(w.max(1), h.max(1));
        inner.visible_focus(true);
        let st = Rc::new(RefCell::new(St {
            source: Source::None,
            shown: None,
            file_inbox: Arc::new(Mutex::new(None)),
            file_seq: 0,
            file_req: 0,
            file_shown_req: 0,
            file_sink: None,
            file_queue: None,
            fit: Fit::Contain,
            name: String::new(),
            role: FrameRole::Document,
            zoom: 1.0,
            pass_escape: false,
            owns_wheel: true,
            reserve: None,
            on_event_dyn: None,
            writer: None,
            on_message: None,
            on_content_size: None,
            on_closed: None,
            closed_told: false,
            move_interval: Duration::from_millis(40),
            last_move: None,
            pending_move: None,
            move_timer: false,
            resize_debounce: Duration::from_millis(160),
            reported: None,
            resize_pending: None,
            resize_gen: 0,
            copy_armed: None,
            buttons: 0,
            placed: None,
            stats: FrameStats::default(),
            latency_sum: Duration::ZERO,
            latency_n: 0,
        }));
        let me = FrameView { inner, st };
        me.install();
        me
    }

    // ------------------------------------------------------------ sources

    /// Creates a shared surface sized for this view (named objects) and
    /// returns its handle for a producer started elsewhere. For a child
    /// process prefer [`FrameView::spawn`] (inherited, unnamed memory).
    pub fn create_surface(&self) -> std::io::Result<Handle> {
        let (w, h, _) = self.device_size();
        let o = Options::new(w, h).transport(super::Transport::Named);
        let (handle, consumer) = Consumer::create_with(o)?;
        self.attach(consumer);
        Ok(handle)
    }

    /// Starts `cmd` as the producer: a surface sized for this view is
    /// passed to it (inherited descriptors and the `OMARCHY_FRAME`
    /// variable, read by [`super::Producer::from_env`]).
    pub fn spawn(&self, cmd: std::process::Command) -> std::io::Result<std::process::Child> {
        let (w, h, _) = self.device_size();
        self.spawn_with(cmd, Options::new(w, h))
    }

    /// [`FrameView::spawn`] with explicit surface options.
    pub fn spawn_with(&self, mut cmd: std::process::Command, o: Options) -> std::io::Result<std::process::Child> {
        let (_, mut consumer) = Consumer::create_with(o)?;
        consumer.share(&mut cmd);
        let child = cmd.spawn();
        consumer.release_remote();
        let child = child?;
        self.attach(consumer);
        Ok(child)
    }

    /// Opens a surface a producer created ([`super::Producer::create`]).
    pub fn open(&self, handle: &Handle) -> std::io::Result<()> {
        self.attach(Consumer::open(handle)?);
        Ok(())
    }

    /// Shows the frames of `consumer` from now on (replaces any source).
    pub fn attach(&self, mut consumer: Consumer) {
        let watch = consumer.take_watch();
        let stop = Arc::new(AtomicBool::new(false));
        let interrupt = watch.waiter.as_ref().map(|w| w.interrupter());
        {
            let mut s = self.st.borrow_mut();
            stop_source(&mut s.source);
            s.shown = None;
            s.reported = None;
            s.closed_told = false;
            s.source = Source::Shm { consumer, stop: stop.clone(), interrupt };
        }
        let pending = Arc::new(AtomicBool::new(false));
        let sink = self.shm_sink(pending.clone());
        start_watch(watch, stop, pending, sink);
        // The producer learns the size now, not after the debounce.
        self.report_size(true);
        self.inner.clone().redraw();
    }

    /// Stops showing frames (the last one stays until another source).
    pub fn detach(&self) {
        stop_source(&mut self.st.borrow_mut().source);
    }

    /// Shows the PNG/BMP file at `path` (decoded on a worker): call it
    /// each time a helper announces a new frame file. PNG needs the
    /// `images` feature.
    pub fn show_file(&self, path: impl Into<PathBuf>) {
        let sink = self.file_sink();
        let mut s = self.st.borrow_mut();
        if matches!(s.source, Source::Shm { .. }) {
            stop_source(&mut s.source);
        }
        s.file_req += 1;
        let req = s.file_req;
        let q = match &s.file_queue {
            Some(q) => q.clone(),
            None => {
                let q = Arc::new(FileQueue { next: Mutex::new(None), cv: std::sync::Condvar::new(), stop: AtomicBool::new(false) });
                s.file_queue = Some(q.clone());
                let (q2, inbox) = (q.clone(), s.file_inbox.clone());
                std::thread::spawn(move || loop {
                    let job = {
                        let mut g = q2.next.lock().unwrap_or_else(|e| e.into_inner());
                        while g.is_none() && !q2.stop.load(Ordering::Relaxed) {
                            g = q2.cv.wait(g).unwrap_or_else(|e| e.into_inner());
                        }
                        if q2.stop.load(Ordering::Relaxed) {
                            return;
                        }
                        g.take()
                    };
                    if let Some((req, path)) = job {
                        deliver(&inbox, req, file::load(&path));
                        sink.wake();
                    }
                });
                q
            }
        };
        *q.next.lock().unwrap_or_else(|e| e.into_inner()) = Some((req, path.into()));
        q.cv.notify_one();
    }

    /// Watches a frame file and shows each new version (polled every
    /// `interval`, at least 8 ms; o3-code's helpers replace the file by
    /// rename). Replaces any source.
    pub fn watch_file(&self, path: impl Into<PathBuf>, interval: Duration) {
        let path = path.into();
        let stop = Arc::new(AtomicBool::new(false));
        let inbox = {
            let mut s = self.st.borrow_mut();
            stop_source(&mut s.source);
            s.source = Source::Files { stop: stop.clone() };
            s.file_inbox.clone()
        };
        let sink = self.file_sink();
        let interval = interval.max(POLL);
        let base = {
            let mut s = self.st.borrow_mut();
            // Leave room above any show_file request numbers.
            s.file_req += 1 << 32;
            s.file_req
        };
        std::thread::spawn(move || {
            let n = std::cell::Cell::new(base);
            file::watch(path, interval, stop, &|d| {
                n.set(n.get() + 1);
                deliver(&inbox, n.get(), d);
                sink.wake();
            });
        });
    }

    fn file_sink(&self) -> crate::task::SinkId {
        if let Some(id) = self.st.borrow().file_sink.clone() {
            return id;
        }
        let weak = Rc::downgrade(&self.st);
        let w = self.inner.clone();
        let id = crate::task::register_sink(move || {
            let Some(st) = weak.upgrade() else { return false };
            if w.was_deleted() {
                return false;
            }
            let got = st.borrow().file_inbox.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some((req, Ok(d))) = got {
                let mut s = st.borrow_mut();
                if req <= s.file_shown_req {
                    return true; // an older decode finished late
                }
                s.file_shown_req = req;
                s.file_seq += 1;
                let seq = s.file_seq;
                s.stats.received += 1;
                let presented_ns = if d.written_ns > 0 { d.written_ns } else { now_ns() };
                s.shown = Some(Shown { seq, presented_ns, size: (d.width, d.height), owned: d.rgba, image: None, counted: false });
                drop(s);
                w.clone().redraw();
            }
            true
        });
        self.st.borrow_mut().file_sink = Some(id.clone());
        id
    }

    fn shm_sink(&self, pending: Arc<AtomicBool>) -> crate::task::SinkId {
        let weak = Rc::downgrade(&self.st);
        let w = self.inner.clone();
        crate::task::register_sink(move || {
            let Some(st) = weak.upgrade() else { return false };
            if w.was_deleted() {
                return false;
            }
            pending.store(false, Ordering::Release);
            let (msgs, new, gone) = {
                let mut s = st.borrow_mut();
                let s = &mut *s;
                let Source::Shm { consumer, .. } = &mut s.source else { return false };
                // The old image borrows the slot that acquire gives back.
                let next = consumer.acquire().map(|f| (f.seq, f.presented_ns, (f.width, f.height)));
                let new = if let Some((seq, presented_ns, size)) = next {
                    let skipped = s.shown.as_ref().map_or(seq - 1, |o| seq.saturating_sub(o.seq + 1));
                    s.stats.skipped += skipped;
                    s.stats.received += 1;
                    s.shown = Some(Shown { seq, presented_ns, size, owned: Vec::new(), image: None, counted: false });
                    true
                } else {
                    false
                };
                (consumer.messages(), new, consumer.producer_gone())
            };
            if new {
                w.clone().redraw();
            }
            for m in msgs {
                handle_message(&st, &m);
            }
            if gone {
                let cb = {
                    let mut s = st.borrow_mut();
                    if s.closed_told {
                        None
                    } else {
                        s.closed_told = true;
                        s.on_closed.clone()
                    }
                };
                if let Some(cb) = cb {
                    cb();
                }
            }
            true
        })
    }

    // ------------------------------------------------------------ options

    pub fn fit(self, fit: Fit) -> Self {
        self.st.borrow_mut().fit = fit;
        self.inner.clone().redraw();
        self
    }

    /// The name screen readers announce ("Browser", "Preview of x.html").
    pub fn name(self, name: &str) -> Self {
        self.st.borrow_mut().name = name.to_string();
        a11y::changed(&self.inner);
        self
    }

    pub fn role(self, role: FrameRole) -> Self {
        self.st.borrow_mut().role = role;
        a11y::changed(&self.inner);
        self
    }

    /// Esc goes to the producer too (default false: Esc stays with the
    /// app, e.g. to close the view).
    pub fn pass_escape(self, on: bool) -> Self {
        self.st.borrow_mut().pass_escape = on;
        self
    }

    /// Keys the app keeps while the view is focused (its shortcuts):
    /// `f(key, mods)` true means "not for the producer".
    pub fn reserve(self, f: impl Fn(Key, Mods) -> bool + 'static) -> Self {
        self.st.borrow_mut().reserve = Some(Rc::new(f));
        self
    }

    /// The wheel goes to the producer only while the view is focused
    /// (false), so an inline view inside a scrolled page lets the page
    /// scroll. Default true.
    pub fn owns_wheel(self, on: bool) -> Self {
        self.st.borrow_mut().owns_wheel = on;
        self
    }

    /// Minimum time between hover moves sent (default 40 ms). Moves with a
    /// button held are sent at once.
    pub fn move_interval(self, d: Duration) -> Self {
        self.st.borrow_mut().move_interval = d;
        self
    }

    /// Quiet time before a new size is reported (default 160 ms).
    pub fn resize_debounce(self, d: Duration) -> Self {
        self.st.borrow_mut().resize_debounce = d;
        self
    }

    /// Calls `f` with every input event (also for file sources, which
    /// have no ring: the app forwards them itself).
    pub fn on_event(self, f: impl Fn(&Input) + 'static) -> Self {
        self.st.borrow_mut().on_event_dyn = Some(Rc::new(f));
        self
    }

    /// Writes every input event as a JSON line to `w` (a helper's stdin
    /// or a socket).
    pub fn events_to(self, w: impl Write + 'static) -> Self {
        self.st.borrow_mut().writer = Some(Box::new(w));
        self
    }

    /// Calls `f` with every message from the producer.
    pub fn on_message(self, f: impl Fn(&Message) + 'static) -> Self {
        self.st.borrow_mut().on_message = Some(Rc::new(f));
        self
    }

    /// Calls `f(width, height)` when the producer reports its content
    /// size (logical pixels): an inline view can resize to it.
    pub fn on_content_size(self, f: impl Fn(u32, u32) + 'static) -> Self {
        self.st.borrow_mut().on_content_size = Some(Rc::new(f));
        self
    }

    /// Calls `f` once when the producer goes away.
    pub fn on_closed(self, f: impl Fn() + 'static) -> Self {
        self.st.borrow_mut().on_closed = Some(Rc::new(f));
        self
    }

    // ------------------------------------------------------------ actions

    /// Sends an event as if the user made it.
    pub fn send(&self, e: &Input) {
        emit(&self.st, e.clone());
    }

    /// Sends any JSON message (the app's own commands) to a shared-memory
    /// producer and to [`FrameView::events_to`].
    pub fn send_json(&self, v: &serde_json::Value) {
        let mut s = self.st.borrow_mut();
        if let Source::Shm { consumer, .. } = &mut s.source {
            consumer.send_json(v);
        }
        if let Some(w) = s.writer.as_mut() {
            let _ = writeln!(w, "{v}");
            let _ = w.flush();
        }
    }

    /// Current zoom (1 = 100 %).
    pub fn zoom(&self) -> f32 {
        self.st.borrow().zoom
    }

    /// Asks the producer to render at `z` (clamped to 0.5–2).
    pub fn set_zoom(&self, z: f32) {
        let z = (z.clamp(0.5, 2.0) * 100.0).round() / 100.0;
        if (self.st.borrow().zoom - z).abs() < f32::EPSILON {
            return;
        }
        self.st.borrow_mut().zoom = z;
        emit(&self.st, Input::Zoom { value: z });
    }

    pub fn zoom_in(&self) {
        self.set_zoom(self.zoom() + 0.25);
    }
    pub fn zoom_out(&self) {
        self.set_zoom(self.zoom() - 0.25);
    }
    pub fn reset_zoom(&self) {
        self.set_zoom(1.0);
    }

    pub fn stats(&self) -> FrameStats {
        self.st.borrow().stats
    }

    /// Size of the frame shown, if any.
    pub fn frame_size(&self) -> Option<(u32, u32)> {
        self.st.borrow().shown.as_ref().map(|s| s.size)
    }

    /// The view's size in device pixels and its scale.
    fn device_size(&self) -> (u32, u32, f32) {
        let ppu = crate::native::pixels_per_unit(&self.inner);
        let w = (self.inner.w().max(1) as f32 * ppu).round() as u32;
        let h = (self.inner.h().max(1) as f32 * ppu).round() as u32;
        (w, h, ppu)
    }

    /// Reports the size now (`now`) or after the debounce when it changed.
    fn report_size(&self, now: bool) {
        let ppu = crate::native::pixels_per_unit(&self.inner);
        let cur = (self.inner.w(), self.inner.h(), ppu);
        if cur.0 <= 0 || cur.1 <= 0 {
            return;
        }
        let (gen, debounce) = {
            let mut s = self.st.borrow_mut();
            if s.reported == Some(cur) && !now {
                s.resize_pending = None;
                return;
            }
            if s.resize_pending == Some(cur) && !now {
                return; // a timer for this very size is running
            }
            s.resize_gen += 1;
            s.resize_pending = Some(cur);
            (s.resize_gen, s.resize_debounce)
        };
        if now || debounce.is_zero() {
            let mut s = self.st.borrow_mut();
            s.reported = Some(cur);
            s.resize_pending = None;
            drop(s);
            emit(&self.st, Input::Resize { width: cur.0 as u32, height: cur.1 as u32, scale: cur.2 });
            return;
        }
        let weak = Rc::downgrade(&self.st);
        let w = self.inner.clone();
        app::add_timeout3(debounce.as_secs_f64(), move |_| {
            let Some(st) = weak.upgrade() else { return };
            if w.was_deleted() || st.borrow().resize_gen != gen {
                return;
            }
            // Measure again: the size the view has now.
            let ppu = crate::native::pixels_per_unit(&w);
            let cur2 = (w.w(), w.h(), ppu);
            st.borrow_mut().resize_pending = None;
            if st.borrow().reported != Some(cur2) && cur2.0 > 0 && cur2.1 > 0 {
                st.borrow_mut().reported = Some(cur2);
                emit(&st, Input::Resize { width: cur2.0 as u32, height: cur2.1 as u32, scale: cur2.2 });
            }
        });
    }

    /// Widget coordinates (window-relative event position) to frame pixels.
    fn to_frame(&self, ex: i32, ey: i32) -> (f32, f32) {
        let s = self.st.borrow();
        let w = &self.inner;
        match s.placed {
            Some(((x, y, pw, ph), (fw, fh))) if pw > 0 && ph > 0 => {
                let fx = (ex - x) as f32 * fw as f32 / pw as f32;
                let fy = (ey - y) as f32 * fh as f32 / ph as f32;
                (fx.clamp(0.0, fw as f32), fy.clamp(0.0, fh as f32))
            }
            _ => {
                let ppu = crate::native::pixels_per_unit(w);
                (((ex - w.x()) as f32 * ppu).max(0.0), ((ey - w.y()) as f32 * ppu).max(0.0))
            }
        }
    }

    fn install(&self) {
        let me = self.clone();
        self.inner.clone().handle(move |w, ev| me.handle_event(w, ev));
        let me = self.clone();
        self.inner.clone().draw(move |w| me.draw(w));
        let weak = Rc::downgrade(&self.st);
        a11y::describe(&self.inner, move |n| {
            let Some(st) = weak.upgrade() else { return };
            let s = st.borrow();
            n.role(match s.role {
                FrameRole::Document => Role::Document,
                FrameRole::Image => Role::Image,
            })
            .name(&s.name)
            .focusable();
        });
    }

    fn handle_event(&self, w: &mut Widget, ev: Event) -> bool {
        match ev {
            Event::Focus => {
                emit(&self.st, Input::FocusIn);
                w.redraw();
                true
            }
            Event::Unfocus => {
                emit(&self.st, Input::FocusOut);
                w.redraw();
                true
            }
            Event::Enter => true,
            Event::Leave => {
                self.flush_move();
                emit(&self.st, Input::Leave);
                true
            }
            Event::Move => {
                let (x, y) = self.to_frame(app::event_x(), app::event_y());
                self.hover(Input::Move { x, y, buttons: 0, mods: mods() });
                true
            }
            Event::Push => {
                if !crate::focus::is_focused(w) {
                    let _ = w.take_focus();
                }
                let Some(button) = button() else { return true };
                self.flush_move();
                let buttons = {
                    let mut s = self.st.borrow_mut();
                    s.buttons |= button as u8;
                    s.buttons
                };
                let (x, y) = self.to_frame(app::event_x(), app::event_y());
                emit(&self.st, Input::Press { x, y, button, buttons, mods: mods() });
                if app::event_clicks() {
                    emit(&self.st, Input::Double { x, y, button, buttons, mods: mods() });
                }
                true
            }
            Event::Drag => {
                let (x, y) = self.to_frame(app::event_x(), app::event_y());
                let buttons = self.st.borrow().buttons;
                self.flush_move();
                emit(&self.st, Input::Move { x, y, buttons, mods: mods() });
                true
            }
            Event::Released => {
                let Some(button) = button() else { return true };
                let buttons = {
                    let mut s = self.st.borrow_mut();
                    s.buttons &= !(button as u8);
                    s.buttons
                };
                let (x, y) = self.to_frame(app::event_x(), app::event_y());
                emit(&self.st, Input::Release { x, y, button, buttons, mods: mods() });
                true
            }
            Event::MouseWheel => {
                if !self.st.borrow().owns_wheel && !crate::focus::is_focused(w) {
                    return false;
                }
                let (x, y) = self.to_frame(app::event_x(), app::event_y());
                let (dx, dy) = (app::event_dx_value() as f32, app::event_dy_value() as f32);
                if dx == 0.0 && dy == 0.0 {
                    return false;
                }
                emit(&self.st, Input::Wheel { x, y, dx, dy, mods: mods() });
                true
            }
            Event::KeyDown | Event::KeyUp => self.key(ev),
            _ => false,
        }
    }

    fn key(&self, ev: Event) -> bool {
        let k = app::event_key();
        let m = mods();
        if k == Key::Tab && !m.contains(Mods::CTRL) {
            return false; // the focus router moves on
        }
        let (pass_escape, reserve) = {
            let s = self.st.borrow();
            (s.pass_escape, s.reserve.clone())
        };
        if k == Key::Escape && !pass_escape {
            return false;
        }
        if reserve.is_some_and(|f| f(k, m)) {
            return false;
        }
        let name = key_name(k);
        if name.is_empty() {
            return false;
        }
        let text = app::event_text();
        if ev == Event::KeyDown {
            let copy = name.eq_ignore_ascii_case("c") && (m.contains(Mods::CTRL) || m.contains(Mods::META)) && !m.contains(Mods::ALT);
            emit(&self.st, Input::KeyDown { key: name, text, mods: m, repeat: crate::keys::is_repeat() });
            if copy {
                self.st.borrow_mut().copy_armed = Some(Instant::now());
                emit(&self.st, Input::Copy);
            }
        } else {
            emit(&self.st, Input::KeyUp { key: name, text, mods: m });
        }
        true
    }

    /// Hover move: at most one per `move_interval`, the newest wins.
    fn hover(&self, e: Input) {
        let (due, wait) = {
            let mut s = self.st.borrow_mut();
            let iv = s.move_interval;
            let since = s.last_move.map(|t| t.elapsed());
            if since.map_or(true, |d| d >= iv) {
                s.last_move = Some(Instant::now());
                s.pending_move = None;
                (Some(e), None)
            } else {
                s.pending_move = Some(e);
                if s.move_timer {
                    (None, None)
                } else {
                    s.move_timer = true;
                    (None, Some(iv - since.unwrap_or_default()))
                }
            }
        };
        if let Some(e) = due {
            emit(&self.st, e);
        }
        if let Some(d) = wait {
            let weak = Rc::downgrade(&self.st);
            app::add_timeout3(d.as_secs_f64(), move |_| {
                if let Some(st) = weak.upgrade() {
                    st.borrow_mut().move_timer = false;
                    flush_move_of(&st);
                }
            });
        }
    }

    /// Sends a held-back hover move now (before a press, a leave).
    fn flush_move(&self) {
        flush_move_of(&self.st);
    }

    fn draw(&self, w: &mut Widget) {
        let t0 = Instant::now();
        self.report_size(false);
        let (bg, ring_color, radius) = theme::with(|t| {
            let bg = t.palette.background;
            (bg, t.border(&t.controls.focus, bg), t.badge_radius())
        });
        let (x, y, ww, wh) = (w.x(), w.y(), w.w(), w.h());
        draw::push_clip(x, y, ww, wh);
        let ppu = crate::native::pixels_per_unit(w);
        let mut s = self.st.borrow_mut();
        let s = &mut *s;
        let mut placed = None;
        let mut drew_new = None;
        if let Some(shown) = s.shown.as_mut() {
            let (fw, fh) = shown.size;
            let dest = place(s.fit, (x, y, ww, wh), (fw, fh), ppu);
            // Bars around a letterboxed frame.
            if dest != (x, y, ww, wh) {
                draw::set_draw_color(bg.fl());
                draw::draw_rectf(x, y, ww, wh);
            }
            if shown.image.is_none() {
                let mut zero_copy = false;
                shown.image = match &s.source {
                    Source::Shm { consumer, .. } if shown.owned.is_empty() => consumer.frame().and_then(|f| {
                        if f.seq != shown.seq {
                            return None;
                        }
                        if f.format == Format::Rgba && f.opaque {
                            zero_copy = true;
                            // SAFETY: the slot stays ours (front) until the
                            // next acquire, which drops this image first.
                            unsafe { RgbImage::from_data_ext(f.pixels, f.width as i32, f.height as i32, 4, f.stride as i32).ok() }
                        } else {
                            shown.owned = to_straight_rgba(f.pixels, f.width, f.height, f.stride, f.format, f.opaque);
                            // SAFETY: `owned` lives in `shown` with the image
                            // and is not touched while the image exists.
                            unsafe { RgbImage::from_data(&shown.owned, f.width as i32, f.height as i32, ColorDepth::Rgba8).ok() }
                        }
                    }),
                    // SAFETY: as above.
                    _ => unsafe { RgbImage::from_data(&shown.owned, fw as i32, fh as i32, ColorDepth::Rgba8).ok() },
                };
                s.stats.zero_copy = zero_copy;
            }
            if let Some(img) = shown.image.as_mut() {
                img.scale(dest.2, dest.3, false, true);
                img.draw(dest.0, dest.1, dest.2, dest.3);
                placed = Some((dest, (fw, fh)));
                if !shown.counted {
                    shown.counted = true;
                    drew_new = Some((shown.seq, shown.presented_ns, (fw, fh)));
                }
            }
        } else {
            draw::set_draw_color(bg.fl());
            draw::draw_rectf(x, y, ww, wh);
        }
        s.placed = placed;
        if crate::focus::show_ring(w) {
            crate::paint::stroke(x, y, ww, wh, radius, 2, ring_color);
        }
        draw::pop_clip();
        if let Some((seq, presented, size)) = drew_new {
            let lat = Duration::from_nanos(now_ns().saturating_sub(presented));
            s.stats.drawn += 1;
            s.stats.frame_size = size;
            s.stats.last_latency = Some(lat);
            s.latency_sum += lat;
            s.latency_n += 1;
            s.stats.mean_latency = Some(s.latency_sum / s.latency_n.max(1) as u32);
            s.stats.max_latency = Some(s.stats.max_latency.map_or(lat, |m| m.max(lat)));
            if let Source::Shm { consumer, .. } = &s.source {
                consumer.mark_drawn(seq);
            }
        }
        s.stats.last_draw = t0.elapsed();
    }
}

/// Where a `fw × fh` pixel frame goes inside `area` (logical units).
fn place(fit: Fit, area: (i32, i32, i32, i32), frame: (u32, u32), ppu: f32) -> (i32, i32, i32, i32) {
    let (x, y, w, h) = area;
    let (fw, fh) = (frame.0.max(1) as f32, frame.1.max(1) as f32);
    let (dw, dh) = match fit {
        Fit::Contain => {
            let s = (w as f32 / fw).min(h as f32 / fh);
            // Within half a logical pixel of the area: fill it exactly.
            let (dw, dh) = ((fw * s).round() as i32, (fh * s).round() as i32);
            (if (dw - w).abs() <= 1 { w } else { dw }, if (dh - h).abs() <= 1 { h } else { dh })
        }
        Fit::Actual => ((fw / ppu).round() as i32, (fh / ppu).round() as i32),
    };
    let ox = if dw < w { (w - dw) / 2 } else { 0 };
    let oy = if dh < h { (h - dh) / 2 } else { 0 };
    (x + ox, y + oy, dw.max(1), dh.max(1))
}

/// Converts a shared frame to straight-alpha RGBA rows without padding.
fn to_straight_rgba(px: &[u8], w: u32, h: u32, stride: usize, format: Format, opaque: bool) -> Vec<u8> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let src = &px[y * stride..y * stride + w * 4];
        let dst = &mut out[y * w * 4..(y + 1) * w * 4];
        for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
            let (r, g, b, a) = match format {
                Format::Rgba => (s[0], s[1], s[2], s[3]),
                Format::Bgra => (s[2], s[1], s[0], s[3]),
            };
            if opaque || a == 255 {
                d.copy_from_slice(&[r, g, b, 255]);
            } else if a == 0 {
                d.copy_from_slice(&[0, 0, 0, 0]);
            } else {
                let un = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                d.copy_from_slice(&[un(r), un(g), un(b), a]);
            }
        }
    }
    out
}

fn handle_message(st: &Rc<RefCell<St>>, m: &Message) {
    let (on_msg, on_size, armed) = {
        let s = st.borrow();
        (s.on_message.clone(), s.on_content_size.clone(), s.copy_armed)
    };
    match m {
        Message::ContentSize { width, height } => {
            if let Some(f) = on_size {
                f(*width, *height);
            }
        }
        Message::Clipboard(text) => {
            // Guarded copy: only right after the user asked for it, and
            // only text of a sane size.
            if armed.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) && text.len() <= 1 << 20 {
                st.borrow_mut().copy_armed = None;
                app::copy(text);
            }
        }
        Message::Other(_) => {}
    }
    if let Some(f) = on_msg {
        f(m);
    }
}

fn flush_move_of(st: &Rc<RefCell<St>>) {
    let e = {
        let mut s = st.borrow_mut();
        let e = s.pending_move.take();
        if e.is_some() {
            s.last_move = Some(Instant::now());
        }
        e
    };
    if let Some(e) = e {
        emit(st, e);
    }
}

/// Delivers an event: the surface's ring, the JSON writer, the callback.
fn emit(st: &Rc<RefCell<St>>, e: Input) {
    let cb = {
        let mut s = st.borrow_mut();
        let s = &mut *s;
        s.stats.events += 1;
        if let Source::Shm { consumer, .. } = &mut s.source {
            consumer.send(&e);
        }
        if let Some(w) = s.writer.as_mut() {
            let _ = w.write_all(e.to_line().as_bytes());
            let _ = w.flush();
        }
        s.on_event_dyn.clone()
    };
    if let Some(f) = cb {
        f(&e);
    }
}

fn mods() -> Mods {
    let st = app::event_state();
    let mut m = Mods::NONE;
    if st.contains(Shortcut::Shift) {
        m = m | Mods::SHIFT;
    }
    if st.contains(Shortcut::Ctrl) {
        m = m | Mods::CTRL;
    }
    if st.contains(Shortcut::Alt) {
        m = m | Mods::ALT;
    }
    if st.contains(Shortcut::Meta) {
        m = m | Mods::META;
    }
    m
}

fn button() -> Option<Button> {
    match app::event_mouse_button() {
        app::MouseButton::Left => Some(Button::Left),
        app::MouseButton::Right => Some(Button::Right),
        app::MouseButton::Middle => Some(Button::Middle),
        _ => None,
    }
}

/// W3C-style key name of an fltk key ("" for keys the view ignores).
pub(crate) fn key_name(k: Key) -> String {
    let named = match k {
        Key::Escape => "Escape",
        Key::Tab => "Tab",
        Key::BackSpace => "Backspace",
        Key::Enter | Key::KPEnter => "Enter",
        Key::Insert => "Insert",
        Key::Delete => "Delete",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        Key::Left => "ArrowLeft",
        Key::Up => "ArrowUp",
        Key::Right => "ArrowRight",
        Key::Down => "ArrowDown",
        Key::ShiftL | Key::ShiftR => "Shift",
        Key::ControlL | Key::ControlR => "Control",
        Key::AltL | Key::AltR => "Alt",
        Key::MetaL | Key::MetaR => "Meta",
        Key::CapsLock => "CapsLock",
        Key::NumLock => "NumLock",
        Key::ScrollLock => "ScrollLock",
        Key::Pause => "Pause",
        Key::Print => "PrintScreen",
        Key::Menu => "ContextMenu",
        Key::Help => "Help",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    let b = k.bits();
    if (Key::F1.bits()..=Key::F12.bits() + 23).contains(&b) {
        return format!("F{}", b - Key::F1.bits() + 1);
    }
    // Keypad digits and operators: their character.
    if (Key::KP.bits()..=Key::KPLast.bits()).contains(&b) {
        let c = (b - Key::KP.bits()) as u8;
        return if c.is_ascii_graphic() { (c as char).to_string() } else { String::new() };
    }
    match char::from_u32(b as u32) {
        Some(' ') => " ".to_string(),
        Some(c) if !c.is_control() && b < 0xfd00 => c.to_lowercase().collect(),
        _ => String::new(),
    }
}

/// Runs the thread that waits for the producer's wake-ups and wakes the
/// UI sink (once until it ran).
fn start_watch(watch: Watch, stop: Arc<AtomicBool>, pending: Arc<AtomicBool>, sink: crate::task::SinkId) {
    let _ = std::thread::Builder::new().name("fltk-omarchy-frame".into()).spawn(move || {
        let Watch { shared, mut waiter } = watch;
        let check = Watch { shared, waiter: None };
        let poll = waiter.as_ref().map_or(Some(POLL), |w| w.poll_interval());
        loop {
            let r = match waiter.as_mut() {
                Some(w) => w.wait(poll),
                None => {
                    std::thread::sleep(POLL);
                    Wait::Timeout
                }
            };
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let closed = r == Wait::Closed || check.shared.closed() & super::shm::CLOSED_PRODUCER != 0;
            if (closed || check.pending()) && !pending.swap(true, Ordering::AcqRel) {
                sink.wake();
            }
            if closed {
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_letterboxes_and_keeps_one_to_one() {
        // A frame of exactly the device size fills the view.
        assert_eq!(place(Fit::Contain, (0, 0, 400, 300), (800, 600), 2.0), (0, 0, 400, 300));
        // Wider frame: bars above and below.
        assert_eq!(place(Fit::Contain, (10, 10, 400, 400), (800, 400), 1.0), (10, 110, 400, 200));
        // Actual: device pixels, centred when smaller.
        assert_eq!(place(Fit::Actual, (0, 0, 400, 300), (400, 300), 2.0), (100, 75, 200, 150));
        assert_eq!(place(Fit::Actual, (0, 0, 100, 100), (400, 300), 1.0), (0, 0, 400, 300));
    }

    #[test]
    fn conversion_swizzles_and_unpremultiplies() {
        // BGRA premultiplied, half-transparent red, then opaque blue.
        let px = [0, 0, 128, 128, 255, 0, 0, 255];
        assert_eq!(to_straight_rgba(&px, 2, 1, 8, Format::Bgra, false), vec![255, 0, 0, 128, 0, 0, 255, 255]);
        // Stride with padding.
        let px = [1, 2, 3, 255, 9, 9, 9, 9, 4, 5, 6, 255, 9, 9, 9, 9];
        assert_eq!(to_straight_rgba(&px, 1, 2, 8, Format::Rgba, true), vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn key_names_are_w3c_style() {
        assert_eq!(key_name(Key::from_char('a')), "a");
        assert_eq!(key_name(Key::from_char('A')), "a");
        assert_eq!(key_name(Key::Left), "ArrowLeft");
        assert_eq!(key_name(Key::F5), "F5");
        assert_eq!(key_name(Key::from_char(' ')), " ");
        assert_eq!(key_name(Key::KPEnter), "Enter");
        assert_eq!(key_name(Key::from_i32(Key::KP.bits() + b'7' as i32)), "7");
    }
}
