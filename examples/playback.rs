//! Video and audio through the system ffmpeg: a video player over an
//! audio player (o3's attachment row). Space/K play/pause, Left/Right
//! ±5 s, Home/End, M mute (global); Tab walks the controls.
//!   cargo run --example playback --features playback -- clip.mp4 [song.ogg]
//!   cargo run --example playback --features playback -- --snapshot build/shots
//! Without files it generates a 6 s test clip with ffmpeg (into the
//! system temp directory). Without ffmpeg the players say so.
use fltk_omarchy::layout::FlexExt;
use fltk_omarchy::media::{self, Audio, PlayState, Video};
use fltk_omarchy::widgets::*;
use fltk_omarchy::{App, AppWindow};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A small clip with a moving test pattern and a tone (cached in the temp dir).
fn test_clip() -> Option<PathBuf> {
    let path = std::env::temp_dir().join("fltk-omarchy-playback-demo.mp4");
    if path.is_file() {
        return Some(path);
    }
    let ok = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=6"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=6:sample_rate=48000", "-c:v", "libx264", "-preset", "veryfast"])
        .args(["-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success());
    ok.then_some(path)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let snapshot = args.iter().skip_while(|a| *a != "--snapshot").nth(1).cloned();
    let files: Vec<String> = args.iter().take_while(|a| *a != "--snapshot").cloned().collect();
    let app = App::new("dev.omarchy.fltk.playback");
    let clip = files.first().map(PathBuf::from).or_else(test_clip);
    let song = files.get(1).map(PathBuf::from).or_else(|| clip.clone());

    let mut win = AppWindow::new("Video and audio", 680, 620);
    let mut body = win.body();
    body.fit(&*Label::section("Video"));
    let mut video = Video::new(640, 400).label("Video clip");
    if let Some(c) = &clip {
        video.set_file(Some(c));
    }
    body.fixed(&*video, 400);
    body.fit(&*Label::section("Audio"));
    let mut audio = Audio::new(640).label(&song.as_ref().and_then(|s| s.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    if let Some(s) = &song {
        audio.set_file(Some(s));
    }
    body.fit(&*audio);
    if let Err(e) = media::playback::available() {
        body.fit(&*Label::new(&e.to_string()));
    }
    win.end();

    if let Some(dir) = snapshot {
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        fltk_omarchy::anim::set_enabled(false);
        // Offscreen: draw until the first frames are in, then pause on one.
        let video2 = video.clone();
        let end = Instant::now() + Duration::from_secs(8);
        while Instant::now() < end && video2.stats().frames < 30 && !matches!(video2.state(), PlayState::Failed(_)) {
            let _ = win.snapshot(dir.join("playback.png"));
            App::pump();
            std::thread::sleep(Duration::from_millis(30));
        }
        video.pause();
        video.seek(2.0);
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end && video.frame_position().map_or(true, |p| (p - 2.0).abs() > 0.1) {
            App::pump();
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = win.snapshot(dir.join("playback.png"));
        video.stop();
        println!("wrote {} (frames {}, first frame {:?})", dir.join("playback.png").display(), video.stats().frames, video.stats().first_frame);
        return;
    }
    win.show();
    video.focus();
    app.run();
}
