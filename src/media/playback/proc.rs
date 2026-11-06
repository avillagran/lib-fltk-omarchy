//! Child processes: finding ffmpeg, spawning and reaping, the video
//! reader with its latest-frame slot, the audio monitor and ffprobe.
//!
//! Every child belongs to a [`Procs`] group registered globally. A group
//! is killed and reaped as a whole ([`Procs::stop`]) from the UI thread;
//! the thread that spawned a child (and reads it) lives until the child
//! is reaped, which is what Linux's parent-death signal watches.

use super::args::{self, Pacer};
use super::{Error, Info};
use crate::platform::playback::{self as os, AudioOut};
use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ tools

static TOOL_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

pub(crate) fn set_tool_dir(dir: Option<PathBuf>) {
    *TOOL_DIR.lock().unwrap() = dir;
}

/// Full path of `tool` (`ffmpeg`, `ffprobe`, `pw-play`…). With an app
/// directory set, ffmpeg and ffprobe come only from there (the app ships
/// them); players are looked up there first, then in [`os::tool_dirs`].
pub(crate) fn find_tool(tool: &str) -> Option<PathBuf> {
    let exe = os::exe_name(tool);
    let own = TOOL_DIR.lock().unwrap().clone();
    if let Some(d) = &own {
        let p = d.join(&exe);
        if p.is_file() {
            return Some(p);
        }
        if matches!(tool, "ffmpeg" | "ffprobe") {
            return None;
        }
    }
    os::tool_dirs().into_iter().map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// ffmpeg's output devices with muxing support (`ffmpeg -devices`), read
/// once.
fn output_devices(ffmpeg: &Path) -> &'static HashSet<String> {
    static DEVICES: OnceLock<HashSet<String>> = OnceLock::new();
    DEVICES.get_or_init(|| {
        let out = run_capture(ffmpeg, &["-hide_banner".into(), "-devices".into()], Duration::from_secs(5), 64 * 1024);
        let mut set = HashSet::new();
        if let Ok((text, _)) = out {
            for line in String::from_utf8_lossy(&text).lines() {
                let mut parts = line.split_whitespace();
                if let (Some(flags), Some(name)) = (parts.next(), parts.next()) {
                    if flags.contains('E') && flags.len() <= 3 && !flags.contains('=') {
                        for n in name.split(',') {
                            set.insert(n.to_string());
                        }
                    }
                }
            }
        }
        set
    })
}

/// Outputs that failed in this run (skipped from then on).
static FAILED_OUTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

pub(crate) fn mark_out_failed(name: &'static str) {
    let mut f = FAILED_OUTS.lock().unwrap();
    if !f.contains(&name) {
        f.push(name);
    }
}

/// Where audio goes now: `Ok(Some(out))`, `Ok(None)` for the silent test
/// output (`OMARCHY_FLTK_AUDIO_OUT=null`), `Err(())` when no output is left.
pub(crate) fn pick_audio_out(ffmpeg: &Path) -> Result<Option<AudioOut>, ()> {
    let forced = std::env::var("OMARCHY_FLTK_AUDIO_OUT").ok().filter(|v| !v.is_empty());
    let failed = FAILED_OUTS.lock().unwrap().clone();
    match forced.as_deref() {
        Some("null") => return Ok(None),
        Some("none") => return Err(()),
        Some(name) => {
            return match os::audio_out_named(name) {
                Some(o) if !failed.contains(&o.name()) => Ok(Some(o)),
                _ => Err(()),
            }
        }
        None => {}
    }
    for o in os::audio_outputs() {
        if failed.contains(&o.name()) {
            continue;
        }
        let usable = match o {
            AudioOut::Device { format, .. } => output_devices(ffmpeg).contains(*format),
            AudioOut::Player { program, .. } => find_tool(program).is_some(),
        };
        if usable {
            return Ok(Some(*o));
        }
    }
    Err(())
}

// -------------------------------------------------------------- registry

/// The children of one stream (a video reader, or ffmpeg + a player).
pub(crate) struct Procs {
    children: Mutex<Vec<Child>>,
    stopped: AtomicBool,
}

static REGISTRY: Mutex<Vec<Weak<Procs>>> = Mutex::new(Vec::new());

extern "C" fn kill_all_at_exit() {
    // Never block at exit: another thread may hold a lock.
    let groups: Vec<Arc<Procs>> = match REGISTRY.try_lock() {
        Ok(r) => r.iter().filter_map(Weak::upgrade).collect(),
        Err(_) => return,
    };
    for g in groups {
        g.stop();
    }
}

impl Procs {
    pub(crate) fn new() -> Arc<Procs> {
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            os::at_exit(kill_all_at_exit);
        });
        let p = Arc::new(Procs { children: Mutex::new(Vec::new()), stopped: AtomicBool::new(false) });
        let mut r = REGISTRY.lock().unwrap();
        r.retain(|w| w.strong_count() > 0);
        r.push(Arc::downgrade(&p));
        p
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    /// Spawns `cmd` into this group (refused once stopped). Returns the
    /// child's pid, stdout and stderr (when piped).
    pub(crate) fn spawn(&self, mut cmd: Command) -> io::Result<(u32, Option<ChildStdout>, Option<ChildStderr>)> {
        os::contain(&mut cmd);
        let mut children = self.children.lock().unwrap();
        if self.is_stopped() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
        }
        let mut child = cmd.spawn()?;
        os::adopt(&child);
        let id = child.id();
        let (out, err) = (child.stdout.take(), child.stderr.take());
        children.push(child);
        Ok((id, out, err))
    }

    /// Kills and reaps every child, now. Idempotent; later spawns fail.
    pub(crate) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        let mut children = self.children.lock().unwrap_or_else(|e| e.into_inner());
        for c in children.iter_mut() {
            let _ = c.kill();
        }
        for mut c in children.drain(..) {
            let _ = c.wait();
        }
    }

    /// Exit status of child `pid` if it has exited (reaped and removed).
    pub(crate) fn try_reap(&self, pid: u32) -> Option<ExitStatus> {
        let mut children = self.children.lock().unwrap();
        let i = children.iter().position(|c| c.id() == pid)?;
        match children[i].try_wait() {
            Ok(Some(st)) => {
                // Already reaped by try_wait: wait returns the kept status.
                let _ = children.remove(i).wait();
                Some(st)
            }
            Ok(None) => None,
            Err(_) => {
                let mut c = children.remove(i);
                let _ = c.kill();
                c.wait().ok()
            }
        }
    }

    /// Waits (up to `limit`, then kills) for child `pid` to exit; `None`
    /// when the group was stopped meanwhile.
    pub(crate) fn reap(&self, pid: u32, limit: Duration) -> Option<ExitStatus> {
        let end = Instant::now() + limit;
        loop {
            if let Some(st) = self.try_reap(pid) {
                return Some(st);
            }
            if !self.children.lock().unwrap().iter().any(|c| c.id() == pid) {
                return None; // reaped by stop()
            }
            if Instant::now() > end {
                let mut children = self.children.lock().unwrap();
                if let Some(i) = children.iter().position(|c| c.id() == pid) {
                    let mut c = children.remove(i);
                    let _ = c.kill();
                    return c.wait().ok();
                }
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    pub(crate) fn live(&self) -> usize {
        self.children.lock().map(|c| c.len()).unwrap_or(0)
    }
}

impl Drop for Procs {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Kills and reaps every playback process of this app.
pub(crate) fn stop_all() {
    let groups: Vec<Arc<Procs>> = REGISTRY.lock().unwrap().iter().filter_map(Weak::upgrade).collect();
    for g in groups {
        g.stop();
    }
}

/// Playback processes alive (spawned and not reaped yet).
pub(crate) fn live_processes() -> usize {
    REGISTRY.lock().unwrap().iter().filter_map(Weak::upgrade).map(|g| g.live()).sum()
}

fn command(program: &Path, args: &[OsString]) -> Command {
    let mut c = Command::new(program);
    c.args(args).stdin(Stdio::null()).env_remove("FFREPORT").env("AV_LOG_FORCE_NOCOLOR", "1");
    for (k, v) in os::child_env() {
        c.env(k, v);
    }
    c
}

/// Keeps the first `MAX` bytes of a child's stderr (and drains the rest,
/// so the child never blocks on a full pipe).
fn drain_stderr(err: Option<ChildStderr>) -> Arc<Mutex<Vec<u8>>> {
    const MAX: usize = 4096;
    let text = Arc::new(Mutex::new(Vec::new()));
    if let Some(mut err) = err {
        let t = text.clone();
        let _ = std::thread::Builder::new().name("ffmpeg-stderr".into()).spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = err.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut t = t.lock().unwrap();
                let keep = n.min(MAX - t.len().min(MAX));
                t.extend_from_slice(&buf[..keep]);
            }
        });
    }
    text
}

fn diagnostic(text: &Mutex<Vec<u8>>, status: Option<ExitStatus>) -> String {
    let t = text.lock().unwrap();
    let s: String = String::from_utf8_lossy(&t).chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).collect();
    let last = s.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
    match (last.is_empty(), status) {
        (false, _) => last,
        (true, Some(st)) => format!("ffmpeg exited with {st}"),
        (true, None) => "ffmpeg stopped".into(),
    }
}

/// Runs a short command and returns its stdout (at most `max` bytes),
/// killing it after `timeout`.
pub(crate) fn run_capture(program: &Path, args: &[OsString], timeout: Duration, max: usize) -> io::Result<(Vec<u8>, ExitStatus)> {
    let procs = Procs::new();
    let mut cmd = command(program, args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let (pid, out, err) = procs.spawn(cmd)?;
    let errs = drain_stderr(err);
    let mut out = out.ok_or_else(|| io::Error::other("no stdout"))?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        while let Ok(n) = out.read(&mut chunk) {
            if n == 0 {
                break;
            }
            if buf.len() + n > max {
                return Err(io::Error::other("output too large"));
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Ok(buf)
    });
    let status = procs.reap(pid, timeout);
    procs.stop();
    let buf = reader.join().map_err(|_| io::Error::other("reader panicked"))??;
    let status = status.ok_or_else(|| io::Error::new(io::ErrorKind::Interrupted, "stopped"))?;
    if !status.success() {
        return Err(io::Error::other(diagnostic(&errs, Some(status))));
    }
    Ok((buf, status))
}

/// Checks a path the app passed: absolute, a regular file (no device,
/// FIFO or directory), not a URL.
pub(crate) fn check_path(path: &Path) -> Result<(), Error> {
    if !path.is_absolute() {
        return Err(Error::Unsupported("not an absolute file path".into()));
    }
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => Ok(()),
        Ok(_) => Err(Error::Unsupported("not a regular file".into())),
        Err(e) => Err(Error::Io(e.to_string())),
    }
}

/// ffprobe `path` (blocking; a worker calls it).
pub(crate) fn probe(path: &Path) -> Result<Info, Error> {
    check_path(path)?;
    let ffprobe = find_tool("ffprobe").ok_or(Error::MissingTools)?;
    let (json, _) = run_capture(&ffprobe, &super::probe::probe_args(path), Duration::from_secs(10), args::MAX_PROBE_BYTES)
        .map_err(|e| Error::Unsupported(e.to_string()))?;
    super::parse_probe(&json)
}

// --------------------------------------------------------------- frames

/// One RGB24 video frame.
pub(crate) struct VFrame {
    pub rgb: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Media position (seek start + frame index / rate).
    pub pts: f64,
    #[cfg_attr(not(test), allow(dead_code))]
    pub seq: u64,
}

#[derive(Default)]
struct SlotState {
    ready: Option<VFrame>,
    spare: Option<Vec<u8>>,
}

/// The latest decoded frame, at most one waiting for the UI, plus one
/// spare buffer: a stream holds at most three frame buffers (the one
/// being filled, the one waiting, the one on screen), and none is
/// allocated per frame.
#[derive(Default)]
pub(crate) struct FrameSlot {
    st: Mutex<SlotState>,
}

impl FrameSlot {
    /// Replaces the waiting frame; returns a buffer to fill next (the
    /// frame never shown, or the spare one).
    pub(crate) fn publish(&self, f: VFrame) -> Option<Vec<u8>> {
        let mut s = self.st.lock().unwrap();
        let old = s.ready.replace(f).map(|o| o.rgb);
        old.or_else(|| s.spare.take())
    }

    pub(crate) fn take(&self) -> Option<VFrame> {
        self.st.lock().unwrap().ready.take()
    }

    /// The UI gives back the buffer it no longer shows.
    pub(crate) fn recycle(&self, buf: Vec<u8>) {
        let mut s = self.st.lock().unwrap();
        if s.spare.is_none() {
            s.spare = Some(buf);
        }
    }

    #[cfg(test)]
    pub(crate) fn clear(&self) {
        *self.st.lock().unwrap() = SlotState::default();
    }

    /// Bytes held (waiting frame + spare).
    #[cfg(test)]
    pub(crate) fn bytes(&self) -> usize {
        let s = self.st.lock().unwrap();
        s.ready.as_ref().map_or(0, |f| f.rgb.capacity()) + s.spare.as_ref().map_or(0, |b| b.capacity())
    }
}

/// What the stream threads tell the UI thread.
#[derive(Debug)]
pub(crate) enum Event {
    /// First frame of segment `gen` arrived at this instant.
    Started { gen: u64, at: Instant },
    /// A new frame waits in the slot.
    Frame { gen: u64 },
    /// The video stream of `gen` ended (`Ok`: end of file).
    VideoEnded { gen: u64, result: Result<u64, String> },
    /// The audio stream of `gen` ended (`Ok`: end of file); `out` is the
    /// output that was used.
    AudioEnded { gen: u64, out: Option<&'static str>, result: Result<(), String> },
    Probed { gen: u64, result: Result<Info, Error> },
}

/// Queue + wake-up of the UI sink (`task::register_sink`).
pub(crate) struct Events {
    q: Mutex<Vec<Event>>,
    scheduled: AtomicBool,
    frame_pending: AtomicBool,
    sink: crate::task::SinkId,
}

impl Events {
    pub(crate) fn new(sink: crate::task::SinkId) -> Arc<Events> {
        Arc::new(Events { q: Mutex::new(Vec::new()), scheduled: AtomicBool::new(false), frame_pending: AtomicBool::new(false), sink })
    }

    pub(crate) fn push(&self, ev: Event) {
        if let Event::Frame { .. } = ev {
            // Frames coalesce: the UI takes the latest one.
            if self.frame_pending.swap(true, Ordering::AcqRel) {
                return;
            }
        }
        self.q.lock().unwrap().push(ev);
        if !self.scheduled.swap(true, Ordering::AcqRel) {
            self.sink.wake();
        }
    }

    pub(crate) fn take(&self) -> Vec<Event> {
        self.scheduled.store(false, Ordering::Release);
        self.frame_pending.store(false, Ordering::Release);
        std::mem::take(&mut *self.q.lock().unwrap())
    }
}

// ---------------------------------------------------------------- video

pub(crate) struct VideoJob {
    pub path: PathBuf,
    pub gen: u64,
    /// Coded size of the track (thread counts).
    pub source: (u32, u32),
    pub size: (u32, u32),
    pub pacer: Pacer,
    pub single: bool,
}

/// Sleeps until `until`, waking early when the group stops.
fn sleep_until(procs: &Procs, until: Instant) {
    loop {
        let now = Instant::now();
        if now >= until || procs.is_stopped() {
            return;
        }
        std::thread::sleep((until - now).min(Duration::from_millis(20)));
    }
}

/// Starts a video stream on its own thread: spawns ffmpeg, reads whole
/// frames, holds each until it is due (pipe back-pressure keeps ffmpeg
/// at most a pipe buffer ahead), publishes it to `slot`, reaps ffmpeg.
pub(crate) fn start_video(ffmpeg: PathBuf, job: VideoJob, procs: Arc<Procs>, slot: Arc<FrameSlot>, events: Arc<Events>) {
    let _ = std::thread::Builder::new().name("ffmpeg-video".into()).spawn(move || {
        let gen = job.gen;
        let result = read_video(&ffmpeg, &job, &procs, &slot, &events);
        if !procs.is_stopped() {
            events.push(Event::VideoEnded { gen, result });
        }
    });
}

fn read_video(ffmpeg: &Path, job: &VideoJob, procs: &Procs, slot: &FrameSlot, events: &Events) -> Result<u64, String> {
    let (w, h) = job.size;
    let bytes = w as usize * h as usize * 3;
    let mut cmd = command(ffmpeg, &args::video_args(&job.path, job.pacer.start, job.source, job.size, job.pacer.rate, job.single));
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let (pid, out, err) = procs.spawn(cmd).map_err(|e| e.to_string())?;
    let errs = drain_stderr(err);
    let mut out = out.ok_or("no video pipe")?;
    platform_pipe_size(&out, bytes);
    let mut seq = 0u64;
    let mut t0: Option<Instant> = None;
    let mut buf: Vec<u8> = Vec::new();
    let failure = loop {
        if buf.len() != bytes {
            buf = Vec::new();
            if buf.try_reserve_exact(bytes).is_err() {
                break Some("not enough memory for a video frame".to_string());
            }
            buf.resize(bytes, 0);
        }
        match read_frame(&mut out, &mut buf) {
            Ok(true) => {}
            Ok(false) => break None, // end of stream
            Err(e) => break Some(e.to_string()),
        }
        if procs.is_stopped() {
            break None;
        }
        match t0 {
            None => {
                let now = Instant::now();
                t0 = Some(now);
                events.push(Event::Started { gen: job.gen, at: now });
            }
            Some(t0) => sleep_until(procs, Instant::now() + job.pacer.wait(t0, seq, Instant::now())),
        }
        if procs.is_stopped() {
            break None;
        }
        let frame = VFrame { rgb: std::mem::take(&mut buf), width: w, height: h, pts: job.pacer.pts(seq), seq };
        if let Some(b) = slot.publish(frame) {
            buf = b;
        }
        events.push(Event::Frame { gen: job.gen });
        seq += 1;
    };
    drop(out);
    let status = procs.reap(pid, Duration::from_secs(2));
    if procs.is_stopped() {
        return Ok(seq);
    }
    match (failure, status) {
        (Some(e), _) => Err(e),
        (None, Some(st)) if st.success() && seq > 0 => Ok(seq),
        (None, st) => Err(diagnostic(&errs, st)),
    }
}

/// Fills `buf` with one frame: `Ok(false)` at a clean end of stream.
fn read_frame(r: &mut impl Read, buf: &mut [u8]) -> io::Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "video stream ended inside a frame")),
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(true)
}

/// Larger pipe for large frames where the OS allows it (Linux): fewer
/// wake-ups per frame. Failure keeps the default.
fn platform_pipe_size(out: &ChildStdout, frame: usize) {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let want = frame.clamp(64 << 10, 1 << 20) as libc::c_int;
        // SAFETY: fcntl on a pipe descriptor we own; failure is harmless.
        unsafe { libc::fcntl(out.as_raw_fd(), libc::F_SETPIPE_SZ, want) };
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (out, frame);
}

// ---------------------------------------------------------------- audio

pub(crate) struct AudioJob {
    pub path: PathBuf,
    pub gen: u64,
    pub start: f64,
    pub out: Option<AudioOut>,
    pub name: String,
}

/// Starts an audio stream: ffmpeg to its output device, or ffmpeg piped
/// into a player; a monitor thread reaps both and reports how it ended.
pub(crate) fn start_audio(ffmpeg: PathBuf, job: AudioJob, procs: Arc<Procs>, events: Arc<Events>) {
    let _ = std::thread::Builder::new().name("ffmpeg-audio".into()).spawn(move || {
        let gen = job.gen;
        let out = job.out.as_ref().map(|o| o.name());
        let result = run_audio(&ffmpeg, &job, &procs);
        if !procs.is_stopped() {
            events.push(Event::AudioEnded { gen, out, result });
        }
    });
}

fn run_audio(ffmpeg: &Path, job: &AudioJob, procs: &Procs) -> Result<(), String> {
    let mut cmd = command(ffmpeg, &args::audio_args(&job.path, job.start, job.out.as_ref(), &job.name));
    let player = match job.out {
        Some(AudioOut::Player { program, args }) => {
            Some((find_tool(program).ok_or_else(|| format!("{program} not found"))?, args))
        }
        _ => None,
    };
    cmd.stdout(if player.is_some() { Stdio::piped() } else { Stdio::null() }).stderr(Stdio::piped());
    let (pid, out, err) = procs.spawn(cmd).map_err(|e| e.to_string())?;
    let errs = drain_stderr(err);
    let mut pids = vec![pid];
    if let Some((program, pargs)) = player {
        let stdin = out.ok_or("no audio pipe")?;
        let mut p = Command::new(program);
        p.args(pargs).stdin(Stdio::from(stdin)).stdout(Stdio::null()).stderr(Stdio::null());
        match procs.spawn(p) {
            Ok((ppid, _, _)) => pids.push(ppid),
            Err(e) => {
                procs.stop();
                return Err(e.to_string());
            }
        }
    }
    // Both exit together: ffmpeg at the end of the file, the player when
    // its input ends; either failing stops the other (broken pipe).
    let mut result = Ok(());
    while !pids.is_empty() {
        if procs.is_stopped() {
            return Ok(());
        }
        let mut i = 0;
        while i < pids.len() {
            if let Some(st) = procs.try_reap(pids[i]) {
                if !st.success() && result.is_ok() {
                    result = Err(if pids[i] == pid { diagnostic(&errs, Some(st)) } else { format!("audio player exited with {st}") });
                }
                pids.remove(i);
            } else {
                i += 1;
            }
        }
        if result.is_err() {
            // Do not leave the partner running.
            for p in &pids {
                let _ = procs.reap(*p, Duration::from_millis(300));
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seq: u64, fill: u8) -> VFrame {
        VFrame { rgb: vec![fill; 6], width: 2, height: 1, pts: seq as f64, seq }
    }

    #[test]
    fn slot_keeps_the_latest_frame_and_reuses_buffers() {
        let slot = FrameSlot::default();
        assert!(slot.publish(frame(0, 1)).is_none());
        let back = slot.publish(frame(1, 2)).expect("the unseen frame's buffer comes back");
        assert_eq!(back, vec![1; 6]);
        let f = slot.take().unwrap();
        assert_eq!((f.seq, f.rgb[0]), (1, 2));
        assert!(slot.take().is_none());
        slot.recycle(f.rgb);
        assert_eq!(slot.bytes(), 6);
        assert_eq!(slot.publish(frame(2, 3)).map(|b| b[0]), Some(2), "spare buffer reused");
        assert_eq!(slot.bytes(), 6, "one waiting frame, no spare");
        slot.recycle(vec![0; 6]);
        slot.recycle(vec![0; 6]);
        assert_eq!(slot.bytes(), 12, "at most one spare");
        slot.clear();
        assert_eq!(slot.bytes(), 0);
    }

    #[test]
    fn whole_frames_only() {
        let mut buf = [0u8; 4];
        let mut r: &[u8] = &[1, 2, 3, 4, 5, 6, 7, 8];
        assert!(read_frame(&mut r, &mut buf).unwrap());
        assert!(read_frame(&mut r, &mut buf).unwrap());
        assert_eq!(buf, [5, 6, 7, 8]);
        assert!(!read_frame(&mut r, &mut buf).unwrap(), "clean end");
        let mut short: &[u8] = &[1, 2];
        assert!(read_frame(&mut short, &mut buf).is_err(), "truncated frame");
    }

    #[test]
    fn paths_are_checked() {
        assert!(check_path(Path::new("relative.mp4")).is_err());
        assert!(check_path(Path::new("/definitely/not/here.mp4")).is_err());
        let dir = std::env::temp_dir();
        assert!(matches!(check_path(&dir), Err(Error::Unsupported(_))), "a directory");
    }

    /// A 2 s clip (mpeg4 + aac: encoders every ffmpeg build has) in the
    /// temp dir, or `None` without ffmpeg (CI may not have it).
    fn clip(name: &str) -> Option<PathBuf> {
        let ffmpeg = find_tool("ffmpeg")?;
        find_tool("ffprobe")?;
        let path = std::env::temp_dir().join(format!("fltk-omarchy-{}-{name}.mp4", std::process::id()));
        let ok = Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=2"])
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=2:sample_rate=48000", "-c:v", "mpeg4", "-c:a", "aac", "-shortest"])
            .arg(&path)
            .status()
            .is_ok_and(|s| s.success());
        ok.then_some(path)
    }

    fn events() -> Arc<Events> {
        Events::new(crate::task::register_sink(|| true))
    }

    #[test]
    fn real_ffmpeg_probe_paced_video_and_reaping() {
        let Some(path) = clip("video") else {
            eprintln!("skipped: no ffmpeg/ffprobe");
            return;
        };
        let info = probe(&path).unwrap();
        assert!((info.duration.unwrap() - 2.0).abs() < 0.1, "{info:?}");
        let v = info.video.clone().unwrap();
        assert_eq!((v.display, v.rate), ((320, 180), Some((25, 1))));
        assert!(info.audio.is_some());
        // Paced: 1 s of video from 1.0 takes about 1 s (25 frames at 25 fps).
        let procs = Procs::new();
        let slot = Arc::new(FrameSlot::default());
        let ev = events();
        let job = VideoJob { path: path.clone(), gen: 7, source: (320, 180), size: (160, 90), pacer: Pacer { start: 1.0, rate: (25, 1) }, single: false };
        let t = Instant::now();
        let r = read_video(&find_tool("ffmpeg").unwrap(), &job, &procs, &slot, &ev);
        let took = t.elapsed();
        assert_eq!(r, Ok(25), "frames of the last second");
        assert!(took >= Duration::from_millis(900) && took < Duration::from_millis(3000), "paced: {took:?}");
        let f = slot.take().unwrap();
        assert_eq!((f.width, f.height, f.rgb.len()), (160, 90, 160 * 90 * 3));
        assert!((f.pts - 1.96).abs() < 1e-9, "last frame at 1.96 s: {}", f.pts);
        assert_eq!(procs.live(), 0, "reaped at the end");
        let evs = ev.take();
        assert!(matches!(evs.first(), Some(Event::Started { gen: 7, .. })), "{evs:?}");
        // Stopping mid-stream kills and reaps at once.
        let procs = Procs::new();
        let job = VideoJob { path: path.clone(), gen: 8, source: (320, 180), size: (160, 90), pacer: Pacer { start: 0.0, rate: (25, 1) }, single: false };
        start_video(find_tool("ffmpeg").unwrap(), job, procs.clone(), slot.clone(), ev.clone());
        let end = Instant::now() + Duration::from_secs(3);
        while slot.take().is_none() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(procs.live(), 1);
        procs.stop();
        assert_eq!(procs.live(), 0);
        // One frame (poster).
        let procs = Procs::new();
        let job = VideoJob { path: path.clone(), gen: 9, source: (320, 180), size: (2, 2), pacer: Pacer { start: 0.5, rate: (25, 1) }, single: true };
        assert_eq!(read_video(&find_tool("ffmpeg").unwrap(), &job, &procs, &slot, &ev), Ok(1));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn real_ffmpeg_audio_plays_in_real_time_and_refuses_junk() {
        let Some(path) = clip("audio") else {
            eprintln!("skipped: no ffmpeg/ffprobe");
            return;
        };
        let ffmpeg = find_tool("ffmpeg").unwrap();
        // The silent output: decoded at playback speed, nothing audible.
        let procs = Procs::new();
        let job = AudioJob { path: path.clone(), gen: 1, start: 1.0, out: None, name: "test".into() };
        let t = Instant::now();
        assert_eq!(run_audio(&ffmpeg, &job, &procs), Ok(()));
        let took = t.elapsed();
        assert!(took >= Duration::from_millis(500) && took < Duration::from_millis(2500), "real time: {took:?}");
        assert_eq!(procs.live(), 0);
        // A player that cannot start is an error, not a hang.
        let procs = Procs::new();
        let bogus = AudioOut::Player { program: "fltk-omarchy-no-such-player", args: &[] };
        let job = AudioJob { path: path.clone(), gen: 2, start: 0.0, out: Some(bogus), name: "test".into() };
        assert!(run_audio(&ffmpeg, &job, &procs).is_err());
        assert_eq!(procs.live(), 0);
        // Not media: ffprobe refuses it, nothing is left running.
        let junk = std::env::temp_dir().join(format!("fltk-omarchy-{}-junk.mp4", std::process::id()));
        std::fs::write(&junk, b"definitely not a video").unwrap();
        assert!(probe(&junk).is_err());
        let _ = std::fs::remove_file(&junk);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stopped_group_refuses_spawns() {
        let p = Procs::new();
        p.stop();
        let me = std::env::current_exe().unwrap();
        assert!(p.spawn(Command::new(me)).is_err());
        assert_eq!(p.live(), 0);
    }
}
