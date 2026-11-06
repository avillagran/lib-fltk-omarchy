//! Frame surfaces across two processes: this test binary starts itself as
//! the producer (`child_producer`, which does nothing unless the role
//! variable is set), for every transport, in a private temporary
//! directory. Checks that pixels arrive intact, that input events reach
//! the producer and that its answers come back. No display needed.
use fltk_omarchy::frame::{Button, Consumer, Format, Input, Message, Mods, Options, Producer, Transport};
use serde_json::json;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

const ROLE: &str = "FLTK_OMARCHY_FRAME_TEST_ROLE";

/// Frame pattern: (x, y) → R = x, G = y, B = `tag`, opaque.
fn paint(p: &mut Producer, tag: u8) {
    let (w, h) = p.size();
    let stride = p.stride();
    let px = p.frame_mut();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let o = y * stride + x * 4;
            px[o..o + 4].copy_from_slice(&[x as u8, y as u8, tag, 255]);
        }
    }
    p.present();
}

/// The producer side, run in the child process.
#[test]
fn child_producer() {
    if std::env::var(ROLE).as_deref() != Ok("producer") {
        return;
    }
    let mut p = Producer::from_env().expect("open the surface from the environment");
    p.set_format(Format::Rgba, true);
    paint(&mut p, 0x5a);
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end && !p.consumer_gone() {
        for e in p.wait_events(Some(Duration::from_millis(200))) {
            match &e {
                Input::Resize { width, height, scale } => {
                    p.set_size((*width as f32 * scale) as u32, (*height as f32 * scale) as u32);
                    paint(&mut p, 0x77);
                }
                Input::KeyDown { key, .. } if key == "q" => return,
                Input::Copy => {
                    p.send(&Message::Clipboard("copied text".into()));
                }
                _ => {
                    p.send(&Message::Other(json!({"echo": e.to_json()})));
                }
            }
        }
    }
}

fn private_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fltk-omarchy-frame-it-{tag}-{}", std::process::id()));
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

fn next_messages(c: &mut Consumer) -> Vec<Message> {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        c.wait(Some(Duration::from_millis(500)));
        let m = c.messages();
        if !m.is_empty() {
            return m;
        }
    }
    Vec::new()
}

/// (width, height, seq, pixel at (10, 20), last pixel) of the next frame.
type Seen = (u32, u32, u64, [u8; 4], [u8; 4]);

fn next_frame(c: &mut Consumer) -> Option<Seen> {
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        c.wait(Some(Duration::from_millis(500)));
        if let Some(f) = c.acquire() {
            let at = |x: usize, y: usize| -> [u8; 4] { f.pixels[y * f.stride + x * 4..y * f.stride + x * 4 + 4].try_into().unwrap() };
            return Some((f.width, f.height, f.seq, at(10, 20), at(f.width as usize - 1, f.height as usize - 1)));
        }
    }
    None
}

fn round_trip(t: Transport) {
    let dir = private_dir(&format!("{t:?}"));
    let o = Options::new(64, 48).capacity(256, 256).transport(t).dir(&dir);
    let (_handle, mut c) = Consumer::create_with(o).expect("create surface");
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "child_producer", "--nocapture", "--test-threads=1"]).env(ROLE, "producer");
    c.share(&mut cmd);
    cmd.stdout(std::process::Stdio::null());
    let mut child = cmd.spawn().expect("spawn producer");
    c.release_remote();

    // First frame: the size the consumer asked for, pixels intact.
    let (w, h, _, p, last) = next_frame(&mut c).unwrap_or_else(|| panic!("{t:?}: no frame"));
    assert_eq!((w, h), (64, 48), "{t:?}");
    assert_eq!(p, [10, 20, 0x5a, 255], "{t:?}");
    assert_eq!(last, [63, 47, 0x5a, 255], "{t:?}");

    // Input reaches the producer; its echo comes back.
    let press = Input::Press { x: 12.5, y: 7.0, button: Button::Left, buttons: 1, mods: Mods::SHIFT };
    assert!(c.send(&press));
    let m = next_messages(&mut c);
    assert_eq!(m, vec![Message::Other(json!({"echo": press.to_json()}))], "{t:?}");
    let key = Input::KeyDown { key: "a".into(), text: "a".into(), mods: Mods::NONE, repeat: false };
    c.send(&key);
    assert_eq!(next_messages(&mut c), vec![Message::Other(json!({"echo": key.to_json()}))], "{t:?}");
    c.send(&Input::Copy);
    assert_eq!(next_messages(&mut c), vec![Message::Clipboard("copied text".into())], "{t:?}");

    // A resize request gives frames of the new device size.
    c.send(&Input::Resize { width: 100, height: 50, scale: 2.0 });
    let (w, h, seq, p, _) = next_frame(&mut c).unwrap_or_else(|| panic!("{t:?}: no resized frame"));
    assert_eq!((w, h, seq), (200, 100, 2), "{t:?}");
    assert_eq!(p, [10, 20, 0x77, 255], "{t:?}");

    // Quit: the producer exits and the consumer sees it go.
    c.send(&Input::KeyDown { key: "q".into(), text: "q".into(), mods: Mods::NONE, repeat: false });
    let status = child.wait().unwrap();
    assert!(status.success(), "{t:?}: producer failed: {status:?}");
    let end = Instant::now() + Duration::from_secs(3);
    while !c.producer_gone() && Instant::now() < end {
        c.wait(Some(Duration::from_millis(100)));
    }
    assert!(c.producer_gone(), "{t:?}");
    drop(c);
    tidy(&dir);
}

#[test]
fn inherited_surface_round_trip() {
    if std::env::var(ROLE).is_ok() {
        return;
    }
    round_trip(Transport::Inherit);
}

#[test]
fn named_surface_round_trip() {
    if std::env::var(ROLE).is_ok() {
        return;
    }
    round_trip(Transport::Named);
}

#[test]
fn file_surface_round_trip() {
    if std::env::var(ROLE).is_ok() {
        return;
    }
    round_trip(Transport::File);
}
