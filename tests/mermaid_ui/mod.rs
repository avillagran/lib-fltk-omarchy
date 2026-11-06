//! UI checks of `mermaid` (feature `mermaid`): a `MermaidView`'s nodes,
//! edges and arrowheads are where its scene says, it restyles with the
//! theme and caches its picture; a `RichText` with `native_mermaid` draws
//! closed fences as diagrams, toggles "Show source", and keeps a fence
//! that is still streaming as code.
use super::{check, png_rgb};
use fltk::prelude::*;
use fltk_omarchy::focus;
use fltk_omarchy::layout::FlexExt;
use fltk_omarchy::mermaid::{self, Item, MermaidView, Role};
use fltk_omarchy::text::{Message, RichText};
use fltk_omarchy::theme::{self, Rgb};
use fltk_omarchy::AppWindow;
use std::path::{Path, PathBuf};

type Check<'a> = &'a dyn Fn(&Path) -> Result<(), String>;

fn shot(win: &AppWindow, dir: &Path, name: &str) -> Result<PathBuf, String> {
    let p = dir.join(name);
    win.clone().snapshot(&p).map_err(|e| e.to_string())?;
    Ok(p)
}

fn delete(win: AppWindow) {
    fltk::window::DoubleWindow::delete((*win).clone());
}

fn rgb(c: Rgb) -> [u8; 3] {
    [c.r(), c.g(), c.b()]
}

fn near(a: [u8; 3], b: [u8; 3], tol: u8) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= tol)
}

fn dist(a: [u8; 3], b: [u8; 3]) -> u32 {
    a.iter().zip(b).map(|(x, y)| x.abs_diff(y) as u32).sum()
}

fn bounds(points: &[[f32; 2]]) -> [f32; 4] {
    points.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

/// Pixels of the window within `tol` of `c`.
fn count(img: &super::Rgb, w: i32, h: i32, c: [u8; 3], tol: u8, area: (i32, i32, i32, i32)) -> usize {
    let (x0, y0, x1, y1) = (area.0.max(0), area.1.max(0), (area.0 + area.2).min(w), (area.1 + area.3).min(h));
    (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| near(img.at(x, y), c, tol)).count()
}

pub fn checks(run: &mut dyn FnMut(&str, Check)) {
    run("mermaid: MermaidView nodes, edge and arrowhead pixels; theme restyle; cached picture", &|shots| {
        let mut win = AppWindow::new("m", 520, 420);
        let mut body = win.body();
        body.set_margin(10);
        let mut view = MermaidView::new("graph TD\n  A[Alpha] --> B[Beta]");
        let h = view.preferred_height(500);
        view.set_size(500, h);
        body.fit(&*view);
        win.end();
        let p = shot(&win, shots, "mermaid-view.png")?;
        let d = view.diagram().ok_or("diagram")?;
        let (ox, oy, dw, dh) = view.diagram_rect().ok_or("rect")?;
        check(dw > 40 && dh > 60 && d.scene.items.len() > 4, &format!("diagram {dw}x{dh}, {} items", d.scene.items.len()))?;
        let img = png_rgb(&p)?;
        let (fill, line, bg) = theme::with(|t| {
            let bg = t.palette.background;
            (rgb(mermaid::color(Role::NodeFill, t, bg)), rgb(mermaid::color(Role::Line, t, bg)), rgb(bg))
        });
        // Inside each node, left of its label: the node fill.
        let nodes: Vec<[f32; 4]> = d.scene.items.iter().filter_map(|i| match i { Item::Fill { points, role: Role::NodeFill } => Some(bounds(points)), _ => None }).collect();
        check(nodes.len() == 2, &format!("two node fills: {}", nodes.len()))?;
        for b in &nodes {
            let (x, y) = (ox + b[0] as i32 + 5, oy + ((b[1] + b[3]) / 2.0) as i32);
            let c = img.at(x, y);
            check(near(c, fill, 3), &format!("node fill at ({x},{y}): {c:?} vs {fill:?}"))?;
        }
        // Node A above node B (top down).
        check(nodes[0][3] < nodes[1][1], "A above B")?;
        // The edge between them: line ink on its middle.
        let edge = d.scene.items.iter().find_map(|i| match i { Item::Stroke { points, role: Role::Line, .. } => Some(points.clone()), _ => None }).ok_or("edge")?;
        let mid = edge[edge.len() / 2];
        let (mx, my) = (ox + mid[0].round() as i32, oy + mid[1].round() as i32);
        let best = (-1..=1).map(|dx| img.at(mx + dx, my)).min_by_key(|c| dist(*c, line)).ok_or("px")?;
        check(dist(best, line) * 2 < dist(best, bg), &format!("edge ink at ({mx},{my}): {best:?} line {line:?} bg {bg:?}"))?;
        // It starts at A's bottom border and its arrowhead ends at B's top border.
        let b = bounds(&edge);
        check((b[1] - nodes[0][3]).abs() < 2.0, &format!("edge starts at A's border: {} vs {}", b[1], nodes[0][3]))?;
        let head = d.scene.items.iter().find_map(|i| match i { Item::Fill { points, role: Role::Line } => Some(bounds(points)), _ => None }).ok_or("arrowhead")?;
        check((head[3] - nodes[1][1]).abs() < 1.5, &format!("arrow tip on B's border: {} vs {}", head[3], nodes[1][1]))?;
        let (hx, hy) = (ox + ((head[0] + head[2]) / 2.0) as i32, oy + (head[1] * 0.3 + head[3] * 0.7) as i32);
        let c = img.at(hx, hy);
        check(dist(c, line) < 40, &format!("arrowhead ink {c:?} vs {line:?}"))?;
        // Outside the diagram: the background.
        check(near(img.at(ox + 1, oy + dh - 2), bg, 2) || near(img.at(ox + dw - 2, oy + 1), bg, 2), "background around the nodes")?;
        // Cached: a second paint does not rasterize again.
        let renders = view.stats().1;
        shot(&win, shots, "mermaid-view.png")?;
        check(view.stats().1 == renders, &format!("cached picture: {} renders after {renders}", view.stats().1))?;
        // Live theme: another accent restyles the nodes.
        let old = theme::current();
        let mut t = old.clone();
        t.palette.accent = Rgb(0xff3040);
        theme::set(t);
        let p = shot(&win, shots, "mermaid-view-theme.png")?;
        let img = png_rgb(&p)?;
        let fill2 = theme::with(|t| rgb(mermaid::color(Role::NodeFill, t, t.palette.background)));
        let b = nodes[0];
        let c = img.at(ox + b[0] as i32 + 5, oy + ((b[1] + b[3]) / 2.0) as i32);
        theme::set(old);
        check(fill2 != fill && near(c, fill2, 3), &format!("restyled fill {c:?} vs {fill2:?} (was {fill:?})"))?;
        check(view.stats().1 > renders, "the theme change rasterized again")?;
        // A source that does not render shows the error.
        view.set_source("pie\n  \"a\": 1");
        check(matches!(view.error(), Some(mermaid::Error::Unsupported(_))), &format!("error {:?}", view.error()))?;
        shot(&win, shots, "mermaid-view-error.png")?;
        delete(win);
        Ok(())
    });
    run("mermaid: RichText draws closed fences natively, Show source toggles, open fences stay code", &|shots| {
        let mut win = AppWindow::new("m", 640, 620);
        let mut body = win.body();
        let rt = RichText::new().native_mermaid(true);
        body.add(&*rt);
        win.end();
        rt.set_message("a", Message::markdown("Flow:\n\n```mermaid\ngraph LR\n  A[One] --> B[Two]\n```\n\nAfter."));
        let p = shot(&win, shots, "mermaid-richtext.png")?;
        let (ww, wh) = (win.w(), win.h());
        let tint = theme::with(|t| rgb(mermaid::color(Role::NodeFill, t, t.palette.lighter_background)));
        let img = png_rgb(&p)?;
        let k = (0..rt.block_count()).find(|&k| rt.block(k).is_some_and(|b| b.is_code())).ok_or("code block")?;
        let src = rt.block(k).ok_or("block")?.text().to_string();
        check(rt.mermaid_diagram(&src).is_some(), "diagram laid out")?;
        let drawn = count(&img, ww, wh, tint, 2, (0, 0, ww, wh));
        check(drawn > 300, &format!("node fills drawn: {drawn} px"))?;
        let (ax, ay, aw, ah) = rt.action_rect(k, mermaid::SHOW_SOURCE).ok_or("Show source action")?;
        check(rt.text_rect("After").is_some(), "text after the diagram")?;
        // Show source: the code comes back, the diagram goes.
        focus::click_at(&*win, ax + aw / 2, ay + ah / 2);
        let p = shot(&win, shots, "mermaid-richtext-source.png")?;
        let img = png_rgb(&p)?;
        let left = count(&img, ww, wh, tint, 2, (0, 0, ww, wh));
        check(left < 20, &format!("diagram hidden: {left} px"))?;
        check(rt.action_rect(k, mermaid::SHOW_DIAGRAM).is_some(), "Show diagram action")?;
        check(rt.text_rect("graph LR").is_some() || rt.text_rect("graph").is_some(), "the source is shown")?;
        let (ax, ay, aw, ah) = rt.action_rect(k, mermaid::SHOW_DIAGRAM).ok_or("Show diagram")?;
        focus::click_at(&*win, ax + aw / 2, ay + ah / 2);
        let p = shot(&win, shots, "mermaid-richtext-back.png")?;
        let img = png_rgb(&p)?;
        check(count(&img, ww, wh, tint, 2, (0, 0, ww, wh)) > 300, "the diagram is back")?;
        // Streaming: an open fence stays code until it closes.
        rt.set_message("b", Message::markdown("Next:\n\n```mermaid\ngraph TD\n  X[Ex] --> Y[Why]\n"));
        let p = shot(&win, shots, "mermaid-richtext-open.png")?;
        let kb = (0..rt.block_count()).rev().find(|&k| rt.block(k).is_some_and(|b| b.is_code())).ok_or("open block")?;
        check(kb != k, "second code block")?;
        let open_src = rt.block(kb).ok_or("block")?.text().to_string();
        check(rt.mermaid_diagram(&open_src).is_none(), "open fence: no diagram")?;
        let img = png_rgb(&p)?;
        let before = count(&img, ww, wh, tint, 2, (0, 0, ww, wh));
        rt.set_message("b", Message::markdown("Next:\n\n```mermaid\ngraph TD\n  X[Ex] --> Y[Why]\n```\n"));
        let p = shot(&win, shots, "mermaid-richtext-closed.png")?;
        let closed_src = rt.block(kb).ok_or("block")?.text().to_string();
        check(rt.mermaid_diagram(&closed_src).is_some(), &format!("closed fence: diagram ({closed_src:?})"))?;
        let img = png_rgb(&p)?;
        let after = count(&img, ww, wh, tint, 2, (0, 0, ww, wh));
        check(after > before + 200, &format!("closed fence drawn: {before} → {after} px"))?;
        delete(win);
        Ok(())
    });
}
