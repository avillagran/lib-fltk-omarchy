//! UI checks of the Sidebar features a file manager needs: section
//! headers (foldable), detail lines with usage bars, a secondary button
//! (eject) with its tooltip, dimmed/disabled/busy rows, context menus,
//! drops, keyed `set_items` with row motion, scrolling, and the sidebar
//! nested in a plain `Group` inside the body (push and overlay).
use super::{check, png_rgb, pump_until};
use fltk::prelude::*;
use fltk_omarchy::anim::{self, clock};
use fltk_omarchy::focus::{self, press};
use fltk_omarchy::layout::{self, FlexExt};
use fltk_omarchy::theme;
use fltk_omarchy::widgets::*;
use fltk_omarchy::{tooltip, App, AppWindow, State};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn shot(win: &AppWindow, dir: &Path, name: &str) -> Result<PathBuf, String> {
    let p = dir.join(name);
    win.clone().snapshot(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

fn delete(win: AppWindow) {
    fltk::window::DoubleWindow::delete((*win).clone());
}

fn ms(n: u64) {
    clock::advance(Duration::from_millis(n));
}

/// Animations on, frozen clock; back to the test defaults afterwards.
struct Motion;
impl Motion {
    fn on() -> Motion {
        anim::set_enabled(true);
        clock::freeze();
        Motion
    }
}
impl Drop for Motion {
    fn drop(&mut self) {
        clock::resume();
        anim::set_enabled(false);
    }
}

/// Within 2 levels per channel (Windows GDI rounds blended colors).
fn near(a: [u8; 3], b: [u8; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2)
}

fn rgb(c: theme::Rgb) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

/// Count of pixels near `color` inside `(x, y, w, h)` of a snapshot.
fn count(p: &Path, (x, y, w, h): (i32, i32, i32, i32), color: [u8; 3]) -> Result<usize, String> {
    let img = png_rgb(p)?;
    let mut n = 0;
    for yy in y.max(0)..y + h {
        for xx in x.max(0)..x + w {
            if near(img.at(xx, yy), color) {
                n += 1;
            }
        }
    }
    Ok(n)
}

fn same_region(a: &Path, b: &Path, (x, y, w, h): (i32, i32, i32, i32)) -> Result<bool, String> {
    let (a, b) = (png_rgb(a)?, png_rgb(b)?);
    for yy in y..y + h {
        for xx in x..x + w {
            if !near(a.at(xx, yy), b.at(xx, yy)) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

// Item indexes of `places()`.
const PLACES: usize = 0;
const HOME: usize = 1;
const DOWNLOADS: usize = 2;
const DEVICES: usize = 3;
const SYSTEM: usize = 4;
const USB: usize = 5;
const BACKUP: usize = 6;
const LOCKED: usize = 7;
const SETTINGS: usize = 8;

fn places() -> Vec<SidebarItem> {
    vec![
        SidebarItem::header("Places").key("h:places"),
        SidebarItem::new(glyph::HOME, "Home").key("home"),
        SidebarItem::new(glyph::DOWNLOAD, "Downloads")
            .key("downloads")
            .badge("3"),
        SidebarItem::header("Devices")
            .key("h:devices")
            .collapsible(),
        SidebarItem::new(glyph::HARDDISK, "System")
            .key("sys")
            .detail("12 GB free of 32 GB")
            .usage(0.62),
        SidebarItem::new(glyph::USB, "USB stick")
            .key("usb")
            .detail("1 GB free of 16 GB")
            .usage(0.94)
            .secondary(glyph::EJECT, "Eject"),
        SidebarItem::new(glyph::HARDDISK, "Backup")
            .key("backup")
            .dimmed(),
        SidebarItem::new(glyph::HARDDISK, "Locked")
            .key("locked")
            .disabled(),
        SidebarItem::new(glyph::SETTINGS, "Settings")
            .key("settings")
            .footer(),
    ]
}

struct Shell {
    win: AppWindow,
    side: Sidebar,
    selected: State<usize>,
}

fn shell(items: Vec<SidebarItem>, mode: SidebarMode, h: i32) -> Shell {
    let mut win = AppWindow::new("Sidebar", 620, h);
    let mut body = win.body();
    body.set_margin(0);
    let selected = State::new(HOME);
    let mut row = layout::row();
    let side = Sidebar::new(items, &selected).mode(mode);
    row.fit(&*side);
    let mut col = layout::column();
    col.fit(&*Button::new("Content").bordered());
    col.end();
    row.end();
    win.end();
    Shell {
        win,
        side,
        selected,
    }
}

type Log = Rc<RefCell<Vec<String>>>;

/// Every callback, logged as text.
fn wire(side: &Sidebar) -> Log {
    let log: Log = Rc::default();
    let (a, b, c, d, e, f) = (
        log.clone(),
        log.clone(),
        log.clone(),
        log.clone(),
        log.clone(),
        log.clone(),
    );
    let _ = side
        .clone()
        .on_activate(move |i| a.borrow_mut().push(format!("activate {i}")))
        .on_secondary(move |i| b.borrow_mut().push(format!("secondary {i}")))
        .on_context(move |i, x, y| c.borrow_mut().push(format!("context {i} {x} {y}")))
        .on_section_toggle(move |i, on| d.borrow_mut().push(format!("section {i} {on}")))
        .on_drop(move |i, t| e.borrow_mut().push(format!("drop {i} {}", t.trim())))
        .on_drop_files(move |i, p| f.borrow_mut().push(format!("files {i} {p:?}")))
        .accept_drop(|i| i != BACKUP);
    log
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

fn center(r: (i32, i32, i32, i32)) -> (i32, i32) {
    (r.0 + r.2 / 2, r.1 + r.3 / 2)
}

/// A point on the secondary button of a row: its trailing end.
fn secondary_point(r: (i32, i32, i32, i32)) -> (i32, i32) {
    (r.0 + r.2 - 18, r.1 + r.3 / 2)
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "sidebar files: headers, detail rows two lines tall when expanded, usage bars (accent, urgent), secondary button, dimmed and disabled",
        &|shots| {
            anim::set_enabled(false);
            let s = shell(places(), SidebarMode::Push, 460);
            shot(&s.win, shots, "sidebar-files-0-rail.png")?;
            let rail_one = s.side.item_rect(HOME).ok_or("no Home rect")?;
            let rail_two = s.side.item_rect(SYSTEM).ok_or("no System rect")?;
            let rail_head = s.side.item_rect(DEVICES).ok_or("no header rect")?;
            check(
                rail_one.3 == rail_two.3 && rail_head.3 < rail_one.3,
                &format!("collapsed: one height {rail_one:?} {rail_two:?}, header shorter {rail_head:?}"),
            )?;
            s.side.expand();
            let p = shot(&s.win, shots, "sidebar-files-1-expanded.png")?;
            let one = s.side.item_rect(HOME).ok_or("no Home rect")?;
            let two = s.side.item_rect(USB).ok_or("no USB rect")?;
            check(
                two.3 > one.3 + theme::current().font.caption,
                &format!("expanded: detail rows are two lines: {one:?} vs {two:?}"),
            )?;
            let (accent, urgent) = theme::with(|t| (rgb(t.palette.accent), rgb(t.palette.urgent)));
            let sys = s.side.item_rect(SYSTEM).ok_or("no System rect")?;
            let a = count(&p, sys, accent)?;
            let u = count(&p, two, urgent)?;
            let u_sys = count(&p, sys, urgent)?;
            check(
                a >= 20 && u >= 10 && u_sys == 0,
                &format!("usage bars: System accent {a} px, USB urgent {u} px, System urgent {u_sys}"),
            )?;
            // The secondary button shows its glyph at the trailing end.
            let (bx, by) = secondary_point(two);
            let img = png_rgb(&p)?;
            let fg = theme::with(|t| rgb(t.palette.foreground));
            let lit = (bx - 10..bx + 10)
                .flat_map(|x| (by - 8..by + 8).map(move |y| (x, y)))
                .filter(|&(x, y)| {
                    let c = img.at(x, y);
                    c.iter().zip(fg).all(|(a, b)| a.abs_diff(b) < 60)
                })
                .count();
            check(lit >= 6, &format!("eject glyph drawn ({lit} px)"))?;
            // Dimmed and disabled labels are dimmer than a normal one.
            let ink = |r: (i32, i32, i32, i32)| -> u32 {
                let mut best = 0;
                for y in r.1..r.1 + r.3 {
                    for x in r.0 + 50..r.0 + 140 {
                        let c = img.at(x, y);
                        best = best.max(c[0] as u32 + c[1] as u32 + c[2] as u32);
                    }
                }
                best
            };
            let (home, backup, locked) = (
                ink(one),
                ink(s.side.item_rect(BACKUP).ok_or("no Backup")?),
                ink(s.side.item_rect(LOCKED).ok_or("no Locked")?),
            );
            check(
                home > backup && backup > locked,
                &format!("brightest label pixel: normal {home} > dimmed {backup} > disabled {locked}"),
            )?;
            delete(s.win);
            Ok(())
        },
    );
    run(
        "sidebar files: clicks (activate, header folds, disabled, secondary with tooltip, right click context) with app indexes",
        &|shots| {
            anim::set_enabled(false);
            let s = shell(places(), SidebarMode::Push, 460);
            let log = wire(&s.side);
            s.side.set_pinned(true);
            shot(&s.win, shots, "sidebar-files-click-0.png")?;
            let r = |i| s.side.item_rect(i).ok_or(format!("no rect {i}"));
            let (x, y) = center(r(DOWNLOADS)?);
            focus::click_at(&*s.win, x, y);
            check(
                take(&log) == ["activate 2"] && s.selected.get() == DOWNLOADS,
                "click activates Downloads (index 2)",
            )?;
            let (x, y) = center(r(PLACES)?);
            focus::click_at(&*s.win, x, y);
            check(take(&log).is_empty() && s.selected.get() == DOWNLOADS, "a plain header does nothing")?;
            let (x, y) = center(r(LOCKED)?);
            focus::click_at(&*s.win, x, y);
            check(take(&log).is_empty(), "a disabled row does not activate")?;
            let (x, y) = center(r(BACKUP)?);
            focus::click_at(&*s.win, x, y);
            check(take(&log) == ["activate 6"], "a dimmed row activates")?;
            // Secondary: its own hover, the tooltip, the callback, no activation.
            // (Clicks mapped the window: let its position settle first.)
            let settle = Instant::now() + Duration::from_millis(150);
            while Instant::now() < settle {
                App::pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            let (bx, by) = secondary_point(r(USB)?);
            focus::hover_at(&*s.win, bx, by);
            check(s.side.tooltip().as_deref() == Some("Eject"), &format!("tooltip {:?}", s.side.tooltip()))?;
            let end = Instant::now() + Duration::from_secs(2);
            while tooltip::shown().is_none() && Instant::now() < end {
                App::pump();
                std::thread::sleep(Duration::from_millis(5));
            }
            let shown = tooltip::shown().map(|t| t.text);
            check(shown.as_deref() == Some("Eject"), &format!("toolkit tooltip shows {shown:?}"))?;
            let p = shot(&s.win, shots, "sidebar-files-click-1-secondary-hover.png")?;
            // The row is hovered too; the button gets a second hover fill.
            let hot = theme::with(|t| {
                let surface = t.palette.foreground.over(t.palette.background, 0.04);
                let row = t.fill(&t.controls.hover, surface);
                rgb(t.fill(&t.controls.hover, row))
            });
            let hover_px = count(&p, (bx - 8, by - 8, 16, 16), hot)?;
            check(hover_px > 20, &format!("secondary hover fill: {hover_px} px"))?;
            focus::click_at(&*s.win, bx, by);
            check(
                take(&log) == ["secondary 5"] && s.selected.get() == BACKUP,
                "secondary click: on_secondary(5), selection kept",
            )?;
            let (x, y) = center(r(USB)?);
            focus::hover_at(&*s.win, x - 60, y);
            check(s.side.tooltip().as_deref().unwrap_or("").is_empty(), "tooltip cleared off the button")?;
            // Right click: context at the pointer (window coordinates).
            let (x, y) = center(r(HOME)?);
            focus::right_click_at(&*s.win, x, y);
            check(
                take(&log) == [format!("context 1 {x} {y}")] && s.selected.get() == BACKUP,
                "right click: on_context(1, x, y), no activation",
            )?;
            // A collapsible header folds and unfolds its section.
            let (x, y) = center(r(DEVICES)?);
            focus::click_at(&*s.win, x, y);
            shot(&s.win, shots, "sidebar-files-click-2-folded.png")?;
            check(
                take(&log) == ["section 3 true"]
                    && s.side.section_collapsed(DEVICES)
                    && (SYSTEM..=LOCKED).all(|i| s.side.item_rect(i).is_none())
                    && s.side.item_rect(SETTINGS).is_some(),
                "header click folds Devices (rows 4..7 hidden)",
            )?;
            focus::click_at(&*s.win, x, y);
            check(
                take(&log) == ["section 3 false"] && s.side.item_rect(USB).is_some(),
                "and unfolds it",
            )?;
            s.side.set_section_collapsed(DEVICES, true);
            check(take(&log).is_empty() && s.side.section_collapsed(DEVICES), "set_section_collapsed: no callback")?;
            s.side.set_section_collapsed(PLACES, true);
            check(!s.side.section_collapsed(PLACES), "a plain header does not fold")?;
            delete(s.win);
            Ok(())
        },
    );
    run(
        "sidebar files: keyboard skips plain headers, disabled and folded rows; Enter/Left/Right fold; Delete = secondary; Menu and Shift+F10 = context",
        &|shots| {
            anim::set_enabled(false);
            let s = shell(places(), SidebarMode::Push, 460);
            let log = wire(&s.side);
            shot(&s.win, shots, "sidebar-files-keys-0.png")?;
            let _ = s.side.clone().take_focus();
            check(s.side.is_expanded() && s.side.cursor() == HOME, "focus expands, cursor on Home")?;
            press("Up");
            check(s.side.cursor() == HOME, "Up on the first row stays (the Places header is skipped)")?;
            press("Down");
            press("Down");
            check(s.side.cursor() == DEVICES, &format!("Down lands on the collapsible header: {}", s.side.cursor()))?;
            press("Enter");
            check(take(&log) == ["section 3 true"], "Enter folds")?;
            press("Down");
            check(s.side.cursor() == SETTINGS, &format!("folded rows are skipped: {}", s.side.cursor()))?;
            press("Up");
            press("Right");
            check(take(&log) == ["section 3 false"] && !s.side.section_collapsed(DEVICES), "Right unfolds")?;
            press("Left");
            press("Space");
            check(take(&log) == ["section 3 true", "section 3 false"], "Left folds, Space toggles")?;
            press("Down");
            press("Down");
            check(s.side.cursor() == USB, "on USB")?;
            press("Delete");
            check(take(&log) == ["secondary 5"], "Delete runs the secondary action")?;
            press("Menu");
            shot(&s.win, shots, "sidebar-files-keys-1.png")?;
            let r = s.side.item_rect(USB).ok_or("no USB rect")?;
            let want = format!("context 5 {} {}", r.0 + theme::current().spacing.sm, r.1 + r.3);
            check(take(&log) == [want.clone()], &format!("Menu key: context at the row ({want})"))?;
            press("Shift+F10");
            check(take(&log) == [want], "Shift+F10 too")?;
            press("Down");
            press("Enter");
            check(take(&log) == ["activate 6"], "dimmed row activates with Enter")?;
            press("Down");
            check(s.side.cursor() == SETTINGS, &format!("disabled row skipped: {}", s.side.cursor()))?;
            press("Home");
            check(s.side.cursor() == HOME, "Home: first usable row")?;
            press("Delete");
            check(take(&log).is_empty(), "Delete without a secondary does nothing")?;
            delete(s.win);
            Ok(())
        },
    );
    run(
        "sidebar files: set_items keeps selection, cursor and folded sections by key; rows arrive, leave and slide (frozen clock); instant when off",
        &|shots| {
            anim::set_enabled(false);
            let mut s = shell(places(), SidebarMode::Push, 460);
            s.side.set_pinned(true);
            s.selected.set(USB);
            s.side.set_section_collapsed(DEVICES, false);
            shot(&s.win, shots, "sidebar-files-items-0.png")?;
            let mut more = places();
            more.insert(2, SidebarItem::new(glyph::BOOKMARK, "Work").key("work"));
            s.side.set_items(more.clone());
            check(s.selected.get() == USB + 1, &format!("selection follows the key: {}", s.selected.get()))?;
            // Folded state follows the header's key.
            s.side.set_section_collapsed(DEVICES + 1, true);
            s.side.set_items(places());
            check(
                s.side.section_collapsed(DEVICES) && s.selected.get() == USB,
                "folded section and selection follow back",
            )?;
            s.side.set_section_collapsed(DEVICES, false);
            // Without keys: indexes stay.
            let plain: Vec<SidebarItem> = places()
                .into_iter()
                .map(|mut it| {
                    it.key.clear();
                    it
                })
                .collect();
            s.side.set_items(plain.clone());
            let mut plain2 = plain.clone();
            plain2.insert(2, SidebarItem::new(glyph::BOOKMARK, "Work"));
            s.side.set_items(plain2);
            check(s.selected.get() == USB, "without keys the index stays")?;
            s.side.set_items(places());
            shot(&s.win, shots, "sidebar-files-items-1.png")?;
            // Row motion: Downloads slides down to make room for Work.
            {
                let _m = Motion::on();
                let before = s.side.item_rect(DOWNLOADS).ok_or("no Downloads")?;
                s.side.set_items(more.clone());
                shot(&s.win, shots, "sidebar-files-items-2-t0.png")?;
                let t0 = s.side.item_rect(DOWNLOADS + 1).ok_or("no Downloads t0")?;
                ms(120);
                let p_mid = shot(&s.win, shots, "sidebar-files-items-3-t120.png")?;
                let mid = s.side.item_rect(DOWNLOADS + 1).ok_or("no Downloads mid")?;
                ms(700);
                let p_end = shot(&s.win, shots, "sidebar-files-items-4-end.png")?;
                let end = s.side.item_rect(DOWNLOADS + 1).ok_or("no Downloads end")?;
                check(
                    t0.1 == before.1 && before.1 < mid.1 && mid.1 < end.1,
                    &format!("Downloads y: before {} t0 {} mid {} end {}", before.1, t0.1, mid.1, end.1),
                )?;
                let work = s.side.item_rect(2).ok_or("no Work")?;
                check(
                    !same_region(&p_mid, &p_end, work)?,
                    "the arriving row is still fading in mid-way",
                )?;
                check(!fltk_omarchy::layout::animating(), "no layout motion involved")?;
                // A row leaving fades out where it was.
                s.side.set_items(places());
                let p0 = shot(&s.win, shots, "sidebar-files-items-5-leave-t0.png")?;
                check(anim::pending() > 0, "a frame is scheduled while rows move")?;
                ms(800);
                let p1 = shot(&s.win, shots, "sidebar-files-items-6-leave-end.png")?;
                check(!same_region(&p0, &p1, work)?, "the leaving row was drawn, then gone")?;
            }
            // Off: at once.
            s.side.set_items(more);
            shot(&s.win, shots, "sidebar-files-items-7-off.png")?;
            let r = s.side.item_rect(DOWNLOADS + 1).ok_or("no Downloads")?;
            let r2 = s.side.item_rect(2).ok_or("no Work")?;
            check(r.1 == r2.1 + r2.3, &format!("off: final place at once {r:?} after {r2:?}"))?;
            delete(s.win);
            Ok(())
        },
    );
    run(
        "sidebar files: section fold animates (rows below slide, 200 ms)",
        &|shots| {
            anim::set_enabled(false);
            let s = shell(places(), SidebarMode::Push, 460);
            s.side.set_pinned(true);
            shot(&s.win, shots, "sidebar-files-fold-0.png")?;
            let _m = Motion::on();
            shot(&s.win, shots, "sidebar-files-fold-1.png")?;
            let mut more = places();
            more.insert(SETTINGS, SidebarItem::header("Other").key("h:other"));
            more.insert(
                SETTINGS + 1,
                SidebarItem::new(glyph::BOOKMARK, "After").key("after"),
            );
            let after = SETTINGS + 1;
            let mut side = s.side.clone();
            side.set_items(more);
            ms(900);
            shot(&s.win, shots, "sidebar-files-fold-2.png")?;
            let open = s.side.item_rect(after).ok_or("no After")?;
            s.side.set_section_collapsed(DEVICES, true);
            shot(&s.win, shots, "sidebar-files-fold-3-t0.png")?;
            let t0 = s.side.item_rect(after).ok_or("no After t0")?;
            ms(60);
            shot(&s.win, shots, "sidebar-files-fold-4-t60.png")?;
            let mid = s.side.item_rect(after).ok_or("no After mid")?;
            ms(400);
            shot(&s.win, shots, "sidebar-files-fold-5-end.png")?;
            let end = s.side.item_rect(after).ok_or("no After end")?;
            check(
                t0.1 == open.1 && end.1 < mid.1 && mid.1 < open.1,
                &format!(
                    "the row below slides up: {} -> {} -> {}",
                    open.1, mid.1, end.1
                ),
            )?;
            delete(s.win);
            Ok(())
        },
    );
    run(
        "sidebar files: drop target highlight, accept filter, on_drop and on_drop_files with the item index",
        &|shots| {
            anim::set_enabled(false);
            let s = shell(places(), SidebarMode::Push, 460);
            let log = wire(&s.side);
            s.side.set_pinned(true);
            let p0 = shot(&s.win, shots, "sidebar-files-drop-0.png")?;
            let r = s.side.item_rect(DOWNLOADS).ok_or("no Downloads")?;
            let (x, y) = center(r);
            let uri = "file:///tmp/My%20File.txt\r\nfile:///tmp/b\r\n";
            check(focus::drop_at(&*s.win, x, y, uri, false), "the row accepts the drag")?;
            check(s.side.drop_target() == Some(DOWNLOADS), &format!("drop target {:?}", s.side.drop_target()))?;
            let p1 = shot(&s.win, shots, "sidebar-files-drop-1-hover.png")?;
            let accent = theme::with(|t| rgb(t.palette.accent));
            let (a0, a1) = (count(&p0, r, accent)?, count(&p1, r, accent)?);
            check(a1 > a0 + 40, &format!("highlight: accent {a0} -> {a1} px"))?;
            // Over a filtered row and a header: refused.
            let (bx, by) = center(s.side.item_rect(BACKUP).ok_or("no Backup")?);
            check(!focus::drop_at(&*s.win, bx, by, uri, false), "accept_drop filter refuses Backup")?;
            check(s.side.drop_target().is_none(), "no target over Backup")?;
            let (hx, hy) = center(s.side.item_rect(DEVICES).ok_or("no header")?);
            check(!focus::drop_at(&*s.win, hx, hy, uri, true), "a header takes no drop")?;
            check(take(&log).is_empty(), "nothing dropped yet")?;
            check(focus::drop_at(&*s.win, x, y, uri, true), "dropped on Downloads")?;
            let got = take(&log);
            check(
                got == [
                    "drop 2 file:///tmp/My%20File.txt\r\nfile:///tmp/b".to_string(),
                    "files 2 [\"/tmp/My File.txt\", \"/tmp/b\"]".to_string(),
                ],
                &format!("callbacks {got:?}"),
            )?;
            check(s.side.drop_target().is_none(), "highlight gone after the drop")?;
            // Leaving clears the highlight.
            focus::drop_at(&*s.win, x, y, uri, false);
            focus::drag_leave(&*s.win);
            check(s.side.drop_target().is_none(), "leaving clears the target")?;
            delete(s.win);
            Ok(())
        },
    );
    run(
        "sidebar files: busy spinner is static with animations off and turns (one frame per tick) when on",
        &|shots| {
            anim::set_enabled(false);
            let mut items = places();
            items[USB] = items[USB].clone().busy(true);
            let s = shell(items, SidebarMode::Push, 460);
            s.side.set_pinned(true);
            let a = shot(&s.win, shots, "sidebar-files-busy-0.png")?;
            let r = s.side.item_rect(USB).ok_or("no USB")?;
            let spot = (r.0 + r.2 - 36, r.1, 36, r.3);
            std::thread::sleep(Duration::from_millis(150));
            App::pump();
            let b = shot(&s.win, shots, "sidebar-files-busy-1.png")?;
            check(same_region(&a, &b, spot)?, "static spinner with animations off")?;
            check(anim::pending() == 0, "no frame scheduled when off")?;
            // Ink that differs from the row background (renderer-neutral:
            // Windows GDI draws the dots thinner, without antialiasing).
            let img = png_rgb(&a)?;
            let back = img.at(spot.0 + 1, spot.1 + 1);
            let dots = (spot.0..spot.0 + spot.2)
                .flat_map(|x| (spot.1..spot.1 + spot.3).map(move |y| (x, y)))
                .filter(|&(x, y)| img.at(x, y).iter().zip(back).map(|(p, q)| p.abs_diff(q) as u32).sum::<u32>() > 60)
                .count();
            check(dots >= 4, &format!("spinner drawn ({dots} px)"))?;
            let _m = Motion::on();
            let c = shot(&s.win, shots, "sidebar-files-busy-2-on.png")?;
            check(anim::pending() > 0, "a frame is scheduled while it turns")?;
            ms(250);
            let d = shot(&s.win, shots, "sidebar-files-busy-3-on-250.png")?;
            check(!same_region(&c, &d, spot)?, "it turned")?;
            delete(s.win);
            Ok(())
        },
    );
    run(
        "sidebar files: rows scroll (wheel, keyboard keeps the cursor in view), footers stay",
        &|shots| {
            anim::set_enabled(false);
            let mut items = vec![SidebarItem::header("Bookmarks").key("h")];
            for i in 0..30 {
                items.push(
                    SidebarItem::new(glyph::BOOKMARK, &format!("Bookmark {i}"))
                        .key(&format!("b{i}")),
                );
            }
            items.push(
                SidebarItem::new(glyph::SETTINGS, "Settings")
                    .key("s")
                    .footer(),
            );
            let s = shell(items, SidebarMode::Push, 300);
            s.side.set_pinned(true);
            shot(&s.win, shots, "sidebar-files-scroll-0.png")?;
            let (sc, max) = s.side.scroll_position();
            check(sc == 0 && max > 0, &format!("scrollable: {sc}/{max}"))?;
            let foot = s.side.item_rect(31).ok_or("no footer")?;
            check(
                foot.1 + foot.3 <= s.side.y() + s.side.h(),
                "footer at the bottom",
            )?;
            check(
                s.side.item_rect(30).is_none(),
                "the last bookmark is out of view",
            )?;
            let _ = s.side.clone().take_focus();
            press("Down");
            for _ in 0..28 {
                press("Down");
            }
            shot(&s.win, shots, "sidebar-files-scroll-1-keys.png")?;
            let (sc, _) = s.side.scroll_position();
            let cur = s.side.item_rect(s.side.cursor());
            check(
                s.side.cursor() == 30 && sc == max && cur.is_some(),
                &format!("cursor {} in view, scroll {sc}/{max}", s.side.cursor()),
            )?;
            let foot2 = s.side.item_rect(31).ok_or("no footer")?;
            check(foot2 == foot, "footer did not move")?;
            let (x, y) = center(s.side.item_rect(28).ok_or("no row 28")?);
            check(focus::wheel_at(&*s.win, x, y, -2), "wheel used")?;
            shot(&s.win, shots, "sidebar-files-scroll-2-wheel.png")?;
            let (sc2, _) = s.side.scroll_position();
            check(sc2 < sc, &format!("wheel up scrolls: {sc} -> {sc2}"))?;
            press("Home");
            shot(&s.win, shots, "sidebar-files-scroll-3-home.png")?;
            check(
                s.side.scroll_position().0 == 0,
                "Home scrolls back to the top",
            )?;
            // Not scrollable: the wheel goes on to the content.
            let small = shell(places(), SidebarMode::Push, 460);
            shot(&small.win, shots, "sidebar-files-scroll-4-small.png")?;
            let (x, y) = center(small.side.item_rect(HOME).ok_or("no Home")?);
            check(
                !focus::wheel_at(&*small.win, x, y, 1),
                "wheel not used when everything fits",
            )?;
            delete(small.win);
            delete(s.win);
            Ok(())
        },
    );
    run(
        "sidebar files: nested in a Group inside the body: push hover-expand (frozen clock) moves the content, overlay layer at the offset rail, pinned",
        &|shots| {
            let build = |mode: SidebarMode| {
                let mut win = AppWindow::new("Nested", 640, 360);
                let mut body = win.body();
                body.set_margin(0);
                body.fit(&*Label::new("Above the group"));
                let mut outer = layout::row();
                let mut gutter = fltk::frame::Frame::default();
                outer.fixed(&gutter, 40);
                gutter.set_label("");
                let g = fltk::group::Group::new(0, 0, 500, 300, None);
                let mut row = layout::row();
                row.resize(g.x(), g.y(), g.w(), g.h());
                let selected = State::new(1usize);
                let side = Sidebar::new(places(), &selected).mode(mode);
                row.fit(&*side);
                let mut col = layout::column();
                let content = Button::new("Content").bordered();
                col.fit(&*content);
                col.end();
                row.end();
                g.end();
                g.resizable(&row);
                outer.end();
                win.end();
                (win, side, content, g)
            };
            anim::set_enabled(false);
            let (win, side, content, g) = build(SidebarMode::Push);
            shot(&win, shots, "sidebar-nested-0.png")?;
            check(
                side.x() == g.x() && side.y() == g.y() && side.h() == g.h() && g.x() >= 40 && g.y() > 10,
                &format!("rail fills the group: side {},{} {}x{}, group {},{} {}x{}", side.x(), side.y(), side.w(), side.h(), g.x(), g.y(), g.w(), g.h()),
            )?;
            let (rail, cx) = (side.w(), content.x());
            {
                let _m = Motion::on();
                side.hover(true);
                pump_until(|| side.is_expanded());
                check(side.is_expanded(), "hover intent expands")?;
                shot(&win, shots, "sidebar-nested-1-t0.png")?;
                ms(60);
                shot(&win, shots, "sidebar-nested-2-t60.png")?;
                let (mw, mx) = (side.w(), content.x());
                ms(300);
                shot(&win, shots, "sidebar-nested-3-end.png")?;
                let (ww, wx) = (side.w(), content.x());
                check(
                    rail < mw && mw < ww && cx < mx && mx < wx && wx - cx == ww - rail,
                    &format!("push: width {rail} < {mw} < {ww}, content {cx} < {mx} < {wx}"),
                )?;
                side.hover(false);
                pump_until(|| !side.is_expanded());
                shot(&win, shots, "sidebar-nested-4-closing.png")?;
                ms(300);
                shot(&win, shots, "sidebar-nested-4-closed.png")?;
                check(side.w() == rail && content.x() == cx, "collapses back")?;
            }
            side.set_pinned(true);
            shot(&win, shots, "sidebar-nested-5-pinned.png")?;
            check(side.w() > rail && content.x() > cx, "pinned: expanded, content moved")?;
            delete(win);
            // Overlay: the layer sits over the offset rail; content stays.
            let (win, side, content, _g) = build(SidebarMode::Overlay);
            shot(&win, shots, "sidebar-nested-6-overlay-0.png")?;
            let (rail, cx) = (side.w(), content.x());
            side.expand();
            let p = shot(&win, shots, "sidebar-nested-7-overlay-open.png")?;
            let r = side.item_rect(HOME).ok_or("no Home rect")?;
            check(
                side.w() == rail && content.x() == cx && side.overlay_width() > rail,
                "overlay: rail keeps its width, content does not move",
            )?;
            check(
                r.0 == side.x() && r.2 == side.overlay_width() && r.1 > side.y(),
                &format!("layer rows start at the rail: {r:?}, rail at {},{}", side.x(), side.y()),
            )?;
            // The panel covers the content where the content is: surface color there.
            let img = png_rgb(&p)?;
            let surf = theme::with(|t| rgb(t.palette.foreground.over(t.palette.background, 0.04)));
            let probe = (side.x() + rail + 20, side.y() + side.h() - 6);
            let above = (side.x() + rail + 20, side.y() - 4);
            check(
                near(img.at(probe.0, probe.1), surf) && !near(img.at(above.0, above.1), surf),
                &format!("panel over the content at {probe:?}, not above the group at {above:?}"),
            )?;
            side.set_pinned(true);
            side.collapse();
            shot(&win, shots, "sidebar-nested-8-overlay-pinned.png")?;
            check(side.is_expanded() && side.overlay_width() > rail && content.x() == cx, "overlay pinned stays open")?;
            side.set_pinned(false);
            shot(&win, shots, "sidebar-nested-9-overlay-unpinned.png")?;
            pump_until(|| side.overlay_width() == 0);
            check(side.overlay_width() == 0, "unpinned: closes")?;
            delete(win);
            Ok(())
        },
    );
}

/// Screen readers: headers are headings (foldable ones with their state),
/// secondary buttons are named by their tooltip, disabled rows say so.
#[cfg(feature = "a11y")]
pub fn a11y_checks(run: &mut dyn FnMut(&str, Check)) {
    run(
        "a11y: sidebar headings, secondary button named by its tooltip, disabled row, detail as description",
        &|shots| {
            anim::set_enabled(false);
            let s = shell(places(), SidebarMode::Push, 460);
            s.side.set_pinned(true);
            shot(&s.win, shots, "a11y-sidebar-files.png")?;
            let snap = fltk_omarchy::a11y::snapshot(&*s.win).ok_or("no tree")?;
            let has = |f: &dyn Fn(&str) -> bool| snap.lines().any(f);
            let ok = [
                ("Places heading", has(&|l| l.contains("Heading \"Places\"") && !l.contains("expanded"))),
                ("Devices heading expanded", has(&|l| l.contains("Heading \"Devices\"") && l.contains("expanded"))),
                ("Eject button", has(&|l| l.contains("Button \"Eject\""))),
                ("Locked disabled", has(&|l| l.contains("Tab \"Locked\"") && l.contains("disabled"))),
                ("System detail", has(&|l| l.contains("Tab \"System\"") && l.contains("desc \"12 GB free of 32 GB\""))),
                ("Home 1 of 7", has(&|l| l.contains("Tab \"Home\"") && l.contains("selected") && l.contains("1 of 7"))),
                ("usage", has(&|l| l.contains("ProgressIndicator"))),
            ];
            s.side.set_section_collapsed(DEVICES, true);
            shot(&s.win, shots, "a11y-sidebar-files-folded.png")?;
            let snap2 = fltk_omarchy::a11y::snapshot(&*s.win).ok_or("no tree")?;
            let folded = snap2.lines().any(|l| l.contains("Heading \"Devices\"") && l.contains("collapsed"))
                && !snap2.contains("Tab \"System\"");
            delete(s.win);
            let bad: Vec<&str> = ok.iter().filter(|(_, v)| !v).map(|(n, _)| *n).collect();
            check(bad.is_empty() && folded, &format!("missing {bad:?}, folded {folded}:\n{snap}\n{snap2}"))
        },
    );
}
