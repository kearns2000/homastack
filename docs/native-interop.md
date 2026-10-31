# Native interop: the kernel ABI and the C ABI

## Part A - Kernel interface used by `homa-sys` (desk-checked against upstream source)

Everything in this section was cross-checked line by line against the pinned
HomaModule revision (`homa.h`, `homa_plumbing.c`, `homa_pool.c`,
`homa_incoming.c`, `homa_sock.c`) and the struct layouts are enforced by
compile-time assertions and the optional `header-check` C probe. It has **not**
yet been exercised against a running kernel; see `tests/integration/README.md`.

Source of truth: `homa.h`, `homa_plumbing.c`, `homa_sock.c`, `homa_pool.c`,
`homa_receiver.{h,cc}`, `util/demo_client1.cc`, `util/demo_server1.cc` and the
man pages `homa.7`, `sendmsg.2`, `recvmsg.2` from
`PlatformLab/HomaModule@4bd0275714439da2955670a6c438f6c23304d785`.

### Constants

| Name | Value | Use |
|------|-------|-----|
| `IPPROTO_HOMA` | 146 | `socket()` protocol and `setsockopt` level |
| `HOMA_MAX_MESSAGE_LENGTH` | 1 000 000 | max payload per message |
| `HOMA_BPAGE_SHIFT` / `HOMA_BPAGE_SIZE` | 16 / 65 536 | receive page size |
| `HOMA_MAX_BPAGES` | 16 | max fragments per message |
| `HOMA_MIN_DEFAULT_PORT` | 0x8000 | ports ≥ this are auto-assigned; `bind` accepts only 1-32767 |
| `HOMA_SENDMSG_PRIVATE` | 0x01 | request flag |
| `SO_HOMA_RCVBUF` / `SO_HOMA_SERVER` | 10 / 11 | setsockopt options |
| `HOMA_ERROR_MSG_SIZE` | 104 | `homa_info.error_msg` |

### Structures (sizes verified by `const` assertions and `header-check`)

| Struct | Size | Notes |
|--------|------|-------|
| `homa_sendmsg_args` | 24 | `id` (in/out), `completion_cookie`, `flags`, `reserved` |
| `homa_recvmsg_args` | 88 | `id`, `completion_cookie`, `num_bpages`, `reserved`, `bpage_offsets[16]` |
| `homa_rcvbuf_args` | 16 | `start`, `length` |
| `homa_abort_args` | 32 | non-stripped module only |
| `homa_rpc_info` | 184 | contains a 128-byte `__kernel_sockaddr_storage` |
| `homa_info` | 136 | `rpc_info`, `rpc_info_length`, `bpool_avail_bytes`, `port`, `num_rpcs`, `error_msg[104]` |

ioctls (asm-generic encoding): `HOMAIOCINFO = 0xc0886890`,
`HOMAIOCRELEASE = 0x40586891`, `HOMAIOCABORT = 0xc0206892`,
`HOMAIOCFREEZE = 0x6893`.

### Call sequences and quirks

| Operation | Exactly what the kernel expects | Where verified |
|-----------|--------------------------------|----------------|
| create | `socket(AF_INET|AF_INET6, SOCK_DGRAM, IPPROTO_HOMA)`; `EPROTONOSUPPORT` if module absent | `homa.7` |
| bind | `bind(sockaddr_in/in6)`; only the port matters; `0` = no-op; `≥0x8000` → `EINVAL`; taken → `EADDRINUSE`; sets `is_server` | `homa_sock_bind` |
| receive region | `setsockopt(IPPROTO_HOMA, SO_HOMA_RCVBUF, &{start,len}, 16)` once, before first `recvmsg`; page-aligned; `len ≥ nr_cpu_ids·64K + 2·1M`, `≤ 4 GiB`; kernel probes first byte for writability | `homa_setsockopt`, `homa_pool_set_region` |
| server flag | `setsockopt(IPPROTO_HOMA, SO_HOMA_SERVER, &int, 4)` | `homa_setsockopt` |
| send | `msg_name` required and family must match; `msg_iov` 1..1 000 000 bytes; `msg_control = &homa_sendmsg_args`; **`msg_controllen = 0`**; returns **0** on success; `id==0` → request, `id` written back; `id!=0` → response, `completion_cookie` must be 0; unknown response id → silent success | `homa_sendmsg`, `sendmsg.2`, `demo_client1.cc` |
| receive | `msg_control = &homa_recvmsg_args`, `msg_controllen == 88` exactly; input `num_bpages`/offsets are freed first; `id==0` waits for shared, `id!=0` for that private RPC; returns length; `msg_name` filled with `sockaddr_in`/`in6`; on error after a wait, `id!=0` ⇒ that RPC failed and is already ended; on pre-wait `EINVAL`/`EFAULT` the input `id` is echoed; `num_bpages` is always 0 on error (input pages were freed first; `recvmsg.2`'s "echoed back" wording does not match this revision) | `homa_recvmsg`, `recvmsg.2` |
| recycle | next `recvmsg`, or `ioctl(HOMAIOCRELEASE, &homa_recvmsg_args)` (reads `num_bpages`, `bpage_offsets`, `reserved==0`) | `homa_ioc_release`, `homa_receiver.cc` |
| info | `ioctl(HOMAIOCINFO, &homa_info)` with `rpc_info=0` → port, `num_rpcs`, pool free bytes, `error_msg` | `homa_ioc_info` |
| abort | `ioctl(HOMAIOCABORT, &{id, error})`; client RPCs only | `homa_ioc_abort` |
| shutdown | `shutdown(fd, any)` disables both directions; waiters get `ESHUTDOWN`; `poll` reports readable | `homa_shutdown`, `homa_poll` |
| readiness | `poll/select/epoll` `POLLIN` when a **shared** RPC is ready or socket shut down; private responses never signal | `homa_poll`, `sendmsg.2` |
| timeouts | `SO_RCVTIMEO` is not consulted by Homa's wait path; use `MSG_DONTWAIT` / `O_NONBLOCK` | `homa_wait_*` (no `sk_rcvtimeo` use) |

Address returned by `recvmsg` is always the peer's **Homa port** (client's
auto-assigned port for requests, server port for responses).

### Uncertainties documented rather than guessed

- Minimum region size depends on `nr_cpu_ids`, which user space cannot read
  portably; `homa-rs` does not pre-validate it and instead surfaces the
  kernel's `EINVAL` plus `error_msg`.
- `HOMAIOCABORT`/`HOMAIOCFREEZE` and `homa_rpc_info.tx_prio` are wrapped in
  `#ifndef __STRIP__` upstream. The GitHub module has them; a module built from
  the `strip.py` output does not (abort returns `EINVAL`, which `homa-rs`
  tolerates after a client timeout).
- Whether `getsockname(2)` reports the Homa port was not verified; `homa-rs`
  uses `HOMAIOCINFO.port` instead.
- Behaviour of `bind` after traffic has flowed is documented upstream as
  "should not be done"; `homa-rs` only binds at creation.

## Part B - Stable C ABI (`homa-ffi`, Phase 2, ABI version 1)

Implemented. Source of truth: `rust/crates/homa-ffi/include/homa_ffi.h` and
the `#[repr(C)]` types in `rust/crates/homa-ffi/src/types.rs`. The header is
hand-written (same approach as `homa-sys` vs `homa.h`);
`cargo test -p homa-ffi --features header-check` compiles it as C11 and
compares every size, offset and constant. A Linux `cdylib` exports only the
`homa_*` symbols (`homa_ffi.map`).

This layer is synchronous. It does not implement the Homa wire protocol. Every
socket call is forwarded to `homa_rs::HomaSocket`.

### Versioning

| Item | Rule |
|------|------|
| `homa_ffi_abi_version()` | `1` for this interface. Callers must check it. |
| Breaking change | bump the integer, keep old symbols only if they still work, document the migration. |
| Compatible addition | new function symbol, or new fields appended to a struct after a larger `struct_size`. Older callers keep working. |
| Unsupported version | a future library that cannot serve ABI 1 should still return the new number; the caller refuses to proceed. |

ABI 1 is the first published interface. The two query symbols
(`homa_ffi_abi_version`, `homa_ffi_platform_supported`) were sketched as a
Phase 1 placeholder that returned `0`; this tree ships them as part of ABI 1
and does not keep a separate ABI-0 library.

### Status codes

`homa_status_t` is `int32_t`. `HOMA_OK = 0`. The other values (`1`..`17`,
`99`) are listed in the header and will not be reused. Last-error state is
thread-local: status, `errno`, UTF-8 message. `homa_last_error_message`
copies into a caller buffer and reports the required length (including NUL).
Successful calls do not clear the record; `homa_clear_last_error` does.
A Rust panic is caught at the boundary and becomes `HOMA_ERR_INTERNAL`.

### Exported functions

```c
uint32_t      homa_ffi_abi_version(void);
bool          homa_ffi_platform_supported(void);
uint32_t      homa_ffi_max_message_length(void);
const char   *homa_status_name(homa_status_t);

homa_status_t homa_last_error_message(char *buf, uint64_t buf_len, uint64_t *required_len);
homa_status_t homa_last_error_status(void);
int32_t       homa_last_error_errno(void);
void          homa_clear_last_error(void);

homa_status_t homa_socket_open(const homa_socket_options_t *options, homa_socket_t **out);
homa_status_t homa_socket_bind(const homa_socket_options_t *options, uint16_t port, homa_socket_t **out);
homa_status_t homa_socket_close(homa_socket_t *socket);
homa_status_t homa_socket_shutdown(homa_socket_t *socket);
homa_status_t homa_socket_local_port(const homa_socket_t *socket, uint16_t *out_port);
homa_status_t homa_socket_info(const homa_socket_t *socket, homa_socket_info_t *info);

homa_status_t homa_send_request(homa_socket_t *, const homa_endpoint_t *,
                                const uint8_t *payload, uint64_t len,
                                uint64_t cookie, uint32_t flags, uint64_t *rpc_id);
homa_status_t homa_recv_response(homa_socket_t *, uint64_t rpc_id, int64_t timeout_ms,
                                 homa_message_t **out);
homa_status_t homa_request(homa_socket_t *, const homa_endpoint_t *,
                           const uint8_t *payload, uint64_t len, int64_t timeout_ms,
                           homa_message_t **out);
homa_status_t homa_recv(homa_socket_t *, int64_t timeout_ms, homa_message_t **out);
homa_status_t homa_reply(homa_socket_t *, uint64_t rpc_id, const homa_endpoint_t *,
                         const uint8_t *payload, uint64_t len);
homa_status_t homa_reply_to_message(homa_socket_t *, const homa_message_t *,
                                    const uint8_t *payload, uint64_t len);
homa_status_t homa_abort_rpc(homa_socket_t *, uint64_t rpc_id);

homa_status_t homa_message_info(const homa_message_t *, homa_message_info_t *);
const uint8_t *homa_message_payload(const homa_message_t *, uint64_t *len);
void          homa_message_free(homa_message_t *);
```

`homa_endpoint_t` carries family (`4`/`6`), host-order port, IPv6 scope id
and 16 network-order address bytes. Timeouts: `timeout_ms < 0` blocks;
`0` is a single non-blocking attempt; `> 0` is a deadline in milliseconds.
Private-response expiry **aborts** the RPC. Shared-receive expiry does not.

### Ownership

- `homa_socket_t *` is a generation-checked registry key. Close with
  `homa_socket_close`. Use after close returns `HOMA_ERR_INVALID_ARGUMENT`
  and cannot use-after-free. Concurrent close wakes in-flight receives with
  `HOMA_ERR_SHUTDOWN`.
- `homa_message_t *` owns a `Vec<u8>` copied out of the kernel region. The
  payload pointer is valid until `homa_message_free`. Free exactly once; do
  not pass it to `free(3)`. Double-free is undefined, same class as `free(3)`.
- Not zero-copy.

### Threading

A handle is usable from many threads. The kernel still allows only one waiter
per private RPC id. Several threads may call `homa_recv`; each shared message
goes to exactly one. Last-error state is per-thread.

### Build and link

```bash
cd rust
cargo build --release -p homa-ffi
# Linux: target/release/libhoma_ffi.so
# headers: crates/homa-ffi/include/homa_ffi.h
```

C example: `rust/examples/ffi-client` (CMake, C11, links only `libhoma_ffi`
and `homa_ffi.h`). Supported runtime architectures: Linux x86_64 and
aarch64. The crate compiles on macOS so the ABI tests run there;
`homa_socket_open` then returns `HOMA_ERR_UNSUPPORTED_PLATFORM`.

### Planned .NET 10 mapping (Phase 3, not implemented)

| C | .NET `LibraryImport` |
|---|---|
| `homa_socket_t *` | `SafeHandle` subclass (`HomaSocketHandle`) |
| `homa_message_t *` | `SafeHandle` subclass (`HomaMessageHandle`); `DangerousGetHandle` only inside the wrapper |
| `homa_status_t` / `int32_t` | `int` |
| `uint64_t` (RPC id, cookie, lengths) | `ulong` |
| `uint16_t` (port, family) | `ushort` |
| `const uint8_t *` + `uint64_t len` | `ReadOnlySpan<byte>` (source-generated marshalling) or `byte*` + `nuint` |
| `homa_endpoint_t` (24 bytes, blittable) | `readonly struct` with `LayoutKind.Sequential` |
| `homa_message_info_t` | sequential struct; `payload` becomes `byte*` then `ReadOnlySpan<byte>` of `payload_len` |
| `bool` (`homa_ffi_platform_supported`) | `[return: MarshalAs(UnmanagedType.U1)] bool` |
| `const char *` (`homa_status_name`) | `byte*` / `Marshal.PtrToStringUTF8` (static, never freed) |
| `homa_last_error_message` | `Span<byte>` plus `out ulong required` |
| timeouts | `int` milliseconds; `-1` = infinite. Future `ValueTask` wrappers run the blocking call on a dedicated thread per socket. |

Packaging: `runtimes/linux-x64/native/libhoma_ffi.so` and `linux-arm64`.
Non-Linux RIDs ship no native asset; `homa_ffi_platform_supported` guards
usage. No reflection-based marshalling is required: every public type is
blittable or a pointer plus length.

Open questions deferred to Phase 3: zero-copy views (needs a zero-copy
`homa-rs` first), richer `HOMAIOCINFO` surface, and whether cancellation
should abort or let the kernel time out.
