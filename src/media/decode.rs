//! Decoders (PNG, JPEG, GIF, WebP) into straight RGBA8, downscaled to a
//! target size with an area filter in premultiplied alpha. Pure functions:
//! they run on any thread (the loader runs them on the task pool).

use super::{Bitmap, Decoded, Error, Fit, Format, Frame, Limits, Target};
use std::io::Cursor;
use std::time::Duration;

/// The format of `bytes`, from its signature (not the file name).
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Format::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(Format::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(Format::Gif)
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(Format::WebP)
    } else {
        None
    }
}

/// Size an image of `src` gets for `target` (never larger than `src`).
pub fn fitted_size(src: (u32, u32), target: Target) -> (u32, u32) {
    let (sw, sh) = (src.0.max(1) as f64, src.1.max(1) as f64);
    if target.width == 0 && target.height == 0 {
        return src;
    }
    let tw = if target.width == 0 { f64::MAX } else { target.width as f64 };
    let th = if target.height == 0 { f64::MAX } else { target.height as f64 };
    let s = match target.fit {
        Fit::Cover if target.width > 0 && target.height > 0 => (tw / sw).max(th / sh),
        _ => (tw / sw).min(th / sh),
    }
    .min(1.0);
    (((sw * s).round() as u32).max(1), ((sh * s).round() as u32).max(1))
}

fn check_pixels(w: u32, h: u32, limits: &Limits) -> Result<(), Error> {
    if w == 0 || h == 0 {
        return Err(Error::Corrupt("zero-sized image".into()));
    }
    if w as u64 * h as u64 > limits.max_pixels {
        return Err(Error::TooManyPixels { width: w, height: h, limit: limits.max_pixels });
    }
    Ok(())
}

/// Allocates a zeroed buffer without aborting the process on failure.
fn alloc(len: usize) -> Result<Vec<u8>, Error> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| Error::OutOfMemory { bytes: len })?;
    v.resize(len, 0);
    Ok(v)
}

/// Decodes `bytes` for `target`. `cancelled` is polled between rows and
/// frames; returning true stops with [`Error::Cancelled`].
pub fn decode(bytes: &[u8], target: Target, limits: &Limits, cancelled: &dyn Fn() -> bool) -> Result<Decoded, Error> {
    if bytes.len() > limits.max_input_bytes {
        return Err(Error::TooLarge { bytes: bytes.len(), limit: limits.max_input_bytes });
    }
    match sniff(bytes) {
        Some(Format::Png) => png(bytes, target, limits, cancelled),
        Some(Format::Jpeg) => jpeg(bytes, target, limits, cancelled),
        Some(Format::Gif) => gif(bytes, target, limits, cancelled),
        Some(Format::WebP) => webp(bytes, target, limits, cancelled),
        None => Err(Error::Unsupported),
    }
}

fn single(format: Format, src: (u32, u32), bitmap: Bitmap) -> Decoded {
    Decoded { format, source_size: src, frames: vec![Frame { bitmap, delay: Duration::ZERO }], truncated: false }
}

fn cancel_check(cancelled: &dyn Fn() -> bool) -> Result<(), Error> {
    if cancelled() { Err(Error::Cancelled) } else { Ok(()) }
}

// ------------------------------------------------------------------ PNG

fn png(bytes: &[u8], target: Target, limits: &Limits, cancelled: &dyn Fn() -> bool) -> Result<Decoded, Error> {
    let corrupt = |e: png::DecodingError| match e {
        png::DecodingError::LimitsExceeded => Error::OutOfMemory { bytes: limits.max_decode_bytes() },
        e => Error::Corrupt(e.to_string()),
    };
    let mut dec = png::Decoder::new_with_limits(Cursor::new(bytes), png::Limits { bytes: limits.max_decode_bytes() });
    dec.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = dec.read_info().map_err(corrupt)?;
    let (w, h) = (reader.info().width, reader.info().height);
    check_pixels(w, h, limits)?;
    cancel_check(cancelled)?;
    let size = reader.output_buffer_size().ok_or(Error::OutOfMemory { bytes: usize::MAX })?;
    let mut buf = alloc(size)?;
    let info = reader.next_frame(&mut buf).map_err(corrupt)?;
    let (ct, _) = reader.output_color_type();
    let stride = info.line_size;
    let rgba = to_rgba(&buf, w, h, stride, ct)?;
    drop(buf);
    cancel_check(cancelled)?;
    let bitmap = scale_to(rgba, w, h, target, cancelled)?;
    Ok(single(Format::Png, (w, h), bitmap))
}

fn to_rgba(buf: &[u8], w: u32, h: u32, stride: usize, ct: png::ColorType) -> Result<Vec<u8>, Error> {
    let (w, h) = (w as usize, h as usize);
    let ch = match ct {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(Error::Corrupt("palette was not expanded".into())),
    };
    if stride < w * ch || buf.len() < stride * (h - 1) + w * ch {
        return Err(Error::Corrupt("short image data".into()));
    }
    if ch == 4 && stride == w * 4 {
        return Ok(buf[..w * h * 4].to_vec());
    }
    let mut out = alloc(w * h * 4)?;
    for y in 0..h {
        let row = &buf[y * stride..y * stride + w * ch];
        let o = &mut out[y * w * 4..(y + 1) * w * 4];
        for x in 0..w {
            let p = &row[x * ch..x * ch + ch];
            let px = match ch {
                1 => [p[0], p[0], p[0], 255],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[1], p[2], p[3]],
            };
            o[x * 4..x * 4 + 4].copy_from_slice(&px);
        }
    }
    Ok(out)
}

// ----------------------------------------------------------------- JPEG

fn jpeg(bytes: &[u8], target: Target, limits: &Limits, cancelled: &dyn Fn() -> bool) -> Result<Decoded, Error> {
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;
    let side = (limits.max_pixels.min(u32::MAX as u64) as usize).min(65_535);
    let opts = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(side)
        .set_max_height(side)
        .set_strict_mode(false);
    let mut dec = zune_jpeg::JpegDecoder::new_with_options(Cursor::new(bytes), opts);
    dec.decode_headers().map_err(|e| Error::Corrupt(format!("{e:?}")))?;
    let info = dec.info().ok_or_else(|| Error::Corrupt("no JPEG header".into()))?;
    let (w, h) = (info.width as u32, info.height as u32);
    check_pixels(w, h, limits)?;
    let orientation = dec.exif().map_or(1, |e| exif_orientation(e));
    let size = dec.output_buffer_size().ok_or_else(|| Error::Corrupt("unknown JPEG size".into()))?;
    if size != w as usize * h as usize * 4 {
        return Err(Error::Corrupt("unexpected JPEG layout".into()));
    }
    cancel_check(cancelled)?;
    let mut buf = alloc(size)?;
    dec.decode_into(&mut buf).map_err(|e| Error::Corrupt(format!("{e:?}")))?;
    cancel_check(cancelled)?;
    // EXIF orientation 5..8 swap the axes: size for the rotated picture.
    let swap = orientation >= 5;
    let t = if swap { Target { width: target.height, height: target.width, ..target } } else { target };
    let small = scale_to(buf, w, h, t, cancelled)?;
    let bitmap = orient(small, orientation)?;
    let src = if swap { (h, w) } else { (w, h) };
    Ok(single(Format::Jpeg, src, bitmap))
}

/// EXIF orientation (1..8) from a TIFF-structured EXIF block, 1 if absent.
pub(crate) fn exif_orientation(tiff: &[u8]) -> u8 {
    let t = tiff.strip_prefix(b"Exif\0\0").unwrap_or(tiff);
    if t.len() < 8 {
        return 1;
    }
    let le = match &t[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return 1,
    };
    let u16_at = |i: usize| t.get(i..i + 2).map(|b| if le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) });
    let u32_at = |i: usize| t.get(i..i + 4).map(|b| if le { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) });
    let Some(ifd) = u32_at(4).map(|o| o as usize) else { return 1 };
    let Some(n) = u16_at(ifd) else { return 1 };
    for i in 0..n.min(512) as usize {
        let e = ifd + 2 + i * 12;
        if u16_at(e) == Some(0x0112) {
            return u16_at(e + 8).filter(|v| (1..=8).contains(v)).map_or(1, |v| v as u8);
        }
    }
    1
}

/// Applies an EXIF orientation to straight RGBA pixels.
fn orient(b: Bitmap, o: u8) -> Result<Bitmap, Error> {
    if o <= 1 || o > 8 {
        return Ok(b);
    }
    let (w, h) = (b.width as usize, b.height as usize);
    let (nw, nh) = if o >= 5 { (h, w) } else { (w, h) };
    let src = b.rgba();
    let mut out = alloc(w * h * 4)?;
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match o {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                _ => (y, w - 1 - x), // 8
            };
            let (s, d) = ((y * w + x) * 4, (ny * nw + nx) * 4);
            out[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    Bitmap::new(nw as u32, nh as u32, out)
}

// ------------------------------------------------------------------ GIF

fn gif(bytes: &[u8], target: Target, limits: &Limits, cancelled: &dyn Fn() -> bool) -> Result<Decoded, Error> {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::RGBA);
    let mem = std::num::NonZeroU64::new(limits.max_decode_bytes() as u64).unwrap_or(std::num::NonZeroU64::MIN);
    opts.set_memory_limit(gif::MemoryLimit::Bytes(mem));
    let mut dec = opts.read_info(Cursor::new(bytes)).map_err(|e| Error::Corrupt(e.to_string()))?;
    let (w, h) = (dec.width() as u32, dec.height() as u32);
    check_pixels(w, h, limits)?;
    let (cw, ch) = (w as usize, h as usize);
    let mut canvas = alloc(cw * ch * 4)?;
    let mut saved: Option<Vec<u8>> = None;
    let mut pending: Option<(gif::DisposalMethod, (usize, usize, usize, usize))> = None;
    let mut frames = Vec::new();
    let mut out_bytes = 0usize;
    let mut truncated = false;
    loop {
        cancel_check(cancelled)?;
        let frame = match dec.read_next_frame() {
            Ok(Some(f)) => f,
            Ok(None) => break,
            // A broken frame after good ones: keep what decoded (as browsers do).
            Err(e) if !frames.is_empty() => {
                let _ = e;
                truncated = true;
                break;
            }
            Err(e) => return Err(Error::Corrupt(e.to_string())),
        };
        // Dispose of the previous frame first.
        match pending.take() {
            Some((gif::DisposalMethod::Background, (fx, fy, fw, fh))) => {
                for y in fy..(fy + fh).min(ch) {
                    let a = (y * cw + fx.min(cw)) * 4;
                    let b = (y * cw + (fx + fw).min(cw)) * 4;
                    canvas[a..b].fill(0);
                }
            }
            Some((gif::DisposalMethod::Previous, _)) => {
                if let Some(s) = saved.take() {
                    canvas = s;
                }
            }
            _ => {}
        }
        let (fx, fy, fw, fh) = (frame.left as usize, frame.top as usize, frame.width as usize, frame.height as usize);
        if frame.dispose == gif::DisposalMethod::Previous {
            let mut copy = Vec::new();
            copy.try_reserve_exact(canvas.len()).map_err(|_| Error::OutOfMemory { bytes: canvas.len() })?;
            copy.extend_from_slice(&canvas);
            saved = Some(copy);
        }
        let buf = &frame.buffer;
        if buf.len() >= fw * fh * 4 {
            for y in 0..fh {
                let cy = fy + y;
                if cy >= ch {
                    break;
                }
                for x in 0..fw {
                    let cx = fx + x;
                    if cx >= cw {
                        break;
                    }
                    let s = (y * fw + x) * 4;
                    if buf[s + 3] != 0 {
                        let d = (cy * cw + cx) * 4;
                        canvas[d..d + 4].copy_from_slice(&buf[s..s + 4]);
                    }
                }
            }
        }
        pending = Some((frame.dispose, (fx, fy, fw, fh)));
        // Browsers play 0–10 ms delays at 100 ms.
        let cs = frame.delay as u64;
        let delay = Duration::from_millis(if cs <= 1 { 100 } else { cs * 10 });
        let bitmap = scale_to(canvas.clone(), w, h, target, cancelled)?;
        out_bytes += bitmap.bytes();
        if frames.len() >= limits.max_frames || (out_bytes > limits.max_output_bytes && !frames.is_empty()) {
            truncated = true;
            break;
        }
        frames.push(Frame { bitmap, delay });
    }
    if frames.is_empty() {
        return Err(Error::Corrupt("GIF without frames".into()));
    }
    if frames.len() == 1 {
        frames[0].delay = Duration::ZERO;
    }
    Ok(Decoded { format: Format::Gif, source_size: (w, h), frames, truncated })
}

// ----------------------------------------------------------------- WebP

fn webp(bytes: &[u8], target: Target, limits: &Limits, cancelled: &dyn Fn() -> bool) -> Result<Decoded, Error> {
    let corrupt = |e: image_webp::DecodingError| Error::Corrupt(e.to_string());
    let mut dec = image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(corrupt)?;
    dec.set_memory_limit(limits.max_decode_bytes());
    let (w, h) = dec.dimensions();
    check_pixels(w, h, limits)?;
    let ch = if dec.has_alpha() { 4 } else { 3 };
    let size = dec.output_buffer_size().ok_or(Error::OutOfMemory { bytes: usize::MAX })?;
    if size != w as usize * h as usize * ch {
        return Err(Error::Corrupt("unexpected WebP layout".into()));
    }
    let mut buf = alloc(size)?;
    let expand = |buf: &[u8]| -> Result<Vec<u8>, Error> {
        if ch == 4 {
            return Ok(buf.to_vec());
        }
        let mut out = alloc(w as usize * h as usize * 4)?;
        for (o, p) in out.chunks_exact_mut(4).zip(buf.chunks_exact(3)) {
            o.copy_from_slice(&[p[0], p[1], p[2], 255]);
        }
        Ok(out)
    };
    if !dec.is_animated() {
        dec.read_image(&mut buf).map_err(corrupt)?;
        cancel_check(cancelled)?;
        let bitmap = scale_to(expand(&buf)?, w, h, target, cancelled)?;
        return Ok(single(Format::WebP, (w, h), bitmap));
    }
    let mut frames = Vec::new();
    let mut out_bytes = 0usize;
    let mut truncated = false;
    for _ in 0..dec.num_frames() {
        cancel_check(cancelled)?;
        let ms = match dec.read_frame(&mut buf) {
            Ok(ms) => ms,
            Err(image_webp::DecodingError::NoMoreFrames) => break,
            Err(_) if !frames.is_empty() => {
                truncated = true;
                break;
            }
            Err(e) => return Err(corrupt(e)),
        };
        let bitmap = scale_to(expand(&buf)?, w, h, target, cancelled)?;
        out_bytes += bitmap.bytes();
        if frames.len() >= limits.max_frames || (out_bytes > limits.max_output_bytes && !frames.is_empty()) {
            truncated = true;
            break;
        }
        let delay = Duration::from_millis(if ms <= 10 { 100 } else { ms as u64 });
        frames.push(Frame { bitmap, delay });
    }
    if frames.is_empty() {
        return Err(Error::Corrupt("WebP without frames".into()));
    }
    if frames.len() == 1 {
        frames[0].delay = Duration::ZERO;
    }
    Ok(Decoded { format: Format::WebP, source_size: (w, h), frames, truncated })
}

// ---------------------------------------------------------------- scale

fn scale_to(rgba: Vec<u8>, w: u32, h: u32, target: Target, cancelled: &dyn Fn() -> bool) -> Result<Bitmap, Error> {
    let (dw, dh) = fitted_size((w, h), target);
    if (dw, dh) == (w, h) {
        return Bitmap::new(w, h, rgba);
    }
    let out = resize(&rgba, w as usize, h as usize, dw as usize, dh as usize, cancelled)?;
    Bitmap::new(dw, dh, out)
}

/// Spans of source pixels (index, weight) for each destination pixel of
/// an area (box) filter from `s` to `d` pixels, `d <= s`.
fn spans(s: usize, d: usize) -> Vec<Vec<(usize, f32)>> {
    let ratio = s as f64 / d as f64;
    (0..d)
        .map(|i| {
            let (a, b) = (i as f64 * ratio, ((i + 1) as f64 * ratio).min(s as f64));
            let mut v = Vec::with_capacity(ratio.ceil() as usize + 1);
            let mut x = a.floor() as usize;
            while (x as f64) < b && x < s {
                let lo = (x as f64).max(a);
                let hi = ((x + 1) as f64).min(b);
                if hi > lo {
                    v.push((x, ((hi - lo) / ratio) as f32));
                }
                x += 1;
            }
            v
        })
        .collect()
}

/// Area-filter downscale of straight RGBA8 (premultiplied while averaging,
/// so transparent pixels do not darken edges). Also handles `d > s` on an
/// axis by nearest neighbour (not used by the decoders).
pub(crate) fn resize(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize, cancelled: &dyn Fn() -> bool) -> Result<Vec<u8>, Error> {
    let xs = if dw <= sw { spans(sw, dw) } else { (0..dw).map(|i| vec![(i * sw / dw, 1.0)]).collect() };
    let ys = if dh <= sh { spans(sh, dh) } else { (0..dh).map(|i| vec![(i * sh / dh, 1.0)]).collect() };
    let mut out = alloc(dw * dh * 4)?;
    let mut row = vec![0f32; dw * 4];
    let mut acc = vec![0f32; dw * 4];
    for (oy, yspan) in ys.iter().enumerate() {
        if oy % 64 == 0 {
            cancel_check(cancelled)?;
        }
        acc.iter_mut().for_each(|v| *v = 0.0);
        for &(sy, wy) in yspan {
            let line = &src[sy * sw * 4..(sy + 1) * sw * 4];
            for (ox, xspan) in xs.iter().enumerate() {
                let mut p = [0f32; 4];
                for &(sx, wx) in xspan {
                    let s = &line[sx * 4..sx * 4 + 4];
                    let a = s[3] as f32 * wx;
                    p[0] += s[0] as f32 * a;
                    p[1] += s[1] as f32 * a;
                    p[2] += s[2] as f32 * a;
                    p[3] += a;
                }
                row[ox * 4..ox * 4 + 4].copy_from_slice(&p);
            }
            for (a, r) in acc.iter_mut().zip(&row) {
                *a += r * wy;
            }
        }
        let o = &mut out[oy * dw * 4..(oy + 1) * dw * 4];
        for x in 0..dw {
            let a = acc[x * 4 + 3];
            if a <= 0.0 {
                o[x * 4..x * 4 + 4].fill(0);
                continue;
            }
            let q = |v: f32| (v / a + 0.5).clamp(0.0, 255.0) as u8;
            o[x * 4] = q(acc[x * 4]);
            o[x * 4 + 1] = q(acc[x * 4 + 1]);
            o[x * 4 + 2] = q(acc[x * 4 + 2]);
            o[x * 4 + 3] = (a + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never() -> bool {
        false
    }

    /// A PNG of `w × h` straight RGBA pixels, through the png crate.
    pub(crate) fn png_bytes(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            let mut wr = e.write_header().unwrap();
            wr.write_image_data(rgba).unwrap();
        }
        out
    }

    /// A two-frame GIF: red then blue, 4 × 2, delays 20 ms and 50 ms; the
    /// second frame covers only the right half (Keep disposal).
    pub(crate) fn gif_bytes() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let pal = [255, 0, 0, 0, 0, 255];
            let mut e = gif::Encoder::new(&mut out, 4, 2, &pal).unwrap();
            e.set_repeat(gif::Repeat::Infinite).unwrap();
            let mut f = gif::Frame { width: 4, height: 2, delay: 2, buffer: std::borrow::Cow::Owned(vec![0; 8]), ..Default::default() };
            f.dispose = gif::DisposalMethod::Keep;
            e.write_frame(&f).unwrap();
            let f2 = gif::Frame { left: 2, width: 2, height: 2, delay: 5, buffer: std::borrow::Cow::Owned(vec![1; 4]), ..Default::default() };
            e.write_frame(&f2).unwrap();
        }
        out
    }

    #[test]
    fn sniffs_signatures_not_names() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n...."), Some(Format::Png));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some(Format::Jpeg));
        assert_eq!(sniff(b"GIF89a"), Some(Format::Gif));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some(Format::WebP));
        assert_eq!(sniff(b"<svg"), None);
        assert!(matches!(decode(b"hello", Target::original(), &Limits::default(), &never), Err(Error::Unsupported)));
    }

    #[test]
    fn fitted_sizes_never_upscale() {
        let t = |w, h, fit| Target { width: w, height: h, fit };
        assert_eq!(fitted_size((4000, 3000), t(400, 400, Fit::Contain)), (400, 300));
        assert_eq!(fitted_size((4000, 3000), t(400, 400, Fit::Cover)), (533, 400));
        assert_eq!(fitted_size((100, 50), t(400, 400, Fit::Contain)), (100, 50));
        assert_eq!(fitted_size((4000, 3000), t(0, 0, Fit::Contain)), (4000, 3000));
        assert_eq!(fitted_size((4000, 3000), t(1000, 0, Fit::Contain)), (1000, 750));
    }

    #[test]
    fn png_decodes_and_downscales_with_premultiplied_alpha() {
        // Left column opaque white, right column fully transparent *black*.
        let mut px = Vec::new();
        for _y in 0..4 {
            for x in 0..4 {
                px.extend_from_slice(if x < 2 { &[255, 255, 255, 255] } else { &[0, 0, 0, 0] });
            }
        }
        let bytes = png_bytes(4, 4, &px);
        let d = decode(&bytes, Target::original(), &Limits::default(), &never).unwrap();
        assert_eq!((d.format, d.source_size, d.frames.len()), (Format::Png, (4, 4), 1));
        assert_eq!(d.first().rgba(), &px[..]);
        // 4 → 1 wide: half covered, still white (not grey).
        let t = Target { width: 1, height: 1, fit: Fit::Contain };
        let d = decode(&bytes, t, &Limits::default(), &never).unwrap();
        assert_eq!((d.first().width, d.first().height), (1, 1));
        let p = d.first().rgba();
        assert_eq!(&p[..3], &[255, 255, 255]);
        assert!((p[3] as i32 - 128).abs() <= 1, "{p:?}");
    }

    #[test]
    fn limits_are_errors_not_panics() {
        let bytes = png_bytes(64, 64, &vec![200; 64 * 64 * 4]);
        let small = Limits { max_pixels: 1000, ..Limits::default() };
        assert!(matches!(decode(&bytes, Target::original(), &small, &never), Err(Error::TooManyPixels { width: 64, height: 64, .. })));
        let tiny = Limits { max_input_bytes: 10, ..Limits::default() };
        assert!(matches!(decode(&bytes, Target::original(), &tiny, &never), Err(Error::TooLarge { .. })));
        // Truncated and garbled files.
        for cut in [9, 20, 40, bytes.len() / 2, bytes.len() - 5] {
            assert!(decode(&bytes[..cut], Target::original(), &Limits::default(), &never).is_err(), "cut at {cut}");
        }
        let mut bad = bytes.clone();
        for b in bad.iter_mut().skip(40).step_by(7) {
            *b ^= 0x5a;
        }
        assert!(decode(&bad, Target::original(), &Limits::default(), &never).is_err());
        for prefix in [&b"\xff\xd8\xff\xe0\0\x10JFIF"[..], b"GIF89a\x01", b"RIFF\x10\0\0\0WEBPVP8 \x04\0\0\0abcd"] {
            assert!(decode(prefix, Target::original(), &Limits::default(), &never).is_err());
        }
        // A huge declared size in a tiny file is refused before allocating.
        let mut huge = bytes.clone();
        huge[16..20].copy_from_slice(&60000u32.to_be_bytes());
        huge[20..24].copy_from_slice(&60000u32.to_be_bytes());
        assert!(decode(&huge, Target::original(), &Limits::default(), &never).is_err());
        // Cancellation.
        assert!(matches!(decode(&bytes, Target::original(), &Limits::default(), &|| true), Err(Error::Cancelled)));
    }

    #[test]
    fn gif_frames_delays_and_disposal() {
        let d = decode(&gif_bytes(), Target::original(), &Limits::default(), &never).unwrap();
        assert_eq!(d.format, Format::Gif);
        assert_eq!(d.frames.len(), 2);
        assert_eq!(d.frames[0].delay, Duration::from_millis(20));
        assert_eq!(d.frames[1].delay, Duration::from_millis(50));
        let (f0, f1) = (d.frames[0].bitmap.rgba(), d.frames[1].bitmap.rgba());
        assert_eq!(&f0[0..4], &[255, 0, 0, 255]);
        assert_eq!(&f1[0..4], &[255, 0, 0, 255], "kept from frame 1");
        assert_eq!(&f1[12..16], &[0, 0, 255, 255], "frame 2 on the right half");
        assert_eq!(d.frame_at(Duration::from_millis(10)).0, 0);
        assert_eq!(d.frame_at(Duration::from_millis(30)).0, 1);
        assert_eq!(d.frame_at(Duration::from_millis(75)).0, 0, "loops");
        let one = Limits { max_frames: 1, ..Limits::default() };
        let d = decode(&gif_bytes(), Target::original(), &one, &never).unwrap();
        assert!(d.truncated && d.frames.len() == 1);
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/media").join(name)).unwrap()
    }

    fn near(p: [u8; 4], q: [u8; 4]) -> bool {
        p.iter().zip(q).all(|(a, b)| (*a as i32 - b as i32).abs() <= 24)
    }

    #[test]
    fn real_files_of_every_format() {
        // quad: 64 × 48, red | green over blue | white.
        let quad = |d: &Decoded, alpha: u8| {
            let b = d.first();
            let (w, h) = (b.width, b.height);
            near(b.pixel(w / 4, h / 4), [255, 0, 0, alpha])
                && near(b.pixel(3 * w / 4, h / 4), [0, 255, 0, alpha])
                && near(b.pixel(w / 4, 3 * h / 4), [0, 0, 255, alpha])
                && near(b.pixel(3 * w / 4, 3 * h / 4), [255, 255, 255, alpha])
        };
        for (name, fmt, alpha) in [("quad.png", Format::Png, 255), ("quad.jpg", Format::Jpeg, 255), ("quad.webp", Format::WebP, 255), ("quad-alpha.webp", Format::WebP, 128)] {
            let d = decode(&fixture(name), Target::original(), &Limits::default(), &never).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!((d.format, d.source_size), (fmt, (64, 48)), "{name}");
            assert!(quad(&d, alpha), "{name}: {:?}", d.first().pixel(16, 12));
            let half = decode(&fixture(name), Target::new(32, 32, Fit::Contain), &Limits::default(), &never).unwrap();
            assert_eq!(half.size(), (32, 24), "{name}");
            assert!(quad(&half, alpha), "{name} downscaled");
        }
        // EXIF orientation 6: displayed rotated 90° clockwise (48 × 64, blue top-left).
        let d = decode(&fixture("quad-rot6.jpg"), Target::new(24, 0, Fit::Contain), &Limits::default(), &never).unwrap();
        assert_eq!((d.source_size, d.size()), ((48, 64), (24, 32)));
        assert!(near(d.first().pixel(4, 4), [0, 0, 255, 255]), "{:?}", d.first().pixel(4, 4));
        assert!(near(d.first().pixel(20, 4), [255, 0, 0, 255]));
        // Animations: every frame with its delay.
        let g = decode(&fixture("rgb.gif"), Target::original(), &Limits::default(), &never).unwrap();
        assert_eq!(g.frames.iter().map(|f| f.delay.as_millis()).collect::<Vec<_>>(), vec![100, 300, 200]);
        assert!(near(g.frames[1].bitmap.pixel(8, 8), [0, 255, 0, 255]));
        let w = decode(&fixture("rg.webp"), Target::original(), &Limits::default(), &never).unwrap();
        assert_eq!(w.frames.len(), 2);
        assert_eq!(w.frames[1].delay.as_millis(), 300);
        assert!(near(w.frames[0].bitmap.pixel(8, 8), [255, 0, 0, 255]) && near(w.frames[1].bitmap.pixel(8, 8), [0, 255, 0, 255]));
        // Truncated real files fail cleanly.
        for name in ["quad.jpg", "quad.webp", "rgb.gif", "rg.webp"] {
            let b = fixture(name);
            for cut in [b.len() / 3, b.len() * 2 / 3] {
                let _ = decode(&b[..cut], Target::original(), &Limits::default(), &never);
            }
        }
    }

    #[test]
    fn exif_orientation_is_read_and_applied() {
        // Little-endian TIFF, one IFD entry: orientation = 6 (rotate 90° cw).
        let mut t = b"II*\0\x08\0\0\0\x01\0".to_vec();
        t.extend_from_slice(&[0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0]);
        assert_eq!(exif_orientation(&t), 6);
        assert_eq!(exif_orientation(b"junk"), 1);
        // 2 × 1 (red, green) rotated 90° cw → 1 × 2 (red on top).
        let b = Bitmap::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        let r = orient(b, 6).unwrap();
        assert_eq!((r.width, r.height), (1, 2));
        assert_eq!(&r.rgba()[..4], &[255, 0, 0, 255]);
    }
}
