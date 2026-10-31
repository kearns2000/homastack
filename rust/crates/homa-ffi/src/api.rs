//! The exported `extern "C"` functions. Declarations live in
//! `include/homa_ffi.h`; the two must match exactly (checked by
//! `tests/ffi_tests.rs`).
//!
//! Every function:
//! 1. validates pointers, sizes and lengths before touching anything,
//! 2. resolves the socket handle through the registry (stale handles fail
//!    with `HOMA_ERR_INVALID_ARGUMENT`),
//! 3. runs inside the panic boundary ([`crate::errors::guard`]),
//! 4. on failure records a thread-local last error and returns a status code.
//!
//! Output pointers are written on success only, except that pointer outputs
//! (`homa_socket_t **`, `homa_message_t **`) are set to null up front so a
//! caller that ignores the status cannot see a dangling value.

use std::ffi::c_char;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use homa_rs::{HomaError, Message, RpcId};

use crate::errors::{fail, fail_with, guard, guard_value};
use crate::handles::{HandleKey, REGISTRY, Socket, homa_socket_t};
use crate::messages::{self, homa_message_t};
use crate::types::{
    HOMA_ENDPOINT_IPV4, HOMA_ENDPOINT_IPV6, HOMA_ERR_INVALID_ARGUMENT, HOMA_ERR_INVALID_ENDPOINT,
    HOMA_ERR_MESSAGE_TOO_LARGE, HOMA_ERR_TIMEOUT, HOMA_ERR_UNSUPPORTED_PLATFORM,
    HOMA_ERR_WOULD_BLOCK, HOMA_FFI_MAX_MESSAGE_LENGTH, HOMA_OK, HOMA_REQUEST_PRIVATE,
    HOMA_SOCKET_SERVER, HomaStatus, homa_endpoint_t, homa_message_info_t, homa_socket_info_t,
    homa_socket_options_t, status_name_c,
};

/// Current ABI version. `0` was the Phase 1 placeholder (two functions);
/// `1` is the first functional interface.
pub const HOMA_FFI_ABI_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Internal error plumbing
// ---------------------------------------------------------------------------

/// Errors inside the adapter: either from `homa-rs` or "this platform cannot
/// do that at all".
enum FfiError {
    Homa(HomaError),
    /// Only constructed by the non-Linux `imp`.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    Unsupported,
}

impl From<HomaError> for FfiError {
    fn from(e: HomaError) -> Self {
        Self::Homa(e)
    }
}

impl FfiError {
    fn record(&self) -> HomaStatus {
        match self {
            Self::Homa(e) => fail_with(e),
            Self::Unsupported => fail(
                HOMA_ERR_UNSUPPORTED_PLATFORM,
                0,
                format!(
                    "Homa sockets require Linux with the homa kernel module; this library was built for {}",
                    std::env::consts::OS
                ),
            ),
        }
    }
}

type FfiResult<T> = Result<T, FfiError>;

// ---------------------------------------------------------------------------
// Platform implementations (identical signatures on every platform)
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod imp {
    use super::{FfiResult, Socket};
    use homa_rs::{AddressFamily, HomaSocket, Message, RpcId};
    use std::net::SocketAddr;
    use std::time::Duration;

    pub fn open(
        family: AddressFamily,
        bind_port: u16,
        region_bytes: Option<usize>,
        server: bool,
    ) -> FfiResult<Socket> {
        let mut cfg = HomaSocket::config(family);
        if let Some(bytes) = region_bytes {
            cfg = cfg.receive_region_bytes(bytes);
        }
        if server {
            cfg = cfg.server(true);
        }
        let socket = if bind_port == 0 {
            cfg.open()?
        } else {
            let ip: std::net::IpAddr = match family {
                AddressFamily::V4 => std::net::Ipv4Addr::UNSPECIFIED.into(),
                AddressFamily::V6 => std::net::Ipv6Addr::UNSPECIFIED.into(),
            };
            cfg.bind(SocketAddr::new(ip, bind_port))?
        };
        Ok(socket)
    }

    pub fn local_port(s: &Socket) -> u16 {
        s.local_port()
    }

    pub fn family(s: &Socket) -> AddressFamily {
        s.family()
    }

    pub fn info(s: &Socket) -> FfiResult<(u16, u32, u64, u32, u64)> {
        let i = s.info()?;
        Ok((
            i.port,
            i.active_rpcs,
            i.buffer_available_bytes,
            s.pending_bpages() as u32,
            s.receive_region_len() as u64,
        ))
    }

    pub fn shutdown(s: &Socket) -> FfiResult<()> {
        Ok(s.shutdown()?)
    }

    pub fn send_request(
        s: &Socket,
        dest: SocketAddr,
        payload: &[u8],
        cookie: u64,
        private: bool,
    ) -> FfiResult<RpcId> {
        Ok(s.send_request(dest, payload, cookie, private)?)
    }

    pub fn recv_response(s: &Socket, id: RpcId, timeout: Option<Duration>) -> FfiResult<Message> {
        Ok(match timeout {
            None => s.recv_response(id)?,
            Some(t) => s.recv_response_timeout(id, t)?,
        })
    }

    pub fn request(
        s: &Socket,
        dest: SocketAddr,
        payload: &[u8],
        timeout: Option<Duration>,
    ) -> FfiResult<Message> {
        Ok(match timeout {
            None => s.request(dest, payload)?,
            Some(t) => s.request_timeout(dest, payload, t)?,
        })
    }

    /// `Ok(None)` means "nothing available within the timeout".
    pub fn recv(s: &Socket, timeout: Option<Duration>) -> FfiResult<Option<Message>> {
        Ok(match timeout {
            None => Some(s.recv()?),
            Some(t) if t.is_zero() => s.try_recv()?,
            Some(t) => s.recv_timeout(t)?,
        })
    }

    pub fn reply(s: &Socket, id: RpcId, peer: SocketAddr, payload: &[u8]) -> FfiResult<()> {
        Ok(s.reply_to(id, peer, payload)?)
    }

    pub fn abort(s: &Socket, id: Option<RpcId>) -> FfiResult<()> {
        Ok(s.abort(id)?)
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    //! Non-Linux stand-ins. `Socket` is uninhabited here, so every function
    //! taking one is unreachable; only `open` can be called, and it reports
    //! `HOMA_ERR_UNSUPPORTED_PLATFORM`.
    use super::{FfiError, FfiResult, Socket};
    use homa_rs::{AddressFamily, Message, RpcId};
    use std::net::SocketAddr;
    use std::time::Duration;

    pub fn open(
        _family: AddressFamily,
        _bind_port: u16,
        _region_bytes: Option<usize>,
        _server: bool,
    ) -> FfiResult<Socket> {
        Err(FfiError::Unsupported)
    }

    pub fn local_port(s: &Socket) -> u16 {
        match *s {}
    }

    pub fn family(s: &Socket) -> AddressFamily {
        match *s {}
    }

    pub fn info(s: &Socket) -> FfiResult<(u16, u32, u64, u32, u64)> {
        match *s {}
    }

    pub fn shutdown(s: &Socket) -> FfiResult<()> {
        match *s {}
    }

    pub fn send_request(
        s: &Socket,
        _dest: SocketAddr,
        _payload: &[u8],
        _cookie: u64,
        _private: bool,
    ) -> FfiResult<RpcId> {
        match *s {}
    }

    pub fn recv_response(s: &Socket, _id: RpcId, _timeout: Option<Duration>) -> FfiResult<Message> {
        match *s {}
    }

    pub fn request(
        s: &Socket,
        _dest: SocketAddr,
        _payload: &[u8],
        _timeout: Option<Duration>,
    ) -> FfiResult<Message> {
        match *s {}
    }

    pub fn recv(s: &Socket, _timeout: Option<Duration>) -> FfiResult<Option<Message>> {
        match *s {}
    }

    pub fn reply(s: &Socket, _id: RpcId, _peer: SocketAddr, _payload: &[u8]) -> FfiResult<()> {
        match *s {}
    }

    pub fn abort(s: &Socket, _id: Option<RpcId>) -> FfiResult<()> {
        match *s {}
    }
}

// ---------------------------------------------------------------------------
// Argument helpers (each records the last error on failure)
// ---------------------------------------------------------------------------

fn lookup(handle: *const homa_socket_t) -> Result<Arc<Socket>, HomaStatus> {
    let Some(key) = HandleKey::from_ptr(handle) else {
        return Err(fail(HOMA_ERR_INVALID_ARGUMENT, 0, "socket handle is null"));
    };
    REGISTRY.get(key).ok_or_else(|| {
        fail(
            HOMA_ERR_INVALID_ARGUMENT,
            0,
            "socket handle is not open (never issued, or already closed)",
        )
    })
}

/// Validate a `(ptr, len)` payload and view it as a slice.
///
/// # Safety
/// If `ptr` is non-null it must be readable for `len` bytes for the duration
/// of the call (caller contract, documented in the header).
unsafe fn payload_slice<'a>(ptr: *const u8, len: u64) -> Result<&'a [u8], HomaStatus> {
    if ptr.is_null() {
        return Err(fail(
            HOMA_ERR_INVALID_ARGUMENT,
            0,
            "payload pointer is null",
        ));
    }
    if len == 0 {
        return Err(fail(
            HOMA_ERR_INVALID_ARGUMENT,
            0,
            "payload length is 0 (Homa messages must contain at least one byte)",
        ));
    }
    if len > u64::from(HOMA_FFI_MAX_MESSAGE_LENGTH) {
        return Err(fail(
            HOMA_ERR_MESSAGE_TOO_LARGE,
            0,
            format!(
                "payload of {len} bytes exceeds HOMA_FFI_MAX_MESSAGE_LENGTH ({HOMA_FFI_MAX_MESSAGE_LENGTH})"
            ),
        ));
    }
    // SAFETY: non-null, length validated against the ABI maximum, readable for
    // `len` bytes by the caller contract; `u8` has no validity invariants.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len as usize) })
}

fn endpoint(ep: *const homa_endpoint_t) -> Result<SocketAddr, HomaStatus> {
    if ep.is_null() {
        return Err(fail(
            HOMA_ERR_INVALID_ARGUMENT,
            0,
            "endpoint pointer is null",
        ));
    }
    // SAFETY: non-null; the header requires a valid `homa_endpoint_t`.
    let ep = unsafe { &*ep };
    ep.to_socket_addr()
        .map_err(|m| fail(HOMA_ERR_INVALID_ENDPOINT, 0, m))
}

fn out_ptr<T>(out: *mut *mut T, what: &str) -> Result<(), HomaStatus> {
    if out.is_null() {
        return Err(fail(
            HOMA_ERR_INVALID_ARGUMENT,
            0,
            format!("{what} output pointer is null"),
        ));
    }
    // SAFETY: non-null output pointer (caller contract); initialise defensively.
    unsafe { out.write(std::ptr::null_mut()) };
    Ok(())
}

fn check_struct_size(found: u32, expected: usize, what: &str) -> Result<(), HomaStatus> {
    if found as usize == expected {
        Ok(())
    } else {
        Err(fail(
            HOMA_ERR_INVALID_ARGUMENT,
            0,
            format!("{what}.struct_size is {found}, this library expects {expected}"),
        ))
    }
}

fn timeout(ms: i64) -> Option<Duration> {
    (ms >= 0).then(|| Duration::from_millis(ms as u64))
}

fn family(raw: u16) -> Result<homa_rs::AddressFamily, HomaStatus> {
    match raw {
        HOMA_ENDPOINT_IPV4 => Ok(homa_rs::AddressFamily::V4),
        HOMA_ENDPOINT_IPV6 => Ok(homa_rs::AddressFamily::V6),
        other => Err(fail(
            HOMA_ERR_INVALID_ENDPOINT,
            0,
            format!("unknown address family {other}"),
        )),
    }
}

fn family_code(f: homa_rs::AddressFamily) -> u16 {
    match f {
        homa_rs::AddressFamily::V4 => HOMA_ENDPOINT_IPV4,
        homa_rs::AddressFamily::V6 => HOMA_ENDPOINT_IPV6,
    }
}

fn emit_message(msg: Message, out: *mut *mut homa_message_t) -> HomaStatus {
    // SAFETY: `out` was validated non-null by `out_ptr`.
    unsafe { out.write(messages::into_raw(msg)) };
    HOMA_OK
}

fn open_with(opts: &homa_socket_options_t, out: *mut *mut homa_socket_t) -> HomaStatus {
    if let Err(s) = out_ptr(out, "socket") {
        return s;
    }
    if let Err(s) = check_struct_size(
        opts.struct_size,
        size_of::<homa_socket_options_t>(),
        "homa_socket_options_t",
    ) {
        return s;
    }
    if opts.reserved != 0 {
        return fail(
            HOMA_ERR_INVALID_ARGUMENT,
            0,
            "homa_socket_options_t.reserved must be 0",
        );
    }
    if opts.flags & !HOMA_SOCKET_SERVER != 0 {
        return fail(
            HOMA_ERR_INVALID_ARGUMENT,
            0,
            format!("unknown homa_socket_options_t.flags 0x{:x}", opts.flags),
        );
    }
    let fam = match family(opts.family) {
        Ok(f) => f,
        Err(s) => return s,
    };
    if opts.bind_port >= homa_sys::HOMA_MIN_DEFAULT_PORT {
        return fail(
            HOMA_ERR_INVALID_ENDPOINT,
            0,
            format!(
                "bind_port {} is not bindable; server ports are 1..=32767",
                opts.bind_port
            ),
        );
    }
    let region = (opts.receive_region_bytes != 0).then_some(opts.receive_region_bytes as usize);
    match imp::open(
        fam,
        opts.bind_port,
        region,
        opts.flags & HOMA_SOCKET_SERVER != 0,
    ) {
        Ok(socket) => {
            let Some(key) = REGISTRY.insert(Arc::new(socket)) else {
                return fail(
                    crate::types::HOMA_ERR_NO_MEMORY,
                    0,
                    "socket handle table is full",
                );
            };
            // SAFETY: `out` validated non-null above.
            unsafe { out.write(key.to_ptr()) };
            HOMA_OK
        }
        Err(e) => e.record(),
    }
}

// ---------------------------------------------------------------------------
// Library information
// ---------------------------------------------------------------------------

/// ABI version of this library (`1`). Callers must check it before using
/// any other function; see `docs/native-interop.md` for the compatibility
/// rules.
#[unsafe(no_mangle)]
pub extern "C" fn homa_ffi_abi_version() -> u32 {
    HOMA_FFI_ABI_VERSION
}

/// `true` if this build can create Homa sockets (Linux). Whether the kernel
/// module is loaded is only known when `homa_socket_open` runs
/// (`HOMA_ERR_UNAVAILABLE` otherwise).
#[unsafe(no_mangle)]
pub extern "C" fn homa_ffi_platform_supported() -> bool {
    homa_rs::is_supported_platform()
}

/// Largest payload accepted (`HOMA_FFI_MAX_MESSAGE_LENGTH`, 1 000 000).
#[unsafe(no_mangle)]
pub extern "C" fn homa_ffi_max_message_length() -> u32 {
    HOMA_FFI_MAX_MESSAGE_LENGTH
}

/// Static, NUL-terminated name of a status code (`"HOMA_OK"`, ...,
/// `"HOMA_ERR_UNKNOWN"` for values this library does not define). Never
/// null; valid for the life of the process; must not be freed.
#[unsafe(no_mangle)]
pub extern "C" fn homa_status_name(status: HomaStatus) -> *const c_char {
    status_name_c(status).as_ptr()
}

// ---------------------------------------------------------------------------
// Last error
// ---------------------------------------------------------------------------

/// Copy this thread's last error message (UTF-8, NUL-terminated) into `buf`.
///
/// Writes the required size including the NUL to `*required_len` (if
/// non-null) and returns `HOMA_ERR_BUFFER_TOO_SMALL` without touching the
/// record if `buf_len` is insufficient, so the call can be repeated. With no
/// recorded error the message is empty (`required_len == 1`).
///
/// # Safety
/// `buf` must be writable for `buf_len` bytes, or null with `buf_len == 0`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_last_error_message(
    buf: *mut c_char,
    buf_len: u64,
    required_len: *mut u64,
) -> HomaStatus {
    // Not wrapped in `guard`: a panic handler would overwrite the record the
    // caller is trying to read. `copy_last_error_message` uses `try_borrow`
    // and does not panic.
    // SAFETY: forwarded caller contract.
    unsafe { crate::errors::copy_last_error_message(buf, buf_len, required_len) }
}

/// Status code of this thread's last recorded error (`HOMA_OK` if none).
#[unsafe(no_mangle)]
pub extern "C" fn homa_last_error_status() -> HomaStatus {
    crate::errors::last_error().map_or(HOMA_OK, |e| e.status)
}

/// OS `errno` behind this thread's last recorded error (`0` if none or not
/// OS-related).
#[unsafe(no_mangle)]
pub extern "C" fn homa_last_error_errno() -> i32 {
    crate::errors::last_error().map_or(0, |e| e.errno)
}

/// Forget this thread's last recorded error.
#[unsafe(no_mangle)]
pub extern "C" fn homa_clear_last_error() {
    crate::errors::clear_last_error();
}

// ---------------------------------------------------------------------------
// Socket lifecycle
// ---------------------------------------------------------------------------

/// Create a socket. `options` may be null for the defaults (IPv4, auto
/// port, 64 MiB region, requests not accepted). On success `*out` receives a
/// handle that must eventually be passed to `homa_socket_close`.
///
/// # Safety
/// `options` must be null or point to a valid `homa_socket_options_t`; `out`
/// must be a valid, writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_socket_open(
    options: *const homa_socket_options_t,
    out: *mut *mut homa_socket_t,
) -> HomaStatus {
    guard(|| {
        let opts = if options.is_null() {
            homa_socket_options_t::default()
        } else {
            // SAFETY: non-null; header requires a valid struct.
            unsafe { *options }
        };
        open_with(&opts, out)
    })
}

/// Create a socket bound to server port `port` (`1..=32767`); equivalent to
/// `homa_socket_open` with `bind_port = port`. Other fields come from
/// `options` (nullable, as for `homa_socket_open`).
///
/// # Safety
/// As for `homa_socket_open`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_socket_bind(
    options: *const homa_socket_options_t,
    port: u16,
    out: *mut *mut homa_socket_t,
) -> HomaStatus {
    guard(|| {
        let mut opts = if options.is_null() {
            homa_socket_options_t::default()
        } else {
            // SAFETY: non-null; header requires a valid struct.
            unsafe { *options }
        };
        if port == 0 {
            return fail(
                HOMA_ERR_INVALID_ENDPOINT,
                0,
                "homa_socket_bind requires a server port in 1..=32767 (use homa_socket_open for an auto-assigned port)",
            );
        }
        opts.bind_port = port;
        open_with(&opts, out)
    })
}

/// Close a socket and release everything it owns. The handle is invalid
/// afterwards; any later use fails with `HOMA_ERR_INVALID_ARGUMENT` (the
/// library never dereferences handles, so this is safe, not undefined).
///
/// If other threads are still inside calls on this socket, the socket is
/// shut down immediately (they return `HOMA_ERR_SHUTDOWN`) and destroyed when
/// the last of them returns. If that shutdown itself fails, the handle is
/// still released and the shutdown status is returned, so a later close sees
/// `HOMA_ERR_INVALID_ARGUMENT` and in-flight waits may not have been woken.
/// Closing an already-closed or unknown handle returns
/// `HOMA_ERR_INVALID_ARGUMENT`; null returns `HOMA_OK`.
#[unsafe(no_mangle)]
pub extern "C" fn homa_socket_close(socket: *mut homa_socket_t) -> HomaStatus {
    guard(|| {
        let Some(key) = HandleKey::from_ptr(socket) else {
            return HOMA_OK;
        };
        match REGISTRY.remove(key) {
            Some(arc) => {
                // The handle is already out of the registry. If another
                // thread is inside a call, shut the socket down so that
                // call returns HOMA_ERR_SHUTDOWN instead of blocking until
                // the kernel's own timeout. A shutdown failure is reported
                // *after* the handle is released: retrying close then
                // returns HOMA_ERR_INVALID_ARGUMENT, and in-flight waits
                // may not have been woken.
                let wake = if Arc::strong_count(&arc) > 1 {
                    imp::shutdown(&arc)
                } else {
                    Ok(())
                };
                drop(arc);
                wake.map_or_else(|e| e.record(), |()| HOMA_OK)
            }
            None => fail(
                HOMA_ERR_INVALID_ARGUMENT,
                0,
                "socket handle is not open (never issued, or already closed)",
            ),
        }
    })
}

/// Disable the socket: blocked receivers fail with `HOMA_ERR_SHUTDOWN` and
/// further sends/receives are rejected. The handle stays valid until
/// `homa_socket_close`.
#[unsafe(no_mangle)]
pub extern "C" fn homa_socket_shutdown(socket: *mut homa_socket_t) -> HomaStatus {
    guard(|| match lookup(socket) {
        Ok(s) => imp::shutdown(&s).map_or_else(|e| e.record(), |()| HOMA_OK),
        Err(st) => st,
    })
}

/// Local Homa port of the socket (bound, or auto-assigned >= 32768).
///
/// # Safety
/// `out_port` must be a valid, writable pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_socket_local_port(
    socket: *const homa_socket_t,
    out_port: *mut u16,
) -> HomaStatus {
    guard(|| {
        if out_port.is_null() {
            return fail(HOMA_ERR_INVALID_ARGUMENT, 0, "out_port is null");
        }
        match lookup(socket) {
            Ok(s) => {
                // SAFETY: non-null, writable (caller contract).
                unsafe { out_port.write(imp::local_port(&s)) };
                HOMA_OK
            }
            Err(st) => st,
        }
    })
}

/// Diagnostic snapshot of the socket. `info->struct_size` must be set by the
/// caller to `sizeof(homa_socket_info_t)`.
///
/// # Safety
/// `info` must point to a valid, writable `homa_socket_info_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_socket_info(
    socket: *const homa_socket_t,
    info: *mut homa_socket_info_t,
) -> HomaStatus {
    guard(|| {
        if info.is_null() {
            return fail(HOMA_ERR_INVALID_ARGUMENT, 0, "info is null");
        }
        // SAFETY: non-null, valid struct (caller contract).
        let info = unsafe { &mut *info };
        if let Err(s) = check_struct_size(
            info.struct_size,
            size_of::<homa_socket_info_t>(),
            "homa_socket_info_t",
        ) {
            return s;
        }
        let s = match lookup(socket) {
            Ok(s) => s,
            Err(st) => return st,
        };
        match imp::info(&s) {
            Ok((port, active_rpcs, avail, pending, region)) => {
                info.port = port;
                info.family = family_code(imp::family(&s));
                info.active_rpcs = active_rpcs;
                info.pending_bpages = pending;
                info.buffer_available_bytes = avail;
                info.receive_region_bytes = region;
                HOMA_OK
            }
            Err(e) => e.record(),
        }
    })
}

// ---------------------------------------------------------------------------
// Messaging
// ---------------------------------------------------------------------------

/// Send a request without waiting. `*out_rpc_id` receives the kernel's RPC
/// id. `flags` is `0` or `HOMA_REQUEST_PRIVATE`. `completion_cookie` is
/// echoed in the response's `homa_message_info_t`.
///
/// # Safety
/// `dest` must point to a valid `homa_endpoint_t`; `payload` must be readable
/// for `payload_len` bytes for the duration of the call (it is copied by the
/// kernel before the call returns); `out_rpc_id` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_send_request(
    socket: *mut homa_socket_t,
    dest: *const homa_endpoint_t,
    payload: *const u8,
    payload_len: u64,
    completion_cookie: u64,
    flags: u32,
    out_rpc_id: *mut u64,
) -> HomaStatus {
    guard(|| {
        if out_rpc_id.is_null() {
            return fail(HOMA_ERR_INVALID_ARGUMENT, 0, "out_rpc_id is null");
        }
        if flags & !HOMA_REQUEST_PRIVATE != 0 {
            return fail(
                HOMA_ERR_INVALID_ARGUMENT,
                0,
                format!("unknown request flags 0x{flags:x}"),
            );
        }
        // SAFETY: caller contract on `payload`.
        let bytes = match unsafe { payload_slice(payload, payload_len) } {
            Ok(b) => b,
            Err(st) => return st,
        };
        let dest = match endpoint(dest) {
            Ok(d) => d,
            Err(st) => return st,
        };
        let s = match lookup(socket) {
            Ok(s) => s,
            Err(st) => return st,
        };
        match imp::send_request(
            &s,
            dest,
            bytes,
            completion_cookie,
            flags & HOMA_REQUEST_PRIVATE != 0,
        ) {
            Ok(id) => {
                // SAFETY: non-null (checked above), writable (caller contract).
                unsafe { out_rpc_id.write(id.as_raw()) };
                HOMA_OK
            }
            Err(e) => e.record(),
        }
    })
}

/// Wait for the response to private request `rpc_id`.
///
/// `timeout_ms < 0` blocks until the response, an RPC failure or shutdown.
/// `timeout_ms >= 0` polls until the deadline; on expiry the RPC is
/// **aborted** and `HOMA_ERR_TIMEOUT` returned (a `0` deadline therefore
/// means "one attempt, then give up on the RPC"). `EINTR` is retried
/// internally. At most one thread may wait on a given `rpc_id`.
///
/// # Safety
/// `out_message` must be writable. The returned message must be released
/// with `homa_message_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_recv_response(
    socket: *mut homa_socket_t,
    rpc_id: u64,
    timeout_ms: i64,
    out_message: *mut *mut homa_message_t,
) -> HomaStatus {
    guard(|| {
        if let Err(st) = out_ptr(out_message, "message") {
            return st;
        }
        let s = match lookup(socket) {
            Ok(s) => s,
            Err(st) => return st,
        };
        match imp::recv_response(&s, RpcId::from_raw(rpc_id), timeout(timeout_ms)) {
            Ok(msg) => emit_message(msg, out_message),
            Err(e) => e.record(),
        }
    })
}

/// Send a private request and wait for its response in one call
/// (`homa_send_request` + `homa_recv_response`). Timeout semantics as for
/// `homa_recv_response`.
///
/// # Safety
/// As for `homa_send_request` and `homa_recv_response`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_request(
    socket: *mut homa_socket_t,
    dest: *const homa_endpoint_t,
    payload: *const u8,
    payload_len: u64,
    timeout_ms: i64,
    out_message: *mut *mut homa_message_t,
) -> HomaStatus {
    guard(|| {
        if let Err(st) = out_ptr(out_message, "message") {
            return st;
        }
        // SAFETY: caller contract on `payload`.
        let bytes = match unsafe { payload_slice(payload, payload_len) } {
            Ok(b) => b,
            Err(st) => return st,
        };
        let dest = match endpoint(dest) {
            Ok(d) => d,
            Err(st) => return st,
        };
        let s = match lookup(socket) {
            Ok(s) => s,
            Err(st) => return st,
        };
        match imp::request(&s, dest, bytes, timeout(timeout_ms)) {
            Ok(msg) => emit_message(msg, out_message),
            Err(e) => e.record(),
        }
    })
}

/// Receive the next *shared* message: an incoming request (servers) or the
/// response to a non-private request.
///
/// `timeout_ms < 0` blocks; `0` returns `HOMA_ERR_WOULD_BLOCK` immediately if
/// nothing is queued; `> 0` waits up to the deadline and returns
/// `HOMA_ERR_TIMEOUT` (nothing is consumed or aborted on timeout). Several
/// threads may call this concurrently; each message goes to exactly one.
///
/// # Safety
/// `out_message` must be writable. The returned message must be released
/// with `homa_message_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_recv(
    socket: *mut homa_socket_t,
    timeout_ms: i64,
    out_message: *mut *mut homa_message_t,
) -> HomaStatus {
    guard(|| {
        if let Err(st) = out_ptr(out_message, "message") {
            return st;
        }
        let s = match lookup(socket) {
            Ok(s) => s,
            Err(st) => return st,
        };
        match imp::recv(&s, timeout(timeout_ms)) {
            Ok(Some(msg)) => emit_message(msg, out_message),
            Ok(None) if timeout_ms == 0 => {
                fail(HOMA_ERR_WOULD_BLOCK, 0, "no shared message is queued")
            }
            Ok(None) => fail(
                HOMA_ERR_TIMEOUT,
                0,
                format!("no shared message arrived within {timeout_ms} ms"),
            ),
            Err(e) => e.record(),
        }
    })
}

/// Send the response for request `rpc_id` (the odd id from
/// `homa_message_info_t`) to `peer` (its `peer` field). Replying to an RPC
/// the kernel no longer knows succeeds silently (upstream semantics).
///
/// # Safety
/// `peer` must point to a valid `homa_endpoint_t`; `payload` must be readable
/// for `payload_len` bytes for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_reply(
    socket: *mut homa_socket_t,
    rpc_id: u64,
    peer: *const homa_endpoint_t,
    payload: *const u8,
    payload_len: u64,
) -> HomaStatus {
    guard(|| {
        // SAFETY: caller contract on `payload`.
        let bytes = match unsafe { payload_slice(payload, payload_len) } {
            Ok(b) => b,
            Err(st) => return st,
        };
        let peer = match endpoint(peer) {
            Ok(p) => p,
            Err(st) => return st,
        };
        let s = match lookup(socket) {
            Ok(s) => s,
            Err(st) => return st,
        };
        imp::reply(&s, RpcId::from_raw(rpc_id), peer, bytes)
            .map_or_else(|e| e.record(), |()| HOMA_OK)
    })
}

/// `homa_reply` using the id and peer of a received request message.
///
/// # Safety
/// `request` must be a live message from this library; `payload` as for
/// `homa_reply`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_reply_to_message(
    socket: *mut homa_socket_t,
    request: *const homa_message_t,
    payload: *const u8,
    payload_len: u64,
) -> HomaStatus {
    guard(|| {
        if request.is_null() {
            return fail(HOMA_ERR_INVALID_ARGUMENT, 0, "request message is null");
        }
        // SAFETY: non-null, live message (caller contract).
        let req = unsafe { messages::borrow(request) };
        // SAFETY: caller contract on `payload`.
        let bytes = match unsafe { payload_slice(payload, payload_len) } {
            Ok(b) => b,
            Err(st) => return st,
        };
        let s = match lookup(socket) {
            Ok(s) => s,
            Err(st) => return st,
        };
        imp::reply(&s, req.id, req.peer, bytes).map_or_else(|e| e.record(), |()| HOMA_OK)
    })
}

/// Abort outgoing RPC `rpc_id` (or every outgoing RPC if `0`). A late
/// response is discarded. Aborting an unknown id fails with
/// `HOMA_ERR_INVALID_ARGUMENT`.
#[unsafe(no_mangle)]
pub extern "C" fn homa_abort_rpc(socket: *mut homa_socket_t, rpc_id: u64) -> HomaStatus {
    guard(|| {
        let s = match lookup(socket) {
            Ok(s) => s,
            Err(st) => return st,
        };
        let id = (rpc_id != 0).then(|| RpcId::from_raw(rpc_id));
        imp::abort(&s, id).map_or_else(|e| e.record(), |()| HOMA_OK)
    })
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Describe a received message. `info->struct_size` must be set by the
/// caller to `sizeof(homa_message_info_t)`. `info->payload` stays valid
/// until `homa_message_free(message)`.
///
/// # Safety
/// `message` must be a live message from this library; `info` must point to
/// a valid, writable `homa_message_info_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_message_info(
    message: *const homa_message_t,
    info: *mut homa_message_info_t,
) -> HomaStatus {
    guard(|| {
        if message.is_null() {
            return fail(HOMA_ERR_INVALID_ARGUMENT, 0, "message is null");
        }
        if info.is_null() {
            return fail(HOMA_ERR_INVALID_ARGUMENT, 0, "info is null");
        }
        // SAFETY: non-null, valid struct (caller contract).
        let info = unsafe { &mut *info };
        if let Err(s) = check_struct_size(
            info.struct_size,
            size_of::<homa_message_info_t>(),
            "homa_message_info_t",
        ) {
            return s;
        }
        // SAFETY: non-null, live message (caller contract).
        messages::fill_info(unsafe { messages::borrow(message) }, info);
        HOMA_OK
    })
}

/// Payload pointer and length of a message (same data as
/// `homa_message_info`). Returns null and writes `0` if `message` is null.
///
/// # Safety
/// `message` must be null or a live message from this library; `out_len` may
/// be null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_message_payload(
    message: *const homa_message_t,
    out_len: *mut u64,
) -> *const u8 {
    guard_value(std::ptr::null(), || {
        let (ptr, len) = if message.is_null() {
            (std::ptr::null(), 0)
        } else {
            // SAFETY: non-null, live message (caller contract).
            let m = unsafe { messages::borrow(message) };
            (m.payload.as_ptr(), m.payload.len() as u64)
        };
        if !out_len.is_null() {
            // SAFETY: non-null (checked), writable (caller contract).
            unsafe { out_len.write(len) };
        }
        ptr
    })
}

/// Release a message returned by `homa_recv*` / `homa_request`. Null is
/// ignored. Each message must be freed exactly once; the payload pointer is
/// invalid afterwards.
///
/// # Safety
/// `message` must be null or a message from this library that has not been
/// freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn homa_message_free(message: *mut homa_message_t) {
    guard_value((), || {
        // SAFETY: forwarded caller contract.
        unsafe { messages::free(message) };
    });
}

/// Names of every function exported by the C ABI, in header order. Used by
/// the header consistency test and by CI's symbol check.
pub const EXPORTED_FUNCTIONS: &[&str] = &[
    "homa_ffi_abi_version",
    "homa_ffi_platform_supported",
    "homa_ffi_max_message_length",
    "homa_status_name",
    "homa_last_error_message",
    "homa_last_error_status",
    "homa_last_error_errno",
    "homa_clear_last_error",
    "homa_socket_open",
    "homa_socket_bind",
    "homa_socket_close",
    "homa_socket_shutdown",
    "homa_socket_local_port",
    "homa_socket_info",
    "homa_send_request",
    "homa_recv_response",
    "homa_request",
    "homa_recv",
    "homa_reply",
    "homa_reply_to_message",
    "homa_abort_rpc",
    "homa_message_info",
    "homa_message_payload",
    "homa_message_free",
];
