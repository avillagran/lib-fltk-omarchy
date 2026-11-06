# Images, icons, video and audio

Two modules:

- **`icon`** (always built, no dependencies): SVG icons such as Lucide,
  parsed into `gfx` paths and drawn tinted by a theme role at any size and
  device pixel ratio. Nerd Font glyphs (`widgets::glyph`) keep working.
- **`media`** (crate feature `images`, off by default): PNG, JPEG, GIF and
  WebP decoded on the task pool at the size they are shown, a byte-bounded
  cache, the `Image` widget and the `ImageViewer` popup. With the feature
  `playback` (off by default, builds on `images`): the `Video` and `Audio`
  players on the system `ffmpeg` ([Video and audio](#video-and-audio)).

`cargo run --example media --features images` shows both
(`-- --snapshot DIR` writes `media.png`, `media-viewer.png` and
`media-viewer-zoom.png`; `-- a.jpg b.webp` shows your files).
`cargo run --example playback --features playback -- clip.mp4` plays a
file (without one it generates a test clip).

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

## Video and audio

Turn the feature on: `lib-fltk-omarchy = { …, features = ["playback"] }`
(it includes `images`). No media library is linked: the players run the
**system `ffmpeg` and `ffprobe`** as child processes, one per stream.

```rust
use fltk_omarchy::media::{self, Audio, Video};

media::muted().set(prefs.muted);                       // the app persists the global mute
media::muted().subscribe(|m| save_muted(*m)).forever();
let video = Video::new(640, 400).file(&path).label("clip.mp4");   // autoplays when shown
body.fixed(&*video, 400);
let audio = Audio::new(420).file(&song).label("voice-note.ogg");  // plays on Play
body.fit(&*audio);
```

- **Probe**: when a player is first drawn, ffprobe (on a worker) reads
  the duration, the first video track (size with sample aspect ratio and
  rotation applied, declared frame rate; cover art is not video) and the
  first audio track (`media::playback::Info`). Nothing starts for a player
  that is never shown.
- **Video**: ffmpeg decodes, converts and scales to the size the picture
  has in device pixels (`native::pixels_per_unit`, so Wayland output
  scales count), aspect-fit, never above the source, at most 4K, and
  writes RGB24 frames to a pipe at the declared rate (60 fps at most,
  30 when unknown). A reader thread holds each frame until it is due
  (`first frame + n / rate`) and publishes it to a latest-frame slot: at
  most three frame buffers per stream, reused, none allocated per frame;
  the UI takes the newest one when it draws (a slow UI drops frames, the
  clock does not slip). The pipe's back-pressure keeps ffmpeg at most a
  short queue ahead. A larger player restarts the stream at the larger
  size once the resize settles (0.3 s). RGB rather than RGBA: opaque
  frames are 25 % smaller and FLTK draws them without blending.
- **Audio**: a second ffmpeg (sound only, `-re`) converts to 48 kHz
  stereo s16 and plays it through the first output that works
  (`src/platform/playback.rs`):

  | OS | Outputs, in order |
  |---|---|
  | Linux / BSD | ffmpeg `pulse` (PulseAudio, and PipeWire through pipewire-pulse), `pw-play`, `paplay`, ffmpeg `alsa`, `aplay`, `ffplay` |
  | macOS | ffmpeg `audiotoolbox` (Core Audio), `ffplay` |
  | Windows | `ffplay` (ffmpeg has no Windows audio output device) |

  ffmpeg's own output device is the lightest choice (one process, the
  PCM never leaves it); a player is fed by ffmpeg's stdout, child to
  child, so the PCM never passes through the app either. Outputs ffmpeg
  was not built with (`ffmpeg -devices`) or that are not installed are
  skipped; one that fails within 2.5 s of starting (no server, no device)
  is skipped for the rest of the run and the next one starts at the
  current position. With none left the video plays **without sound** and
  says so (a caption on the picture, the audio player's name line, the
  accessible description; `Video::without_sound()`).
  `OMARCHY_FLTK_AUDIO_OUT=<name>` forces one output, `null` decodes in
  real time and discards (the UI tests use it, so tests never make a
  sound), `none` simulates no output.
- **Sync**: both streams start at the same position; the clock starts at
  the first video frame (sound only: at start). No drift correction: the
  audio device clock and the system clock agree to well under a frame
  over a clip; the start offset is the difference between the two
  processes' start-up times (a few tens of ms).
- **Controls**: play/pause button, position slider (drag seeks on
  release; Slider role, value "0:12 of 1:30"), time, mute button
  (Unmute when muted). Keys while focus is anywhere in the player: Space
  or K play/pause, Left/Right ±5 s, Home/End, M mute; Tab walks the
  controls; clicking the picture plays/pauses and focuses Play. Seeks
  closer than 0.15 s restart ffmpeg once; a paused seek shows the frame
  there (a one-frame ffmpeg run). The controls row stays left to right in
  right-to-left languages (a timeline). Play and the slider are disabled
  while there is nothing to play. Colors come from the theme when drawn;
  the only motion is the toolkit's control feedback, which follows the
  system animation setting.
- **Mute** is global (`media::muted()`, a `State<bool>`): muting stops
  every player's audio process; unmuting starts it at the current
  position. Off at start; the app restores and saves it.
- **Processes** (`media::playback::live_processes()`): pause, seek (until
  it settles), end of file, hiding the player (not drawn for 1.5 s and
  its window not shown), deleting it, `Video::stop()` and
  `media::playback::stop_all()` kill **and reap** them on the spot.
  `App` calls `stop_all` when it ends. If the app dies anyway: on Linux
  every child has `PR_SET_PDEATHSIG = SIGKILL` (set by the thread that
  reads and reaps it); on Unix an `atexit` hook kills them on `exit`; on
  Windows they are in a job object that kills them when the app's last
  handle closes; the video stream also ends on its broken pipe. Core dumps
  of children are off; on Windows they start without a console window.
- **Only local files**: the app passes a path; it must be absolute and a
  regular file. ffmpeg gets `file:<path>` (no protocol or pattern is read
  from the name), `-protocol_whitelist file` and a demuxer allowlist of
  self-contained containers (MP4/MOV, Matroska/WebM, AVI, MPEG-TS/PS,
  Ogg, MP3, WAV, FLAC, AAC, AIFF, GIF, FLV, ASF, CAF, W64, WavPack, IVF):
  no playlists (HLS, DASH, concat), image sequences, devices or network.
  `-max_alloc 256 MiB` bounds single allocations. This is defense in
  depth, not a decoder sandbox.
- **Tools**: `PATH`, then Homebrew/MacPorts on macOS and WinGet, Scoop,
  Chocolatey, `Program Files\ffmpeg\bin` on Windows (apps started from
  the Finder or the Start menu get a short `PATH`).
  `media::playback::set_tool_dir(Some(dir))` uses the app's own copy.
  Without ffmpeg or ffprobe the player shows "Playback needs ffmpeg and
  ffprobe" (translated) with Play disabled.
- **Accessibility**: the player is a group named by its `label`; Play,
  Pause, Mute and Unmute are named buttons with their shortcut; the
  position is a slider (increment/decrement ±5 s, set value) with the
  time as text; the picture is an image whose description says why it
  cannot play or has no sound.

### Measurements

Release build, this machine (24 cores, Linux), `playback_bench`
(`cargo run --release --example playback_bench --features playback --
clip.mp4 --box 640x360`): a 10 s 1920 × 1080 H.264 + AAC clip at 30 fps,
8 s of steady playback after 1 s, silent audio output (`null`: same
decode and conversion as a real output, nothing sent to a device). CPU
is % of one core; PSS averaged over the run. Two runs each, rounded.

| | Wayland (Hyprland, hidden workspace) | X11 (Xvfb) |
|---|---|---|
| 640 × 360 picture: app CPU / ffmpeg CPU | 6.7 % / 22 % | 6.1 % / 20.5 % |
| 640 × 360: app PSS (idle) / ffmpeg PSS (2 processes) | 15 MB (9) / 108 MB | 11.7 MB (6.6) / 105 MB |
| native 1920 × 1080: app CPU / ffmpeg CPU | 45 % / 37 % | 45 % / 39 % |
| native: app PSS / ffmpeg PSS | 52 MB / 172 MB | 47 MB / 175 MB |
| ffprobe, set file → ready | 50 ms | 47 ms |
| Play → first frame | 67–76 ms | 67–77 ms |
| Frames shown per second | 30.0 | 30.0 |
| Processes left after `stop()` / after closing the window mid-play | 0 / 0 | 0 / 0 |

- ffmpeg's PSS is mostly ffmpeg itself: the audio process alone is about
  38 MB (a distribution ffmpeg maps dozens of codec libraries), the video
  process about 68 MB at 640 × 360 and 135 MB at native 1080p (decoder
  reference frames plus RGB frames of 6 MB in ffmpeg's queues). Measured
  ways to keep it there, now in the defaults: two decoder threads (each
  frame thread +8 MB at 1080p; one thread already decodes 1080p H.264 at
  18 % of a core, ffmpeg's default of one per core adds only memory), one
  scaler thread for small pictures (default: one per core), a muxer queue
  of two frames (190 → 142 MB at native size), and on glibc two malloc
  arenas with a fixed 1 MiB mmap threshold for the children (216 → 130 MB
  in the same test).
- The app's cost at native size is FLTK putting a 6 MB RGB image on the
  window 30 times a second (Cairo on Wayland, Xlib on X11), plus the
  frame buffers (three in the slot and reader, one image). Drawing
  straight from the buffer (`fl_draw_image`) was slower on X11 (82 %).
  Showing video at a moderate size is cheap; a GPU texture path is not
  done.
- Process clean-up is also checked by the UI tests (pause, seek, end,
  hidden, deleted, `stop_all`), counting both the toolkit's registry and
  the app's child processes in `/proc`.

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
- Playback needs a system ffmpeg; no hardware decoding, no volume control
  (mute only), no subtitles, no track choice (first video and audio
  track), no playback speed. Pause and seek restart ffmpeg (a frame
  appears 70–80 ms later); A/V sync is open loop (no drift correction).
- Files without a duration (some WebM recordings) play, but the slider
  is off.
- Audio output was verified on this Linux machine with the silent output
  and with a PulseAudio connection failure (the real `pulse` device fails
  at once and the player continues without sound); the other outputs are
  built from their documented command lines. macOS (`audiotoolbox`,
  Homebrew paths) and Windows (`ffplay`, job object, no console window)
  are type-checked only.
- At native 1080p the app spends about half a core drawing frames; ffmpeg
  needs about 170 MB PSS for its two processes.

Fixtures: `tests/fixtures/icons` holds unmodified Lucide icons
(lucide-static 0.564.0, ISC, `LICENSE-lucide`) and o3-code UI icons;
`tests/fixtures/media` holds small images made for the tests.
