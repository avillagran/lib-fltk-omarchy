//! A bounded, per-renderer image page. No eviction or cross-scene cache.
use super::{Paint, PreparedScene, Texture};
use std::collections::{HashMap, HashSet};

pub(super) struct Plan {
    pub width: u32,
    pub height: u32,
    pub slots: HashMap<usize, (u32, u32)>,
}
impl Plan {
    pub fn new(scene: &PreparedScene, limit: u32) -> Option<Self> {
        let limit = limit.min(2048);
        let mut seen = HashSet::new();
        let mut images = Vec::new();
        let mut area = 0u64;
        for item in &scene.items {
            if let Paint::Image(image, _) = &item.paint {
                let key = image.pixels().as_ptr() as usize;
                if image.width() + 2 <= limit && image.height() + 2 <= limit && seen.insert(key) {
                    images.push((key, image.width() + 2, image.height() + 2));
                    area += image.pixels().len() as u64;
                }
            }
        }
        if images.len() < 2 { return None; }
        images.sort_by_key(|i| std::cmp::Reverse(i.2));
        let minimum = images.iter().map(|i| i.1).max()?;
        let mut widths = vec![minimum];
        let mut width = minimum.next_power_of_two();
        while width <= limit { widths.push(width); width *= 2; }
        let mut best: Option<Self> = None;
        for width in widths {
            let (mut x, mut y, mut row) = (0, 0, 0);
            let mut slots = HashMap::new();
            for &(key, w, h) in &images {
                if x + w > width { x = 0; y += row; row = 0; }
                slots.insert(key, (x + 1, y + 1));
                x += w;
                row = row.max(h);
            }
            let height = y + row;
            let bytes = width as u64 * height as u64 * 4;
            if height <= limit && bytes <= area * 2 + 4096
                && best.as_ref().map_or(true, |b| bytes < b.width as u64 * b.height as u64 * 4) {
                best = Some(Self { width, height, slots });
            }
        }
        best
    }
}

pub(super) fn padded(image: &Texture) -> Vec<u8> {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let mut pixels = vec![0; (w + 2) * (h + 2) * 4];
    for y in 0..h+2 {
        for x in 0..w+2 {
            let source = (y.saturating_sub(1).min(h-1) * w + x.saturating_sub(1).min(w-1)) * 4;
            let target = (y * (w+2) + x) * 4;
            let p = &image.pixels()[source..source+4];
            pixels[target..target+4].copy_from_slice(p);
            if !image.premultiplied {
                for channel in 0..3 { pixels[target+channel] = (p[channel] as u32 * p[3] as u32 / 255) as u8; }
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gutters_extrude_premultiplied_edge_pixels() {
        let image = Texture { width: 2, height: 1, pixels: vec![255,0,0,128, 0,255,0,255].into(), premultiplied: false };
        let pixels = padded(&image);
        let row = [128,0,0,128, 128,0,0,128, 0,255,0,255, 0,255,0,255];
        assert_eq!(pixels, row.repeat(3));
    }
    #[test]
    fn page_requires_distinct_resources_and_fits_padded_bounds() {
        use crate::scene::{Image, Rect, Scene};
        let a = Image::rgba(8,8,vec![255;8*8*4]).unwrap();
        let b = Image::rgba(8,8,vec![128;8*8*4]).unwrap();
        let prepare = |distinct| {
            let (scene, ()) = Scene::record(|| {
                a.draw(Rect::new(0,0,8,8));
                if distinct { b.draw(Rect::new(8,0,8,8)); }
                else { a.draw(Rect::new(8,0,8,8)); }
            });
            PreparedScene::new(&scene,Rect::new(0,0,32,32)).unwrap()
        };
        assert!(Plan::new(&prepare(false),32).is_none());
        assert!(Plan::new(&prepare(true),16).is_none());
        let plan=Plan::new(&prepare(true),32).unwrap();
        assert_eq!(plan.slots.len(),2);
        for &(x,y) in plan.slots.values() {
            assert!(x>=1 && y>=1 && x+8<plan.width && y+8<plan.height);
        }
    }
}
