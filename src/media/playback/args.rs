//! ffmpeg command lines, output sizes and the pacing clock (pure code,
//! unit-tested without ffmpeg).

use crate::platform::playback::AudioOut;
use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

/// Bytes of ffprobe JSON read at most.
pub(crate) const MAX_PROBE_BYTES: usize = 64 * 1024;
/// Largest frame the player asks ffmpeg for (4K UHD).
pub(crate) const MAX_SIDE: u32 = 3840;
pub(crate) const MAX_PIXELS: u64 = 3840 * 2160;
/// Highest frame rate shown; faster sources are resampled to it.
pub(crate) const MAX_RATE: u32 = 60;

/// Demuxers ffmpeg may pick: self-contained media containers only. No
/// playlists (hls, dash, concat), image sequences, network or device
/// inputs, so a file cannot make ffmpeg open other files or URLs.
pub(crate) const FORMATS: &str = "mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,avi,mpegts,mpeg,mpegps,ogg,mp3,wav,flac,aac,aiff,gif,flv,asf,caf,w64,wv,ivf";

/// Input options every ffmpeg/ffprobe run gets: only the `file` protocol
/// and the demuxers in [`FORMATS`].
pub(crate) fn push_input_guards(a: &mut Vec<OsString>) {
    for s in ["-protocol_whitelist", "file", "-format_whitelist", FORMATS] {
        a.push(s.into());
    }
}

/// `file:<path>`: ffmpeg reads the path literally (no protocol or
/// pattern in the name is interpreted).
pub(crate) fn file_url(path: &Path) -> OsString {
    let mut s = OsString::from("file:");
    s.push(path.as_os_str());
    s
}

/// Seconds with microsecond precision, as ffmpeg takes them.
fn secs(t: f64) -> String {
    format!("{:.6}", t.max(0.0))
}

/// Output frame rate: the declared rate up to 60 fps, 60 for faster
/// sources, 30 when unknown.
pub(crate) fn output_rate(rate: Option<(u32, u32)>) -> (u32, u32) {
    match rate {
        Some((n, d)) if n > 0 && d > 0 && n as u64 <= d as u64 * MAX_RATE as u64 => (n, d),
        Some(_) => (MAX_RATE, 1),
        None => (30, 1),
    }
}

/// Frame size for a source shown (aspect fit) in a box of device pixels:
/// never larger than the source, never larger than 4K, even sides (what
/// the scaler and YUV sources like), at least 2 × 2.
pub(crate) fn output_size(display: (u32, u32), box_px: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = (display.0.max(1) as f64, display.1.max(1) as f64);
    let (bw, bh) = (box_px.0.max(2) as f64, box_px.1.max(2) as f64);
    let mut k = (bw / sw).min(bh / sh).min(1.0);
    k = k.min(MAX_SIDE as f64 / sw).min(MAX_SIDE as f64 / sh);
    let area = sw * sh * k * k;
    if area > MAX_PIXELS as f64 {
        k *= (MAX_PIXELS as f64 / area).sqrt();
    }
    let even = |v: f64| ((v.floor() as u32) & !1).max(2);
    (even(sw * k), even(sh * k))
}

/// Threads for one stream: decoder threads by the source size (frame
/// threads each hold frames: +8 MB per thread at 1080p), scaler threads by
/// the output size. Measured on a 1080p H.264 clip: one decoder thread
/// keeps up at 18 % of a core; ffmpeg's default (one thread per core)
/// only adds memory.
pub(crate) fn threads(source: (u32, u32), output: (u32, u32)) -> (u32, u32) {
    let cores = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1);
    let px = |s: (u32, u32)| s.0 as u64 * s.1 as u64;
    let decode = if px(source) > 1920 * 1088 { 4 } else { 2 };
    let filter = if px(output) > 1280 * 720 { 2 } else { 1 };
    (decode.min(cores).max(1), filter.min(cores).max(1))
}

/// One video stream of a `source`-sized track: RGB24 frames of `size` on
/// stdout at `rate`, from `start` seconds; `single` stops after one frame
/// (a poster or a paused seek).
pub(crate) fn video_args(path: &Path, start: f64, source: (u32, u32), size: (u32, u32), rate: (u32, u32), single: bool) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::with_capacity(40);
    for s in ["-hide_banner", "-nostdin", "-nostats", "-loglevel", "error", "-max_alloc", "268435456"] {
        a.push(s.into());
    }
    push_input_guards(&mut a);
    let (decode, filter) = threads(source, size);
    let (decode, filter) = (decode.to_string(), filter.to_string());
    for s in ["-threads", decode.as_str(), "-hwaccel", "none"] {
        a.push(s.into());
    }
    if start > 0.0 {
        a.push("-ss".into());
        a.push(secs(start).into());
    }
    a.push("-i".into());
    a.push(file_url(path));
    for s in ["-map", "0:v:0", "-an", "-sn", "-dn"] {
        a.push(s.into());
    }
    let scale = format!("scale={}:{}:flags=bicubic,setsar=1", size.0, size.1);
    a.push("-filter_threads".into());
    a.push(filter.into());
    a.push("-vf".into());
    a.push(if single { scale } else { format!("fps={}/{},{scale}", rate.0, rate.1) }.into());
    if single {
        a.push("-frames:v".into());
        a.push("1".into());
    }
    // A short muxer queue: the reader paces the pipe, and every queued
    // native-size frame is MBs in ffmpeg (1080p: 190 → 142 MB PSS).
    for s in ["-pix_fmt", "rgb24", "-thread_queue_size", "2", "-f", "rawvideo", "pipe:1"] {
        a.push(s.into());
    }
    a
}

/// One audio stream from `start`: decoded, converted to s16 48 kHz
/// stereo and sent to `out` (an ffmpeg device), to stdout for a player
/// (`Player`), or discarded at playback speed (`None`, tests).
pub(crate) fn audio_args(path: &Path, start: f64, out: Option<&AudioOut>, name: &str) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::with_capacity(40);
    for s in ["-hide_banner", "-nostdin", "-nostats", "-loglevel", "error", "-max_alloc", "268435456", "-threads", "1"] {
        a.push(s.into());
    }
    push_input_guards(&mut a);
    // Read at playback speed: device outputs then never run ahead, and a
    // discarded stream ends when the sound would have.
    a.push("-re".into());
    if start > 0.0 {
        a.push("-ss".into());
        a.push(secs(start).into());
    }
    a.push("-i".into());
    a.push(file_url(path));
    for s in ["-map", "0:a:0", "-vn", "-sn", "-dn", "-af", "aformat=sample_fmts=s16:sample_rates=48000:channel_layouts=stereo"] {
        a.push(s.into());
    }
    match out {
        Some(AudioOut::Device { format, device }) => {
            for s in ["-c:a", "pcm_s16le", "-f", format] {
                a.push(s.into());
            }
            if *format == "pulse" {
                let name: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
                for s in ["-buffer_duration", "100", "-name", name.as_str(), "-stream_name", name.as_str()] {
                    a.push(s.into());
                }
            }
            a.push(device.into());
        }
        Some(AudioOut::Player { .. }) => {
            for s in ["-c:a", "pcm_s16le", "-f", "s16le", "pipe:1"] {
                a.push(s.into());
            }
        }
        None => {
            for s in ["-f", "null", "-"] {
                a.push(s.into());
            }
        }
    }
    a
}

/// Frame timing of one stream: frame `seq` shows `seq / rate` after the
/// first one, at position `start + seq / rate`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pacer {
    pub start: f64,
    pub rate: (u32, u32),
}

impl Pacer {
    pub(crate) fn offset(&self, seq: u64) -> Duration {
        Duration::from_nanos((seq as u128 * self.rate.1 as u128 * 1_000_000_000 / self.rate.0.max(1) as u128) as u64)
    }

    /// Media position of frame `seq`.
    pub(crate) fn pts(&self, seq: u64) -> f64 {
        self.start + self.offset(seq).as_secs_f64()
    }

    /// How long to wait before showing frame `seq` (the first frame came
    /// at `t0`); zero when it is due or late.
    pub(crate) fn wait(&self, t0: Instant, seq: u64, now: Instant) -> Duration {
        (t0 + self.offset(seq)).saturating_duration_since(now)
    }
}

/// The playback position: `base` while paused; while playing, `base`
/// plus the time since `origin` (the first frame), within the duration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Clock {
    pub base: f64,
    pub origin: Option<Instant>,
    pub duration: Option<f64>,
}

impl Clock {
    pub(crate) fn position(&self, now: Instant) -> f64 {
        let p = match self.origin {
            Some(o) => self.base + now.saturating_duration_since(o).as_secs_f64(),
            None => self.base,
        };
        let p = p.max(0.0);
        match self.duration {
            Some(d) => p.min(d),
            None => p,
        }
    }

    /// Stops at the current position.
    pub(crate) fn freeze(&mut self, now: Instant) {
        self.base = self.position(now);
        self.origin = None;
    }
}

/// `m:ss`, or `h:mm:ss` from an hour on.
pub(crate) fn format_time(t: f64) -> String {
    let s = if t.is_finite() { t.max(0.0).floor() as u64 } else { 0 };
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(a: &[OsString]) -> Vec<String> {
        a.iter().map(|s| s.to_string_lossy().into_owned()).collect()
    }

    fn has(a: &[String], pair: [&str; 2]) -> bool {
        a.windows(2).any(|w| w[0] == pair[0] && w[1] == pair[1])
    }

    #[test]
    fn video_command() {
        let a = strs(&video_args(Path::new("/m/clip one.mp4"), 12.5, (1920, 1080), (640, 360), (30000, 1001), false));
        assert!(has(&a, ["-protocol_whitelist", "file"]));
        assert!(has(&a, ["-format_whitelist", FORMATS]));
        assert!(has(&a, ["-ss", "12.500000"]));
        assert!(has(&a, ["-i", "file:/m/clip one.mp4"]), "{a:?}");
        assert!(has(&a, ["-vf", "fps=30000/1001,scale=640:360:flags=bicubic,setsar=1"]));
        assert!(has(&a, ["-pix_fmt", "rgb24"]));
        assert!(has(&a, ["-map", "0:v:0"]));
        assert!(has(&a, ["-filter_threads", "1"]), "small output: one scaler thread");
        assert!(a.contains(&"-an".to_string()) && a.contains(&"-nostdin".to_string()));
        assert_eq!(a.last().map(String::as_str), Some("pipe:1"));
        let ss = a.iter().position(|s| s == "-ss").unwrap();
        let i = a.iter().position(|s| s == "-i").unwrap();
        assert!(ss < i, "input seek (fast, then exact)");
        let single = strs(&video_args(Path::new("/m/a.mkv"), 0.0, (2, 2), (2, 2), (30, 1), true));
        assert!(!single.contains(&"-ss".to_string()), "no seek at 0");
        assert!(has(&single, ["-frames:v", "1"]));
        assert!(has(&single, ["-vf", "scale=2:2:flags=bicubic,setsar=1"]));
    }

    #[test]
    fn no_urls_or_playlists() {
        for f in ["hls", "concat", "image2", "dash", "http", "rtsp", "sdp", "tee"] {
            assert!(!FORMATS.split(',').any(|x| x == f), "{f}");
        }
        // A name that looks like a URL stays a file name.
        let a = strs(&video_args(Path::new("http://example.com/x.mp4"), 0.0, (2, 2), (2, 2), (30, 1), true));
        assert!(has(&a, ["-i", "file:http://example.com/x.mp4"]));
    }

    #[test]
    fn audio_commands_per_output() {
        let p = Path::new("/m/a.ogg");
        let pulse = AudioOut::Device { format: "pulse", device: "default" };
        let a = strs(&audio_args(p, 3.0, Some(&pulse), "Demo\napp"));
        assert!(has(&a, ["-f", "pulse"]) && has(&a, ["-name", "Demoapp"]));
        assert_eq!(a.last().map(String::as_str), Some("default"));
        assert!(has(&a, ["-map", "0:a:0"]) && a.contains(&"-re".to_string()) && has(&a, ["-ss", "3.000000"]));
        let player = AudioOut::Player { program: "pw-play", args: &[] };
        let a = strs(&audio_args(p, 0.0, Some(&player), "x"));
        assert!(has(&a, ["-f", "s16le"]) && a.last().unwrap() == "pipe:1");
        assert!(a.iter().any(|s| s.contains("sample_rates=48000") && s.contains("stereo")));
        let a = strs(&audio_args(p, 0.0, None, "x"));
        assert!(has(&a, ["-f", "null"]));
    }

    #[test]
    fn sizes() {
        // Fit in the box, keep the aspect, never upscale, even sides.
        assert_eq!(output_size((1920, 1080), (640, 360)), (640, 360));
        assert_eq!(output_size((1920, 1080), (1280, 1280)), (1280, 720));
        assert_eq!(output_size((1920, 1080), (4000, 4000)), (1920, 1080));
        assert_eq!(output_size((1080, 1920), (640, 360)), (202, 360));
        assert_eq!(output_size((7680, 4320), (8000, 8000)), (3840, 2160), "4K cap");
        assert_eq!(output_size((101, 51), (1000, 1000)), (100, 50), "even sides");
        assert_eq!(output_size((1, 1), (0, 0)), (2, 2));
        let (w, h) = output_size((8000, 8000), (9000, 9000));
        assert!((w as u64 * h as u64) <= MAX_PIXELS && w <= MAX_SIDE);
    }

    #[test]
    fn thread_counts() {
        let cores = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(1);
        assert_eq!(threads((1920, 1080), (640, 360)), (2.min(cores), 1));
        assert_eq!(threads((3840, 2160), (3840, 2160)), (4.min(cores), 2.min(cores)));
    }

    #[test]
    fn rates() {
        assert_eq!(output_rate(None), (30, 1));
        assert_eq!(output_rate(Some((24000, 1001))), (24000, 1001));
        assert_eq!(output_rate(Some((60, 1))), (60, 1));
        assert_eq!(output_rate(Some((120, 1))), (60, 1));
        assert_eq!(output_rate(Some((0, 1))), (60, 1));
    }

    #[test]
    fn pacing() {
        let p = Pacer { start: 10.0, rate: (30000, 1001) };
        assert_eq!(p.offset(0), Duration::ZERO);
        assert_eq!(p.offset(30), Duration::from_nanos(1_001_000_000));
        assert!((p.pts(300) - 20.01).abs() < 1e-9);
        let t0 = Instant::now();
        let q = Pacer { start: 0.0, rate: (25, 1) };
        assert_eq!(q.wait(t0, 1, t0), Duration::from_millis(40));
        assert_eq!(q.wait(t0, 1, t0 + Duration::from_millis(15)), Duration::from_millis(25));
        assert_eq!(q.wait(t0, 1, t0 + Duration::from_millis(90)), Duration::ZERO, "late: now");
        // No drift over an hour at 60 fps (integer nanoseconds per frame index).
        let r = Pacer { start: 0.0, rate: (60, 1) };
        assert_eq!(r.offset(216_000), Duration::from_secs(3600));
    }

    #[test]
    fn clock() {
        let t0 = Instant::now();
        let mut c = Clock { base: 5.0, origin: None, duration: Some(8.0) };
        assert_eq!(c.position(t0 + Duration::from_secs(9)), 5.0, "paused");
        c.origin = Some(t0);
        assert!((c.position(t0 + Duration::from_millis(1500)) - 6.5).abs() < 1e-9);
        assert_eq!(c.position(t0 + Duration::from_secs(60)), 8.0, "clamped to the duration");
        c.freeze(t0 + Duration::from_secs(1));
        assert_eq!((c.base, c.origin), (6.0, None));
        let open = Clock { base: 1.0, origin: Some(t0), duration: None };
        assert!((open.position(t0 + Duration::from_secs(100)) - 101.0).abs() < 1e-9);
    }

    #[test]
    fn times() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(65.9), "1:05");
        assert_eq!(format_time(3600.0 + 61.0), "1:01:01");
        assert_eq!(format_time(f64::NAN), "0:00");
        assert_eq!(format_time(-3.0), "0:00");
    }
}
