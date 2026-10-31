//! `#[repr(C)]` mirrors of the user-space ABI declared in upstream `homa.h`.
//!
//! Every item documents the upstream declaration it corresponds to. The
//! upstream header is vendored at `upstream/homa.h` in this crate.
//!
//! Naming deliberately follows the C header (snake_case struct names) so that
//! the correspondence is obvious when auditing against the kernel source.

#![allow(non_camel_case_types)]

use core::ffi::{c_char, c_int, c_ulong};
use core::mem::offset_of;

/// Git commit of `PlatformLab/HomaModule` these definitions were verified
/// against.
pub const UPSTREAM_REVISION: &str = "4bd0275714439da2955670a6c438f6c23304d785";

/// IANA-assigned IP protocol number for Homa (`IPPROTO_HOMA`).
///
/// Used as the third argument to `socket(2)` and as the `level` argument to
/// `setsockopt(2)`/`getsockopt(2)`.
pub const IPPROTO_HOMA: c_int = 146;

/// Maximum bytes of payload in a request or response (`HOMA_MAX_MESSAGE_LENGTH`).
pub const HOMA_MAX_MESSAGE_LENGTH: usize = 1_000_000;

/// `HOMA_BPAGE_SHIFT`: log2 of the receive buffer page ("bpage") size.
pub const HOMA_BPAGE_SHIFT: u32 = 16;

/// `HOMA_BPAGE_SIZE`: size in bytes of a receive buffer page (64 KiB).
pub const HOMA_BPAGE_SIZE: usize = 1 << HOMA_BPAGE_SHIFT;

/// `HOMA_MAX_BPAGES`: largest number of bpages needed for one message (16).
pub const HOMA_MAX_BPAGES: usize =
    (HOMA_MAX_MESSAGE_LENGTH + HOMA_BPAGE_SIZE - 1) >> HOMA_BPAGE_SHIFT;

/// `HOMA_MIN_DEFAULT_PORT`: ports `>= 0x8000` are auto-assigned client ports;
/// `bind(2)` only accepts ports `1..HOMA_MIN_DEFAULT_PORT`.
pub const HOMA_MIN_DEFAULT_PORT: u16 = 0x8000;

/// `HOMA_SENDMSG_PRIVATE`: flag bit for [`homa_sendmsg_args::flags`]. Marks a
/// request as *private*: its response can only be received by passing the RPC
/// id explicitly to `recvmsg`, and it never satisfies a shared (`id == 0`)
/// receive or `poll(2)`.
pub const HOMA_SENDMSG_PRIVATE: u32 = 0x01;

/// `HOMA_SENDMSG_VALID_FLAGS`: mask of all currently defined send flags.
pub const HOMA_SENDMSG_VALID_FLAGS: u32 = 0x01;

/// `SO_HOMA_RCVBUF`: `setsockopt` option (level [`IPPROTO_HOMA`]) that
/// registers the receive buffer region; argument is [`homa_rcvbuf_args`].
pub const SO_HOMA_RCVBUF: c_int = 10;

/// `SO_HOMA_SERVER`: `setsockopt`/`getsockopt` option (level
/// [`IPPROTO_HOMA`]) with an `int` argument; nonzero enables incoming requests.
pub const SO_HOMA_SERVER: c_int = 11;

/// `HOMA_ERROR_MSG_SIZE`: size of [`homa_info::error_msg`].
pub const HOMA_ERROR_MSG_SIZE: usize = 104;

/// `HOMA_RPC_BUF_STALL` flag bit in [`homa_rpc_info::flags`].
pub const HOMA_RPC_BUF_STALL: u16 = 1;
/// `HOMA_RPC_PRIVATE` flag bit in [`homa_rpc_info::flags`].
pub const HOMA_RPC_PRIVATE: u16 = 2;
/// `HOMA_RPC_RX_READY` flag bit in [`homa_rpc_info::flags`].
pub const HOMA_RPC_RX_READY: u16 = 4;
/// `HOMA_RPC_RX_COPY` flag bit in [`homa_rpc_info::flags`].
pub const HOMA_RPC_RX_COPY: u16 = 8;

/// `struct homa_sendmsg_args` - passed to `sendmsg(2)` through
/// `msghdr::msg_control`.
///
/// Upstream quirk (see `man/sendmsg.2` and `homa_plumbing.c::homa_sendmsg`):
/// `msg_controllen` **must be 0**. The kernel reads `sizeof(struct
/// homa_sendmsg_args)` bytes directly through the user pointer in
/// `msg_control` and writes the assigned RPC id back through the same pointer.
/// A nonzero `msg_controllen` makes the generic socket layer copy the control
/// buffer into kernel space, after which Homa rejects the call with `EINVAL`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct homa_sendmsg_args {
    /// (in/out) `0` ⇒ send a new request; the kernel overwrites this with the
    /// new RPC id. Nonzero ⇒ send the response for that (server-side) id.
    pub id: u64,
    /// (in) Requests only: opaque value returned by `recvmsg` with the response.
    /// Must be `0` for responses (`EINVAL` otherwise).
    pub completion_cookie: u64,
    /// (in) OR of `HOMA_SENDMSG_*` bits.
    pub flags: u32,
    /// Must be `0`.
    pub reserved: u32,
}

/// `struct homa_recvmsg_args` - passed to `recvmsg(2)` through
/// `msghdr::msg_control` with `msg_controllen == size_of::<Self>()` (exactly;
/// anything else is `EINVAL`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct homa_recvmsg_args {
    /// (in/out) `0` ⇒ wait for any shared message (request or non-private
    /// response). Nonzero ⇒ wait for the response of that private client RPC.
    /// On return: the id of the message received. Odd ids are requests
    /// (server side), even ids are responses (client side).
    pub id: u64,
    /// (out) For responses, the cookie passed when the request was sent;
    /// `0` for requests.
    pub completion_cookie: u64,
    /// (in/out) Number of valid entries in [`Self::bpage_offsets`]. In:
    /// bpages being returned to Homa. Out: bpages holding the new message.
    pub num_bpages: u32,
    /// Must be `0`.
    pub reserved: u32,
    /// (in/out) Byte offsets into the registered receive region. All entries
    /// but the last describe full, `HOMA_BPAGE_SIZE`-aligned bpages; the last
    /// may be a shorter, unaligned fragment.
    pub bpage_offsets: [u32; HOMA_MAX_BPAGES],
}

impl Default for homa_recvmsg_args {
    fn default() -> Self {
        Self::zeroed()
    }
}

impl homa_recvmsg_args {
    /// All-zero arguments: "receive any shared message, return no bpages".
    #[must_use]
    pub const fn zeroed() -> Self {
        Self {
            id: 0,
            completion_cookie: 0,
            num_bpages: 0,
            reserved: 0,
            bpage_offsets: [0; HOMA_MAX_BPAGES],
        }
    }

    /// The valid prefix of [`Self::bpage_offsets`] (clamped to
    /// `HOMA_MAX_BPAGES` so a corrupt count can never index out of bounds).
    #[must_use]
    pub fn offsets(&self) -> &[u32] {
        let n = (self.num_bpages as usize).min(HOMA_MAX_BPAGES);
        &self.bpage_offsets[..n]
    }
}

/// `struct homa_rcvbuf_args` - argument of `SO_HOMA_RCVBUF`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct homa_rcvbuf_args {
    /// User-space address of the first byte of the region (page aligned).
    pub start: u64,
    /// Total bytes available at `start`. The kernel requires at least
    /// `nr_cpu_ids * HOMA_BPAGE_SIZE + 2 * HOMA_MAX_MESSAGE_LENGTH` and at
    /// most 4 GiB (offsets are 32-bit).
    pub length: u64,
}

/// `struct homa_abort_args` - argument of [`HOMAIOCABORT`].
///
/// Present in the GitHub `HomaModule` build; the `strip.py`-generated
/// upstream-kernel variant omits it (guarded by `#ifndef __STRIP__`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::pub_underscore_fields)] // field names mirror the C header
pub struct homa_abort_args {
    /// RPC id to abort, or `0` for all outgoing RPCs on the socket.
    pub id: u64,
    /// `0` ⇒ free the RPC silently; nonzero ⇒ complete it with this errno so a
    /// `recvmsg` returns the failure.
    pub error: u32,
    /// Must be `0`.
    pub _pad1: u32,
    /// Must be `0`.
    pub _pad2: [u64; 2],
}

/// 128-byte, 8-aligned storage mirroring `struct __kernel_sockaddr_storage`
/// (the `peer` union in [`homa_rpc_info`]). Interpret via `sa_family` in the
/// first two bytes.
#[repr(C, align(8))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct homa_sockaddr_storage {
    /// Raw bytes of a `sockaddr_in` / `sockaddr_in6`.
    pub bytes: [u8; 128],
}

/// `struct homa_rpc_info` - per-RPC record returned by [`HOMAIOCINFO`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct homa_rpc_info {
    /// RPC id; low bit set ⇒ this node is the server.
    pub id: u64,
    /// Peer socket address.
    pub peer: homa_sockaddr_storage,
    /// Completion cookie (client RPCs only).
    pub completion_cookie: u64,
    /// Outgoing message length or `-1`.
    pub tx_length: i32,
    /// Bytes transmitted at least once.
    pub tx_sent: u32,
    /// Bytes the receiver has granted.
    pub tx_granted: u32,
    /// Current transmit priority (`reserved` in the stripped variant).
    pub tx_prio: u32,
    /// Incoming message length or `-1`.
    pub rx_length: i32,
    /// Bytes not yet received.
    pub rx_remaining: u32,
    /// Number of gaps in the incoming message.
    pub rx_gaps: u32,
    /// Total bytes in gaps.
    pub rx_gap_bytes: u32,
    /// Bytes granted to the sender.
    pub rx_granted: u32,
    /// OR of `HOMA_RPC_*` flag bits.
    pub flags: u16,
}

/// `struct homa_info` - in/out argument of [`HOMAIOCINFO`].
#[repr(C)]
#[derive(Clone, Copy)]
pub struct homa_info {
    /// (in) User address of a `homa_rpc_info` array, or `0`.
    pub rpc_info: u64,
    /// (in) Bytes available at `rpc_info`.
    pub rpc_info_length: u64,
    /// (out) Free bytes in the receive buffer pool.
    pub bpool_avail_bytes: u64,
    /// (out) Port number of the socket.
    pub port: u32,
    /// (out) Number of active RPCs.
    pub num_rpcs: u32,
    /// (out) NUL-terminated description of the last Homa error on this socket.
    pub error_msg: [c_char; HOMA_ERROR_MSG_SIZE],
}

impl Default for homa_info {
    fn default() -> Self {
        Self {
            rpc_info: 0,
            rpc_info_length: 0,
            bpool_avail_bytes: 0,
            port: 0,
            num_rpcs: 0,
            error_msg: [0; HOMA_ERROR_MSG_SIZE],
        }
    }
}

impl core::fmt::Debug for homa_info {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("homa_info")
            .field("rpc_info", &self.rpc_info)
            .field("rpc_info_length", &self.rpc_info_length)
            .field("bpool_avail_bytes", &self.bpool_avail_bytes)
            .field("port", &self.port)
            .field("num_rpcs", &self.num_rpcs)
            .field("error_msg", &self.error_message())
            .finish()
    }
}

impl homa_info {
    /// `error_msg` as a Rust string (lossy UTF-8, truncated at the first NUL).
    #[must_use]
    pub fn error_message(&self) -> String {
        // `c_char` is `i8` on x86 and `u8` on aarch64; reinterpret the bytes
        // without a (platform-dependent) numeric cast.
        let bytes: Vec<u8> = self
            .error_msg
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c.to_ne_bytes()[0])
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

// ---------------------------------------------------------------------------
// ioctl request numbers
// ---------------------------------------------------------------------------
//
// Linux `_IOC(dir, type, nr, size)` on the generic (asm-generic) layout used by
// x86, x86_64, arm, aarch64, riscv, s390, loongarch:
//
//   bits  0..8  nr
//   bits  8..16 type
//   bits 16..30 size (14 bits)
//   bits 30..32 dir  (_IOC_NONE=0, _IOC_WRITE=1, _IOC_READ=2)
//
// alpha, mips, powerpc and sparc use different field widths/dir values; refuse
// to build there rather than emit wrong numbers.
#[cfg(any(
    target_arch = "powerpc",
    target_arch = "powerpc64",
    target_arch = "mips",
    target_arch = "mips64",
    target_arch = "sparc",
    target_arch = "sparc64"
))]
compile_error!(
    "homa-sys: ioctl number encoding for this architecture differs from asm-generic; \
     add the correct _IOC layout before building"
);

const IOC_NONE: u32 = 0;
const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

const fn ioc(dir: u32, ty: u8, nr: u8, size: usize) -> c_ulong {
    assert!(size < (1 << 14), "ioctl size field overflow");
    ((dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | (nr as u32)) as c_ulong
}

/// `HOMAIOCINFO = _IOWR('h', 0x90, struct homa_info)`.
pub const HOMAIOCINFO: c_ulong = ioc(IOC_READ | IOC_WRITE, b'h', 0x90, size_of::<homa_info>());
/// `HOMAIOCRELEASE = _IOW('h', 0x91, struct homa_recvmsg_args)`.
pub const HOMAIOCRELEASE: c_ulong = ioc(IOC_WRITE, b'h', 0x91, size_of::<homa_recvmsg_args>());
/// `HOMAIOCABORT = _IOWR('h', 0x92, struct homa_abort_args)` (non-stripped
/// module only).
pub const HOMAIOCABORT: c_ulong = ioc(
    IOC_READ | IOC_WRITE,
    b'h',
    0x92,
    size_of::<homa_abort_args>(),
);
/// `HOMAIOCFREEZE = _IO('h', 0x93)` (debug aid, non-stripped module only).
pub const HOMAIOCFREEZE: c_ulong = ioc(IOC_NONE, b'h', 0x93, 0);

// ---------------------------------------------------------------------------
// Compile-time layout assertions. The expected numbers were produced by
// compiling the upstream struct definitions with a C compiler (see
// tests/header_check.rs for the runtime equivalent against the real header).
// ---------------------------------------------------------------------------
const _: () = {
    assert!(HOMA_MAX_BPAGES == 16);
    assert!(size_of::<homa_sendmsg_args>() == 24);
    assert!(align_of::<homa_sendmsg_args>() == 8);
    assert!(offset_of!(homa_sendmsg_args, completion_cookie) == 8);
    assert!(offset_of!(homa_sendmsg_args, flags) == 16);
    assert!(offset_of!(homa_sendmsg_args, reserved) == 20);

    assert!(size_of::<homa_recvmsg_args>() == 88);
    assert!(align_of::<homa_recvmsg_args>() == 8);
    assert!(offset_of!(homa_recvmsg_args, completion_cookie) == 8);
    assert!(offset_of!(homa_recvmsg_args, num_bpages) == 16);
    assert!(offset_of!(homa_recvmsg_args, reserved) == 20);
    assert!(offset_of!(homa_recvmsg_args, bpage_offsets) == 24);

    assert!(size_of::<homa_rcvbuf_args>() == 16);
    assert!(size_of::<homa_abort_args>() == 32);
    assert!(size_of::<homa_sockaddr_storage>() == 128);
    assert!(align_of::<homa_sockaddr_storage>() == 8);
    assert!(size_of::<homa_rpc_info>() == 184);
    assert!(offset_of!(homa_rpc_info, peer) == 8);
    assert!(offset_of!(homa_rpc_info, completion_cookie) == 136);
    assert!(offset_of!(homa_rpc_info, tx_length) == 144);
    assert!(offset_of!(homa_rpc_info, flags) == 180);
    assert!(size_of::<homa_info>() == 136);
    assert!(offset_of!(homa_info, port) == 24);
    assert!(offset_of!(homa_info, error_msg) == 32);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_c_encoding() {
        // Values computed with `_IOWR('h', 0x90, struct homa_info)` etc. using
        // the asm-generic ioctl macros.
        assert_eq!(HOMAIOCINFO, 0xc088_6890);
        assert_eq!(HOMAIOCRELEASE, 0x4058_6891);
        assert_eq!(HOMAIOCABORT, 0xc020_6892);
        assert_eq!(HOMAIOCFREEZE, 0x0000_6893);
    }

    #[test]
    fn recvmsg_args_offsets_are_clamped() {
        let mut args = homa_recvmsg_args::zeroed();
        assert!(args.offsets().is_empty());
        args.num_bpages = 3;
        args.bpage_offsets[..3].copy_from_slice(&[0, 65_536, 131_072]);
        assert_eq!(args.offsets(), &[0, 65_536, 131_072]);
        args.num_bpages = u32::MAX; // corrupt count must not panic / overrun
        assert_eq!(args.offsets().len(), HOMA_MAX_BPAGES);
    }

    #[test]
    fn info_error_message_is_nul_terminated() {
        let mut info = homa_info::default();
        for (dst, src) in info.error_msg.iter_mut().zip(b"bad things\0garbage") {
            *dst = c_char::from_ne_bytes([*src]);
        }
        assert_eq!(info.error_message(), "bad things");
        assert!(format!("{info:?}").contains("bad things"));
    }

    #[test]
    fn constants_match_header() {
        assert_eq!(IPPROTO_HOMA, 146);
        assert_eq!(HOMA_BPAGE_SIZE, 65536);
        assert_eq!(HOMA_MAX_MESSAGE_LENGTH, 1_000_000);
        assert_eq!(HOMA_MIN_DEFAULT_PORT, 32768);
        assert_eq!(SO_HOMA_RCVBUF, 10);
        assert_eq!(SO_HOMA_SERVER, 11);
        assert_eq!(HOMA_SENDMSG_PRIVATE, 1);
    }
}
