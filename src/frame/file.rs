//! Frame files: the compatibility path for helpers that write each frame
//! as a PNG or BMP file (o3-code's document and preview hosts today).
//! BMP is decoded here (uncompressed 24/32-bit, the files Qt writes); PNG
//! needs the `images` feature (the `png` crate).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// A decoded frame file: straight-alpha RGBA.
pub(crate) struct Decoded {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Vec<u8>,
    /// When the file was written (ns since the Unix epoch; 0 unknown):
    /// the view measures file written → drawn from it.
    pub(crate) written_ns: u64,
}

/// Largest frame file accepted (pixels), like the shared surfaces.
const MAX_PIXELS: u64 = 16384 * 16384;

/// Decodes a PNG or BMP frame (by its signature).
pub(crate) fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    if bytes.starts_with(b"BM") {
        bmp(bytes)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        png(bytes)
    } else {
        Err("not a PNG or BMP file".into())
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// Uncompressed BMP: 24-bit BGR or 32-bit BGRA/BGRX (`BI_RGB`, or
/// `BI_BITFIELDS` with any byte-aligned masks), bottom-up or top-down.
fn bmp(b: &[u8]) -> Result<Decoded, String> {
    let short = || "short BMP".to_string();
    let data_off = u32_at(b, 10).ok_or_else(short)? as usize;
    let hdr = u32_at(b, 14).ok_or_else(short)? as usize;
    if hdr < 40 {
        return Err("old BMP header".into());
    }
    let w = u32_at(b, 18).ok_or_else(short)? as i32;
    let h = u32_at(b, 22).ok_or_else(short)? as i32;
    let bpp = u16_at(b, 28).ok_or_else(short)?;
    let comp = u32_at(b, 30).ok_or_else(short)?;
    let (top_down, h) = (h < 0, h.unsigned_abs());
    if w <= 0 || h == 0 || u64::from(w as u32) * u64::from(h) > MAX_PIXELS {
        return Err("bad BMP size".into());
    }
    let w = w as u32;
    // Byte offsets of R, G, B, A in a pixel (A: None = opaque).
    let (bytes_pp, order): (usize, [Option<usize>; 4]) = match (bpp, comp) {
        (24, 0) => (3, [Some(2), Some(1), Some(0), None]),
        (32, 0) => (4, [Some(2), Some(1), Some(0), Some(3)]),
        (32, 3) | (32, 6) => {
            // Masks follow a 40-byte header, or sit inside a V4/V5 header.
            let m = |i: usize| u32_at(b, 14 + 40 + i * 4).ok_or_else(short);
            let alpha = if hdr >= 56 || comp == 6 { m(3)? } else { 0 };
            let at = |mask: u32| -> Result<Option<usize>, String> {
                match mask {
                    0 => Ok(None),
                    0xff => Ok(Some(0)),
                    0xff00 => Ok(Some(1)),
                    0xff_0000 => Ok(Some(2)),
                    0xff00_0000 => Ok(Some(3)),
                    _ => Err("unsupported BMP masks".into()),
                }
            };
            (4, [at(m(0)?)?, at(m(1)?)?, at(m(2)?)?, at(alpha)?])
        }
        _ => return Err(format!("unsupported BMP ({bpp} bits, compression {comp})")),
    };
    let row = (w as usize * bytes_pp).div_ceil(4) * 4;
    let need = data_off + row * h as usize;
    if b.len() < need {
        return Err(short());
    }
    let mut rgba = vec![0u8; w as usize * h as usize * 4];
    let mut any_alpha = false;
    for y in 0..h as usize {
        let src_y = if top_down { y } else { h as usize - 1 - y };
        let src = &b[data_off + src_y * row..data_off + src_y * row + w as usize * bytes_pp];
        let dst = &mut rgba[y * w as usize * 4..(y + 1) * w as usize * 4];
        for (s, d) in src.chunks_exact(bytes_pp).zip(dst.chunks_exact_mut(4)) {
            for c in 0..3 {
                d[c] = order[c].map_or(0, |i| s[i]);
            }
            d[3] = order[3].map_or(255, |i| s[i]);
            any_alpha |= d[3] != 0;
        }
    }
    if order[3].is_some() && !any_alpha {
        // 32-bit BI_RGB with an unused (zero) fourth byte: opaque.
        rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    Ok(Decoded { width: w, height: h, rgba, written_ns: 0 })
}

#[cfg(feature = "images")]
fn png(bytes: &[u8]) -> Result<Decoded, String> {
    let mut dec = png::Decoder::new_with_limits(std::io::Cursor::new(bytes), png::Limits { bytes: (MAX_PIXELS * 4) as usize });
    dec.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut r = dec.read_info().map_err(|e| e.to_string())?;
    let (w, h) = (r.info().width, r.info().height);
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err("PNG too large".into());
    }
    let mut buf = vec![0u8; r.output_buffer_size().ok_or("PNG too large")?];
    let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (ct, _) = r.output_color_type();
    let ch = match ct {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err("palette not expanded".into()),
    };
    let (wu, stride) = (w as usize, info.line_size);
    if ch == 4 && stride == wu * 4 {
        buf.truncate(wu * h as usize * 4);
        return Ok(Decoded { width: w, height: h, rgba: buf, written_ns: 0 });
    }
    let mut rgba = vec![0u8; wu * h as usize * 4];
    for y in 0..h as usize {
        let src = &buf[y * stride..y * stride + wu * ch];
        for (x, d) in rgba[y * wu * 4..(y + 1) * wu * 4].chunks_exact_mut(4).enumerate() {
            let p = &src[x * ch..x * ch + ch];
            d.copy_from_slice(&match ch {
                1 => [p[0], p[0], p[0], 255],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 255],
                _ => [p[0], p[1], p[2], p[3]],
            });
        }
    }
    Ok(Decoded { width: w, height: h, rgba, written_ns: 0 })
}

#[cfg(not(feature = "images"))]
fn png(_bytes: &[u8]) -> Result<Decoded, String> {
    Err("PNG frame files need the `images` feature (BMP works without it)".into())
}

/// Reads and decodes one frame file.
pub(crate) fn load(path: &Path) -> Result<Decoded, String> {
    let written = std::fs::metadata(path).ok().and_then(|m| m.modified().ok());
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut d = decode(&bytes)?;
    d.written_ns = written
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64);
    Ok(d)
}

/// Polls `path` every `interval` and calls `changed` with each new
/// decoded frame (a file replaced by rename, as the helpers do, or
/// rewritten). Runs on the calling thread until `stop`.
pub(crate) fn watch(path: PathBuf, interval: Duration, stop: Arc<AtomicBool>, changed: &dyn Fn(Result<Decoded, String>)) {
    let stamp = |p: &Path| std::fs::metadata(p).ok().map(|m| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
    let mut last = None;
    while !stop.load(Ordering::Relaxed) {
        let now = stamp(&path);
        if now.is_some() && now != last {
            last = now;
            changed(load(&path));
        }
        std::thread::sleep(interval);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A bottom-up 24-bit BMP of `w × h` pixels from straight RGB.
    pub(crate) fn bmp24(w: u32, h: u32, rgb: &dyn Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let row = (w as usize * 3).div_ceil(4) * 4;
        let size = 54 + row * h as usize;
        let mut b = Vec::with_capacity(size);
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&(size as u32).to_le_bytes());
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&54u32.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(&(w as i32).to_le_bytes());
        b.extend_from_slice(&(h as i32).to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&24u16.to_le_bytes());
        b.extend_from_slice(&[0; 24]);
        for y in (0..h).rev() {
            let start = b.len();
            for x in 0..w {
                let [r, g, bl] = rgb(x, y);
                b.extend_from_slice(&[bl, g, r]);
            }
            b.resize(start + row, 0);
        }
        b
    }

    #[test]
    fn bmp_24_bit_bottom_up_with_row_padding() {
        let b = bmp24(3, 2, &|x, y| [x as u8 * 10, y as u8 * 100, 7]);
        let d = decode(&b).unwrap();
        assert_eq!((d.width, d.height), (3, 2));
        assert_eq!(&d.rgba[..4], &[0, 0, 7, 255], "top-left");
        assert_eq!(&d.rgba[(3 + 2) * 4..(3 + 2) * 4 + 4], &[20, 100, 7, 255], "bottom-right");
    }

    #[test]
    fn bmp_32_bit_with_zero_alpha_is_opaque_and_top_down_works() {
        let mut b = bmp24(1, 1, &|_, _| [0, 0, 0]);
        // Rewrite as a 2x1 top-down 32-bit BI_RGB image.
        b.truncate(54);
        b[18..22].copy_from_slice(&2i32.to_le_bytes());
        b[22..26].copy_from_slice(&(-1i32).to_le_bytes());
        b[28..30].copy_from_slice(&32u16.to_le_bytes());
        b.extend_from_slice(&[1, 2, 3, 0, 4, 5, 6, 0]);
        let d = decode(&b).unwrap();
        assert_eq!(d.rgba, vec![3, 2, 1, 255, 6, 5, 4, 255]);
        assert!(decode(b"BM\0\0").is_err());
        assert!(decode(b"GIF89a").is_err());
    }

    #[cfg(feature = "images")]
    #[test]
    fn png_frames_decode_with_the_images_feature() {
        let px = [255u8, 0, 0, 255, 0, 255, 0, 128];
        let enc = crate::png::encode(2, 1, 4, &px);
        let d = decode(&enc).unwrap();
        assert_eq!((d.width, d.height), (2, 1));
        assert_eq!(d.rgba, px);
    }

    #[test]
    fn watch_sees_a_replaced_file() {
        let dir = std::env::temp_dir().join(format!("fltk-omarchy-framefile-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("frame.bmp");
        let tmp = dir.join("frame.bmp.next");
        std::fs::write(&path, bmp24(2, 2, &|_, _| [1, 2, 3])).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (s2, seen2, p2) = (stop.clone(), seen.clone(), path.clone());
        let t = std::thread::spawn(move || {
            watch(p2, Duration::from_millis(5), s2, &|d| seen2.lock().unwrap().push(d.map(|d| d.rgba[0])));
        });
        let wait = |n: usize| {
            let end = std::time::Instant::now() + Duration::from_secs(3);
            while seen.lock().unwrap().len() < n && std::time::Instant::now() < end {
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        wait(1);
        std::fs::write(&tmp, bmp24(3, 2, &|_, _| [9, 2, 3])).unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        wait(2);
        stop.store(true, Ordering::Relaxed);
        t.join().unwrap();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "initial + replaced");
        assert_eq!(seen[1], Ok(9));
    }
}
