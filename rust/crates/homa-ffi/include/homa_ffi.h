#ifndef HOMA_FFI_H
#define HOMA_FFI_H

/*
 * homa_ffi.h - public C ABI of libhoma_ffi.so (HomaStack Phase 2).
 *
 * Version: 1. Call homa_ffi_abi_version() and refuse to proceed if it is
 * not 1. Additive later versions keep this header's layouts and symbols
 * and only append fields (after a larger struct_size) or new functions.
 *
 * This header is C11. Include it from C++ as well; the declarations are
 * wrapped in extern "C".
 *
 * Platforms: Linux x86_64 and aarch64 for sockets. The library builds
 * elsewhere so hosts can be developed on macOS/Windows; there
 * homa_ffi_platform_supported() is false and homa_socket_open() returns
 * HOMA_ERR_UNSUPPORTED_PLATFORM.
 *
 * Calling convention: the System V AMD64 / AAPCS64 C ABI (what Rust
 * `extern "C"` produces on those targets). No stdcall, no name mangling.
 *
 * Ownership, in brief (see each function):
 *   - homa_socket_t * is an opaque *key*, never a pointer to dereference.
 *     Close it with homa_socket_close. Use after close is safe: it returns
 *     HOMA_ERR_INVALID_ARGUMENT; it cannot use-after-free.
 *   - homa_message_t * owns a copied payload. Free exactly once with
 *     homa_message_free. Do not pass it to free(3). Double-free is
 *     undefined (same class of bug as free(3)).
 *   - Payloads are raw bytes with an explicit length; they may contain NUL
 *     and are never C strings.
 *   - Error text is copied into a caller-owned buffer.
 *
 * Not zero-copy. Not an implementation of the Homa wire protocol: every
 * socket call is forwarded to homa-rs, which drives the Linux Homa kernel
 * module.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* =========================================================================
 * Status codes (int32_t). 0 is success. Numeric values are part of ABI 1
 * and will not be reused for a different meaning.
 * ========================================================================= */

typedef int32_t homa_status_t;

#define HOMA_OK                       0
#define HOMA_ERR_INVALID_ARGUMENT     1
#define HOMA_ERR_UNSUPPORTED_PLATFORM 2
#define HOMA_ERR_UNAVAILABLE          3
#define HOMA_ERR_INVALID_ENDPOINT     4
#define HOMA_ERR_TIMEOUT              5
#define HOMA_ERR_INTERRUPTED          6
#define HOMA_ERR_RPC_FAILED           7
#define HOMA_ERR_SHUTDOWN             8
#define HOMA_ERR_NO_MEMORY            9
#define HOMA_ERR_WOULD_BLOCK          10
#define HOMA_ERR_ADDRESS_IN_USE       11
#define HOMA_ERR_PERMISSION_DENIED    12
#define HOMA_ERR_NO_SERVER            13
#define HOMA_ERR_UNREACHABLE          14
#define HOMA_ERR_MESSAGE_TOO_LARGE    15
#define HOMA_ERR_BUFFER_TOO_SMALL     16
#define HOMA_ERR_IO                   17
#define HOMA_ERR_INTERNAL             99

/* Address families for homa_endpoint_t.family / homa_socket_options_t.family. */
#define HOMA_ENDPOINT_IPV4 4
#define HOMA_ENDPOINT_IPV6 6

/* homa_message_info_t.kind */
#define HOMA_MESSAGE_REQUEST  1
#define HOMA_MESSAGE_RESPONSE 2

/* homa_send_request flags */
#define HOMA_REQUEST_PRIVATE 1u

/* homa_socket_options_t.flags */
#define HOMA_SOCKET_SERVER 1u

/* Largest payload Homa accepts. Also returned by homa_ffi_max_message_length. */
#define HOMA_FFI_MAX_MESSAGE_LENGTH 1000000u

/* timeout_ms < 0 means "block until completion or shutdown". */
#define HOMA_TIMEOUT_INFINITE ((int64_t)-1)

/* =========================================================================
 * Opaque handles
 * ========================================================================= */

/*
 * Socket handle. The bit pattern is a generation-checked registry key, not
 * a heap pointer. Never dereference it. Never pass it to free(3).
 */
typedef struct homa_socket homa_socket_t;

/*
 * Native-owned received message. The payload pointer from
 * homa_message_info / homa_message_payload is valid until
 * homa_message_free. Free exactly once; null is ignored.
 */
typedef struct homa_message homa_message_t;

/* =========================================================================
 * Public structures. All are #[repr(C)] on the Rust side; sizes and
 * offsets are asserted at compile time and re-checked by the header-check
 * test against this header. Callers set struct_size to sizeof(...) so a
 * newer library can reject (or, later, accept) an older layout.
 * ========================================================================= */

/*
 * IP endpoint that does not depend on sockaddr.
 *
 * addr is in network byte order. IPv4 uses addr[0..4]; addr[4..16] and
 * scope_id must be 0. port is in host byte order. scope_id is the IPv6
 * interface index.
 */
typedef struct homa_endpoint {
    uint16_t family;
    uint16_t port;
    uint32_t scope_id;
    uint8_t  addr[16];
} homa_endpoint_t;

/*
 * Arguments for homa_socket_open / homa_socket_bind.
 * reserved must be 0. receive_region_bytes 0 means the default (64 MiB).
 */
typedef struct homa_socket_options {
    uint32_t struct_size;
    uint16_t family;
    uint16_t bind_port;
    uint64_t receive_region_bytes;
    uint32_t flags;
    uint32_t reserved;
} homa_socket_options_t;

/* Diagnostic snapshot written by homa_socket_info. */
typedef struct homa_socket_info {
    uint32_t struct_size;
    uint16_t port;
    uint16_t family;
    uint32_t active_rpcs;
    uint32_t pending_bpages;
    uint64_t buffer_available_bytes;
    uint64_t receive_region_bytes;
} homa_socket_info_t;

/*
 * Metadata of a received message. payload points at memory owned by the
 * homa_message_t and is valid until homa_message_free. Never a C string.
 */
typedef struct homa_message_info {
    uint32_t          struct_size;
    uint32_t          kind;
    uint64_t          rpc_id;
    uint64_t          completion_cookie;
    homa_endpoint_t   peer;
    const uint8_t    *payload;
    uint64_t          payload_len;
    uint32_t          fragments;
    uint32_t          reserved;
} homa_message_info_t;

/* =========================================================================
 * Library information
 * ========================================================================= */

/* ABI version of this library. ABI 1 is the first functional interface. */
uint32_t homa_ffi_abi_version(void);

/*
 * true if this *build* can create Homa sockets (Linux). Whether the kernel
 * module is loaded is only known when homa_socket_open runs
 * (HOMA_ERR_UNAVAILABLE).
 */
bool homa_ffi_platform_supported(void);

/* HOMA_FFI_MAX_MESSAGE_LENGTH (1 000 000). */
uint32_t homa_ffi_max_message_length(void);

/*
 * Static, NUL-terminated name of a status code ("HOMA_OK", ...,
 * "HOMA_ERR_UNKNOWN"). Never null; valid for the process lifetime; do not
 * free.
 */
const char *homa_status_name(homa_status_t status);

/* =========================================================================
 * Last error (thread-local)
 *
 * Every function that returns a status other than HOMA_OK records a
 * description for the calling thread before returning: the status, the
 * underlying errno (or 0), and a UTF-8 message. The record is unchanged by
 * successful calls and by homa_last_error_message when the buffer is too
 * small, so the caller can retry. It is replaced by the next failing call
 * on the same thread, or cleared by homa_clear_last_error. Threads do not
 * share a record.
 * ========================================================================= */

/*
 * Copy the message as a NUL-terminated string into buf.
 * Writes the required size including the NUL to *required_len when that
 * pointer is non-null. Returns HOMA_ERR_BUFFER_TOO_SMALL without changing
 * the record if buf is null or buf_len is too small. With no recorded
 * error the message is empty (required_len == 1).
 */
homa_status_t homa_last_error_message(char *buf, uint64_t buf_len,
                                      uint64_t *required_len);

/* Status of this thread's last recorded error (HOMA_OK if none). */
homa_status_t homa_last_error_status(void);

/* OS errno of this thread's last recorded error (0 if none or not OS). */
int32_t homa_last_error_errno(void);

void homa_clear_last_error(void);

/* =========================================================================
 * Socket lifecycle
 *
 * Threading: a handle may be used from any number of threads at once.
 * The kernel still allows only one waiter per private RPC id. Several
 * threads may call homa_recv concurrently; each shared message is
 * delivered to exactly one. A second waiter on the same private RPC id
 * gets HOMA_ERR_INVALID_ARGUMENT and does not cancel the first waiter's
 * RPC. homa_socket_close while other threads are inside calls shuts the
 * socket down (they return HOMA_ERR_SHUTDOWN) and releases the handle. If
 * that shutdown fails, close still releases the handle and returns the
 * shutdown status; in-flight waits may not have been woken.
 * ========================================================================= */

/*
 * Create a socket. options may be null (IPv4, auto port, 64 MiB region,
 * requests not accepted). On success *out is a handle that must be passed
 * to homa_socket_close. *out is set to null before any work, so a caller
 * that ignores the status never sees a stale pointer.
 */
homa_status_t homa_socket_open(const homa_socket_options_t *options,
                               homa_socket_t **out);

/*
 * Create a socket bound to server port `port` (1..=32767). Other fields
 * come from options (nullable, as for homa_socket_open).
 */
homa_status_t homa_socket_bind(const homa_socket_options_t *options,
                               uint16_t port, homa_socket_t **out);

/*
 * Close a socket. The handle is invalid afterwards. Closing null is
 * HOMA_OK; closing an already-closed or unknown handle is
 * HOMA_ERR_INVALID_ARGUMENT.
 */
homa_status_t homa_socket_close(homa_socket_t *socket);

/*
 * Disable the socket: blocked receivers fail with HOMA_ERR_SHUTDOWN.
 * The handle stays valid until homa_socket_close.
 */
homa_status_t homa_socket_shutdown(homa_socket_t *socket);

/* Local Homa port (bound, or auto-assigned >= 32768). */
homa_status_t homa_socket_local_port(const homa_socket_t *socket,
                                     uint16_t *out_port);

/* Diagnostic snapshot. Caller sets info->struct_size = sizeof(*info). */
homa_status_t homa_socket_info(const homa_socket_t *socket,
                               homa_socket_info_t *info);

/* =========================================================================
 * Messaging
 * ========================================================================= */

/*
 * Send a request without waiting. *out_rpc_id receives the kernel RPC id.
 * flags is 0 or HOMA_REQUEST_PRIVATE. completion_cookie is echoed on the
 * response. payload is copied by the kernel before this returns; it may
 * contain NUL.
 */
homa_status_t homa_send_request(homa_socket_t *socket,
                                const homa_endpoint_t *dest,
                                const uint8_t *payload, uint64_t payload_len,
                                uint64_t completion_cookie, uint32_t flags,
                                uint64_t *out_rpc_id);

/*
 * Wait for the response to private request rpc_id.
 * timeout_ms < 0 blocks; >= 0 polls until the deadline and on expiry
 * aborts the RPC (HOMA_ERR_TIMEOUT). A 0 deadline is "one attempt, then
 * give up". EINTR and a spurious EAGAIN are retried and do not abort.
 * At most one thread may wait on a given rpc_id. A second waiter, or a
 * wait on an id that was not sent as a private request, returns
 * HOMA_ERR_INVALID_ARGUMENT and does not abort the live RPC.
 * The returned message must be freed.
 */
homa_status_t homa_recv_response(homa_socket_t *socket, uint64_t rpc_id,
                                 int64_t timeout_ms,
                                 homa_message_t **out_message);

/*
 * Private request + wait, one call. Timeout as for homa_recv_response.
 */
homa_status_t homa_request(homa_socket_t *socket, const homa_endpoint_t *dest,
                           const uint8_t *payload, uint64_t payload_len,
                           int64_t timeout_ms, homa_message_t **out_message);

/*
 * Next shared message: an incoming request, or the response to a
 * non-private request. timeout_ms < 0 blocks; 0 is non-blocking
 * (HOMA_ERR_WOULD_BLOCK); > 0 waits and returns HOMA_ERR_TIMEOUT without
 * consuming or aborting anything.
 */
homa_status_t homa_recv(homa_socket_t *socket, int64_t timeout_ms,
                        homa_message_t **out_message);

/*
 * Reply to request rpc_id (the odd id from homa_message_info_t) at peer.
 * Replying to an RPC the kernel no longer knows succeeds silently.
 */
homa_status_t homa_reply(homa_socket_t *socket, uint64_t rpc_id,
                         const homa_endpoint_t *peer, const uint8_t *payload,
                         uint64_t payload_len);

/* homa_reply using the id and peer of a received request message. */
homa_status_t homa_reply_to_message(homa_socket_t *socket,
                                    const homa_message_t *request,
                                    const uint8_t *payload,
                                    uint64_t payload_len);

/*
 * Abort outgoing RPC rpc_id, or every outgoing RPC if rpc_id is 0.
 * A late response is discarded. An id the kernel does not know returns
 * HOMA_ERR_INVALID_ARGUMENT. A module built with upstream strip.py has no
 * HOMAIOCABORT; every abort then returns that same status and the RPC
 * stays until the kernel's own timeout (see the README).
 */
homa_status_t homa_abort_rpc(homa_socket_t *socket, uint64_t rpc_id);

/* =========================================================================
 * Messages
 * ========================================================================= */

/*
 * Fill info (caller sets info->struct_size). info->payload stays valid
 * until homa_message_free(message).
 */
homa_status_t homa_message_info(const homa_message_t *message,
                                homa_message_info_t *info);

/*
 * Payload pointer and length (same bytes as homa_message_info).
 * Returns null and writes 0 if message is null. out_len may be null.
 */
const uint8_t *homa_message_payload(const homa_message_t *message,
                                    uint64_t *out_len);

/*
 * Release a message. Null is ignored. Each message must be freed exactly
 * once; the payload pointer is invalid afterwards. Do not call free(3).
 */
void homa_message_free(homa_message_t *message);

#ifdef __cplusplus
}
#endif

#endif /* HOMA_FFI_H */
