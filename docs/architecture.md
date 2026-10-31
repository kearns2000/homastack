# HomaStack architecture (Phase 1)

## Layers

```mermaid
flowchart TB
    subgraph user["User Space"]
        direction TB

        app["Applications / Echo Client / Echo Server"]
        rs["homa-rs: HomaSocket, Message, RpcId, Buffer Pool"]
        sys["homa-sys: Native Linux Bindings and Syscall Wrappers"]
        ffi["homa-ffi: Stable C ABI - Phase 2"]
        net["Homa.NET: Managed .NET 10 API - Phase 3"]
        rpc["Homa.Rpc: RPC Framework - Phase 4"]

        app --> rs
        rs --> sys

        rpc --> net
        net --> ffi
        ffi --> rs
    end

    subgraph kernel["Linux Kernel"]
        mod["Homa Kernel Module"]
        protocol["RPC State, Retransmission, Grants and Priorities"]
        ip["IP and Network Interface"]

        mod --> protocol
        protocol --> ip
    end

    sys -->|"Linux Socket API"| mod
    mod -->|"Receive Buffer Management"| sys
```

The Rust transport integrates directly with the existing Linux Homa kernel
module through its socket API. The future .NET components consume the Rust
transport through a stable C ABI; they never talk to the kernel themselves.

| Component | Language | Responsibility |
|---|---|---|
| homa-sys | Rust | Low-level Linux Homa socket bindings and native interoperability |
| homa-rs | Rust | Safe Rust API, RPC messages, buffer ownership and error handling |
| homa-ffi (planned; placeholder crate only) | Rust | Stable C ABI exposing the Rust transport to other languages |
| Homa.NET (planned) | C# / .NET 10 | Managed transport API, native handles, asynchronous operations and cancellation |
| Homa.Rpc, Homa.Rpc.Hosting (planned) | C# / .NET 10 | RPC clients, handlers, serialization, middleware and observability; `Hosting` adds DI and `IHostedService` integration |
| Homa kernel module | C / Linux | Wire protocol, reliability, grants, packet scheduling and congestion control |

Only `homa-sys`, `homa-rs` and the echo examples are implemented in Phase 1.
`homa-ffi` (Phase 2) exists as a crate that exports an ABI version number and
a platform probe and nothing else; `Homa.NET` (Phase 3) and `Homa.Rpc` /
`Homa.Rpc.Hosting` (Phase 4) have no code yet.

### Integration flow

Two application paths are intended:

- **Rust application:**
  Rust Application → homa-rs → homa-sys → Linux Homa kernel module → Network
- **.NET application:**
  .NET Application → Homa.Rpc → Homa.NET → homa-ffi → homa-rs → homa-sys →
  Linux Homa kernel module → Network

In both paths the Linux Homa kernel module is an **external dependency**
maintained by the upstream Homa project
([PlatformLab/HomaModule](https://github.com/PlatformLab/HomaModule)); it must
be built and loaded separately (see `linux-setup.md`). HomaStack does not
implement the Homa wire protocol. The Rust crates only drive the module's
socket interface: they create sockets, register the receive region, build the
`sendmsg`/`recvmsg` control blocks, track and return buffer pages, and
translate results into Rust types. Everything on the wire is the kernel's job
(see "What stays in the kernel" below).

### `homa-sys`

- `abi.rs`: `#[repr(C)]` structs and constants transcribed from the pinned
  `homa.h`, with compile-time `size_of`/`offset_of` assertions and the ioctl
  request numbers computed with the asm-generic `_IOC` encoding. Platform
  independent, so it compiles and is unit-tested on macOS too.
- `region.rs`: `MappedRegion`, an RAII anonymous `mmap` used as the receive
  pool. Unix.
- `sys.rs`: one safe function per kernel entry point (`socket`, `bind`,
  `set_rcvbuf_region` (unsafe), `set_server`, `sendmsg`, `recvmsg`,
  `release_bpages`, `info`, `abort`, `shutdown`, `poll_readable`). Linux only.
  Every `unsafe` block carries a `SAFETY:` comment; `unsafe_op_in_unsafe_fn`
  is denied workspace-wide.
- `tests/header_check.rs` (feature `header-check`): compiles the vendored
  header with `cc` and compares every number with the Rust side.

### `homa-rs`

- `transport.rs`: `RpcId` (parity rules: odd = server side/request, even =
  client side/response), `MessageKind`, `Message` (owned payload, peer,
  cookie, fragment count).
- `addr.rs`: `SocketAddr` ⇄ `sockaddr_in`/`sockaddr_in6`, family matching,
  resolution.
- `buffer.rs`: `ReceiveBuffer` (owns the region), `FragmentLayout`
  (validates the kernel's `bpage_offsets` for a given length *before* any
  memory is touched: count = ⌈len/64 KiB⌉, non-final fragments aligned, all
  inside the region, no overlap), `copy_out` (the only reader of the region),
  `BpageQueue` (pages owned by the application awaiting return).
- `error.rs`: `HomaError` (keeps the `io::Error`, adds Homa's `error_msg`
  when the kernel supplies one) and `ErrorKind` classification of `errno`.
- `socket.rs`: `HomaSocket`. Creation order: map region → `socket()` →
  `bind()` → `SO_HOMA_RCVBUF` → `SO_HOMA_SERVER` → `HOMAIOCINFO` (actual
  port). Drop order: release pending bpages → close fd → unmap.

## Message flow

```mermaid
sequenceDiagram
    participant C as echo-client (homa-rs)
    participant KC as kernel (client host)
    participant KS as kernel (server host)
    participant S as echo-server (homa-rs)

    C->>KC: sendmsg(msg_name=server, iov=payload,<br/>control={id:0, cookie, PRIVATE}, controllen=0)
    KC-->>C: returns 0, control.id = 2k (even)
    KC->>KS: DATA packets (plus grants for long messages)
    KS->>KS: allocate bpages in S's region,<br/>copy payload, queue RPC 2k+1
    S->>KS: recvmsg(control={id:0, bpages to recycle})
    KS-->>S: len, control.id = 2k+1 (odd), bpage_offsets[], msg_name = client
    S->>S: FragmentLayout::validate, copy_out to owned Vec,<br/>queue bpages for return
    S->>KS: sendmsg(msg_name=client, iov=payload,<br/>control={id: 2k+1})
    KS->>KC: DATA packets
    KC->>KC: allocate bpages in C's region, copy, complete RPC 2k
    C->>KC: recvmsg(control={id: 2k})
    KC-->>C: len, control.id = 2k, cookie, bpage_offsets[], msg_name = server
    C->>C: validate, copy_out, verify bytes
    C->>KC: next recvmsg carries the previous bpages back
```

## Ownership of receive buffers

1. `ReceiveBuffer::new` maps N × 64 KiB (default 64 MiB), zeroed, page aligned.
2. `set_rcvbuf_region` hands it to the kernel. From now on the kernel may
   write anywhere in it. Rust never creates a `&[u8]` into it except
   transiently in `copy_out`, and only over bpages the kernel just assigned to
   this thread's message.
3. After a successful `recvmsg`, the listed bpages belong to the application.
   `recv_raw` validates the layout, copies the payload, and pushes the offsets
   into `BpageQueue` - on *every* path, including validation failure.
4. The next `recvmsg` on the socket (any thread) carries up to 16 of those
   offsets back in `num_bpages`/`bpage_offsets`. The kernel frees them before
   it waits, even if the wait then fails with `EINTR`/`EAGAIN`, and reports
   `num_bpages = 0` on every error path (the pinned `homa_recvmsg` zeroes it
   before checking the free's result, contrary to what `recvmsg.2` says).
   `recv_raw` still re-queues offsets if a kernel ever reports
   `num_bpages != 0` on error, and logs a warning when a release is rejected
   with `EINVAL`, which can only be caused by a bookkeeping bug in this crate.
5. `release_pending()` or `Drop` returns anything left via `HOMAIOCRELEASE`.
   `Drop` then closes the fd, and only then unmaps the region, so the kernel
   never references freed memory.

A `Message` therefore never aliases the region and has no lifetime parameter.
The cost is one copy per message; that is the documented Phase 1 trade-off.

## Threading

`HomaSocket` is `Send + Sync`; all methods take `&self`. The only shared
mutable state is the bpage queue behind a `Mutex`. Multiple threads can block
in `recv` simultaneously (the kernel hands each ready RPC to exactly one
waiter) and others can send concurrently. Each receive uses its own
`homa_recvmsg_args` on the stack, so there is no cross-thread sharing of
kernel-visible structures.

## Why synchronous first

Homa's `recvmsg` is already message-granular and `sendmsg` returns once the
message is queued, so a blocking API maps directly onto the kernel calls and
has no hidden state machine. The socket exposes `AsFd`, `try_recv`,
`recv_timeout` and `poll_readable`; an async layer can register the fd with
an reactor (shared messages) and use a small poller task for private responses
without changing `Message`, `RpcId` or the buffer ownership model.

## What stays in the kernel

Packet formats, unscheduled bytes and grants, priority assignment, pacing,
retransmission and timeouts, RPC id allocation, server-side RPC bookkeeping
until the response is acknowledged, bpage allocation inside the region, and
the copy from packet buffers into the region. See upstream `protocol.md` and
the man pages vendored in `rust/crates/homa-sys/upstream/`.
