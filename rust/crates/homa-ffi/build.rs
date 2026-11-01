//! Emit the public include path and, on Linux/macOS, restrict the cdylib
//! to the documented `homa_*` symbols.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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
            let linker = write_linux_linker(&map);
            // rustc always adds its own anonymous --version-script for a
            // cdylib. GNU ld 2.46 errors when that is combined with the named
            // HOMA_FFI_1 script. This wrapper drops rustc's script.
            // CARGO_TARGET_<triple>_LINKER overrides rustc-linker, so the
            // aarch64 CI job sets that variable to link-linux.sh instead.
            println!("cargo:rustc-linker={}", linker.display());
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

/// Linker `rustc` should invoke. Honour Cargo's target linker so the
/// aarch64 cross job still calls `aarch64-linux-gnu-gcc`.
fn linux_linker() -> String {
    let target = env::var("TARGET").unwrap_or_default();
    let key = format!(
        "CARGO_TARGET_{}_LINKER",
        target.to_ascii_uppercase().replace('-', "_")
    );
    println!("cargo:rerun-if-env-changed={key}");
    env::var(key).unwrap_or_else(|_| "cc".to_string())
}

fn write_linux_linker(map: &Path) -> PathBuf {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let wrapper = out_dir.join("link-homa-ffi.sh");
    let real = linux_linker();
    let script = format!(
        r#"#!/bin/sh
set -eu
real={real}
map={map}
list=$(mktemp)
trap 'rm -f "$list"' EXIT
: > "$list"
while [ $# -gt 0 ]; do
  arg=$1
  shift
  case $arg in
    -Wl,--version-script=*|--version-script=*)
      case $arg in
        *"$map") printf '%s\n' "$arg" >> "$list" ;;
      esac
      ;;
    --version-script)
      if [ $# -eq 0 ]; then
        break
      fi
      nxt=$1
      shift
      case $nxt in
        "$map")
          printf '%s\n' --version-script >> "$list"
          printf '%s\n' "$nxt" >> "$list"
          ;;
      esac
      ;;
    *)
      printf '%s\n' "$arg" >> "$list"
      ;;
  esac
done
set --
while IFS= read -r line; do
  set -- "$@" "$line"
done < "$list"
exec "$real" "$@"
"#,
        real = shell_quote(&real),
        map = shell_quote(&map.display().to_string()),
    );
    fs::write(&wrapper, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&wrapper).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&wrapper, perms).unwrap();
    }
    wrapper
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
