//! Retained CPU-widget overlay bridge. Capture outside native GL drawing;
//! retain premultiplied bytes without a lossy straight-alpha round trip.
use super::{Rect, MAX_IMAGE_BYTES};
use fltk::{draw, enums::Color, prelude::*, surface::ImageSurface, widget::Widget};

pub(super) struct Capture {
    pub rect: Rect,
    pub pixels: Vec<u8>,
}
#[derive(Default)]
pub(super) struct Cache {
    stamps: Vec<(usize, i32, i32, i32, i32, u64)>,
    geometry: Option<(i32, i32, i32, i32, u64, u64)>,
    pub capture: Option<Capture>,
}
pub(super) struct Update {
    pub changed: bool,
    pub captured: bool,
}
impl Cache {
    pub fn update<W: WidgetExt>(&mut self, host: &W) -> Result<Update, String> {
        let mut layers = crate::overlay::layers_over(host);
        let area = Rect::new(host.x(), host.y(), host.w(), host.h());
        layers.retain(|w| area.intersects(Rect::new(w.x(), w.y(), w.w(), w.h())));
        let stamps: Vec<_> = layers
            .iter()
            .map(|w| {
                (
                    w.as_widget_ptr() as usize,
                    w.x(),
                    w.y(),
                    w.w(),
                    w.h(),
                    crate::overlay::capture_phase(w),
                )
            })
            .collect();
        let geometry = Some((
            host.x(),
            host.y(),
            host.w(),
            host.h(),
            crate::theme::generation(),
            crate::overlay::layers_revision(),
        ));
        let changed =
            self.stamps != stamps || self.geometry != geometry || layers.iter().any(|w| w.damage());
        if !changed {
            return Ok(Update {
                changed: false,
                captured: false,
            });
        }
        let captured = !layers.is_empty();
        let capture = if captured {
            crate::anim::sample_time(|| capture(area, &layers))?
        } else {
            None
        };
        let changed = match (&self.capture, &capture) {
            (None, None) => false,
            (Some(a), Some(b)) => a.rect != b.rect || a.pixels != b.pixels,
            _ => true,
        };
        self.stamps = stamps;
        self.geometry = geometry;
        self.capture = capture;
        Ok(Update { changed, captured })
    }
}
fn has_native_window(widget: &Widget) -> bool {
    if widget.as_window().is_some() {
        return true;
    }
    widget.as_group().is_some_and(|g| {
        (0..g.children())
            .filter_map(|i| g.child(i))
            .any(|c| c.visible() && has_native_window(&c))
    })
}
fn capture(area: Rect, layers: &[Widget]) -> Result<Option<Capture>, String> {
    let bytes = (area.w.max(0) as usize)
        .checked_mul(area.h.max(0) as usize)
        .and_then(|n| n.checked_mul(4));
    if bytes.map_or(true, |n| n == 0 || n > MAX_IMAGE_BYTES) {
        return Err("overlay capture exceeds the image resource budget (64 MiB)".into());
    }
    if layers.iter().any(has_native_window) {
        return Err(
            "overlay bridge requires CPU widget subtrees, without native subwindows".into(),
        );
    }
    let surface = ImageSurface::new(area.w, area.h, false);
    ImageSurface::push_current(&surface);
    draw::push_no_clip();
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            draw::pop_clip();
            ImageSurface::pop_current();
        }
    }
    let _restore = Restore;
    let pass = |color| -> Result<Vec<u8>, String> {
        draw::set_draw_color(color);
        draw::draw_rectf(0, 0, area.w, area.h);
        super::surface::cpu_scope(|| {
            for layer in layers {
                super::surface::draw_cpu_widget(
                    &surface,
                    layer,
                    layer.x() - area.x,
                    layer.y() - area.y,
                );
            }
        });
        let image = surface
            .image()
            .ok_or("overlay native image surface failed")?;
        if image.data_w() != area.w
            || image.data_h() != area.h
            || image.depth() != fltk::enums::ColorDepth::Rgb8
        {
            return Err("overlay native image surface has unexpected dimensions or depth".into());
        }
        Ok(image.to_rgb_data())
    };
    let black = pass(Color::Black)?;
    let white = pass(Color::White)?;
    let mut pixels = Vec::with_capacity(bytes.unwrap());
    let (mut left, mut top, mut right, mut bottom) = (area.w, area.h, 0, 0);
    for (i, (b, w)) in black.chunks_exact(3).zip(white.chunks_exact(3)).enumerate() {
        let alpha = 255 - (0..3).map(|c| w[c].saturating_sub(b[c])).min().unwrap();
        pixels.extend_from_slice(b);
        pixels.push(alpha);
        if alpha > 0 {
            let (x, y) = ((i % area.w as usize) as i32, (i / area.w as usize) as i32);
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    if right <= left || bottom <= top {
        return Ok(None);
    }
    let rect = Rect::new(left, top, right - left, bottom - top);
    if (rect.w, rect.h) != (area.w, area.h) {
        let mut cropped = Vec::with_capacity(rect.w as usize * rect.h as usize * 4);
        for y in top..bottom {
            let at = (y as usize * area.w as usize + left as usize) * 4;
            cropped.extend_from_slice(&pixels[at..at + rect.w as usize * 4]);
        }
        pixels = cropped;
    }
    Ok(Some(Capture { rect, pixels }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_capture_bounds_before_native_allocation() {
        assert!(capture(Rect::new(0, 0, 0, 1), &[]).is_err());
        assert!(capture(Rect::new(0, 0, i32::MAX, i32::MAX), &[]).is_err());
        assert!(capture(Rect::new(0, 0, -1, 10), &[]).is_err());
    }
}
