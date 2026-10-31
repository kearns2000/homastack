# Native interop: the kernel ABI and the planned C ABI

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
| `HOMA_MIN_DEFAULT_PORT` | 0x8000 | ports ≥ this are auto-assigned; `bind` accepts only 1–32767 |
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

## Part B - Planned stable C ABI (`homa-ffi`, Phase 2)

Design goals: flat, versioned, no Rust types across the boundary, no callbacks
in v1, every allocation owned by one side with an explicit free.

```c
// Proposed surface (not implemented in Phase 1)
uint32_t homa_ffi_abi_version(void);               // implemented (returns 0)
bool     homa_ffi_platform_supported(void);        // implemented

typedef struct homa_socket homa_socket_t;          // opaque

int32_t homa_socket_open(uint16_t family, uint16_t bind_port,
                         uint64_t region_bytes, homa_socket_t **out);
void    homa_socket_close(homa_socket_t *s);
int32_t homa_socket_shutdown(homa_socket_t *s);
uint16_t homa_socket_port(const homa_socket_t *s);

// Requests
int32_t homa_send_request(homa_socket_t *s, const uint8_t *addr, uint32_t addr_len,
                          const uint8_t *payload, uint32_t len,
                          uint64_t cookie, uint32_t flags, uint64_t *rpc_id);
int32_t homa_recv_response(homa_socket_t *s, uint64_t rpc_id, int32_t timeout_ms,
                           homa_message_t **msg);
// Shared receive + reply
int32_t homa_recv(homa_socket_t *s, int32_t timeout_ms, homa_message_t **msg);
int32_t homa_reply(homa_socket_t *s, uint64_t rpc_id, const uint8_t *addr,
                   uint32_t addr_len, const uint8_t *payload, uint32_t len);

// Messages (owned by native side; free exactly once)
const uint8_t *homa_message_data(const homa_message_t *m, uint32_t *len);
uint64_t homa_message_id(const homa_message_t *m);
uint64_t homa_message_cookie(const homa_message_t *m);
int32_t  homa_message_peer(const homa_message_t *m, uint8_t *addr, uint32_t *addr_len);
void     homa_message_free(homa_message_t *m);

// Errors: negative errno-style codes; details via thread-local last error.
int32_t homa_last_error_message(char *buf, uint32_t len);
```

.NET side (`Homa.NET`):

- `LibraryImport` (source-generated P/Invoke) with `SafeHandle` subclasses for
  `homa_socket_t` and `homa_message_t`; `Message.Span` exposes the native bytes
  until disposal, or copies into a `byte[]`/`Memory<byte>` for the simple API.
- `HomaSocket` (sync) mirrors `homa-rs`; `HomaClient` adds `Task`-based
  `RequestAsync` by running the blocking receive on a dedicated thread per
  socket (Phase 2) and later on a native poller once async lands in Rust.
- Errors map to `HomaException { ErrorKind, Errno, Detail }` with the same
  classification as `homa_rs::ErrorKind`.
- Packaging: `runtimes/linux-x64/native/libhoma_ffi.so` and
  `linux-arm64`; non-Linux RIDs ship no native asset and
  `homa_ffi_platform_supported` guards usage.

Open questions for Phase 2: whether to expose zero-copy message views (needs a
zero-copy `homa-rs` first), how to surface `HOMAIOCINFO` for diagnostics, and
cancellation semantics (abort vs. let the kernel time out).
