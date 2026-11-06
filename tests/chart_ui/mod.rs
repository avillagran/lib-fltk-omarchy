//! UI checks of `chart`: bar and line charts (pixels, keyboard cursor,
//! select, right to left), the meter's colors and expected mark, the ring.
use super::{check, pixel, pump_until};
use fltk::prelude::*;
use fltk_omarchy::chart::{Bar, BarChart, Line, LineChart, Meter, MeterValue, Ring};
use fltk_omarchy::focus::press;
use fltk_omarchy::layout::{self, Direction, FlexExt};
use fltk_omarchy::text::ColorRole;
use fltk_omarchy::theme::{self, Rgb};
use fltk_omarchy::{AppWindow, State};
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn window() -> AppWindow {
    let win = AppWindow::new("Chart", 420, 320);
    win.body().set_margin(10);
    win
}

fn show(mut win: AppWindow) -> AppWindow {
    win.end();
    win.show();
    let body = win.body();
    pump_until(|| body.w() > 0 && body.children() > 0 && body.child(0).is_some_and(|c| c.w() > 0));
    win
}

fn close(win: AppWindow) {
    fltk::window::DoubleWindow::delete((*win).clone());
}

fn px(win: &AppWindow, shots: &Path, name: &str, x: i32, y: i32) -> Result<Rgb, String> {
    let p = shots.join(name);
    win.clone().snapshot(&p).map_err(|e| e.to_string())?;
    let [r, g, b] = pixel(&p, x, y)?;
    Ok(Rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32))
}

fn near(a: Rgb, b: Rgb) -> bool {
    a.r().abs_diff(b.r()) <= 3 && a.g().abs_diff(b.g()) <= 3 && a.b().abs_diff(b.b()) <= 3
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("chart: bars, highlighted bar, keys, select, right to left", &|shots| {
        let data = State::new(vec![Bar::new("Mon", 10.0, "Mon · 10"), Bar::new("Tue", 5.0, "Tue · 5"), Bar::new("Wed", 8.0, "Wed · 8"), Bar::new("Thu", 2.0, "Thu · 2")]);
        let picked = Rc::new(Cell::new(usize::MAX));
        let win = window();
        let mut body = win.body();
        let p2 = picked.clone();
        let chart = BarChart::new(&data).on_select(move |i| p2.set(i));
        body.fit(&*chart);
        let win = show(win);
        let (accent, bg) = theme::with(|t| (t.palette.accent, t.palette.background));
        let soft = accent.over(bg, 0.45);
        let slot = chart.w() / 4;
        let bottom = chart.y() + chart.h() - 30; // inside the plot, above the axis labels
        let bar_x = |i: i32| chart.x() + slot * i + slot / 2;
        let c = px(&win, shots, "chart-bars.png", bar_x(0), bottom)?;
        check(near(c, soft), &format!("bar 0 soft accent: {c:?} vs {soft:?}"))?;
        let mut ch = (*chart).clone();
        let _ = ch.take_focus();
        check(chart.cursor() == Some(3), &format!("focus puts the cursor on the last bar: {:?}", chart.cursor()))?;
        press("Home");
        press("Right");
        check(chart.cursor() == Some(1), &format!("Home, Right → 1: {:?}", chart.cursor()))?;
        let c = px(&win, shots, "chart-bars-cursor.png", bar_x(1), bottom)?;
        check(near(c, accent), &format!("highlighted bar is the accent: {c:?}"))?;
        press("Enter");
        check(picked.get() == 1, &format!("Enter selects: {}", picked.get()))?;
        press("End");
        press("Right");
        check(chart.cursor() == Some(3), "End stays on the last bar")?;
        // Thu (value 2) is short: above it is background.
        let c = px(&win, shots, "chart-bars-short.png", bar_x(3), chart.y() + 40)?;
        check(near(c, bg), &format!("above a short bar: {c:?}"))?;
        // Right to left: bar 0 (the tallest) is drawn on the right.
        layout::set_direction(&*chart, Some(Direction::Rtl));
        let c = px(&win, shots, "chart-bars-rtl.png", bar_x(3), chart.y() + 40)?;
        check(near(c, soft), &format!("RTL: the tallest bar is on the right: {c:?}"))?;
        layout::set_direction(&*chart, None);
        close(win);
        Ok(())
    });

    run("chart: meter colors and expected mark, ring arc", &|shots| {
        let value = State::new(MeterValue { used: 0.4, expected: Some(0.6) });
        let ring_v = State::new(0.25);
        let win = window();
        let mut body = win.body();
        let mut meter = Meter::new(&value);
        meter.set_size(200, 10);
        body.fit(&*meter);
        let mut ring = Ring::new(&ring_v).color(ColorRole::Green).thickness(4.0);
        ring.set_size(40, 40);
        body.fit(&*ring);
        let win = show(win);
        let (accent, yellow, urgent, green, bg) = theme::with(|t| (t.palette.accent, t.palette.yellow, t.palette.urgent, t.palette.green, t.palette.background));
        let my = meter.y() + meter.h() / 2;
        let at = |f: f64| meter.x() + (meter.w() as f64 * f) as i32;
        let c = px(&win, shots, "chart-meter.png", at(0.2), my)?;
        check(near(c, accent), &format!("behind the expectation: accent {c:?}"))?;
        let c = px(&win, shots, "chart-meter-mark.png", at(0.6), my)?;
        check(!near(c, accent) && c.luminance() > bg.luminance() + 0.2, &format!("expected mark drawn: {c:?}"))?;
        value.set(MeterValue { used: 0.8, expected: Some(0.6) });
        let c = px(&win, shots, "chart-meter-ahead.png", at(0.2), my)?;
        check(near(c, yellow), &format!("ahead of the expectation: yellow {c:?}"))?;
        value.set(MeterValue { used: 1.2, expected: None });
        let c = px(&win, shots, "chart-meter-full.png", at(0.95), my)?;
        check(near(c, urgent), &format!("over 100 %: urgent {c:?}"))?;
        // Ring at 25 %: from the top clockwise to 3 o'clock.
        let (cx, cy) = (ring.x() + ring.w() / 2, ring.y() + ring.h() / 2);
        let r = (ring.w().min(ring.h()) - 4) / 2;
        let d = (r as f64 * std::f64::consts::FRAC_1_SQRT_2) as i32;
        let c = px(&win, shots, "chart-ring.png", cx + d, cy - d)?;
        check(near(c, green), &format!("ring arc (top right) is green: {c:?}"))?;
        let c = px(&win, shots, "chart-ring.png", cx - d, cy + d)?;
        check(!near(c, green), &format!("ring track (bottom left) is not: {c:?}"))?;
        close(win);
        Ok(())
    });

    run("chart: line chart cursor, guide and fixed range", &|shots| {
        let data = State::new(vec![
            Line::new("CPU", vec![10.0, 40.0, 20.0, 80.0, 30.0], ColorRole::Accent),
            Line::new("Mem", vec![50.0, 50.0, 55.0, 60.0, 58.0], ColorRole::Blue),
        ]);
        let win = window();
        let mut body = win.body();
        let chart = LineChart::new(&data).range(0.0, 100.0);
        body.fit(&*chart);
        let win = show(win);
        let mut ch = (*chart).clone();
        let _ = ch.take_focus();
        check(chart.cursor() == Some(4), "focus → last sample")?;
        press("Left");
        press("Left");
        check(chart.cursor() == Some(2), &format!("Left ×2 → 2: {:?}", chart.cursor()))?;
        press("Home");
        check(chart.cursor() == Some(0), "Home → 0")?;
        // Sample 3 (CPU 80) on a 0..100 range: the CPU line is drawn high up.
        press("End");
        press("Left");
        let p = shots.join("chart-line.png");
        win.clone().snapshot(&p).map_err(|e| e.to_string())?;
        let x = chart.x() + chart.w() * 3 / 4;
        let accent = theme::with(|t| t.palette.accent);
        let hit = (chart.y()..chart.y() + chart.h() / 2).any(|y| {
            pixel(&p, x, y).is_ok_and(|[r, g, b]| near(Rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32), accent))
        });
        check(hit, "the CPU line peaks in the upper half at sample 3")?;
        close(win);
        Ok(())
    });
}
