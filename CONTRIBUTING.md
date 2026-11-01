# Contributing to HomaStack

Thanks for your interest in contributing. HomaStack is a Rust and .NET stack over the Linux Homa kernel module. Fixes, tests, and documentation that stay faithful to that module are welcome.

## Before you start

- Search [existing issues](https://github.com/kearns2000/homastack/issues) to avoid duplicate work.
- For large changes (new ABI versions, public API changes, architecture), open an issue first to discuss approach.
- Keep pull requests focused. One feature or fix per PR is easier to review.

## Prerequisites

- [Rust](https://rustup.rs/) as pinned in `rust/rust-toolchain.toml` (1.85 or newer, edition 2024)
- [.NET 10 SDK](https://dotnet.microsoft.com/download/dotnet/10.0)
- Any editor (VS, VS Code, Rider)
- Linux with `homa.ko` only when running the ignored kernel suites. Ordinary tests must pass without it.

## Getting started

```bash
git clone https://github.com/kearns2000/homastack.git
cd homastack

cd rust
cargo test --workspace

cd ../dotnet
dotnet test HomaStack.slnx -c Release
```

## Project layout

```text
rust/crates/homa-sys/     # Bindings to the vendored homa.h socket interface
rust/crates/homa-rs/      # Safe synchronous Rust API
rust/crates/homa-ffi/     # Stable C ABI, libhoma_ffi
rust/examples/            # Echo server, echo client, C FFI client
dotnet/src/Homa.NET/      # .NET 10 wrapper over libhoma_ffi
dotnet/src/Homa.Rpc/      # RPC frame, pipeline, client, server
dotnet/src/Homa.Rpc.Hosting/
dotnet/tests/
docs/                     # Architecture, native contract, Linux setup
tests/integration/        # How to run the ignored kernel suites
```

## Making changes

### Bug fixes

1. Add a failing test that reproduces the bug. Rust unit tests live next to the crate. .NET tests live under `dotnet/tests/`.
2. Fix the issue.
3. Ensure the commands in [Testing expectations](#testing-expectations) pass.

### Public API changes

- Keep the public surface small.
- Update `README.md`, and `docs/architecture.md` or `docs/native-interop.md` when the user-visible contract changes.
- The C ABI is version 1. Do not add, remove, or reorder exported symbols or change struct layout without an issue first.
- Avoid breaking changes in patch or minor releases without discussion.

### Kernel behaviour

- Kernel constants come from the vendored `homa.h` at the pinned HomaModule revision. Do not invent ioctl numbers, struct sizes, or errno mappings.
- Kernel tests stay `#[ignore]`. `cargo test` and `dotnet test` must pass on a machine with no `homa.ko`.
- Do not open a Homa socket from a .NET unit test.
- Do not write that a kernel RPC was executed unless `tests/integration/results/` has the log.

## Code guidelines

- Match existing naming and file structure.
- Do not use em dashes in comments, docs, or commit messages.
- Rust: `cargo fmt` and clippy with warnings denied, matching CI.
- C#: nullable reference types, implicit usings, warnings as errors.
- The vendored header under `rust/crates/homa-sys/upstream/` stays under its BSD-2-Clause notice. HomaStack code is MIT. See [LICENSE](LICENSE).

## Testing expectations

All PRs should pass:

```bash
cd rust
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

cd ../dotnet
dotnet test HomaStack.slnx -c Release
```

On Linux, also run the header checks:

```bash
cd rust
cargo test -p homa-sys --features header-check
cargo test -p homa-ffi --features header-check
```

Add tests when you:

- Fix a bug
- Change framing, dispatch, middleware, or cancellation
- Change the C ABI, struct layout, or error mapping
- Touch buffer-page release or socket lifetime

The ignored kernel suites (`scripts/run-integration.sh`) need a host with `homa.ko`. They are not required for a change that does not touch the kernel path, and they are not run in ordinary CI.

## Pull request checklist

- [ ] `cargo fmt --all --check` succeeds
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` succeeds
- [ ] `cargo test --workspace` passes
- [ ] `dotnet test HomaStack.slnx -c Release` passes
- [ ] Tests added or updated for the change
- [ ] README or docs updated if public behaviour changed
- [ ] No unrelated formatting or drive-by refactors

## Code of conduct

This project follows the [Code of Conduct](CODE_OF_CONDUCT.md). By participating, you agree to uphold it.

## Licence

By contributing, you agree that your contributions will be licensed under the [MIT Licence](LICENSE).

## Questions

Open a [GitHub issue](https://github.com/kearns2000/homastack/issues) for questions or ideas. For security vulnerabilities, see [SECURITY.md](SECURITY.md).
