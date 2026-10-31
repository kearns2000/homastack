//! Plain C data types shared across the ABI boundary.
//!
//! Everything here is `#[repr(C)]`, uses fixed-width integers only, and has
//! its layout pinned by compile-time assertions that the C probe test
//! (`tests/ffi_tests.rs`, "header_layout_matches") re-checks against
//! `include/homa_ffi.h` with a real C compiler.

use std::ffi::CStr;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

// ---------------------------------------------------------------------------
// Status codes
// ---------------------------------------------------------------------------

/// Status code type (`int32_t` in C). `0` is success, everything else an
/// error; the numeric values are part of the ABI and never change meaning.
pub type HomaStatus = i32;

/// Success.
pub const HOMA_OK: HomaStatus = 0;
/// A pointer was null, a struct size/flag/reserved field was wrong, a length
/// was zero, an RPC id was invalid, or a socket handle is unknown/closed.
pub const HOMA_ERR_INVALID_ARGUMENT: HomaStatus = 1;
/// This build of the library cannot create Homa sockets (not Linux).
pub const HOMA_ERR_UNSUPPORTED_PLATFORM: HomaStatus = 2;
/// Linux, but the `homa` kernel module is not loaded (`EPROTONOSUPPORT`).
pub const HOMA_ERR_UNAVAILABLE: HomaStatus = 3;
/// A `homa_endpoint_t` is malformed, its family does not match the socket, or
/// a bind port is outside `1..=32767`.
pub const HOMA_ERR_INVALID_ENDPOINT: HomaStatus = 4;
/// A caller-supplied deadline elapsed (for `homa_recv_response` /
/// `homa_request` the RPC has been aborted; for `homa_recv` nothing was
/// consumed).
pub const HOMA_ERR_TIMEOUT: HomaStatus = 5;
/// The blocking call was interrupted by a signal (`EINTR`). Safe to retry.
pub const HOMA_ERR_INTERRUPTED: HomaStatus = 6;
/// The kernel abandoned the RPC (e.g. the peer timed out, `ETIMEDOUT`). The
/// errno is available through `homa_last_error_errno`.
pub const HOMA_ERR_RPC_FAILED: HomaStatus = 7;
/// The socket has been shut down (`ESHUTDOWN`).
pub const HOMA_ERR_SHUTDOWN: HomaStatus = 8;
/// Out of memory, including an exhausted receive-buffer pool (`ENOMEM`).
pub const HOMA_ERR_NO_MEMORY: HomaStatus = 9;
/// Non-blocking receive found nothing (`homa_recv` with `timeout_ms == 0`).
pub const HOMA_ERR_WOULD_BLOCK: HomaStatus = 10;
/// The requested server port is already bound (`EADDRINUSE`).
pub const HOMA_ERR_ADDRESS_IN_USE: HomaStatus = 11;
/// Insufficient permission (`EACCES` / `EPERM`).
pub const HOMA_ERR_PERMISSION_DENIED: HomaStatus = 12;
/// No Homa socket is listening on the destination port (`ENOTCONN`).
pub const HOMA_ERR_NO_SERVER: HomaStatus = 13;
/// The destination host or network is unreachable.
pub const HOMA_ERR_UNREACHABLE: HomaStatus = 14;
/// Payload longer than `HOMA_FFI_MAX_MESSAGE_LENGTH`.
pub const HOMA_ERR_MESSAGE_TOO_LARGE: HomaStatus = 15;
/// The caller's buffer is too small; the required size has been written to
/// the accompanying `required_len` output.
pub const HOMA_ERR_BUFFER_TOO_SMALL: HomaStatus = 16;
/// Any other operating-system error; see `homa_last_error_errno`.
pub const HOMA_ERR_IO: HomaStatus = 17;
/// A bug: a Rust panic was caught at the boundary, or the kernel returned
/// data the library could not interpret. The library remains usable.
pub const HOMA_ERR_INTERNAL: HomaStatus = 99;

/// Static, NUL-terminated name of a status code (`"HOMA_OK"`, ...,
/// `"HOMA_ERR_UNKNOWN"` for undefined values). Backs `homa_status_name`.
#[must_use]
pub const fn status_name_c(status: HomaStatus) -> &'static CStr {
    match status {
        HOMA_OK => c"HOMA_OK",
        HOMA_ERR_INVALID_ARGUMENT => c"HOMA_ERR_INVALID_ARGUMENT",
        HOMA_ERR_UNSUPPORTED_PLATFORM => c"HOMA_ERR_UNSUPPORTED_PLATFORM",
        HOMA_ERR_UNAVAILABLE => c"HOMA_ERR_UNAVAILABLE",
        HOMA_ERR_INVALID_ENDPOINT => c"HOMA_ERR_INVALID_ENDPOINT",
        HOMA_ERR_TIMEOUT => c"HOMA_ERR_TIMEOUT",
        HOMA_ERR_INTERRUPTED => c"HOMA_ERR_INTERRUPTED",
        HOMA_ERR_RPC_FAILED => c"HOMA_ERR_RPC_FAILED",
        HOMA_ERR_SHUTDOWN => c"HOMA_ERR_SHUTDOWN",
        HOMA_ERR_NO_MEMORY => c"HOMA_ERR_NO_MEMORY",
        HOMA_ERR_WOULD_BLOCK => c"HOMA_ERR_WOULD_BLOCK",
        HOMA_ERR_ADDRESS_IN_USE => c"HOMA_ERR_ADDRESS_IN_USE",
        HOMA_ERR_PERMISSION_DENIED => c"HOMA_ERR_PERMISSION_DENIED",
        HOMA_ERR_NO_SERVER => c"HOMA_ERR_NO_SERVER",
        HOMA_ERR_UNREACHABLE => c"HOMA_ERR_UNREACHABLE",
        HOMA_ERR_MESSAGE_TOO_LARGE => c"HOMA_ERR_MESSAGE_TOO_LARGE",
        HOMA_ERR_BUFFER_TOO_SMALL => c"HOMA_ERR_BUFFER_TOO_SMALL",
        HOMA_ERR_IO => c"HOMA_ERR_IO",
        HOMA_ERR_INTERNAL => c"HOMA_ERR_INTERNAL",
        _ => c"HOMA_ERR_UNKNOWN",
    }
}

/// [`status_name_c`] as a Rust string.
#[must_use]
pub fn status_name(status: HomaStatus) -> &'static str {
    // All names are ASCII literals, so this cannot fail.
    status_name_c(status).to_str().unwrap_or("HOMA_ERR_UNKNOWN")
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// `homa_endpoint_t.family` for IPv4.
pub const HOMA_ENDPOINT_IPV4: u16 = 4;
/// `homa_endpoint_t.family` for IPv6.
pub const HOMA_ENDPOINT_IPV6: u16 = 6;

/// `homa_message_info_t.kind`: an incoming request that expects a reply.
pub const HOMA_MESSAGE_REQUEST: u32 = 1;
/// `homa_message_info_t.kind`: the response to a request this socket sent.
pub const HOMA_MESSAGE_RESPONSE: u32 = 2;

/// `homa_send_request` flag: the response can only be collected with
/// `homa_recv_response(rpc_id)`; it is never delivered by `homa_recv`.
pub const HOMA_REQUEST_PRIVATE: u32 = 1;

/// `homa_socket_options_t.flags`: accept incoming requests even on an
/// auto-assigned port (`SO_HOMA_SERVER`). Binding a port in `1..=32767`
/// enables this implicitly.
pub const HOMA_SOCKET_SERVER: u32 = 1;

/// Largest payload Homa accepts, in bytes.
pub const HOMA_FFI_MAX_MESSAGE_LENGTH: u32 = homa_rs::MAX_MESSAGE_LENGTH as u32;

/// `timeout_ms` value meaning "block until completion or shutdown".
pub const HOMA_TIMEOUT_INFINITE: i64 = -1;

// ---------------------------------------------------------------------------
// Structures
// ---------------------------------------------------------------------------

/// An IP endpoint in a representation that needs no `sockaddr` knowledge.
///
/// `addr` is in network byte order; IPv4 uses `addr[0..4]` and the rest must
/// be zero. `port` is in host byte order. `scope_id` is the IPv6 scope
/// (interface index) and must be `0` for IPv4.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct homa_endpoint_t {
    /// [`HOMA_ENDPOINT_IPV4`] or [`HOMA_ENDPOINT_IPV6`].
    pub family: u16,
    /// Homa port, host byte order.
    pub port: u16,
    /// IPv6 scope id; `0` for IPv4.
    pub scope_id: u32,
    /// Address bytes, network order.
    pub addr: [u8; 16],
}

impl homa_endpoint_t {
    /// Convert from a Rust address.
    #[must_use]
    pub fn from_socket_addr(sa: &SocketAddr) -> Self {
        let mut ep = Self {
            port: sa.port(),
            ..Self::default()
        };
        match sa.ip() {
            IpAddr::V4(v4) => {
                ep.family = HOMA_ENDPOINT_IPV4;
                ep.addr[..4].copy_from_slice(&v4.octets());
            }
            IpAddr::V6(v6) => {
                ep.family = HOMA_ENDPOINT_IPV6;
                ep.addr = v6.octets();
                if let SocketAddr::V6(s6) = sa {
                    ep.scope_id = s6.scope_id();
                }
            }
        }
        ep
    }

    /// Convert to a Rust address, validating the family and the unused bytes.
    pub fn to_socket_addr(&self) -> Result<SocketAddr, String> {
        match self.family {
            HOMA_ENDPOINT_IPV4 => {
                if self.scope_id != 0 || self.addr[4..].iter().any(|&b| b != 0) {
                    return Err("IPv4 endpoint has non-zero scope_id or addr[4..16]".into());
                }
                let ip = Ipv4Addr::new(self.addr[0], self.addr[1], self.addr[2], self.addr[3]);
                Ok(SocketAddr::from((ip, self.port)))
            }
            HOMA_ENDPOINT_IPV6 => Ok(SocketAddr::V6(SocketAddrV6::new(
                Ipv6Addr::from(self.addr),
                self.port,
                0,
                self.scope_id,
            ))),
            other => Err(format!(
                "unknown endpoint family {other} (expected {HOMA_ENDPOINT_IPV4} or {HOMA_ENDPOINT_IPV6})"
            )),
        }
    }
}

/// Parameters for `homa_socket_open` / `homa_socket_bind`.
///
/// `struct_size` must be `sizeof(homa_socket_options_t)`; it lets future ABI
/// versions accept older, shorter layouts. `reserved` must be `0`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct homa_socket_options_t {
    /// `sizeof(homa_socket_options_t)`.
    pub struct_size: u32,
    /// [`HOMA_ENDPOINT_IPV4`] or [`HOMA_ENDPOINT_IPV6`].
    pub family: u16,
    /// `0` for an auto-assigned client port (>= 32768); `1..=32767` to bind a
    /// server port.
    pub bind_port: u16,
    /// Receive-buffer region size in bytes; `0` selects the default (64 MiB).
    /// The kernel requires at least `nr_cpu_ids * 64 KiB + 2 MiB` and at most
    /// 4 GiB.
    pub receive_region_bytes: u64,
    /// Bitwise OR of `HOMA_SOCKET_*` flags.
    pub flags: u32,
    /// Must be `0`.
    pub reserved: u32,
}

impl Default for homa_socket_options_t {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            family: HOMA_ENDPOINT_IPV4,
            bind_port: 0,
            receive_region_bytes: 0,
            flags: 0,
            reserved: 0,
        }
    }
}

/// Snapshot of a socket for diagnostics (`homa_socket_info`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct homa_socket_info_t {
    /// Caller sets to `sizeof(homa_socket_info_t)`.
    pub struct_size: u32,
    /// Local Homa port.
    pub port: u16,
    /// [`HOMA_ENDPOINT_IPV4`] or [`HOMA_ENDPOINT_IPV6`].
    pub family: u16,
    /// Active RPCs (client and server side) as reported by the kernel.
    pub active_rpcs: u32,
    /// Receive-buffer pages owned by the application, not yet returned.
    pub pending_bpages: u32,
    /// Free bytes in the kernel's receive-buffer pool.
    pub buffer_available_bytes: u64,
    /// Size of the registered receive region.
    pub receive_region_bytes: u64,
}

/// Metadata of a received message (`homa_message_info`).
///
/// `payload` points into memory owned by the `homa_message_t` and stays valid
/// until `homa_message_free`. It is never null for a successfully received
/// message (Homa rejects empty messages) and is not NUL-terminated.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct homa_message_info_t {
    /// Caller sets to `sizeof(homa_message_info_t)`.
    pub struct_size: u32,
    /// [`HOMA_MESSAGE_REQUEST`] or [`HOMA_MESSAGE_RESPONSE`].
    pub kind: u32,
    /// Homa RPC id as seen by this socket (odd = request, even = response).
    pub rpc_id: u64,
    /// Completion cookie given to `homa_send_request`; `0` for requests.
    pub completion_cookie: u64,
    /// Sender of the message (the client to reply to, for requests).
    pub peer: homa_endpoint_t,
    /// Read-only payload bytes; owned by the message.
    pub payload: *const u8,
    /// Payload length in bytes (`1..=HOMA_FFI_MAX_MESSAGE_LENGTH`).
    pub payload_len: u64,
    /// Receive-buffer pages the kernel used (diagnostic).
    pub fragments: u32,
    /// Always `0`.
    pub reserved: u32,
}

impl Default for homa_message_info_t {
    fn default() -> Self {
        Self {
            struct_size: size_of::<Self>() as u32,
            kind: 0,
            rpc_id: 0,
            completion_cookie: 0,
            peer: homa_endpoint_t::default(),
            payload: std::ptr::null(),
            payload_len: 0,
            fragments: 0,
            reserved: 0,
        }
    }
}

// The ABI is 64-bit only (Linux x86_64 and aarch64). Pointer-carrying structs
// and the socket-handle encoding assume 8-byte pointers.
#[cfg(not(target_pointer_width = "64"))]
compile_error!("homa-ffi supports 64-bit targets only (x86_64, aarch64)");

const _: () = {
    assert!(size_of::<homa_endpoint_t>() == 24);
    assert!(align_of::<homa_endpoint_t>() == 4);
    assert!(size_of::<homa_socket_options_t>() == 24);
    assert!(align_of::<homa_socket_options_t>() == 8);
    assert!(size_of::<homa_socket_info_t>() == 32);
    assert!(align_of::<homa_socket_info_t>() == 8);
    assert!(size_of::<homa_message_info_t>() == 72);
    assert!(align_of::<homa_message_info_t>() == 8);
    assert!(std::mem::offset_of!(homa_message_info_t, peer) == 24);
    assert!(std::mem::offset_of!(homa_message_info_t, payload) == 48);
    assert!(std::mem::offset_of!(homa_message_info_t, payload_len) == 56);
    assert!(std::mem::offset_of!(homa_message_info_t, fragments) == 64);
    assert!(HOMA_FFI_MAX_MESSAGE_LENGTH == 1_000_000);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_round_trips_v4_and_v6() {
        let v4: SocketAddr = "192.0.2.10:5000".parse().unwrap();
        let ep = homa_endpoint_t::from_socket_addr(&v4);
        assert_eq!(ep.family, HOMA_ENDPOINT_IPV4);
        assert_eq!(&ep.addr[..4], &[192, 0, 2, 10]);
        assert_eq!(ep.to_socket_addr().unwrap(), v4);

        let v6: SocketAddr = "[fe80::1%7]:6000".parse().unwrap();
        let ep = homa_endpoint_t::from_socket_addr(&v6);
        assert_eq!(ep.family, HOMA_ENDPOINT_IPV6);
        assert_eq!(ep.scope_id, 7);
        assert_eq!(ep.to_socket_addr().unwrap(), v6);
    }

    #[test]
    fn endpoint_rejects_garbage() {
        let bad = homa_endpoint_t {
            family: 9,
            ..Default::default()
        };
        assert!(bad.to_socket_addr().is_err());
        let mut v4 = homa_endpoint_t::from_socket_addr(&"10.0.0.1:1".parse().unwrap());
        v4.addr[15] = 1;
        assert!(v4.to_socket_addr().is_err());
        v4.addr[15] = 0;
        v4.scope_id = 3;
        assert!(v4.to_socket_addr().is_err());
    }

    #[test]
    fn status_names_are_unique_and_known() {
        let codes = [
            HOMA_OK,
            HOMA_ERR_INVALID_ARGUMENT,
            HOMA_ERR_UNSUPPORTED_PLATFORM,
            HOMA_ERR_UNAVAILABLE,
            HOMA_ERR_INVALID_ENDPOINT,
            HOMA_ERR_TIMEOUT,
            HOMA_ERR_INTERRUPTED,
            HOMA_ERR_RPC_FAILED,
            HOMA_ERR_SHUTDOWN,
            HOMA_ERR_NO_MEMORY,
            HOMA_ERR_WOULD_BLOCK,
            HOMA_ERR_ADDRESS_IN_USE,
            HOMA_ERR_PERMISSION_DENIED,
            HOMA_ERR_NO_SERVER,
            HOMA_ERR_UNREACHABLE,
            HOMA_ERR_MESSAGE_TOO_LARGE,
            HOMA_ERR_BUFFER_TOO_SMALL,
            HOMA_ERR_IO,
            HOMA_ERR_INTERNAL,
        ];
        let names: std::collections::HashSet<&str> =
            codes.iter().map(|&c| status_name(c)).collect();
        assert_eq!(names.len(), codes.len());
        assert!(!names.contains("HOMA_ERR_UNKNOWN"));
        assert_eq!(status_name(12345), "HOMA_ERR_UNKNOWN");
    }
}
