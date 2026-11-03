//! [`HomaSocket`]: an owned Homa socket with its receive-buffer pool.
//!
//! Linux only. See the crate docs for the overall model; this module maps the
//! public API onto the verified kernel interface:
//!
//! | API                                   | Kernel call(s)                                   |
//! |---------------------------------------|--------------------------------------------------|
//! | [`HomaSocket::bind`] / [`HomaSocket::open`] | `socket`, `bind`, `setsockopt(SO_HOMA_RCVBUF)` |
//! | [`HomaSocket::send_request`]          | `sendmsg` with `id == 0`                         |
//! | [`HomaSocket::recv_response`]         | `recvmsg` with `id == <rpc>`                     |
//! | [`HomaSocket::recv`]                  | `recvmsg` with `id == 0`                         |
//! | [`HomaSocket::reply`]                 | `sendmsg` with `id == <odd rpc id>`              |
//! | [`HomaSocket::release_pending`]       | `ioctl(HOMAIOCRELEASE)`                          |
//! | [`HomaSocket::abort`]                 | `ioctl(HOMAIOCABORT)`                            |
//! | [`HomaSocket::shutdown`]              | `shutdown`                                       |
//! | [`HomaSocket::info`]                  | `ioctl(HOMAIOCINFO)`                             |
//!
//! Buffer pages returned by every successful receive are queued in a
//! [`BpageQueue`] and recycled on the next `recvmsg` (or by
//! [`HomaSocket::release_pending`] / drop). Payloads are copied into owned
//! `Vec<u8>` before the pages are queued, so a [`Message`] never aliases the
//! kernel-managed region.

use std::io::IoSlice;
use std::net::{SocketAddr, ToSocketAddrs};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

use homa_sys::sys;
use homa_sys::{HOMA_MIN_DEFAULT_PORT, HOMA_SENDMSG_PRIVATE, homa_recvmsg_args, homa_sendmsg_args};
use tracing::{debug, trace, warn};

use crate::addr::{AddressFamily, RawSocketAddr, resolve};
use crate::buffer::{BpageQueue, DEFAULT_REGION_BYTES, FragmentLayout, ReceiveBuffer};
use crate::error::{ErrorKind, HomaError, Result};
use crate::transport::{Message, MessageKind, RpcId};

/// Configuration for creating a [`HomaSocket`].
#[derive(Clone, Debug)]
pub struct SocketConfig {
    family: AddressFamily,
    region_bytes: usize,
    server: Option<bool>,
}

impl SocketConfig {
    /// Defaults: 64 MiB receive region, server mode decided by `bind`.
    #[must_use]
    pub fn new(family: AddressFamily) -> Self {
        Self {
            family,
            region_bytes: DEFAULT_REGION_BYTES,
            server: None,
        }
    }

    /// Size of the receive-buffer region in bytes (rounded up to 64 KiB).
    ///
    /// The kernel rejects regions smaller than
    /// `nr_cpu_ids × 64 KiB + 2 MB` with `EINVAL` ("buffer pool is not large
    /// enough") and larger than 4 GiB.
    #[must_use]
    pub fn receive_region_bytes(mut self, bytes: usize) -> Self {
        self.region_bytes = bytes;
        self
    }

    /// Explicitly enable/disable incoming requests (`SO_HOMA_SERVER`).
    /// Binding a port enables it implicitly; use this to accept requests on an
    /// auto-assigned port or to turn requests off on a bound socket.
    #[must_use]
    pub fn server(mut self, enabled: bool) -> Self {
        self.server = Some(enabled);
        self
    }

    /// Create a socket bound to `addr`'s port (the IP part is ignored by Homa;
    /// the socket receives on all interfaces).
    pub fn bind<A: ToSocketAddrs>(self, addr: A) -> Result<HomaSocket> {
        let addr = resolve(addr, Some(self.family))?;
        HomaSocket::create(&self, Some(addr))
    }

    /// Create an unbound socket with an automatically assigned port
    /// (`>= 32768`). Suitable for clients.
    pub fn open(self) -> Result<HomaSocket> {
        HomaSocket::create(&self, None)
    }
}

/// Point-in-time status of a socket from `HOMAIOCINFO`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketInfo {
    /// Port number owned by the socket.
    pub port: u16,
    /// Active RPCs (client and server side).
    pub active_rpcs: u32,
    /// Free bytes in the receive-buffer pool.
    pub buffer_available_bytes: u64,
    /// Homa's description of the last error on this socket (may be empty).
    pub last_error: String,
}

/// A Homa socket plus the receive-buffer region registered with it.
///
/// Dropping the socket returns any outstanding buffer pages, closes the file
/// descriptor and only then unmaps the region (field order below is the drop
/// order and is load-bearing).
#[derive(Debug)]
pub struct HomaSocket {
    fd: OwnedFd,
    buffer: ReceiveBuffer,
    pending: BpageQueue,
    family: AddressFamily,
    local_port: u16,
}

impl HomaSocket {
    /// Create a socket bound to the port in `addr` with default settings.
    ///
    /// Server ports must be in `1..=32767`. Port `0` is accepted and yields an
    /// auto-assigned client port (identical to [`HomaSocket::open`]). The
    /// address family is taken from `addr`.
    pub fn bind<A: ToSocketAddrs>(addr: A) -> Result<Self> {
        let addr = resolve(addr, None)?;
        SocketConfig::new(AddressFamily::of(&addr)).bind(addr)
    }

    /// Create an unbound client socket with default settings.
    pub fn open(family: AddressFamily) -> Result<Self> {
        SocketConfig::new(family).open()
    }

    /// Start building a socket with custom settings.
    #[must_use]
    pub fn config(family: AddressFamily) -> SocketConfig {
        SocketConfig::new(family)
    }

    fn create(config: &SocketConfig, bind_addr: Option<SocketAddr>) -> Result<Self> {
        if let Some(addr) = bind_addr {
            let port = addr.port();
            if port != 0 && port >= HOMA_MIN_DEFAULT_PORT {
                return Err(HomaError::InvalidPort(port));
            }
        }

        // 1. Map the receive region first so a failure here costs nothing.
        //    Locals drop in reverse order: on any early return below `fd` is
        //    closed before `buffer` is unmapped.
        let buffer = ReceiveBuffer::new(config.region_bytes)?;

        // 2. socket(AF_*, SOCK_DGRAM, IPPROTO_HOMA)
        let fd = sys::socket(config.family.to_raw()).map_err(|e| HomaError::os("socket", e))?;

        // 3. bind (optional)
        if let Some(addr) = bind_addr.filter(|a| a.port() != 0) {
            let raw = RawSocketAddr::from_socket_addr(&addr);
            sys::bind(fd.as_fd(), raw.as_storage(), raw.len())
                .map_err(|e| os_err_fd(fd.as_fd(), "bind", e))?;
        }

        // 4. Register the receive region.
        //
        // SAFETY: `buffer` is a page-aligned writable mapping that will be
        // owned by the returned `HomaSocket` and dropped *after* `fd` (field
        // order), so the kernel never holds a reference to unmapped memory.
        // No other code creates references into the region: reads go through
        // `ReceiveBuffer::copy_out`, only for pages the kernel handed to us.
        unsafe { sys::set_rcvbuf_region(fd.as_fd(), buffer.as_ptr(), buffer.len()) }
            .map_err(|e| os_err_fd(fd.as_fd(), "setsockopt(SO_HOMA_RCVBUF)", e))?;

        // 5. Server flag (optional).
        if let Some(enabled) = config.server {
            sys::set_server(fd.as_fd(), enabled)
                .map_err(|e| os_err_fd(fd.as_fd(), "setsockopt(SO_HOMA_SERVER)", e))?;
        }

        // 6. Learn the actual port (auto-assigned when unbound).
        let local_port = match sys::info(fd.as_fd()) {
            Ok(info) => info.port as u16,
            Err(e) => {
                warn!(error = %e, "HOMAIOCINFO failed; local port unknown");
                bind_addr.map_or(0, |a| a.port())
            }
        };

        let socket = Self {
            fd,
            buffer,
            pending: BpageQueue::new(),
            family: config.family,
            local_port,
        };
        debug!(
            fd = socket.fd.as_raw_fd(),
            family = ?socket.family,
            port = socket.local_port,
            region_bytes = socket.buffer.len(),
            bpages = socket.buffer.bpage_count(),
            "homa socket created"
        );
        Ok(socket)
    }

    // ------------------------------------------------------------------
    // Introspection
    // ------------------------------------------------------------------

    /// Address family the socket was created with.
    #[must_use]
    pub fn family(&self) -> AddressFamily {
        self.family
    }

    /// Homa port owned by this socket (bound or auto-assigned).
    #[must_use]
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// Size of the registered receive region in bytes.
    #[must_use]
    pub fn receive_region_len(&self) -> usize {
        self.buffer.len()
    }

    /// Number of buffer pages currently owned by the application and not yet
    /// returned to the kernel.
    #[must_use]
    pub fn pending_bpages(&self) -> usize {
        self.pending.len()
    }

    /// Query `HOMAIOCINFO`.
    pub fn info(&self) -> Result<SocketInfo> {
        let info =
            sys::info(self.fd.as_fd()).map_err(|e| HomaError::os("ioctl(HOMAIOCINFO)", e))?;
        Ok(SocketInfo {
            port: info.port as u16,
            active_rpcs: info.num_rpcs,
            buffer_available_bytes: info.bpool_avail_bytes,
            last_error: info.error_message(),
        })
    }

    /// Homa's explanation of the most recent error on this socket, if the
    /// kernel can be asked (`None` after shutdown or on failure).
    #[must_use]
    pub fn last_error_message(&self) -> Option<String> {
        last_error_message_fd(self.fd.as_fd())
    }

    /// Whether incoming requests are accepted (`SO_HOMA_SERVER`).
    pub fn is_server(&self) -> Result<bool> {
        sys::get_server(self.fd.as_fd()).map_err(|e| self.os_err("getsockopt(SO_HOMA_SERVER)", e))
    }

    /// Enable or disable incoming requests (`SO_HOMA_SERVER`).
    pub fn set_server(&self, enabled: bool) -> Result<()> {
        sys::set_server(self.fd.as_fd(), enabled)
            .map_err(|e| self.os_err("setsockopt(SO_HOMA_SERVER)", e))
    }

    // ------------------------------------------------------------------
    // Client side
    // ------------------------------------------------------------------

    /// Send a request and return its RPC id without waiting for the response.
    ///
    /// `completion_cookie` is an opaque value echoed back with the response.
    /// If `private` is `true` the response can only be collected with
    /// [`HomaSocket::recv_response`] for this id; otherwise it is delivered by
    /// [`HomaSocket::recv`] like any shared message (and makes the socket
    /// readable for [`HomaSocket::poll_readable`]).
    pub fn send_request<A: ToSocketAddrs>(
        &self,
        dest: A,
        payload: &[u8],
        completion_cookie: u64,
        private: bool,
    ) -> Result<RpcId> {
        let dest = resolve(dest, Some(self.family))?;
        Self::check_payload(payload)?;
        let raw = RawSocketAddr::from_socket_addr(&dest);
        let mut args = homa_sendmsg_args {
            id: 0,
            completion_cookie,
            flags: if private { HOMA_SENDMSG_PRIVATE } else { 0 },
            reserved: 0,
        };
        sys::sendmsg(
            self.fd.as_fd(),
            raw.as_storage(),
            raw.len(),
            &[IoSlice::new(payload)],
            &mut args,
            0,
        )
        .map_err(|e| self.os_err("sendmsg(request)", e))?;
        let id = RpcId::from_raw(args.id);
        trace!(%id, %dest, len = payload.len(), private, "request sent");
        Ok(id)
    }

    /// Block until the response for private RPC `id` arrives.
    ///
    /// `id` must come from a request sent with `private = true`; the kernel
    /// rejects waiting on a non-private RPC by id with `EINVAL`
    /// (`homa_wait_private`). Fails with [`HomaError::RpcFailed`] if the
    /// kernel abandoned the RPC (`ETIMEDOUT`, `ENOTCONN` when no server
    /// listens on the port, `EHOSTUNREACH`, ...).
    ///
    /// A signal interrupting the wait (`EINTR`), and a spurious `EAGAIN`,
    /// do **not** end the RPC in the kernel and a private response is never
    /// delivered through [`HomaSocket::recv`], so both are retried. `EAGAIN`
    /// is not expected on a blocking wait (the pinned kernel returns it only
    /// for `MSG_DONTWAIT`); retrying it, with a short back-off, keeps a
    /// sticky `EAGAIN` from aborting a live RPC or spinning a core. If the
    /// wait fails for another reason that leaves the RPC alive *and* this
    /// thread was the waiter, it is aborted before the error is returned.
    ///
    /// Only one thread may wait on a given private id at a time. The kernel
    /// fails a concurrent second waiter with `EINVAL` and does not end the
    /// RPC; this method returns that error **without** aborting, so the
    /// first waiter's response is not cancelled. The same applies to an even
    /// id that was not sent as a private request.
    pub fn recv_response(&self, id: RpcId) -> Result<Message> {
        Self::check_private_id(id)?;
        let mut backoff = Duration::from_micros(20);
        loop {
            match self.recv_raw(id.as_raw(), 0) {
                Err(e) if matches!(e.kind(), ErrorKind::Interrupted | ErrorKind::WouldBlock) => {
                    trace!(%id, kind = ?e.kind(), "private wait not finished; retrying");
                    if e.kind() == ErrorKind::WouldBlock {
                        std::thread::sleep(backoff);
                        backoff = (backoff * 2).min(Duration::from_millis(1));
                    }
                }
                Err(e) => return Err(self.finish_failed_private_wait(id, e)),
                Ok(msg) => {
                    debug_assert_eq!(msg.id, id);
                    return Ok(msg);
                }
            }
        }
    }

    /// Like [`HomaSocket::recv_response`] but gives up after `timeout`.
    ///
    /// Homa does not honour `SO_RCVTIMEO` and private responses do not wake
    /// `poll(2)`, so this polls with `MSG_DONTWAIT` and a short exponential
    /// back-off (20 µs → 1 ms). On expiry the RPC is aborted
    /// (`HOMAIOCABORT`, `error = 0`) so the kernel frees its state and a late
    /// response is discarded, and [`HomaError::Timeout`] is returned.
    /// `EINTR` is retried until the deadline, as in
    /// [`HomaSocket::recv_response`].
    pub fn recv_response_timeout(&self, id: RpcId, timeout: Duration) -> Result<Message> {
        Self::check_private_id(id)?;
        let deadline = Instant::now() + timeout;
        let mut backoff = Duration::from_micros(20);
        loop {
            match self.recv_raw(id.as_raw(), libc::MSG_DONTWAIT) {
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    let now = Instant::now();
                    if now >= deadline {
                        self.abort_quietly(id, "timeout");
                        return Err(HomaError::Timeout(timeout));
                    }
                    std::thread::sleep(backoff.min(deadline - now));
                    backoff = (backoff * 2).min(Duration::from_millis(1));
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {
                    trace!(%id, "private wait interrupted; retrying");
                }
                Err(e) => return Err(self.finish_failed_private_wait(id, e)),
                Ok(msg) => {
                    debug_assert_eq!(msg.id, id);
                    return Ok(msg);
                }
            }
        }
    }

    /// Reject ids that cannot name a private client RPC.
    fn check_private_id(id: RpcId) -> Result<()> {
        if !id.is_valid() {
            return Err(HomaError::InvalidRpcId(id));
        }
        if !id.is_client_side() {
            return Err(HomaError::UnexpectedMessage {
                expected: MessageKind::Response,
                actual: MessageKind::Request,
                id,
            });
        }
        Ok(())
    }

    /// A private wait ended with `err` and the caller is giving up on `id`.
    /// Abort only when [`HomaError::private_wait_should_abort`] says this
    /// thread may still own a live RPC.
    fn finish_failed_private_wait(&self, id: RpcId, err: HomaError) -> HomaError {
        if err.private_wait_should_abort() {
            self.abort_quietly(id, "failed wait");
        }
        err
    }

    fn abort_quietly(&self, id: RpcId, why: &'static str) {
        if let Err(abort_err) = sys::abort(self.fd.as_fd(), id.as_raw(), 0) {
            debug!(%id, why, error = %abort_err, "abort of private RPC failed");
        }
    }

    /// Send `payload` to `dest` as a private request and block for the reply.
    pub fn request<A: ToSocketAddrs>(&self, dest: A, payload: &[u8]) -> Result<Message> {
        let id = self.send_request(dest, payload, 0, true)?;
        self.recv_response(id)
    }

    /// [`HomaSocket::request`] with a deadline (see
    /// [`HomaSocket::recv_response_timeout`] for semantics).
    pub fn request_timeout<A: ToSocketAddrs>(
        &self,
        dest: A,
        payload: &[u8],
        timeout: Duration,
    ) -> Result<Message> {
        let id = self.send_request(dest, payload, 0, true)?;
        self.recv_response_timeout(id, timeout)
    }

    /// Abort outgoing RPC `id` (or all outgoing RPCs if `None`). The RPC is
    /// freed silently; any later response is discarded.
    pub fn abort(&self, id: Option<RpcId>) -> Result<()> {
        sys::abort(self.fd.as_fd(), id.map_or(0, RpcId::as_raw), 0)
            .map_err(|e| self.os_err("ioctl(HOMAIOCABORT)", e))
    }

    // ------------------------------------------------------------------
    // Server side / shared receive
    // ------------------------------------------------------------------

    /// Block until any shared message arrives: an incoming request, or the
    /// response to a non-private request.
    pub fn recv(&self) -> Result<Message> {
        self.recv_raw(0, 0)
    }

    /// Non-blocking [`HomaSocket::recv`]: `Ok(None)` if nothing is queued.
    pub fn try_recv(&self) -> Result<Option<Message>> {
        match self.recv_raw(0, libc::MSG_DONTWAIT) {
            Ok(m) => Ok(Some(m)),
            Err(e) if e.kind() == ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// [`HomaSocket::recv`] with a deadline: `Ok(None)` on timeout.
    ///
    /// Implemented with `poll(2)` (which Homa supports for shared messages)
    /// followed by a non-blocking receive, so it does not busy-wait.
    /// `poll` takes whole milliseconds and may wake before the requested
    /// timeout, so a short wake is not itself the deadline.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<Message>> {
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let remaining = deadline - now;
            if remaining.as_millis() == 0 {
                std::thread::sleep(remaining);
                continue;
            }
            if !self.poll_readable(Some(remaining))? {
                continue;
            }
            // Another thread may have consumed the message between poll and
            // recv; loop in that case.
            if let Some(m) = self.try_recv()? {
                return Ok(Some(m));
            }
        }
    }

    /// Wait until a shared message is queued (or the socket is shut down).
    /// `None` waits indefinitely. Private responses never trigger readiness.
    pub fn poll_readable(&self, timeout: Option<Duration>) -> Result<bool> {
        loop {
            match sys::poll_readable(self.fd.as_fd(), timeout) {
                Ok(ready) => return Ok(ready),
                Err(e) if e.raw_os_error() == Some(libc::EINTR) => {}
                Err(e) => return Err(HomaError::os("poll", e)),
            }
        }
    }

    /// Send the response for `request`.
    pub fn reply(&self, request: &Message, payload: &[u8]) -> Result<()> {
        self.reply_to(request.id, request.peer, payload)
    }

    /// Send the response for RPC `id` (odd, as received) to `peer`.
    ///
    /// Replying to an RPC the kernel no longer knows (client gave up) succeeds
    /// silently, matching upstream semantics.
    pub fn reply_to(&self, id: RpcId, peer: SocketAddr, payload: &[u8]) -> Result<()> {
        if !id.is_server_side() {
            return Err(HomaError::UnexpectedMessage {
                expected: MessageKind::Request,
                actual: MessageKind::Response,
                id,
            });
        }
        if AddressFamily::of(&peer) != self.family {
            return Err(HomaError::AddressFamilyMismatch {
                socket: self.family,
                address: AddressFamily::of(&peer),
            });
        }
        Self::check_payload(payload)?;
        let raw = RawSocketAddr::from_socket_addr(&peer);
        let mut args = homa_sendmsg_args {
            id: id.as_raw(),
            completion_cookie: 0,
            flags: 0,
            reserved: 0,
        };
        sys::sendmsg(
            self.fd.as_fd(),
            raw.as_storage(),
            raw.len(),
            &[IoSlice::new(payload)],
            &mut args,
            0,
        )
        .map_err(|e| self.os_err("sendmsg(response)", e))?;
        trace!(%id, %peer, len = payload.len(), "response sent");
        Ok(())
    }

    // ------------------------------------------------------------------
    // Lifecycle
    // ------------------------------------------------------------------

    /// Return all application-owned buffer pages to the kernel now
    /// (`HOMAIOCRELEASE`) instead of on the next receive. Returns the number
    /// of pages released.
    pub fn release_pending(&self) -> Result<usize> {
        let mut released = 0;
        let mut batches = self.pending.drain_batches().into_iter();
        for batch in batches.by_ref() {
            if let Err(e) = sys::release_bpages(self.fd.as_fd(), &batch) {
                // Re-queue the failed batch *and* every batch not yet
                // attempted: the kernel still considers all of them ours.
                self.pending.push(batch.offsets());
                for rest in batches {
                    self.pending.push(rest.offsets());
                }
                return Err(self.os_err("ioctl(HOMAIOCRELEASE)", e));
            }
            released += batch.num_bpages as usize;
        }
        Ok(released)
    }

    /// Disable the socket. Threads blocked in receive fail with
    /// [`ErrorKind::Shutdown`]; further sends/receives are rejected.
    pub fn shutdown(&self) -> Result<()> {
        sys::shutdown(self.fd.as_fd()).map_err(|e| HomaError::os("shutdown", e))
    }

    // ------------------------------------------------------------------
    // Internals
    // ------------------------------------------------------------------

    fn check_payload(payload: &[u8]) -> Result<()> {
        if payload.is_empty() {
            return Err(HomaError::EmptyMessage);
        }
        if payload.len() > crate::MAX_MESSAGE_LENGTH {
            return Err(HomaError::MessageTooLarge {
                len: payload.len(),
                max: crate::MAX_MESSAGE_LENGTH,
            });
        }
        Ok(())
    }

    fn os_err(&self, op: &'static str, source: std::io::Error) -> HomaError {
        os_err_fd(self.fd.as_fd(), op, source)
    }

    /// The one place `recvmsg` is called. Handles bpage recycling on every
    /// path, address decoding, layout validation and the payload copy.
    fn recv_raw(&self, id: u64, flags: libc::c_int) -> Result<Message> {
        let mut args = homa_recvmsg_args::zeroed();
        args.id = id;
        let recycled = self.pending.load_into(&mut args);

        let mut source = RawSocketAddr::empty();
        let outcome = sys::recvmsg(
            self.fd.as_fd(),
            Some(source.as_storage_mut()),
            &mut args,
            flags,
        );

        let outcome = match outcome {
            Ok(o) => o,
            Err(source_err) => {
                // recvmsg.2 says the input offsets are echoed back in
                // `num_bpages` when the kernel did not free them. The pinned
                // kernel (homa_plumbing.c, homa_recvmsg) actually zeroes
                // `num_bpages` *before* checking the result of
                // `homa_pool_free_bufs`, so this branch is defensive for other
                // revisions; on this one `num_bpages` is always 0 here.
                if args.num_bpages != 0 {
                    self.pending.push(args.offsets());
                } else if recycled != 0 {
                    trace!(recycled, "bpages recycled despite recvmsg error");
                }
                let errno = source_err.raw_os_error();
                let detail = match errno {
                    Some(libc::EAGAIN | libc::EINTR) => None,
                    _ => self.last_error_message(),
                };
                // A failed free (`EINVAL`, error_msg "error while releasing
                // buffer pages") means our queue handed the kernel an offset
                // it did not recognise or had already freed. The kernel
                // released an unknown prefix of the batch and reports 0, so
                // the remainder cannot be recovered until the socket closes.
                // This only happens on a bug in this crate's bookkeeping.
                if recycled != 0
                    && errno == Some(libc::EINVAL)
                    && detail
                        .as_deref()
                        .is_some_and(|m| m.contains("releasing buffer pages"))
                {
                    warn!(
                        recycled,
                        offsets = ?&args.bpage_offsets[..recycled],
                        "kernel rejected a bpage release; some pages are now unrecoverable"
                    );
                }
                // The kernel writes `id` back as the failed RPC's id only
                // after a wait (RPC-level failure, `homa_rpc_end` already
                // run). On the pre-wait checks (`EINVAL`/`EFAULT`: bad
                // control block, no buffer pool, failed free, unknown id) it
                // echoes the *input* id, which must not be reported as an RPC
                // failure; `EINTR`/`EAGAIN` during a private wait come back
                // with `id == 0`.
                let rpc_level = args.id != 0 && !matches!(errno, Some(libc::EINVAL | libc::EFAULT));
                if rpc_level {
                    return Err(HomaError::RpcFailed {
                        id: RpcId::from_raw(args.id),
                        completion_cookie: args.completion_cookie,
                        source: source_err,
                        detail,
                    });
                }
                return Err(HomaError::os("recvmsg", source_err).with_detail(detail));
            }
        };

        // From here on the bpages in `args` belong to us and must be queued
        // for return on every path.
        let layout = FragmentLayout::from_recv_args(&args, outcome.length, self.buffer.len());
        let layout = match layout {
            Ok(l) => l,
            Err(e) => {
                self.pending.push(args.offsets());
                return Err(e);
            }
        };
        let payload = self.buffer.copy_out(&layout);
        self.pending.push(args.offsets());

        source.set_len(outcome.addr_len);
        let peer = source.to_socket_addr()?;
        let msg = Message {
            id: RpcId::from_raw(args.id),
            peer,
            completion_cookie: args.completion_cookie,
            payload,
            fragments: layout.fragments().len(),
        };
        trace!(id = %msg.id, kind = ?msg.kind(), %peer, len = msg.len(), fragments = msg.fragments, recycled, "message received");
        Ok(msg)
    }
}

fn last_error_message_fd(fd: BorrowedFd<'_>) -> Option<String> {
    sys::info(fd)
        .ok()
        .map(|i| i.error_message())
        .filter(|m| !m.is_empty())
}

/// Wrap a syscall failure, attaching Homa's `error_msg` when it is likely to
/// be informative (not for EAGAIN/EINTR, which the kernel does not annotate
/// and which are hot paths).
fn os_err_fd(fd: BorrowedFd<'_>, op: &'static str, source: std::io::Error) -> HomaError {
    let detail = match source.raw_os_error() {
        Some(libc::EAGAIN | libc::EINTR) => None,
        _ => last_error_message_fd(fd),
    };
    HomaError::os(op, source).with_detail(detail)
}

impl AsFd for HomaSocket {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl AsRawFd for HomaSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl Drop for HomaSocket {
    fn drop(&mut self) {
        // Hand back any pages we still own. Errors are not actionable here:
        // closing the fd (next, by field order) frees the whole pool anyway.
        let batches = self.pending.drain_batches();
        let mut leaked = 0usize;
        for batch in &batches {
            if let Err(e) = sys::release_bpages(self.fd.as_fd(), batch) {
                leaked += batch.num_bpages as usize;
                debug!(error = %e, "HOMAIOCRELEASE on drop failed (pool is freed with the socket)");
            }
        }
        debug!(
            fd = self.fd.as_raw_fd(),
            port = self.local_port,
            released = batches.iter().map(|b| b.num_bpages as usize).sum::<usize>() - leaked,
            "homa socket closing"
        );
        // `fd` is closed and `buffer` unmapped by the compiler-generated drop
        // glue, in that order.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_builder_sets_fields() {
        let c = SocketConfig::new(AddressFamily::V6)
            .receive_region_bytes(1 << 20)
            .server(true);
        assert_eq!(c.family, AddressFamily::V6);
        assert_eq!(c.region_bytes, 1 << 20);
        assert_eq!(c.server, Some(true));
    }

    #[test]
    fn payload_validation() {
        assert!(matches!(
            HomaSocket::check_payload(b""),
            Err(HomaError::EmptyMessage)
        ));
        assert!(HomaSocket::check_payload(&[0u8; 10]).is_ok());
        let big = vec![0u8; crate::MAX_MESSAGE_LENGTH + 1];
        assert!(matches!(
            HomaSocket::check_payload(&big),
            Err(HomaError::MessageTooLarge { .. })
        ));
    }

    #[test]
    fn bind_rejects_auto_assigned_port_range_before_syscall() {
        // Port 40000 is in the auto-assigned range; this must fail without
        // needing the kernel module (checked before `socket()`).
        let err = HomaSocket::bind("0.0.0.0:40000").unwrap_err();
        assert!(matches!(err, HomaError::InvalidPort(40000)));
    }
}
