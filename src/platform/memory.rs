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
