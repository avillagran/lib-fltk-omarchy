//! The player's state machine on the UI thread, shared by [`super::Video`]
//! and [`super::Audio`]: probe, play, pause, seek, mute, end, and the
//! processes of the current segment (one start position).

use super::args::{self, Clock, Pacer};
use super::proc::{self, AudioJob, Event, Events, FrameSlot, Procs, VFrame, VideoJob};
use super::{Error, Info, PlayState};
use fltk::{app, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Seeks (keys, drags) closer than this restart ffmpeg once.
const SEEK_SETTLE: f64 = 0.15;
/// A resize restarts the stream at the new size after this pause.
const RESIZE_SETTLE: f64 = 0.3;
/// Position updates while playing.
const TICK: f64 = 0.25;
/// Seek step of the arrow keys.
pub(crate) const STEP: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Video,
    Audio,
}

/// Why there is no sound although the file has some.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Notice {
    NoAudioOutput,
}

/// Playback statistics (tests, benchmarks).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stats {
    /// Frames shown since the file was set.
    pub frames: u64,
    /// From the last Play (or autoplay) to its first frame.
    pub first_frame: Option<Duration>,
    /// Size of the frames ffmpeg sends, in device pixels.
    pub frame_size: (u32, u32),
    /// ffmpeg segments started (play, seek, resize, unmute).
    pub segments: u64,
    /// The audio output in use (`pulse`, `pw-play`…), if any.
    pub audio_out: Option<&'static str>,
}

pub(crate) struct Core {
    pub kind: Kind,
    pub path: Option<PathBuf>,
    pub name: String,
    pub info: Option<Info>,
    pub state: PlayState,
    pub notice: Option<Notice>,
    pub autoplay: bool,
    want_play: bool,
    probing: bool,
    pub clock: Clock,
    gen: u64,
    poster: bool,
    seg_start: Instant,
    video: Option<Arc<Procs>>,
    audio: Option<Arc<Procs>>,
    audio_started: Option<Instant>,
    slot: Arc<FrameSlot>,
    pub frame: Option<VFrame>,
    pub image: Option<fltk::image::RgbImage>,
    pub box_px: (u32, u32),
    events: Arc<Events>,
    views: Vec<Widget>,
    hooks: Vec<Box<dyn FnMut()>>,
    /// The widget whose visibility decides whether processes may run.
    pub anchor: Option<Widget>,
    tick_armed: bool,
    /// Last time a view of this player was drawn.
    drawn: Option<Instant>,
    need_start: bool,
    settle_gen: u64,
    played_at: Option<Instant>,
    pub stats: Stats,
    /// While dragging the seek bar: the position under the pointer.
    pub scrub: Option<f64>,
}

impl Drop for Core {
    fn drop(&mut self) {
        for p in [self.video.take(), self.audio.take()].into_iter().flatten() {
            p.stop();
        }
    }
}

/// A player: shared by its widgets, driven from the UI thread.
#[derive(Clone)]
pub(crate) struct Player(pub Rc<RefCell<Core>>);

impl Player {
    pub(crate) fn new(kind: Kind) -> Player {
        let cell: Rc<RefCell<Option<Weak<RefCell<Core>>>>> = Rc::default();
        let c2 = cell.clone();
        let sink = crate::task::register_sink(move || {
            let Some(core) = c2.borrow().as_ref().and_then(Weak::upgrade) else { return false };
            Player(core).drain();
            true
        });
        let core = Rc::new(RefCell::new(Core {
            kind,
            path: None,
            name: String::new(),
            info: None,
            state: PlayState::Empty,
            notice: None,
            autoplay: true,
            want_play: false,
            probing: false,
            clock: Clock { base: 0.0, origin: None, duration: None },
            gen: 0,
            poster: false,
            seg_start: Instant::now(),
            video: None,
            audio: None,
            audio_started: None,
            slot: Arc::new(FrameSlot::default()),
            frame: None,
            image: None,
            box_px: (0, 0),
            events: Events::new(sink),
            views: Vec::new(),
            hooks: Vec::new(),
            anchor: None,
            tick_armed: false,
            drawn: None,
            need_start: false,
            settle_gen: 0,
            played_at: None,
            stats: Stats::default(),
            scrub: None,
        }));
        *cell.borrow_mut() = Some(Rc::downgrade(&core));
        let p = Player(core);
        // The global mute starts and stops the sound of a playing player.
        let weak = Rc::downgrade(&p.0);
        super::muted()
            .subscribe(move |m| {
                if let Some(c) = weak.upgrade() {
                    Player(c).muted_changed(*m);
                }
            })
            .forever();
        p
    }

    fn weak(&self) -> Weak<RefCell<Core>> {
        Rc::downgrade(&self.0)
    }

    pub(crate) fn add_view(&self, w: &Widget) {
        self.0.borrow_mut().views.push(w.clone());
    }

    /// Runs `f` on every change (layout that follows the player).
    pub(crate) fn add_hook(&self, f: impl FnMut() + 'static) {
        self.0.borrow_mut().hooks.push(Box::new(f));
    }

    /// Redraws every view (and updates their accessible state).
    pub(crate) fn notify(&self) {
        let mut hooks = std::mem::take(&mut self.0.borrow_mut().hooks);
        for h in hooks.iter_mut() {
            h();
        }
        let mut c = self.0.borrow_mut();
        hooks.append(&mut c.hooks);
        c.hooks = hooks;
        drop(c);
        let views: Vec<Widget> = {
            let mut c = self.0.borrow_mut();
            c.views.retain(|w| !w.was_deleted());
            c.views.clone()
        };
        for mut w in views {
            w.redraw();
            crate::a11y::changed(&w);
        }
    }

    pub(crate) fn state(&self) -> PlayState {
        self.0.borrow().state.clone()
    }

    pub(crate) fn position(&self) -> f64 {
        let c = self.0.borrow();
        c.scrub.unwrap_or_else(|| c.clock.position(Instant::now()))
    }

    pub(crate) fn duration(&self) -> Option<f64> {
        self.0.borrow().clock.duration
    }

    pub(crate) fn is_playing(&self) -> bool {
        self.0.borrow().state == PlayState::Playing
    }

    // ------------------------------------------------------------ file

    pub(crate) fn set_file(&self, path: Option<&Path>) {
        self.stop_segment();
        {
            let mut c = self.0.borrow_mut();
            c.path = path.map(Path::to_path_buf);
            c.info = None;
            c.state = if path.is_some() { PlayState::Loading } else { PlayState::Empty };
            c.notice = None;
            c.want_play = false;
            c.probing = false;
            c.clock = Clock { base: 0.0, origin: None, duration: None };
            c.frame = None;
            c.image = None;
            c.stats = Stats::default();
            c.scrub = None;
            c.need_start = false;
            c.gen += 1;
        }
        self.notify();
    }

    /// Probes the file if not done yet (players call it when first drawn,
    /// so a player that is never shown never starts a process).
    pub(crate) fn ensure_loaded(&self) {
        let (path, gen, events) = {
            let mut c = self.0.borrow_mut();
            if c.info.is_some() || c.probing || c.state != PlayState::Loading {
                return;
            }
            let Some(path) = c.path.clone() else { return };
            c.probing = true;
            (path, c.gen, c.events.clone())
        };
        let _ = std::thread::Builder::new().name("ffprobe".into()).spawn(move || {
            let result = super::available().and_then(|_| proc::probe(&path));
            events.push(Event::Probed { gen, result });
        });
    }

    // --------------------------------------------------------- control

    pub(crate) fn play(&self) {
        let start = {
            let mut c = self.0.borrow_mut();
            match &c.state {
                PlayState::Empty | PlayState::Failed(_) | PlayState::Playing => return,
                PlayState::Loading => {
                    c.want_play = true;
                    drop(c);
                    self.ensure_loaded();
                    return;
                }
                PlayState::Ended => c.clock.base = 0.0,
                PlayState::Paused => {}
            }
            c.state = PlayState::Playing;
            c.played_at = Some(Instant::now());
            c.clock.base
        };
        self.start_segment(start, false);
        self.arm_tick();
        self.notify();
    }

    pub(crate) fn pause(&self) {
        {
            let mut c = self.0.borrow_mut();
            c.want_play = false;
            if c.state != PlayState::Playing {
                return;
            }
            c.clock.freeze(Instant::now());
            c.state = PlayState::Paused;
        }
        self.stop_segment();
        self.notify();
    }

    pub(crate) fn toggle(&self) {
        let s = self.state();
        match s {
            PlayState::Playing => self.pause(),
            PlayState::Loading if self.0.borrow().want_play => {
                self.0.borrow_mut().want_play = false;
                self.notify();
            }
            _ => self.play(),
        }
    }

    /// Jumps to `t` seconds (clamped). Playing: ffmpeg restarts there once
    /// the seeks settle; paused: the frame there is shown.
    pub(crate) fn seek(&self, t: f64) {
        {
            let mut c = self.0.borrow_mut();
            let Some(d) = c.clock.duration else { return };
            if c.info.is_none() || !t.is_finite() {
                return;
            }
            c.clock.base = t.clamp(0.0, d);
            c.clock.origin = None;
            if c.state == PlayState::Ended {
                c.state = PlayState::Paused;
            }
        }
        self.stop_segment();
        self.settle(SEEK_SETTLE, move |p| {
            let (pos, playing_now) = {
                let c = p.0.borrow();
                (c.clock.base, c.state == PlayState::Playing)
            };
            if playing_now {
                p.start_segment(pos, false);
            } else {
                p.start_poster(pos);
            }
        });
        self.notify();
    }

    pub(crate) fn seek_by(&self, dt: f64) {
        let base = {
            let c = self.0.borrow();
            c.clock.position(Instant::now())
        };
        self.seek(base + dt);
    }

    /// Runs `f` once no newer `settle` came within `delay` seconds.
    fn settle(&self, delay: f64, f: impl FnOnce(&Player) + 'static) {
        let gen = {
            let mut c = self.0.borrow_mut();
            c.settle_gen += 1;
            c.settle_gen
        };
        let weak = self.weak();
        let mut f = Some(f);
        app::add_timeout3(delay, move |_| {
            if let Some(c) = weak.upgrade() {
                if c.borrow().settle_gen == gen {
                    if let Some(f) = f.take() {
                        f(&Player(c));
                    }
                }
            }
        });
    }

    fn muted_changed(&self, muted: bool) {
        let (playing, has_audio) = {
            let c = self.0.borrow();
            (c.state == PlayState::Playing, c.info.as_ref().is_some_and(|i| i.audio.is_some()))
        };
        if playing && has_audio {
            if muted {
                if let Some(a) = self.0.borrow_mut().audio.take() {
                    a.stop();
                }
                self.0.borrow_mut().stats.audio_out = None;
            } else {
                let pos = self.position();
                self.start_audio(pos);
            }
        }
        self.notify();
    }

    /// The box the video is drawn in changed (device pixels): restarts the
    /// stream at the size it needs, once the resize settles.
    pub(crate) fn set_box(&self, px: (u32, u32)) {
        let restart = {
            let mut c = self.0.borrow_mut();
            if c.box_px == px {
                return;
            }
            c.box_px = px;
            let Some(v) = c.info.as_ref().and_then(|i| i.video.as_ref()) else { return };
            let need = args::output_size(v.display, px);
            let have = c.stats.frame_size;
            let differs = have.0 == 0 || need.0 > have.0 + 16 || need.0 * 4 < have.0 * 3;
            differs && matches!(c.state, PlayState::Playing | PlayState::Paused) && c.frame.is_some()
        };
        if restart {
            self.settle(RESIZE_SETTLE, |p| {
                let (pos, state) = {
                    let c = p.0.borrow();
                    (c.clock.position(Instant::now()), c.state.clone())
                };
                match state {
                    PlayState::Playing => {
                        p.0.borrow_mut().clock.freeze(Instant::now());
                        p.start_segment(pos, false);
                    }
                    PlayState::Paused => p.start_poster(pos),
                    _ => {}
                }
            });
        }
    }

    // --------------------------------------------------------- segments

    fn ffmpeg() -> Option<PathBuf> {
        proc::find_tool("ffmpeg")
    }

    fn stop_segment(&self) {
        let (v, a) = {
            let mut c = self.0.borrow_mut();
            c.gen += 1;
            (c.video.take(), c.audio.take())
        };
        for p in [v, a].into_iter().flatten() {
            p.stop();
        }
    }

    fn start_segment(&self, pos: f64, poster: bool) {
        self.stop_segment();
        let Some(ffmpeg) = Self::ffmpeg() else {
            self.fail(Error::MissingTools);
            return;
        };
        let (job, audio) = {
            let mut c = self.0.borrow_mut();
            let Some(info) = c.info.clone() else { return };
            let Some(path) = c.path.clone() else { return };
            c.poster = poster;
            c.seg_start = Instant::now();
            c.clock.base = pos;
            c.clock.origin = None;
            c.stats.segments += 1;
            c.slot = Arc::new(FrameSlot::default());
            let job = match (&info.video, c.kind) {
                (Some(v), Kind::Video) => {
                    let bx = if c.box_px.0 > 1 && c.box_px.1 > 1 { c.box_px } else { (640, 360) };
                    let size = args::output_size(v.display, bx);
                    c.stats.frame_size = size;
                    Some(VideoJob { path: path.clone(), gen: c.gen, source: (v.width, v.height), size, pacer: Pacer { start: pos, rate: args::output_rate(v.rate) }, single: poster })
                }
                _ => None,
            };
            if job.is_none() && !poster {
                // Sound only: the clock runs from now.
                c.clock.origin = Some(Instant::now());
            }
            let audio = !poster && info.audio.is_some() && !super::muted().get();
            (job, audio)
        };
        if let Some(job) = job {
            let procs = Procs::new();
            let (slot, events) = {
                let mut c = self.0.borrow_mut();
                c.video = Some(procs.clone());
                (c.slot.clone(), c.events.clone())
            };
            proc::start_video(ffmpeg.clone(), job, procs, slot, events);
        }
        if audio {
            self.start_audio(pos);
        }
    }

    fn start_poster(&self, pos: f64) {
        let has_video = {
            let c = self.0.borrow();
            c.kind == Kind::Video && c.info.as_ref().is_some_and(|i| i.video.is_some())
        };
        if has_video {
            self.start_segment(pos, true);
        }
    }

    fn start_audio(&self, pos: f64) {
        let Some(ffmpeg) = Self::ffmpeg() else { return };
        if let Some(a) = self.0.borrow_mut().audio.take() {
            a.stop();
        }
        let out = match proc::pick_audio_out(&ffmpeg) {
            Ok(o) => o,
            Err(()) => {
                let mut c = self.0.borrow_mut();
                c.notice = Some(Notice::NoAudioOutput);
                c.stats.audio_out = None;
                return;
            }
        };
        let procs = Procs::new();
        let job = {
            let mut c = self.0.borrow_mut();
            c.audio = Some(procs.clone());
            c.audio_started = Some(Instant::now());
            c.stats.audio_out = Some(out.as_ref().map_or("null", |o| o.name()));
            let Some(path) = c.path.clone() else { return };
            let name = if c.name.is_empty() { app_name() } else { c.name.clone() };
            AudioJob { path, gen: c.gen, start: pos, out, name }
        };
        let events = self.0.borrow().events.clone();
        proc::start_audio(ffmpeg, job, procs, events);
    }

    fn fail(&self, e: Error) {
        self.stop_segment();
        {
            let mut c = self.0.borrow_mut();
            c.clock.freeze(Instant::now());
            c.state = PlayState::Failed(e);
        }
        self.notify();
    }

    fn ended(&self) {
        self.stop_segment();
        {
            let mut c = self.0.borrow_mut();
            let now = Instant::now();
            c.clock.freeze(now);
            if let Some(d) = c.clock.duration {
                c.clock.base = d;
            }
            c.state = PlayState::Ended;
        }
        self.notify();
    }

    // ----------------------------------------------------------- events

    fn drain(&self) {
        let events = self.0.borrow().events.take();
        // Frames only redraw the picture; the controls follow the tick.
        let only_frames = events.iter().all(|e| matches!(e, Event::Frame { .. }));
        if only_frames && !events.is_empty() {
            for ev in events {
                if let Event::Frame { gen } = ev {
                    self.on_frame(gen);
                }
            }
            let anchor = self.0.borrow().anchor.clone();
            if let Some(mut w) = anchor.filter(|w| !w.was_deleted()) {
                w.redraw();
            }
            return;
        }
        let redraw = !events.is_empty();
        for ev in events {
            match ev {
                Event::Probed { gen, result } => self.on_probed(gen, result),
                Event::Started { gen, at } => {
                    let mut c = self.0.borrow_mut();
                    if gen == c.gen && !c.poster && c.state == PlayState::Playing {
                        c.clock.origin = Some(at);
                    }
                }
                Event::Frame { gen } => self.on_frame(gen),
                Event::VideoEnded { gen, result } => {
                    let (current, poster, near_end) = {
                        let c = self.0.borrow();
                        let pos = c.clock.position(Instant::now());
                        (gen == c.gen, c.poster, c.clock.duration.is_some_and(|d| pos > d - 1.0))
                    };
                    if !current {
                        continue;
                    }
                    match (result, poster) {
                        (_, true) => {
                            // A poster ends after its frame; its process is gone.
                            self.0.borrow_mut().video = None;
                        }
                        (Ok(_), false) => self.ended(),
                        (Err(_), false) if near_end => self.ended(),
                        (Err(e), false) => self.fail(Error::Failed(e)),
                    }
                }
                Event::AudioEnded { gen, out, result } => self.on_audio_ended(gen, out, result),
            }
        }
        if redraw {
            self.notify();
        }
    }

    fn on_probed(&self, gen: u64, result: Result<Info, Error>) {
        let play = {
            let mut c = self.0.borrow_mut();
            if gen != c.gen || c.state != PlayState::Loading {
                return;
            }
            c.probing = false;
            match result {
                Ok(info) => {
                    c.clock.duration = info.duration;
                    c.info = Some(info);
                    c.state = PlayState::Paused;
                    c.want_play || c.autoplay
                }
                Err(e) => {
                    c.state = PlayState::Failed(e);
                    return;
                }
            }
        };
        if !self.visible() {
            // Shown later: `drawn` starts it then.
            self.0.borrow_mut().need_start = true;
        } else if play {
            self.play();
        } else {
            self.start_poster(0.0);
        }
    }

    fn on_frame(&self, gen: u64) {
        let mut c = self.0.borrow_mut();
        if gen != c.gen {
            return;
        }
        let Some(f) = c.slot.take() else { return };
        if let Some(old) = c.frame.take() {
            if old.rgb.len() == f.rgb.len() {
                c.slot.recycle(old.rgb);
            }
        }
        c.frame = Some(f);
        c.image = None;
        c.stats.frames += 1;
        if let Some(t) = c.played_at.take() {
            c.stats.first_frame = Some(t.elapsed());
        }
    }

    fn on_audio_ended(&self, gen: u64, out: Option<&'static str>, result: Result<(), String>) {
        let (current, has_video, playing, started) = {
            let c = self.0.borrow();
            let has_video = c.kind == Kind::Video && c.info.as_ref().is_some_and(|i| i.video.is_some());
            (gen == c.gen, has_video, c.state == PlayState::Playing, c.audio_started)
        };
        if !current {
            return;
        }
        self.0.borrow_mut().audio = None;
        match result {
            Ok(()) if !has_video => self.ended(),
            Ok(()) => {}
            Err(_) if !playing => {}
            Err(e) => {
                // An output that fails right away (no server, no device) is
                // skipped from now on; then the next one is tried.
                let early = started.is_some_and(|t| t.elapsed() < Duration::from_millis(2500));
                match out {
                    Some(name) if early => {
                        proc::mark_out_failed(name);
                        let pos = self.position();
                        self.start_audio(pos);
                    }
                    _ if !has_video => self.fail(Error::Failed(e)),
                    _ => self.0.borrow_mut().notice = Some(Notice::NoAudioOutput),
                }
            }
        }
    }

    // ------------------------------------------------------------- tick

    /// Shown: its window is shown, or it was drawn within the last
    /// 1.5 s (offscreen snapshots; a hidden or scrolled-away player is
    /// not drawn).
    fn visible(&self) -> bool {
        let c = self.0.borrow();
        let recent = c.drawn.is_some_and(|t| t.elapsed() < Duration::from_millis(1500));
        c.anchor.as_ref().map_or(true, |w| !w.was_deleted() && (w.visible_r() || recent))
    }

    /// A view was drawn (the player is on screen): starts the probe, and
    /// what waited for the player to be shown (autoplay or the poster).
    pub(crate) fn drawn(&self) {
        let (start, play) = {
            let mut c = self.0.borrow_mut();
            c.drawn = Some(Instant::now());
            let start = std::mem::take(&mut c.need_start) && c.state == PlayState::Paused;
            (start, c.want_play || c.autoplay)
        };
        self.ensure_loaded();
        if start && play {
            self.play();
        } else if start {
            let pos = self.0.borrow().clock.base;
            self.start_poster(pos);
        }
    }

    /// While playing: updates the position four times a second, and stops
    /// the processes when the player is hidden, deleted or stopped from
    /// outside ([`super::stop_all`]).
    fn arm_tick(&self) {
        if std::mem::replace(&mut self.0.borrow_mut().tick_armed, true) {
            return;
        }
        let weak = self.weak();
        app::add_timeout3(TICK, move |_| {
            let Some(c) = weak.upgrade() else { return };
            let p = Player(c);
            p.0.borrow_mut().tick_armed = false;
            let (deleted, playing, killed) = {
                let c = p.0.borrow();
                let deleted = c.anchor.as_ref().is_some_and(|w| w.was_deleted());
                let killed = c.video.as_ref().is_some_and(|v| v.is_stopped()) || c.audio.as_ref().is_some_and(|a| a.is_stopped());
                (deleted, c.state == PlayState::Playing, killed)
            };
            if deleted {
                p.stop_segment();
                return;
            }
            if !playing {
                return;
            }
            if !p.visible() || killed {
                p.pause();
                return;
            }
            p.notify();
            p.arm_tick();
        });
    }

    /// Stops processes and keeps the position (the widget went away).
    pub(crate) fn release(&self) {
        self.pause();
        self.stop_segment();
    }
}

/// The name audio servers show for the stream when the player has none.
fn app_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "fltk-omarchy".into())
}
