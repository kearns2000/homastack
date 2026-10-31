/*
 * ffi-client - C smoke test for libhoma_ffi.so.
 *
 * Uses only homa_ffi.h. Talks to the Rust echo-server:
 *
 *   cargo run -p homa-echo-server -- --port 5000
 *   ./ffi-client --server 127.0.0.1 --port 5000 --payload 'Hello Homa'
 *
 * Exit 0 on a verified echo; non-zero on any failure (including "not Linux").
 */

#include "homa_ffi.h"

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void die_status(const char *what, homa_status_t st)
{
    char buf[512];
    uint64_t needed = 0;
    buf[0] = '\0';
    if (homa_last_error_message(buf, sizeof(buf), &needed) != HOMA_OK) {
        /* Buffer too small, or no text. Do not read an unwritten buf. */
        buf[0] = '\0';
    }
    fprintf(stderr, "%s: %s (%d)", what, homa_status_name(st), (int)st);
    if (buf[0] != '\0') {
        fprintf(stderr, ": %s", buf);
    }
    int32_t errn = homa_last_error_errno();
    if (errn != 0) {
        fprintf(stderr, " (errno %d)", (int)errn);
    }
    fputc('\n', stderr);
    exit(1);
}

static void usage(const char *argv0)
{
    fprintf(stderr,
            "Usage: %s [--server HOST] [--port PORT] [--payload TEXT] "
            "[--timeout-ms MS]\n",
            argv0);
}

static int parse_ipv4(const char *s, uint8_t out[4])
{
    unsigned a, b, c, d;
    char extra;
    if (sscanf(s, "%u.%u.%u.%u%c", &a, &b, &c, &d, &extra) != 4) {
        return -1;
    }
    if (a > 255 || b > 255 || c > 255 || d > 255) {
        return -1;
    }
    out[0] = (uint8_t)a;
    out[1] = (uint8_t)b;
    out[2] = (uint8_t)c;
    out[3] = (uint8_t)d;
    return 0;
}

int main(int argc, char **argv)
{
    const char *host = "127.0.0.1";
    uint16_t port = 5000;
    const char *payload = "Hello Homa";
    int64_t timeout_ms = 5000;

    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--help") == 0 || strcmp(argv[i], "-h") == 0) {
            usage(argv[0]);
            return 0;
        }
        if (i + 1 >= argc) {
            usage(argv[0]);
            return 2;
        }
        if (strcmp(argv[i], "--server") == 0) {
            host = argv[++i];
        } else if (strcmp(argv[i], "--port") == 0) {
            int p = atoi(argv[++i]);
            if (p < 1 || p > 65535) {
                fprintf(stderr, "invalid --port\n");
                return 2;
            }
            port = (uint16_t)p;
        } else if (strcmp(argv[i], "--payload") == 0) {
            payload = argv[++i];
        } else if (strcmp(argv[i], "--timeout-ms") == 0) {
            timeout_ms = (int64_t)atoll(argv[++i]);
        } else {
            usage(argv[0]);
            return 2;
        }
    }

    uint32_t ver = homa_ffi_abi_version();
    printf("homa_ffi_abi_version = %u\n", ver);
    if (ver != 1) {
        fprintf(stderr, "this client requires ABI version 1\n");
        return 1;
    }
    printf("homa_ffi_platform_supported = %s\n",
           homa_ffi_platform_supported() ? "true" : "false");
    printf("homa_ffi_max_message_length = %u\n",
           homa_ffi_max_message_length());

    if (!homa_ffi_platform_supported()) {
        fprintf(stderr,
                "Homa sockets require Linux with the homa kernel module\n");
        return 1;
    }

    homa_socket_options_t opts;
    memset(&opts, 0, sizeof(opts));
    opts.struct_size = (uint32_t)sizeof(opts);
    opts.family = HOMA_ENDPOINT_IPV4;

    homa_socket_t *sock = NULL;
    homa_status_t st = homa_socket_open(&opts, &sock);
    if (st != HOMA_OK) {
        die_status("homa_socket_open", st);
    }

    uint16_t local = 0;
    st = homa_socket_local_port(sock, &local);
    if (st != HOMA_OK) {
        homa_socket_close(sock);
        die_status("homa_socket_local_port", st);
    }
    printf("client port = %u\n", local);

    homa_endpoint_t dest;
    memset(&dest, 0, sizeof(dest));
    dest.family = HOMA_ENDPOINT_IPV4;
    dest.port = port;
    if (parse_ipv4(host, dest.addr) != 0) {
        fprintf(stderr, "this example accepts IPv4 --server only (got %s)\n",
                host);
        homa_socket_close(sock);
        return 2;
    }

    size_t len = strlen(payload);
    if (len == 0) {
        fprintf(stderr, "--payload must not be empty\n");
        homa_socket_close(sock);
        return 2;
    }

    printf("sending %zu bytes to %s:%u (timeout %" PRId64 " ms)\n", len, host,
           port, timeout_ms);

    homa_message_t *msg = NULL;
    st = homa_request(sock, &dest, (const uint8_t *)payload, (uint64_t)len,
                      timeout_ms, &msg);
    if (st != HOMA_OK) {
        homa_socket_close(sock);
        die_status("homa_request", st);
    }

    homa_message_info_t info;
    memset(&info, 0, sizeof(info));
    info.struct_size = (uint32_t)sizeof(info);
    st = homa_message_info(msg, &info);
    if (st != HOMA_OK) {
        homa_message_free(msg);
        homa_socket_close(sock);
        die_status("homa_message_info", st);
    }

    printf("response rpc_id=%" PRIu64 " cookie=%" PRIu64 " kind=%u len=%" PRIu64
           " fragments=%u\n",
           info.rpc_id, info.completion_cookie, info.kind, info.payload_len,
           info.fragments);

    if (info.payload_len != (uint64_t)len ||
        memcmp(info.payload, payload, len) != 0) {
        fprintf(stderr, "response payload MISMATCH (got %" PRIu64 " bytes)\n",
                info.payload_len);
        homa_message_free(msg);
        homa_socket_close(sock);
        return 1;
    }

    printf("response verified\n");
    homa_message_free(msg);
    st = homa_socket_close(sock);
    if (st != HOMA_OK) {
        die_status("homa_socket_close", st);
    }
    return 0;
}
