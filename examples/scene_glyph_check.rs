//! Positioned native glyph experiment; does not change the scene renderer.
//! Run on a private display: env -u WAYLAND_DISPLAY xvfb-run -a cargo run
//! --offline --example scene_glyph_check -- target/glyph-check
//! Add --require-individual or --require-groups after the output path to
//! enforce those experimental gates. Linux FFI, pinned FLTK 1.5.23 Cairo.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("scene_glyph_check requires the Linux Pango/Cairo backend");
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use fltk::{draw, enums::*, prelude::*, surface::ImageSurface};
    use fltk_omarchy::{paint, png, App};
    use serde_json::json;
    use std::{
        cell::{Cell, RefCell},
        ffi::{c_char, c_void, CString},
        path::PathBuf,
        ptr,
    };

    const W: i32 = 720;
    const H: i32 = 128;
    const SCALE: f64 = 1024.0;
    type Ptr = *mut c_void;

    // Public Pango C structures, not private FLTK object layouts.
    #[repr(C)]
    #[derive(Default)]
    struct Rectangle {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Geometry {
        width: i32,
        x_offset: i32,
        y_offset: i32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct GlyphInfo {
        glyph: u32,
        geometry: Geometry,
        // PangoGlyphVisAttr's C unsigned bitfield storage.
        attributes: u32,
    }
    #[repr(C)]
    struct GlyphString {
        count: i32,
        glyphs: *mut GlyphInfo,
        clusters: *mut i32,
        space: i32,
    }
    #[repr(C)]
    struct Analysis {
        shape_engine: Ptr,
        lang_engine: Ptr,
        font: Ptr,
        level: u8,
        gravity: u8,
        flags: u8,
        script: u8,
        language: Ptr,
        extra_attrs: Ptr,
    }
    #[repr(C)]
    struct Item {
        offset: i32,
        length: i32,
        num_chars: i32,
        analysis: Analysis,
    }
    #[repr(C)]
    struct GlyphItem {
        item: *mut Item,
        glyphs: *mut GlyphString,
        y_offset: i32,
        start_x_offset: i32,
        end_x_offset: i32,
    }

    extern "C" {
        // Itanium C++ ABI, pinned fltk-sys 1.5.23. Check the actual driver
        // through RTTI before invoking its font-description method.
        static mut fl_graphics_driver: Ptr;
        #[link_name = "_Z13fl_wl_displayv"]
        fn fl_wl_display() -> Ptr;
        #[link_name = "_ZTI18Fl_Graphics_Driver"]
        static BASE_TYPE: u8;
        #[link_name = "_ZTI24Fl_Cairo_Graphics_Driver"]
        static CAIRO_TYPE: u8;
        fn __dynamic_cast(object: Ptr, base: *const u8, derived: *const u8, hint: isize) -> Ptr;
        #[link_name = "_ZN24Fl_Cairo_Graphics_Driver22pango_font_descriptionEv"]
        fn font_description(driver: Ptr) -> Ptr;
        #[link_name = "_Z11fl_cairo_gcv"]
        fn fl_cairo_gc() -> Ptr;
        #[link_name = "_Z7fl_drawPKciiiijPFvS0_iiiEP8Fl_Imageii"]
        fn layout_lines(
            text: *const c_char,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            align: u32,
            callback: extern "C" fn(*const c_char, i32, i32, i32),
            image: Ptr,
            draw_symbols: i32,
            spacing: i32,
        );
    }
    #[link(name = "pango-1.0")]
    extern "C" {
        fn pango_version() -> i32;
        fn pango_font_map_create_context(map: Ptr) -> Ptr;
        fn pango_context_set_font_description(context: Ptr, description: Ptr);
        fn pango_language_get_default() -> Ptr;
        fn pango_font_map_load_fontset(map: Ptr, context: Ptr, description: Ptr, lang: Ptr) -> Ptr;
        fn pango_fontset_get_metrics(fontset: Ptr) -> Ptr;
        fn pango_font_metrics_get_descent(metrics: Ptr) -> i32;
        fn pango_font_metrics_get_height(metrics: Ptr) -> i32;
        fn pango_font_metrics_unref(metrics: Ptr);
        fn pango_layout_new(context: Ptr) -> Ptr;
        fn pango_layout_set_text(layout: Ptr, text: *const c_char, length: i32);
        fn pango_layout_get_iter(layout: Ptr) -> Ptr;
        fn pango_layout_iter_get_run_readonly(iter: Ptr) -> *mut GlyphItem;
        fn pango_layout_iter_get_run_extents(
            iter: Ptr,
            ink: *mut Rectangle,
            logical: *mut Rectangle,
        );
        fn pango_layout_iter_get_run_baseline(iter: Ptr) -> i32;
        fn pango_layout_iter_next_run(iter: Ptr) -> i32;
        fn pango_layout_iter_free(iter: Ptr);
        fn pango_font_describe(font: Ptr) -> Ptr;
        fn pango_font_description_to_string(description: Ptr) -> *mut c_char;
        fn pango_font_description_free(description: Ptr);
        fn pango_font_get_glyph_extents(
            font: Ptr,
            glyph: u32,
            ink: *mut Rectangle,
            logical: *mut Rectangle,
        );
    }
    #[link(name = "pangocairo-1.0")]
    extern "C" {
        fn pango_cairo_font_map_get_default() -> Ptr;
        fn pango_cairo_show_layout(cr: Ptr, layout: Ptr);
        fn pango_cairo_show_glyph_string(cr: Ptr, font: Ptr, glyphs: *mut GlyphString);
    }
    #[link(name = "gobject-2.0")]
    extern "C" {
        fn g_object_unref(object: Ptr);
    }
    #[link(name = "glib-2.0")]
    extern "C" {
        fn g_free(data: Ptr);
    }
    #[link(name = "cairo")]
    extern "C" {
        fn cairo_save(cr: Ptr);
        fn cairo_restore(cr: Ptr);
        fn cairo_identity_matrix(cr: Ptr);
        fn cairo_translate(cr: Ptr, x: f64, y: f64);
        fn cairo_rectangle(cr: Ptr, x: f64, y: f64, w: f64, h: f64);
        fn cairo_clip(cr: Ptr);
        fn cairo_move_to(cr: Ptr, x: f64, y: f64);
        fn cairo_set_source_rgb(cr: Ptr, r: f64, g: f64, b: f64);
    }

    struct Object(Ptr);
    impl Object {
        unsafe fn new(pointer: Ptr) -> Self {
            assert!(!pointer.is_null(), "native object allocation failed");
            Self(pointer)
        }
    }
    impl Drop for Object {
        fn drop(&mut self) {
            unsafe { g_object_unref(self.0) }
        }
    }
    struct Iter(Ptr);
    impl Drop for Iter {
        fn drop(&mut self) {
            unsafe { pango_layout_iter_free(self.0) }
        }
    }
    struct Canvas {
        surface: ImageSurface,
        width: i32,
    }
    impl Canvas {
        fn new(background: u8, case: &Case) -> Self {
            let surface = ImageSurface::new(W, H, false);
            ImageSurface::push_current(&surface);
            draw::set_draw_color(fltk_omarchy::theme::Rgb(background as u32 * 0x010101).fl());
            draw::draw_rectf(0, 0, W, H);
            ImageSurface::pop_current();
            Self {
                surface,
                width: case.width,
            }
        }
        unsafe fn paint(&self, x: f64, y: f64, paint: impl FnOnce(Ptr)) {
            ImageSurface::push_current(&self.surface);
            let cr = fl_cairo_gc();
            cairo_save(cr);
            cairo_identity_matrix(cr);
            cairo_rectangle(cr, 12.0, 8.0, self.width as f64, 112.0);
            cairo_clip(cr);
            cairo_set_source_rgb(cr, 54.0 / 255.0, 145.0 / 255.0, 227.0 / 255.0);
            cairo_translate(cr, x, y);
            cairo_move_to(cr, 0.0, 0.0);
            paint(cr);
            cairo_restore(cr);
            ImageSurface::pop_current();
        }
        fn rgb(&self) -> Vec<u8> {
            ImageSurface::push_current(&self.surface);
            let rgb = self.surface.image().unwrap().to_rgb_data();
            ImageSurface::pop_current();
            rgb
        }
    }

    struct Line {
        text: Vec<u8>,
        x: i32,
        y: i32,
    }
    thread_local! {
        static LINES: RefCell<Vec<Line>> = const { RefCell::new(Vec::new()) };
        static CALLBACK_FAILED: Cell<bool> = const { Cell::new(false) };
    }
    extern "C" fn collect(text: *const c_char, length: i32, x: i32, y: i32) {
        // No Rust unwind may cross FLTK's C++ callback. FLTK owns this
        // temporary buffer and guarantees length valid bytes during call.
        let result = std::panic::catch_unwind(|| {
            assert!((0..=1024 * 1024).contains(&length));
            let text = unsafe { std::slice::from_raw_parts(text.cast(), length as usize) };
            LINES.with(|lines| {
                lines.borrow_mut().push(Line {
                    text: text.to_vec(),
                    x,
                    y,
                })
            });
        });
        if result.is_err() {
            CALLBACK_FAILED.with(|failed| failed.set(true));
        }
    }
    struct Case {
        name: &'static str,
        text: &'static str,
        font: Font,
        size: i32,
        align: Align,
        width: i32,
    }
    fn native(case: &Case, background: u8) -> Vec<u8> {
        let surface = ImageSurface::new(W, H, false);
        ImageSurface::push_current(&surface);
        draw::set_draw_color(fltk_omarchy::theme::Rgb(background as u32 * 0x010101).fl());
        draw::draw_rectf(0, 0, W, H);
        draw::set_font(case.font, case.size);
        draw::set_draw_color(Color::from_rgb(54, 145, 227));
        draw::draw_text2(
            &paint::escape(case.text),
            12,
            8,
            case.width,
            112,
            case.align | Align::Inside | Align::Clip,
        );
        let image = surface.image().unwrap().to_rgb_data();
        ImageSurface::pop_current();
        image
    }
    unsafe fn face_name(font: Ptr) -> String {
        let description = pango_font_describe(font);
        let name = pango_font_description_to_string(description);
        let value = std::ffi::CStr::from_ptr(name)
            .to_string_lossy()
            .into_owned();
        g_free(name.cast());
        pango_font_description_free(description);
        value
    }
    fn delta(left: &[u8], right: &[u8]) -> u8 {
        assert_eq!(left.len(), right.len());
        left.iter()
            .zip(right)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap_or(0)
    }
    // Diagnostic quadratic reference, not the eventual hot-path algorithm.
    // Candidate: render overlapping ink/clusters together so Cairo combines
    // their masks in one operation. The separate --require-groups gate can
    // reject this partition even when whole native runs reproduce exactly.
    unsafe fn ink_groups(font: Ptr, glyphs: &[GlyphInfo]) -> Vec<std::ops::Range<usize>> {
        let mut groups: Vec<std::ops::Range<usize>> = Vec::new();
        let mut bounds: Vec<[i64; 4]> = Vec::new();
        let mut advance = 0i64;
        for (i, glyph) in glyphs.iter().enumerate() {
            let mut ink = Rectangle::default();
            pango_font_get_glyph_extents(font, glyph.glyph, &mut ink, ptr::null_mut());
            let left = advance + glyph.geometry.x_offset as i64 + ink.x as i64 - 1024;
            let top = glyph.geometry.y_offset as i64 + ink.y as i64 - 1024;
            let rect = [
                left,
                top,
                left + ink.width as i64 + 2048,
                top + ink.height as i64 + 2048,
            ];
            let first = groups
                .iter()
                .position(|group| {
                    group.clone().any(|j| {
                        let b = bounds[j];
                        b[0] < rect[2] && rect[0] < b[2] && b[1] < rect[3] && rect[1] < b[3]
                    })
                })
                .or_else(|| (i > 0 && glyph.attributes & 1 == 0).then(|| groups.len() - 1));
            if let Some(first) = first {
                groups[first].end = i + 1;
                groups.truncate(first + 1);
            } else {
                groups.push(i..i + 1);
            }
            bounds.push(rect);
            advance += glyph.geometry.width as i64;
        }
        groups
    }
    unsafe fn probe(case: &Case, output: &std::path::Path) -> Result<serde_json::Value, String> {
        let native_black = native(case, 0);
        let native_white = native(case, 255);
        // The surface owns this driver/descriptor until we've copied the
        // description into an independently owned Pango context below.
        let surface = ImageSurface::new(W, H, false);
        ImageSurface::push_current(&surface);
        draw::set_font(case.font, case.size);
        let driver = __dynamic_cast(fl_graphics_driver, &BASE_TYPE, &CAIRO_TYPE, -1);
        if driver.is_null() {
            ImageSurface::pop_current();
            return Err("current FLTK driver is not Cairo; probe unsupported".into());
        }
        let description = font_description(driver);
        let map = pango_cairo_font_map_get_default(); // borrowed singleton
        let context = Object::new(pango_font_map_create_context(map));
        pango_context_set_font_description(context.0, description);
        let fontset = Object::new(pango_font_map_load_fontset(
            map,
            context.0,
            description,
            pango_language_get_default(),
        ));
        let metrics = pango_fontset_get_metrics(fontset.0);
        // Matches FLTK's >=1.50.6 compile-header branch. This diagnostic
        // validates that assumption by independent native pixel readback.
        let top_offset = (pango_font_metrics_get_height(metrics)
            - pango_font_metrics_get_descent(metrics)) as f64
            / SCALE;
        pango_font_metrics_unref(metrics);
        let escaped = CString::new(paint::escape(case.text)).map_err(|e| e.to_string())?;
        LINES.with(|lines| lines.borrow_mut().clear());
        CALLBACK_FAILED.with(|failed| failed.set(false));
        layout_lines(
            escaped.as_ptr(),
            12,
            8,
            case.width,
            112,
            (case.align | Align::Inside | Align::Clip).bits() as u32,
            collect,
            ptr::null_mut(),
            0,
            0,
        );
        ImageSurface::pop_current();
        if CALLBACK_FAILED.with(Cell::get) {
            return Err("line callback failed".into());
        }
        let lines = LINES.with(|lines| std::mem::take(&mut *lines.borrow_mut()));
        let layout = Object::new(pango_layout_new(context.0));
        let layouts = [Canvas::new(0, case), Canvas::new(255, case)];
        let batched = [Canvas::new(0, case), Canvas::new(255, case)];
        let grouped = [Canvas::new(0, case), Canvas::new(255, case)];
        let singles = [Canvas::new(0, case), Canvas::new(255, case)];
        let mut runs = Vec::new();
        let mut glyph_count = 0;
        for line in &lines {
            pango_layout_set_text(layout.0, line.text.as_ptr().cast(), line.text.len() as i32);
            // set_cairo() translates the FLTK driver by (+0.5,+0.5),
            // cancelling draw()'s (-0.5,-0.5) for logical 1x surfaces.
            let origin = (line.x as f64, line.y as f64 - top_offset);
            for canvas in &layouts {
                canvas.paint(origin.0, origin.1, |cr| {
                    pango_cairo_show_layout(cr, layout.0)
                });
            }
            let iter = Iter(pango_layout_get_iter(layout.0));
            loop {
                let run = pango_layout_iter_get_run_readonly(iter.0);
                if let Some(run) = run.as_ref() {
                    let item = &*run.item;
                    let glyphs = &*run.glyphs;
                    assert!((0..=65536).contains(&glyphs.count));
                    let mut logical = Rectangle::default();
                    pango_layout_iter_get_run_extents(iter.0, ptr::null_mut(), &mut logical);
                    let baseline = pango_layout_iter_get_run_baseline(iter.0);
                    let original_glyphs =
                        std::slice::from_raw_parts(glyphs.glyphs, glyphs.count as usize);
                    let groups = ink_groups(item.analysis.font, original_glyphs);
                    for group in &groups {
                        let mut subset = original_glyphs[group.clone()].to_vec();
                        let advance: i64 = original_glyphs[..group.start]
                            .iter()
                            .map(|g| g.geometry.width as i64)
                            .sum();
                        for glyph in &mut subset {
                            glyph.geometry.x_offset =
                                i32::try_from(advance + glyph.geometry.x_offset as i64)
                                    .map_err(|_| "glyph group position overflow")?;
                        }
                        let mut clusters = vec![0; subset.len()];
                        let mut string = GlyphString {
                            count: subset.len() as i32,
                            glyphs: subset.as_mut_ptr(),
                            clusters: clusters.as_mut_ptr(),
                            space: subset.len() as i32,
                        };
                        for canvas in &grouped {
                            canvas.paint(
                                origin.0 + logical.x as f64 / SCALE,
                                origin.1 + baseline as f64 / SCALE,
                                |cr| {
                                    pango_cairo_show_glyph_string(
                                        cr,
                                        item.analysis.font,
                                        &mut string,
                                    )
                                },
                            );
                        }
                    }
                    for canvas in &batched {
                        canvas.paint(
                            origin.0 + logical.x as f64 / SCALE,
                            origin.1 + baseline as f64 / SCALE,
                            |cr| pango_cairo_show_glyph_string(cr, item.analysis.font, run.glyphs),
                        );
                    }
                    let mut advance = 0i64;
                    let mut ids = Vec::new();
                    for original in std::slice::from_raw_parts(glyphs.glyphs, glyphs.count as usize)
                    {
                        let mut glyph = *original;
                        let x = origin.0
                            + (logical.x as i64 + advance + glyph.geometry.x_offset as i64) as f64
                                / SCALE;
                        let y = origin.1
                            + (baseline as i64 + glyph.geometry.y_offset as i64) as f64 / SCALE;
                        // Keep Cairo's transform identical to run replay.
                        // Encode the accumulated advance in Pango geometry
                        // rather than moving the CTM for each glyph.
                        glyph.geometry.x_offset =
                            i32::try_from(advance + glyph.geometry.x_offset as i64)
                                .map_err(|_| "glyph position overflow")?;
                        advance += glyph.geometry.width as i64;
                        let mut cluster = 0;
                        let mut one = GlyphString {
                            count: 1,
                            glyphs: &mut glyph,
                            clusters: &mut cluster,
                            space: 1,
                        };
                        for canvas in &singles {
                            canvas.paint(
                                origin.0 + logical.x as f64 / SCALE,
                                origin.1 + baseline as f64 / SCALE,
                                |cr| {
                                    pango_cairo_show_glyph_string(cr, item.analysis.font, &mut one)
                                },
                            );
                        }
                        ids.push(json!({"glyph":glyph.glyph,"x":x,"y":y,
                            "cluster_start":glyph.attributes & 1 != 0,
                            "color":glyph.attributes & 2 != 0}));
                    }
                    glyph_count += glyphs.count;
                    runs.push(json!({"font":face_name(item.analysis.font),
                        "ink_group_sizes":groups.iter().map(|g|g.len()).collect::<Vec<_>>(),
                        "byte_offset":item.offset,"byte_length":item.length,
                        "characters":item.num_chars,"bidi_level":item.analysis.level,"glyphs":ids}));
                }
                if pango_layout_iter_next_run(iter.0) == 0 {
                    break;
                }
            }
        }
        let layout_black = layouts[0].rgb();
        let layout_white = layouts[1].rgb();
        let single_black = singles[0].rgb();
        let single_white = singles[1].rgb();
        let batch_black = batched[0].rgb();
        let batch_white = batched[1].rgb();
        let group_black = grouped[0].rgb();
        let group_white = grouped[1].rgb();
        for (name, rgb) in [
            ("native", &native_black),
            ("layout", &layout_black),
            ("runs", &batch_black),
            ("groups", &group_black),
            ("glyphs", &single_black),
        ] {
            png::write(
                &output.join(format!("{}-{name}.png", case.name)),
                W as u32,
                H as u32,
                3,
                rgb,
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(json!({"case":case.name,"text":case.text,"size":case.size,
            "lines":lines.len(),"glyph_count":glyph_count,"runs":runs,
            "layout_native_max_delta":delta(&layout_black,&native_black).max(delta(&layout_white,&native_white)),
            "run_native_max_delta":delta(&batch_black,&native_black).max(delta(&batch_white,&native_white)),
            "group_native_max_delta":delta(&group_black,&native_black).max(delta(&group_white,&native_white)),
            "glyph_layout_max_delta":delta(&single_black,&layout_black).max(delta(&single_white,&layout_white)),
            "glyph_native_max_delta":delta(&single_black,&native_black).max(delta(&single_white,&native_white))}))
    }

    pub fn run() -> Result<(), String> {
        let output = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/glyph-check"));
        std::fs::create_dir_all(&output).map_err(|e| e.to_string())?;
        let _app = App::new("dev.omarchy.scene-glyph-check");
        Font::set_font(Font::Helvetica, "JetBrainsMono Nerd Font");
        Font::set_font(Font::HelveticaBold, "JetBrainsMono Nerd Font Bold");
        Font::set_font(Font::Courier, "Noto Sans");
        let version = unsafe { pango_version() };
        let backend = unsafe {
            if fl_wl_display().is_null() {
                "x11"
            } else {
                "wayland"
            }
        };
        if version < 15006 {
            return Err("probe requires Pango >=1.50.6; renderer keeps native fallback".into());
        }
        let mut reports = Vec::new();
        for (name, text, size, font, align, width) in [
            (
                "ligatures",
                "AV To office affine ffi fl Wa",
                27,
                Font::Courier,
                Align::Left,
                692,
            ),
            (
                "latin",
                "AV To office affine ffi fl Wa",
                27,
                Font::Helvetica,
                Align::Left,
                692,
            ),
            (
                "marks",
                "café cafe\u{301} A\u{30a} n\u{303} Z\u{307}\u{323}",
                29,
                Font::Helvetica,
                Align::Left,
                692,
            ),
            (
                "arabic",
                "مرحبا بالعالم العربية لا",
                29,
                Font::Helvetica,
                Align::Right,
                692,
            ),
            (
                "mixed",
                "Files 123 שלום 456 العربية end",
                23,
                Font::Helvetica,
                Align::Center,
                692,
            ),
            (
                "fallback",
                "日本語 한글 हिन्दी বাংলা ไทย",
                24,
                Font::Helvetica,
                Align::Left,
                692,
            ),
            (
                "emoji",
                "A 👩🏽‍💻 🇨🇱 ❤️ 🙂 end",
                29,
                Font::Helvetica,
                Align::Left,
                692,
            ),
            (
                "wrap",
                "office @home\tline one\nsecond line wraps with العربية and fallback 日本語",
                19,
                Font::Helvetica,
                Align::Left | Align::Wrap | Align::Top,
                262,
            ),
            (
                "bold",
                "AV office 👩‍💻 café שלום",
                31,
                Font::HelveticaBold,
                Align::Center | Align::Bottom,
                692,
            ),
            (
                "clipped",
                "office العربية 👩‍💻 日本語",
                37,
                Font::Helvetica,
                Align::Right,
                120,
            ),
            (
                "missing",
                "Missing \u{0378} glyph, end",
                23,
                Font::Helvetica,
                Align::Left,
                692,
            ),
        ] {
            let report = unsafe {
                probe(
                    &Case {
                        name,
                        text,
                        size,
                        font,
                        align,
                        width,
                    },
                    &output,
                )?
            };
            println!(
                "{name}: layout/native={} runs/native={} groups/native={} glyph/layout={} glyph/native={}",
                report["layout_native_max_delta"],
                report["run_native_max_delta"],
                report["group_native_max_delta"],
                report["glyph_layout_max_delta"],
                report["glyph_native_max_delta"]
            );
            reports.push(report);
        }
        let runs_passed = reports
            .iter()
            .all(|r| r["run_native_max_delta"].as_u64().unwrap() <= 2);
        let individual_passed = reports
            .iter()
            .all(|r| r["glyph_native_max_delta"].as_u64().unwrap() <= 2);
        let groups_passed = reports
            .iter()
            .all(|r| r["group_native_max_delta"].as_u64().unwrap() <= 2);
        let report = json!({"pango_version":version,"backend":backend,"threshold":2,
            "positioned_runs_passed":runs_passed,"individual_glyphs_passed":individual_passed,
            "ink_groups_passed":groups_passed,
            "scope":"Linux logical 1x positioned native runs and overlapping ink groups; no atlas/cache/GL integration",
            "cases":reports});
        std::fs::write(
            output.join("report.json"),
            format!("{}\n", serde_json::to_string_pretty(&report).unwrap()),
        )
        .map_err(|e| e.to_string())?;
        let require_individual = std::env::args_os().any(|s| s == "--require-individual");
        let require_groups = std::env::args_os().any(|s| s == "--require-groups");
        println!("Positioned runs: {runs_passed}; ink groups: {groups_passed}; individual glyphs: {individual_passed}");
        if runs_passed
            && (!require_groups || groups_passed)
            && (!require_individual || individual_passed)
        {
            Ok(())
        } else {
            Err(format!("glyph parity failed; inspect {}", output.display()))
        }
    }
}
