//! Experimental single-surface scene compositor (feature `gpu`).
//!
//! Prepare CPU fallbacks before entering the GL draw callback, then render
//! all commands into one current OpenGL 3.3 framebuffer. Rectangles and
//! portable shaders are native GL; text, rounded borders and isolated
//! layers are retained CPU RGBA textures for now, not a glyph atlas.
//! This low-level spike does not change AppWindow's CPU default.
use super::{Command, Image, Layer, Rect, Scene, NodeId, NodeError, MAX_IMAGE_BYTES};
use crate::gfx::{Color, Uniforms};
use glow::HasContext;
use std::{cell::{Cell, RefCell}, collections::{HashMap, HashSet}, rc::Rc, sync::Arc};

// Rectangles and colors are retained instance attributes, avoiding
// dynamically indexed uniform arrays in driver shader compilation.
mod atlas;
const INSTANCE_FLOATS: usize = 12;
const INSTANCE_STRIDE: usize = INSTANCE_FLOATS * std::mem::size_of::<f32>();
const BASIC_VERTEX: &str = r#"#version 330 core
uniform vec2 u_view;
layout(location = 0) in vec4 instance_rect;
layout(location = 1) in vec4 instance_color;
layout(location = 2) in vec4 instance_region;
uniform vec4 u_rect, u_color, u_region;
uniform int u_instanced;
out vec2 uv;
out vec4 tint;
void main() {
    vec2 p[6] = vec2[6](vec2(0,0),vec2(1,0),vec2(0,1),vec2(0,1),vec2(1,0),vec2(1,1));
    uv = p[gl_VertexID];
    vec4 rect = u_instanced == 1 ? instance_rect : u_rect;
    tint = u_instanced == 1 ? instance_color : u_color;
    vec2 v = (rect.xy + uv * rect.zw) / u_view;
    vec4 region = u_instanced == 1 ? instance_region : u_region;
    uv = region.xy + uv * region.zw;
    gl_Position = vec4(v.x * 2.0 - 1.0, 1.0 - v.y * 2.0, 0, 1);
}"#;

const VERTEX: &str = r#"#version 330 core
uniform vec2 u_view;
uniform vec4 u_rect;
out vec2 uv;
void main() {
    vec2 p[6] = vec2[6](vec2(0,0),vec2(1,0),vec2(0,1),vec2(0,1),vec2(1,0),vec2(1,1));
    uv = p[gl_VertexID];
    vec2 v = (u_rect.xy + uv * u_rect.zw) / u_view;
    gl_Position = vec4(v.x * 2.0 - 1.0, 1.0 - v.y * 2.0, 0, 1);
}"#;
const FRAGMENT: &str = r#"#version 330 core
in vec2 uv;
uniform sampler2D u_tex;
in vec4 tint;
uniform int u_image;
out vec4 output_color;
void main() {
    if (u_image == 1) {
        vec4 c = texture(u_tex, uv);
        output_color = vec4(c.rgb * tint.rgb * tint.a, c.a * tint.a);
    } else { output_color = vec4(tint.rgb * tint.a, tint.a); }
}"#;
const SHADER_PRELUDE: &str = r#"#version 330 core
in vec2 uv;
uniform float u_time;
uniform vec2 u_size;
uniform vec2 u_pointer;
uniform vec4 u_background, u_foreground, u_accent, u_urgent, u_params;
out vec4 output_color;
vec4 shade(vec2 p);
void main() { vec4 c = shade(uv * u_size); output_color = vec4(c.rgb * c.a, c.a); }
"#;

// Public Image resources stay straight RGBA. Native fallback textures
// retain premultiplied bytes separately to avoid a lossy round trip.
#[derive(Clone)]
struct Texture { width: u32, height: u32, pixels: Arc<[u8]>, premultiplied: bool }
impl Texture {
    fn straight(image: Image) -> Self {
        Self { width: image.width(), height: image.height(), pixels: image.shared_pixels(), premultiplied: false }
    }
    fn pixels(&self) -> &[u8] { &self.pixels }
    fn width(&self) -> u32 { self.width }
    fn height(&self) -> u32 { self.height }
}

#[derive(Clone)]
enum Paint {
    Solid(Color),
    Image(Texture, f32),
    Shader(String, Uniforms),
}
#[derive(Clone)]
struct Item { rect: Rect, clip: Rect, paint: Paint, id: Option<NodeId>, order: super::node::Order }

/// CPU preparation, including bounded native fallbacks. Rebuild on
/// theme/font/layout/language/scale changes. No GL calls here.
#[derive(Clone)]
pub struct PreparedScene { items: Vec<Item>, nodes: HashMap<NodeId, usize> }

impl PreparedScene {
    pub fn new(scene: &Scene, viewport: Rect) -> Result<Self, String> {
        Self::with_node_ceiling(scene, viewport, scene.commands().len())
    }
    // Managed focus chrome remains above root nodes, even when raised.
    pub(crate) fn with_node_ceiling(scene: &Scene, viewport: Rect, ceiling: usize) -> Result<Self, String> {
        if viewport.w <= 0 || viewport.h <= 0 { return Err("empty scene viewport".into()); }
        scene.validate_nodes().map_err(|e| e.to_string())?;
        let ends = super::node::scope_ends(scene.commands());
        let mut scopes = Vec::with_capacity(4);
        scopes.push(0..ceiling);
        let mut nodes = HashMap::new();
        let mut clips = vec![viewport];
        let mut items = Vec::new();
        let mut resources = HashSet::new();
        let mut resource_bytes = 0usize;
        for (source, command) in scene.commands().iter().enumerate() {
            match command {
                Command::PushClip(r) => { clips.push(intersection(*clips.last().unwrap(), *r)); scopes.push(source + 1..ends[source]); continue; }
                Command::PopClip => { if clips.len() > 1 { clips.pop(); scopes.pop(); } continue; }
                _ => {}
            }
            let Some(rect) = command.bounds() else { continue };
            let clip = *clips.last().unwrap();
            let id = match command { Command::Layer { id, .. } => *id, _ => None };
            // Keyed layers must survive initial culling/zero opacity: an
            // update may bring them into view without rebuilding resources.
            if id.is_none() && !rect.intersects(clip) { continue; }
            let paint = match command {
                Command::Fill { radius: 0, color, .. } => Paint::Solid((*color).into()),
                Command::AlphaFill { color, alpha, .. } => Paint::Solid(Color::from(*color).with_alpha(alpha.clamp(0.0, 1.0) as f32)),
                Command::Image { image, .. } => Paint::Image(Texture::straight(image.resampled(rect.w as u32, rect.h as u32).map_err(|e|e.to_string())?), 1.0),
                Command::Layer { layer, opacity, .. } => Paint::Image(Texture::straight(layer.image().ok_or("scene layer raster failed")?), *opacity as f32 / 255.0),
                Command::Shader { program, uniforms, .. } => Paint::Shader(program.glsl().to_owned(), *uniforms),
                _ => {
                    let mut one = Scene::default();
                    one.commands.push(command.clone());
                    let layer = Layer::new(rect, one).map_err(|e| e.to_string())?;
                    Paint::Image(Texture { width: rect.w as u32, height: rect.h as u32,
                        pixels: layer.premultiplied_pixels().ok_or("scene primitive raster failed")?.into(), premultiplied: true }, 1.0)
                }
            };
            if let Paint::Image(image, _) = &paint {
                if resources.insert(image.pixels().as_ptr() as usize) {
                    resource_bytes += image.pixels().len();
                    if resource_bytes > MAX_IMAGE_BYTES { return Err("prepared scene texture budget exceeded (64 MiB)".into()); }
                }
            }
            if let Some(id) = id { nodes.insert(id, items.len()); }
            items.push(Item { rect, clip, paint, id, order: super::node::Order { source, scope: scopes.last().unwrap().clone() } });
        }
        Ok(Self { items, nodes })
    }
    /// Same keyed position update as Scene, before any GL upload.
    pub fn move_node(&mut self, id: NodeId, x: i32, y: i32) -> Result<(), NodeError> {
        let i = *self.nodes.get(&id).ok_or(NodeError::NotFound(id))?;
        self.items[i].rect.x = x;
        self.items[i].rect.y = y;
        Ok(())
    }
    pub fn set_node_opacity(&mut self, id: NodeId, value: f32) -> Result<(), NodeError> {
        let opacity = super::node::opacity(value)? as f32 / 255.0;
        let i = *self.nodes.get(&id).ok_or(NodeError::NotFound(id))?;
        let Paint::Image(_, alpha) = &mut self.items[i].paint else { unreachable!() };
        *alpha = opacity;
        Ok(())
    }
    /// Raises within the recorded parent clip scope. Retained resources
    /// and the original source/clip identity stay unchanged.
    pub fn raise_node(&mut self, id: NodeId) -> Result<(), NodeError> {
        super::node::raise(&mut self.items, &mut self.nodes, id, |item| &item.order)
    }
    pub fn primitive_count(&self) -> usize { self.items.len() }
}

fn intersection(a: Rect, b: Rect) -> Rect {
    let x = (a.x as i64).max(b.x as i64);
    let y = (a.y as i64).max(b.y as i64);
    let right = (a.x as i64 + a.w.max(0) as i64).min(b.x as i64 + b.w.max(0) as i64);
    let bottom = (a.y as i64 + a.h.max(0) as i64).min(b.y as i64 + b.h.max(0) as i64);
    Rect::new(x as i32, y as i32, (right - x).max(0) as i32, (bottom - y).max(0) as i32)
}

// Locations belong to a linked program, not a frame or primitive. Share
// them across draws and discard them with the program/context.
struct Locations {
    view: Option<glow::UniformLocation>,
    rect: Option<glow::UniformLocation>,
    image: Option<glow::UniformLocation>,
    color: Option<glow::UniformLocation>,
    region: Option<glow::UniformLocation>,
    instanced: Option<glow::UniformLocation>,
    tex: Option<glow::UniformLocation>,
    time: Option<glow::UniformLocation>,
    size: Option<glow::UniformLocation>,
    pointer: Option<glow::UniformLocation>,
    theme: [Option<glow::UniformLocation>; 5],
}
impl Locations {
    unsafe fn new(gl: &glow::Context, program: glow::Program) -> Rc<Self> {
        let loc = |name| gl.get_uniform_location(program, name);
        Rc::new(Self {
            view: loc("u_view"), rect: loc("u_rect"), image: loc("u_image"),
            tex: loc("u_tex"), region: loc("u_region"), color: loc("u_color"), instanced: loc("u_instanced"), time: loc("u_time"),
            size: loc("u_size"), pointer: loc("u_pointer"),
            theme: ["u_background", "u_foreground", "u_accent", "u_urgent", "u_params"].map(loc),
        })
    }
}
struct Draw { region: [f32; 4], rect: Rect, clip: Rect, texture: Option<glow::Texture>, program: glow::Program, locations: Rc<Locations>, color: Color, uniforms: Option<Uniforms>, order: super::node::Order }
struct Overlay { rect: Rect, texture: glow::Texture, bytes: usize }

/// Actual GL commands from the most recent draw, including the overlay.
/// These are submissions; they do not measure physical presentation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub primitives: u64,
    pub draw_calls: u64,
    pub instance_upload_bytes: u64,
}

/// GL resources for one retained scene. Textures/programs upload only in
/// `new`; draws update uniforms and submit quads, without CPU rasterizing.
/// No animation timer is installed. Caller owns pacing and framebuffer.
pub struct Renderer {
    gl: Rc<glow::Context>,
    basic: glow::Program,
    basic_locations: Rc<Locations>,
    vao: glow::VertexArray,
    textures: Vec<glow::Texture>,
    programs: Vec<glow::Program>,
    draws: Vec<Draw>,
    texture_bytes: usize,
    context_alive: bool,
    nodes: HashMap<NodeId, usize>,
    overlay: Option<Overlay>,
    frame: Cell<FrameStats>,
    instance_buffer: glow::Buffer,
    instances: RefCell<Vec<f32>>,
    instances_dirty: Cell<bool>,
    instances_uploaded: Cell<bool>,
    instanced_mode: Cell<bool>,
}

impl Renderer {
    /// # Safety
    /// `gl` must be the current OpenGL 3.3 context for construction, every
    /// draw and destruction of this renderer. Never move it across threads.
    /// Preparation must run before this call; it uses FLTK CPU surfaces.
    pub unsafe fn new(gl: Rc<glow::Context>, mut scene: PreparedScene) -> Result<Self, String> {
        let basic = link(&gl, BASIC_VERTEX, FRAGMENT)?;
        let vao = match gl.create_vertex_array() { Ok(v) => v, Err(e) => { gl.delete_program(basic); return Err(e); } };
        let instance_buffer = match gl.create_buffer() { Ok(b) => b, Err(e) => { gl.delete_vertex_array(vao); gl.delete_program(basic); return Err(e); } };
        let basic_locations = Locations::new(&gl, basic);
        let mut renderer = Self { gl, basic, basic_locations, vao, textures: Vec::new(), programs: Vec::new(), draws: Vec::new(), texture_bytes: 0, context_alive: true, nodes: std::mem::take(&mut scene.nodes), overlay: None, frame: Cell::default(), instance_buffer, instances: RefCell::default(), instances_dirty: Cell::new(true), instances_uploaded: Cell::new(false), instanced_mode: Cell::new(false) };
        let reserved = scene.items.len().checked_add(1).and_then(|n|n.checked_mul(INSTANCE_STRIDE)).ok_or("scene instance bytes overflow")?;
        if reserved > MAX_IMAGE_BYTES { return Err("scene instance budget exceeded (64 MiB)".into()); }
        let max = renderer.gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) as u32;
        let mut plan = atlas::Plan::new(&scene, max);
        if let Some(p) = &plan {
            let mut unique = HashSet::new();
            let unpaged: usize = scene.items.iter().filter_map(|item| match &item.paint {
                Paint::Image(image, _) if !p.slots.contains_key(&(image.pixels().as_ptr() as usize)) && unique.insert(image.pixels().as_ptr() as usize) => Some(image.pixels().len()),
                _ => None,
            }).sum();
            if unpaged.checked_add(p.width as usize * p.height as usize * 4).and_then(|n|n.checked_add(reserved)).map_or(true, |n|n>MAX_IMAGE_BYTES) { plan = None; }
        }
        let page = if let Some(plan) = plan {
            let texture = renderer.gl.create_texture()?;
            renderer.textures.push(texture);
            renderer.gl.active_texture(glow::TEXTURE0);
            renderer.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            renderer.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            renderer.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            renderer.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            renderer.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            renderer.gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, plan.width as i32, plan.height as i32, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
            renderer.texture_bytes = plan.width as usize * plan.height as usize * 4;
            Some((plan, texture))
        } else { None };
        let mut textures = HashMap::new();
        let mut programs: HashMap<String, (glow::Program, Rc<Locations>)> = HashMap::new();
        for item in scene.items {
            let mut region = [0.0, 0.0, 1.0, 1.0];
            let (texture, program, locations, color, uniforms) = match item.paint {
                Paint::Solid(c) => (None, basic, renderer.basic_locations.clone(), c, None),
                Paint::Image(image, opacity) => {
                    let key = image.pixels().as_ptr() as usize;
                    let slot = page.as_ref().and_then(|(p,t)| p.slots.get(&key).map(|&(x,y)| (p,*t,x,y)));
                    if let Some((p,_,x,y)) = slot { region = [x as f32 / p.width as f32, y as f32 / p.height as f32, image.width() as f32 / p.width as f32, image.height() as f32 / p.height as f32]; }
                    let texture = if let Some(t) = textures.get(&key) { *t } else if let Some((_,t,x,y)) = slot {
                        let padded = atlas::padded(&image);
                        renderer.gl.active_texture(glow::TEXTURE0);
                        renderer.gl.bind_texture(glow::TEXTURE_2D, Some(t));
                        renderer.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                        renderer.gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
                        renderer.gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, x as i32-1, y as i32-1, image.width() as i32+2, image.height() as i32+2, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&padded)));
                        textures.insert(key,t);
                        t
                    } else {
                        if renderer.texture_bytes + image.pixels().len() > MAX_IMAGE_BYTES { return Err("scene texture budget exceeded (64 MiB)".into()); }
                        let max = renderer.gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE) as u32;
                        if image.width() > max || image.height() > max { return Err("scene image exceeds GL texture size".into()); }
                        let t = renderer.gl.create_texture()?;
                        renderer.textures.push(t);
                        renderer.gl.active_texture(glow::TEXTURE0);
                        renderer.gl.bind_texture(glow::TEXTURE_2D, Some(t));
                        renderer.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                        renderer.gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
                        renderer.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
                        renderer.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                        renderer.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
                        renderer.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
                        // Linear filtering must interpolate premultiplied
                        // colors; straight RGBA leaks colors from transparent
                        // texels into the edge of scaled sprites.
                        // Match FLTK image-cache truncation. Rounding up
                        // here biases each translucent edge brighter and
                        // accumulates across overlapping native fallbacks.
                        let mut upload = Vec::new();
                        let pixels = if image.premultiplied { image.pixels() } else {
                            upload.extend_from_slice(image.pixels());
                            for pixel in upload.chunks_exact_mut(4) {
                                for channel in 0..3 { pixel[channel] = ((pixel[channel] as u32 * pixel[3] as u32) / 255) as u8; }
                            }
                            &upload
                        };
                        renderer.gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, image.width() as i32, image.height() as i32, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(pixels)));
                        renderer.texture_bytes += image.pixels().len();
                        textures.insert(key, t);
                        t
                    };
                    (Some(texture), basic, renderer.basic_locations.clone(), Color::rgba(1.0, 1.0, 1.0, opacity), None)
                }
                Paint::Shader(source, uniforms) => {
                    let (program, locations) = if let Some(p) = programs.get(&source) { p.clone() } else {
                        let p = link(&renderer.gl, VERTEX, &format!("{SHADER_PRELUDE}{source}"))?;
                        renderer.programs.push(p);
                        let locations = Locations::new(&renderer.gl, p);
                        programs.insert(source, (p, locations.clone()));
                        (p, locations)
                    };
                    (None, program, locations, Color::TRANSPARENT, Some(uniforms))
                }
            };
            debug_assert!(item.id.map_or(true, |id| renderer.nodes.get(&id) == Some(&renderer.draws.len())));
            renderer.draws.push(Draw { region, rect: item.rect, clip: item.clip, texture, program, locations, color, uniforms, order: item.order });
        }
        let count = renderer.draws.len().checked_add(1).ok_or("scene instance count overflow")?;
        let bytes = count.checked_mul(INSTANCE_STRIDE).ok_or("scene instance bytes overflow")?;
        if renderer.texture_bytes.checked_add(bytes).map_or(true, |n| n > MAX_IMAGE_BYTES) {
            return Err("scene texture and instance budget exceeded (64 MiB)".into());
        }
        renderer.instances = RefCell::new(vec![0.0; count * INSTANCE_FLOATS]);
        renderer.gl.bind_vertex_array(Some(renderer.vao));
        renderer.gl.bind_buffer(glow::ARRAY_BUFFER, Some(renderer.instance_buffer));
        for attribute in 0..3 {
            renderer.gl.vertex_attrib_divisor(attribute, 1);
            renderer.gl.vertex_attrib_pointer_f32(attribute, 4, glow::FLOAT, false, INSTANCE_STRIDE as i32, attribute as i32 * 16);
        }
        renderer.gl.bind_vertex_array(None);
        renderer.gl.bind_buffer(glow::ARRAY_BUFFER, None);
        Ok(renderer)
    }

    // The managed surface calls this only after FLTK reports context loss
    // or native widget destruction. The driver owns resource reclamation;
    // deleting old names in a different/new context would be unsafe.
    pub(crate) fn abandon_context(mut self) { self.context_alive = false; }

    pub fn texture_bytes(&self) -> usize { self.texture_bytes + self.overlay_bytes() }
    pub fn frame_stats(&self) -> FrameStats { self.frame.get() }
    pub fn instance_bytes(&self) -> usize { if self.instances_uploaded.get() { self.instance_capacity_bytes() } else { 0 } }
    fn instance_capacity_bytes(&self) -> usize { (self.draws.len() + 1) * INSTANCE_STRIDE }
    pub(crate) fn overlay_bytes(&self) -> usize { self.overlay.as_ref().map_or(0, |o|o.bytes) }

    /// Uses the existing basic program/VAO, independently of scene resources.
    /// Same-size captures update one texture; removing the last layer frees it.
    pub(super) unsafe fn update_overlay(&mut self, capture: Option<&super::bridge::Capture>) -> Result<(), String> {
        self.instances_dirty.set(true);
        let Some(capture) = capture else {
            if let Some(overlay) = self.overlay.take() { self.gl.delete_texture(overlay.texture); }
            return Ok(());
        };
        if self.texture_bytes.checked_add(self.instance_capacity_bytes()).and_then(|n| n.checked_add(capture.pixels.len())).map_or(true, |n|n>MAX_IMAGE_BYTES) {
            return Err("scene instance and overlay texture budget exceeded (64 MiB)".into());
        }
        let max = self.gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE);
        if capture.rect.w>max || capture.rect.h>max { return Err("overlay exceeds GL texture size".into()); }
        let old_size = self.overlay.as_ref().map(|o|(o.rect.w,o.rect.h));
        let texture = match &self.overlay { Some(o)=>o.texture, None=>self.gl.create_texture()? };
        self.gl.active_texture(glow::TEXTURE0);
        self.gl.bind_texture(glow::TEXTURE_2D,Some(texture));
        self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT,1);
        self.gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH,0);
        self.gl.tex_parameter_i32(glow::TEXTURE_2D,glow::TEXTURE_MIN_FILTER,glow::LINEAR as i32);
        self.gl.tex_parameter_i32(glow::TEXTURE_2D,glow::TEXTURE_MAG_FILTER,glow::LINEAR as i32);
        self.gl.tex_parameter_i32(glow::TEXTURE_2D,glow::TEXTURE_WRAP_S,glow::CLAMP_TO_EDGE as i32);
        self.gl.tex_parameter_i32(glow::TEXTURE_2D,glow::TEXTURE_WRAP_T,glow::CLAMP_TO_EDGE as i32);
        if old_size == Some((capture.rect.w,capture.rect.h)) {
            self.gl.tex_sub_image_2d(glow::TEXTURE_2D,0,0,0,capture.rect.w,capture.rect.h,glow::RGBA,glow::UNSIGNED_BYTE,glow::PixelUnpackData::Slice(Some(&capture.pixels)));
        } else {
            self.gl.tex_image_2d(glow::TEXTURE_2D,0,glow::RGBA8 as i32,capture.rect.w,capture.rect.h,0,glow::RGBA,glow::UNSIGNED_BYTE,glow::PixelUnpackData::Slice(Some(&capture.pixels)));
        }
        self.overlay = Some(Overlay {rect:capture.rect,texture,bytes:capture.pixels.len()});
        Ok(())
    }

    /// Moves one prepared primitive without rasterization, compilation or
    /// texture upload. Indexes follow current prepared paint order and can
    /// change after raise_node; prefer keys for application identities.
    /// The viewport/ancestor clip stays fixed. This does not reflow text.
    pub fn move_to(&mut self, index: usize, x: i32, y: i32) -> bool {
        let Some(draw) = self.draws.get_mut(index) else { return false };
        if draw.rect.x != x || draw.rect.y != y {
            draw.rect.x = x;
            draw.rect.y = y;
            self.instances_dirty.set(true);
        }
        true
    }

    /// Changes group opacity of a prepared texture or solid primitive.
    /// Top-level shader opacity is not implemented; returns false for it,
    /// invalid indexes and non-finite alpha. No GL calls or uploads.
    pub fn set_opacity(&mut self, index: usize, opacity: f32) -> bool {
        let Some(draw) = self.draws.get_mut(index) else { return false };
        if draw.uniforms.is_some() || !opacity.is_finite() { return false; }
        let alpha = opacity.clamp(0.0, 1.0);
        if draw.color.a != alpha {
            draw.color.a = alpha;
            self.instances_dirty.set(true);
        }
        true
    }

    /// O(1) stable-ID update. Unlike visible primitive indexes, keys also
    /// address layers initially outside the viewport or fully transparent.
    /// No context, compilation, rasterization or texture upload is needed.
    pub fn move_node(&mut self, id: NodeId, x: i32, y: i32) -> Result<(), NodeError> {
        let i = *self.nodes.get(&id).ok_or(NodeError::NotFound(id))?;
        self.move_to(i, x, y);
        Ok(())
    }
    pub fn set_node_opacity(&mut self, id: NodeId, value: f32) -> Result<(), NodeError> {
        let opacity = super::node::opacity(value)? as f32 / 255.0;
        let i = *self.nodes.get(&id).ok_or(NodeError::NotFound(id))?;
        self.set_opacity(i, opacity);
        Ok(())
    }

    /// Raises within the node's original parent clip scope. O(draws +
    /// nodes), no GL calls/uploads; stable IDs are reindexed after motion.
    pub fn raise_node(&mut self, id: NodeId) -> Result<(), NodeError> {
        super::node::raise(&mut self.draws, &mut self.nodes, id, |draw| &draw.order)?;
        self.instances_dirty.set(true);
        Ok(())
    }

    /// Render to the current framebuffer; caller clears/retains it.
    /// `logical` is its logical size, `pixels` the actual framebuffer size.
    /// `time` overrides shader time without rebuilding/rasterizing text.
    /// # Safety
    /// This renderer's context must be current, with its framebuffer bound.
    /// The function owns GL blend/scissor/program/VAO/array-buffer/texture-unit-0 state.
    pub unsafe fn draw(&self, logical: (i32, i32), pixels: (i32, i32), time: Option<f32>) {
        self.frame.set(FrameStats::default());
        if logical.0 <= 0 || logical.1 <= 0 || pixels.0 <= 0 || pixels.1 <= 0 { return; }
        let gl = &self.gl;
        gl.viewport(0, 0, pixels.0, pixels.1);
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::CULL_FACE);
        gl.disable(glow::FRAMEBUFFER_SRGB);
        gl.color_mask(true, true, true, true);
        gl.enable(glow::BLEND);
        gl.blend_equation(glow::FUNC_ADD);
        gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
        gl.enable(glow::SCISSOR_TEST);
        gl.bind_vertex_array(Some(self.vao));
        gl.active_texture(glow::TEXTURE0);
        let mut frame = FrameStats::default();
        let mut instance_data_bound = false;
        let viewport = Rect::new(0, 0, logical.0, logical.1);
        let sx = pixels.0 as f64 / logical.0 as f64;
        let sy = pixels.1 as f64 / logical.1 as f64;
        let mut index = 0;
        while index < self.draws.len() {
            let d = &self.draws[index];
            let r = intersection(d.clip, viewport);
            if r.w <= 0 || r.h <= 0 || !d.rect.intersects(r) || (d.uniforms.is_none() && d.color.a <= 0.0) {
                index += 1;
                continue;
            }
            let (x, y, right, bottom) = ((r.x as f64 * sx).round() as i32, (r.y as f64 * sy).round() as i32, ((r.x as f64 + r.w as f64) * sx).round() as i32, ((r.y as f64 + r.h as f64) * sy).round() as i32);
            gl.scissor(x, pixels.1 - bottom, right - x, bottom - y);
            if let Some(u) = d.uniforms {
                gl.use_program(Some(d.program));
                let loc = &d.locations;
                gl.uniform_2_f32(loc.view.as_ref(), logical.0 as f32, logical.1 as f32);
                gl.uniform_4_f32(loc.rect.as_ref(), d.rect.x as f32, d.rect.y as f32, d.rect.w as f32, d.rect.h as f32);
                gl.uniform_1_f32(loc.time.as_ref(), time.unwrap_or(u.time));
                gl.uniform_2_f32(loc.size.as_ref(), d.rect.w as f32, d.rect.h as f32);
                let p = u.pointer.unwrap_or((-1.0, -1.0));
                gl.uniform_2_f32(loc.pointer.as_ref(), p.0, p.1);
                for (location, c) in loc.theme.iter().zip([u.background, u.foreground, u.accent, u.urgent]) {
                    gl.uniform_4_f32(location.as_ref(), c.r, c.g, c.b, c.a);
                }
                gl.uniform_4_f32(loc.theme[4].as_ref(), u.params[0], u.params[1], u.params[2], u.params[3]);
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
                index += 1;
                frame.primitives += 1;
            } else {
                let start = index;
                // Only consecutive compatible primitives may share a
                // submission. Never sort across shaders, clips or textures.
                while index < self.draws.len() {
                    let next = &self.draws[index];
                    if next.uniforms.is_some() || next.texture != d.texture
                        || intersection(next.clip, viewport) != r
                        || !next.rect.intersects(r) || next.color.a <= 0.0 { break; }
                    index += 1;
                }
                let count = index - start;
                if count > 1 && !instance_data_bound {
                    frame.instance_upload_bytes += self.upload_instances() as u64;
                    instance_data_bound = true;
                }
                self.draw_batch(logical, d.texture, start, count, d.rect, d.color, d.region);
                frame.primitives += count as u64;
            }
            frame.draw_calls += 1;
        }
        if let Some(o) = &self.overlay {
            gl.scissor(0, 0, pixels.0, pixels.1);
            self.draw_batch(logical, Some(o.texture), self.draws.len(), 1, o.rect, Color::rgba(1.0, 1.0, 1.0, 1.0), [0.0, 0.0, 1.0, 1.0]);
            frame.primitives += 1;
            frame.draw_calls += 1;
        }
        self.frame.set(frame);
        gl.disable(glow::SCISSOR_TEST);
        gl.bind_vertex_array(None);
        gl.bind_buffer(glow::ARRAY_BUFFER, None);
        gl.use_program(None);
    }
    unsafe fn upload_instances(&self) -> usize {
        let gl = &self.gl;
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.instance_buffer));
        if !self.instances_dirty.replace(false) { return 0; }
        let mut data = self.instances.borrow_mut();
        for (draw, instance) in self.draws.iter().zip(data.chunks_exact_mut(INSTANCE_FLOATS)) {
            instance.copy_from_slice(&[draw.rect.x as f32, draw.rect.y as f32, draw.rect.w as f32, draw.rect.h as f32,
                draw.color.r, draw.color.g, draw.color.b, draw.color.a, draw.region[0], draw.region[1], draw.region[2], draw.region[3]]);
        }
        if let Some(overlay) = &self.overlay {
            let rect = overlay.rect;
            data[self.draws.len() * INSTANCE_FLOATS..].copy_from_slice(&[rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0]);
        }
        // SAFETY: f32 storage is initialized, contiguous and has no padding;
        // new checked the byte count and draw count before allocation.
        let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), self.instance_capacity_bytes());
        if self.instances_uploaded.replace(true) {
            gl.buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
        } else {
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::DYNAMIC_DRAW);
        }
        bytes.len()
    }
    unsafe fn draw_batch(&self, logical: (i32, i32), texture: Option<glow::Texture>, start: usize, count: usize, rect: Rect, color: Color, region: [f32;4]) {
        let gl = &self.gl;
        let loc = &self.basic_locations;
        gl.use_program(Some(self.basic));
        gl.uniform_2_f32(loc.view.as_ref(), logical.0 as f32, logical.1 as f32);
        let instanced = count > 1;
        if self.instanced_mode.replace(instanced) != instanced {
            gl.uniform_1_i32(loc.instanced.as_ref(), i32::from(instanced));
            for attribute in 0..3 {
                if instanced { gl.enable_vertex_attrib_array(attribute); }
                else { gl.disable_vertex_attrib_array(attribute); }
            }
        }
        if instanced {
            let offset = (start * INSTANCE_STRIDE) as i32;
            gl.vertex_attrib_pointer_f32(0, 4, glow::FLOAT, false, INSTANCE_STRIDE as i32, offset);
            gl.vertex_attrib_pointer_f32(1, 4, glow::FLOAT, false, INSTANCE_STRIDE as i32, offset + 16);
            gl.vertex_attrib_pointer_f32(2, 4, glow::FLOAT, false, INSTANCE_STRIDE as i32, offset + 32);
        } else {
            gl.uniform_4_f32(loc.rect.as_ref(), rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
            gl.uniform_4_f32(loc.color.as_ref(), color.r, color.g, color.b, color.a);
            gl.uniform_4_f32(loc.region.as_ref(), region[0], region[1], region[2], region[3]);
        }
        gl.uniform_1_i32(loc.image.as_ref(), i32::from(texture.is_some()));
        gl.uniform_1_i32(loc.tex.as_ref(), 0);
        gl.bind_texture(glow::TEXTURE_2D, texture);
        if instanced { gl.draw_arrays_instanced(glow::TRIANGLES, 0, 6, count as i32); }
        else { gl.draw_arrays(glow::TRIANGLES, 0, 6); }
    }

}

impl Drop for Renderer {
    fn drop(&mut self) {
        if !self.context_alive { return; }
        // SAFETY: new's contract requires our context current at destruction.
        unsafe {
            for t in &self.textures { self.gl.delete_texture(*t); }
            if let Some(o)=&self.overlay { self.gl.delete_texture(o.texture); }
            for p in &self.programs { self.gl.delete_program(*p); }
            self.gl.delete_program(self.basic);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_buffer(self.instance_buffer);
        }
    }
}

unsafe fn link(gl: &glow::Context, vertex: &str, fragment: &str) -> Result<glow::Program, String> {
    let program = gl.create_program()?;
    let mut shaders = Vec::new();
    for (kind, source) in [(glow::VERTEX_SHADER, vertex), (glow::FRAGMENT_SHADER, fragment)] {
        let shader = match gl.create_shader(kind) { Ok(s) => s, Err(e) => { for s in shaders { gl.delete_shader(s); } gl.delete_program(program); return Err(e); } };
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            let error = gl.get_shader_info_log(shader);
            gl.delete_shader(shader);
            for s in shaders { gl.delete_shader(s); }
            gl.delete_program(program);
            return Err(error);
        }
        gl.attach_shader(program, shader);
        shaders.push(shader);
    }
    gl.link_program(program);
    for shader in shaders { gl.detach_shader(program, shader); gl.delete_shader(shader); }
    if !gl.get_program_link_status(program) {
        let error = gl.get_program_info_log(program);
        gl.delete_program(program);
        return Err(error);
    }
    Ok(program)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipped_resources_are_not_rasterized_and_clip_scopes_restore() {
        let (scene, ()) = Scene::record(|| {
            super::super::clip(Rect::new(0, 0, 0, 0), || {
                crate::paint::fill(0, 0, i32::MAX, i32::MAX, 10, crate::theme::Rgb(0));
            });
            crate::paint::fill(0, 0, 10, 10, 0, crate::theme::Rgb(0));
        });
        let prepared = PreparedScene::new(&scene, Rect::new(0, 0, 100, 100)).unwrap();
        assert_eq!(prepared.primitive_count(), 1);
    }
}
