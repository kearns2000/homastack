# Kernel integration tests

![Kernel interop: not yet executed](https://img.shields.io/badge/kernel%20interop-not%20yet%20executed-red)
![Platform: Linux + homa.ko](https://img.shields.io/badge/platform-Linux%20%2B%20homa.ko-FCC624?logo=linux&logoColor=black)
![cargo test: ignored by default](https://img.shields.io/badge/cargo%20test-%23%5Bignore%5D%20by%20default-000000?logo=rust)
[![HomaModule 4bd0275](https://img.shields.io/badge/HomaModule-4bd0275-1f6feb)](https://github.com/PlatformLab/HomaModule/tree/4bd0275714439da2955670a6c438f6c23304d785)

These tests exercise the real Linux Homa kernel module over loopback. They are
never mocked. The test code lives in
`rust/crates/homa-rs/tests/kernel_integration.rs` and
`rust/crates/homa-ffi/tests/ffi_kernel.rs`. Every test is marked
`#[ignore]` so that an ordinary `cargo test` does not fail where Homa is
absent. `dotnet test` on `Homa.NET` checks the managed wrapper against
`libhoma_ffi` and does not exchange an RPC. Kernel interoperability of the
Rust and .NET layers has not been executed.

## Requirements

- Linux host with `homa.ko` loaded (see `docs/linux-setup.md`), ideally built
  from the pinned revision `4bd0275714439da2955670a6c438f6c23304d785`.
- Homa ports 7400-7599 free (7400-7499 for `homa-rs`, 7500-7599 for `homa-ffi`).
- About 1.5 GB of address space headroom: each test socket maps a 64 MiB region
  (only touched pages are backed).
- `cc` and Linux UAPI headers for the header check.

## Procedure

```bash
scripts/check-environment.sh                       # must report "Homa sockets are usable"
scripts/run-integration.sh                         # header check + integration tests, logged
```

Manual equivalent:

```bash
cd rust
cargo test -p homa-sys --features header-check
cargo test -p homa-ffi --features header-check
cargo test -p homa-rs --test kernel_integration -- --ignored --test-threads=1 --nocapture
cargo test -p homa-ffi --test ffi_kernel -- --ignored --test-threads=1 --nocapture
```

`--test-threads=1` keeps the per-test echo servers from competing for CPU and
makes the logs readable; the tests use distinct ports so they are also safe in
parallel.

## Test matrix

| # | Test | Covers (from the Phase 1 brief) |
|---|------|--------------------------------|
| 1 | `small_request_and_response` | small RPC request/response, single bpage |
| 2 | `binary_payload_with_null_bytes` | binary payload with NULs, 1-byte message |
| 3 | `large_payload_spans_multiple_bpages` | 300 000 B (5 bpages), 2×64 KiB exact, 1 000 000 B (16 bpages), > max rejected |
| 4 | `multiple_sequential_requests_recycle_buffers` | 200 sequential requests, unique even ids, bpage recycling verified via `pending_bpages` and `HOMAIOCINFO.bpool_avail_bytes` |
| 4b | `completion_cookie_and_shared_response` | cookie round trip, non-private response via `recv`, private via id, reply-with-wrong-id rejected |
| 4c | `concurrent_requests_on_one_socket` | 4 threads × 50 requests on a shared socket |
| 5 | `request_to_port_without_server_fails` | invalid destination port (`ENOTCONN` or deadline), socket reusable afterwards |
| 5b | `request_to_unreachable_host_times_out` | unreachable IP → timeout/unreachable; family mismatch rejected locally |
| 6 | `recv_timeout_and_shutdown_unblock` | `recv_timeout`, `try_recv`, `poll_readable`, `shutdown` wakes a blocked `recv` with `ESHUTDOWN` |
| 7 | `resources_are_released_after_failures` | `EADDRINUSE`, port reuse after drop, drop with pending bpages, too-small region rejected with kernel `error_msg`, subsequent sockets work |
| F1 | `homa-ffi` `small_request_through_c_abi` | C ABI `homa_request` against an FFI-backed echo server |
| F2 | `homa-ffi` `binary_payload_with_nuls` | NUL bytes through the C ABI |
| F3 | `homa-ffi` `large_payload_and_repeated_alloc_free` | 1 B .. 200 KiB, message free each time |
| F4 | `homa-ffi` `sequential_private_send_recv` | 20 private send+recv, cookie and even ids |
| F5 | `homa-ffi` `concurrent_requests_on_one_socket` | 8 threads, one client handle |
| F6 | `homa-ffi` `rust_server_serves_ffi_client` | `homa-rs` server, C ABI client |
| F7 | `homa-ffi` `timeout_aborts_and_shutdown_unblocks` | request timeout / no-server; `homa_recv` woken by shutdown |
| F8 | `homa-ffi` `close_invalidates_handle_and_abort_unknown_id` | close, double-close, abort of unknown id |
| F1 | `homa-ffi` `small_request_through_c_abi` | C ABI `homa_request` against an FFI-backed echo server |
| F2 | `homa-ffi` `binary_payload_with_nuls` | NUL bytes through the C ABI |
| F3 | `homa-ffi` `large_payload_and_repeated_alloc_free` | 1 B .. 200 KiB, message free each time |
| F4 | `homa-ffi` `sequential_private_send_recv` | 20 private send+recv, cookie and even ids |
| F5 | `homa-ffi` `concurrent_requests_on_one_socket` | 8 threads, one client handle |
| F6 | `homa-ffi` `rust_server_serves_ffi_client` | `homa-rs` server, C ABI client |
| F7 | `homa-ffi` `timeout_aborts_and_shutdown_unblocks` | request timeout / no-server; `homa_recv` woken by shutdown |
| F8 | `homa-ffi` `close_invalidates_handle_and_abort_unknown_id` | close, double-close, abort of unknown id |

## Results log

Each `scripts/run-integration.sh` run appends a file to `results/`
(`<utc-timestamp>-<host>.log`) with kernel version, module version, toolchain,
git revision and the full test output. Commit the log with the change it
validates.

| Date (UTC) | Host / kernel | HomaModule rev | Result | Log |
|------------|---------------|----------------|--------|-----|
| - | - | - | **not yet executed** - Phase 1 development was done on macOS (no Homa). | - |

Until a row with a passing result exists here, kernel interoperability of this
code is **unverified**. Do not report it as working.
