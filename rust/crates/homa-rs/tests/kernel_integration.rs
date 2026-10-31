//! Kernel integration tests - **require Linux with the `homa` kernel module
//! loaded**. They talk to the real kernel over real Homa sockets (loopback);
//! nothing is mocked.
//!
//! Every test is `#[ignore]`d so a plain `cargo test` stays green on machines
//! without Homa. Run them explicitly:
//!
//! ```text
//! cargo test -p homa-rs --test kernel_integration -- --ignored --test-threads=1
//! ```
//!
//! or via `scripts/run-integration.sh`, which also records the results.
//! Ports 7400–7499 must be free. See `tests/integration/README.md` at the
//! repository root for the procedure and the results log.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use homa_rs::{AddressFamily, ErrorKind, HomaError, HomaSocket, Message};

const BASE_PORT: u16 = 7400;

fn dest(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

fn must<T>(r: homa_rs::Result<T>, what: &str) -> T {
    match r {
        Ok(v) => v,
        Err(e) if e.kind() == ErrorKind::Unavailable => panic!(
            "{what}: Homa is unavailable ({e}). Load the kernel module (see docs/linux-setup.md) \
             before running the ignored integration tests."
        ),
        Err(e) => panic!("{what}: {e} (kind {:?})", e.kind()),
    }
}

/// An in-process echo server on its own thread. Dropping it shuts the socket
/// down (unblocking `recv`) and joins the thread.
struct EchoServer {
    socket: Arc<HomaSocket>,
    handle: Option<JoinHandle<u64>>,
    port: u16,
}

impl EchoServer {
    fn start(port: u16) -> Self {
        let socket = Arc::new(must(HomaSocket::bind(("0.0.0.0", port)), "bind server"));
        assert_eq!(socket.local_port(), port);
        let worker = Arc::clone(&socket);
        let handle = thread::spawn(move || {
            let mut served = 0u64;
            loop {
                match worker.recv() {
                    Ok(msg) if msg.is_request() => {
                        if worker.reply(&msg, &msg.payload).is_ok() {
                            served += 1;
                        }
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == ErrorKind::Shutdown => break served,
                    Err(e) if e.is_recoverable() => {}
                    Err(e) => panic!("echo server died: {e}"),
                }
            }
        });
        Self {
            socket,
            handle: Some(handle),
            port,
        }
    }

    fn addr(&self) -> SocketAddr {
        dest(self.port)
    }
}

impl Drop for EchoServer {
    fn drop(&mut self) {
        let _ = self.socket.shutdown();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn new_client() -> HomaSocket {
    let c = must(HomaSocket::open(AddressFamily::V4), "open client");
    assert!(
        c.local_port() >= homa_sys::HOMA_MIN_DEFAULT_PORT,
        "client port is auto-assigned"
    );
    c
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn check_echo(client: &HomaSocket, server: &EchoServer, payload: &[u8]) -> Message {
    let msg = must(client.request(server.addr(), payload), "request");
    assert!(
        msg.is_response(),
        "client must receive a response (even id): {msg:?}"
    );
    assert_eq!(
        msg.peer.port(),
        server.port,
        "response comes from the server port"
    );
    assert_eq!(msg.len(), payload.len());
    assert_eq!(msg.payload, payload, "echoed bytes must match");
    assert_eq!(msg.fragments, payload.len().div_ceil(homa_rs::BPAGE_SIZE));
    msg
}

// ---------------------------------------------------------------------------
// 1. Small RPC request and response
// ---------------------------------------------------------------------------
#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn small_request_and_response() {
    let server = EchoServer::start(BASE_PORT);
    let client = new_client();
    let msg = check_echo(&client, &server, b"Hello Homa");
    assert_eq!(msg.fragments, 1);
}

// ---------------------------------------------------------------------------
// 2. Binary payload containing null bytes
// ---------------------------------------------------------------------------
#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn binary_payload_with_null_bytes() {
    let server = EchoServer::start(BASE_PORT + 1);
    let client = new_client();
    let mut payload = vec![0u8; 4096];
    for (i, b) in payload.iter_mut().enumerate() {
        *b = if i % 7 == 0 { 0 } else { (i & 0xff) as u8 };
    }
    payload[0] = 0;
    payload[4095] = 0;
    check_echo(&client, &server, &payload);
    check_echo(&client, &server, &[0u8]); // smallest legal message
}

// ---------------------------------------------------------------------------
// 3. Large payload spanning multiple receive pages
// ---------------------------------------------------------------------------
#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn large_payload_spans_multiple_bpages() {
    let server = EchoServer::start(BASE_PORT + 2);
    let client = new_client();

    // 300 000 bytes -> 5 bpages (4 full + partial tail).
    let msg = check_echo(&client, &server, &pattern(300_000));
    assert_eq!(msg.fragments, 5);

    // Exactly two full bpages, no tail.
    let msg = check_echo(&client, &server, &pattern(2 * homa_rs::BPAGE_SIZE));
    assert_eq!(msg.fragments, 2);

    // Maximum message size -> 16 bpages.
    let msg = check_echo(&client, &server, &pattern(homa_rs::MAX_MESSAGE_LENGTH));
    assert_eq!(msg.fragments, 16);

    // One byte over the limit is rejected locally, before any syscall.
    let err = client
        .request(server.addr(), &pattern(homa_rs::MAX_MESSAGE_LENGTH + 1))
        .unwrap_err();
    assert!(matches!(err, HomaError::MessageTooLarge { .. }));
}

// ---------------------------------------------------------------------------
// 4. Multiple sequential requests (ids, cookies, buffer recycling)
// ---------------------------------------------------------------------------
#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn multiple_sequential_requests_recycle_buffers() {
    let server = EchoServer::start(BASE_PORT + 3);
    let client = new_client();
    let mut ids = Vec::new();
    for i in 0..200u32 {
        let payload = format!("request number {i}").into_bytes();
        let msg = check_echo(&client, &server, &payload);
        ids.push(msg.id);
    }
    // Every RPC id is distinct and even on the client side.
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "RPC ids must be unique");
    assert!(ids.iter().all(|id| id.is_client_side()));

    // Buffer pages are recycled on each receive, so at most one message's
    // worth is ever outstanding on the client.
    assert!(
        client.pending_bpages() <= 1,
        "pending = {}",
        client.pending_bpages()
    );
    let released = must(client.release_pending(), "release_pending");
    assert!(released <= 1);
    assert_eq!(client.pending_bpages(), 0);

    // The server likewise holds at most one message's pages; the kernel's
    // view of the pool should be (nearly) full again.
    let info = must(server.socket.info(), "server info");
    assert_eq!(info.port, server.port);
    assert!(
        info.buffer_available_bytes
            >= server.socket.receive_region_len() as u64 - 2 * homa_rs::BPAGE_SIZE as u64,
        "server pool not recycled: {info:?}"
    );
}

#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn completion_cookie_and_shared_response() {
    let server = EchoServer::start(BASE_PORT + 4);
    let client = new_client();
    // Non-private request: the response arrives via the shared `recv` path
    // and carries the completion cookie.
    let id = must(
        client.send_request(server.addr(), b"cookie", 0xDEAD_BEEF, false),
        "send",
    );
    assert!(id.is_client_side());
    let msg = must(client.recv(), "recv shared response");
    assert_eq!(msg.id, id);
    assert_eq!(msg.completion_cookie, 0xDEAD_BEEF);
    assert_eq!(msg.payload, b"cookie");

    // Private request: must be collected by id.
    let id = must(
        client.send_request(server.addr(), b"private", 7, true),
        "send private",
    );
    let msg = must(
        client.recv_response_timeout(id, Duration::from_secs(5)),
        "recv private",
    );
    assert_eq!(msg.id, id);
    assert_eq!(msg.completion_cookie, 7);

    // Replying with a client-side id is rejected locally.
    let err = client.reply_to(id, server.addr(), b"x").unwrap_err();
    assert!(matches!(err, HomaError::UnexpectedMessage { .. }));
}

#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn concurrent_requests_on_one_socket() {
    let server = EchoServer::start(BASE_PORT + 5);
    let client = Arc::new(new_client());
    let workers: Vec<_> = (0..4u8)
        .map(|w| {
            let client = Arc::clone(&client);
            let addr = server.addr();
            thread::spawn(move || {
                for i in 0..50u32 {
                    let payload = pattern(1000 + usize::from(w) * 10_000 + i as usize);
                    let msg = must(client.request(addr, &payload), "concurrent request");
                    assert_eq!(msg.payload, payload);
                }
            })
        })
        .collect();
    for w in workers {
        w.join().expect("worker panicked");
    }
}

// ---------------------------------------------------------------------------
// 5. Invalid destination
// ---------------------------------------------------------------------------
#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn request_to_port_without_server_fails() {
    let client = new_client();
    let start = Instant::now();
    let err = client
        .request_timeout(
            dest(BASE_PORT + 90),
            b"anyone home?",
            Duration::from_secs(10),
        )
        .unwrap_err();
    eprintln!(
        "no-server error after {:?}: {err} (kind {:?})",
        start.elapsed(),
        err.kind()
    );
    // Upstream documents ENOTCONN ("no server connected to the port"); the
    // deadline is a fallback if the peer never answers at all.
    assert!(
        matches!(err.kind(), ErrorKind::NoServer | ErrorKind::Timeout),
        "unexpected error kind {:?}: {err}",
        err.kind()
    );
    if let HomaError::RpcFailed { id, .. } = &err {
        assert!(id.is_client_side());
    }
    // The socket remains usable afterwards.
    let server = EchoServer::start(BASE_PORT + 6);
    check_echo(&client, &server, b"still alive");
}

#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn request_to_unreachable_host_times_out() {
    let client = new_client();
    // 192.0.2.0/24 is TEST-NET-1: never routable.
    let err = client
        .request_timeout(
            dest_ip([192, 0, 2, 1], BASE_PORT),
            b"void",
            Duration::from_secs(2),
        )
        .unwrap_err();
    eprintln!("unreachable error: {err} (kind {:?})", err.kind());
    assert!(
        matches!(err.kind(), ErrorKind::Timeout | ErrorKind::Unreachable),
        "unexpected error kind {:?}: {err}",
        err.kind()
    );
    // Address family mismatch is caught before any syscall.
    let err = client.request("[::1]:7400", b"x").unwrap_err();
    assert!(matches!(err, HomaError::AddressFamilyMismatch { .. }));
}

fn dest_ip(ip: [u8; 4], port: u16) -> SocketAddr {
    SocketAddr::from((ip, port))
}

// ---------------------------------------------------------------------------
// 6. Receive timeout and socket shutdown
// ---------------------------------------------------------------------------
#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn recv_timeout_and_shutdown_unblock() {
    let socket = Arc::new(must(HomaSocket::bind(("0.0.0.0", BASE_PORT + 7)), "bind"));

    // Nothing arrives: timeout returns None promptly.
    let start = Instant::now();
    assert!(
        must(
            socket.recv_timeout(Duration::from_millis(200)),
            "recv_timeout"
        )
        .is_none()
    );
    assert!(start.elapsed() >= Duration::from_millis(200));
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(must(socket.try_recv(), "try_recv").is_none());
    assert!(!must(
        socket.poll_readable(Some(Duration::from_millis(50))),
        "poll"
    ));

    // A blocked receiver is woken by shutdown with ESHUTDOWN.
    let blocked = Arc::clone(&socket);
    let waiter = thread::spawn(move || blocked.recv());
    thread::sleep(Duration::from_millis(100));
    must(socket.shutdown(), "shutdown");
    let result = waiter.join().expect("waiter panicked");
    let err = result.expect_err("recv must fail after shutdown");
    assert_eq!(err.kind(), ErrorKind::Shutdown, "{err}");
    assert!(!err.is_recoverable());

    // Further operations are rejected too.
    let err = socket.request(dest(BASE_PORT + 7), b"x").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Shutdown, "{err}");
}

// ---------------------------------------------------------------------------
// 7. Resource cleanup following failure
// ---------------------------------------------------------------------------
#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn resources_are_released_after_failures() {
    let port = BASE_PORT + 8;

    // Port conflict is reported and leaves the first socket intact.
    let first = must(HomaSocket::bind(("0.0.0.0", port)), "bind first");
    let err = HomaSocket::bind(("0.0.0.0", port)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AddressInUse, "{err}");
    assert_eq!(must(first.info(), "info").port, port);

    // Dropping the first socket (which also unmaps its 64 MiB region) frees
    // the port for immediate reuse.
    drop(first);
    let server = EchoServer::start(port);

    // A failed request (no server) followed by a successful one on the same
    // client socket; then a drop with outstanding bpages must not leak or
    // crash, and a brand-new socket must still work.
    {
        let client = new_client();
        let _ = client.request_timeout(dest(BASE_PORT + 91), b"x", Duration::from_millis(500));
        check_echo(&client, &server, &pattern(200_000)); // 4 bpages outstanding
        assert!(client.pending_bpages() >= 1);
        // Dropped here with pages pending -> HOMAIOCRELEASE then close.
    }
    let client = new_client();
    check_echo(&client, &server, b"after cleanup");

    // Too-small receive regions are rejected by the kernel with a readable
    // explanation, and the failed creation leaks nothing (next open works).
    let err = HomaSocket::config(AddressFamily::V4)
        .receive_region_bytes(64 * 1024)
        .open()
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err}");
    assert!(
        err.to_string().contains("buffer pool"),
        "expected Homa's error_msg, got: {err}"
    );
    let _ok = new_client();
}
