//! **homa-ffi** - a stable, versioned C ABI over [`homa_rs`] (Phase 2).
//!
//! This crate is an *adapter*: every socket operation is performed by
//! `homa_rs::HomaSocket`, which in turn drives the Linux Homa kernel module
//! through `homa-sys`. Nothing here talks to the kernel, implements any part
//! of the wire protocol, or adds a second transport.
//!
//! Built as a `cdylib` it produces `libhoma_ffi.so` (Linux) whose only
//! exported symbols are the `homa_*` functions declared in
//! `include/homa_ffi.h`. The header is the public contract; this crate's Rust
//! items are public only so the tests can exercise them and so an in-process
//! Rust host can link the `rlib` instead.
//!
//! # Design (see `docs/native-interop.md` for the full rules)
//!
//! * **Status codes** - every fallible function returns an `int32_t`
//!   ([`types::HomaStatus`]); `0` is success. The codes are fixed for the life
//!   of ABI version 1.
//! * **Last error** - a thread-local record (status, `errno`, UTF-8 message)
//!   set by every failing call; read with `homa_last_error_*`.
//! * **Socket handles** - `homa_socket_t *` is an opaque *key* into a
//!   generation-checked registry, never a pointer that is dereferenced. A
//!   closed or forged handle fails with `HOMA_ERR_INVALID_ARGUMENT`; it
//!   cannot cause a use-after-free. See [`handles`].
//! * **Messages** - `homa_message_t *` owns a copied payload; the pointer
//!   from `homa_message_info` / `homa_message_payload` is stable until
//!   `homa_message_free`. See [`messages`]. Not zero-copy.
//! * **Panic boundary** - socket and message exports run under
//!   [`errors::guard`]; a Rust panic becomes `HOMA_ERR_INTERNAL` instead of
//!   unwinding into C. The last-error accessors do not use `guard` (that
//!   would overwrite the record being read) and instead touch the
//!   thread-local with `try_borrow`, so a conflicting borrow returns the
//!   empty record rather than panicking.
//! * **Threading** - a handle may be used from any number of threads at once.
//!   The kernel's rule remains: only one waiter per private RPC id. A second
//!   waiter gets `HOMA_ERR_INVALID_ARGUMENT` and does not cancel the first.
//!   `homa_socket_close` is safe to call while other threads are inside calls
//!   on the same socket; they return `HOMA_ERR_SHUTDOWN` and the socket is
//!   destroyed when the last of them has returned.
//! * **Synchronous** - all receives block (or poll up to a deadline). There is
//!   no callback, background thread or async runtime.
//!
//! # Platforms
//!
//! Linux x86_64 and aarch64 (64-bit only; enforced at compile time). The
//! crate compiles everywhere so hosts can be developed on macOS/Windows;
//! there `homa_ffi_platform_supported()` is `false` and `homa_socket_open`
//! fails with `HOMA_ERR_UNSUPPORTED_PLATFORM`.

#![cfg_attr(not(target_os = "linux"), allow(rustdoc::broken_intra_doc_links))]

pub mod api;
pub mod errors;
pub mod handles;
pub mod messages;
pub mod types;

pub use api::{EXPORTED_FUNCTIONS, HOMA_FFI_ABI_VERSION};
pub use types::*;

#[cfg(test)]
mod tests {
    #[test]
    fn abi_version_is_one() {
        assert_eq!(super::api::homa_ffi_abi_version(), 1);
        assert_eq!(super::HOMA_FFI_ABI_VERSION, 1);
    }

    #[test]
    fn platform_support_matches_build() {
        assert_eq!(
            super::api::homa_ffi_platform_supported(),
            cfg!(target_os = "linux")
        );
    }
}
