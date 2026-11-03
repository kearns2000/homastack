//! Receive-buffer pool management.
//!
//! Homa does not copy into a caller-supplied buffer on `recvmsg`. Instead the
//! application registers one large region once (`SO_HOMA_RCVBUF`); the kernel
//! writes each incoming message into one or more 64 KiB **bpages** inside that
//! region and returns their offsets. The application owns those bpages until
//! it hands them back (via a later `recvmsg` or `HOMAIOCRELEASE`), after which
//! the kernel may reuse them.
//!
//! This module provides:
//!
//! * [`ReceiveBuffer`] - owns the mapped region for one socket.
//! * [`FragmentLayout`] - validated description of where a message lives,
//!   computed from the kernel's `homa_recvmsg_args` without touching memory.
//! * [`ReceiveBuffer::copy_out`] - the single place that reads the region.
//! * [`BpageQueue`] - bpages owned by the application awaiting return.
//!
//! The layout logic is pure and unit-tested on every platform. The region
//! itself is an anonymous `mmap`, so [`ReceiveBuffer`] and the copy path exist
//! only on Unix, where tests write synthetic messages the way the kernel would.

use std::sync::Mutex;

use homa_sys::{HOMA_BPAGE_SIZE, HOMA_MAX_BPAGES, HOMA_MAX_MESSAGE_LENGTH, homa_recvmsg_args};

use crate::error::{HomaError, Result};

/// Default receive-region size: 1024 bpages = 64 MiB.
///
/// The kernel's minimum is `nr_cpu_ids × 64 KiB + 2 × HOMA_MAX_MESSAGE_LENGTH`
/// (about 2 MiB + 64 KiB per possible CPU; `nr_cpu_ids` can exceed the number of
/// online CPUs). 64 MiB comfortably covers typical hosts and matches the
/// upstream examples' `1000 * HOMA_BPAGE_SIZE`.
pub const DEFAULT_REGION_BYTES: usize = 1024 * HOMA_BPAGE_SIZE;

/// Largest region the kernel accepts (bpage offsets are 32-bit).
pub const MAX_REGION_BYTES: u64 = 1 << 32;

/// One contiguous piece of a received message inside the region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fragment {
    /// Byte offset of the fragment within the region.
    pub region_offset: usize,
    /// Number of payload bytes in this fragment.
    pub len: usize,
}

/// Validated placement of a message inside the receive region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentLayout {
    fragments: Vec<Fragment>,
    total_len: usize,
}

impl FragmentLayout {
    /// Interpret the kernel's `recvmsg` output for a message of `message_len`
    /// bytes inside a region of `region_len` bytes.
    ///
    /// Rules (from `recvmsg.2` / `homa_receiver.h`): fragment `i` holds
    /// message bytes `[i·BPAGE, min((i+1)·BPAGE, len))`; every fragment but
    /// the last is a full, bpage-aligned bpage; the last may be shorter and
    /// unaligned. The number of fragments must therefore equal
    /// `ceil(len / BPAGE)`.
    ///
    /// # Errors
    /// [`HomaError::MalformedReceive`] if the descriptors are internally
    /// inconsistent or point outside the region. Nothing is read from memory.
    pub fn from_recv_args(
        args: &homa_recvmsg_args,
        message_len: usize,
        region_len: usize,
    ) -> Result<Self> {
        let malformed = |what: String| HomaError::MalformedReceive(what);

        if message_len == 0 || message_len > HOMA_MAX_MESSAGE_LENGTH {
            return Err(malformed(format!(
                "message length {message_len} out of range"
            )));
        }
        let num = args.num_bpages as usize;
        if num > HOMA_MAX_BPAGES {
            return Err(malformed(format!(
                "num_bpages {num} exceeds HOMA_MAX_BPAGES"
            )));
        }
        let expected = message_len.div_ceil(HOMA_BPAGE_SIZE);
        if num != expected {
            return Err(malformed(format!(
                "num_bpages {num} does not match ceil({message_len} / {HOMA_BPAGE_SIZE}) = {expected}"
            )));
        }

        let mut fragments = Vec::with_capacity(num);
        let mut remaining = message_len;
        for (i, &off) in args.bpage_offsets[..num].iter().enumerate() {
            let region_offset = off as usize;
            let len = remaining.min(HOMA_BPAGE_SIZE);
            let is_last = i + 1 == num;
            if !is_last && region_offset % HOMA_BPAGE_SIZE != 0 {
                return Err(malformed(format!(
                    "fragment {i} offset {region_offset} is not bpage-aligned"
                )));
            }
            // The tail may be unaligned but must stay inside one bpage: the
            // kernel allocates a fresh page rather than straddle a boundary
            // (`homa_pool_alloc_msg`). A straddling tail would otherwise read
            // into a page that may belong to another in-flight message.
            if is_last && region_offset % HOMA_BPAGE_SIZE + len > HOMA_BPAGE_SIZE {
                return Err(malformed(format!(
                    "final fragment [{region_offset}, {}) crosses a bpage boundary",
                    region_offset + len
                )));
            }
            let end = region_offset
                .checked_add(len)
                .ok_or_else(|| malformed(format!("fragment {i} offset overflow")))?;
            if end > region_len {
                return Err(malformed(format!(
                    "fragment {i} [{region_offset}, {end}) exceeds region of {region_len} bytes"
                )));
            }
            fragments.push(Fragment { region_offset, len });
            remaining -= len;
        }
        debug_assert_eq!(remaining, 0);

        // Fragments must not overlap one another (a kernel bug would otherwise
        // let us read a page being written for another message).
        let mut sorted: Vec<Fragment> = fragments.clone();
        sorted.sort_by_key(|f| f.region_offset);
        for w in sorted.windows(2) {
            if w[0].region_offset + w[0].len > w[1].region_offset {
                return Err(malformed(format!(
                    "fragments overlap: {:?} and {:?}",
                    w[0], w[1]
                )));
            }
        }

        Ok(Self {
            fragments,
            total_len: message_len,
        })
    }

    /// Fragments in message order.
    #[must_use]
    pub fn fragments(&self) -> &[Fragment] {
        &self.fragments
    }

    /// Total message length.
    #[must_use]
    pub fn total_len(&self) -> usize {
        self.total_len
    }
}

/// The receive-buffer region owned by one socket.
///
/// The mapping is an anonymous `mmap`, so this type exists only on Unix.
#[cfg(unix)]
#[derive(Debug)]
pub struct ReceiveBuffer {
    region: homa_sys::region::MappedRegion,
}

#[cfg(unix)]
impl ReceiveBuffer {
    /// Map a region of `bytes` (rounded up to a whole number of bpages).
    ///
    /// # Errors
    /// [`HomaError::InvalidBufferConfig`] if `bytes` is zero or exceeds 4 GiB;
    /// [`HomaError::Os`] if `mmap` fails.
    pub fn new(bytes: usize) -> Result<Self> {
        if bytes == 0 {
            return Err(HomaError::InvalidBufferConfig("size must be > 0".into()));
        }
        let rounded = bytes.div_ceil(HOMA_BPAGE_SIZE) * HOMA_BPAGE_SIZE;
        if rounded as u64 > MAX_REGION_BYTES {
            return Err(HomaError::InvalidBufferConfig(format!(
                "size {rounded} exceeds the 4 GiB limit imposed by 32-bit bpage offsets"
            )));
        }
        let region = homa_sys::region::MappedRegion::anonymous(rounded)
            .map_err(|e| HomaError::os("mmap", e))?;
        Ok(Self { region })
    }

    /// Base address to register with `SO_HOMA_RCVBUF`.
    #[must_use]
    pub fn as_ptr(&self) -> *mut u8 {
        self.region.as_ptr()
    }

    /// Region length in bytes (a multiple of the bpage size).
    #[must_use]
    pub fn len(&self) -> usize {
        self.region.len()
    }

    /// Always `false`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.region.is_empty()
    }

    /// Number of bpages in the region.
    #[must_use]
    pub fn bpage_count(&self) -> usize {
        self.len() / HOMA_BPAGE_SIZE
    }

    /// Copy a message out of the region into an owned `Vec<u8>`.
    ///
    /// # Safety contract (upheld internally)
    ///
    /// `layout` was validated against `self.len()`, so every fragment lies in
    /// the mapping. The caller (the socket) guarantees the bpages in `layout`
    /// are currently owned by the application - i.e. returned by the most
    /// recent `recvmsg` and not yet released - so the kernel will not write to
    /// them concurrently.
    #[must_use]
    pub fn copy_out(&self, layout: &FragmentLayout) -> Vec<u8> {
        let mut out = Vec::with_capacity(layout.total_len());
        for frag in layout.fragments() {
            debug_assert!(frag.region_offset + frag.len <= self.len());
            // SAFETY: `FragmentLayout::from_recv_args` proved
            // `region_offset + len <= region.len()`, the mapping is readable
            // for its whole length, and (per the contract above) the kernel
            // does not mutate these bytes while we own the bpages. We read
            // raw bytes (`u8` has no validity requirements) and never form a
            // long-lived reference into the region.
            let src = unsafe {
                core::slice::from_raw_parts(self.region.as_ptr().add(frag.region_offset), frag.len)
            };
            out.extend_from_slice(src);
        }
        out
    }

    /// Write bytes into the region at `offset`. **Test support only**: used to
    /// simulate the kernel depositing a message; never call this on a region
    /// that has been registered with a live socket.
    #[cfg(test)]
    pub(crate) fn write_at(&self, offset: usize, bytes: &[u8]) {
        assert!(offset + bytes.len() <= self.len());
        // SAFETY: bounds asserted; region is exclusively ours in tests.
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.region.as_ptr().add(offset),
                bytes.len(),
            );
        }
    }
}

/// Bpages owned by the application that still have to be returned to Homa.
///
/// Returning is batched: the next `recvmsg` carries up to
/// `HOMA_MAX_BPAGES` offsets in its control block; anything beyond that is
/// released explicitly with `HOMAIOCRELEASE`. The queue is also drained when
/// the socket is dropped.
#[derive(Debug, Default)]
pub struct BpageQueue {
    pending: Mutex<Vec<u32>>,
}

impl BpageQueue {
    /// Empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<u32>> {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Record bpages that now belong to the application.
    pub fn push(&self, offsets: &[u32]) {
        self.lock().extend_from_slice(offsets);
    }

    /// Move up to `HOMA_MAX_BPAGES` pending offsets into `args` so the next
    /// `recvmsg` returns them. Returns how many were loaded.
    pub fn load_into(&self, args: &mut homa_recvmsg_args) -> usize {
        let mut pending = self.lock();
        let n = pending.len().min(HOMA_MAX_BPAGES);
        let start = pending.len() - n;
        args.bpage_offsets[..n].copy_from_slice(&pending[start..]);
        pending.truncate(start);
        args.num_bpages = n as u32;
        n
    }

    /// Take *all* pending offsets (for explicit release / shutdown), in
    /// batches of at most `HOMA_MAX_BPAGES`.
    pub fn drain_batches(&self) -> Vec<homa_recvmsg_args> {
        let mut pending = self.lock();
        let mut batches = Vec::new();
        for chunk in pending.chunks(HOMA_MAX_BPAGES) {
            let mut args = homa_recvmsg_args::zeroed();
            args.bpage_offsets[..chunk.len()].copy_from_slice(chunk);
            args.num_bpages = chunk.len() as u32;
            batches.push(args);
        }
        pending.clear();
        batches
    }

    /// Number of bpages waiting to be returned.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// `true` if nothing is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BP: usize = HOMA_BPAGE_SIZE;

    fn args(offsets: &[u32]) -> homa_recvmsg_args {
        let mut a = homa_recvmsg_args::zeroed();
        a.bpage_offsets[..offsets.len()].copy_from_slice(offsets);
        a.num_bpages = offsets.len() as u32;
        a
    }

    #[test]
    fn single_fragment_layout() {
        let l = FragmentLayout::from_recv_args(&args(&[3 * BP as u32 + 100]), 10, 16 * BP).unwrap();
        assert_eq!(
            l.fragments(),
            &[Fragment {
                region_offset: 3 * BP + 100,
                len: 10
            }]
        );
        assert_eq!(l.total_len(), 10);
    }

    #[test]
    fn multi_fragment_layout_splits_at_bpage_boundaries() {
        let len = 2 * BP + 1234;
        let l = FragmentLayout::from_recv_args(
            &args(&[0, 5 * BP as u32, 2 * BP as u32 + 64]),
            len,
            16 * BP,
        )
        .unwrap();
        assert_eq!(
            l.fragments(),
            &[
                Fragment {
                    region_offset: 0,
                    len: BP
                },
                Fragment {
                    region_offset: 5 * BP,
                    len: BP
                },
                Fragment {
                    region_offset: 2 * BP + 64,
                    len: 1234
                },
            ]
        );
    }

    #[test]
    fn exact_multiple_of_bpage_has_no_tail_fragment() {
        let l = FragmentLayout::from_recv_args(&args(&[BP as u32, 0]), 2 * BP, 4 * BP).unwrap();
        assert_eq!(l.fragments().len(), 2);
        assert_eq!(l.fragments()[1].len, BP);
    }

    #[test]
    fn max_message_uses_sixteen_fragments() {
        let offsets: Vec<u32> = (0..16).map(|i| (i * BP) as u32).collect();
        let l = FragmentLayout::from_recv_args(&args(&offsets), HOMA_MAX_MESSAGE_LENGTH, 16 * BP)
            .unwrap();
        assert_eq!(l.fragments().len(), 16);
        assert_eq!(l.fragments()[15].len, HOMA_MAX_MESSAGE_LENGTH - 15 * BP);
    }

    #[test]
    fn rejects_inconsistent_descriptors() {
        let region = 16 * BP;
        // Wrong fragment count for the length.
        assert!(FragmentLayout::from_recv_args(&args(&[0]), BP + 1, region).is_err());
        assert!(FragmentLayout::from_recv_args(&args(&[0, BP as u32]), 10, region).is_err());
        // Zero-length / oversized messages.
        assert!(FragmentLayout::from_recv_args(&args(&[0]), 0, region).is_err());
        assert!(
            FragmentLayout::from_recv_args(&args(&[0]), HOMA_MAX_MESSAGE_LENGTH + 1, region)
                .is_err()
        );
        // Out of region.
        assert!(FragmentLayout::from_recv_args(&args(&[(16 * BP - 4) as u32]), 8, region).is_err());
        // Non-final fragment not aligned.
        assert!(FragmentLayout::from_recv_args(&args(&[8, BP as u32]), BP + 8, region).is_err());
        // Overlapping fragments.
        assert!(FragmentLayout::from_recv_args(&args(&[0, 0]), BP + 8, region).is_err());
        // Final fragment straddling a bpage boundary (single- and multi-page).
        assert!(FragmentLayout::from_recv_args(&args(&[(BP - 4) as u32]), 8, region).is_err());
        assert!(
            FragmentLayout::from_recv_args(&args(&[0, (2 * BP - 4) as u32]), BP + 8, region)
                .is_err()
        );
        // ...but an unaligned tail that stays within its page is fine.
        assert!(FragmentLayout::from_recv_args(&args(&[(BP - 8) as u32]), 8, region).is_ok());
        // Too many fragments.
        let mut a = args(&[0; 16]);
        a.num_bpages = 17;
        assert!(FragmentLayout::from_recv_args(&a, HOMA_MAX_MESSAGE_LENGTH, region).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn copy_out_reassembles_multi_page_message_in_order() {
        let buf = ReceiveBuffer::new(8 * BP).unwrap();
        assert_eq!(buf.bpage_count(), 8);
        let len = 2 * BP + 777;
        let message: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
        // Scatter the message the way the kernel might: pages 6, 1, then a
        // tail at an unaligned offset inside page 3.
        let offsets = [6 * BP as u32, BP as u32, (3 * BP + 4096) as u32];
        buf.write_at(offsets[0] as usize, &message[..BP]);
        buf.write_at(offsets[1] as usize, &message[BP..2 * BP]);
        buf.write_at(offsets[2] as usize, &message[2 * BP..]);

        let layout = FragmentLayout::from_recv_args(&args(&offsets), len, buf.len()).unwrap();
        assert_eq!(buf.copy_out(&layout), message);
    }

    #[cfg(unix)]
    #[test]
    fn copy_out_preserves_null_bytes() {
        let buf = ReceiveBuffer::new(BP).unwrap();
        let message = b"\0\0abc\0def\0".to_vec();
        buf.write_at(100, &message);
        let layout =
            FragmentLayout::from_recv_args(&args(&[100]), message.len(), buf.len()).unwrap();
        assert_eq!(buf.copy_out(&layout), message);
    }

    #[cfg(unix)]
    #[test]
    fn receive_buffer_rounds_up_and_validates() {
        assert!(ReceiveBuffer::new(0).is_err());
        let buf = ReceiveBuffer::new(BP + 1).unwrap();
        assert_eq!(buf.len(), 2 * BP);
        assert!(!buf.is_empty());
        assert_eq!(buf.as_ptr() as usize % homa_sys::region::page_size(), 0);
        #[cfg(target_pointer_width = "64")]
        assert!(ReceiveBuffer::new((MAX_REGION_BYTES + 1) as usize).is_err());
    }

    #[test]
    fn bpage_queue_batches_returns() {
        let q = BpageQueue::new();
        assert!(q.is_empty());
        let offsets: Vec<u32> = (0..20).map(|i| i * BP as u32).collect();
        q.push(&offsets);
        assert_eq!(q.len(), 20);

        let mut a = homa_recvmsg_args::zeroed();
        assert_eq!(q.load_into(&mut a), 16);
        assert_eq!(a.num_bpages, 16);
        assert_eq!(q.len(), 4);
        // The 16 most recently pushed were taken; the first 4 remain.
        let mut taken = a.offsets().to_vec();
        taken.sort_unstable();
        assert_eq!(taken, offsets[4..].to_vec());

        let batches = q.drain_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].offsets(), &offsets[..4]);
        assert!(q.is_empty());

        // Draining an empty queue yields nothing.
        assert!(q.drain_batches().is_empty());
        let mut a = homa_recvmsg_args::zeroed();
        assert_eq!(q.load_into(&mut a), 0);
        assert_eq!(a.num_bpages, 0);
    }

    #[test]
    fn bpage_queue_drains_in_max_bpage_chunks() {
        let q = BpageQueue::new();
        let offsets: Vec<u32> = (0..40).collect();
        q.push(&offsets);
        let batches = q.drain_batches();
        assert_eq!(
            batches.iter().map(|b| b.num_bpages).collect::<Vec<_>>(),
            vec![16, 16, 8]
        );
        let all: Vec<u32> = batches.iter().flat_map(|b| b.offsets().to_vec()).collect();
        assert_eq!(all, offsets);
    }
}
