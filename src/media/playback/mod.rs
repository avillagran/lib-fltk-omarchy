//! Video and audio playback through the system `ffmpeg` and `ffprobe`
//! (feature `playback`): no native media library is linked; each stream
//! is one child process.
//!
//! - ffprobe reads duration, size, frame rate and the audio track.
//! - Video: ffmpeg decodes, scales to the size shown in device pixels and
//!   writes RGB24 frames to a pipe at the file's frame rate (≤ 60 fps);
//!   a reader thread holds each frame until it is due and keeps only the
//!   latest one for the UI ([`Video`]).
//! - Audio: a second ffmpeg converts to 48 kHz stereo and plays it through
//!   ffmpeg's output device where the OS has one (PulseAudio/PipeWire,
//!   Core Audio), or pipes PCM into a system player (`pw-play`, `paplay`,
//!   `aplay`, `ffplay`); without any, video plays silently with a notice.
//! - Processes start only while a player is shown and playing; pause,
//!   seek, hide, close and drop kill and reap them. They are tied to the
//!   app (Linux parent-death signal, Windows job object, exit hook), and
//!   [`App`](crate::App) stops them all when it ends.
//! - Only local files the app passes are opened: `file:` protocol only,
//!   no playlists, image sequences or URLs ([`Info`], docs/MEDIA.md).
//!
//! ```no_run
//! use fltk_omarchy::media::{self, Video};
//! use fltk_omarchy::layout::FlexExt;
//! # let mut body = fltk_omarchy::layout::column();
//! media::muted().set(false); // restore the app's saved global mute
//! let video = Video::new(640, 400).file("/home/me/clip.mp4").label("clip.mp4");
//! body.fit(&*video);
//! ```

mod args;
mod core;
mod probe;
mod proc;
mod widgets;

pub use probe::{parse_probe, AudioTrack, Info, VideoTrack};
pub use core::Stats;
pub use widgets::{Audio, Video};

use std::fmt;
use std::path::{Path, PathBuf};

/// Why a file cannot play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `ffmpeg` or `ffprobe` is not installed (or not found).
    MissingTools,
    /// Not a local regular file, or ffprobe/ffmpeg refused it.
    Unsupported(String),
    /// The file could not be read.
    Io(String),
    /// ffmpeg stopped with an error while playing.
    Failed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::MissingTools => write!(f, "ffmpeg and ffprobe are needed for playback"),
            Error::Unsupported(e) => write!(f, "cannot play this file: {e}"),
            Error::Io(e) => write!(f, "cannot read the file: {e}"),
            Error::Failed(e) => write!(f, "playback failed: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// What a player is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayState {
    /// No file.
    Empty,
    /// Probing the file (it starts when the player is first shown).
    Loading,
    Paused,
    Playing,
    /// Played to the end (Play starts over).
    Ended,
    /// Cannot play: tools missing, unsupported or unreadable file, or
    /// ffmpeg failed.
    Failed(Error),
}

/// Where `ffmpeg` and `ffprobe` are, if the app ships or knows them
/// (only that directory is used for them; audio players are looked up
/// there first). `None` (default) searches `PATH` and the usual package
/// manager directories.
pub fn set_tool_dir(dir: Option<PathBuf>) {
    proc::set_tool_dir(dir)
}

/// `Ok` when `ffmpeg` and `ffprobe` are found.
pub fn available() -> Result<(), Error> {
    match (proc::find_tool("ffmpeg"), proc::find_tool("ffprobe")) {
        (Some(_), Some(_)) => Ok(()),
        _ => Err(Error::MissingTools),
    }
}

/// Probes a file now (blocking: a worker thread or a CLI). Players do it
/// on their own.
pub fn probe(path: impl AsRef<Path>) -> Result<Info, Error> {
    proc::probe(path.as_ref())
}

thread_local! {
    static MUTED: crate::State<bool> = crate::State::new(false);
}

/// The global mute every player follows (M, the speaker button). Off at
/// start; the app restores and saves it: `media::muted().set(saved)`,
/// `media::muted().subscribe(|m| save(*m))`. UI thread only.
pub fn muted() -> crate::State<bool> {
    MUTED.with(|m| m.clone())
}

/// Kills and reaps every playback process (players show their last frame,
/// paused). [`App`](crate::App) calls it when it ends.
pub fn stop_all() {
    proc::stop_all()
}

/// Playback processes spawned and not reaped yet (tests, diagnostics).
pub fn live_processes() -> usize {
    proc::live_processes()
}
