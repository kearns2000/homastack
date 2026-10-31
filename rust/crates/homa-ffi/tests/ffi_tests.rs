//! ABI tests that do **not** need the Homa kernel module.
//!
//! They call the exported `extern "C"` functions (not just helpers) so a
//! mismatch between `include/homa_ffi.h` and the Rust side fails here.
//! Kernel-dependent coverage lives in `ffi_kernel.rs` and is `#[ignore]`d.

#![allow(clippy::undocumented_unsafe_blocks)]

use std::ffi::{CStr, c_char};
use std::mem::{align_of, offset_of, size_of};
use std::ptr;

use homa_ffi::api::{
    EXPORTED_FUNCTIONS, homa_clear_last_error, homa_ffi_abi_version, homa_ffi_max_message_length,
    homa_ffi_platform_supported, homa_last_error_errno, homa_last_error_message,
    homa_last_error_status, homa_message_free, homa_message_info, homa_message_payload, homa_recv,
    homa_recv_response, homa_reply, homa_reply_to_message, homa_send_request, homa_socket_bind,
    homa_socket_close, homa_socket_info, homa_socket_local_port, homa_socket_open,
    homa_socket_shutdown, homa_status_name,
};
use homa_ffi::errors::{clear_last_error, guard};
use homa_ffi::handles::homa_socket_t;
use homa_ffi::{
    HOMA_ENDPOINT_IPV4, HOMA_ERR_BUFFER_TOO_SMALL, HOMA_ERR_INTERNAL, HOMA_ERR_INVALID_ARGUMENT,
    HOMA_ERR_INVALID_ENDPOINT, HOMA_ERR_MESSAGE_TOO_LARGE, HOMA_ERR_UNSUPPORTED_PLATFORM, HOMA_OK,
    homa_endpoint_t, homa_message_info_t, homa_socket_info_t, homa_socket_options_t,
};

fn last_message() -> String {
    let mut needed = 0u64;
    unsafe { homa_last_error_message(ptr::null_mut(), 0, &raw mut needed) };
    let mut buf = vec![0 as c_char; needed as usize];
    let st = unsafe { homa_last_error_message(buf.as_mut_ptr(), needed, ptr::null_mut()) };
    assert_eq!(st, HOMA_OK, "copying last-error of size {needed}");
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

#[test]
fn abi_version_is_one_through_the_export() {
    assert_eq!(homa_ffi_abi_version(), 1);
    assert_eq!(homa_ffi::HOMA_FFI_ABI_VERSION, 1);
}

#[test]
fn platform_detection_matches_cfg() {
    assert_eq!(homa_ffi_platform_supported(), cfg!(target_os = "linux"));
}

#[test]
fn max_message_length_matches_constant() {
    assert_eq!(homa_ffi_max_message_length(), 1_000_000);
}

#[test]
fn status_name_is_static_and_covers_known_codes() {
    let ok = unsafe { CStr::from_ptr(homa_status_name(HOMA_OK)) };
    assert_eq!(ok.to_str().unwrap(), "HOMA_OK");
    let unk = unsafe { CStr::from_ptr(homa_status_name(12345)) };
    assert_eq!(unk.to_str().unwrap(), "HOMA_ERR_UNKNOWN");
    assert!(!homa_status_name(HOMA_OK).is_null());
}

#[test]
fn rust_struct_layouts_match_documented_sizes() {
    assert_eq!(size_of::<homa_endpoint_t>(), 24);
    assert_eq!(align_of::<homa_endpoint_t>(), 4);
    assert_eq!(size_of::<homa_socket_options_t>(), 24);
    assert_eq!(align_of::<homa_socket_options_t>(), 8);
    assert_eq!(size_of::<homa_socket_info_t>(), 32);
    assert_eq!(size_of::<homa_message_info_t>(), 72);
    assert_eq!(offset_of!(homa_message_info_t, peer), 24);
    assert_eq!(offset_of!(homa_message_info_t, payload), 48);
    assert_eq!(offset_of!(homa_message_info_t, payload_len), 56);
}

#[test]
#[allow(clippy::too_many_lines)]
fn null_and_invalid_arguments_are_rejected() {
    homa_clear_last_error();
    assert_eq!(
        unsafe { homa_socket_open(ptr::null(), ptr::null_mut()) },
        HOMA_ERR_INVALID_ARGUMENT
    );
    assert!(last_message().contains("output pointer is null"));
    assert_eq!(homa_last_error_status(), HOMA_ERR_INVALID_ARGUMENT);

    let mut sock: *mut homa_socket_t = ptr::null_mut();
    let opts = homa_socket_options_t {
        struct_size: 1,
        ..Default::default()
    };
    assert_eq!(
        unsafe { homa_socket_open(&raw const opts, &raw mut sock) },
        HOMA_ERR_INVALID_ARGUMENT
    );
    assert!(sock.is_null());
    assert!(last_message().contains("struct_size"));

    let opts = homa_socket_options_t {
        reserved: 1,
        ..Default::default()
    };
    assert_eq!(
        unsafe { homa_socket_open(&raw const opts, &raw mut sock) },
        HOMA_ERR_INVALID_ARGUMENT
    );

    let opts = homa_socket_options_t {
        flags: 0x100,
        ..Default::default()
    };
    assert_eq!(
        unsafe { homa_socket_open(&raw const opts, &raw mut sock) },
        HOMA_ERR_INVALID_ARGUMENT
    );

    let opts = homa_socket_options_t {
        family: 99,
        ..Default::default()
    };
    assert_eq!(
        unsafe { homa_socket_open(&raw const opts, &raw mut sock) },
        HOMA_ERR_INVALID_ENDPOINT
    );

    let opts = homa_socket_options_t {
        bind_port: 40_000,
        ..Default::default()
    };
    assert_eq!(
        unsafe { homa_socket_open(&raw const opts, &raw mut sock) },
        HOMA_ERR_INVALID_ENDPOINT
    );

    assert_eq!(
        unsafe { homa_socket_bind(ptr::null(), 0, &raw mut sock) },
        HOMA_ERR_INVALID_ENDPOINT
    );

    assert_eq!(
        homa_socket_shutdown(ptr::null_mut()),
        HOMA_ERR_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { homa_socket_local_port(ptr::null(), ptr::null_mut()) },
        HOMA_ERR_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { homa_socket_info(ptr::null(), ptr::null_mut()) },
        HOMA_ERR_INVALID_ARGUMENT
    );

    let mut id = 0u64;
    assert_eq!(
        unsafe {
            homa_send_request(
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                0,
                0,
                0,
                &raw mut id,
            )
        },
        HOMA_ERR_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { homa_recv_response(ptr::null_mut(), 2, -1, ptr::null_mut()) },
        HOMA_ERR_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { homa_recv(ptr::null_mut(), -1, ptr::null_mut()) },
        HOMA_ERR_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { homa_reply(ptr::null_mut(), 1, ptr::null(), ptr::null(), 0) },
        HOMA_ERR_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { homa_reply_to_message(ptr::null_mut(), ptr::null(), ptr::null(), 0) },
        HOMA_ERR_INVALID_ARGUMENT
    );
    assert_eq!(
        unsafe { homa_message_info(ptr::null(), ptr::null_mut()) },
        HOMA_ERR_INVALID_ARGUMENT
    );
}

#[test]
fn payload_length_validation() {
    let mut id = 0u64;
    let dest = {
        let mut e = homa_endpoint_t {
            family: HOMA_ENDPOINT_IPV4,
            port: 5000,
            ..Default::default()
        };
        e.addr[..4].copy_from_slice(&[127, 0, 0, 1]);
        e
    };
    let bytes = [1u8];
    assert_eq!(
        unsafe {
            homa_send_request(
                ptr::null_mut(),
                &raw const dest,
                ptr::null(),
                4,
                0,
                0,
                &raw mut id,
            )
        },
        HOMA_ERR_INVALID_ARGUMENT
    );
    assert!(last_message().contains("payload pointer is null"));
    assert_eq!(
        unsafe {
            homa_send_request(
                ptr::null_mut(),
                &raw const dest,
                bytes.as_ptr(),
                0,
                0,
                0,
                &raw mut id,
            )
        },
        HOMA_ERR_INVALID_ARGUMENT
    );
    assert!(last_message().contains("payload length is 0"));
    assert_eq!(
        unsafe {
            homa_send_request(
                ptr::null_mut(),
                &raw const dest,
                bytes.as_ptr(),
                2_000_000,
                0,
                0,
                &raw mut id,
            )
        },
        HOMA_ERR_MESSAGE_TOO_LARGE
    );
}

#[test]
fn invalid_endpoint_is_rejected_before_the_socket() {
    let mut id = 0u64;
    let dest = homa_endpoint_t {
        family: 99,
        ..Default::default()
    };
    let bytes = [1u8];
    assert_eq!(
        unsafe {
            homa_send_request(
                ptr::null_mut(),
                &raw const dest,
                bytes.as_ptr(),
                1,
                0,
                0,
                &raw mut id,
            )
        },
        HOMA_ERR_INVALID_ENDPOINT
    );
}

#[test]
fn last_error_copy_reports_required_length() {
    homa_clear_last_error();
    let mut needed = 99u64;
    let st = unsafe { homa_last_error_message(ptr::null_mut(), 0, &raw mut needed) };
    assert_eq!(st, HOMA_ERR_BUFFER_TOO_SMALL);
    assert_eq!(needed, 1);
    assert_eq!(homa_last_error_status(), HOMA_OK);
    assert_eq!(homa_last_error_errno(), 0);

    let _ = unsafe { homa_socket_open(ptr::null(), ptr::null_mut()) };
    let mut needed = 0u64;
    let st = unsafe { homa_last_error_message(ptr::null_mut(), 0, &raw mut needed) };
    assert_eq!(st, HOMA_ERR_BUFFER_TOO_SMALL);
    assert!(needed > 1);
    let mut too_small = [0 as c_char; 2];
    let st = unsafe { homa_last_error_message(too_small.as_mut_ptr(), 2, &raw mut needed) };
    assert_eq!(st, HOMA_ERR_BUFFER_TOO_SMALL);
    assert_eq!(homa_last_error_status(), HOMA_ERR_INVALID_ARGUMENT);
}

#[test]
fn message_free_accepts_null_and_payload_of_null_is_empty() {
    unsafe { homa_message_free(ptr::null_mut()) };
    let mut len = 42u64;
    let ptr = unsafe { homa_message_payload(ptr::null(), &raw mut len) };
    assert!(ptr.is_null());
    assert_eq!(len, 0);
}

#[test]
fn close_of_null_is_ok_unknown_handle_is_invalid() {
    assert_eq!(homa_socket_close(ptr::null_mut()), HOMA_OK);
    let forged = 0x0000_0001_0000_0000u64 as *mut homa_socket_t;
    assert_eq!(homa_socket_close(forged), HOMA_ERR_INVALID_ARGUMENT);
    assert!(last_message().contains("not open"));
}

#[test]
fn open_on_this_host_is_either_linux_or_unsupported() {
    let mut sock: *mut homa_socket_t = ptr::dangling_mut();
    let st = unsafe { homa_socket_open(ptr::null(), &raw mut sock) };
    if cfg!(target_os = "linux") {
        // Linux without the module: UNAVAILABLE. Linux with it: OK, then close.
        if st == HOMA_OK {
            assert!(!sock.is_null());
            assert_eq!(homa_socket_close(sock), HOMA_OK);
            assert_eq!(homa_socket_close(sock), HOMA_ERR_INVALID_ARGUMENT);
        } else {
            assert_ne!(st, HOMA_ERR_UNSUPPORTED_PLATFORM);
            assert!(sock.is_null());
        }
    } else {
        assert_eq!(st, HOMA_ERR_UNSUPPORTED_PLATFORM);
        assert!(sock.is_null());
        assert!(last_message().contains("require Linux"));
    }
}

#[test]
fn panic_boundary_does_not_unwind() {
    clear_last_error();
    assert_eq!(guard(|| panic!("ffi-test-panic")), HOMA_ERR_INTERNAL);
    assert_eq!(homa_last_error_status(), HOMA_ERR_INTERNAL);
    assert!(last_message().contains("ffi-test-panic"));
}

#[test]
fn header_declares_every_exported_symbol() {
    let header = include_str!("../include/homa_ffi.h");
    for name in EXPORTED_FUNCTIONS {
        assert!(
            header.contains(&format!("{name}(")),
            "include/homa_ffi.h is missing a declaration of {name}()"
        );
    }
}

/// Compile `homa_ffi.h` as C11 and compare sizes/offsets/constants.
#[cfg(feature = "header-check")]
#[test]
fn header_layout_matches() {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::process::Command;

    const PROBE: &str = r#"
#include <stdio.h>
#include <stddef.h>
#include "homa_ffi.h"
#define P(name, value) printf("%s=%llu\n", name, (unsigned long long)(value))
int main(void) {
    P("HOMA_OK", HOMA_OK);
    P("HOMA_ERR_INVALID_ARGUMENT", HOMA_ERR_INVALID_ARGUMENT);
    P("HOMA_ERR_UNSUPPORTED_PLATFORM", HOMA_ERR_UNSUPPORTED_PLATFORM);
    P("HOMA_ERR_UNAVAILABLE", HOMA_ERR_UNAVAILABLE);
    P("HOMA_ERR_INVALID_ENDPOINT", HOMA_ERR_INVALID_ENDPOINT);
    P("HOMA_ERR_TIMEOUT", HOMA_ERR_TIMEOUT);
    P("HOMA_ERR_INTERRUPTED", HOMA_ERR_INTERRUPTED);
    P("HOMA_ERR_RPC_FAILED", HOMA_ERR_RPC_FAILED);
    P("HOMA_ERR_SHUTDOWN", HOMA_ERR_SHUTDOWN);
    P("HOMA_ERR_NO_MEMORY", HOMA_ERR_NO_MEMORY);
    P("HOMA_ERR_WOULD_BLOCK", HOMA_ERR_WOULD_BLOCK);
    P("HOMA_ERR_ADDRESS_IN_USE", HOMA_ERR_ADDRESS_IN_USE);
    P("HOMA_ERR_PERMISSION_DENIED", HOMA_ERR_PERMISSION_DENIED);
    P("HOMA_ERR_NO_SERVER", HOMA_ERR_NO_SERVER);
    P("HOMA_ERR_UNREACHABLE", HOMA_ERR_UNREACHABLE);
    P("HOMA_ERR_MESSAGE_TOO_LARGE", HOMA_ERR_MESSAGE_TOO_LARGE);
    P("HOMA_ERR_BUFFER_TOO_SMALL", HOMA_ERR_BUFFER_TOO_SMALL);
    P("HOMA_ERR_IO", HOMA_ERR_IO);
    P("HOMA_ERR_INTERNAL", HOMA_ERR_INTERNAL);
    P("HOMA_ENDPOINT_IPV4", HOMA_ENDPOINT_IPV4);
    P("HOMA_ENDPOINT_IPV6", HOMA_ENDPOINT_IPV6);
    P("HOMA_MESSAGE_REQUEST", HOMA_MESSAGE_REQUEST);
    P("HOMA_MESSAGE_RESPONSE", HOMA_MESSAGE_RESPONSE);
    P("HOMA_REQUEST_PRIVATE", HOMA_REQUEST_PRIVATE);
    P("HOMA_SOCKET_SERVER", HOMA_SOCKET_SERVER);
    P("HOMA_FFI_MAX_MESSAGE_LENGTH", HOMA_FFI_MAX_MESSAGE_LENGTH);
    P("sizeof(homa_endpoint_t)", sizeof(homa_endpoint_t));
    P("alignof(homa_endpoint_t)", _Alignof(homa_endpoint_t));
    P("offsetof(homa_endpoint_t,addr)", offsetof(homa_endpoint_t, addr));
    P("sizeof(homa_socket_options_t)", sizeof(homa_socket_options_t));
    P("alignof(homa_socket_options_t)", _Alignof(homa_socket_options_t));
    P("offsetof(homa_socket_options_t,receive_region_bytes)",
      offsetof(homa_socket_options_t, receive_region_bytes));
    P("sizeof(homa_socket_info_t)", sizeof(homa_socket_info_t));
    P("sizeof(homa_message_info_t)", sizeof(homa_message_info_t));
    P("alignof(homa_message_info_t)", _Alignof(homa_message_info_t));
    P("offsetof(homa_message_info_t,peer)", offsetof(homa_message_info_t, peer));
    P("offsetof(homa_message_info_t,payload)", offsetof(homa_message_info_t, payload));
    P("offsetof(homa_message_info_t,payload_len)", offsetof(homa_message_info_t, payload_len));
    P("offsetof(homa_message_info_t,fragments)", offsetof(homa_message_info_t, fragments));
    return 0;
}
"#;

    let include = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include");
    let out_dir =
        std::env::temp_dir().join(format!("homa-ffi-header-check-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let src = out_dir.join("probe.c");
    let bin = out_dir.join("probe");
    std::fs::write(&src, PROBE).unwrap();
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let status = Command::new(&cc)
        .arg("-std=c11")
        .arg("-I")
        .arg(&include)
        .arg("-o")
        .arg(&bin)
        .arg(&src)
        .status()
        .unwrap_or_else(|e| panic!("failed to run `{cc}`: {e}"));
    assert!(status.success(), "C compiler failed compiling homa_ffi.h");
    let output = Command::new(&bin).output().expect("run probe");
    assert!(output.status.success());
    let _ = std::fs::remove_dir_all(&out_dir);

    let got: HashMap<String, u64> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|l| {
            let (k, v) = l.split_once('=').unwrap();
            (k.to_string(), v.parse().unwrap())
        })
        .collect();

    let expect: &[(&str, u64)] = &[
        ("HOMA_OK", 0),
        ("HOMA_ERR_INVALID_ARGUMENT", 1),
        ("HOMA_ERR_UNSUPPORTED_PLATFORM", 2),
        ("HOMA_ERR_UNAVAILABLE", 3),
        ("HOMA_ERR_INVALID_ENDPOINT", 4),
        ("HOMA_ERR_TIMEOUT", 5),
        ("HOMA_ERR_INTERRUPTED", 6),
        ("HOMA_ERR_RPC_FAILED", 7),
        ("HOMA_ERR_SHUTDOWN", 8),
        ("HOMA_ERR_NO_MEMORY", 9),
        ("HOMA_ERR_WOULD_BLOCK", 10),
        ("HOMA_ERR_ADDRESS_IN_USE", 11),
        ("HOMA_ERR_PERMISSION_DENIED", 12),
        ("HOMA_ERR_NO_SERVER", 13),
        ("HOMA_ERR_UNREACHABLE", 14),
        ("HOMA_ERR_MESSAGE_TOO_LARGE", 15),
        ("HOMA_ERR_BUFFER_TOO_SMALL", 16),
        ("HOMA_ERR_IO", 17),
        ("HOMA_ERR_INTERNAL", 99),
        ("HOMA_ENDPOINT_IPV4", 4),
        ("HOMA_ENDPOINT_IPV6", 6),
        ("HOMA_MESSAGE_REQUEST", 1),
        ("HOMA_MESSAGE_RESPONSE", 2),
        ("HOMA_REQUEST_PRIVATE", 1),
        ("HOMA_SOCKET_SERVER", 1),
        ("HOMA_FFI_MAX_MESSAGE_LENGTH", 1_000_000),
        (
            "sizeof(homa_endpoint_t)",
            size_of::<homa_endpoint_t>() as u64,
        ),
        (
            "alignof(homa_endpoint_t)",
            align_of::<homa_endpoint_t>() as u64,
        ),
        ("offsetof(homa_endpoint_t,addr)", 8),
        (
            "sizeof(homa_socket_options_t)",
            size_of::<homa_socket_options_t>() as u64,
        ),
        (
            "alignof(homa_socket_options_t)",
            align_of::<homa_socket_options_t>() as u64,
        ),
        ("offsetof(homa_socket_options_t,receive_region_bytes)", 8),
        (
            "sizeof(homa_socket_info_t)",
            size_of::<homa_socket_info_t>() as u64,
        ),
        (
            "sizeof(homa_message_info_t)",
            size_of::<homa_message_info_t>() as u64,
        ),
        (
            "alignof(homa_message_info_t)",
            align_of::<homa_message_info_t>() as u64,
        ),
        ("offsetof(homa_message_info_t,peer)", 24),
        ("offsetof(homa_message_info_t,payload)", 48),
        ("offsetof(homa_message_info_t,payload_len)", 56),
        ("offsetof(homa_message_info_t,fragments)", 64),
    ];
    for (name, want) in expect {
        assert_eq!(
            got.get(*name).copied(),
            Some(*want),
            "{name}: C header {got:?} vs Rust {want}"
        );
    }
}
