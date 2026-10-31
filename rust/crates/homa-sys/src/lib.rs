//! Low-level bindings to the Linux **Homa** transport kernel module.
//!
//! This crate is the `-sys` layer of HomaStack. It contains:
//!
//! * [`abi`] - `#[repr(C)]` mirrors of the structures and constants declared in
//!   the upstream user-space header `homa.h`, plus the `ioctl` request numbers.
//!   These are platform-independent and are unit-tested for size/offset
//!   compatibility (and, with the `header-check` feature on Linux, compared
//!   against the real header compiled with the system C compiler).
//! * [`region`] - a small RAII wrapper around an anonymous `mmap` region used as
//!   the Homa receive-buffer pool (any Unix).
//! * [`sys`] - thin, documented wrappers around the system calls Homa uses
//!   (`socket`, `bind`, `setsockopt`, `sendmsg`, `recvmsg`, `ioctl`,
//!   `shutdown`, `poll`). Linux only.
//!
//! # Upstream revision
//!
//! Definitions are taken from
//! <https://github.com/PlatformLab/HomaModule> at commit
//! `4bd0275714439da2955670a6c438f6c23304d785` (2026-10-27). A verbatim copy of
//! that `homa.h` lives in `upstream/homa.h` next to this crate. If you update
//! the module, re-vendor the header, re-run the `header-check` tests and update
//! [`abi::UPSTREAM_REVISION`].
//!
//! # Binding strategy
//!
//! The structures are written by hand rather than generated with `bindgen`.
//! `homa.h` is small (five plain-old-data structs, no bitfields, no nested
//! unions except in `homa_rpc_info`), so hand-written definitions avoid a
//! libclang build dependency and keep the crate buildable on non-Linux
//! developer machines. Layout is independently verified in two ways:
//!
//! 1. compile-time/unit assertions of `size_of`/`offset_of` against the values
//!    the C compiler produces for the header (see `tests` in [`abi`]);
//! 2. the optional `header-check` test, which compiles the vendored header with
//!    `cc` and prints the real values for comparison.
//!
//! # Safety
//!
//! All `unsafe` code is confined to [`sys`] and [`region`]. Each unsafe block
//! documents the invariant it relies on. The two invariants that callers must
//! uphold are:
//!
//! * The receive-buffer region handed to the kernel with
//!   [`sys::set_rcvbuf_region`] must stay mapped, writable and *exclusively
//!   reserved for the kernel* until the socket is closed. The kernel writes
//!   incoming message data into it asynchronously.
//! * Buffer pages (`bpage_offsets`) returned by [`sys::recvmsg`] belong to the
//!   application until they are handed back via a later `recvmsg` or
//!   [`sys::release_bpages`]; after that the kernel may overwrite them at any
//!   time.

#![cfg_attr(docsrs, feature(doc_cfg))]
// `sys` only exists on Linux; links to it are valid where the docs matter.
#![cfg_attr(not(target_os = "linux"), allow(rustdoc::broken_intra_doc_links))]

pub mod abi;
#[cfg(unix)]
pub mod region;
#[cfg(target_os = "linux")]
pub mod sys;

pub use abi::*;
