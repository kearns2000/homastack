//! **homa-rs** - a small, safe, synchronous Rust API over the Linux Homa
//! transport kernel module.
//!
//! Homa is a message-oriented, connectionless RPC transport: a client sends a
//! *request* to `host:port`, the kernel assigns an RPC id, the server receives
//! the request together with that id and sends exactly one *response* for it.
//! This crate wraps the kernel's socket interface (see [`homa_sys`]) in owned
//! Rust types that manage the receive-buffer pool and RPC ids correctly.
//!
//! ```no_run
//! # #[cfg(not(target_os = "linux"))]
//! # fn main() {}
//! # #[cfg(target_os = "linux")]
//! # fn main() -> homa_rs::Result<()> {
//! use homa_rs::HomaSocket;
//!
//! // Server: bind a well-known port (1..32767) and echo requests.
//! let server = HomaSocket::bind("0.0.0.0:5000")?;
//! std::thread::spawn(move || {
//!     loop {
//!         let msg = server.recv().expect("recv");
//!         if msg.is_request() {
//!             server.reply(&msg, &msg.payload).expect("reply");
//!         }
//!     }
//! });
//!
//! // Client: an unbound socket gets an automatic port.
//! let client = HomaSocket::open(homa_rs::AddressFamily::V4)?;
//! let response = client.request("127.0.0.1:5000", b"Hello Homa")?;
//! assert_eq!(response.payload, b"Hello Homa");
//! # Ok(()) }
//! ```
//!
//! # Platform support
//!
//! Homa sockets exist only on Linux with the `homa` kernel module loaded.
//! [`HomaSocket`] is therefore compiled only for `target_os = "linux"`. The
//! remaining modules ([`error`], [`addr`], [`buffer`], [`transport`]) are
//! portable and unit-tested everywhere, which keeps `cargo test` green on
//! developer machines without Homa. Use [`is_supported_platform`] to branch
//! at runtime in cross-platform code.
//!
//! # Copying behaviour
//!
//! Received payloads are **copied** out of the kernel-managed receive region
//! into an owned `Vec<u8>` and the underlying buffer pages are recycled to the
//! kernel on the next receive. This is deliberate for Phase 1: it makes
//! ownership trivial and provably safe. Zero-copy access is not implemented
//! and is not claimed.
//!
//! # Threading and async
//!
//! All [`HomaSocket`] methods take `&self`; the type is `Send + Sync` and a
//! single socket can be shared between threads (e.g. one thread blocked in
//! [`HomaSocket::recv`] while another calls [`HomaSocket::request`]). One
//! restriction comes from the kernel: **at most one thread may wait on a
//! given private RPC id at a time** (`homa_interest_init_private` fails a
//! second waiter with `EINVAL`). Different ids, and any number of shared
//! receivers, may be waited on concurrently. There is no async runtime. The non-blocking primitives ([`HomaSocket::try_recv`],
//! [`HomaSocket::poll_readable`]) and the fd-based design are the hooks a
//! future async layer would build on without changing these types.

#![cfg_attr(docsrs, feature(doc_cfg))]
// `socket` only exists on Linux; links to it are valid where the docs matter.
#![cfg_attr(not(target_os = "linux"), allow(rustdoc::broken_intra_doc_links))]

pub mod addr;
pub mod buffer;
pub mod error;
pub mod transport;

#[cfg(target_os = "linux")]
pub mod socket;

pub use addr::AddressFamily;
pub use error::{ErrorKind, HomaError, Result};
#[cfg(target_os = "linux")]
pub use socket::{HomaSocket, SocketConfig};
pub use transport::{Message, MessageKind, RpcId};

/// Largest payload accepted by Homa (`HOMA_MAX_MESSAGE_LENGTH`, 1 000 000 bytes).
pub const MAX_MESSAGE_LENGTH: usize = homa_sys::HOMA_MAX_MESSAGE_LENGTH;

/// Size of one receive buffer page (`HOMA_BPAGE_SIZE`, 64 KiB).
pub const BPAGE_SIZE: usize = homa_sys::HOMA_BPAGE_SIZE;

/// `true` when this build can create Homa sockets (Linux). Whether the kernel
/// module is actually loaded is only known at runtime: creating a socket
/// fails with [`ErrorKind::Unavailable`] otherwise.
#[must_use]
pub const fn is_supported_platform() -> bool {
    cfg!(target_os = "linux")
}
