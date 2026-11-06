//! CPU, memory and startup of `media::Video` playing a file in a shown
//! window, as one JSON line (Linux diagnostic: reads /proc).
//!   cargo run --release --example playback_bench --features playback -- clip.mp4 --box 640x360 --seconds 10
//! `--out FILE` also writes the line there. `--box WxH` is the picture
//! size in units (the window adds the
//! controls). Sound goes to the silent output unless OMARCHY_FLTK_AUDIO_OUT
//! says otherwise. Reports: time from Play to the first frame, frames
//! shown per second, CPU of the app and of its ffmpeg children (% of one
//! core), PSS of the app and of the children, and whether every child was
//! killed and reaped after `stop()`.
use fltk::{app, prelude::*};
use fltk_omarchy::media::{self, PlayState, Video};
use fltk_omarchy::App;
use std::time::{Duration, Instant};

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().skip_while(|a| *a != name).nth(1).cloned()
}

/// (pid, utime + stime in clock ticks) of `pid`.
fn ticks(pid: u32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest: Vec<&str> = s[s.rfind(')')? + 1..].split_whitespace().collect();
    Some(rest.get(11)?.parse::<u64>().ok()? + rest.get(12)?.parse::<u64>().ok()?)
}

fn pss_kib(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup"))
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("Pss:"))?.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(0)
}

/// Direct children of this process (pid, name).
fn children() -> Vec<(u32, String)> {
    let me = std::process::id().to_string();
    let mut v = Vec::new();
    for e in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
        let Ok(s) = std::fs::read_to_string(e.path().join("stat")) else { continue };
        let (Some(open), Some(close)) = (s.find('('), s.rfind(')')) else { continue };
        let rest: Vec<&str> = s[close + 1..].split_whitespace().collect();
        if rest.get(1) == Some(&me.as_str()) {
            if let Ok(pid) = e.file_name().to_string_lossy().parse() {
                v.push((pid, s[open + 1..close].to_string()));
            }
        }
    }
    v
}

/// Runs the event loop for up to `t` seconds and the posted work.
fn wait(t: f64) {
    let _ = app::wait_for(t);
    fltk_omarchy::task::drain();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(file) = args.first().cloned() else {
        eprintln!("usage: playback_bench <video> [--box WxH] [--seconds N]");
        std::process::exit(2);
    };
    let (bw, bh) = arg(&args, "--box")
        .and_then(|s| s.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?))))
        .unwrap_or((640, 360));
    let seconds: f64 = arg(&args, "--seconds").and_then(|s| s.parse().ok()).unwrap_or(10.0);
    if std::env::var_os("OMARCHY_FLTK_AUDIO_OUT").is_none() {
        std::env::set_var("OMARCHY_FLTK_AUDIO_OUT", "null");
    }
    let app = App::new("dev.omarchy.fltk.playback-bench");
    fltk_omarchy::anim::set_enabled(false);
    let controls = 40;
    let mut win = fltk::window::DoubleWindow::new(0, 0, bw, bh + controls, "playback bench");
    let mut video = Video::new(bw, bh + controls).autoplay(false);
    video.set_pos(0, 0);
    win.end();
    win.show();
    let pss_idle = pss_kib(std::process::id());
    video.set_file(Some(std::path::Path::new(&file)));
    // Probe (the player is shown), then Play and time the first frame.
    let t_probe = Instant::now();
    let end = Instant::now() + Duration::from_secs(10);
    while video.state() != PlayState::Paused && Instant::now() < end {
        wait(0.002);
    }
    let probe_ms = t_probe.elapsed().as_secs_f64() * 1000.0;
    // Wait for the poster frame and its process to finish.
    while (video.stats().frames == 0 || media::playback::live_processes() > 0) && Instant::now() < end {
        wait(0.01);
    }
    video.play();
    while video.stats().first_frame.is_none() && Instant::now() < end {
        wait(0.002);
    }
    let first = video.stats().first_frame;
    // Settle one second, then measure.
    let settle = Instant::now() + Duration::from_secs(1);
    while Instant::now() < settle {
        wait(0.01);
    }
    let me = std::process::id();
    let kids = children();
    let hz = 100.0; // USER_HZ on Linux
    let t0 = Instant::now();
    let (c0, k0) = (ticks(me).unwrap_or(0), kids.iter().map(|(p, _)| ticks(*p).unwrap_or(0)).sum::<u64>());
    let f0 = video.stats().frames;
    let mut pss_app = 0;
    let mut pss_kids = 0;
    let mut samples = 0;
    let stop = Instant::now() + Duration::from_secs_f64(seconds);
    let mut next_sample = Instant::now();
    while Instant::now() < stop && video.state() == PlayState::Playing {
        wait(0.01);
        if Instant::now() >= next_sample {
            pss_app += pss_kib(me);
            pss_kids += kids.iter().map(|(p, _)| pss_kib(*p)).sum::<u64>();
            samples += 1;
            next_sample += Duration::from_millis(500);
        }
    }
    let dt = t0.elapsed().as_secs_f64();
    let (c1, k1) = (ticks(me).unwrap_or(0), kids.iter().map(|(p, _)| ticks(*p).unwrap_or(0)).sum::<u64>());
    let frames = video.stats().frames - f0;
    let st = video.stats();
    video.stop();
    let reaped = media::playback::live_processes() == 0 && children().iter().all(|(_, n)| n != "ffmpeg" && n != "ffprobe");
    // Closing the window mid-play also ends the processes.
    video.play();
    let end = Instant::now() + Duration::from_secs(5);
    while media::playback::live_processes() == 0 && Instant::now() < end {
        wait(0.01);
    }
    let playing_again = media::playback::live_processes();
    win.hide();
    let end = Instant::now() + Duration::from_secs(5);
    while media::playback::live_processes() > 0 && Instant::now() < end {
        wait(0.05);
    }
    let reaped_on_close = media::playback::live_processes() == 0 && children().iter().all(|(_, n)| n != "ffmpeg");
    let n = samples.max(1);
    let line = format!(
        "{{\"file\":{:?},\"box\":[{bw},{bh}],\"frame\":[{},{}],\"audio\":{:?},\"probe_ms\":{probe_ms:.1},\"first_frame_ms\":{:.1},\"fps\":{:.1},\"cpu_app_pct\":{:.1},\"cpu_ffmpeg_pct\":{:.1},\"children\":{:?},\"pss_idle_mib\":{:.1},\"pss_app_mib\":{:.1},\"pss_ffmpeg_mib\":{:.1},\"reaped_after_stop\":{reaped},\"live_before_close\":{playing_again},\"reaped_after_window_close\":{reaped_on_close}}}",
        file,
        st.frame_size.0,
        st.frame_size.1,
        st.audio_out.unwrap_or("none"),
        first.map_or(f64::NAN, |d| d.as_secs_f64() * 1000.0),
        frames as f64 / dt,
        (c1 - c0) as f64 / hz / dt * 100.0,
        (k1 - k0) as f64 / hz / dt * 100.0,
        kids.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>(),
        pss_idle as f64 / 1024.0,
        pss_app as f64 / n as f64 / 1024.0,
        pss_kids as f64 / n as f64 / 1024.0,
    );
    println!("{line}");
    if let Some(out) = arg(&args, "--out") {
        let _ = std::fs::write(out, format!("{line}\n"));
    }
    drop(app);
}
