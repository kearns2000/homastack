#!/bin/sh
# Linux cdylib linker for targets that use GNU ld 2.46 or newer.
#
# rustc adds an anonymous --version-script for every cdylib. Combined with
# homa_ffi.map (version node HOMA_FFI_1), aarch64-linux-gnu-ld.bfd fails:
# "anonymous version tag cannot be combined with other version tags".
# Drop rustc's script and keep homa_ffi.map.
#
# Cargo's CARGO_TARGET_<triple>_LINKER overrides cargo:rustc-linker, so the
# aarch64 cross job must set that variable to this script rather than to
# aarch64-linux-gnu-gcc. HOMA_FFI_REAL_LINKER selects the compiler driver
# (default: aarch64-linux-gnu-gcc).
set -eu
real=${HOMA_FFI_REAL_LINKER:-aarch64-linux-gnu-gcc}
list=$(mktemp)
trap 'rm -f "$list"' EXIT
: > "$list"
while [ $# -gt 0 ]; do
  arg=$1
  shift
  case $arg in
    -Wl,--version-script=*|--version-script=*)
      case $arg in
        *homa_ffi.map) printf '%s\n' "$arg" >> "$list" ;;
      esac
      ;;
    --version-script)
      if [ $# -eq 0 ]; then
        break
      fi
      nxt=$1
      shift
      case $nxt in
        *homa_ffi.map)
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
