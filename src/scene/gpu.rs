//! Experimental single-surface scene compositor (feature `gpu`).
//!
//! Prepare CPU fallbacks before entering the GL draw callback, then render
//! all commands into one current OpenGL 3.3 framebuffer. Rectangles and
//! portable shaders are native GL; text, rounded borders and isolated
//! layers are retained CPU RGBA textures for now, not a glyph atlas.
//! This low-level spike does not change AppWindow's CPU default.
use super::{Command, Image, Layer, Rect, Scene, MAX_IMAGE_BYTES};
use crate::gfx::{Color, Uniforms};
use glow::HasContext;
use std::{collections::{HashMap, HashSet}, rc::Rc};

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
uniform vec4 u_color;
uniform int u_image;
out vec4 output_color;
void main() {
    if (u_image == 1) {
        vec4 c = texture(u_tex, uv);
        output_color = vec4(c.rgb * u_color.rgb * u_color.a, c.a * u_color.a);
    } else { output_color = vec4(u_color.rgb * u_color.a, u_color.a); }
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

#[derive(Clone)]
enum Paint {
    Solid(Color),
    Image(Image, f32),
    Shader(String, Uniforms),
}
#[derive(Clone)]
struct Item { rect: Rect, clip: Rect, paint: Paint }

/// CPU preparation, including bounded native fallbacks. Rebuild on
/// theme/font/layout/language/scale changes. No GL calls here.
#[derive(Clone)]
pub struct PreparedScene { items: Vec<Item> }

impl PreparedScene {
    pub fn new(scene: &Scene, viewport: Rect) -> Result<Self, String> {
        if viewport.w <= 0 || viewport.h <= 0 { return Err("empty scene viewport".into()); }
        let mut clips = vec![viewport];
        let mut items = Vec::new();
        let mut resources = HashSet::new();
        let mut resource_bytes = 0usize;
        for command in scene.commands() {
            match command {
                Command::PushClip(r) => { clips.push(intersection(*clips.last().unwrap(), *r)); continue; }
                Command::PopClip => { if clips.len() > 1 { clips.pop(); } continue; }
                _ => {}
            }
            let Some(rect) = command.bounds() else { continue };
            let clip = *clips.last().unwrap();
            if !rect.intersects(clip) { continue; }
            let paint = match command {
                Command::Fill { radius: 0, color, .. } => Paint::Solid((*color).into()),
                Command::AlphaFill { color, alpha, .. } => Paint::Solid(Color::from(*color).with_alpha(alpha.clamp(0.0, 1.0) as f32)),
                Command::Image { image, .. } if image.width() == rect.w as u32 && image.height() == rect.h as u32 => Paint::Image(image.clone(), 1.0),
                Command::Layer { layer, opacity, .. } => Paint::Image(layer.image().ok_or("scene layer raster failed")?, *opacity as f32 / 255.0),
                Command::Shader { program, uniforms, .. } => Paint::Shader(program.glsl().to_owned(), *uniforms),
                _ => {
                    let mut one = Scene::default();
                    one.commands.push(command.clone());
                    let layer = Layer::new(rect, one).map_err(|e| e.to_string())?;
                    Paint::Image(layer.image().ok_or("scene primitive raster failed")?, 1.0)
                }
            };
            if let Paint::Image(image, _) = &paint {
                if resources.insert(image.pixels().as_ptr() as usize) {
                    resource_bytes += image.pixels().len();
                    if resource_bytes > MAX_IMAGE_BYTES { return Err("prepared scene texture budget exceeded (64 MiB)".into()); }
                }
            }
            items.push(Item { rect, clip, paint });
        }
        Ok(Self { items })
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

struct Draw { rect: Rect, clip: Rect, texture: Option<glow::Texture>, program: glow::Program, color: Color, uniforms: Option<Uniforms> }

/// GL resources for one retained scene. Textures/programs upload only in
/// `new`; draws update uniforms and submit quads, without CPU rasterizing.
/// No animation timer is installed. Caller owns pacing and framebuffer.
pub struct Renderer {
    gl: Rc<glow::Context>,
    basic: glow::Program,
    vao: glow::VertexArray,
    textures: Vec<glow::Texture>,
    programs: Vec<glow::Program>,
    draws: Vec<Draw>,
    texture_bytes: usize,
    context_alive: bool,
}

impl Renderer {
    /// # Safety
    /// `gl` must be the current OpenGL 3.3 context for construction, every
    /// draw and destruction of this renderer. Never move it across threads.
    /// Preparation must run before this call; it uses FLTK CPU surfaces.
    pub unsafe fn new(gl: Rc<glow::Context>, scene: PreparedScene) -> Result<Self, String> {
        let basic = link(&gl, FRAGMENT)?;
        let vao = match gl.create_vertex_array() { Ok(v) => v, Err(e) => { gl.delete_program(basic); return Err(e); } };
        let mut renderer = Self { gl, basic, vao, textures: Vec::new(), programs: Vec::new(), draws: Vec::new(), texture_bytes: 0, context_alive: true };
        let mut textures = HashMap::new();
        let mut programs = HashMap::new();
        for item in scene.items {
            let (texture, program, color, uniforms) = match item.paint {
                Paint::Solid(c) => (None, basic, c, None),
                Paint::Image(image, opacity) => {
                    let key = image.pixels().as_ptr() as usize;
                    let texture = if let Some(t) = textures.get(&key) { *t } else {
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
                        let mut premultiplied = image.pixels().to_vec();
                        for pixel in premultiplied.chunks_exact_mut(4) {
                            for channel in 0..3 { pixel[channel] = ((pixel[channel] as u32 * pixel[3] as u32 + 127) / 255) as u8; }
                        }
                        renderer.gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, image.width() as i32, image.height() as i32, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&premultiplied)));
                        renderer.texture_bytes += image.pixels().len();
                        textures.insert(key, t);
                        t
                    };
                    (Some(texture), basic, Color::rgba(1.0, 1.0, 1.0, opacity), None)
                }
                Paint::Shader(source, uniforms) => {
                    let program = if let Some(p) = programs.get(&source) { *p } else {
                        let p = link(&renderer.gl, &format!("{SHADER_PRELUDE}{source}"))?;
                        renderer.programs.push(p);
                        programs.insert(source, p);
                        p
                    };
                    (None, program, Color::TRANSPARENT, Some(uniforms))
                }
            };
            renderer.draws.push(Draw { rect: item.rect, clip: item.clip, texture, program, color, uniforms });
        }
        Ok(renderer)
    }

    // The managed surface calls this only after FLTK reports context loss
    // or native widget destruction. The driver owns resource reclamation;
    // deleting old names in a different/new context would be unsafe.
    pub(crate) fn abandon_context(mut self) { self.context_alive = false; }

    pub fn texture_bytes(&self) -> usize { self.texture_bytes }

    /// Moves one prepared primitive without rasterization, compilation or
    /// texture upload. Indexes follow PreparedScene's visible paint order;
    /// its viewport/ancestor clip stays fixed. This does not reflow text.
    pub fn move_to(&mut self, index: usize, x: i32, y: i32) -> bool {
        let Some(draw) = self.draws.get_mut(index) else { return false };
        draw.rect.x = x;
        draw.rect.y = y;
        true
    }

    /// Changes group opacity of a prepared texture or solid primitive.
    /// Top-level shader opacity is not implemented; returns false for it,
    /// invalid indexes and non-finite alpha. No GL calls or uploads.
    pub fn set_opacity(&mut self, index: usize, opacity: f32) -> bool {
        let Some(draw) = self.draws.get_mut(index) else { return false };
        if draw.uniforms.is_some() || !opacity.is_finite() { return false; }
        draw.color.a = opacity.clamp(0.0, 1.0);
        true
    }

    /// Render to the current framebuffer; caller clears/retains it.
    /// `logical` is its logical size, `pixels` the actual framebuffer size.
    /// `time` overrides shader time without rebuilding/rasterizing text.
    /// # Safety
    /// This renderer's context must be current, with its framebuffer bound.
    /// The function owns GL blend/scissor/program/VAO/texture-unit-0 state.
    pub unsafe fn draw(&self, logical: (i32, i32), pixels: (i32, i32), time: Option<f32>) {
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
        for d in &self.draws {
            let r = intersection(d.clip, Rect::new(0, 0, logical.0, logical.1));
            if r.w <= 0 || r.h <= 0 { continue; }
            let sx = pixels.0 as f64 / logical.0 as f64;
            let sy = pixels.1 as f64 / logical.1 as f64;
            let (x, y, right, bottom) = ((r.x as f64 * sx).round() as i32, (r.y as f64 * sy).round() as i32, ((r.x as f64 + r.w as f64) * sx).round() as i32, ((r.y as f64 + r.h as f64) * sy).round() as i32);
            gl.scissor(x, pixels.1 - bottom, right - x, bottom - y);
            gl.use_program(Some(d.program));
            let loc = |s: &str| gl.get_uniform_location(d.program, s);
            gl.uniform_2_f32(loc("u_view").as_ref(), logical.0 as f32, logical.1 as f32);
            gl.uniform_4_f32(loc("u_rect").as_ref(), d.rect.x as f32, d.rect.y as f32, d.rect.w as f32, d.rect.h as f32);
            if let Some(u) = d.uniforms {
                gl.uniform_1_f32(loc("u_time").as_ref(), time.unwrap_or(u.time));
                gl.uniform_2_f32(loc("u_size").as_ref(), d.rect.w as f32, d.rect.h as f32);
                let p = u.pointer.unwrap_or((-1.0, -1.0));
                gl.uniform_2_f32(loc("u_pointer").as_ref(), p.0, p.1);
                for (name, c) in [("u_background", u.background), ("u_foreground", u.foreground), ("u_accent", u.accent), ("u_urgent", u.urgent)] {
                    gl.uniform_4_f32(loc(name).as_ref(), c.r, c.g, c.b, c.a);
                }
                gl.uniform_4_f32(loc("u_params").as_ref(), u.params[0], u.params[1], u.params[2], u.params[3]);
            } else {
                gl.uniform_1_i32(loc("u_image").as_ref(), i32::from(d.texture.is_some()));
                gl.uniform_1_i32(loc("u_tex").as_ref(), 0);
                gl.uniform_4_f32(loc("u_color").as_ref(), d.color.r, d.color.g, d.color.b, d.color.a);
                gl.bind_texture(glow::TEXTURE_2D, d.texture);
            }
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
        }
        gl.disable(glow::SCISSOR_TEST);
        gl.bind_vertex_array(None);
        gl.use_program(None);
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        if !self.context_alive { return; }
        // SAFETY: new's contract requires our context current at destruction.
        unsafe {
            for t in &self.textures { self.gl.delete_texture(*t); }
            for p in &self.programs { self.gl.delete_program(*p); }
            self.gl.delete_program(self.basic);
            self.gl.delete_vertex_array(self.vao);
        }
    }
}

unsafe fn link(gl: &glow::Context, fragment: &str) -> Result<glow::Program, String> {
    let program = gl.create_program()?;
    let mut shaders = Vec::new();
    for (kind, source) in [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, fragment)] {
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
