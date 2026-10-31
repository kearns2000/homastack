//! Kernel integration tests for the C ABI. Require Linux with `homa.ko`.
//!
//! Every test is `#[ignore]`d so `cargo test` stays green without Homa.
//! Run with:
//!
//! ```text
//! cargo test -p homa-ffi --test ffi_kernel -- --ignored --test-threads=1
//! ```
//!
//! Ports 7500-7599 must be free. These tests call the exported C functions
//! (not `homa-rs` directly) against a Rust echo server also driven through
//! the C ABI, plus one case that uses a `homa-rs` server.

#![cfg(target_os = "linux")]
#![allow(clippy::missing_panics_doc, clippy::undocumented_unsafe_blocks)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use homa_ffi::api::{
    homa_abort_rpc, homa_message_free, homa_message_info, homa_recv, homa_recv_response,
    homa_reply_to_message, homa_request, homa_send_request, homa_socket_bind, homa_socket_close,
    homa_socket_local_port, homa_socket_open, homa_socket_shutdown,
};
use homa_ffi::handles::homa_socket_t;
use homa_ffi::messages::homa_message_t;
use homa_ffi::{
    HOMA_ENDPOINT_IPV4, HOMA_ERR_INVALID_ARGUMENT, HOMA_ERR_NO_SERVER, HOMA_ERR_SHUTDOWN,
    HOMA_ERR_TIMEOUT, HOMA_ERR_UNAVAILABLE, HOMA_ERR_WOULD_BLOCK, HOMA_MESSAGE_RESPONSE, HOMA_OK,
    HOMA_REQUEST_PRIVATE, HOMA_TIMEOUT_INFINITE, homa_endpoint_t, homa_message_info_t,
    homa_socket_options_t,
};

const BASE_PORT: u16 = 7500;

fn must(st: i32, what: &str) {
    if st == HOMA_OK {
        return;
    }
    assert!(
        st != HOMA_ERR_UNAVAILABLE,
        "{what}: Homa is unavailable (status {st}). Load the kernel module \
         (see docs/linux-setup.md) before running --ignored tests."
    );
    panic!("{what}: status {st} ({})", homa_ffi::status_name(st));
}

fn localhost(port: u16) -> homa_endpoint_t {
    let mut ep = homa_endpoint_t {
        family: HOMA_ENDPOINT_IPV4,
        port,
        ..Default::default()
    };
    ep.addr[..4].copy_from_slice(&[127, 0, 0, 1]);
    ep
}

fn open_client() -> *mut homa_socket_t {
    let mut sock = std::ptr::null_mut();
    let opts = homa_socket_options_t {
        struct_size: size_of::<homa_socket_options_t>() as u32,
        ..Default::default()
    };
    must(
        unsafe { homa_socket_open(&raw const opts, &raw mut sock) },
        "open client",
    );
    assert!(!sock.is_null());
    sock
}

fn bind_server(port: u16) -> *mut homa_socket_t {
    let mut sock = std::ptr::null_mut();
    let opts = homa_socket_options_t {
        struct_size: size_of::<homa_socket_options_t>() as u32,
        ..Default::default()
    };
    must(
        unsafe { homa_socket_bind(&raw const opts, port, &raw mut sock) },
        "bind server",
    );
    let mut got = 0u16;
    must(
        unsafe { homa_socket_local_port(sock, &raw mut got) },
        "local_port",
    );
    assert_eq!(got, port);
    sock
}

fn info_of(msg: *mut homa_message_t) -> homa_message_info_t {
    let mut info = homa_message_info_t {
        struct_size: size_of::<homa_message_info_t>() as u32,
        ..Default::default()
    };
    must(
        unsafe { homa_message_info(msg, &raw mut info) },
        "message_info",
    );
    info
}

fn payload_of(msg: *mut homa_message_t) -> Vec<u8> {
    let info = info_of(msg);
    unsafe { std::slice::from_raw_parts(info.payload, info.payload_len as usize) }.to_vec()
}

/// Echo server driven entirely through the C ABI.
struct FfiEcho {
    socket: *mut homa_socket_t,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    port: u16,
}

impl FfiEcho {
    fn start(port: u16) -> Self {
        let socket = bind_server(port);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_w = Arc::clone(&stop);
        let worker_bits = socket as usize;
        let handle = thread::spawn(move || {
            let worker = worker_bits as *mut homa_socket_t;
            while !stop_w.load(Ordering::Relaxed) {
                let mut msg = std::ptr::null_mut();
                let st = unsafe { homa_recv(worker, 100, &raw mut msg) };
                if st == HOMA_ERR_TIMEOUT || st == HOMA_ERR_WOULD_BLOCK {
                    continue;
                }
                if st == HOMA_ERR_SHUTDOWN {
                    break;
                }
                must(st, "server recv");
                let body = payload_of(msg);
                must(
                    unsafe { homa_reply_to_message(worker, msg, body.as_ptr(), body.len() as u64) },
                    "server reply",
                );
                unsafe { homa_message_free(msg) };
            }
        });
        Self {
            socket,
            stop,
            handle: Some(handle),
            port,
        }
    }

    fn dest(&self) -> homa_endpoint_t {
        localhost(self.port)
    }
}

impl Drop for FfiEcho {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = homa_socket_shutdown(self.socket);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let _ = homa_socket_close(self.socket);
    }
}

fn request_bytes(client: *mut homa_socket_t, dest: &homa_endpoint_t, payload: &[u8]) -> Vec<u8> {
    let mut msg = std::ptr::null_mut();
    must(
        unsafe {
            homa_request(
                client,
                dest,
                payload.as_ptr(),
                payload.len() as u64,
                5_000,
                &raw mut msg,
            )
        },
        "homa_request",
    );
    let out = payload_of(msg);
    unsafe { homa_message_free(msg) };
    out
}

#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn small_request_through_c_abi() {
    let server = FfiEcho::start(BASE_PORT);
    let client = open_client();
    let reply = request_bytes(client, &server.dest(), b"Hello Homa");
    assert_eq!(reply, b"Hello Homa");
    must(homa_socket_close(client), "close client");
}

#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn binary_payload_with_nuls() {
    let server = FfiEcho::start(BASE_PORT + 1);
    let client = open_client();
    let payload = b"a\0b\0c".as_slice();
    assert_eq!(request_bytes(client, &server.dest(), payload), payload);
    must(homa_socket_close(client), "close");
}

#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn large_payload_and_repeated_alloc_free() {
    let server = FfiEcho::start(BASE_PORT + 2);
    let client = open_client();
    let dest = server.dest();
    for n in [1usize, 64, 65_536, 200_000] {
        let payload = vec![0x5A; n];
        let reply = request_bytes(client, &dest, &payload);
        assert_eq!(reply.len(), n);
        assert_eq!(reply, payload);
    }
    must(homa_socket_close(client), "close");
}

#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn sequential_private_send_recv() {
    let server = FfiEcho::start(BASE_PORT + 3);
    let client = open_client();
    let dest = server.dest();
    for i in 0..20u32 {
        let payload = format!("n={i}").into_bytes();
        let mut id = 0u64;
        must(
            unsafe {
                homa_send_request(
                    client,
                    &raw const dest,
                    payload.as_ptr(),
                    payload.len() as u64,
                    u64::from(i) + 1,
                    HOMA_REQUEST_PRIVATE,
                    &raw mut id,
                )
            },
            "send_request",
        );
        assert_ne!(id, 0);
        assert_eq!(id & 1, 0);
        let mut msg = std::ptr::null_mut();
        must(
            unsafe { homa_recv_response(client, id, 5_000, &raw mut msg) },
            "recv_response",
        );
        let info = info_of(msg);
        assert_eq!(info.kind, HOMA_MESSAGE_RESPONSE);
        assert_eq!(info.rpc_id, id);
        assert_eq!(info.completion_cookie, u64::from(i) + 1);
        assert_eq!(payload_of(msg), payload);
        unsafe { homa_message_free(msg) };
    }
    must(homa_socket_close(client), "close");
}

#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn concurrent_requests_on_one_socket() {
    let server = FfiEcho::start(BASE_PORT + 4);
    let client = open_client();
    let dest = server.dest();
    let mut joins = Vec::new();
    for i in 0..8u32 {
        let client_bits = client as usize;
        joins.push(thread::spawn(move || {
            let client = client_bits as *mut homa_socket_t;
            let payload = format!("c={i}").into_bytes();
            request_bytes(client, &dest, &payload)
        }));
    }
    for (i, j) in joins.into_iter().enumerate() {
        let got = j.join().unwrap();
        assert_eq!(got, format!("c={i}").into_bytes());
    }
    must(homa_socket_close(client), "close");
}

#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn rust_server_serves_ffi_client() {
    use homa_rs::{HomaSocket, Message};
    let server = HomaSocket::bind(("0.0.0.0", BASE_PORT + 5)).expect("bind rust server");
    let port = server.local_port();
    let worker = thread::spawn(move || {
        let msg: Message = server.recv().expect("recv");
        server.reply(&msg, &msg.payload).expect("reply");
        server
    });
    let client = open_client();
    let reply = request_bytes(client, &localhost(port), b"via rust server");
    assert_eq!(reply, b"via rust server");
    must(homa_socket_close(client), "close");
    drop(worker.join().unwrap());
}

#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn timeout_aborts_and_shutdown_unblocks() {
    let client = open_client();
    let dest = localhost(BASE_PORT + 6); // nothing listening
    let mut msg = std::ptr::null_mut();
    let st = unsafe { homa_request(client, &raw const dest, b"x".as_ptr(), 1, 50, &raw mut msg) };
    assert!(
        st == HOMA_ERR_TIMEOUT || st == HOMA_ERR_NO_SERVER,
        "expected timeout or no-server, got {st}"
    );
    assert!(msg.is_null());

    let server = bind_server(BASE_PORT + 7);
    let server_bits = server as usize;
    let handle = thread::spawn(move || {
        let server = server_bits as *mut homa_socket_t;
        let mut m = std::ptr::null_mut();
        unsafe { homa_recv(server, HOMA_TIMEOUT_INFINITE, &raw mut m) }
    });
    thread::sleep(Duration::from_millis(20));
    must(homa_socket_shutdown(server), "shutdown");
    let st = handle.join().unwrap();
    assert_eq!(st, HOMA_ERR_SHUTDOWN);
    must(homa_socket_close(server), "close server");
    must(homa_socket_close(client), "close client");
}

#[test]
#[ignore = "requires the Homa kernel module (run with --ignored)"]
fn close_invalidates_handle_and_abort_unknown_id() {
    let client = open_client();
    assert_eq!(homa_abort_rpc(client, 2), HOMA_ERR_INVALID_ARGUMENT);
    must(homa_socket_close(client), "close");
    assert_eq!(homa_socket_close(client), HOMA_ERR_INVALID_ARGUMENT);
    assert_eq!(homa_abort_rpc(client, 0), HOMA_ERR_INVALID_ARGUMENT);
}
