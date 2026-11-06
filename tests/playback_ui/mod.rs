//! UI checks of `media::Video` / `media::Audio` (feature `playback`):
//! offscreen players on a clip generated with the system ffmpeg, keys
//! through `focus::press`, frames read back from snapshots, processes
//! counted after every stop. Skipped (with a note) when ffmpeg or
//! ffprobe is missing; the missing-tools state is checked either way.
//! Sound goes to the silent test output (`OMARCHY_FLTK_AUDIO_OUT=null`).
use super::{check, png_rgb};
use fltk::prelude::*;
use fltk_omarchy::focus::{self, press};
use fltk_omarchy::media::playback::{self, Error};
use fltk_omarchy::media::{self, Audio, PlayState, Video};
use fltk_omarchy::{theme, App, AppWindow};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("build/test-ui/playback")
}

/// A 6 s clip (320 × 180, 25 fps, test pattern + tone; mpeg4 and aac,
/// which every ffmpeg build can encode), made once per run.
fn clip() -> Option<PathBuf> {
    playback::available().ok()?;
    let path = root().join("clip.mp4");
    if path.is_file() {
        return Some(path);
    }
    std::fs::create_dir_all(root()).ok()?;
    let ok = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=6"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=6:sample_rate=48000", "-c:v", "mpeg4", "-q:v", "4", "-c:a", "aac", "-shortest"])
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success());
    ok.then_some(path)
}

/// Draws (offscreen) and pumps until `f` or `secs` pass: the player counts
/// as shown while it is being drawn.
fn play_until(win: &mut AppWindow, shot: &Path, secs: f64, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs_f64(secs);
    while Instant::now() < end {
        let _ = win.snapshot(shot);
        App::pump();
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    f()
}

/// Pumps without drawing (a hidden player).
fn pump_for(secs: f64, mut f: impl FnMut() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs_f64(secs);
    while Instant::now() < end {
        App::pump();
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    f()
}

/// Children of this process still alive (Linux: /proc), besides the
/// toolkit's own count.
fn os_children() -> usize {
    let me = std::process::id().to_string();
    let Ok(dir) = std::fs::read_dir("/proc") else { return 0 };
    dir.flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("stat")).ok())
        .filter(|s| {
            let comm_end = s.rfind(')').unwrap_or(0);
            let rest: Vec<&str> = s[comm_end + 1..].split_whitespace().collect();
            rest.get(1) == Some(&me.as_str()) && (s.contains("(ffmpeg)") || s.contains("(ffprobe)"))
        })
        .count()
}

fn no_processes(what: &str) -> Result<(), String> {
    check(pump_for(2.0, || media::playback::live_processes() == 0 && os_children() == 0), &format!("{what}: processes left: {} (os {})", media::playback::live_processes(), os_children()))
}

/// A window with room for `h` units of body below the header.
fn window(w: i32, h: i32) -> AppWindow {
    let win = AppWindow::new("playback", w, h + 80);
    let _ = win.body();
    win
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    // Never make a sound on the machine running the tests.
    std::env::set_var("OMARCHY_FLTK_AUDIO_OUT", "null");
    run("playback: without ffmpeg the player says so and Play is disabled (theme colors)", &|shots| {
        let empty = root().join("no-tools");
        std::fs::create_dir_all(&empty).map_err(|e| e.to_string())?;
        playback::set_tool_dir(Some(empty));
        let mut win = window(360, 260);
        let mut body = win.body();
        let fake = root().join("missing-tools.mp4");
        std::fs::write(&fake, b"x").map_err(|e| e.to_string())?;
        let v = Video::new(340, 230).file(&fake);
        body.fixed(&*v, 230);
        win.end();
        let shot = shots.join("playback-no-ffmpeg.png");
        let failed = play_until(&mut win, &shot, 3.0, || matches!(v.state(), PlayState::Failed(Error::MissingTools)));
        playback::set_tool_dir(None);
        check(failed, &format!("state {:?}", v.state()))?;
        win.snapshot(&shot).map_err(|e| e.to_string())?;
        check(!v.play_button().active(), "play disabled")?;
        check(!v.seek_bar().active(), "slider disabled")?;
        // The film icon and the message in the urgent color.
        let img = png_rgb(&shot)?;
        let urgent = theme::with(|t| t.palette.urgent);
        let want = [urgent.r(), urgent.g(), urgent.b()];
        let (sx, sy, sw, sh) = (v.x(), v.y(), v.w(), v.h() - 40);
        let mut found = 0;
        for y in sy..sy + sh {
            for x in sx..sx + sw {
                let p = img.at(x, y);
                if p.iter().zip(want).all(|(a, b)| (*a as i32 - b as i32).abs() < 40) {
                    found += 1;
                }
            }
        }
        check(found > 20, &format!("urgent pixels {found}"))?;
        v.toggle();
        check(matches!(v.state(), PlayState::Failed(_)), "Play does nothing")?;
        no_processes("missing tools")
    });

    let Some(_) = clip() else {
        println!("playback checks skipped: no ffmpeg/ffprobe");
        return;
    };

    run("playback: video probes when shown, autoplays, paces frames, pauses with Space and reaps ffmpeg", &|shots| {
        let path = clip().ok_or("clip")?;
        let mut win = window(420, 320);
        let mut body = win.body();
        let v = Video::new(400, 300).file(&path).label("clip.mp4");
        body.fixed(&*v, 300);
        win.end();
        check(v.state() == PlayState::Loading && media::playback::live_processes() == 0, "nothing runs before it is shown")?;
        let shot = shots.join("playback-video.png");
        check(play_until(&mut win, &shot, 5.0, || v.stats().frames >= 12), &format!("frames {:?} state {:?}", v.stats(), v.state()))?;
        let info = v.info().ok_or("info")?;
        check((info.duration.unwrap_or(0.0) - 6.0).abs() < 0.2 && info.has_audio(), &format!("{info:?}"))?;
        check(v.state() == PlayState::Playing, &format!("autoplay: {:?}", v.state()))?;
        let st = v.stats();
        check(st.first_frame.is_some_and(|d| d < Duration::from_secs(2)), &format!("first frame {:?}", st.first_frame))?;
        check(st.audio_out == Some("null"), &format!("silent test output: {:?}", st.audio_out))?;
        // Frames at the shown size in device pixels (never above the source).
        check(st.frame_size.0 <= 320 && st.frame_size.0 >= 200 && st.frame_size.0 % 2 == 0, &format!("frame size {:?}", st.frame_size))?;
        // Paced: the frame on screen follows the clock (25 fps from ffmpeg,
        // not as fast as it decodes); the UI takes the latest frame.
        let (f0, t0, p0, q0) = (v.stats().frames, Instant::now(), v.position(), v.frame_position().unwrap_or(0.0));
        play_until(&mut win, &shot, 1.0, || false);
        let (df, dt, dp) = (v.stats().frames - f0, t0.elapsed().as_secs_f64(), v.position() - p0);
        let dq = v.frame_position().unwrap_or(0.0) - q0;
        check(df as f64 / dt > 4.0 && df as f64 / dt < 32.0, &format!("{df} frames in {dt:.2} s"))?;
        check((dp - dt).abs() < 0.4, &format!("position moved {dp:.2} s in {dt:.2} s"))?;
        check((dq - dt).abs() < 0.4, &format!("frames moved {dq:.2} s in {dt:.2} s"))?;
        check(media::playback::live_processes() == 2, &format!("video + audio: {}", media::playback::live_processes()))?;
        // The picture is drawn: the test pattern's colors are in the surface.
        win.snapshot(&shot).map_err(|e| e.to_string())?;
        let img = png_rgb(&shot)?;
        let bg = theme::with(|t| t.palette.darker_background);
        let mut colorful = 0;
        for y in (v.y() + 20..v.y() + 240).step_by(7) {
            for x in (v.x() + 20..v.x() + 380).step_by(7) {
                let p = img.at(x, y);
                let spread = p.iter().max().unwrap() - p.iter().min().unwrap();
                if spread > 120 && p != [bg.r(), bg.g(), bg.b()] {
                    colorful += 1;
                }
            }
        }
        check(colorful > 50, &format!("video pixels {colorful}"))?;
        // Space on the play button pauses; processes are killed and reaped.
        v.focus();
        check(press("space"), "space used")?;
        check(v.state() == PlayState::Paused, &format!("paused: {:?}", v.state()))?;
        no_processes("paused")?;
        let p = v.position();
        play_until(&mut win, &shot, 0.4, || false);
        check((v.position() - p).abs() < 1e-6, "position holds while paused")?;
        // K plays again.
        check(press("k"), "k used")?;
        check(play_until(&mut win, &shot, 3.0, || v.stats().segments >= 2 && media::playback::live_processes() == 2), "k resumes")?;
        v.pause();
        no_processes("paused again")
    });

    run("playback: seeking (Right/Left ±5 s, Home, End, slider) shows the frame there; M toggles the global mute", &|shots| {
        let path = clip().ok_or("clip")?;
        let mut win = window(420, 320);
        let mut body = win.body();
        let v = Video::new(400, 300).file(&path).autoplay(false);
        body.fixed(&*v, 300);
        win.end();
        let shot = shots.join("playback-seek.png");
        check(play_until(&mut win, &shot, 5.0, || v.state() == PlayState::Paused && v.frame_position().is_some()), &format!("poster {:?}", v.state()))?;
        check(v.frame_position() == Some(0.0), &format!("poster at 0: {:?}", v.frame_position()))?;
        no_processes("after the poster")?;
        let at = |want: f64| {
            let v = v.clone();
            move || v.frame_position().is_some_and(|p| (p - want).abs() < 0.05)
        };
        focus::press("Tab"); // into the player
        v.focus();
        check(press("Right"), "right used")?;
        check((v.position() - 5.0).abs() < 1e-6, &format!("position {}", v.position()))?;
        check(play_until(&mut win, &shot, 3.0, at(5.0)), &format!("frame at 5: {:?}", v.frame_position()))?;
        check(press("Left"), "left used")?;
        check(play_until(&mut win, &shot, 3.0, at(0.0)), &format!("frame at 0: {:?}", v.frame_position()))?;
        check(press("End"), "end used")?;
        check(v.position() == 6.0, &format!("end: {}", v.position()))?;
        check(press("Home"), "home used")?;
        check(v.position() == 0.0, "home")?;
        // On the slider: Right seeks too; a click seeks where it lands.
        let mut seek = v.seek_bar();
        let _ = seek.take_focus();
        check(press("Right"), "slider right")?;
        check((v.position() - 5.0).abs() < 1e-6, &format!("slider position {}", v.position()))?;
        let (sx, sy, sw, sh) = (seek.x(), seek.y(), seek.w(), seek.h());
        focus::mouse_drag(&*win, (sx + sw / 2, sy + sh / 2), (sx + sw / 2, sy + sh / 2), true);
        check((v.position() - 3.0).abs() < 0.5, &format!("click at the middle: {}", v.position()))?;
        check(play_until(&mut win, &shot, 3.0, || v.frame_position().is_some_and(|p| (p - v.position()).abs() < 0.05)), "frame there")?;
        // Seeking while playing restarts ffmpeg once the keys settle.
        v.play();
        check(play_until(&mut win, &shot, 3.0, || v.stats().frames > 3 && v.state() == PlayState::Playing), "playing")?;
        let segs = v.stats().segments;
        v.focus();
        for _ in 0..3 {
            press("Left");
        }
        check(play_until(&mut win, &shot, 3.0, || v.stats().segments == segs + 1 && v.frame_position().is_some_and(|p| p < 1.0)), &format!("one restart for three keys: {:?}", v.stats()))?;
        // M: the global mute (persisted by the app) stops and restarts the sound.
        check(!media::muted().get(), "unmuted at start")?;
        check(press("m"), "m used")?;
        check(media::muted().get(), "muted")?;
        check(pump_for(1.0, || media::playback::live_processes() == 1), &format!("video only: {}", media::playback::live_processes()))?;
        check(press("m"), "m again")?;
        check(!media::muted().get() && pump_for(1.0, || media::playback::live_processes() == 2), "sound back")?;
        v.stop();
        no_processes("seek checks")
    });

    run("playback: plays to the end (Ended, Play starts over); hidden, deleted and stop_all players end their processes", &|shots| {
        let path = clip().ok_or("clip")?;
        let mut win = window(420, 320);
        let mut body = win.body();
        let v = Video::new(400, 300).file(&path).autoplay(false);
        body.fixed(&*v, 300);
        win.end();
        let shot = shots.join("playback-end.png");
        check(play_until(&mut win, &shot, 5.0, || v.state() == PlayState::Paused), "probed")?;
        v.seek(5.2);
        v.play();
        check(play_until(&mut win, &shot, 4.0, || v.state() == PlayState::Ended), &format!("ended: {:?}", v.state()))?;
        check(v.position() == 6.0, &format!("at the end: {}", v.position()))?;
        no_processes("ended")?;
        v.play();
        check(play_until(&mut win, &shot, 3.0, || v.frame_position().is_some_and(|p| p < 1.0) && v.state() == PlayState::Playing), "starts over")?;
        // Not drawn any more (window hidden, scrolled away): it pauses.
        check(pump_for(3.0, || v.state() == PlayState::Paused), &format!("hidden: {:?}", v.state()))?;
        no_processes("hidden")?;
        // stop_all (App::drop calls it): processes end, the player pauses.
        v.play();
        check(play_until(&mut win, &shot, 3.0, || media::playback::live_processes() == 2), "playing again")?;
        media::playback::stop_all();
        check(media::playback::live_processes() == 0, "stop_all reaps at once")?;
        check(play_until(&mut win, &shot, 1.0, || v.state() == PlayState::Paused), "paused after stop_all")?;
        // Deleted while playing: dropping the widget kills and reaps.
        v.play();
        check(play_until(&mut win, &shot, 3.0, || media::playback::live_processes() == 2), "playing before delete")?;
        drop(v);
        fltk::window::DoubleWindow::delete((*win).clone());
        no_processes("deleted")
    });

    run("playback: audio player (no autoplay), plays to the end; no audio output gives a notice and silent video", &|shots| {
        let path = clip().ok_or("clip")?;
        let mut win = window(420, 160);
        let mut body = win.body();
        let a = Audio::new(400).file(&path).label("clip.mp4");
        use fltk_omarchy::layout::FlexExt;
        body.fit(&*a);
        win.end();
        let shot = shots.join("playback-audio.png");
        check(play_until(&mut win, &shot, 4.0, || a.state() == PlayState::Paused), &format!("probed: {:?}", a.state()))?;
        check(media::playback::live_processes() == 0, "no autoplay")?;
        a.seek(4.5);
        a.focus();
        check(press("space"), "space")?;
        check(a.state() == PlayState::Playing && pump_for(1.0, || media::playback::live_processes() == 1), &format!("one ffmpeg (sound only): {:?} {}", a.state(), media::playback::live_processes()))?;
        check(play_until(&mut win, &shot, 4.0, || a.state() == PlayState::Ended), &format!("ended: {:?} at {}", a.state(), a.position()))?;
        no_processes("audio ended")?;
        // No output at all: the video plays without sound and says so.
        std::env::set_var("OMARCHY_FLTK_AUDIO_OUT", "none");
        let mut win2 = window(420, 320);
        let mut body2 = win2.body();
        let v = Video::new(400, 300).file(&path);
        body2.fixed(&*v, 300);
        win2.end();
        let shot2 = shots.join("playback-no-audio.png");
        let ok = play_until(&mut win2, &shot2, 5.0, || v.stats().frames > 5);
        std::env::set_var("OMARCHY_FLTK_AUDIO_OUT", "null");
        check(ok && v.without_sound() && media::playback::live_processes() == 1, &format!("silent video: {:?} notice {}", v.stats(), v.without_sound()))?;
        v.stop();
        no_processes("silent video")
    });

    run("playback: a larger player restarts the stream at the larger size", &|shots| {
        let path = clip().ok_or("clip")?;
        let mut win = window(220, 200);
        let mut body = win.body();
        let v = Video::new(200, 160).file(&path);
        body.fixed(&*v, 160);
        win.end();
        let shot = shots.join("playback-resize.png");
        check(play_until(&mut win, &shot, 5.0, || v.stats().frames > 3), "playing small")?;
        let small = v.stats().frame_size;
        check(small.0 < 220, &format!("small {small:?}"))?;
        win.resize(0, 0, 420, 440);
        body.fixed(&*v, 320);
        check(play_until(&mut win, &shot, 4.0, || v.stats().frame_size.0 > small.0 && v.stats().frames > 0), &format!("larger {:?}", v.stats()))?;
        check(v.stats().frame_size.0 <= 320, "never above the source")?;
        v.stop();
        no_processes("resize")
    });
}

#[cfg(feature = "a11y")]
pub fn a11y_checks(run: &mut dyn FnMut(&str, Check)) {
    use fltk_omarchy::a11y;
    run("a11y: players expose named buttons, a position slider with the time, the state", &|_| {
        let mut win = window(420, 380);
        let mut body = win.body();
        let fake = root().join("a11y.mp4");
        let _ = std::fs::create_dir_all(root());
        std::fs::write(&fake, b"x").map_err(|e| e.to_string())?;
        let v = Video::new(400, 260).label("clip.mp4");
        body.fixed(&*v, 260);
        let a = Audio::new(400).label("song.ogg");
        use fltk_omarchy::layout::FlexExt;
        body.fit(&*a);
        win.end();
        let snap = a11y::snapshot(&*win).ok_or("no tree")?;
        for want in ["Group \"clip.mp4\"", "Button \"Play\"", "Slider \"Playback position\"", "Button \"Mute\"", "Group \"song.ogg\""] {
            check(snap.contains(want), &format!("has {want:?}:\n{snap}"))?;
        }
        if let Some(path) = clip() {
            let mut v2 = v.clone();
            v2.set_file(Some(&path));
            let shot = root().join("a11y.png");
            check(play_until(&mut win, &shot, 5.0, || v.stats().frames > 2), "playing")?;
            let snap = a11y::snapshot(&*win).ok_or("no tree")?;
            check(snap.contains("Button \"Pause\"") && snap.contains("of 0:06"), &format!("playing:\n{snap}"))?;
            media::muted().set(true);
            let snap = a11y::snapshot(&*win).ok_or("no tree")?;
            media::muted().set(false);
            check(snap.contains("Button \"Unmute\""), &format!("muted:\n{snap}"))?;
            v.stop();
        }
        Ok(())
    });
}
