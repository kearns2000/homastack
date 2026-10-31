//! Error translation, the thread-local "last error", and the panic boundary.
//!
//! # Last-error model
//!
//! Every exported function that returns a status other than [`HOMA_OK`]
//! records a description for the *calling thread* before returning. The record
//! holds the status code, the underlying `errno` (or `0`) and a UTF-8 message.
//! It stays readable until the next failing call on the same thread or
//! `homa_clear_last_error`; successful calls do not touch it. Nothing is
//! shared between threads.
//!
//! # Panic boundary
//!
//! `extern "C"` functions must never unwind. Every export runs its body inside
//! [`guard`], which catches a Rust panic, records it as
//! [`HOMA_ERR_INTERNAL`] and returns normally. The library's shared state is
//! designed so that a panic cannot leave it half-updated: the socket registry
//! only performs single `Vec` operations under its lock, and `homa-rs`
//! tolerates a poisoned bpage-queue mutex. A panic therefore never requires
//! aborting the process; the status code is the signal that a bug occurred.

use std::cell::RefCell;
use std::ffi::c_char;
use std::panic::{AssertUnwindSafe, catch_unwind};

use homa_rs::{ErrorKind, HomaError};

use crate::types::{
    HOMA_ERR_ADDRESS_IN_USE, HOMA_ERR_BUFFER_TOO_SMALL, HOMA_ERR_INTERNAL, HOMA_ERR_INTERRUPTED,
    HOMA_ERR_INVALID_ARGUMENT, HOMA_ERR_INVALID_ENDPOINT, HOMA_ERR_IO, HOMA_ERR_MESSAGE_TOO_LARGE,
    HOMA_ERR_NO_MEMORY, HOMA_ERR_NO_SERVER, HOMA_ERR_PERMISSION_DENIED, HOMA_ERR_RPC_FAILED,
    HOMA_ERR_SHUTDOWN, HOMA_ERR_TIMEOUT, HOMA_ERR_UNAVAILABLE, HOMA_ERR_UNREACHABLE,
    HOMA_ERR_WOULD_BLOCK, HOMA_OK, HomaStatus,
};

/// What the calling thread last failed with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LastError {
    /// Status code that was returned.
    pub status: HomaStatus,
    /// Underlying OS `errno`, or `0` if none.
    pub errno: i32,
    /// Human-readable description (UTF-8, no NUL).
    pub message: String,
}

thread_local! {
    static LAST_ERROR: RefCell<Option<LastError>> = const { RefCell::new(None) };
}

/// Record `status` with `message` for this thread and return `status`.
///
/// Uses `try_borrow_mut` so a conflicting borrow cannot panic across the C
/// boundary. If the slot is already borrowed, the status is still returned
/// and the previous record is left as it was.
pub fn fail(status: HomaStatus, errno: i32, message: impl Into<String>) -> HomaStatus {
    let mut message = message.into();
    message.retain(|c| c != '\0');
    LAST_ERROR.with(|slot| {
        if let Ok(mut guard) = slot.try_borrow_mut() {
            *guard = Some(LastError {
                status,
                errno,
                message,
            });
        }
    });
    status
}

/// Record a [`HomaError`] and return its status code.
pub fn fail_with(err: &HomaError) -> HomaStatus {
    fail(
        status_for_error(err),
        err.raw_os_error().unwrap_or(0),
        err.to_string(),
    )
}

/// Current record for this thread, if any.
///
/// `None` when nothing is recorded or the slot is already borrowed. Never
/// panics.
#[must_use]
pub fn last_error() -> Option<LastError> {
    LAST_ERROR.with(|slot| slot.try_borrow().ok().and_then(|guard| guard.clone()))
}

/// Forget this thread's record. A conflicting borrow is ignored.
pub fn clear_last_error() {
    LAST_ERROR.with(|slot| {
        if let Ok(mut guard) = slot.try_borrow_mut() {
            *guard = None;
        }
    });
}

/// Map a [`HomaError`] to the stable C status code.
///
/// RPC-level failures reported by the kernel keep their most useful
/// distinction (`NO_SERVER`, `UNREACHABLE`) and otherwise become
/// `RPC_FAILED`; the errno is always retained in the last-error record.
#[must_use]
pub fn status_for_error(err: &HomaError) -> HomaStatus {
    match err {
        HomaError::MessageTooLarge { .. } => HOMA_ERR_MESSAGE_TOO_LARGE,
        HomaError::EmptyMessage
        | HomaError::InvalidRpcId(_)
        | HomaError::UnexpectedMessage { .. }
        | HomaError::InvalidBufferConfig(_) => HOMA_ERR_INVALID_ARGUMENT,
        HomaError::InvalidAddress(_)
        | HomaError::AddressFamilyMismatch { .. }
        | HomaError::InvalidPort(_) => HOMA_ERR_INVALID_ENDPOINT,
        HomaError::Timeout(_) => HOMA_ERR_TIMEOUT,
        HomaError::MalformedReceive(_) => HOMA_ERR_INTERNAL,
        HomaError::RpcFailed { .. } => match err.kind() {
            ErrorKind::NoServer => HOMA_ERR_NO_SERVER,
            ErrorKind::Unreachable => HOMA_ERR_UNREACHABLE,
            ErrorKind::Shutdown => HOMA_ERR_SHUTDOWN,
            _ => HOMA_ERR_RPC_FAILED,
        },
        HomaError::Os { .. } => status_for_kind(err.kind()),
        // `HomaError` is `#[non_exhaustive]`; anything new is "some OS error"
        // until this mapping is extended.
        _ => HOMA_ERR_IO,
    }
}

fn status_for_kind(kind: ErrorKind) -> HomaStatus {
    match kind {
        ErrorKind::Unavailable => HOMA_ERR_UNAVAILABLE,
        ErrorKind::WouldBlock => HOMA_ERR_WOULD_BLOCK,
        ErrorKind::Timeout => HOMA_ERR_TIMEOUT,
        ErrorKind::Shutdown => HOMA_ERR_SHUTDOWN,
        ErrorKind::NoServer => HOMA_ERR_NO_SERVER,
        ErrorKind::Unreachable => HOMA_ERR_UNREACHABLE,
        ErrorKind::AddressInUse => HOMA_ERR_ADDRESS_IN_USE,
        ErrorKind::PermissionDenied => HOMA_ERR_PERMISSION_DENIED,
        ErrorKind::NoMemory => HOMA_ERR_NO_MEMORY,
        ErrorKind::Interrupted => HOMA_ERR_INTERRUPTED,
        ErrorKind::InvalidInput => HOMA_ERR_INVALID_ARGUMENT,
        _ => HOMA_ERR_IO,
    }
}

/// Run `body` inside the panic boundary. A panic is recorded as
/// [`HOMA_ERR_INTERNAL`] for this thread and that status is returned.
///
/// Public so the behaviour can be tested through the real boundary; it is not
/// part of the C ABI.
pub fn guard<F>(body: F) -> HomaStatus
where
    F: FnOnce() -> HomaStatus,
{
    // `AssertUnwindSafe`: bodies only capture raw pointers and integers from
    // the C caller plus `Arc`s looked up inside; nothing captured is observed
    // after a panic.
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(status) => status,
        Err(payload) => fail(
            HOMA_ERR_INTERNAL,
            0,
            format!("panic inside homa-ffi: {}", panic_text(payload.as_ref())),
        ),
    }
}

/// [`guard`] for functions that return a value rather than a status; `fallback`
/// is returned (and the panic recorded) if `body` panics.
pub fn guard_value<T, F>(fallback: T, body: F) -> T
where
    F: FnOnce() -> T,
{
    match catch_unwind(AssertUnwindSafe(body)) {
        Ok(v) => v,
        Err(payload) => {
            fail(
                HOMA_ERR_INTERNAL,
                0,
                format!("panic inside homa-ffi: {}", panic_text(payload.as_ref())),
            );
            fallback
        }
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_owned()
    }
}

/// Implementation of `homa_last_error_message`: copy the message as a
/// NUL-terminated string into `buf`.
///
/// # Safety
///
/// `buf` must be valid for `buf_len` writable bytes (or null with
/// `buf_len == 0`); `required_len` may be null.
pub unsafe fn copy_last_error_message(
    buf: *mut c_char,
    buf_len: u64,
    required_len: *mut u64,
) -> HomaStatus {
    let message = last_error().map(|e| e.message).unwrap_or_default();
    let needed = message.len() as u64 + 1;
    if !required_len.is_null() {
        // SAFETY: caller contract - `required_len` is null or writable.
        unsafe { required_len.write(needed) };
    }
    if buf.is_null() || buf_len < needed {
        // Note: deliberately does *not* overwrite the last-error record, so
        // the caller can retry with a bigger buffer.
        return HOMA_ERR_BUFFER_TOO_SMALL;
    }
    // SAFETY: `buf` is valid for `buf_len >= needed` bytes (caller contract);
    // we write `message.len()` bytes plus one NUL. `u8` -> `c_char` is a
    // bit-for-bit reinterpretation.
    unsafe {
        std::ptr::copy_nonoverlapping(message.as_ptr().cast::<c_char>(), buf, message.len());
        buf.add(message.len()).write(0);
    }
    HOMA_OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn maps_pure_rust_errors() {
        assert_eq!(
            status_for_error(&HomaError::MessageTooLarge { len: 2, max: 1 }),
            HOMA_ERR_MESSAGE_TOO_LARGE
        );
        assert_eq!(
            status_for_error(&HomaError::EmptyMessage),
            HOMA_ERR_INVALID_ARGUMENT
        );
        assert_eq!(
            status_for_error(&HomaError::InvalidPort(40000)),
            HOMA_ERR_INVALID_ENDPOINT
        );
        assert_eq!(
            status_for_error(&HomaError::InvalidAddress("x".into())),
            HOMA_ERR_INVALID_ENDPOINT
        );
        assert_eq!(
            status_for_error(&HomaError::Timeout(std::time::Duration::from_millis(5))),
            HOMA_ERR_TIMEOUT
        );
        assert_eq!(
            status_for_error(&HomaError::MalformedReceive("bad".into())),
            HOMA_ERR_INTERNAL
        );
        assert_eq!(
            status_for_error(&HomaError::InvalidRpcId(homa_rs::RpcId::from_raw(0))),
            HOMA_ERR_INVALID_ARGUMENT
        );
    }

    #[test]
    fn maps_rpc_failures_by_kind() {
        let rpc = |errno: i32| HomaError::RpcFailed {
            id: homa_rs::RpcId::from_raw(2),
            completion_cookie: 0,
            source: io::Error::from_raw_os_error(errno),
            detail: None,
        };
        assert_eq!(status_for_error(&rpc(libc::ETIMEDOUT)), HOMA_ERR_RPC_FAILED);
        assert_eq!(status_for_error(&rpc(libc::ENOTCONN)), HOMA_ERR_NO_SERVER);
        assert_eq!(
            status_for_error(&rpc(libc::EHOSTUNREACH)),
            HOMA_ERR_UNREACHABLE
        );
        assert_eq!(status_for_error(&rpc(libc::ESHUTDOWN)), HOMA_ERR_SHUTDOWN);
        assert_eq!(status_for_error(&rpc(libc::EIO)), HOMA_ERR_RPC_FAILED);
    }

    #[test]
    fn maps_every_error_kind() {
        assert_eq!(
            status_for_kind(ErrorKind::Unavailable),
            HOMA_ERR_UNAVAILABLE
        );
        assert_eq!(status_for_kind(ErrorKind::WouldBlock), HOMA_ERR_WOULD_BLOCK);
        assert_eq!(status_for_kind(ErrorKind::Timeout), HOMA_ERR_TIMEOUT);
        assert_eq!(status_for_kind(ErrorKind::Shutdown), HOMA_ERR_SHUTDOWN);
        assert_eq!(status_for_kind(ErrorKind::NoServer), HOMA_ERR_NO_SERVER);
        assert_eq!(
            status_for_kind(ErrorKind::Unreachable),
            HOMA_ERR_UNREACHABLE
        );
        assert_eq!(
            status_for_kind(ErrorKind::AddressInUse),
            HOMA_ERR_ADDRESS_IN_USE
        );
        assert_eq!(
            status_for_kind(ErrorKind::PermissionDenied),
            HOMA_ERR_PERMISSION_DENIED
        );
        assert_eq!(status_for_kind(ErrorKind::NoMemory), HOMA_ERR_NO_MEMORY);
        assert_eq!(
            status_for_kind(ErrorKind::Interrupted),
            HOMA_ERR_INTERRUPTED
        );
        assert_eq!(
            status_for_kind(ErrorKind::InvalidInput),
            HOMA_ERR_INVALID_ARGUMENT
        );
        assert_eq!(status_for_kind(ErrorKind::Other), HOMA_ERR_IO);
    }

    #[test]
    fn last_error_is_thread_local_and_clearable() {
        clear_last_error();
        assert!(last_error().is_none());
        assert_eq!(fail(HOMA_ERR_IO, 5, "disk\0on fire"), HOMA_ERR_IO);
        let e = last_error().unwrap();
        assert_eq!(e.status, HOMA_ERR_IO);
        assert_eq!(e.errno, 5);
        assert_eq!(e.message, "diskon fire"); // NUL stripped
        let other = std::thread::spawn(last_error).join().unwrap();
        assert!(other.is_none(), "must not leak across threads");
        clear_last_error();
        assert!(last_error().is_none());
    }

    #[test]
    fn guard_contains_panics() {
        clear_last_error();
        assert_eq!(guard(|| panic!("boom {}", 42)), HOMA_ERR_INTERNAL);
        let e = last_error().unwrap();
        assert_eq!(e.status, HOMA_ERR_INTERNAL);
        assert!(e.message.contains("boom 42"), "{}", e.message);
        assert_eq!(guard(|| HOMA_OK), HOMA_OK);
        assert_eq!(guard_value(7u32, || panic!()), 7);
        assert_eq!(guard_value(7u32, || 8), 8);
    }

    #[test]
    // All pointers below are to live local arrays of the stated length.
    #[allow(clippy::undocumented_unsafe_blocks)]
    fn copies_message_with_required_length() {
        fail(HOMA_ERR_IO, 0, "hello");
        let mut needed = 0u64;
        // Too small: status + required length, record preserved.
        let mut small = [0 as c_char; 3];
        let st = unsafe { copy_last_error_message(small.as_mut_ptr(), 3, &raw mut needed) };
        assert_eq!(st, HOMA_ERR_BUFFER_TOO_SMALL);
        assert_eq!(needed, 6);
        assert_eq!(last_error().unwrap().message, "hello");
        // Query only.
        let st = unsafe { copy_last_error_message(std::ptr::null_mut(), 0, &raw mut needed) };
        assert_eq!(st, HOMA_ERR_BUFFER_TOO_SMALL);
        assert_eq!(needed, 6);
        // Big enough.
        let mut big = [0x7f as c_char; 16];
        let st = unsafe { copy_last_error_message(big.as_mut_ptr(), 16, std::ptr::null_mut()) };
        assert_eq!(st, HOMA_OK);
        let text = unsafe { std::ffi::CStr::from_ptr(big.as_ptr()) };
        assert_eq!(text.to_str().unwrap(), "hello");
        // No error recorded: empty string, needs one byte.
        clear_last_error();
        let st = unsafe { copy_last_error_message(big.as_mut_ptr(), 16, &raw mut needed) };
        assert_eq!(st, HOMA_OK);
        assert_eq!(needed, 1);
        assert_eq!(big[0], 0);
    }
}
