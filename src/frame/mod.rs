//! External frame surfaces: show what another process draws and send it
//! the user's input, with as few copies as possible.
//!
//! The app shows a [`FrameView`]; a helper process (a browser engine, an
//! HTML, Mermaid or QML renderer, a device-screen decoder) draws into a
//! [`Producer`]. They share memory: a small header, three frame buffers
//! (the producer never waits, the view always shows the newest frame and
//! never a torn one) and two rings of JSON lines (input to the producer,
//! messages back). The producer wakes the view through a pipe, FIFO or
//! event; the view never polls. Where shared memory is not available a
//! mapped file is polled at a capped rate, and helpers that still write
//! PNG/BMP files work through [`FrameView::watch_file`] /
//! [`FrameView::show_file`].
//!
//! ```no_run
//! use fltk_omarchy::frame::{FrameView, Input};
//! # let app = fltk_omarchy::App::new("dev.example.frames");
//! # let mut win = fltk_omarchy::AppWindow::new("Frames", 800, 600);
//! let view = FrameView::new(800, 560)
//!     .name("Preview")
//!     .on_event(|e: &Input| println!("{}", e.to_line().trim()));
//! let child = view.spawn(std::process::Command::new("my-renderer")).expect("spawn");
//! # win.end(); win.show(); app.run();
//! ```
//!
//! In the child:
//!
//! ```no_run
//! let mut p = fltk_omarchy::frame::Producer::from_env().unwrap();
//! let (w, h) = p.size();
//! p.frame_mut()[..(w * h * 4) as usize].fill(255);
//! p.present();
//! ```
//!
//! Guide, wire format for producers in other languages and measurements:
//! docs/FRAMES.md.

mod event;
mod file;
pub mod qt;
mod ring;
mod shm;
mod surface;
mod view;

pub use event::{Button, Input, Message, Mods};
pub use shm::Format;
pub use surface::{Consumer, Frame, Handle, Options, Producer, Transport, DEFAULT_CAPACITY, ENV};
pub use view::{Fit, FrameRole, FrameStats, FrameView};

/// Decodes a PNG or BMP frame file into straight-alpha RGBA
/// `(width, height, pixels)` (PNG needs the `images` feature). For tools
/// and benchmarks of the file path.
pub fn decode_frame_file(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    file::decode(bytes).map(|d| (d.width, d.height, d.rgba))
}
