//! The memory layout of a frame surface, shared by both processes. All
//! numbers are little-endian (every supported OS); every field the two
//! sides both touch is an aligned atomic. Layout version 1:
//!
//! ```text
//! offset  size  field
//!      0     4  magic "OFRM"
//!      4     4  version (1)
//!      8     4  slots_offset (where slot 0 starts, page aligned)
//!     12     4  slot count (3)
//!     16     8  slot_bytes (capacity of one slot, page aligned)
//!     24     8  total_bytes
//!     32     4  max_width   (pixels a slot can hold at 4 bytes/pixel,
//!     36     4  max_height   stride = max_width * 4 at most)
//!     40     4  ring_bytes (each event ring, a power of two)
//!     44     4  reserved
//!     48     8  seq: frames presented so far (producer)
//!     56     4  middle: triple-buffer slot shared by both sides,
//!                bit 2 (4) set while it holds a frame not yet taken
//!     60     4  producer_back: the slot the producer draws into
//!     64     4  consumer_front: the slot the consumer shows
//!     68     4  closed: bit 0 producer gone, bit 1 consumer gone
//!     72     4  producer_pid
//!     76     4  consumer_pid
//!     80     4  want_width   last size the consumer asked for (pixels)
//!     84     4  want_height
//!     88     4  want_scale   f32 bits (device pixels per logical pixel)
//!     92     4  zoom         f32 bits
//!     96     8  drawn_seq: last frame the consumer drew
//!    128  3×32  slot descriptors: width u32, height u32, stride u32,
//!                format u32 (bits 0-7: 1 RGBA, 2 BGRA, premultiplied;
//!                bit 8: opaque), seq u64, present time u64 (ns since
//!                the Unix epoch, for latency)
//!    224    12  input ring:  head u32, tail u32, dropped u32
//!    240    12  output ring: head u32, tail u32, dropped u32
//!   4096  ring  input ring bytes (consumer → producer, JSON lines)
//!   4096+ring  ring  output ring bytes (producer → consumer, JSON lines)
//!  slots_offset  3 × slot_bytes  pixels
//! ```

use super::ring::Ring;
use crate::platform::frame::Mapping;
use std::io;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub(crate) const MAGIC: u32 = u32::from_le_bytes(*b"OFRM");
pub(crate) const VERSION: u32 = 1;
pub(crate) const SLOTS: u32 = 3;
pub(crate) const RING: u32 = 64 * 1024;
const PAGE: usize = 4096;
const FRESH: u32 = 4;

const O_MAGIC: usize = 0;
const O_VERSION: usize = 4;
const O_SLOTS_OFF: usize = 8;
const O_SLOTS: usize = 12;
const O_SLOT_BYTES: usize = 16;
const O_TOTAL: usize = 24;
const O_MAX_W: usize = 32;
const O_MAX_H: usize = 36;
const O_RING: usize = 40;
const O_SEQ: usize = 48;
const O_MIDDLE: usize = 56;
const O_BACK: usize = 60;
const O_FRONT: usize = 64;
const O_CLOSED: usize = 68;
const O_PRODUCER_PID: usize = 72;
const O_CONSUMER_PID: usize = 76;
const O_WANT_W: usize = 80;
const O_WANT_H: usize = 84;
const O_WANT_SCALE: usize = 88;
const O_ZOOM: usize = 92;
const O_DRAWN: usize = 96;
const O_DESC: usize = 128;
const DESC: usize = 32;
const O_IN_RING: usize = 224;
const O_OUT_RING: usize = 240;

pub(crate) const CLOSED_PRODUCER: u32 = 1;
pub(crate) const CLOSED_CONSUMER: u32 = 2;

/// Pixel layout of a frame (4 bytes per pixel, premultiplied alpha).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// R, G, B, A in memory (Qt `Format_RGBA8888_Premultiplied`).
    Rgba,
    /// B, G, R, A in memory (Qt `Format_ARGB32_Premultiplied` on
    /// little-endian machines, Cairo `ARGB32`, Windows DIBs).
    Bgra,
}

impl Format {
    pub(crate) fn code(self) -> u32 {
        match self {
            Format::Rgba => 1,
            Format::Bgra => 2,
        }
    }
    pub(crate) fn from_code(c: u32) -> Option<Format> {
        match c & 0xff {
            1 => Some(Format::Rgba),
            2 => Some(Format::Bgra),
            _ => None,
        }
    }
}

pub(crate) const OPAQUE: u32 = 0x100;

/// Bytes needed for a surface whose slots hold `max_w × max_h` pixels.
pub(crate) fn layout(max_w: u32, max_h: u32) -> (usize, usize, usize) {
    let slot = round_page(max_w as usize * 4 * max_h as usize);
    let slots_off = PAGE + 2 * RING as usize;
    (slots_off, slot, slots_off + SLOTS as usize * slot)
}

fn round_page(n: usize) -> usize {
    n.div_ceil(PAGE).max(1) * PAGE
}

enum Backing {
    Map(#[allow(dead_code)] Mapping),
    /// Tests and in-process use without the OS layer.
    #[allow(dead_code)]
    Heap(Vec<u64>),
}

/// A mapped surface: typed access to the header, slots and rings.
pub(crate) struct Shared {
    base: *mut u8,
    len: usize,
    _keep: Backing,
}

// SAFETY: see `Mapping`: shared fields are atomics, pixels are handed
// between sides by the triple buffer.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

/// A slot's descriptor, as last written by the producer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Desc {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) stride: u32,
    pub(crate) format: u32,
    pub(crate) seq: u64,
    pub(crate) present_ns: u64,
}

impl Shared {
    pub(crate) fn from_mapping(m: Mapping) -> Shared {
        Shared { base: m.ptr(), len: m.len(), _keep: Backing::Map(m) }
    }

    #[cfg(test)]
    pub(crate) fn heap(len: usize) -> Shared {
        let mut v = vec![0u64; len.div_ceil(8)];
        let base = v.as_mut_ptr() as *mut u8;
        Shared { base, len, _keep: Backing::Heap(v) }
    }

    fn a32(&self, off: usize) -> &AtomicU32 {
        debug_assert!(off + 4 <= self.len && off % 4 == 0);
        // SAFETY: in bounds and aligned (the mapping is page aligned).
        unsafe { &*(self.base.add(off) as *const AtomicU32) }
    }

    fn a64(&self, off: usize) -> &AtomicU64 {
        debug_assert!(off + 8 <= self.len && off % 8 == 0);
        // SAFETY: in bounds and aligned.
        unsafe { &*(self.base.add(off) as *const AtomicU64) }
    }

    /// Writes a fresh header (creator side, memory zeroed by the OS).
    pub(crate) fn init(&self, max_w: u32, max_h: u32) {
        let (slots_off, slot, total) = layout(max_w, max_h);
        assert!(total <= self.len, "surface too small");
        self.a32(O_VERSION).store(VERSION, Ordering::Relaxed);
        self.a32(O_SLOTS_OFF).store(slots_off as u32, Ordering::Relaxed);
        self.a32(O_SLOTS).store(SLOTS, Ordering::Relaxed);
        self.a64(O_SLOT_BYTES).store(slot as u64, Ordering::Relaxed);
        self.a64(O_TOTAL).store(total as u64, Ordering::Relaxed);
        self.a32(O_MAX_W).store(max_w, Ordering::Relaxed);
        self.a32(O_MAX_H).store(max_h, Ordering::Relaxed);
        self.a32(O_RING).store(RING, Ordering::Relaxed);
        // Producer draws into 0, 1 is in the middle (empty), consumer has 2.
        self.a32(O_BACK).store(0, Ordering::Relaxed);
        self.a32(O_MIDDLE).store(1, Ordering::Relaxed);
        self.a32(O_FRONT).store(2, Ordering::Relaxed);
        self.a32(O_ZOOM).store(1f32.to_bits(), Ordering::Relaxed);
        self.a32(O_MAGIC).store(MAGIC, Ordering::Release);
    }

    /// Checks the header of a surface opened from a handle.
    pub(crate) fn validate(&self) -> io::Result<()> {
        let bad = |m: &str| Err(io::Error::new(io::ErrorKind::InvalidData, format!("frame surface: {m}")));
        if self.len < PAGE + 2 * RING as usize {
            return bad("too small");
        }
        if self.a32(O_MAGIC).load(Ordering::Acquire) != MAGIC {
            return bad("bad magic");
        }
        if self.a32(O_VERSION).load(Ordering::Relaxed) != VERSION {
            return bad("unsupported version");
        }
        let (slots_off, slot, total) = layout(self.max_width(), self.max_height());
        let ok = self.a32(O_SLOTS).load(Ordering::Relaxed) == SLOTS
            && self.a32(O_RING).load(Ordering::Relaxed) == RING
            && self.a32(O_SLOTS_OFF).load(Ordering::Relaxed) as usize == slots_off
            && self.a64(O_SLOT_BYTES).load(Ordering::Relaxed) as usize == slot
            && self.a64(O_TOTAL).load(Ordering::Relaxed) as usize == total
            && total <= self.len;
        if ok {
            Ok(())
        } else {
            bad("inconsistent header")
        }
    }

    pub(crate) fn max_width(&self) -> u32 {
        self.a32(O_MAX_W).load(Ordering::Relaxed)
    }
    pub(crate) fn max_height(&self) -> u32 {
        self.a32(O_MAX_H).load(Ordering::Relaxed)
    }
    fn slot_bytes(&self) -> usize {
        self.a64(O_SLOT_BYTES).load(Ordering::Relaxed) as usize
    }

    /// Pointer to slot `i`'s pixels (`slot_bytes` long).
    pub(crate) fn slot_ptr(&self, i: u32) -> *mut u8 {
        let off = self.a32(O_SLOTS_OFF).load(Ordering::Relaxed) as usize + (i % SLOTS) as usize * self.slot_bytes();
        debug_assert!(off + self.slot_bytes() <= self.len);
        // SAFETY: validated layout: the slot lies inside the mapping.
        unsafe { self.base.add(off) }
    }

    pub(crate) fn slot_len(&self) -> usize {
        self.slot_bytes()
    }

    pub(crate) fn seq(&self) -> u64 {
        self.a64(O_SEQ).load(Ordering::Acquire)
    }

    pub(crate) fn desc(&self, i: u32) -> Desc {
        let o = O_DESC + (i % SLOTS) as usize * DESC;
        Desc {
            width: self.a32(o).load(Ordering::Relaxed),
            height: self.a32(o + 4).load(Ordering::Relaxed),
            stride: self.a32(o + 8).load(Ordering::Relaxed),
            format: self.a32(o + 12).load(Ordering::Relaxed),
            seq: self.a64(o + 16).load(Ordering::Relaxed),
            present_ns: self.a64(o + 24).load(Ordering::Relaxed),
        }
    }

    fn set_desc(&self, i: u32, d: &Desc) {
        let o = O_DESC + (i % SLOTS) as usize * DESC;
        self.a32(o).store(d.width, Ordering::Relaxed);
        self.a32(o + 4).store(d.height, Ordering::Relaxed);
        self.a32(o + 8).store(d.stride, Ordering::Relaxed);
        self.a32(o + 12).store(d.format, Ordering::Relaxed);
        self.a64(o + 16).store(d.seq, Ordering::Relaxed);
        self.a64(o + 24).store(d.present_ns, Ordering::Relaxed);
    }

    /// The slot the producer draws into (restored by a producer that
    /// opens an existing surface).
    pub(crate) fn producer_back(&self) -> u32 {
        self.a32(O_BACK).load(Ordering::Relaxed) % SLOTS
    }
    pub(crate) fn consumer_front(&self) -> u32 {
        self.a32(O_FRONT).load(Ordering::Relaxed) % SLOTS
    }

    /// Producer: publishes slot `back` (descriptor `d`, seq filled in)
    /// and returns (the slot to draw the next frame into, the new seq).
    pub(crate) fn present(&self, back: u32, mut d: Desc) -> (u32, u64) {
        let seq = self.a64(O_SEQ).load(Ordering::Relaxed) + 1;
        d.seq = seq;
        self.set_desc(back, &d);
        let prev = self.a32(O_MIDDLE).swap(back | FRESH, Ordering::AcqRel);
        let next = prev & 3;
        self.a32(O_BACK).store(next, Ordering::Relaxed);
        self.a64(O_SEQ).store(seq, Ordering::Release);
        (next, seq)
    }

    /// Consumer: takes the newest presented slot if there is one not taken
    /// yet, giving `front` back. Returns the new front slot.
    pub(crate) fn acquire(&self, front: u32) -> Option<u32> {
        if self.a32(O_MIDDLE).load(Ordering::Acquire) & FRESH == 0 {
            return None;
        }
        let prev = self.a32(O_MIDDLE).swap(front % SLOTS, Ordering::AcqRel);
        let got = prev & 3;
        self.a32(O_FRONT).store(got, Ordering::Relaxed);
        Some(got)
    }

    /// A frame is waiting in the middle slot.
    pub(crate) fn fresh(&self) -> bool {
        self.a32(O_MIDDLE).load(Ordering::Acquire) & FRESH != 0
    }

    pub(crate) fn set_closed(&self, bit: u32) {
        self.a32(O_CLOSED).fetch_or(bit, Ordering::AcqRel);
    }
    pub(crate) fn closed(&self) -> u32 {
        self.a32(O_CLOSED).load(Ordering::Acquire)
    }

    pub(crate) fn set_pid(&self, producer: bool) {
        let off = if producer { O_PRODUCER_PID } else { O_CONSUMER_PID };
        self.a32(off).store(std::process::id(), Ordering::Relaxed);
    }

    pub(crate) fn set_want(&self, w: u32, h: u32, scale: f32) {
        self.a32(O_WANT_W).store(w, Ordering::Relaxed);
        self.a32(O_WANT_H).store(h, Ordering::Relaxed);
        self.a32(O_WANT_SCALE).store(scale.to_bits(), Ordering::Release);
    }
    pub(crate) fn want(&self) -> Option<(u32, u32, f32)> {
        let s = f32::from_bits(self.a32(O_WANT_SCALE).load(Ordering::Acquire));
        let (w, h) = (self.a32(O_WANT_W).load(Ordering::Relaxed), self.a32(O_WANT_H).load(Ordering::Relaxed));
        (w > 0 && h > 0).then_some((w, h, if s.is_finite() && s > 0.0 { s } else { 1.0 }))
    }

    pub(crate) fn set_zoom(&self, z: f32) {
        self.a32(O_ZOOM).store(z.to_bits(), Ordering::Relaxed);
    }
    pub(crate) fn zoom(&self) -> f32 {
        f32::from_bits(self.a32(O_ZOOM).load(Ordering::Relaxed))
    }

    pub(crate) fn set_drawn(&self, seq: u64) {
        self.a64(O_DRAWN).store(seq, Ordering::Relaxed);
    }
    pub(crate) fn drawn(&self) -> u64 {
        self.a64(O_DRAWN).load(Ordering::Relaxed)
    }

    fn ring_at(&self, ctl: usize, data: usize) -> Ring<'_> {
        Ring {
            // SAFETY: the ring bytes lie inside the validated mapping.
            data: unsafe { self.base.add(data) },
            size: RING,
            head: self.a32(ctl),
            tail: self.a32(ctl + 4),
            dropped: self.a32(ctl + 8),
        }
    }

    /// Consumer → producer messages (input events).
    pub(crate) fn input_ring(&self) -> Ring<'_> {
        self.ring_at(O_IN_RING, PAGE)
    }

    /// Producer → consumer messages (content size, clipboard, app data).
    pub(crate) fn output_ring(&self) -> Ring<'_> {
        self.ring_at(O_OUT_RING, PAGE + RING as usize)
    }
}

/// Nanoseconds since the Unix epoch (both processes read the same clock).
pub(crate) fn now_ns() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(w: u32, h: u32) -> Shared {
        let (_, _, total) = layout(w, h);
        let s = Shared::heap(total);
        s.init(w, h);
        s
    }

    #[test]
    fn header_round_trips_and_rejects_garbage() {
        let s = surface(64, 32);
        s.validate().unwrap();
        assert_eq!((s.max_width(), s.max_height()), (64, 32));
        assert_eq!(s.slot_len(), 8192, "64*32*4 rounded to pages");
        assert_eq!(s.zoom(), 1.0);
        let (off, slot, total) = layout(64, 32);
        assert_eq!(off % 4096, 0);
        assert_eq!(total, off + 3 * slot);
        // Slots do not overlap and stay in bounds.
        assert_eq!(s.slot_ptr(1) as usize - s.slot_ptr(0) as usize, slot);
        assert!(s.slot_ptr(2) as usize + slot <= s.base as usize + total);
        let z = Shared::heap(total);
        assert!(z.validate().is_err(), "zeroed memory");
        s.a32(O_VERSION).store(2, Ordering::Relaxed);
        assert!(s.validate().is_err(), "newer version");
    }

    fn desc(w: u32) -> Desc {
        Desc { width: w, height: 1, stride: w * 4, format: Format::Rgba.code(), seq: 0, present_ns: now_ns() }
    }

    #[test]
    fn triple_buffer_hands_the_newest_frame_and_never_shares_a_slot() {
        let s = surface(16, 16);
        let (mut back, mut front) = (s.producer_back(), s.consumer_front());
        assert_eq!(s.acquire(front), None, "nothing presented yet");
        // Three frames before the consumer looks: it gets the last one.
        let mut seq = 0;
        for w in 1..=3 {
            assert_ne!(back, front);
            (back, seq) = s.present(back, desc(w));
            assert_ne!(back, front, "producer never draws into the shown slot");
        }
        assert_eq!(seq, 3);
        front = s.acquire(front).expect("fresh frame");
        assert_eq!(s.desc(front).width, 3, "newest");
        assert_eq!(s.desc(front).seq, 3);
        assert_eq!(s.acquire(front), None, "taken once");
        assert!(!s.fresh());
        // Interleaved: every slot index stays distinct between the sides.
        for w in 4..100 {
            (back, _) = s.present(back, desc(w));
            assert_ne!(back, front);
            if w % 3 == 0 {
                front = s.acquire(front).unwrap();
                assert_eq!(s.desc(front).width, w);
                assert_ne!(back, front);
            }
        }
        assert_eq!(s.seq(), 99);
    }

    #[test]
    fn triple_buffer_under_threads_delivers_increasing_frames() {
        let s = std::sync::Arc::new(surface(8, 8));
        let p = s.clone();
        let t = std::thread::spawn(move || {
            let mut back = p.producer_back();
            for i in 1..=20_000u32 {
                // SAFETY: the producer owns `back` until it presents it.
                unsafe { std::ptr::write_bytes(p.slot_ptr(back), (i % 251) as u8, 256) };
                (back, _) = p.present(back, desc(i));
            }
        });
        let (mut front, mut last) = (s.consumer_front(), 0u64);
        while last < 20_000 {
            if let Some(f) = s.acquire(front) {
                front = f;
                let d = s.desc(f);
                assert!(d.seq > last, "frames only move forward");
                // The whole slot holds one frame (no tearing).
                // SAFETY: the consumer owns `front` until the next acquire.
                let px = unsafe { std::slice::from_raw_parts(s.slot_ptr(f), 256) };
                assert!(px.iter().all(|&b| b == px[0]), "torn frame at seq {}", d.seq);
                assert_eq!(px[0] as u64, d.seq % 251);
                last = d.seq;
            }
        }
        t.join().unwrap();
    }

    #[test]
    fn requested_size_and_flags() {
        let s = surface(8, 8);
        assert_eq!(s.want(), None);
        s.set_want(640, 400, 2.0);
        assert_eq!(s.want(), Some((640, 400, 2.0)));
        s.set_closed(CLOSED_PRODUCER);
        assert_eq!(s.closed(), CLOSED_PRODUCER);
        assert_eq!(Format::from_code(Format::Bgra.code() | OPAQUE), Some(Format::Bgra));
        assert_eq!(Format::from_code(9), None);
    }
}
