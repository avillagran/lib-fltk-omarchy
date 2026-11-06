# External frames: `frame::FrameView`

`frame` shows what another process draws and sends that process the
user's input. It is meant for helpers that cannot live inside the app: a
browser engine, HTML attachments, Mermaid, MCP apps, Qt Quick previews,
device screens. The helper (the *producer*) draws into shared memory; the
app shows it in a `FrameView` (the *consumer*).

- Shared memory with **three buffers**: the producer never waits, the
  view always takes the newest frame and never a half-written one.
- The producer **wakes** the view (a pipe, a FIFO or an event): no
  polling, 0 % CPU while nothing changes.
- **Input** goes back as toolkit-neutral events (JSON lines) in a ring
  in the same memory; the producer can answer (content size, clipboard,
  its own messages) through a second ring.
- A **file mode** shows PNG/BMP frame files the way o3-code's helpers
  write them today, so an app can migrate one helper at a time.

```rust
use fltk_omarchy::frame::{FrameView, Input};
use std::process::Command;

let view = FrameView::new(800, 560)
    .name("Preview")                       // screen readers
    .on_content_size(|w, h| println!("page is {w}×{h}"))
    .on_event(|e: &Input| { /* also see every event */ });
body.add(&*view);                          // un-fitted: fills the body
let child = view.spawn(Command::new("my-renderer"))?;
```

The child (Rust):

```rust
use fltk_omarchy::frame::{Input, Producer};
let mut p = Producer::from_env()?;         // reads OMARCHY_FRAME
loop {
    for e in p.wait_events(Some(std::time::Duration::from_millis(33))) {
        if let Input::Resize { width, height, scale } = e {
            p.set_size((width as f32 * scale) as u32, (height as f32 * scale) as u32);
        }
    }
    let (w, h) = p.size();
    let stride = p.stride();
    draw_into(p.frame_mut(), w, h, stride); // RGBA by default
    p.present();
}
```

`examples/frame_view.rs` and `examples/frame_producer.rs` are a complete
pair (`cargo build --example frame_producer --example frame_view`, then
run `target/debug/examples/frame_view`).

## Who creates the surface

| Situation | Consumer (app) | Producer (helper) |
|---|---|---|
| The app starts the helper (usual) | `view.spawn(cmd)` or `Consumer::create` + `consumer.share(&mut cmd)` + `release_remote()` | `Producer::from_env()` |
| The helper exists already | `view.create_surface()` → handle (named objects); pass it any way | `Producer::open(&handle)` |
| The helper creates it | `view.open(&handle)` | `Producer::create_with(Options::new(w, h).transport(Transport::Named))`, print the handle |

`Options` sets the initial size, the **capacity** (largest frame; default
2560 × 1600, or the initial size when larger), the transport and the
directory for FIFOs and frame files (default: a private directory in
`platform::dirs::runtime()`).

## Transports per OS

| | Memory | Wake-up | Handle |
|---|---|---|---|
| Linux, `Transport::Inherit` (default) | `memfd_create` (fallback: an unlinked file in the runtime dir) | pipes, one byte per wake-up | `fd:<pid>:<mem>:<present>:<input>` (descriptors inherited by the child) |
| macOS, BSDs, `Inherit` | `shm_open` + `shm_unlink` (fallback: unlinked file) | pipes | same |
| Unix, `Transport::Named` | `shm_open("/ofrm-<id>")` | FIFOs `<id>.present` / `<id>.input` in `<dir>/fltk-omarchy-frames-<uid>` (0700) | `named:<id>:<dir>` |
| Windows, `Named` (also chosen for `Inherit`) | `CreateFileMappingW("Local\fltk-omarchy-<id>")` | named auto-reset events `…-present`, `…-input` | `named:<id>` |
| Every OS, `Transport::File` | a 0600 file `<id>.frame` in the private dir, mapped | none: the waiting side polls the header every 8 ms | `file:<path>` |
| Other OSes | not supported: use the file mode below | | |

`Inherit` falls back to `Named`, then `File`, when anonymous memory is not
available. Every descriptor is close-on-exec except while the one child
that should inherit it is spawned. The creator removes only what it
created (its shm name, its two FIFOs, its frame file) when dropped;
nothing deletes a directory. All OS code is in `src/platform/frame/`.

## Wire format (for producers in other languages)

A producer in C++ (Qt) needs `mmap`/`MapViewOfFile`, C++ atomics and a
JSON parser. All numbers are little-endian; shared fields are aligned
atomics.

**Handle**: read `OMARCHY_FRAME`. `fd:<pid>:<mem>:<present>:<input>`:
descriptor numbers in the child; `mmap` `<mem>` (size from `fstat`),
write one byte to `<present>` after each present, read `<input>` (non
blocking) to wait for input. `named:<id>:<dir>`: `shm_open("/ofrm-<id>")`,
open both FIFOs read-write and non-blocking. `named:<id>` on Windows:
`OpenFileMappingW("Local\fltk-omarchy-<id>")`, `OpenEventW` on `…-present`
(`SetEvent`) and `…-input` (wait). `file:<path>`: map the file; no
wake-ups (the view polls).

**Header** (layout version 1):

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | magic `"OFRM"` |
| 4 | 4 | version = 1 |
| 8 | 4 | `slots_offset` (page aligned) |
| 12 | 4 | slot count = 3 |
| 16 | 8 | `slot_bytes` (page aligned) |
| 24 | 8 | `total_bytes` |
| 32 / 36 | 4 + 4 | capacity: `max_width`, `max_height` (pixels) |
| 40 | 4 | `ring_bytes` = 65536 |
| 48 | 8 | `seq`: frames presented |
| 56 | 4 | `middle`: slot index, bit 2 (value 4) = holds a frame not taken yet |
| 60 | 4 | `producer_back`: slot the producer draws into |
| 64 | 4 | `consumer_front`: slot the view shows |
| 68 | 4 | `closed`: bit 0 producer gone, bit 1 consumer gone |
| 72 / 76 | 4 + 4 | producer pid, consumer pid |
| 80 / 84 / 88 | 4 × 3 | last requested size: logical width, height, scale (f32 bits) |
| 92 | 4 | zoom (f32 bits) |
| 96 | 8 | `drawn_seq`: last frame the view drew |
| 128 | 3 × 32 | slot descriptors: width, height, stride (u32), format (u32: 1 RGBA, 2 BGRA, both premultiplied; bit 8 = opaque), seq (u64), present time (u64, ns since the Unix epoch) |
| 224 | 12 | input ring: head, tail, dropped (u32) |
| 240 | 12 | output ring: head, tail, dropped (u32) |
| 4096 | 65536 | input ring bytes (view → producer) |
| 69632 | 65536 | output ring bytes (producer → view) |
| `slots_offset` | 3 × `slot_bytes` | pixels |

**Presenting** (producer): start with `back = producer_back`. Write the
pixels of slot `back`, its descriptor (with `seq = seq + 1`), then
`prev = middle.exchange(back | 4, acq_rel)`, `back = prev & 3`, store
`producer_back = back` and `seq` (release), write one byte to the
present pipe / `SetEvent`. The view takes the middle slot only when bit 2
is set, swapping in the slot it showed, so the two sides never touch the
same slot.

**Rings**: single writer, single reader. `head` and `tail` count bytes
ever written and read (wrapping at 2³²); the byte at count `n` lives at
`n % 65536`. The writer copies one whole `\n`-terminated JSON line, then
stores `head` (release); a line that does not fit is dropped (`dropped`
counts them). The reader copies `head - tail` bytes and stores `tail`.

## Input events

Pointer coordinates are in **frame pixels** of the frame the view showed
(letterbox bars and HiDPI scaling already taken out).

| `type` | Fields |
|---|---|
| `move` | `x`, `y`, `buttons`, `mods` |
| `press`, `release`, `double` | `x`, `y`, `button`, `buttons`, `mods` |
| `leave` | |
| `wheel` | `x`, `y`, `dx`, `dy` (notches; positive `dy` scrolls down), `mods` |
| `key`, `keyup` | `key` (W3C-style: `"a"`, `"Enter"`, `"ArrowLeft"`, `"F5"`), `text`, `mods`, `repeat` (`key` only) |
| `focus-in`, `focus-out` | |
| `resize` | `width`, `height` (logical pixels), `scale` (device pixels per logical pixel) |
| `zoom` | `value` (0.5–2) |
| `copy` | |

Buttons: left 1, right 2, middle 4. `mods`: Shift 1, Ctrl 2, Alt 4,
Meta (Super/Cmd) 8. The app can also send its own JSON commands with
`FrameView::send_json` (the producer reads them with
`Producer::messages_raw`).

- **Hover moves** are sent at most every 40 ms (`move_interval`), the
  newest one wins; moves with a button held go at once.
- **Double click**: the second press sends `press`, then `double`.
- **Resize** is reported at once when a source is attached, then 160 ms
  after the size stops changing (`resize_debounce`). The newest request
  is also in the header for a producer that starts late.
- **Zoom**: `set_zoom` / `zoom_in` / `zoom_out` / `reset_zoom` (steps of
  0.25, clamped to 0.5–2): the producer renders at that zoom (crisper than
  scaling the frame).
- **Wheel**: with `owns_wheel(false)` the view takes the wheel only while
  focused, so an inline preview in a scrolled transcript lets it scroll.

**Keyboard**: the view is one Tab stop. While it is focused every key
goes to the producer except Tab / Shift+Tab (leave the view), Esc (stays
with the app unless `pass_escape(true)`) and keys the app keeps with
`reserve(|key, mods| ..)` (its own shortcuts). Mod+C also sends `copy`;
the producer answers with a `clipboard` message, and the view sets the
clipboard only within a second of that request (a page cannot fill the
clipboard on its own).

**Messages back**: `{"type":"content-size","width":..,"height":..}`
(`on_content_size`: o3's inline HTML sizes itself between 240 and 1800 px
from it), `{"type":"clipboard","text":..}`, anything else goes to
`on_message` as `Message::Other`.

### Qt helpers

The events stay toolkit-neutral; o3-code maps them to the Qt codes its
WebEngine and Qt Quick hosts expect. `frame::qt` has the mapping:
`qt::key("ArrowLeft")` → `Qt::Key_Left`, `qt::modifiers(mods)` (Control
and Meta swapped on macOS, as Qt does), `qt::buttons`, `qt::angle_delta`
(120 per notch, positive away from the user) and `qt::to_qt_json(&event)`,
which produces exactly the JSON `document-host.h` parses today (`type`,
`x`, `y`, `button`, `buttons`, `modifiers`, `key`, `text`, `ax`, `ay`, …).

## Frame files (compatibility)

o3's hosts write `frame-0.png` / `frame-1.png` (or BMP to tmpfs) through
a temporary file and `rename`, and announce each one on stdout.

- `view.show_file(path)` on each announcement: decoded on one worker, the
  newest request wins (a slow decode never shows an older frame later).
- `view.watch_file(path, interval)` polls one path (at least 8 ms).
- `view.events_to(child_stdin)` writes the input events as JSON lines to
  the helper's stdin, as o3 does now (or translate them with `frame::qt`
  in `on_event`).

BMP (24-bit and 32-bit, `BI_RGB` or `BI_BITFIELDS`, bottom-up or
top-down) is decoded without dependencies; PNG needs the `images`
feature. `frame::decode_frame_file` exposes the decoder.

## Display

- Draws only when a frame arrives (or the widget is exposed).
- `Fit::Contain` (default): scaled to fit, aspect kept, theme background
  around it. A frame of the size the view asked for shows 1:1.
  `Fit::Actual`: one frame pixel per device pixel, centred or cropped.
- Device pixels come from `native::pixels_per_unit` (Wayland output scale
  included), so a producer that follows `resize` × `scale` gives sharp
  frames on HiDPI outputs.
- Opaque RGBA frames are handed to FLTK straight from the shared slot
  (`FrameStats::zero_copy`); BGRA or translucent frames are converted once
  per frame. FLTK still converts the image for the display (XRender on
  X11, Cairo on Wayland): that is the remaining copy, 4–5 ms for a
  1280 × 800 frame (below).
- `FrameStats`: frames received, skipped (the producer was faster),
  drawn, latency present → drawn (last, mean, max), last draw time.

## Accessibility

A `FrameView` is a focusable **Document** (`FrameRole::Image` for
pictures such as a device screen) named with `.name(..)`. What is inside
the frame is not exposed: the producer owns that content.

## Measurements

`script/frame-bench` runs `examples/frame_view --bench` against
`examples/frame_producer` for each mode: 30 fps, 10 s after 1 s warm-up,
release build, a page-like pattern (text rows, a moving bar) and a
photo-like one (gradients + per-pixel noise, like video). 2026-11-06,
AMD Ryzen 9 5900X. Raw lines:
[frame-bench-2026-11-06-x11.jsonl](frame-bench-2026-11-06-x11.jsonl),
[frame-bench-2026-11-06-wayland.jsonl](frame-bench-2026-11-06-wayland.jsonl).

**X11 (Xvfb), 1280 × 800:**

| Content, path | fps shown | Latency mean / max | UI CPU | Producer CPU | Producer encode | PSS UI / producer |
|---|---|---|---|---|---|---|
| page, shared memory | 30.1 | 4.6 / 5.5 ms | 13.9 % | 7.0 % | — | 20.1 / 6.5 MB |
| page, PNG files | 30.0 | 7.5 / 9.1 ms + 9.5 ms encode | 20.5 % | 34.4 % | 9.5 ms | 25.6 / 4.7 MB |
| page, BMP files | 29.5 | 12.2 / 78 ms + 7.3 ms encode | 23.9 % | 24.1 % | 7.3 ms | 26.2 / 8.5 MB |
| photo, shared memory | 30.0 | 4.9 / 5.8 ms | 14.7 % | 10.1 % | — | 19.2 / 6.5 MB |
| photo, PNG files | **12.4** | 16.5 / 26 ms + 44 ms encode | 17.2 % | 58.8 % | 44.1 ms | 28.2 / 7.4 MB |
| photo, BMP files | 29.8 | 11.7 / 58 ms + 7.2 ms encode | 26.0 % | 29.7 % | 7.2 ms | 27.0 / 8.4 MB |

**Wayland (Hyprland, hidden workspace; the tiled window gave 1100 × 800):**

| Content, path | fps shown | Latency mean / max | UI CPU | Producer CPU | PSS UI / producer |
|---|---|---|---|---|---|
| page, shared memory | 30.0 | 4.3 / 4.9 ms | 13.0 % | 7.0 % | 22.9 / 5.6 MB |
| page, PNG files | 30.0 | 7.9 / 15 ms + 9.1 ms encode | 19.8 % | 33.0 % | 28.6 / 4.7 MB |
| page, BMP files | 29.4 | 24.2 / 60 ms + 10.6 ms encode | 49.4 % | 35.3 % | 29.9 / 8.5 MB |
| photo, shared memory | 29.9 | 7.1 / 17 ms | 19.9 % | 10.0 % | 22.9 / 5.7 MB |
| photo, PNG files | **14.0** | 13.7 / 17 ms + 35 ms encode | 16.7 % | 53.2 % | 33.9 / 12.0 MB |
| photo, BMP files | 30.0 | 9.1 / 12 ms + 5.1 ms encode | 21.4 % | 23.9 % | 32.2 / 8.5 MB |

What the numbers say:

- **Latency**: with shared memory the frame is on screen 4–5 ms after
  `present`, and almost all of it is FLTK converting and blitting the
  image (`draw_ms_mean` 4.1–4.6 ms). With a 1 × 1 frame the whole path
  (ring the pipe, wake the thread, wake the UI loop, take the slot) took
  0.07–0.1 ms. The file path adds the encode before the file exists
  (9–44 ms) and the read + decode after it: 17 ms (page PNG) to 60 ms
  (photo PNG) from "frame ready" to screen, against 4.6 ms.
- **CPU at 30 fps 1280 × 800**: shared memory costs 14 % (UI) + 7–10 %
  (producer, its own drawing only) of a core. PNG costs 20 % + 34 % on
  page content and cannot reach 30 fps on photo-like content (44 ms per
  encode: 12–14 fps at 59 % of a core). BMP avoids the encode but the
  UI's read + convert costs more than PNG decode on Wayland (49 %).
- **Memory**: the surface for 1280 × 800 frames touches 3 × 4 MB, shared
  by both processes (PSS splits it); the default capacity reserves
  3 × 16 MB of address space, used only as frames grow. The file path
  holds the decoded copy and the file in the page cache on top
  (+5–11 MB PSS on the UI side).
- Remaining cost: FLTK's per-frame conversion. On Wayland a Cairo surface
  over the shared premultiplied BGRA slot would remove it; not done yet.

## Limits

- The capacity is fixed when the surface is created; a producer asked
  for a larger size clamps to it (`Producer::set_size` returns the size
  that applies) and the view scales the frame.
- Windows (named mapping and events) and macOS (`shm_open`, pipes) are
  type-checked, not run. Linux is tested: unit tests, a second process
  per transport (`tests/frame.rs`), UI checks (`tests/frame_ui`).
- A producer that dies is seen through its pipe closing (`Inherit`) or
  its `closed` bit (clean exit); with named objects a crash is not
  detected (the app owns the `Child`).
- Input methods: committed text arrives in `key` events' `text`; no
  preedit forwarding. No cursor-shape messages yet.
