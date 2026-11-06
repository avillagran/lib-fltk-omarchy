//! What ffprobe says about a file: duration, video size and rate, audio.

use super::{args, Error};
use std::ffi::OsString;
use std::path::Path;

/// Longest duration accepted (seven days): beyond it the value is junk.
const MAX_DURATION: f64 = 7.0 * 24.0 * 3600.0;
/// Largest side of a video track accepted.
const MAX_SIDE: u64 = 16384;

/// A playable file, as probed.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    /// Seconds; `None` when the container does not say (some recordings):
    /// the position is shown but seeking is off.
    pub duration: Option<f64>,
    /// The demuxer ffprobe picked (`mov,mp4,m4a,3gp,3g2,mj2`, `matroska,webm`…).
    pub container: String,
    pub video: Option<VideoTrack>,
    pub audio: Option<AudioTrack>,
}

/// The first video track that is not cover art.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoTrack {
    pub codec: String,
    /// Coded size in pixels.
    pub width: u32,
    pub height: u32,
    /// Size as shown: sample aspect ratio and rotation applied (ffmpeg
    /// rotates while decoding).
    pub display: (u32, u32),
    /// Declared frame rate (numerator, denominator).
    pub rate: Option<(u32, u32)>,
    /// Clockwise quarter turns from the display matrix, 0–3.
    pub quarter_turns: u8,
}

/// The first audio track.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioTrack {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
}

impl Info {
    pub fn has_video(&self) -> bool {
        self.video.is_some()
    }

    pub fn has_audio(&self) -> bool {
        self.audio.is_some()
    }
}

/// ffprobe arguments for `path` (absolute, checked by the caller): JSON
/// with only the fields the player reads, the same protocol and demuxer
/// allowlists as playback.
pub(crate) fn probe_args(path: &Path) -> Vec<OsString> {
    let mut a: Vec<OsString> = ["-hide_banner", "-loglevel", "error"].iter().map(OsString::from).collect();
    args::push_input_guards(&mut a);
    for s in [
        "-of",
        "json",
        "-show_entries",
        "format=duration,format_name:stream=index,codec_type,codec_name,width,height,avg_frame_rate,r_frame_rate,sample_aspect_ratio,duration,sample_rate,channels:stream_disposition=attached_pic:stream_side_data=rotation",
    ] {
        a.push(s.into());
    }
    a.push(args::file_url(path));
    a
}

fn number(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().filter(|s| s.len() <= 32)?.parse().ok())
}

fn integer(v: &serde_json::Value) -> Option<u64> {
    v.as_u64().or_else(|| {
        let s = v.as_str().filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()))?;
        s.parse().ok()
    })
}

/// `"30000/1001"` → (30000, 1001); zero, junk or rates outside
/// 0.1–1000 fps give `None`.
pub(crate) fn parse_rate(s: &str) -> Option<(u32, u32)> {
    let (n, d) = s.split_once('/')?;
    let part = |p: &str| {
        (!p.is_empty() && p.len() <= 10 && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse::<u32>().ok()).flatten().filter(|v| *v > 0)
    };
    let (n, d) = (part(n)?, part(d)?);
    let fps = n as f64 / d as f64;
    (0.1..=1000.0).contains(&fps).then_some((n, d))
}

/// `"4:3"` → 1.333; missing, `N/A`, `0:1` → 1.0; out of 1/16–16 → error.
fn parse_aspect(v: Option<&str>) -> Result<f64, Error> {
    match v {
        None | Some("N/A") | Some("0:1") | Some("1:1") => Ok(1.0),
        Some(s) => {
            let bad = || Error::Unsupported(format!("bad sample aspect ratio {s:?}"));
            let (n, d) = s.split_once(':').ok_or_else(bad)?;
            let n: u32 = n.parse().map_err(|_| bad())?;
            let d: u32 = d.parse().map_err(|_| bad())?;
            if n == 0 || d == 0 {
                return Ok(1.0);
            }
            let r = n as f64 / d as f64;
            if (1.0 / 16.0..=16.0).contains(&r) {
                Ok(r)
            } else {
                Err(bad())
            }
        }
    }
}

/// Clockwise quarter turns of a display-matrix rotation in degrees
/// (ffprobe reports counter-clockwise as positive: 90 → 3, -90 → 1).
fn quarter_turns(deg: f64) -> Option<u8> {
    if !deg.is_finite() || deg.abs() > 36000.0 {
        return None;
    }
    let q = deg / 90.0;
    if (q - q.round()).abs() > 0.01 {
        return None;
    }
    Some(((-(q.round() as i64)).rem_euclid(4)) as u8)
}

/// Parses ffprobe's JSON (`probe_args`). Errors are values: a file with
/// no usable track, a broken duration or a huge frame is refused.
pub fn parse_probe(json: &[u8]) -> Result<Info, Error> {
    if json.len() > args::MAX_PROBE_BYTES {
        return Err(Error::Unsupported("probe output too large".into()));
    }
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| Error::Unsupported(format!("bad probe output: {e}")))?;
    let container = v["format"]["format_name"].as_str().unwrap_or_default().chars().take(64).collect::<String>();
    if container.is_empty() {
        return Err(Error::Unsupported("unknown container".into()));
    }
    let streams = v["streams"].as_array().map(|s| s.as_slice()).unwrap_or_default();
    let duration = number(&v["format"]["duration"])
        .or_else(|| streams.iter().filter_map(|s| number(&s["duration"])).fold(None, |m: Option<f64>, d| Some(m.map_or(d, |m| m.max(d)))))
        .filter(|d| d.is_finite() && *d > 0.0 && *d <= MAX_DURATION);
    let mut video = None;
    let mut audio = None;
    for s in streams.iter().take(64) {
        match s["codec_type"].as_str() {
            Some("video") if video.is_none() => {
                if integer(&s["disposition"]["attached_pic"]) == Some(1) {
                    continue; // cover art of an audio file
                }
                let dim = |k: &str| integer(&s[k]).filter(|n| (1..=MAX_SIDE).contains(n)).map(|n| n as u32);
                let (Some(w), Some(h)) = (dim("width"), dim("height")) else { continue };
                let aspect = parse_aspect(s["sample_aspect_ratio"].as_str())?;
                let mut turns = 0;
                if let Some(list) = s["side_data_list"].as_array() {
                    for sd in list.iter().take(8) {
                        if let Some(r) = number(&sd["rotation"]) {
                            turns = quarter_turns(r).ok_or_else(|| Error::Unsupported(format!("rotation {r} is not a quarter turn")))?;
                        }
                    }
                }
                let sw = ((w as f64 * aspect).round() as u32).clamp(1, MAX_SIDE as u32 * 16);
                let display = if turns % 2 == 1 { (h, sw) } else { (sw, h) };
                let rate = s["avg_frame_rate"].as_str().and_then(parse_rate).or_else(|| s["r_frame_rate"].as_str().and_then(parse_rate));
                video = Some(VideoTrack {
                    codec: s["codec_name"].as_str().unwrap_or("?").chars().take(32).collect(),
                    width: w,
                    height: h,
                    display,
                    rate,
                    quarter_turns: turns,
                });
            }
            Some("audio") if audio.is_none() => {
                audio = Some(AudioTrack {
                    codec: s["codec_name"].as_str().unwrap_or("?").chars().take(32).collect(),
                    sample_rate: integer(&s["sample_rate"]).filter(|r| (1..=768_000).contains(r)).unwrap_or(0) as u32,
                    channels: integer(&s["channels"]).filter(|c| (1..=64).contains(c)).unwrap_or(0) as u32,
                });
            }
            _ => {}
        }
    }
    if video.is_none() && audio.is_none() {
        return Err(Error::Unsupported("no audio or video track".into()));
    }
    Ok(Info { duration, container, video, audio })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MP4: &str = r#"{"programs":[],"streams":[
        {"index":0,"codec_name":"h264","codec_type":"video","width":1920,"height":1080,"sample_aspect_ratio":"1:1",
         "r_frame_rate":"30/1","avg_frame_rate":"30000/1001","duration":"10.000000","disposition":{"attached_pic":0}},
        {"index":1,"codec_name":"aac","codec_type":"audio","sample_rate":"48000","channels":2,
         "r_frame_rate":"0/0","avg_frame_rate":"0/0","duration":"10.000000","disposition":{"attached_pic":0}}],
        "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"10.000000"}}"#;

    #[test]
    fn parses_video_and_audio() {
        let i = parse_probe(MP4.as_bytes()).unwrap();
        assert_eq!(i.duration, Some(10.0));
        assert_eq!(i.container, "mov,mp4,m4a,3gp,3g2,mj2");
        let v = i.video.unwrap();
        assert_eq!((v.width, v.height, v.display), (1920, 1080, (1920, 1080)));
        assert_eq!(v.rate, Some((30000, 1001)), "avg_frame_rate first");
        let a = i.audio.unwrap();
        assert_eq!((a.codec.as_str(), a.sample_rate, a.channels), ("aac", 48000, 2));
    }

    #[test]
    fn rotation_and_aspect_change_the_display_size() {
        let j = r#"{"streams":[{"index":0,"codec_type":"video","codec_name":"h264","width":1920,"height":1080,
            "sample_aspect_ratio":"1:1","avg_frame_rate":"25/1","side_data_list":[{"rotation":-90}]}],
            "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","duration":"3.5"}}"#;
        let v = parse_probe(j.as_bytes()).unwrap().video.unwrap();
        assert_eq!(v.display, (1080, 1920));
        assert_eq!(v.quarter_turns, 1);
        let j = r#"{"streams":[{"index":0,"codec_type":"video","codec_name":"mpeg2video","width":720,"height":576,
            "sample_aspect_ratio":"16:11","avg_frame_rate":"25/1"}],"format":{"format_name":"mpeg","duration":"1"}}"#;
        assert_eq!(parse_probe(j.as_bytes()).unwrap().video.unwrap().display, (1047, 576));
        let j = r#"{"streams":[{"index":0,"codec_type":"video","codec_name":"h264","width":64,"height":64,
            "side_data_list":[{"rotation":45}]}],"format":{"format_name":"mov","duration":"1"}}"#;
        assert!(parse_probe(j.as_bytes()).is_err(), "only quarter turns");
    }

    #[test]
    fn audio_only_cover_art_and_missing_duration() {
        let j = r#"{"streams":[{"index":0,"codec_type":"audio","codec_name":"mp3","sample_rate":"44100","channels":2},
            {"index":1,"codec_type":"video","codec_name":"mjpeg","width":500,"height":500,"disposition":{"attached_pic":1}}],
            "format":{"format_name":"mp3","duration":"N/A"}}"#;
        let i = parse_probe(j.as_bytes()).unwrap();
        assert!(i.video.is_none(), "cover art is not a video track");
        assert!(i.has_audio());
        assert_eq!(i.duration, None);
        // The longest stream duration stands in for a missing format one.
        let j = r#"{"streams":[{"index":0,"codec_type":"audio","codec_name":"opus","duration":"4.25"}],"format":{"format_name":"ogg"}}"#;
        assert_eq!(parse_probe(j.as_bytes()).unwrap().duration, Some(4.25));
    }

    #[test]
    fn refuses_junk() {
        for j in [
            "",
            "not json",
            r#"{"streams":[],"format":{"format_name":"mp3","duration":"1"}}"#,
            r#"{"streams":[{"codec_type":"data"}],"format":{"format_name":"mov","duration":"1"}}"#,
            r#"{"streams":[{"codec_type":"audio"}],"format":{}}"#,
            r#"{"streams":[{"codec_type":"video","width":99999,"height":10}],"format":{"format_name":"mov"}}"#,
        ] {
            assert!(parse_probe(j.as_bytes()).is_err(), "{j}");
        }
        let j = r#"{"streams":[{"codec_type":"audio"}],"format":{"format_name":"wav","duration":"-3"}}"#;
        assert_eq!(parse_probe(j.as_bytes()).unwrap().duration, None, "negative duration is unknown");
        let j = r#"{"streams":[{"codec_type":"audio"}],"format":{"format_name":"wav","duration":"1e12"}}"#;
        assert_eq!(parse_probe(j.as_bytes()).unwrap().duration, None, "over seven days is unknown");
        assert!(parse_probe(&vec![b' '; args::MAX_PROBE_BYTES + 1]).is_err());
    }

    #[test]
    fn rates() {
        assert_eq!(parse_rate("30/1"), Some((30, 1)));
        assert_eq!(parse_rate("24000/1001"), Some((24000, 1001)));
        for bad in ["0/0", "30", "1/0", "-30/1", "30.5/1", "99999/1", "1/100", "x/y", ""] {
            assert_eq!(parse_rate(bad), None, "{bad}");
        }
        assert_eq!(quarter_turns(90.0), Some(3));
        assert_eq!(quarter_turns(-90.0), Some(1));
        assert_eq!(quarter_turns(180.0), Some(2));
        assert_eq!(quarter_turns(-270.0), Some(3));
        assert_eq!(quarter_turns(0.0), Some(0));
    }
}
