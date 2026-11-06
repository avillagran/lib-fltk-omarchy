//! Single-producer, single-consumer byte ring in shared memory, carrying
//! newline-terminated JSON messages. `head` (bytes ever written) belongs
//! to the writer, `tail` (bytes ever read) to the reader; both wrap at
//! 2³². A message is copied in full before `head` moves (Release), so the
//! reader never sees half a message. A message that does not fit is
//! dropped and counted: the writer never blocks.

use std::sync::atomic::{AtomicU32, Ordering};

/// One ring: `size` (a power of two) bytes at `data`, with its counters.
pub(crate) struct Ring<'a> {
    pub(crate) data: *mut u8,
    pub(crate) size: u32,
    pub(crate) head: &'a AtomicU32,
    pub(crate) tail: &'a AtomicU32,
    pub(crate) dropped: &'a AtomicU32,
}

impl Ring<'_> {
    /// Bytes waiting to be read.
    pub(crate) fn pending(&self) -> u32 {
        self.head.load(Ordering::Acquire).wrapping_sub(self.tail.load(Ordering::Acquire))
    }

    /// Writer: appends `msg` (one whole message). False when it did not
    /// fit (counted in `dropped`).
    pub(crate) fn push(&self, msg: &[u8]) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.tail.load(Ordering::Acquire);
        let used = head.wrapping_sub(tail);
        if used > self.size || msg.len() as u64 > u64::from(self.size - used) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let mask = self.size - 1;
        let start = (head & mask) as usize;
        let first = msg.len().min(self.size as usize - start);
        // SAFETY: `start + first <= size` and `msg.len() - first <= start`
        // (the message fits in the free space), inside the ring's bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(msg.as_ptr(), self.data.add(start), first);
            std::ptr::copy_nonoverlapping(msg.as_ptr().add(first), self.data, msg.len() - first);
        }
        self.head.store(head.wrapping_add(msg.len() as u32), Ordering::Release);
        true
    }

    /// Reader: moves every pending byte to the end of `out`.
    pub(crate) fn drain_into(&self, out: &mut Vec<u8>) -> usize {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        let n = head.wrapping_sub(tail);
        if n == 0 || n > self.size {
            if n > self.size {
                // A corrupt writer: resynchronize instead of reading garbage.
                self.tail.store(head, Ordering::Release);
            }
            return 0;
        }
        let mask = self.size - 1;
        let start = (tail & mask) as usize;
        let n = n as usize;
        let first = n.min(self.size as usize - start);
        out.reserve(n);
        // SAFETY: both ranges lie inside the ring's `size` bytes.
        unsafe {
            out.extend_from_slice(std::slice::from_raw_parts(self.data.add(start), first));
            out.extend_from_slice(std::slice::from_raw_parts(self.data, n - first));
        }
        self.tail.store(tail.wrapping_add(n as u32), Ordering::Release);
        n
    }
}

/// Splits complete lines off the front of `buf` (keeps a trailing
/// partial line for the next call).
pub(crate) fn take_lines(buf: &mut Vec<u8>) -> Vec<String> {
    let Some(end) = buf.iter().rposition(|&b| b == b'\n') else { return Vec::new() };
    let rest = buf.split_off(end + 1);
    let lines = std::mem::replace(buf, rest);
    lines
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| String::from_utf8_lossy(l).into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ring memory and counters, shared by reference between threads
    /// (as the mapping is between processes).
    struct Fixture {
        data: *mut u8,
        size: usize,
        head: AtomicU32,
        tail: AtomicU32,
        dropped: AtomicU32,
    }

    // SAFETY: test fixture; the ring protocol splits the bytes between
    // one writer and one reader.
    unsafe impl Sync for Fixture {}

    impl Fixture {
        fn new(size: usize, start: u32) -> Fixture {
            let data = Box::into_raw(vec![0u8; size].into_boxed_slice()) as *mut u8;
            Fixture { data, size, head: AtomicU32::new(start), tail: AtomicU32::new(start), dropped: AtomicU32::new(0) }
        }
        fn ring(&self) -> Ring<'_> {
            Ring { data: self.data, size: self.size as u32, head: &self.head, tail: &self.tail, dropped: &self.dropped }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            // SAFETY: allocated in `new` with this length.
            drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(self.data, self.size)) });
        }
    }

    #[test]
    fn messages_round_trip_across_the_wrap_and_the_counter_wrap() {
        // Counters start just below 2^32: they wrap during the test.
        let f = Fixture::new(64, u32::MAX - 40);
        let r = f.ring();
        let mut out = Vec::new();
        for i in 0..200 {
            let m = format!("{{\"n\":{i}}}\n");
            assert!(r.push(m.as_bytes()), "push {i}");
            out.clear();
            assert_eq!(r.drain_into(&mut out), m.len());
            assert_eq!(out, m.as_bytes());
        }
        assert_eq!(r.pending(), 0);
    }

    #[test]
    fn full_ring_drops_whole_messages_only() {
        let f = Fixture::new(16, 0);
        let r = f.ring();
        assert!(r.push(b"0123456789\n"));
        assert!(!r.push(b"abcdef\n"), "does not fit: 11 + 7 > 16");
        assert_eq!(r.dropped.load(Ordering::Relaxed), 1);
        assert!(r.push(b"abcd\n"), "exactly fills");
        let mut out = Vec::new();
        r.drain_into(&mut out);
        assert_eq!(out, b"0123456789\nabcd\n");
        assert!(!r.push(&[b'x'; 17]), "larger than the ring");
    }

    #[test]
    fn lines_keep_a_partial_tail() {
        let mut b = b"{\"a\":1}\n{\"b\":2}\n{\"c\"".to_vec();
        assert_eq!(take_lines(&mut b), vec!["{\"a\":1}".to_string(), "{\"b\":2}".to_string()]);
        assert_eq!(b, b"{\"c\"");
        b.extend_from_slice(b":3}\n");
        assert_eq!(take_lines(&mut b), vec!["{\"c\":3}".to_string()]);
        assert!(b.is_empty());
    }

    #[test]
    fn concurrent_writer_and_reader_see_every_message_in_order() {
        let f = Fixture::new(256, 0);
        std::thread::scope(|sc| {
            sc.spawn(|| {
                let r = f.ring();
                let mut i = 0;
                while i < 5000 {
                    if r.push(format!("{i}\n").as_bytes()) {
                        i += 1;
                    } else {
                        std::thread::yield_now();
                    }
                }
            });
            let r = f.ring();
            let (mut buf, mut next) = (Vec::new(), 0);
            while next < 5000 {
                r.drain_into(&mut buf);
                for l in take_lines(&mut buf) {
                    assert_eq!(l, next.to_string());
                    next += 1;
                }
            }
        });
    }
}
