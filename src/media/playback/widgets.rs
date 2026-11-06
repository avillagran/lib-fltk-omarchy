//! `Video` and `Audio`: a player's picture and controls.

use super::args::format_time;
use super::core::{Kind, Notice, Player, Stats, STEP};
use super::{Error, Info, PlayState};
use crate::a11y::{self, Action, Role};
use crate::icon::{self, Svg};
use crate::layout::{self, Direction, FlexExt};
use crate::paint::{self, Interaction};
use crate::{theme, tr};
use fltk::{app, enums::*, group::Flex, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

// Lucide icons (lucide-static, ISC): play and pause filled.
const PLAY: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="6 3 20 12 6 21 6 3" fill="currentColor"/></svg>"#;
const PAUSE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="14" y="4" width="4" height="16" rx="1" fill="currentColor"/><rect x="6" y="4" width="4" height="16" rx="1" fill="currentColor"/></svg>"#;
const VOLUME: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z"/><path d="M16 9a5 5 0 0 1 0 6"/><path d="M19.364 18.364a9 9 0 0 0 0-12.728"/></svg>"#;
const VOLUME_X: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 4.702a.705.705 0 0 0-1.203-.498L6.413 7.587A1.4 1.4 0 0 1 5.416 8H3a1 1 0 0 0-1 1v6a1 1 0 0 0 1 1h2.416a1.4 1.4 0 0 1 .997.413l3.383 3.384A.705.705 0 0 0 11 19.298z"/><line x1="22" x2="16" y1="9" y2="15"/><line x1="16" x2="22" y1="9" y2="15"/></svg>"#;
const FILM: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect width="18" height="18" x="3" y="3" rx="2"/><path d="M7 3v18"/><path d="M3 7.5h4"/><path d="M3 12h18"/><path d="M3 16.5h4"/><path d="M17 3v18"/><path d="M17 7.5h4"/><path d="M17 16.5h4"/></svg>"#;
const MUSIC: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18V5l12-2v13"/><circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/></svg>"#;

fn scaled(px: f64) -> i32 {
    theme::with(|t| (px * t.font.base as f64 / 12.0).round() as i32)
}

fn dim(t: &theme::Theme) -> theme::Rgb {
    t.palette.foreground.darker(1.4)
}

/// What to say instead of the picture (and to screen readers).
fn status_text(p: &Player) -> Option<String> {
    let c = p.0.borrow();
    match &c.state {
        PlayState::Loading => Some(tr!("Loading…")),
        PlayState::Failed(Error::MissingTools) => Some(tr!("Playback needs ffmpeg and ffprobe")),
        PlayState::Failed(_) => Some(tr!("Cannot play this file")),
        _ => None,
    }
}

fn notice_text(p: &Player) -> Option<String> {
    p.0.borrow().notice.map(|n| match n {
        Notice::NoAudioOutput => tr!("No audio output: playing without sound"),
    })
}

fn play_name(p: &Player) -> String {
    if p.is_playing() {
        tr!("Pause")
    } else {
        tr!("Play")
    }
}

fn mute_name() -> String {
    if super::muted().get() {
        tr!("Unmute")
    } else {
        tr!("Mute")
    }
}

fn toggle_mute() {
    let m = super::muted();
    m.set(!m.get());
}

/// Square icon button whose icon and name follow the player.
fn icon_button(icon: Rc<dyn Fn() -> (&'static str, String)>, on: Rc<dyn Fn()>, keys: &'static str) -> Widget {
    let mut w = Widget::default();
    w.set_frame(FrameType::NoBox);
    let side = theme::with(|t| t.spacing.control_height);
    w.set_size(side, side);
    let ia = Rc::new(RefCell::new(Interaction::default()));
    crate::widgets::track_widget(&mut w, ia.clone(), true, on);
    let i2 = icon.clone();
    w.draw(move |w| {
        let (src, name) = i2();
        if w.tooltip().as_deref() != Some(name.as_str()) {
            w.set_tooltip(&name);
        }
        theme::with(|t| {
            let s = crate::widgets::interaction_of(w, &ia);
            let under = t.palette.background;
            // A square at the bottom (the audio row is taller than a control).
            let sq = w.w().min(w.h());
            let (bx, by) = (w.x() + (w.w() - sq) / 2, w.y() + w.h() - sq);
            paint::draw_look(bx, by, sq, sq, t.radius, paint::control_look(t, s, under, None, false), under);
            let fg = if s.disabled { t.palette.foreground.darker(2.0) } else { t.palette.foreground };
            if let Ok(svg) = Svg::cached(src) {
                let side = scaled(16.0).min(sq - 4).max(1);
                icon::draw(w, &svg, bx + (sq - side) / 2, by + (sq - side) / 2, side, side, fg);
            }
        });
    });
    a11y::describe(&w, move |n| {
        let (_, name) = icon();
        n.role(Role::Button).name(&name).focusable().action(Action::Click).shortcut(keys);
    });
    w
}

fn play_button(p: &Player) -> Widget {
    let (p1, p2) = (p.clone(), p.clone());
    icon_button(
        Rc::new(move || (if p1.is_playing() { PAUSE } else { PLAY }, play_name(&p1))),
        Rc::new(move || p2.toggle()),
        "Space",
    )
}

fn mute_button() -> Widget {
    let b = icon_button(Rc::new(|| (if super::muted().get() { VOLUME_X } else { VOLUME }, mute_name())), Rc::new(toggle_mute), "M");
    let mut w = b.clone();
    crate::state::bind_widget(&b, &super::muted(), false, move |_| {
        w.redraw();
        a11y::changed(&w);
    });
    b
}

/// The position slider: drag (seeks on release), Left/Right ±5 s,
/// Home/End; a Slider for screen readers with the time as its value.
fn seek_bar(p: &Player) -> Widget {
    let mut w = Widget::default();
    w.set_frame(FrameType::NoBox);
    w.set_size(scaled(160.0), theme::with(|t| t.spacing.control_height));
    let hot = Rc::new(std::cell::Cell::new(false));
    fn knob() -> i32 { theme::with(|t| 12.max((t.spacing.control_height as f64 * 0.34).round() as i32)) }
    let pos_at = move |w: &Widget, d: f64| {
        let k = knob();
        let f = ((app::event_x() - w.x() - k / 2) as f64 / (w.w() - k).max(1) as f64).clamp(0.0, 1.0);
        f * d
    };
    let (p2, h2) = (p.clone(), hot.clone());
    w.handle(move |w, ev| {
        let dur = p2.duration().filter(|_| w.active_r());
        match ev {
            Event::Enter => {
                h2.set(true);
                w.redraw();
                true
            }
            Event::Leave => {
                h2.set(false);
                w.redraw();
                true
            }
            Event::Push => {
                let _ = w.take_focus();
                if let Some(d) = dur {
                    p2.0.borrow_mut().scrub = Some(pos_at(w, d));
                    p2.notify();
                }
                true
            }
            Event::Drag => {
                if let Some(d) = dur {
                    p2.0.borrow_mut().scrub = Some(pos_at(w, d));
                    p2.notify();
                }
                true
            }
            Event::Released => {
                let s = p2.0.borrow_mut().scrub.take();
                if let Some(t) = s {
                    p2.seek(t);
                }
                true
            }
            Event::KeyDown => {
                let Some(d) = dur else { return false };
                match app::event_key() {
                    Key::Left => p2.seek_by(-STEP),
                    Key::Right => p2.seek_by(STEP),
                    Key::Home => p2.seek(0.0),
                    Key::End => p2.seek(d),
                    _ => return false,
                }
                true
            }
            Event::Focus | Event::Unfocus => {
                w.redraw();
                true
            }
            _ => false,
        }
    });
    let p3 = p.clone();
    w.draw(move |w| {
        let (pos, dur) = (p3.position(), p3.duration());
        theme::with(|t| {
            let under = t.palette.background;
            let fg = if w.active_r() { t.palette.foreground } else { t.palette.foreground.darker(2.0) };
            let track = 4.max((t.spacing.control_height as f64 * 0.11).round() as i32);
            let k = (knob() as f64 * if hot.get() { 1.15 } else { 1.0 }).round() as i32;
            let ty = w.y() + (w.h() - track) / 2;
            paint::fill(w.x(), ty, w.w(), track, track / 2, t.fill(&t.controls.selected, under));
            let f = dur.map_or(0.0, |d| if d > 0.0 { (pos / d).clamp(0.0, 1.0) } else { 0.0 });
            let fw = (w.w() as f64 * f).round() as i32;
            paint::fill(w.x(), ty, fw, track, track / 2, fg);
            if dur.is_some() {
                let kx = (w.x() + ((w.w() - k).max(0) as f64 * f).round() as i32).min(w.x() + w.w() - k).max(w.x());
                let ky = w.y() + (w.h() - k) / 2;
                fltk::draw::set_draw_color(under.fl());
                fltk::draw::draw_pie(kx - 2, ky - 2, k + 4, k + 4, 0.0, 360.0);
                fltk::draw::set_draw_color(fg.fl());
                fltk::draw::draw_pie(kx, ky, k, k, 0.0, 360.0);
            }
            if crate::focus::show_ring(w) {
                paint::stroke(w.x(), w.y(), w.w(), w.h(), t.radius, 1, t.border(&t.controls.focus, under));
            }
        });
    });
    let p4 = p.clone();
    a11y::describe(&w, move |n| {
        let (pos, dur) = (p4.position(), p4.duration());
        let text = match dur {
            Some(d) => tr!("{position} of {duration}", position = format_time(pos), duration = format_time(d)),
            None => format_time(pos),
        };
        n.role(Role::Slider).name(&tr!("Playback position")).numeric(pos, 0.0, dur.unwrap_or(0.0), STEP).value(&text).focusable();
        if dur.is_some() {
            n.action(Action::Increment).action(Action::Decrement).action(Action::SetValue);
        } else {
            n.disabled(true);
        }
    });
    let p5 = p.clone();
    a11y::on_action(&w, move |r| {
        match (r.action, &r.value) {
            (Action::Increment, _) => p5.seek_by(STEP),
            (Action::Decrement, _) => p5.seek_by(-STEP),
            (Action::SetValue, Some(a11y::Value::Number(x))) => p5.seek(*x),
            _ => return false,
        }
        true
    });
    w.set_visible_focus();
    w
}

/// "0:12 / 1:30" in the dim tone.
fn time_label(p: &Player) -> Widget {
    let mut w = Widget::default();
    w.set_frame(FrameType::NoBox);
    w.set_size(time_width(None), theme::with(|t| t.spacing.control_height));
    let p2 = p.clone();
    w.draw(move |w| {
        let text = time_text(&p2);
        theme::with(|t| {
            // On the controls' line (the bottom of a taller audio row).
            let h = w.h().min(t.spacing.control_height);
            paint::text(&text, w.x(), w.y() + w.h() - h, w.w(), h, t.font.body_small, false, dim(t), Align::Right)
        });
    });
    a11y::describe(&w, |n| {
        n.role(Role::Unknown).leaf(); // the slider reads the same
    });
    w
}

fn time_text(p: &Player) -> String {
    match p.duration() {
        Some(d) => format!("{} / {}", format_time(p.position()), format_time(d)),
        None => format_time(p.position()),
    }
}

fn time_width(dur: Option<f64>) -> i32 {
    let sample = match dur {
        Some(d) if d >= 3600.0 => "0:00:00 / 0:00:00",
        Some(d) if d < 600.0 => "0:00 / 0:00",
        _ => "00:00 / 00:00",
    };
    theme::with(|t| paint::measure(sample, t.font.body_small, false)) + scaled(4.0)
}

/// Shared by both players: the controls row's keys (Space/K, M, arrows,
/// Home/End) while focus is anywhere inside `group`.
fn keys(group: &mut Flex, p: &Player) {
    let p2 = p.clone();
    group.handle(move |_, ev| {
        if ev != Event::KeyDown || app::event_state().intersects(Shortcut::Ctrl | Shortcut::Alt | Shortcut::Meta) {
            return false;
        }
        let k = app::event_key();
        if k == Key::from_char(' ') || k == Key::from_char('k') {
            p2.toggle();
        } else if k == Key::from_char('m') {
            toggle_mute();
        } else if k == Key::Home {
            p2.seek(0.0);
        } else if k == Key::End {
            if let Some(d) = p2.duration() {
                p2.seek(d);
            }
        } else {
            return false;
        }
        true
    });
    let p3 = p.clone();
    crate::focus::scope(group, move |k| match k {
        Key::Left => {
            p3.seek_by(-STEP);
            true
        }
        Key::Right => {
            p3.seek_by(STEP);
            true
        }
        _ => false,
    });
}

macro_rules! player_api {
    () => {
        /// Plays the file (`set_file`) from the start; ends at its end.
        pub fn file(mut self, path: impl AsRef<Path>) -> Self {
            self.set_file(Some(path.as_ref()));
            self
        }

        /// Replaces the file (stops the current one); `None` empties the
        /// player. Only local files: never a URL.
        pub fn set_file(&mut self, path: Option<&Path>) {
            self.player.set_file(path);
        }

        /// Starts playing when the file is ready and the player shown.
        pub fn autoplay(self, on: bool) -> Self {
            self.player.0.borrow_mut().autoplay = on;
            self
        }

        /// The name screen readers read (a file name, a title); also the
        /// stream name the audio server shows.
        pub fn label(self, name: &str) -> Self {
            self.player.0.borrow_mut().name = name.to_string();
            self
        }

        pub fn play(&self) {
            self.player.play();
        }

        pub fn pause(&self) {
            self.player.pause();
        }

        /// Play or pause (Space, K, the play button).
        pub fn toggle(&self) {
            self.player.toggle();
        }

        /// Jumps to `seconds` (clamped to the duration).
        pub fn seek(&self, seconds: f64) {
            self.player.seek(seconds);
        }

        pub fn state(&self) -> PlayState {
            self.player.state()
        }

        /// Seconds from the start of the file.
        pub fn position(&self) -> f64 {
            self.player.position()
        }

        /// Seconds, once probed (`None` when the file does not say).
        pub fn duration(&self) -> Option<f64> {
            self.player.duration()
        }

        /// What ffprobe found, once probed.
        pub fn info(&self) -> Option<Info> {
            self.player.0.borrow().info.clone()
        }

        pub fn stats(&self) -> Stats {
            self.player.0.borrow().stats
        }

        /// True when the file has sound but no audio output works (the
        /// player says so and plays silently).
        pub fn without_sound(&self) -> bool {
            self.player.0.borrow().notice.is_some()
        }

        /// Pauses and ends this player's processes (also done when it is
        /// hidden, deleted or the app ends).
        pub fn stop(&self) {
            self.player.release();
        }

        /// Keyboard focus to the play button.
        pub fn focus(&self) {
            let _ = self.play_button.clone().take_focus();
        }

        /// The play/pause button (tests, focus order).
        pub fn play_button(&self) -> Widget {
            self.play_button.clone()
        }

        /// The position slider.
        pub fn seek_bar(&self) -> Widget {
            self.seek_bar.clone()
        }
    };
}

/// A video player: the picture over a row of controls (play/pause,
/// position slider, time, mute). Processes start only once it is shown;
/// hiding, pausing or deleting it ends them.
///
/// Keys while focus is inside: Space or K play/pause, Left/Right ±5 s,
/// Home/End, M mute (global, [`super::muted`]). Clicking the picture
/// plays or pauses.
#[derive(Clone)]
pub struct Video {
    inner: Flex,
    player: Player,
    play_button: Widget,
    seek_bar: Widget,
}
fltk::widget_extends!(Video, Flex, inner);

impl Video {
    /// A player of `w × h` units (picture and controls).
    pub fn new(w: i32, h: i32) -> Video {
        let player = Player::new(Kind::Video);
        let mut inner = Flex::default().column().with_size(w.max(80), h.max(60));
        inner.set_spacing(theme::with(|t| t.spacing.xs));
        let surface = surface(&player);
        let mut row = layout::row();
        row.direction(Direction::Ltr);
        row.animated(false);
        let play = play_button(&player);
        row.fit(&play);
        let seek = seek_bar(&player);
        let time = time_label(&player);
        row.fit(&time);
        let mute = mute_button();
        row.fit(&mute);
        row.end();
        inner.fixed(&row, theme::with(|t| t.spacing.control_height));
        inner.end();
        let _ = surface;
        for v in [&play, &seek, &time, &mute] {
            player.add_view(v);
        }
        player.add_view(&surface);
        player.0.borrow_mut().anchor = Some(surface.clone());
        refit_time(&player, &row, &time);
        enable_controls(&player, &play, &seek);
        // The surface click focuses the play button (keys work from there).
        let pb = play.clone();
        let mut s2 = surface.clone();
        let p2 = player.clone();
        s2.handle(move |_, ev| match ev {
            Event::Push => {
                let _ = pb.clone().take_focus();
                p2.toggle();
                true
            }
            _ => false,
        });
        keys(&mut inner, &player);
        let p3 = player.clone();
        a11y::describe(&inner, move |n| {
            let name = p3.0.borrow().name.clone();
            n.role(Role::Group).name(&if name.is_empty() { tr!("Video") } else { name });
        });
        Video { inner, player, play_button: play, seek_bar: seek }
    }

    player_api!();

    /// Media position of the frame on screen (seconds).
    pub fn frame_position(&self) -> Option<f64> {
        self.player.0.borrow().frame.as_ref().map(|f| f.pts)
    }

    /// The frame on screen as straight RGBA (copy, tests, a still).
    pub fn frame(&self) -> Option<crate::media::Bitmap> {
        let c = self.player.0.borrow();
        let f = c.frame.as_ref()?;
        let mut rgba = Vec::with_capacity(f.rgb.len() / 3 * 4);
        for px in f.rgb.chunks_exact(3) {
            rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
        crate::media::Bitmap::new(f.width, f.height, rgba).ok()
    }
}

/// Widens the time label once the duration is known (hours).
fn refit_time(p: &Player, row: &Flex, time: &Widget) {
    let (row, time) = (row.clone(), time.clone());
    let last = std::cell::Cell::new(time.w());
    let p2 = p.clone();
    p.add_hook(move || {
        let want = time_width(p2.duration());
        if last.get() != want && !row.was_deleted() {
            last.set(want);
            let mut r = row.clone();
            r.fixed(&time, want);
            r.layout();
        }
    });
}

/// Play and the slider are disabled while there is nothing to play
/// (no file, ffmpeg missing, a file that cannot play); the slider also
/// without a known duration.
fn enable_controls(p: &Player, play: &Widget, seek: &Widget) {
    let (play, seek) = (play.clone(), seek.clone());
    let p2 = p.clone();
    let apply = move || {
        let state = p2.state();
        let playable = !matches!(state, PlayState::Empty | PlayState::Failed(_));
        let seekable = playable && p2.duration().is_some();
        for (w, on) in [(&play, playable), (&seek, seekable)] {
            let mut w = w.clone();
            if w.was_deleted() || w.active() == on {
                continue;
            }
            if on {
                w.activate();
            } else {
                w.deactivate();
            }
        }
    };
    apply();
    p.add_hook(apply);
}

/// The picture: the latest frame fitted on the dark background, a play
/// mark while paused, an icon and a status line while there is none.
fn surface(p: &Player) -> Widget {
    let mut w = Widget::default();
    w.set_frame(FrameType::NoBox);
    let p2 = p.clone();
    w.draw(move |w| draw_surface(w, &p2));
    let p3 = p.clone();
    a11y::describe(&w, move |n| {
        let name = p3.0.borrow().name.clone();
        n.role(Role::Image).name(&if name.is_empty() { tr!("Video") } else { name });
        if let Some(s) = status_text(&p3).or_else(|| notice_text(&p3)) {
            n.description(&s);
        }
    });
    w
}

fn draw_surface(w: &mut Widget, p: &Player) {
    let ppu = crate::native::pixels_per_unit(w);
    p.set_box(((w.w() as f32 * ppu).round() as u32, (w.h() as f32 * ppu).round() as u32));
    p.drawn();
    let status = status_text(p);
    let notice = notice_text(p);
    let (x, y, bw, bh) = (w.x(), w.y(), w.w(), w.h());
    theme::with(|t| {
        let bg = t.palette.darker_background;
        paint::fill(x, y, bw, bh, t.badge_radius(), bg);
        let mut c = p.0.borrow_mut();
        let paused = matches!(c.state, PlayState::Paused | PlayState::Ended);
        let has_frame = c.frame.is_some();
        if let Some(f) = &c.frame {
            let (fw, fh) = (f.width as f64, f.height as f64);
            let k = (bw as f64 / fw).min(bh as f64 / fh);
            let (dw, dh) = ((fw * k).round().max(1.0) as i32, (fh * k).round().max(1.0) as i32);
            let (dx, dy) = (x + (bw - dw) / 2, y + (bh - dh) / 2);
            if c.image.is_none() {
                c.image = fltk::image::RgbImage::new(&f.rgb, f.width as i32, f.height as i32, ColorDepth::Rgb8).ok();
            }
            if let Some(img) = c.image.as_mut() {
                img.scale(dw, dh, false, true);
                img.draw(dx, dy, dw, dh);
            }
        }
        let audio_only = c.info.as_ref().is_some_and(|i| i.video.is_none());
        drop(c);
        let fg = t.palette.foreground;
        if has_frame && paused && status.is_none() {
            // Play mark in the middle.
            let side = scaled(44.0).min(bw.min(bh) / 2).max(8);
            let (cx, cy) = (x + (bw - side) / 2, y + (bh - side) / 2);
            paint::fill(cx, cy, side, side, side / 2, t.palette.background);
            if let Ok(svg) = Svg::cached(PLAY) {
                let s = side / 2;
                icon::draw(w, &svg, cx + (side - s) / 2 + s / 10, cy + (side - s) / 2, s, s, fg);
            }
        } else if !has_frame {
            let src = if audio_only { MUSIC } else { FILM };
            let color = if matches!(p.state(), PlayState::Failed(_)) { t.palette.urgent } else { dim(t) };
            let side = scaled(28.0).min(bw.min(bh) / 3).max(8);
            let text_h = t.font.body_small + scaled(8.0);
            let top = y + (bh - side - if status.is_some() { text_h } else { 0 }) / 2;
            if let Ok(svg) = Svg::cached(src) {
                icon::draw(w, &svg, x + (bw - side) / 2, top, side, side, color);
            }
            if let Some(s) = &status {
                paint::text(s, x + 4, top + side, bw - 8, text_h, t.font.body_small, false, color, Align::Center);
            }
        }
        if has_frame {
            if let Some(s) = status.as_ref().filter(|_| matches!(p.state(), PlayState::Failed(_))) {
                caption(x, y + bh, bw, s, t.palette.urgent, t);
            }
        }
        if let Some(n) = &notice {
            caption(x, y + bh, bw, n, dim(t), t);
        }
    });
}

/// A line of text on a dark band at the bottom of the picture.
fn caption(x: i32, bottom: i32, w: i32, text: &str, color: theme::Rgb, t: &theme::Theme) {
    let h = t.font.body_small + scaled(10.0);
    paint::fill(x, bottom - h, w, h, 0, t.palette.darker_background);
    paint::text(text, x + scaled(8.0), bottom - h, w - scaled(16.0), h, t.font.body_small, false, color, Align::Left);
}

/// An audio player in one row: play/pause, the name over the position
/// slider, the time, mute (o3's audio attachment). Probes when first
/// shown; plays on Play (no autoplay unless asked). Same keys as
/// [`Video`].
#[derive(Clone)]
pub struct Audio {
    inner: Flex,
    player: Player,
    play_button: Widget,
    seek_bar: Widget,
}
fltk::widget_extends!(Audio, Flex, inner);

impl Audio {
    /// A player `w` units wide.
    pub fn new(w: i32) -> Audio {
        let player = Player::new(Kind::Audio);
        player.0.borrow_mut().autoplay = false;
        let line = theme::with(|t| t.font.caption + scaled(6.0));
        let ch = theme::with(|t| t.spacing.control_height);
        let mut inner = Flex::default().row().with_size(w.max(120), line + ch);
        inner.set_spacing(theme::with(|t| t.spacing.control_gap));
        layout::set_direction(&inner, Some(Direction::Ltr));
        // One row; the buttons and the time sit on the slider's line.
        let play = play_button(&player);
        inner.fixed(&play, ch);
        let mut col = Flex::default().column();
        col.set_spacing(0);
        let name = name_line(&player);
        col.fixed(&name, line);
        let seek = seek_bar(&player);
        col.fixed(&seek, ch);
        col.end();
        let time = time_label(&player);
        inner.fixed(&time, time.w());
        let mute = mute_button();
        inner.fixed(&mute, ch);
        inner.end();
        refit_time(&player, &inner, &time);
        for v in [&play, &seek, &time, &mute, &name] {
            player.add_view(v);
        }
        player.0.borrow_mut().anchor = Some(name.clone());
        enable_controls(&player, &play, &seek);
        keys(&mut inner, &player);
        let p3 = player.clone();
        a11y::describe(&inner, move |n| {
            let name = p3.0.borrow().name.clone();
            n.role(Role::Group).name(&if name.is_empty() { tr!("Audio") } else { name });
        });
        Audio { inner, player, play_button: play, seek_bar: seek }
    }

    player_api!();
}

/// The file name, or why it cannot play, above the slider.
fn name_line(p: &Player) -> Widget {
    let mut w = Widget::default();
    w.set_frame(FrameType::NoBox);
    let p2 = p.clone();
    w.draw(move |w| {
        p2.drawn();
        let failed = matches!(p2.state(), PlayState::Failed(_));
        let text = match status_text(&p2).filter(|_| failed).or_else(|| notice_text(&p2)) {
            Some(s) => s,
            None => p2.0.borrow().name.clone(),
        };
        theme::with(|t| {
            let color = if failed { t.palette.urgent } else { t.palette.foreground };
            paint::text_elided(&text, w.x(), w.y(), w.w(), w.h(), t.font.caption, false, color, Align::Left);
        });
    });
    let p3 = p.clone();
    a11y::describe(&w, move |n| {
        let text = status_text(&p3).or_else(|| notice_text(&p3)).unwrap_or_else(|| p3.0.borrow().name.clone());
        n.role(Role::Label).name(&text);
    });
    w
}
