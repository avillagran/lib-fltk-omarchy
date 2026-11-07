//! Current-scene image residency. Entries use immutable resource IDs, not
//! pixel addresses, and do not retain a second CPU copy of uploaded pixels.
use super::{atlas, Paint, PreparedScene, ResourceStats, Texture};
use glow::HasContext;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
pub(super) struct Slot {
    pub texture: glow::Texture,
    pub region: [f32; 4],
}

#[derive(Default)]
pub(super) struct Images {
    resident: HashMap<u64, Slot>,
    allocations: HashMap<glow::Texture, usize>,
    pub bytes: usize,
}
impl Images {
    pub fn get(&self, identity: u64) -> Slot {
        self.resident[&identity]
    }
    pub fn len(&self) -> usize {
        self.resident.len()
    }
    pub fn objects(&self) -> usize {
        self.allocations.len()
    }

    pub unsafe fn clear(&mut self, gl: &glow::Context) {
        for (texture, _) in self.allocations.drain() {
            gl.delete_texture(texture);
        }
        self.resident.clear();
        self.bytes = 0;
    }

    pub unsafe fn replace(
        &mut self,
        gl: &glow::Context,
        scene: &PreparedScene,
        budget: usize,
    ) -> Result<ResourceStats, String> {
        let mut unique = HashMap::new();
        for item in &scene.items {
            if let Paint::Image(image, _) = &item.paint {
                unique.entry(image.identity).or_insert(image);
            }
        }
        let max = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) as u32;
        if unique
            .values()
            .any(|image| image.width() > max || image.height() > max)
        {
            return Err("scene image exceeds GL texture size".into());
        }
        // A page is deleted only when its last current-scene tile leaves.
        // Vacant tiles need no CPU backing or stale-ID metadata.
        self.resident.retain(|id, _| unique.contains_key(id));
        let live: HashSet<_> = self.resident.values().map(|slot| slot.texture).collect();
        self.allocations.retain(|texture, bytes| {
            if live.contains(texture) {
                true
            } else {
                gl.delete_texture(*texture);
                self.bytes -= *bytes;
                false
            }
        });
        if self.resident.capacity() > 256
            && self.resident.len().saturating_mul(4) < self.resident.capacity()
        {
            self.resident.shrink_to(self.resident.len().max(64));
        }
        if self.allocations.capacity() > 256
            && self.allocations.len().saturating_mul(4) < self.allocations.capacity()
        {
            self.allocations.shrink_to(self.allocations.len().max(64));
        }
        let mut missing: Vec<_> = unique
            .values()
            .copied()
            .filter(|image| !self.resident.contains_key(&image.identity))
            .collect();
        missing.sort_by_key(|image| image.identity);
        let mut stats = ResourceStats::default();
        let chosen = layout(&missing, max, budget.saturating_sub(self.bytes));
        let (plan, _) = if let Some(layout) = chosen.filter(|_| self.bytes <= budget) {
            layout
        } else {
            // Shared pages can have holes after removals. Under pressure,
            // compact the current scene before allocating replacements;
            // never keep old and new page sets above the live byte budget.
            self.clear(gl);
            missing = unique.values().copied().collect();
            missing.sort_by_key(|image| image.identity);
            stats.repacks = 1;
            layout(&missing, max, budget)
                .ok_or("scene texture and instance budget exceeded (64 MiB)")?
        };
        stats.reused_images = (unique.len() - missing.len()) as u64;
        let page = if let Some(plan) = &plan {
            Some(self.allocate(gl, plan.width, plan.height, None)?)
        } else {
            None
        };
        for image in missing {
            let mut region = [0.0, 0.0, 1.0, 1.0];
            let texture = if let Some((plan, texture, &(x, y))) = plan.as_ref().and_then(|p| {
                p.slots
                    .get(&image.identity)
                    .map(|xy| (p, page.unwrap(), xy))
            }) {
                let padded = atlas::padded(image);
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    x as i32 - 1,
                    y as i32 - 1,
                    image.width() as i32 + 2,
                    image.height() as i32 + 2,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(&padded)),
                );
                region = [
                    x as f32 / plan.width as f32,
                    y as f32 / plan.height as f32,
                    image.width() as f32 / plan.width as f32,
                    image.height() as f32 / plan.height as f32,
                ];
                stats.texture_upload_bytes += padded.len() as u64;
                texture
            } else {
                let mut premultiplied = Vec::new();
                let pixels = if image.premultiplied {
                    image.pixels()
                } else {
                    premultiplied.extend_from_slice(image.pixels());
                    for pixel in premultiplied.chunks_exact_mut(4) {
                        for channel in 0..3 {
                            pixel[channel] = (pixel[channel] as u32 * pixel[3] as u32 / 255) as u8;
                        }
                    }
                    &premultiplied
                };
                stats.texture_upload_bytes += pixels.len() as u64;
                self.allocate(gl, image.width(), image.height(), Some(pixels))?
            };
            self.resident
                .insert(image.identity, Slot { texture, region });
            stats.texture_uploads += 1;
        }
        debug_assert!(self.bytes <= budget);
        Ok(stats)
    }

    unsafe fn allocate(
        &mut self,
        gl: &glow::Context,
        width: u32,
        height: u32,
        pixels: Option<&[u8]>,
    ) -> Result<glow::Texture, String> {
        let texture = gl.create_texture()?;
        let bytes = width as usize * height as usize * 4;
        self.allocations.insert(texture, bytes);
        self.bytes += bytes;
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            width as i32,
            height as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(pixels),
        );
        Ok(texture)
    }
}

// Prefer a page, but fall back to individual textures if padded page
// storage would exceed the available budget. No full CPU page is created.
fn layout(images: &[&Texture], max: u32, available: usize) -> Option<(Option<atlas::Plan>, usize)> {
    let plan = atlas::Plan::images(images.iter().copied(), max);
    if let Some(plan) = plan {
        let bytes = plan.width as usize * plan.height as usize * 4
            + images
                .iter()
                .filter(|image| !plan.slots.contains_key(&image.identity))
                .map(|image| image.pixels().len())
                .sum::<usize>();
        if bytes <= available {
            return Some((Some(plan), bytes));
        }
    }
    let bytes = images.iter().map(|image| image.pixels().len()).sum();
    (bytes <= available).then_some((None, bytes))
}
