# Kernel integration tests

![Kernel interop: .NET smoke passed on Linux 6.17](https://img.shields.io/badge/kernel%20interop-.NET%20smoke%20passed%20on%206.17-2ea44f)
![Platform: Linux + homa.ko](https://img.shields.io/badge/platform-Linux%20%2B%20homa.ko-FCC624?logo=linux&logoColor=black)
![cargo test: ignored by default](https://img.shields.io/badge/cargo%20test-%23%5Bignore%5D%20by%20default-000000?logo=rust)
[![HomaModule 4bd0275](https://img.shields.io/badge/HomaModule-4bd0275-1f6feb)](https://github.com/PlatformLab/HomaModule/tree/4bd0275714439da2955670a6c438f6c23304d785)

These tests exercise the real Linux Homa kernel module over loopback. They are
never mocked. The test code lives in
`rust/crates/homa-rs/tests/kernel_integration.rs` and
`rust/crates/homa-ffi/tests/ffi_kernel.rs`. Every test is marked
`#[ignore]` so that an ordinary `cargo test` does not fail where Homa is
absent. `dotnet test` checks `Homa.NET` against `libhoma_ffi`, and checks
`Homa.Rpc` framing and dispatch in memory. It does not open a socket.
`dotnet/tests/Homa.KernelSmoke` is a separate program. `scripts/run-integration.sh`
runs it after the Rust and C suites and appends its output to the same log.
It has passed once on `lima-homa`; see the 2026-11-10 row below. `dotnet test`
still does not open a socket.

## Requirements

- Linux host with `homa.ko` loaded (see `docs/linux-setup.md`), ideally built
  from the pinned revision `4bd0275714439da2955670a6c438f6c23304d785`.
- Homa ports 7400-7599 free (7400-7499 for `homa-rs`, 7500-7599 for `homa-ffi`), plus 7601 and 7602 for the .NET smoke.
- .NET 10 SDK, so `Homa.KernelSmoke` can build.
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
| D1 | `Homa.KernelSmoke` | .NET 10 transport `ping` / `pong:ping` on port 7601, then `Homa.Rpc` `echo` of `hello` returning `rpc:hello` on port 7602. Passed on `lima-homa`. |

## Results log

Each `scripts/run-integration.sh` run appends a file to `results/`
(`<utc-timestamp>-<host>.log`) with kernel version, module version, toolchain,
git revision and the full test output. Commit the log with the change it
validates.

| Date (UTC) | Host / kernel | HomaModule rev | Result | Log |
|------------|---------------|----------------|--------|-----|
| 2026-11-03 21:05 | PKMZK00 / `6.18.54.1-microsoft-standard-WSL2+` (x86_64) | `4bd0275` plus a local `recvmsg` change for Linux 6.18, not in this repo | pass: 10 `homa-rs` kernel tests, 8 `homa-ffi` kernel tests, both header checks. No .NET output | [20261103T210543Z-PKMZK00.log](results/20261103T210543Z-PKMZK00.log) |
| 2026-11-10 22:48 | lima-homa / `6.17.0-40-generic` (aarch64) | `4bd0275` unmodified | pass: `Homa.KernelSmoke` (`ping` / `pong:ping`, `echo` of `hello` returned `rpc:hello`). Same guest, separate commands: 8 `homa-ffi` kernel tests passed; 9 of 10 `homa-rs` kernel tests passed (`request_to_unreachable_host_times_out` returned `EPROTONOSUPPORT`) | [20261110T224800Z-lima-homa.log](results/20261110T224800Z-lima-homa.log) |

The 2026-11-03 log is `homa-rs` and `homa-ffi` on one custom WSL2 kernel. That
loaded module is not the unmodified pinned tree. The 2026-11-10 log is the
managed smoke on a Lima guest whose kernel built the pinned tree with no local
patch. `dotnet test` still does not open a socket. The hosted service's
`StartAsync` was not the process that ran; the smoke calls `HomaRpcServer.Bind`
and `RunAsync` directly.
