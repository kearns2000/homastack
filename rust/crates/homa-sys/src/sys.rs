//! Thin wrappers around the Linux system calls used with Homa sockets.
//!
//! Each function maps one-to-one onto a kernel entry point documented in the
//! upstream man pages (`homa.7`, `sendmsg.2`, `recvmsg.2`) and verified against
//! `homa_plumbing.c`. Errors are reported as [`std::io::Error`] built from
//! `errno`. No Rust-side retries, buffering or policy live here.
//!
//! Functions that take references are safe: all pointers handed to the kernel
//! are derived from live Rust references for the duration of the call. The
//! only `unsafe fn` is [`set_rcvbuf_region`], because it establishes a
//! long-lived aliasing relationship between the kernel and user memory.

use core::ffi::c_int;
use core::mem::{size_of, zeroed};
use core::ptr;
use std::io::{self, IoSlice};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::time::Duration;

use crate::abi::{
    HOMAIOCABORT, HOMAIOCINFO, HOMAIOCRELEASE, IPPROTO_HOMA, SO_HOMA_RCVBUF, SO_HOMA_SERVER,
    homa_abort_args, homa_info, homa_rcvbuf_args, homa_recvmsg_args, homa_sendmsg_args,
};

/// Outcome of a successful [`recvmsg`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecvOutcome {
    /// Total length of the received message in bytes.
    pub length: usize,
    /// Number of bytes the kernel wrote into the `source` address (`0` if no
    /// address buffer was supplied).
    pub addr_len: libc::socklen_t,
}

#[inline]
fn cvt(rc: libc::c_int) -> io::Result<libc::c_int> {
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc)
    }
}

/// `socket(family, SOCK_DGRAM | SOCK_CLOEXEC, IPPROTO_HOMA)`.
///
/// `family` must be `AF_INET` or `AF_INET6`. `EPROTONOSUPPORT` means the Homa
/// kernel module is not loaded.
pub fn socket(family: c_int) -> io::Result<OwnedFd> {
    // SAFETY: plain syscall with integer arguments.
    let fd =
        cvt(unsafe { libc::socket(family, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, IPPROTO_HOMA) })?;
    // SAFETY: `fd` is a freshly created descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// `bind(fd, addr, len)`.
///
/// Homa only honours the port: it must be in `1..HOMA_MIN_DEFAULT_PORT`
/// (`EINVAL` otherwise, `EADDRINUSE` if taken). Port `0` is a no-op. The
/// address family must match the socket's (`EAFNOSUPPORT`).
pub fn bind(
    fd: BorrowedFd<'_>,
    addr: &libc::sockaddr_storage,
    len: libc::socklen_t,
) -> io::Result<()> {
    // SAFETY: `addr` is a live reference to at least `len` bytes (callers pass
    // the length of the sockaddr_in/in6 stored inside the storage).
    cvt(unsafe {
        libc::bind(
            fd.as_raw_fd(),
            ptr::from_ref(addr).cast::<libc::sockaddr>(),
            len,
        )
    })?;
    Ok(())
}

/// `setsockopt(fd, IPPROTO_HOMA, SO_HOMA_RCVBUF, {start, length})`.
///
/// Registers the receive-buffer region. Must be called exactly once per
/// socket before the first `recvmsg`.
///
/// # Safety
///
/// After this call the kernel will write incoming message data anywhere in
/// `[start, start + length)` at arbitrary times, for as long as the socket is
/// open. The caller must guarantee that:
///
/// * the region is page aligned, mapped and writable (`EINVAL`/`EFAULT`
///   otherwise - the kernel probes the first byte);
/// * no Rust object other than the receive-buffer abstraction ever lives in
///   or borrows from that memory;
/// * the mapping outlives the socket (close the socket *before* unmapping).
pub unsafe fn set_rcvbuf_region(
    fd: BorrowedFd<'_>,
    start: *mut u8,
    length: usize,
) -> io::Result<()> {
    let args = homa_rcvbuf_args {
        start: start as u64,
        length: length as u64,
    };
    // SAFETY: `args` is a live local; `optlen` matches its size exactly as the
    // kernel requires.
    cvt(unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            IPPROTO_HOMA,
            SO_HOMA_RCVBUF,
            ptr::from_ref(&args).cast(),
            size_of::<homa_rcvbuf_args>() as libc::socklen_t,
        )
    })?;
    Ok(())
}

/// `getsockopt(fd, IPPROTO_HOMA, SO_HOMA_RCVBUF)`: the registered region.
pub fn get_rcvbuf_region(fd: BorrowedFd<'_>) -> io::Result<homa_rcvbuf_args> {
    let mut args = homa_rcvbuf_args::default();
    let mut len = size_of::<homa_rcvbuf_args>() as libc::socklen_t;
    // SAFETY: `args` and `len` are live locals of the advertised size.
    cvt(unsafe {
        libc::getsockopt(
            fd.as_raw_fd(),
            IPPROTO_HOMA,
            SO_HOMA_RCVBUF,
            ptr::from_mut(&mut args).cast(),
            &raw mut len,
        )
    })?;
    Ok(args)
}

/// `setsockopt(fd, IPPROTO_HOMA, SO_HOMA_SERVER, int)`.
pub fn set_server(fd: BorrowedFd<'_>, enabled: bool) -> io::Result<()> {
    let value: c_int = c_int::from(enabled);
    // SAFETY: `value` is a live local; optlen is sizeof(int) as required.
    cvt(unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            IPPROTO_HOMA,
            SO_HOMA_SERVER,
            ptr::from_ref(&value).cast(),
            size_of::<c_int>() as libc::socklen_t,
        )
    })?;
    Ok(())
}

/// `getsockopt(fd, IPPROTO_HOMA, SO_HOMA_SERVER)`.
pub fn get_server(fd: BorrowedFd<'_>) -> io::Result<bool> {
    let mut value: c_int = 0;
    let mut len = size_of::<c_int>() as libc::socklen_t;
    // SAFETY: `value` and `len` are live locals of the advertised size.
    cvt(unsafe {
        libc::getsockopt(
            fd.as_raw_fd(),
            IPPROTO_HOMA,
            SO_HOMA_SERVER,
            ptr::from_mut(&mut value).cast(),
            &raw mut len,
        )
    })?;
    Ok(value != 0)
}

/// `sendmsg(fd, msghdr, flags)` with Homa's control block.
///
/// * `args.id == 0` sends a request; on success `args.id` holds the new RPC id.
/// * `args.id != 0` sends the response for that server-side id
///   (`completion_cookie` must be `0`). Replying to an RPC that no longer
///   exists succeeds silently.
///
/// `dest_len` must be `sizeof(sockaddr_in)` or `sizeof(sockaddr_in6)` matching
/// the socket family. `iov` must contain between 1 and
/// `HOMA_MAX_MESSAGE_LENGTH` bytes in total. `flags` is normally `0`;
/// `MSG_DONTWAIT` turns a wait for transmit memory into `EAGAIN`.
///
/// Per upstream, `msg_controllen` is set to **0** (the kernel reads the
/// structure through the raw `msg_control` pointer) and the return value is
/// `0` on success rather than a byte count.
pub fn sendmsg(
    fd: BorrowedFd<'_>,
    dest: &libc::sockaddr_storage,
    dest_len: libc::socklen_t,
    iov: &[IoSlice<'_>],
    args: &mut homa_sendmsg_args,
    flags: c_int,
) -> io::Result<()> {
    // SAFETY: msghdr is plain-old-data; zeroing yields a valid "empty" header
    // (also covers musl's padding fields).
    let mut hdr: libc::msghdr = unsafe { zeroed() };
    hdr.msg_name = ptr::from_ref(dest).cast_mut().cast();
    hdr.msg_namelen = dest_len;
    // `IoSlice` is guaranteed ABI-compatible with `iovec`.
    hdr.msg_iov = iov.as_ptr().cast_mut().cast::<libc::iovec>();
    hdr.msg_iovlen = iov.len() as _;
    hdr.msg_control = ptr::from_mut(args).cast();
    hdr.msg_controllen = 0; // Required by Homa; see module docs.
    hdr.msg_flags = 0;

    // SAFETY: every pointer in `hdr` is derived from a reference that is live
    // for the whole call: `dest`, `iov` (and the buffers it borrows), `args`.
    // The kernel writes back only into `*args` (sizeof(homa_sendmsg_args)).
    let rc = unsafe { libc::sendmsg(fd.as_raw_fd(), &raw const hdr, flags) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `recvmsg(fd, msghdr, flags)` with Homa's control block.
///
/// On entry `args.id` selects what to wait for and
/// `args.num_bpages`/`args.bpage_offsets` list bpages being returned to the
/// kernel. On success the same fields describe the received message and the
/// kernel has written message bytes into the registered region. `source`, if
/// given, receives the sender's `sockaddr_in`/`sockaddr_in6`.
///
/// On error, `args` is still written back by the kernel (`homa_recvmsg` in
/// `homa_plumbing.c`, pinned revision):
///
/// * `args.num_bpages` is **always 0**: the kernel zeroes it before checking
///   the result of `homa_pool_free_bufs`. The vendored `recvmsg.2` says the
///   input offsets are echoed back when the free was not performed; that is
///   *not* what this revision does. If the free fails (`EINVAL`, `error_msg`
///   "error while releasing buffer pages") an unknown prefix of the batch
///   was freed and the rest is unrecoverable until `close`.
/// * `args.id`: after a wait that ended with an RPC-level failure
///   (`ETIMEDOUT`, `ENOTCONN`, `EHOSTUNREACH`, ...) it is the failed RPC's
///   id and the kernel has already ended that RPC. On `EINTR`/`EAGAIN`
///   during a private wait it is 0. On the pre-wait validation failures
///   (`EINVAL`/`EFAULT`) it is the caller's *input* id, unchanged.
///
/// `flags`: `0` or `MSG_DONTWAIT` (`EAGAIN` if nothing is ready).
pub fn recvmsg(
    fd: BorrowedFd<'_>,
    source: Option<&mut libc::sockaddr_storage>,
    args: &mut homa_recvmsg_args,
    flags: c_int,
) -> io::Result<RecvOutcome> {
    args.reserved = 0;
    // SAFETY: see `sendmsg`.
    let mut hdr: libc::msghdr = unsafe { zeroed() };
    if let Some(src) = source {
        hdr.msg_name = ptr::from_mut(src).cast();
        hdr.msg_namelen = size_of::<libc::sockaddr_storage>() as libc::socklen_t;
    }
    hdr.msg_control = ptr::from_mut(args).cast();
    hdr.msg_controllen = size_of::<homa_recvmsg_args>() as _; // Must be exact.

    // SAFETY: `hdr` points only at `source` and `args`, both live for the
    // call. Message payload is written into the region registered with
    // `set_rcvbuf_region`, whose validity the caller guaranteed there.
    let rc = unsafe { libc::recvmsg(fd.as_raw_fd(), &raw mut hdr, flags) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(RecvOutcome {
        length: rc as usize,
        addr_len: hdr.msg_namelen,
    })
}

/// `ioctl(fd, HOMAIOCRELEASE, args)`: return `args.num_bpages` bpages to the
/// kernel without receiving. Only `num_bpages`, `bpage_offsets` and
/// `reserved` (must be 0) are read.
pub fn release_bpages(fd: BorrowedFd<'_>, args: &homa_recvmsg_args) -> io::Result<()> {
    if args.num_bpages == 0 {
        return Ok(());
    }
    // SAFETY: `args` is a live reference to a complete homa_recvmsg_args; the
    // kernel only reads from it for this request.
    cvt(unsafe { libc::ioctl(fd.as_raw_fd(), HOMAIOCRELEASE as _, ptr::from_ref(args)) })?;
    Ok(())
}

/// `ioctl(fd, HOMAIOCINFO, info)` with no per-RPC detail requested.
///
/// Useful for the socket's port and for `error_msg`, which explains the last
/// `EINVAL`/`EFAULT` returned by Homa on this socket.
pub fn info(fd: BorrowedFd<'_>) -> io::Result<homa_info> {
    let mut info = homa_info::default();
    // SAFETY: `info` is a live, fully initialised local; rpc_info == 0 tells
    // the kernel not to write per-RPC records anywhere else.
    cvt(unsafe { libc::ioctl(fd.as_raw_fd(), HOMAIOCINFO as _, ptr::from_mut(&mut info)) })?;
    Ok(info)
}

/// `ioctl(fd, HOMAIOCABORT, {id, error})`.
///
/// `id == 0` aborts every outgoing RPC on the socket. `error == 0` frees the
/// RPC(s) silently; nonzero completes them with that errno so a subsequent
/// `recvmsg` reports it. Only client-side RPCs can be aborted. Not available
/// on the stripped (upstream-kernel) variant of the module (`EINVAL`).
pub fn abort(fd: BorrowedFd<'_>, id: u64, error: u32) -> io::Result<()> {
    let args = homa_abort_args {
        id,
        error,
        _pad1: 0,
        _pad2: [0; 2],
    };
    // SAFETY: `args` is a live local; the kernel reads it and may write it back.
    cvt(unsafe { libc::ioctl(fd.as_raw_fd(), HOMAIOCABORT as _, ptr::from_ref(&args)) })?;
    Ok(())
}

/// `shutdown(fd, SHUT_RDWR)`. Homa ignores `how`: the socket is disabled for
/// both directions and blocked callers fail with `ESHUTDOWN`.
pub fn shutdown(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: plain syscall with integer arguments.
    cvt(unsafe { libc::shutdown(fd.as_raw_fd(), libc::SHUT_RDWR) })?;
    Ok(())
}

/// `poll(fd, POLLIN, timeout)`.
///
/// Returns `Ok(true)` when the socket is readable (a shared message is queued
/// or the socket was shut down), `Ok(false)` on timeout. `None` waits forever.
/// Private responses never make the socket readable (upstream limitation).
pub fn poll_readable(fd: BorrowedFd<'_>, timeout: Option<Duration>) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let ms: c_int = match timeout {
        None => -1,
        Some(d) => c_int::try_from(d.as_millis()).unwrap_or(c_int::MAX),
    };
    // SAFETY: `pfd` is a live local array of length 1.
    let rc = cvt(unsafe { libc::poll(&raw mut pfd, 1, ms) })?;
    Ok(rc > 0)
}
