//! Asynchronous loading through the task pool and the decoded-image LRU
//! cache, keyed by (source, target size), bounded in bytes. UI thread only.

use super::{decode, Decoded, Error, Limits, Target};
use crate::task::{Outcome, Task};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Where an image comes from. Clones are cheap.
#[derive(Clone, Debug)]
pub enum Source {
    /// A file, read on a worker (bounded by [`Limits::max_input_bytes`]).
    File(PathBuf),
    /// Bytes already in memory (a download, an attachment); `key`
    /// identifies them in the cache (an id or a content hash).
    Bytes { key: Arc<str>, data: Arc<[u8]> },
}

impl Source {
    pub fn file(path: impl Into<PathBuf>) -> Source {
        Source::File(path.into())
    }

    pub fn bytes(key: &str, data: impl Into<Arc<[u8]>>) -> Source {
        Source::Bytes { key: key.into(), data: data.into() }
    }

    /// The cache identity: `file:<path>` or `bytes:<key>`.
    pub fn key(&self) -> String {
        match self {
            Source::File(p) => format!("file:{}", p.display()),
            Source::Bytes { key, .. } => format!("bytes:{key}"),
        }
    }
}

impl PartialEq for Source {
    fn eq(&self, other: &Source) -> bool {
        self.key() == other.key()
    }
}

type Key = (String, Target);
type Waiter = Box<dyn FnOnce(Result<Arc<Decoded>, Error>)>;

struct Entry {
    img: Arc<Decoded>,
    bytes: usize,
    used: u64,
}

#[derive(Default)]
struct Cache {
    map: HashMap<Key, Entry>,
    failed: HashMap<Key, Error>,
    pending: HashMap<Key, Vec<Waiter>>,
    bytes: usize,
    tick: u64,
    budget: Option<usize>,
    hits: u64,
    misses: u64,
    decodes: u64,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

const DEFAULT_BUDGET: usize = 64 << 20;

/// What the image cache holds ([`cache_stats`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: usize,
    pub budget: usize,
    pub hits: u64,
    pub misses: u64,
    /// Decodes finished (successful or not).
    pub decodes: u64,
    /// Loads still running.
    pub pending: usize,
}

/// Bytes of decoded pixels the cache keeps (default 64 MiB). Images on
/// screen stay alive while shown even after they leave the cache.
pub fn set_cache_budget(bytes: usize) {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.budget = Some(bytes);
        evict(&mut c, 0);
    });
}

pub fn cache_stats() -> CacheStats {
    CACHE.with(|c| {
        let c = c.borrow();
        CacheStats {
            entries: c.map.len(),
            bytes: c.bytes,
            budget: c.budget.unwrap_or(DEFAULT_BUDGET),
            hits: c.hits,
            misses: c.misses,
            decodes: c.decodes,
            pending: c.pending.len(),
        }
    })
}

/// Drops every cached size (and remembered failure) of `source`: the
/// file changed, or the bytes behind a key did.
pub fn forget(source: &Source) {
    let k = source.key();
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let gone: Vec<Key> = c.map.keys().filter(|(s, _)| *s == k).cloned().collect();
        for key in gone {
            if let Some(e) = c.map.remove(&key) {
                c.bytes -= e.bytes;
            }
        }
        c.failed.retain(|(s, _), _| *s != k);
    });
}

fn evict(c: &mut Cache, room: usize) {
    let budget = c.budget.unwrap_or(DEFAULT_BUDGET);
    while c.bytes + room > budget && !c.map.is_empty() {
        let Some(k) = c.map.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone()) else { break };
        if let Some(e) = c.map.remove(&k) {
            c.bytes -= e.bytes;
        }
    }
}

/// The cached decode of `source` at `target`, if any (no loading).
pub fn peek(source: &Source, target: Target) -> Option<Arc<Decoded>> {
    let key = (source.key(), target.normalized());
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.tick += 1;
        let t = c.tick;
        c.map.get_mut(&key).map(|e| {
            e.used = t;
            e.img.clone()
        })
    })
}

/// The largest cached decode of `source` (a thumbnail to show while a
/// bigger size loads).
pub(crate) fn largest(source: &Source) -> Option<Arc<Decoded>> {
    let k = source.key();
    CACHE.with(|c| {
        let c = c.borrow();
        c.map.iter().filter(|((s, _), _)| *s == k).map(|(_, e)| e.img.clone()).max_by_key(|d| d.size().0 as u64 * d.size().1 as u64)
    })
}

/// Loads `source` decoded for `target` with the default [`Limits`]; `done`
/// runs on the UI thread (right away when cached). Concurrent loads of
/// the same (source, target) share one decode.
pub fn load(source: &Source, target: Target, done: impl FnOnce(Result<Arc<Decoded>, Error>) + 'static) {
    load_with(source, target, Limits::default(), done)
}

/// [`load`] with explicit limits.
pub fn load_with(source: &Source, target: Target, limits: Limits, done: impl FnOnce(Result<Arc<Decoded>, Error>) + 'static) {
    let target = target.normalized();
    let key = (source.key(), target);
    enum Next {
        Ready(Result<Arc<Decoded>, Error>),
        Wait,
        Start,
    }
    let mut done: Option<Waiter> = Some(Box::new(done));
    let next = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.tick += 1;
        let t = c.tick;
        if let Some(e) = c.map.get_mut(&key) {
            e.used = t;
            let img = e.img.clone();
            c.hits += 1;
            return Next::Ready(Ok(img));
        }
        if let Some(e) = c.failed.get(&key) {
            return Next::Ready(Err(e.clone()));
        }
        c.misses += 1;
        if let Some(w) = c.pending.get_mut(&key) {
            w.push(done.take().expect("waiter"));
            return Next::Wait;
        }
        c.pending.insert(key.clone(), vec![done.take().expect("waiter")]);
        Next::Start
    });
    match next {
        Next::Ready(r) => {
            if let Some(f) = done.take() {
                f(r)
            }
        }
        Next::Wait => {}
        Next::Start => start(source.clone(), key, target, limits),
    }
}

fn start(source: Source, key: Key, target: Target, limits: Limits) {
    // Full-size frames are freed right after downscaling: make sure the
    // allocator gives them back (see platform::memory).
    crate::platform::memory::return_large_allocations();
    let k2 = key.clone();
    let task = Task::new(move |ctx| {
        let bytes: Arc<[u8]> = match &source {
            Source::File(p) => match read_bounded(p, limits.max_input_bytes) {
                Ok(b) => b.into(),
                Err(e) => return Ok(Err(e)),
            },
            Source::Bytes { data, .. } => data.clone(),
        };
        let r = decode(&bytes, target, &limits, &|| ctx.is_cancelled());
        drop(bytes);
        // The decoders' smaller working buffers stay in this worker's
        // arena: release them after a large file.
        if r.as_ref().map_or(true, |d| d.source_size.0 as u64 * d.source_size.1 as u64 >= 1 << 20) {
            crate::platform::memory::trim_heap();
        }
        Ok(r)
    })
    .on_done(move |outcome| {
        let r: Result<Arc<Decoded>, Error> = match outcome {
            Outcome::Done(Ok(d)) => Ok(Arc::new(d)),
            Outcome::Done(Err(e)) => Err(e),
            Outcome::Failed(e) => Err(Error::Corrupt(e)),
            Outcome::Cancelled => Err(Error::Cancelled),
        };
        finish(k2, r);
    });
    if let Err(e) = task.try_start() {
        finish(key, Err(Error::Io(e)));
    }
}

fn finish(key: Key, r: Result<Arc<Decoded>, Error>) {
    let waiters = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.decodes += 1;
        match &r {
            Ok(img) => {
                let bytes = img.bytes() + 128;
                if bytes <= c.budget.unwrap_or(DEFAULT_BUDGET) {
                    evict(&mut c, bytes);
                    c.tick += 1;
                    let used = c.tick;
                    c.bytes += bytes;
                    c.map.insert(key.clone(), Entry { img: img.clone(), bytes, used });
                }
            }
            Err(Error::Cancelled) | Err(Error::Io(_)) => {}
            Err(e) => {
                // Bounded: a page of broken links does not grow it forever.
                if c.failed.len() > 512 {
                    c.failed.clear();
                }
                c.failed.insert(key.clone(), e.clone());
            }
        }
        c.pending.remove(&key).unwrap_or_default()
    });
    for w in waiters {
        w(r.clone());
    }
}

/// Reads at most `max` bytes of a regular file.
pub(crate) fn read_bounded(path: &Path, max: usize) -> Result<Vec<u8>, Error> {
    let f = std::fs::File::open(path).map_err(|e| Error::Io(e.to_string()))?;
    let meta = f.metadata().map_err(|e| Error::Io(e.to_string()))?;
    if !meta.is_file() {
        return Err(Error::Io("not a regular file".into()));
    }
    if meta.len() > max as u64 {
        return Err(Error::TooLarge { bytes: meta.len().min(usize::MAX as u64) as usize, limit: max });
    }
    let mut out = Vec::new();
    out.try_reserve_exact(meta.len() as usize).map_err(|_| Error::OutOfMemory { bytes: meta.len() as usize })?;
    f.take(max as u64 + 1).read_to_end(&mut out).map_err(|e| Error::Io(e.to_string()))?;
    if out.len() > max {
        return Err(Error::TooLarge { bytes: out.len(), limit: max });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Fit;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    fn png(w: u32, h: u32) -> Vec<u8> {
        crate::png::encode(w, h, 4, &vec![128; (w * h * 4) as usize])
    }

    fn pump_until(f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !f() && Instant::now() < end {
            crate::task::drain();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn loads_share_one_decode_cache_by_size_and_evict_by_bytes() {
        set_cache_budget(64 << 20);
        let src = Source::bytes("cache-test-a", png(100, 100));
        let t = Target::new(50, 50, Fit::Contain);
        let got = Rc::new(Cell::new(0));
        let before = cache_stats();
        for _ in 0..3 {
            let g = got.clone();
            load(&src, t, move |r| {
                assert_eq!(r.unwrap().size(), (50, 50));
                g.set(g.get() + 1);
            });
        }
        pump_until(|| got.get() == 3);
        assert_eq!(got.get(), 3);
        let s = cache_stats();
        assert_eq!(s.decodes - before.decodes, 1, "concurrent loads share one decode");
        assert!(peek(&src, t).is_some());
        assert!(peek(&src, Target::new(50, 50, Fit::Letterbox)).is_some(), "letterbox shares contain");
        assert!(peek(&src, Target::new(80, 80, Fit::Contain)).is_none(), "keyed by size");
        // Cached: answered synchronously.
        let sync = Rc::new(Cell::new(false));
        let s2 = sync.clone();
        load(&src, t, move |_| s2.set(true));
        assert!(sync.get());
        // Errors are values, and remembered until forgotten.
        let bad = Source::bytes("cache-test-bad", b"not an image".to_vec());
        let err = Rc::new(RefCell::new(None));
        let e2 = err.clone();
        load(&bad, t, move |r| *e2.borrow_mut() = Some(r.err()));
        pump_until(|| err.borrow().is_some());
        assert_eq!(err.borrow().clone().flatten(), Some(Error::Unsupported));
        // Byte budget: a second image pushes the least recently used out.
        let one = 50 * 50 * 4 + 128;
        set_cache_budget(one + 10);
        let src_b = Source::bytes("cache-test-b", png(100, 100));
        let done = Rc::new(Cell::new(false));
        let d2 = done.clone();
        load(&src_b, t, move |_| d2.set(true));
        pump_until(|| done.get());
        assert!(peek(&src_b, t).is_some() && peek(&src, t).is_none(), "LRU evicted");
        assert!(cache_stats().bytes <= one + 10);
        forget(&src_b);
        assert!(peek(&src_b, t).is_none());
        set_cache_budget(64 << 20);
    }

    #[test]
    fn files_are_read_with_a_bound() {
        let dir = std::env::temp_dir().join(format!("fltk-omarchy-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.png");
        std::fs::write(&f, png(4, 4)).unwrap();
        assert!(read_bounded(&f, 1 << 20).is_ok());
        assert!(matches!(read_bounded(&f, 10), Err(Error::TooLarge { .. })));
        assert!(matches!(read_bounded(&dir, 1 << 20), Err(Error::Io(_))), "a directory is not an image");
        assert!(matches!(read_bounded(&dir.join("missing.png"), 1 << 20), Err(Error::Io(_))));
        // Only the file this test wrote is removed (no tree deletion).
        std::fs::remove_file(&f).unwrap();
        let _ = std::fs::remove_dir(&dir);
    }
}
