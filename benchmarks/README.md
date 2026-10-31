# benchmarks/

![Status: no results published](https://img.shields.io/badge/results-none%20published-lightgrey)
![Rust harness planned](https://img.shields.io/badge/rust-planned-000000?logo=rust)
![BenchmarkDotNet planned](https://img.shields.io/badge/BenchmarkDotNet-Phase%202%2B-512BD4?logo=dotnet&logoColor=white)

Reserved for latency/throughput benchmarks.

- `rust/` - will hold a Criterion-free, `std::time::Instant`-based harness
  comparing `homa-rs` round trips against the upstream `cp_node` numbers and
  against TCP on the same hosts. Phase 1 ships only the ad-hoc statistics in
  `rust/examples/echo-client` (`--iterations N`).
- `dotnet/` - Phase 2+ (BenchmarkDotNet).

Any numbers published here must state the kernel version, HomaModule revision,
NIC, MTU, whether `homa_prio` and the Homa qdisc were active, and whether the
measurement is user-space round trip or wire latency.
