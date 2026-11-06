//! Giving freed heap memory back to the OS.
//!
//! glibc serves allocations above a threshold with their own mappings and
//! unmaps them on free, but it raises that threshold to the size of each
//! large block freed (up to 32 MiB). After a worker decoded a
//! 12-megapixel photo (a 48 MB buffer, freed after downscaling), the next
//! multi-MiB buffers come from the arenas and stay in the process: measured
//! +21 MB PSS for 24 thumbnails of one 2000 × 1500 WebP, +0.3 MB with the
//! thresholds fixed and the heap trimmed after large decodes
//! (docs/MEDIA.md). musl, macOS and Windows return large blocks on their
//! own; there these functions do nothing.

/// Fixes glibc's mmap threshold at 4 MiB (it stops growing), so buffers of
/// 4 MiB and more (decoded frames) go back to the OS when freed. Idempotent;
/// `media` calls it before its first decode. Returns true when it applied.
pub fn return_large_allocations() -> bool {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            // SAFETY: mallopt only changes allocator tuning; thread-safe.
            unsafe {
                libc::mallopt(libc::M_MMAP_THRESHOLD, 4 << 20);
                // The dynamic rule also raised the trim threshold (to twice
                // the largest block freed); put it back to glibc's default.
                libc::mallopt(libc::M_TRIM_THRESHOLD, 128 << 10);
            }
        });
        true
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        false
    }
}

/// Returns free heap pages to the OS where the allocator keeps them
/// (glibc: `malloc_trim(0)`, all arenas). Returns true when it ran.
pub fn trim_heap() -> bool {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: malloc_trim only walks and releases the allocator's free
        // lists; it is thread-safe and takes no pointers.
        unsafe { libc::malloc_trim(0) };
        true
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        false
    }
}

/// Diagnostic allocator accounting, not resident/PSS memory. Counters
/// cover glibc allocations across the process, not individual owners.
#[derive(Debug, Clone, Copy)]
pub struct AllocatorSnapshot {
    pub arena_bytes: usize,
    pub used_arena_bytes: usize,
    pub free_arena_bytes: usize,
    pub mapped_bytes: usize,
}

/// Returns glibc's size_t counters when available, otherwise None. Resolve
/// dynamically to preserve compatibility with glibc older than 2.33.
/// Call after allocator setup, without concurrent allocator reconfiguration.
pub fn allocator_snapshot() -> Option<AllocatorSnapshot> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        let symbol = libc::dlsym(libc::RTLD_DEFAULT, b"mallinfo2\0".as_ptr().cast());
        if symbol.is_null() {
            return None;
        }
        let mut provider: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(symbol, &mut provider) == 0 || provider.dli_fname.is_null() {
            return None;
        }
        let path = std::ffi::CStr::from_ptr(provider.dli_fname).to_bytes();
        let name = path.rsplit(|b| *b == b'/').next()?;
        if !name.starts_with(b"libc.so") && !name.starts_with(b"libc-") {
            // Do not interpret an interposed provider as glibc accounting.
            return None;
        }
        // SAFETY: glibc's mallinfo2 symbol has precisely this C signature;
        // it remains loaded for the process lifetime. No ownership passes.
        let query: unsafe extern "C" fn() -> libc::mallinfo2 = std::mem::transmute(symbol);
        let info = query();
        Some(AllocatorSnapshot {
            arena_bytes: info.arena,
            used_arena_bytes: info.uordblks,
            free_arena_bytes: info.fordblks,
            mapped_bytes: info.hblkhd,
        })
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn optional_allocator_snapshot_has_consistent_arena_accounting() {
        if let Some(s) = super::allocator_snapshot() {
            assert_eq!(
                s.used_arena_bytes.checked_add(s.free_arena_bytes),
                Some(s.arena_bytes)
            );
        }
    }
}
