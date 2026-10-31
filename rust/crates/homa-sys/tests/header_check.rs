//! Independent verification of the hand-written ABI against the vendored
//! upstream `homa.h`, compiled with the system C compiler.
//!
//! Enabled with `cargo test -p homa-sys --features header-check` on Linux.
//! Requires `cc` (gcc or clang) and the Linux UAPI headers (`linux/types.h`,
//! `linux/in.h`, ...), i.e. `linux-libc-dev` on Debian/Ubuntu.

#![cfg(all(target_os = "linux", feature = "header-check"))]

use std::collections::HashMap;
use std::mem::{offset_of, size_of};
use std::path::PathBuf;
use std::process::Command;

use homa_sys::*;

const PROBE: &str = r#"
#include <stdio.h>
#include <stddef.h>
#include <sys/ioctl.h>
#include "homa.h"
#define P(name, value) printf("%s=%llu\n", name, (unsigned long long)(value))
int main(void) {
    P("IPPROTO_HOMA", IPPROTO_HOMA);
    P("HOMA_MAX_MESSAGE_LENGTH", HOMA_MAX_MESSAGE_LENGTH);
    P("HOMA_BPAGE_SIZE", HOMA_BPAGE_SIZE);
    P("HOMA_MAX_BPAGES", HOMA_MAX_BPAGES);
    P("HOMA_MIN_DEFAULT_PORT", HOMA_MIN_DEFAULT_PORT);
    P("HOMA_SENDMSG_PRIVATE", HOMA_SENDMSG_PRIVATE);
    P("SO_HOMA_RCVBUF", SO_HOMA_RCVBUF);
    P("SO_HOMA_SERVER", SO_HOMA_SERVER);
    P("HOMA_ERROR_MSG_SIZE", HOMA_ERROR_MSG_SIZE);
    P("sizeof(homa_sendmsg_args)", sizeof(struct homa_sendmsg_args));
    P("offsetof(homa_sendmsg_args,flags)", offsetof(struct homa_sendmsg_args, flags));
    P("sizeof(homa_recvmsg_args)", sizeof(struct homa_recvmsg_args));
    P("offsetof(homa_recvmsg_args,num_bpages)", offsetof(struct homa_recvmsg_args, num_bpages));
    P("offsetof(homa_recvmsg_args,bpage_offsets)", offsetof(struct homa_recvmsg_args, bpage_offsets));
    P("sizeof(homa_rcvbuf_args)", sizeof(struct homa_rcvbuf_args));
    P("sizeof(homa_abort_args)", sizeof(struct homa_abort_args));
    P("sizeof(homa_rpc_info)", sizeof(struct homa_rpc_info));
    P("offsetof(homa_rpc_info,peer)", offsetof(struct homa_rpc_info, peer));
    P("offsetof(homa_rpc_info,completion_cookie)", offsetof(struct homa_rpc_info, completion_cookie));
    P("offsetof(homa_rpc_info,flags)", offsetof(struct homa_rpc_info, flags));
    P("sizeof(homa_info)", sizeof(struct homa_info));
    P("offsetof(homa_info,port)", offsetof(struct homa_info, port));
    P("offsetof(homa_info,error_msg)", offsetof(struct homa_info, error_msg));
    P("HOMAIOCINFO", HOMAIOCINFO);
    P("HOMAIOCRELEASE", HOMAIOCRELEASE);
    P("HOMAIOCABORT", HOMAIOCABORT);
    P("HOMAIOCFREEZE", HOMAIOCFREEZE);
    return 0;
}
"#;

fn compile_and_run() -> HashMap<String, u64> {
    let include = PathBuf::from(env!("HOMA_SYS_UPSTREAM_INCLUDE"));
    let out_dir =
        std::env::temp_dir().join(format!("homa-sys-header-check-{}", std::process::id()));
    std::fs::create_dir_all(&out_dir).unwrap();
    let src = out_dir.join("probe.c");
    let bin = out_dir.join("probe");
    std::fs::write(&src, PROBE).unwrap();

    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let status = Command::new(&cc)
        .arg("-I")
        .arg(&include)
        .arg("-o")
        .arg(&bin)
        .arg(&src)
        .status()
        .unwrap_or_else(|e| panic!("failed to run `{cc}`: {e}"));
    assert!(status.success(), "C compiler failed; see output above");

    let output = Command::new(&bin).output().expect("run probe");
    assert!(output.status.success());
    let _ = std::fs::remove_dir_all(&out_dir);

    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.trim().parse::<u64>().unwrap()))
        .collect()
}

#[test]
fn rust_abi_matches_upstream_header() {
    let c = compile_and_run();
    let expect = |key: &str, rust_value: u64| {
        let c_value = *c
            .get(key)
            .unwrap_or_else(|| panic!("probe did not print {key}"));
        assert_eq!(
            c_value, rust_value,
            "mismatch for {key}: C={c_value} Rust={rust_value}"
        );
    };

    expect("IPPROTO_HOMA", IPPROTO_HOMA as u64);
    expect("HOMA_MAX_MESSAGE_LENGTH", HOMA_MAX_MESSAGE_LENGTH as u64);
    expect("HOMA_BPAGE_SIZE", HOMA_BPAGE_SIZE as u64);
    expect("HOMA_MAX_BPAGES", HOMA_MAX_BPAGES as u64);
    expect("HOMA_MIN_DEFAULT_PORT", u64::from(HOMA_MIN_DEFAULT_PORT));
    expect("HOMA_SENDMSG_PRIVATE", u64::from(HOMA_SENDMSG_PRIVATE));
    expect("SO_HOMA_RCVBUF", SO_HOMA_RCVBUF as u64);
    expect("SO_HOMA_SERVER", SO_HOMA_SERVER as u64);
    expect("HOMA_ERROR_MSG_SIZE", HOMA_ERROR_MSG_SIZE as u64);

    expect(
        "sizeof(homa_sendmsg_args)",
        size_of::<homa_sendmsg_args>() as u64,
    );
    expect(
        "offsetof(homa_sendmsg_args,flags)",
        offset_of!(homa_sendmsg_args, flags) as u64,
    );
    expect(
        "sizeof(homa_recvmsg_args)",
        size_of::<homa_recvmsg_args>() as u64,
    );
    expect(
        "offsetof(homa_recvmsg_args,num_bpages)",
        offset_of!(homa_recvmsg_args, num_bpages) as u64,
    );
    expect(
        "offsetof(homa_recvmsg_args,bpage_offsets)",
        offset_of!(homa_recvmsg_args, bpage_offsets) as u64,
    );
    expect(
        "sizeof(homa_rcvbuf_args)",
        size_of::<homa_rcvbuf_args>() as u64,
    );
    expect(
        "sizeof(homa_abort_args)",
        size_of::<homa_abort_args>() as u64,
    );
    expect("sizeof(homa_rpc_info)", size_of::<homa_rpc_info>() as u64);
    expect(
        "offsetof(homa_rpc_info,peer)",
        offset_of!(homa_rpc_info, peer) as u64,
    );
    expect(
        "offsetof(homa_rpc_info,completion_cookie)",
        offset_of!(homa_rpc_info, completion_cookie) as u64,
    );
    expect(
        "offsetof(homa_rpc_info,flags)",
        offset_of!(homa_rpc_info, flags) as u64,
    );
    expect("sizeof(homa_info)", size_of::<homa_info>() as u64);
    expect(
        "offsetof(homa_info,port)",
        offset_of!(homa_info, port) as u64,
    );
    expect(
        "offsetof(homa_info,error_msg)",
        offset_of!(homa_info, error_msg) as u64,
    );

    expect("HOMAIOCINFO", HOMAIOCINFO as u64);
    expect("HOMAIOCRELEASE", HOMAIOCRELEASE as u64);
    expect("HOMAIOCABORT", HOMAIOCABORT as u64);
    expect("HOMAIOCFREEZE", HOMAIOCFREEZE as u64);
}
