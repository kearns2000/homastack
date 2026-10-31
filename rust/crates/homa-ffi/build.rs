//! Emit the public include path and, on Linux/macOS, restrict the cdylib
//! to the documented `homa_*` symbols.

use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:include={}", manifest.join("include").display());
    println!("cargo:rerun-if-changed=include/homa_ffi.h");
    println!("cargo:rerun-if-changed=homa_ffi.map");
    println!("cargo:rerun-if-changed=homa_ffi.exp");

    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match os.as_str() {
        "linux" => {
            let map = manifest.join("homa_ffi.map");
            println!(
                "cargo:rustc-cdylib-link-arg=-Wl,--version-script={}",
                map.display()
            );
            println!("cargo:rustc-cdylib-link-arg=-Wl,--no-undefined");
        }
        "macos" => {
            let exp = manifest.join("homa_ffi.exp");
            println!(
                "cargo:rustc-cdylib-link-arg=-Wl,-exported_symbols_list,{}",
                exp.display()
            );
        }
        _ => {}
    }
}
