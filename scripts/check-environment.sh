#!/usr/bin/env bash
# check-environment.sh - verify that this machine can build HomaStack and run
# Homa sockets. Read-only: it never installs packages, loads modules or changes
# networking. Exit code 0 = Homa sockets usable; 1 = build-only; 2 = cannot
# build.
#
# Usage: scripts/check-environment.sh [--quiet]

set -u

QUIET=0
[[ "${1:-}" == "--quiet" ]] && QUIET=1

ok()   { printf '  [ OK ] %s\n' "$*"; }
warn() { printf '  [WARN] %s\n' "$*"; }
fail() { printf '  [FAIL] %s\n' "$*"; }
info() { [[ $QUIET -eq 1 ]] || printf '         %s\n' "$*"; }

BUILD_OK=1
HOMA_OK=1
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PINNED_REV="4bd0275714439da2955670a6c438f6c23304d785"

echo "HomaStack environment check"
echo "==========================="

# ---------------------------------------------------------------------------
echo "1. Operating system"
OS="$(uname -s)"
case "$OS" in
  Linux)
    ok "Linux ($(uname -m))"
    if grep -qi microsoft /proc/version 2>/dev/null; then
      warn "WSL detected. The WSL2 kernel does not ship Homa; a custom WSL kernel build is required. Not supported here."
      HOMA_OK=0
    fi
    if [[ -f /.dockerenv ]] || grep -qE '(docker|containerd|kubepods)' /proc/1/cgroup 2>/dev/null; then
      warn "Container detected. Homa must be loaded on the *host* kernel; containers cannot insmod. Sockets may still work if the host has Homa."
    fi
    ;;
  Darwin)
    fail "macOS: Homa sockets are not available. You can build and run unit tests only."
    HOMA_OK=0
    ;;
  *)
    fail "$OS: unsupported for Homa sockets."
    HOMA_OK=0
    ;;
esac

# ---------------------------------------------------------------------------
echo "2. Kernel"
if [[ "$OS" == "Linux" ]]; then
  KREL="$(uname -r)"
  ok "running kernel $KREL"
  info "Upstream HomaModule main (rev ${PINNED_REV:0:12}) is tested on Linux 6.17.x; branches exist for RHEL 8/9.5 and older kernels."
  if [[ -d "/lib/modules/$KREL/build" ]]; then
    ok "kernel headers present at /lib/modules/$KREL/build (needed to compile HomaModule)"
  else
    warn "kernel headers missing: /lib/modules/$KREL/build"
    info "Debian/Ubuntu: sudo apt install linux-headers-$KREL   Fedora/RHEL: sudo dnf install kernel-devel-$KREL"
  fi
fi

# ---------------------------------------------------------------------------
echo "3. Rust toolchain"
if command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1; then
  ok "$(rustc --version) / $(cargo --version)"
  if command -v rustup >/dev/null 2>&1; then
    WANT="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$ROOT/rust/rust-toolchain.toml" 2>/dev/null)"
    if [[ -n "$WANT" ]] && rustup toolchain list | grep -q "^$WANT"; then
      ok "pinned toolchain $WANT installed"
    elif [[ -n "$WANT" ]]; then
      warn "pinned toolchain $WANT not installed; rustup will fetch it on first cargo invocation in rust/"
    fi
  fi
else
  fail "cargo/rustc not found. Install from https://rustup.rs"
  BUILD_OK=0
fi

# ---------------------------------------------------------------------------
echo "4. Native build dependencies"
if command -v cc >/dev/null 2>&1 || command -v gcc >/dev/null 2>&1 || command -v clang >/dev/null 2>&1; then
  ok "C compiler available ($(cc --version 2>/dev/null | head -1 || echo cc))"
else
  warn "no C compiler (cc/gcc/clang): needed for the optional 'header-check' test and for building HomaModule"
fi
if [[ "$OS" == "Linux" ]]; then
  if [[ -f /usr/include/linux/types.h ]]; then
    ok "Linux UAPI headers present (/usr/include/linux)"
  else
    warn "Linux UAPI headers missing (linux-libc-dev / kernel-headers): needed for the 'header-check' test"
  fi
  for t in make; do
    command -v "$t" >/dev/null 2>&1 && ok "$t available" || warn "$t missing (needed to build HomaModule)"
  done
fi

# ---------------------------------------------------------------------------
echo "5. Homa kernel module"
if [[ "$OS" == "Linux" ]]; then
  if [[ -r /proc/modules ]] && grep -q '^homa ' /proc/modules; then
    ok "homa module is loaded"
    [[ -d /proc/sys/net/homa ]] && ok "/proc/sys/net/homa present"
    [[ -r /proc/net/homa_metrics ]] && ok "/proc/net/homa_metrics readable"
  else
    fail "homa module is NOT loaded"
    HOMA_OK=0
    if modinfo homa >/dev/null 2>&1; then
      info "homa.ko is installed; load it with: sudo modprobe homa"
    else
      KO="$(find "$HOME" / -maxdepth 4 -name 'homa.ko' -not -path '*/proc/*' 2>/dev/null | head -1)"
      if [[ -n "$KO" ]]; then
        info "found a built module at $KO; load it with: sudo insmod $KO"
      else
        info "build it from https://github.com/PlatformLab/HomaModule (see docs/linux-setup.md), then: sudo insmod homa.ko"
      fi
    fi
  fi
fi

# ---------------------------------------------------------------------------
echo "6. Can this user create a Homa socket?"
if [[ "$OS" == "Linux" ]] && command -v python3 >/dev/null 2>&1; then
  RESULT="$(python3 - <<'PY' 2>&1
import socket, errno, sys
IPPROTO_HOMA = 146
try:
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, IPPROTO_HOMA)
    s.close()
    print("OK")
except OSError as e:
    print(f"ERR {e.errno} {errno.errorcode.get(e.errno, '?')} {e.strerror}")
PY
)"
  case "$RESULT" in
    OK) ok "socket(AF_INET, SOCK_DGRAM, IPPROTO_HOMA) succeeded" ;;
    "ERR 93"*|*EPROTONOSUPPORT*) fail "socket() -> EPROTONOSUPPORT: module not loaded"; HOMA_OK=0 ;;
    "ERR 13"*|"ERR 1 "*) fail "socket() -> permission denied ($RESULT). Check capabilities/seccomp (containers often block unknown protocols)."; HOMA_OK=0 ;;
    *) fail "socket() failed: $RESULT"; HOMA_OK=0 ;;
  esac
elif [[ "$OS" == "Linux" ]]; then
  warn "python3 not found; skipping socket probe (build and run 'cargo run -p homa-echo-server' to test)"
fi

# ---------------------------------------------------------------------------
echo
if [[ $BUILD_OK -eq 0 ]]; then
  echo "RESULT: cannot build (fix FAIL items above)."
  exit 2
elif [[ $HOMA_OK -eq 1 ]]; then
  echo "RESULT: Homa sockets are usable. Run the echo server/client and the integration tests:"
  echo "  cd rust && cargo test -p homa-rs --test kernel_integration -- --ignored --test-threads=1"
  exit 0
else
  echo "RESULT: build + unit tests only. Homa sockets are not usable on this machine."
  echo "  See docs/linux-setup.md for installing the kernel module on a supported Linux host."
  exit 1
fi
