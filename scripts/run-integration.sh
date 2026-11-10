#!/usr/bin/env bash
# run-integration.sh - run the kernel integration tests against the real Homa
# module and record the outcome in tests/integration/results/.
#
# Requirements: Linux, homa module loaded, ports 7400-7599 and 7601-7602 free,
# Rust toolchain, and the .NET 10 SDK (for the managed smoke).
# Usage: scripts/run-integration.sh [extra cargo test args]

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! scripts/check-environment.sh --quiet; then
  echo "environment check failed; not running integration tests" >&2
  exit 1
fi

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUT_DIR="tests/integration/results"
mkdir -p "$OUT_DIR"
LOG="$OUT_DIR/$STAMP-$(hostname -s).log"

{
  echo "# HomaStack kernel integration run"
  echo "date_utc: $STAMP"
  echo "host: $(hostname)"
  echo "kernel: $(uname -r) ($(uname -m))"
  echo "homa_module: $(modinfo -F version homa 2>/dev/null || echo 'loaded (version unknown)')"
  echo "rustc: $(rustc --version)"
  echo "dotnet: $(dotnet --version 2>/dev/null || echo missing)"
  echo "git: $(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
  echo
} | tee "$LOG"

cd rust
set +e
cargo test -p homa-sys --features header-check 2>&1 | tee -a "../$LOG"
HEADER_RC=${PIPESTATUS[0]}
cargo test -p homa-ffi --features header-check 2>&1 | tee -a "../$LOG"
FFI_HEADER_RC=${PIPESTATUS[0]}
cargo test -p homa-rs --test kernel_integration -- --ignored --test-threads=1 --nocapture "$@" 2>&1 | tee -a "../$LOG"
RC=${PIPESTATUS[0]}
cargo test -p homa-ffi --test ffi_kernel -- --ignored --test-threads=1 --nocapture "$@" 2>&1 | tee -a "../$LOG"
FFI_RC=${PIPESTATUS[0]}
set -e

cd "$ROOT"
set +e
dotnet run --project dotnet/tests/Homa.KernelSmoke/Homa.KernelSmoke.csproj -c Release --no-launch-profile 2>&1 | tee -a "$LOG"
DOTNET_RC=${PIPESTATUS[0]}
set -e

{
  echo
  echo "header_check_exit: $HEADER_RC"
  echo "ffi_header_check_exit: $FFI_HEADER_RC"
  echo "integration_exit: $RC"
  echo "ffi_kernel_exit: $FFI_RC"
  echo "dotnet_smoke_exit: $DOTNET_RC"
} | tee -a "$LOG"

echo "results written to $LOG"
exit $(( RC != 0 || HEADER_RC != 0 || FFI_HEADER_RC != 0 || FFI_RC != 0 || DOTNET_RC != 0 ))
