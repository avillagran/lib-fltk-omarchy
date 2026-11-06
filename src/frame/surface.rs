//! Both ends of a frame surface: [`Producer`] (draws frames, reads input)
//! and [`Consumer`] (takes frames, sends input). Either side can create
//! the surface and hand a [`Handle`] to the other process.

use super::event::{Input, Message};
use super::ring::take_lines;
use super::shm::{self, Desc, Format, Shared, CLOSED_CONSUMER, CLOSED_PRODUCER, OPAQUE};
use crate::platform::frame::{self as os, Bell, Extras, Kind, Role, Waiter};
use serde_json::Value;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Environment variable that carries a [`Handle`] to a child process
/// ([`Consumer::share`], [`Producer::from_env`]).
pub const ENV: &str = "OMARCHY_FRAME";

/// Default slot capacity: frames up to this size (in pixels) fit without
/// creating a larger surface. Memory is only used for the pixels written
/// (pages are touched on demand), but the address space is reserved.
pub const DEFAULT_CAPACITY: (u32, u32) = (2560, 1600);

/// How the other process reaches the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transport {
    /// Anonymous memory and pipes inherited by a child process (Unix:
    /// memfd on Linux). The default; Windows uses `Named`.
    #[default]
    Inherit,
    /// Named objects any process of this user can open (POSIX shm and
    /// FIFOs; Windows file mapping and events).
    Named,
    /// A mapped file in a private runtime directory, polled at a capped
    /// rate (8 ms): the fallback that works wherever files can be mapped.
    File,
}

impl Transport {
    fn kind(self) -> Kind {
        match self {
            Transport::Inherit => Kind::Fd,
            Transport::Named => Kind::Named,
            Transport::File => Kind::File,
        }
    }
}

/// What the other process needs to open a surface: a short string
/// (`fd:…`, `named:…`, `file:…`), passed in [`ENV`] or on the command
/// line. The format is documented in docs/FRAMES.md for producers
/// written in other languages.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Handle(String);

impl Handle {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The handle in [`ENV`], if set.
    pub fn from_env() -> Option<Handle> {
        std::env::var(ENV).ok().and_then(|s| s.parse().ok())
    }
}

impl std::fmt::Display for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for Handle {
    type Err = io::Error;
    fn from_str(s: &str) -> io::Result<Handle> {
        if ["fd:", "named:", "file:"].iter().any(|p| s.starts_with(p)) && !s.contains('\n') {
            Ok(Handle(s.to_string()))
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidInput, format!("not a frame handle: {s:?}")))
        }
    }
}

/// How to create a surface.
#[derive(Debug, Clone)]
pub struct Options {
    width: u32,
    height: u32,
    max: (u32, u32),
    transport: Transport,
    dir: Option<PathBuf>,
}

impl Options {
    /// Frames of `width × height` pixels, room for up to
    /// [`DEFAULT_CAPACITY`] (or the size itself when larger).
    pub fn new(width: u32, height: u32) -> Options {
        let (w, h) = (width.max(1), height.max(1));
        Options { width: w, height: h, max: (w.max(DEFAULT_CAPACITY.0), h.max(DEFAULT_CAPACITY.1)), transport: Transport::default(), dir: None }
    }

    /// The largest frame the surface can hold (at least the initial size).
    pub fn capacity(mut self, max_width: u32, max_height: u32) -> Options {
        self.max = (max_width.max(self.width), max_height.max(self.height));
        self
    }

    pub fn transport(mut self, t: Transport) -> Options {
        self.transport = t;
        self
    }

    /// Where named FIFOs and frame files go (default: a private directory
    /// in [`crate::platform::dirs::runtime`]).
    pub fn dir(mut self, dir: impl Into<PathBuf>) -> Options {
        self.dir = Some(dir.into());
        self
    }
}

/// What both ends hold. Field order matters: the mapping goes before the
/// names are removed.
struct End {
    shared: Arc<Shared>,
    bell: Bell,
    waiter: Option<Waiter>,
    extras: Extras,
    kind: Kind,
}

impl End {
    fn create(o: &Options, role: Role) -> io::Result<(Handle, End)> {
        let (max_w, max_h) = o.max;
        if u64::from(max_w) * u64::from(max_h) > 16384 * 16384 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame capacity too large"));
        }
        let (_, _, total) = shm::layout(max_w, max_h);
        let mut seg = os::create(o.transport.kind(), total, o.dir.as_deref(), role);
        if seg.is_err() && o.transport == Transport::Inherit {
            // No anonymous memory here: named objects, then a file.
            seg = os::create(Kind::Named, total, o.dir.as_deref(), role)
                .or_else(|_| os::create(Kind::File, total, o.dir.as_deref(), role));
        }
        let seg = seg?;
        let shared = Shared::from_mapping(seg.map);
        shared.init(max_w, max_h);
        shared.set_pid(role == Role::Producer);
        let handle = Handle(seg.handle);
        Ok((handle, End { shared: Arc::new(shared), bell: seg.bell, waiter: Some(seg.waiter), extras: seg.extras, kind: seg.kind }))
    }

    fn open(h: &Handle, role: Role) -> io::Result<End> {
        let seg = os::open(h.as_str(), role)?;
        let shared = Shared::from_mapping(seg.map);
        shared.validate()?;
        shared.set_pid(role == Role::Producer);
        Ok(End { shared: Arc::new(shared), bell: seg.bell, waiter: Some(seg.waiter), extras: seg.extras, kind: seg.kind })
    }

    fn share(&self, h: &Handle, cmd: &mut std::process::Command) {
        self.extras.share(cmd);
        cmd.env(ENV, h.as_str());
    }

    fn wait(&mut self, timeout: Option<Duration>) -> os::Wait {
        match self.waiter.as_mut() {
            Some(w) => w.wait(timeout),
            None => {
                std::thread::sleep(timeout.unwrap_or(os::POLL).min(os::POLL));
                os::Wait::Timeout
            }
        }
    }
}

// ---------------------------------------------------------------- producer

/// The side that draws frames (usually a helper process).
///
/// ```no_run
/// use fltk_omarchy::frame::Producer;
/// let mut p = Producer::from_env().expect("started by a FrameView");
/// loop {
///     for e in p.wait_events(Some(std::time::Duration::from_millis(33))) {
///         println!("{e:?}");
///     }
///     let (w, h) = p.size();
///     let px = p.frame_mut();
///     px[..(w * h * 4) as usize].fill(0x80);
///     p.present();
/// }
/// ```
pub struct Producer {
    end: End,
    handle: Option<Handle>,
    back: u32,
    size: (u32, u32),
    format: Format,
    opaque: bool,
    inbuf: Vec<u8>,
}

impl Producer {
    /// Creates a surface for `width × height` frames; pass the handle to
    /// the consumer process ([`Producer::share`] for a child).
    pub fn create(width: u32, height: u32) -> io::Result<(Handle, Producer)> {
        Producer::create_with(Options::new(width, height))
    }

    pub fn create_with(o: Options) -> io::Result<(Handle, Producer)> {
        let (h, end) = End::create(&o, Role::Producer)?;
        let p = Producer::wrap(end, Some(h.clone()), (o.width, o.height));
        Ok((h, p))
    }

    /// Opens a surface the consumer created.
    pub fn open(h: &Handle) -> io::Result<Producer> {
        let end = End::open(h, Role::Producer)?;
        let size = end.shared.want().map_or((640, 400), |(w, h, s)| {
            ((w as f32 * s).round() as u32, (h as f32 * s).round() as u32)
        });
        let size = (size.0.clamp(1, end.shared.max_width()), size.1.clamp(1, end.shared.max_height()));
        Ok(Producer::wrap(end, None, size))
    }

    /// Opens the surface named in [`ENV`] (a child started by
    /// `FrameView::spawn` or [`Consumer::share`]).
    pub fn from_env() -> io::Result<Producer> {
        let h = Handle::from_env().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{ENV} is not set")))?;
        Producer::open(&h)
    }

    fn wrap(end: End, handle: Option<Handle>, size: (u32, u32)) -> Producer {
        let back = end.shared.producer_back();
        Producer { end, handle, back, size, format: Format::Rgba, opaque: true, inbuf: Vec::new() }
    }

    /// Lets the child `cmd` open this surface (inherited descriptors and
    /// [`ENV`]). Call [`Producer::release_remote`] after spawning it.
    pub fn share(&self, cmd: &mut std::process::Command) {
        if let Some(h) = &self.handle {
            self.end.share(h, cmd);
        }
    }

    pub fn release_remote(&mut self) {
        self.end.extras.release_remote();
    }

    /// The transport in use (an `Inherit` request may have fallen back).
    pub fn transport(&self) -> Transport {
        transport_of(self.end.kind)
    }

    /// Frame size in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The largest frame this surface holds.
    pub fn capacity(&self) -> (u32, u32) {
        (self.end.shared.max_width(), self.end.shared.max_height())
    }

    /// Bytes per row of [`Producer::frame_mut`] (`width * 4`).
    pub fn stride(&self) -> usize {
        self.size.0 as usize * 4
    }

    /// Changes the size of the next frames (clamped to the capacity;
    /// returns the size that applies).
    pub fn set_size(&mut self, width: u32, height: u32) -> (u32, u32) {
        let (mw, mh) = self.capacity();
        self.size = (width.clamp(1, mw), height.clamp(1, mh));
        self.size
    }

    /// Pixel layout of the next frames (default RGBA, opaque). `opaque`
    /// promises alpha 255 everywhere: the view then blits without a copy.
    pub fn set_format(&mut self, format: Format, opaque: bool) {
        self.format = format;
        self.opaque = opaque;
    }

    /// The pixels of the next frame (`stride() * height` bytes). The
    /// buffer is not cleared: it holds a frame from three presents ago.
    pub fn frame_mut(&mut self) -> &mut [u8] {
        let len = self.stride() * self.size.1 as usize;
        // SAFETY: the producer owns `back` until `present` (triple buffer),
        // and `len` fits in a slot (size clamped to the capacity).
        unsafe { std::slice::from_raw_parts_mut(self.end.shared.slot_ptr(self.back), len) }
    }

    /// Publishes the frame written into [`Producer::frame_mut`] and wakes
    /// the view. Returns its sequence number.
    pub fn present(&mut self) -> u64 {
        let d = Desc {
            width: self.size.0,
            height: self.size.1,
            stride: self.stride() as u32,
            format: self.format.code() | if self.opaque { OPAQUE } else { 0 },
            seq: 0,
            present_ns: shm::now_ns(),
        };
        let (next, seq) = self.end.shared.present(self.back, d);
        self.back = next;
        self.end.bell.ring();
        seq
    }

    /// Input events received so far (does not block). App messages that
    /// are not [`Input`] are skipped: see [`Producer::messages_raw`].
    pub fn events(&mut self) -> Vec<Input> {
        self.messages_raw().iter().filter_map(Input::from_json).collect()
    }

    /// Every JSON message received so far, input or not.
    pub fn messages_raw(&mut self) -> Vec<Value> {
        self.end.shared.input_ring().drain_into(&mut self.inbuf);
        take_lines(&mut self.inbuf).iter().filter_map(|l| serde_json::from_str(l).ok()).collect()
    }

    /// Blocks until a message arrives (or `timeout`, or the view goes
    /// away). True when one is waiting; read it with
    /// [`Producer::events`] or [`Producer::messages_raw`].
    pub fn wait(&mut self, timeout: Option<Duration>) -> bool {
        let deadline = timeout.map(|t| std::time::Instant::now() + t);
        loop {
            if self.end.shared.input_ring().pending() > 0 {
                return true;
            }
            if self.consumer_gone() {
                return false;
            }
            let left = deadline.map(|d| d.saturating_duration_since(std::time::Instant::now()));
            if left.is_some_and(|l| l.is_zero()) {
                return false;
            }
            if matches!(self.end.wait(left), os::Wait::Closed | os::Wait::Interrupted) {
                return self.end.shared.input_ring().pending() > 0;
            }
        }
    }

    /// Waits until input arrives (or `timeout`), then returns it.
    pub fn wait_events(&mut self, timeout: Option<Duration>) -> Vec<Input> {
        self.wait(timeout);
        self.events()
    }

    /// Sends a message to the view (content size, clipboard, app data).
    /// False when the ring is full.
    pub fn send(&mut self, m: &Message) -> bool {
        let ok = self.end.shared.output_ring().push(m.to_line().as_bytes());
        self.end.bell.ring();
        ok
    }

    /// The size the view last asked for: (logical width, logical height,
    /// device scale).
    pub fn requested_size(&self) -> Option<(u32, u32, f32)> {
        self.end.shared.want()
    }

    /// The zoom the view last asked for (1 = 100 %).
    pub fn zoom(&self) -> f32 {
        self.end.shared.zoom()
    }

    /// The last frame the view drew (for pacing: skip work while the view
    /// is far behind).
    pub fn drawn(&self) -> u64 {
        self.end.shared.drawn()
    }

    /// True once the consumer dropped its end.
    pub fn consumer_gone(&self) -> bool {
        self.end.shared.closed() & CLOSED_CONSUMER != 0
    }
}

impl Drop for Producer {
    fn drop(&mut self) {
        self.end.shared.set_closed(CLOSED_PRODUCER);
        self.end.bell.ring();
    }
}

fn transport_of(k: Kind) -> Transport {
    match k {
        Kind::Fd => Transport::Inherit,
        Kind::Named => Transport::Named,
        Kind::File => Transport::File,
    }
}

// ---------------------------------------------------------------- consumer

/// A frame as the producer presented it. The pixels stay valid until the
/// next [`Consumer::acquire`].
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    /// Bytes per row (at least `width * 4`).
    pub stride: usize,
    pub format: Format,
    /// Alpha is 255 everywhere.
    pub opaque: bool,
    pub seq: u64,
    /// When it was presented (ns since the Unix epoch).
    pub presented_ns: u64,
    pub pixels: &'a [u8],
}

/// The side that shows frames (a `FrameView` uses one), usable on its own
/// for tests and tools.
pub struct Consumer {
    end: End,
    handle: Option<Handle>,
    front: u32,
    have: bool,
    outbuf: Vec<u8>,
}

/// What a background thread needs to wait for a consumer's wake-ups.
pub(crate) struct Watch {
    pub(crate) shared: Arc<Shared>,
    pub(crate) waiter: Option<Waiter>,
}

impl Consumer {
    /// Creates a surface; pass the handle to the producer process
    /// ([`Consumer::share`] for a child).
    pub fn create(width: u32, height: u32) -> io::Result<(Handle, Consumer)> {
        Consumer::create_with(Options::new(width, height))
    }

    pub fn create_with(o: Options) -> io::Result<(Handle, Consumer)> {
        let (h, end) = End::create(&o, Role::Consumer)?;
        end.shared.set_want(o.width, o.height, 1.0);
        Ok((h.clone(), Consumer::wrap(end, Some(h))))
    }

    /// Opens a surface the producer created.
    pub fn open(h: &Handle) -> io::Result<Consumer> {
        Ok(Consumer::wrap(End::open(h, Role::Consumer)?, None))
    }

    fn wrap(end: End, handle: Option<Handle>) -> Consumer {
        let front = end.shared.consumer_front();
        Consumer { end, handle, front, have: false, outbuf: Vec::new() }
    }

    /// The handle of a surface this side created.
    pub fn handle(&self) -> Option<&Handle> {
        self.handle.as_ref()
    }

    /// Lets the child `cmd` open this surface (inherited descriptors and
    /// [`ENV`]). Call [`Consumer::release_remote`] after spawning it.
    pub fn share(&self, cmd: &mut std::process::Command) {
        if let Some(h) = &self.handle {
            self.end.share(h, cmd);
        }
    }

    pub fn release_remote(&mut self) {
        self.end.extras.release_remote();
    }

    pub fn transport(&self) -> Transport {
        transport_of(self.end.kind)
    }

    /// The largest frame this surface holds.
    pub fn capacity(&self) -> (u32, u32) {
        (self.end.shared.max_width(), self.end.shared.max_height())
    }

    /// Takes the newest frame if one arrived since the last call.
    pub fn acquire(&mut self) -> Option<Frame<'_>> {
        let f = self.end.shared.acquire(self.front)?;
        self.front = f;
        self.have = true;
        self.frame()
    }

    /// The frame taken last (None before the first, or when its
    /// descriptor is invalid).
    pub fn frame(&self) -> Option<Frame<'_>> {
        if !self.have {
            return None;
        }
        let s = &self.end.shared;
        let d = s.desc(self.front);
        let format = Format::from_code(d.format)?;
        let need = d.stride as usize * d.height as usize;
        if d.width == 0 || d.height == 0 || (d.stride as usize) < d.width as usize * 4 || need > s.slot_len() {
            return None;
        }
        // SAFETY: the consumer owns `front` until the next acquire; the
        // range was checked against the slot size.
        let pixels = unsafe { std::slice::from_raw_parts(s.slot_ptr(self.front), need) };
        Some(Frame {
            width: d.width,
            height: d.height,
            stride: d.stride as usize,
            format,
            opaque: d.format & OPAQUE != 0,
            seq: d.seq,
            presented_ns: d.present_ns,
            pixels,
        })
    }

    /// Frames presented so far (whether taken or not).
    pub fn presented(&self) -> u64 {
        self.end.shared.seq()
    }

    /// Sends an input event to the producer. False when its ring is full.
    pub fn send(&mut self, e: &Input) -> bool {
        match e {
            Input::Resize { width, height, scale } => self.end.shared.set_want(*width, *height, *scale),
            Input::Zoom { value } => self.end.shared.set_zoom(*value),
            _ => {}
        }
        self.push(e.to_line().as_bytes())
    }

    /// Sends any JSON message (the app's own commands) to the producer.
    pub fn send_json(&mut self, v: &Value) -> bool {
        let mut s = v.to_string();
        s.push('\n');
        self.push(s.as_bytes())
    }

    fn push(&mut self, line: &[u8]) -> bool {
        let ok = self.end.shared.input_ring().push(line);
        self.end.bell.ring();
        ok
    }

    /// Messages from the producer received so far.
    pub fn messages(&mut self) -> Vec<Message> {
        self.end.shared.output_ring().drain_into(&mut self.outbuf);
        take_lines(&mut self.outbuf).iter().filter_map(|l| Message::from_line(l)).collect()
    }

    /// Blocks until a frame or a message arrives, the producer goes away
    /// or `timeout` passes. True when there is something to look at.
    pub fn wait(&mut self, timeout: Option<Duration>) -> bool {
        let deadline = timeout.map(|t| std::time::Instant::now() + t);
        loop {
            if self.pending() {
                return true;
            }
            let left = deadline.map(|d| d.saturating_duration_since(std::time::Instant::now()));
            if left.is_some_and(|l| l.is_zero()) {
                return false;
            }
            if self.end.wait(left) == os::Wait::Closed {
                return true;
            }
        }
    }

    fn pending(&self) -> bool {
        self.end.shared.fresh() || self.end.shared.output_ring().pending() > 0 || self.producer_gone()
    }

    /// True once the producer dropped its end.
    pub fn producer_gone(&self) -> bool {
        self.end.shared.closed() & CLOSED_PRODUCER != 0
    }

    /// Records that frame `seq` reached the screen (the producer can read
    /// it for pacing).
    pub fn mark_drawn(&self, seq: u64) {
        self.end.shared.set_drawn(seq);
    }

    /// Hands the waiting end to a background thread (once).
    pub(crate) fn take_watch(&mut self) -> Watch {
        Watch { shared: self.end.shared.clone(), waiter: self.end.waiter.take() }
    }
}

impl Drop for Consumer {
    fn drop(&mut self) {
        self.end.shared.set_closed(CLOSED_CONSUMER);
        self.end.bell.ring();
    }
}

impl Watch {
    /// True when the UI should look: a frame or a message is waiting or
    /// the producer left.
    pub(crate) fn pending(&self) -> bool {
        self.shared.fresh() || self.shared.output_ring().pending() > 0 || self.shared.closed() & CLOSED_PRODUCER != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Button;
    use crate::frame::Mods;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fltk-omarchy-surface-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Removes the (empty) test directories: never a tree.
    fn tidy(d: &std::path::Path) {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let _ = std::fs::remove_dir(e.path());
        }
        let _ = std::fs::remove_dir(d);
    }

    fn exchange(t: Transport) {
        let d = dir(&format!("{t:?}"));
        let o = Options::new(32, 16).capacity(64, 64).transport(t).dir(&d);
        let (h, mut c) = Consumer::create_with(o).unwrap();
        let mut p = Producer::open(&h).unwrap();
        assert_eq!(p.size(), (32, 16), "starts at the size the consumer asked for");
        assert!(c.acquire().is_none());
        for (i, v) in [10u8, 20, 30].iter().enumerate() {
            p.frame_mut().fill(*v);
            assert_eq!(p.present(), i as u64 + 1);
        }
        assert!(c.wait(Some(Duration::from_secs(2))));
        let f = c.acquire().expect("frame");
        assert_eq!((f.width, f.height, f.stride, f.seq), (32, 16, 128, 3));
        assert!(f.pixels.iter().all(|&b| b == 30), "newest frame, whole");
        assert!(f.opaque && f.format == Format::Rgba);
        // Input round trip.
        let e = Input::Press { x: 3.0, y: 4.0, button: Button::Left, buttons: 1, mods: Mods::NONE };
        assert!(c.send(&e));
        assert!(c.send(&Input::Resize { width: 50, height: 40, scale: 1.0 }));
        let got = p.wait_events(Some(Duration::from_secs(2)));
        assert_eq!(got.first(), Some(&e), "{t:?}");
        assert_eq!(p.requested_size(), Some((50, 40, 1.0)));
        assert_eq!(p.set_size(500, 40), (64, 40), "clamped to the capacity");
        // Messages back.
        assert!(p.send(&Message::ContentSize { width: 64, height: 900 }));
        assert!(c.wait(Some(Duration::from_secs(2))));
        assert_eq!(c.messages(), vec![Message::ContentSize { width: 64, height: 900 }]);
        drop(p);
        assert!(c.producer_gone());
        drop(c);
        tidy(&d);
    }

    #[test]
    fn inherit_transport_exchanges_frames_input_and_messages() {
        exchange(Transport::Inherit);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn named_transport_exchanges_frames_input_and_messages() {
        exchange(Transport::Named);
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn file_transport_exchanges_frames_input_and_messages() {
        exchange(Transport::File);
    }

    #[test]
    fn handles_parse_and_reject_garbage() {
        assert!("fd:1:2:3:4".parse::<Handle>().is_ok());
        assert!("named:ab-1:/run/user/1000".parse::<Handle>().is_ok());
        assert!("file:/x".parse::<Handle>().is_ok());
        assert!("http://x".parse::<Handle>().is_err());
        assert!("fd:1\n".parse::<Handle>().is_err());
        assert!(Producer::open(&"fd:1:999999:999998:999997".parse().unwrap()).is_err());
    }
}
