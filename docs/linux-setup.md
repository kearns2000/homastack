# Linux setup for Homa

HomaStack needs a Linux host whose kernel has the out-of-tree `homa` module
loaded. Nothing in this repository installs or loads kernel modules; the steps
below are the upstream procedure (`INSTALL.md` in HomaModule), reproduced for
the pinned revision.

## Supported environments

| Environment | Works? | Notes |
|-------------|--------|-------|
| Bare-metal Linux, your own kernel | yes | upstream `main` is tested on 6.17.8; RHEL 8 / 9.5 have their own branches |
| Linux VM (KVM, VMware, Hyper-V, Lima, cloud VM) | yes | you must be able to build/load modules; loopback tests need one VM, real latency needs two + a switch. A Lima guest on Apple Silicon (Ubuntu 25.10, kernel `6.17.0-40-generic`) loaded the unmodified pin and `Homa.KernelSmoke` passed |
| WSL2 | not with the stock kernel | A custom kernel can load the module. On 6.18 the pinned HomaModule tree needs a local `recvmsg` change in `homa_plumbing.c` (its adapter is gated at 7.1). That change is not in this repository. Loopback tests passed once on `6.18.54.1-microsoft-standard-WSL2+`. This remains outside the supported setup. |
| Docker / Podman | only if the **host** kernel has Homa | containers cannot `insmod`; the socket probe may also be blocked by seccomp |
| Kubernetes / managed PaaS (App Service, Cloud Run, …) | no | no control over the kernel |
| macOS, Windows | no | build + unit tests only |

## 1. Prerequisites

Debian/Ubuntu:

```bash
sudo apt update
sudo apt install build-essential linux-headers-$(uname -r) linux-libc-dev git
```

Fedora/RHEL:

```bash
sudo dnf install gcc make kernel-devel-$(uname -r) kernel-headers git
```

Rust: <https://rustup.rs>. The pinned toolchain in `rust/rust-toolchain.toml`
is installed automatically on the first `cargo` command inside `rust/`.

Check the machine (read-only):

```bash
scripts/check-environment.sh
```

## 2. Build the kernel module (pinned revision)

```bash
git clone https://github.com/PlatformLab/HomaModule.git
cd HomaModule
git checkout 4bd0275714439da2955670a6c438f6c23304d785
make all
```

If your kernel is older/newer than what `main` supports, see the branch list in
upstream `INSTALL.md` (`rhel8`, `rhel9.5`, `linux_4.15.18`, …) or build a
matching kernel. Keep `homa.h` from the revision you actually run; if it
differs from the vendored copy in `rust/crates/homa-sys/upstream/homa.h`,
re-vendor it and run `cargo test -p homa-sys --features header-check`.

## 3. Load the module

```bash
sudo insmod homa.ko        # load
lsmod | grep '^homa'       # verify
ls /proc/sys/net/homa      # tunables appear
sudo rmmod homa            # unload (fails while any Homa socket is open)
```

Optional but recommended by upstream for performance (not required for the
functional tests): run `util/homa_prio` as root, enable jumbo frames, tune NIC
interrupt coalescing, configure switch priority queues. See upstream
`INSTALL.md`; none of that is automated here.

## 4. Verify from this repository

```bash
scripts/check-environment.sh          # should end with "Homa sockets are usable"
cd rust
cargo test -p homa-sys --features header-check
cargo build --release
./target/release/echo-server --port 5000 &
./target/release/echo-client --port 5000 --iterations 1000
cargo test -p homa-rs --test kernel_integration -- --ignored --test-threads=1
```

Or in one go, with a log written to `tests/integration/results/`:

```bash
scripts/run-integration.sh
```

## 5. Troubleshooting

| Symptom | Meaning / fix |
|---------|---------------|
| `socket failed: Protocol not supported (os error 93)` | module not loaded: `sudo insmod homa.ko` |
| `bind failed: Invalid argument … port number invalid` | server ports must be 1-32767 |
| `bind failed: Address already in use` | another Homa socket owns the port (TCP/UDP ports do not conflict) |
| `setsockopt(SO_HOMA_RCVBUF) failed: Invalid argument (homa: buffer pool is not large enough)` | increase `--buffer-mib`; minimum about 2 MiB + 64 KiB × `nr_cpu_ids` |
| `recvmsg failed: Cannot allocate memory` | receive pool exhausted - messages were not recycled or the pool is too small |
| `RPC … failed: Transport endpoint is not connected` | no Homa socket on the destination port |
| `RPC … failed: Connection timed out` | peer host unreachable or its module not loaded (kernel `timeout_ticks`) |
| client hangs forever | you used `--timeout-ms 0`; otherwise check the server log |
| `rmmod: Module homa is in use` | close all Homa sockets (kill echo-server) first |

Useful kernel-side diagnostics: `/proc/net/homa_metrics`, `sysctl net.homa`,
`dmesg | grep -i homa` (set `sysctl net.homa.verbose=1` for more).
