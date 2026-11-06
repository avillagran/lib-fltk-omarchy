//! [`MermaidView`]: a widget showing a Mermaid source as a native diagram.

use super::{render, Diagram, Error, Picture};
use crate::a11y::{self, Role as A11yRole};
use crate::state::State;
use crate::text::ColorRole;
use crate::{paint, theme};
use fltk::{enums::*, prelude::*, widget::Widget};
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

struct ViewState {
    src: String,
    /// (source hash, width, theme generation) the result was laid out for.
    laid: Option<(u64, i32, u64)>,
    result: Option<Result<Diagram, Error>>,
    pic: Picture,
    pad: i32,
    surface: Option<ColorRole>,
    fit_height: bool,
    layouts: u64,
}

impl ViewState {
    /// Lays the source out for a widget `w` wide (cached).
    fn ensure(&mut self, w: i32) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.src.hash(&mut h);
        let key = (h.finish(), w, theme::generation());
        if self.laid != Some(key) {
            self.result = Some(render(&self.src, (w - 2 * self.pad).max(40) as f32));
            self.laid = Some(key);
            self.layouts += 1;
        }
    }

    fn height(&self) -> i32 {
        match &self.result {
            Some(Ok(d)) => d.height.ceil() as i32 + 2 * self.pad,
            _ => theme::with(|t| (t.font.body as f32 * 1.4) as i32 * 3 + 2 * self.pad),
        }
    }
}

/// A Mermaid flowchart, sequence or state diagram drawn natively.
///
/// The diagram is laid out for the widget's width (wider ones turn
/// top-down or shrink, see [`super::Options`]) and kept until the source,
/// the width or the theme change; the shapes are rasterized at the
/// screen's pixel density and cached, so a redraw is one image blit plus
/// the labels. A source that does not render shows the error and the
/// source text. With [`fit_height`](Self::fit_height) (on by default)
/// setting a source resizes the widget to the diagram's height.
///
/// ```no_run
/// # #[cfg(feature = "mermaid")] {
/// use fltk_omarchy::{mermaid::MermaidView, State};
/// # let _app = fltk_omarchy::App::new("dev.example.mermaid");
/// let src = State::new("graph TD\n  A --> B".to_string());
/// let view = MermaidView::new("").bind(&src);
/// # }
/// ```
#[derive(Clone)]
pub struct MermaidView {
    inner: Widget,
    st: Rc<RefCell<ViewState>>,
}
fltk::widget_extends!(MermaidView, Widget, inner);

impl MermaidView {
    pub fn new(src: &str) -> MermaidView {
        let mut inner = Widget::default();
        inner.set_frame(FrameType::NoBox);
        inner.set_size(480, 160);
        let pad = theme::with(|t| t.spacing.sm.max(4));
        let st = Rc::new(RefCell::new(ViewState {
            src: src.to_string(),
            laid: None,
            result: None,
            pic: Picture::new(),
            pad,
            surface: None,
            fit_height: true,
            layouts: 0,
        }));
        let s = st.clone();
        inner.draw(move |w| draw(w, &mut s.borrow_mut()));
        let s = st.clone();
        a11y::describe(&inner, move |n| {
            let s = s.borrow();
            let name = match &s.result {
                Some(Ok(d)) => d.summary.clone(),
                Some(Err(e)) => crate::tr!("Diagram: {reason}", reason = e.to_string()),
                None => crate::tr!("Diagram"),
            };
            n.role(A11yRole::Image).name(&name);
        });
        let v = MermaidView { inner, st };
        v.refit();
        v
    }

    /// Follows a bound source.
    pub fn bind(self, src: &State<String>) -> Self {
        let v = self.clone();
        crate::state::bind_widget(&self.inner, src, true, move |s: &String| v.set_source(s));
        self
    }

    /// Replaces the source.
    pub fn set_source(&self, src: &str) {
        {
            let mut s = self.st.borrow_mut();
            if s.src == src {
                return;
            }
            s.src = src.to_string();
        }
        self.refit();
        self.inner.clone().redraw();
    }

    pub fn source(&self) -> String {
        self.st.borrow().src.clone()
    }

    /// Paints over this theme role instead of the window background
    /// (`ColorRole::LighterBackground` for a card).
    pub fn surface(self, role: ColorRole) -> Self {
        self.st.borrow_mut().surface = Some(role);
        self.inner.clone().redraw();
        self
    }

    /// Resize to the diagram's height when the source changes (default on).
    pub fn fit_height(self, on: bool) -> Self {
        self.st.borrow_mut().fit_height = on;
        self.refit();
        self
    }

    /// The diagram laid out for the current width (None before the first
    /// layout or when the source does not render).
    pub fn diagram(&self) -> Option<Diagram> {
        let mut s = self.st.borrow_mut();
        let w = self.inner.w();
        s.ensure(w);
        s.result.as_ref().and_then(|r| r.as_ref().ok().cloned())
    }

    /// Why the source does not render.
    pub fn error(&self) -> Option<Error> {
        let mut s = self.st.borrow_mut();
        let w = self.inner.w();
        s.ensure(w);
        s.result.as_ref().and_then(|r| r.as_ref().err().cloned())
    }

    /// Window rectangle where the diagram is drawn (hit tests, tests).
    pub fn diagram_rect(&self) -> Option<(i32, i32, i32, i32)> {
        let d = self.diagram()?;
        let pad = self.st.borrow().pad;
        let ww = self.inner.w();
        let dx = ((ww - 2 * pad) as f32 - d.width).max(0.0) as i32 / 2;
        Some((self.inner.x() + pad + dx, self.inner.y() + pad, d.width.ceil() as i32, d.height.ceil() as i32))
    }

    /// The height the widget needs at `width`.
    pub fn preferred_height(&self, width: i32) -> i32 {
        let mut s = self.st.borrow_mut();
        s.ensure(width);
        s.height()
    }

    /// Layouts and shape renders so far (tests, benchmarks).
    pub fn stats(&self) -> (u64, u64) {
        let s = self.st.borrow();
        (s.layouts, s.pic.renders())
    }

    fn refit(&self) {
        if !self.st.borrow().fit_height {
            return;
        }
        let w = self.inner.w();
        if w <= 0 {
            return;
        }
        let h = self.preferred_height(w);
        if h != self.inner.h() {
            let mut me = self.inner.clone();
            me.resize(me.x(), me.y(), w, h);
            if let Some(mut p) = me.parent() {
                p.redraw();
            }
        }
    }
}

fn draw(w: &mut Widget, s: &mut ViewState) {
    let (x, y, ww, wh) = (w.x(), w.y(), w.w(), w.h());
    let under = theme::with(|t| s.surface.map_or(t.palette.background, |r| r.rgb(t)));
    if s.surface.is_some() {
        theme::with(|t| crate::glass::fill(x, y, ww, wh, t.radius.min(8), under));
    } else {
        paint::fill(x, y, ww, wh, 0, under);
    }
    s.ensure(ww);
    fltk::draw::push_clip(x, y, ww, wh);
    let pad = s.pad;
    let unit = crate::native::pixels_per_unit(w);
    match s.result.take() {
        Some(Ok(d)) => {
            let dx = ((ww - 2 * pad) as f32 - d.width).max(0.0) as i32 / 2;
            s.pic.draw(&d, x + pad + dx, y + pad, unit, under);
            s.result = Some(Ok(d));
        }
        Some(Err(e)) => {
            theme::with(|t| {
                let lh = (t.font.body as f32 * 1.4) as i32;
                let dim = t.palette.foreground.darker(1.4);
                paint::text(&crate::tr!("Diagram: {reason}", reason = e.to_string()), x + pad, y + pad, ww - 2 * pad, lh, t.font.caption, false, dim, Align::Left);
                fltk::draw::set_font(Font::Courier, t.font.body);
                fltk::draw::set_draw_color(t.palette.foreground.fl());
                fltk::draw::draw_text2(&paint::escape(&s.src), x + pad, y + pad + lh, ww - 2 * pad, wh - 2 * pad - lh, Align::Left | Align::Top | Align::Inside | Align::Clip);
            });
            s.result = Some(Err(e));
        }
        None => {}
    }
    fltk::draw::pop_clip();
}
