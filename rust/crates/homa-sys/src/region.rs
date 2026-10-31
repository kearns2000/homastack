//! Anonymous memory mapping used as a Homa receive-buffer pool.
//!
//! Homa requires the region passed to `SO_HOMA_RCVBUF` to be page aligned and
//! writable. `mmap(MAP_PRIVATE | MAP_ANONYMOUS)` satisfies both and matches the
//! upstream examples (`util/demo_server1.cc`).

use core::ptr::NonNull;
use std::io;

/// An owned, page-aligned anonymous mapping.
///
/// The mapping is released with `munmap` on drop. The type is `Send + Sync`:
/// it only hands out raw pointers, and all access through them is the
/// responsibility of the caller (the kernel writes into the region
/// concurrently once it has been registered with a Homa socket).
#[derive(Debug)]
pub struct MappedRegion {
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: `MappedRegion` owns a raw mapping and exposes only raw pointers; it
// has no interior state that is unsafe to share or move between threads.
unsafe impl Send for MappedRegion {}
// SAFETY: see above.
unsafe impl Sync for MappedRegion {}

impl MappedRegion {
    /// Map `len` bytes of zeroed anonymous memory, rounded up to the system
    /// page size.
    ///
    /// # Errors
    /// Returns the `mmap` error (typically `ENOMEM`) or `EINVAL` for `len == 0`.
    pub fn anonymous(len: usize) -> io::Result<Self> {
        if len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "mapping length must be > 0",
            ));
        }
        let page = page_size();
        let len = len.checked_add(page - 1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "mapping length overflow")
        })? / page
            * page;

        // SAFETY: anonymous private mapping with a null hint and a non-zero,
        // page-rounded length; no file descriptor is involved. `mmap` has no
        // preconditions on memory we own.
        let raw = unsafe {
            libc::mmap(
                core::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let ptr = NonNull::new(raw.cast::<u8>())
            .ok_or_else(|| io::Error::other("mmap returned a null mapping"))?;
        Ok(Self { ptr, len })
    }

    /// Base address of the mapping (page aligned).
    #[must_use]
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// Length of the mapping in bytes (a multiple of the page size).
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Always `false`; a region can never be empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Drop for MappedRegion {
    fn drop(&mut self) {
        // SAFETY: `ptr`/`len` describe exactly the mapping created in
        // `anonymous`, which has not been unmapped before (we own it).
        // Callers of `sys::set_rcvbuf_region` guarantee the kernel no longer
        // references the region (socket closed) before this drop runs.
        let rc = unsafe { libc::munmap(self.ptr.as_ptr().cast(), self.len) };
        debug_assert_eq!(rc, 0, "munmap failed: {}", io::Error::last_os_error());
    }
}

/// System page size in bytes.
#[must_use]
pub fn page_size() -> usize {
    // SAFETY: sysconf is always safe to call; a negative result means the
    // query is unsupported, in which case we fall back to 4 KiB.
    let v = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    usize::try_from(v).unwrap_or(4096)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_and_rounds_up_to_page_size() {
        let page = page_size();
        let region = MappedRegion::anonymous(page + 1).expect("mmap");
        assert_eq!(region.len(), 2 * page);
        assert_eq!(region.as_ptr() as usize % page, 0);
        assert!(!region.is_empty());
    }

    #[test]
    fn region_is_zeroed_and_writable() {
        let region = MappedRegion::anonymous(8192).expect("mmap");
        // SAFETY: the region is exclusively owned here and not yet registered
        // with any socket; `len()` bytes are valid for reads and writes.
        let bytes = unsafe { core::slice::from_raw_parts_mut(region.as_ptr(), region.len()) };
        assert!(bytes.iter().all(|&b| b == 0));
        bytes[4097] = 0xAB;
        assert_eq!(bytes[4097], 0xAB);
    }

    #[test]
    fn zero_length_is_rejected() {
        assert!(MappedRegion::anonymous(0).is_err());
    }
}
