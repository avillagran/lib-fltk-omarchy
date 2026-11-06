# Images and icons

Two modules:

- **`icon`** (always built, no dependencies): SVG icons such as Lucide,
  parsed into `gfx` paths and drawn tinted by a theme role at any size and
  device pixel ratio. Nerd Font glyphs (`widgets::glyph`) keep working.
- **`media`** (crate feature `images`, off by default): PNG, JPEG, GIF and
  WebP decoded on the task pool at the size they are shown, a byte-bounded
  cache, the `Image` widget and the `ImageViewer` popup.

`cargo run --example media --features images` shows both
(`-- --snapshot DIR` writes `media.png`, `media-viewer.png` and
`media-viewer-zoom.png`; `-- a.jpg b.webp` shows your files).

## Icons

```rust
use fltk_omarchy::icon::{self, lucide, Icon, Svg, Tint};
use fltk_omarchy::text::ColorRole;

let folder = Svg::cached(lucide::FOLDER)?;                // built-in, parsed once
let rocket = Svg::parse(include_str!("rocket.svg"))?;      // any SVG of the subset
row.fit(&*Icon::new(&folder, 16).tint(ColorRole::Dim));   // decorative
row.fit(&*Icon::new(&rocket, 16).label("Project icon"));  // named image
row.fit(&*Icon::new(&rocket, 16).tooltip("Launch").on_click(|| launch()));  // a button
// In any draw callback (`w` is the widget being drawn):
icon::draw(&w, &folder, x, y, 16, 16, theme::with(|t| t.palette.accent));
// In a gfx::View paint callback:
folder.paint(canvas, 8.0, 8.0, 24.0, 24.0, u.accent);
```

- **Tint**: `Tint::Role(ColorRole::…)` is resolved when drawn, so
  `omarchy-theme-set` restyles icons live; `Tint::Fixed(rgb)` is for colors
  that come from app data (o3's project colors, already picked from the
  theme palette). `currentColor`, and a fill left unspecified, take the
  tint; colors written in the SVG stay (multicolor logos). A translucent
  tint applies to the icon as a whole (crossing strokes do not darken).
- **Supported SVG** (`icon::svg`): `svg` with `viewBox` (any size and
  origin, fitted `xMidYMid meet`), `g`, `path` (M L H V C S Q T A Z,
  relative and absolute, compact arc flags), `circle`, `ellipse`, `rect`
  with `rx`/`ry`, `line`, `polyline`, `polygon`; `fill`, `stroke`,
  `stroke-width`, `stroke-linecap`, `stroke-linejoin`,
  `stroke-miterlimit`, `fill-rule` (non-zero and even-odd), `opacity`,
  `fill-opacity`, `stroke-opacity`, inline `style`, `transform`.
  Ignored: gradients and patterns (drawn in the tint), `use`, `text`,
  `image`, clip paths, masks, filters, CSS sheets, dashes; the contents
  of `defs`, `clipPath`, `mask`, `symbol`, `title`, `desc`, `metadata`.
- **Errors are values**: `Svg::parse` returns `SvgError { message,
  offset }` for malformed markup or path data; sources over 1 MiB, more
  than 4 096 shapes or 200 000 path segments are refused.
- **Raster cache**: one RGBA raster per (icon, device size, color), LRU,
  4 MiB by default (`icon::set_cache_budget`, `icon::cache_stats`).
- **Accessibility**: an `Icon` is decorative (absent from the accessible
  tree) unless it has a `label` or a `tooltip`; with `on_click` it is a
  button named by its tooltip, a Tab stop, and runs on Enter, Space or
  click, with the `ActionButton` look.

Checked against o3-code's assets: all 1 917 Lucide project icons and the
110 UI icons parse and rasterize with ink
(`cargo run --release --example icons_check -- lucide-project-icons.json`):

| 2 027 icons, release, one core | per icon |
|---|---|
| parse | 5.1 µs |
| rasterize 16 px | 25.6 µs |
| rasterize 24 px | 32.8 µs |
| rasterize 48 px | 78.1 µs |

A first frame with 100 different icons costs about 3 ms; later frames
blit cached rasters.

## Images

Turn the feature on: `lib-fltk-omarchy = { …, features = ["images"] }`.
It adds four pure-Rust decoders (png 0.18, gif 0.14, zune-jpeg 0.5,
image-webp 0.2; MIT/Apache/Zlib) and needs **Rust 1.80.1** (image-webp
0.2.4); the rest of the crate keeps 1.80. fltk keeps its `no-images`
build, so apps without the feature (Files) are unchanged.

```rust
use fltk_omarchy::media::{Fit, Image, ImageViewer, Source};

let src = Source::file(&path);                 // or Source::bytes("attachment-42", bytes)
let thumb = Image::new(220, 160).source(&src).fit(Fit::Cover).framed(true).label(&name);
let t = thumb.clone();
let thumb = thumb.on_activate(move || {
    if let Some(v) = t.viewer() {
        v.on_copy(|bitmap| clipboard_set_png(bitmap.to_png())).open(&*t);
    }
});
```

- **Decoding**: on the task pool (`task::Task`), only when the widget is
  first drawn (images scrolled away are not decoded), at the widget's size
  in device pixels (`native::pixels_per_unit`, rounded up to 64 px steps;
  a larger size is decoded when the widget grows). Downscaling is an area
  filter in premultiplied alpha (no dark fringes); the decoder never
  upscales. JPEG EXIF orientation is applied. Output is straight RGBA8
  (`media::Bitmap`), what FLTK blends.
- **Formats**: PNG (8/16-bit, gray, palette, alpha; the first frame of an
  APNG), JPEG (baseline and progressive, CMYK), GIF (every frame,
  disposal, delays; 0–10 ms delays play at 100 ms like browsers), WebP
  (lossy, lossless, alpha, animated). Detected by signature, not by name.
  HEIC and AVIF give `Error::Unsupported`.
- **Limits** (`media::Limits`, per decode): 32 MiB of input, 40 megapixels
  per frame (o3's limit), 1 000 frames, 96 MiB of decoded frames (later
  frames are dropped, `Decoded::truncated`). Dimensions are checked from
  the header before any pixel buffer is allocated, allocations use
  `try_reserve`, files are read with a bound. Every failure is an
  `Error` value (`TooLarge`, `TooManyPixels`, `Unsupported`, `Corrupt`,
  `OutOfMemory`, `Io`, `Cancelled`); a broken frame after good ones keeps
  the good ones. Release builds use `panic = "abort"`: the toolkit's own
  code does not panic on input, and the decoders were exercised with
  truncated and garbled files in the tests.
- **Cache**: decoded images are kept by (source, target size), LRU, 64 MiB
  of pixels by default (`media::set_cache_budget`, `media::cache_stats`,
  `media::forget(&source)` after the file changes, `media::peek`).
  Concurrent loads of the same image share one decode; failures are
  remembered until `forget`. `media::load(&src, target, |r| …)` and
  `media::decode_file` are available without the widget.
- **`Image` widget**: `Fit::Contain` (whole image, the background shows
  around it), `Fit::Cover` (fills, cropped), `Fit::Letterbox` (whole image
  on `darker_background` bars); rounded corners with the theme's
  `badge_radius()` (`.rounded(false)` for square); `.framed(true)` for
  o3's thumbnail card. While loading it shows a faint box with the Lucide
  image icon; on error the alert icon in the urgent color. Colors are read
  when drawn: a theme change restyles it.
- **Animation**: GIF and WebP frames follow the shared animation clock
  (`anim::now`, so `anim::clock::freeze`/`advance` drive them in tests).
  One fltk timeout per visible image, armed by its draw for the next
  frame's delay; an image that is hidden or scrolled away is not drawn, so
  its timer is not re-armed and it stops. With the system's animations
  off (`anim::enabled() == false`) the first frame shows.
- **Viewer** (`ImageViewer`, an overlay layer like `ConfirmDialog`): the
  image fitted under a toolbar (zoom out, zoom in, fit, copy if
  `on_copy` is set, close), never above its natural size at zoom 1; it
  shows the thumbnail at once and swaps in a decode up to twice the
  window size (4 096 px a side at most). Keys: Esc closes and focus goes
  back, `+`/`=` and `-` zoom (×1.25, 0.1–32×), `0` fits, arrows pan (10 %
  of the view, Shift 50 %), Mod+C copies, Tab/Shift+Tab walk the toolbar
  and Enter/Space press. The wheel zooms at the pointer, dragging pans, a
  click outside the image closes. The copy hook receives the frame on
  screen as a `Bitmap` (`to_png()` gives `image/png` bytes); for the full
  original resolution decode with `media::decode_file(path,
  Target::original(), …)`. Toolbar names and states are translated
  (`tr!`) and the toolbar mirrors in right-to-left languages.
- **Accessibility**: an `Image` with a `label` is an image node with that
  name; without one it is decorative; with `on_activate` it is a button
  (Tab stop, Enter, Space, click, focus ring). The viewer is a modal
  dialog named after the image with named toolbar buttons and their
  shortcuts.

### Measurements

Release build, this machine (Linux, Xvfb), `media_bench`
(`cargo run --release --example media_bench --features images -- files…`):

| File | Full decode | 640 × 480 thumbnail (cover) |
|---|---|---|
| JPEG 4000 × 3000 (12 MP, 1.9 MB) | 94 ms | 88 ms → 640 × 480 |
| PNG 3840 × 2160 (screenshot-like, 138 KB) | 120 ms | 143 ms → 853 × 480 |
| WebP 2000 × 1500 lossy (308 KB) | 96 ms | 104 ms → 640 × 480 |
| GIF 480 × 270, 60 frames (3 MB) | 54 ms (31 MB of frames) | same (smaller than the target) |

Thumbnails cost the full decode (none of the decoders scales during the
DCT); the downscale itself is a few ms. Decodes run on workers, so the UI
thread only blits.

Memory (PSS, `media_bench --memory`, 24 thumbnails of 140 × 105 in one
window; baseline is the same window drawn with empty images, ≈ 13 MB):

| Files shown | Cache | PSS growth |
|---|---|---|
| one 12 MP JPEG | 0.2 MB | +0.3 MB |
| one 4K PNG | 0.2 MB | +0.0 MB |
| one 2000 × 1500 WebP | 0.2 MB | +0.3 MB |
| one 60-frame GIF | 14 MB | +23 MB |
| all four | 7.5 MB | +7.9 MB |

Full-size frames are freed right after downscaling. On glibc that memory
would stay in the process (its mmap threshold grows to the largest block
freed, so later multi-MB buffers come from arenas that are not returned:
+9 to +21 MB for the same single-file runs before the fix), so the first
decode fixes the mmap threshold at 4 MiB and the trim threshold at
128 KiB (`platform::memory::return_large_allocations`) and large decodes
end with `malloc_trim` (`platform::memory::trim_heap`). Other allocators
need neither; there both are no-ops.

The feature adds nothing to apps that do not call it: the stripped
release `gallery` example is 2 520 776 bytes without and 2 520 936 with
`images`; `examples/media`, which uses all four decoders, is 2.2 MB.

## Known gaps

- HEIC and AVIF are not decoded (o3 hands them to Qt's kimageformats
  today); APNG shows its first frame.
- No color management (ICC profiles are ignored), no 16-bit output.
- The viewer upscales with FLTK's bilinear filter past the decoded size
  (smooth, not pixel-exact).
- The `Image` widget composes a copy of the visible frame per widget
  (crop and corner mask); many widgets showing one large animated GIF hold
  one copy per frame drawn.
- SVG: no gradients, dashes, `use` or text (none appear in Lucide or o3's
  icons).

Fixtures: `tests/fixtures/icons` holds unmodified Lucide icons
(lucide-static 0.564.0, ISC, `LICENSE-lucide`) and o3-code UI icons;
`tests/fixtures/media` holds small images made for the tests.
