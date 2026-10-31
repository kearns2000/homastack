//! Homa echo client.
//!
//! Sends a payload to an echo server as a private Homa request, waits for the
//! response, verifies it byte-for-byte and reports round-trip latency.
//!
//! ```text
//! echo-client --server 127.0.0.1 --port 5000 --payload "Hello Homa" --iterations 1000
//! ```
//!
//! Latency reported here is **application-level round trip**: from just
//! before `sendmsg` returns control to the kernel until `recvmsg` returns the
//! copied response payload in user space, measured with the monotonic
//! `std::time::Instant`. It includes two system calls, the kernel's copy of
//! the payload into the receive region and our copy into a `Vec<u8>`. It is
//! *not* Homa's internal protocol latency or NIC wire latency; see
//! `/proc/net/homa_metrics` and the upstream `cp_node` tool for those.

use std::net::IpAddr;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;

/// Homa echo client (Phase 1 proof of concept).
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Echo server IP address (IPv4 or IPv6).
    #[arg(long, default_value = "127.0.0.1")]
    server: IpAddr,

    /// Echo server Homa port.
    #[arg(long, default_value_t = 5000)]
    port: u16,

    /// UTF-8 payload to send. Ignored if --payload-size is given.
    #[arg(long, default_value = "Hello Homa")]
    payload: String,

    /// Generate a deterministic binary payload of this many bytes instead of
    /// --payload (max 1_000_000).
    #[arg(long)]
    payload_size: Option<usize>,

    /// Number of sequential requests.
    #[arg(long, default_value_t = 1)]
    iterations: u32,

    /// Per-request timeout in milliseconds (0 = wait forever).
    #[arg(long, default_value_t = 5000)]
    timeout_ms: u64,

    /// Receive buffer region size in MiB.
    #[arg(long, default_value_t = 64)]
    buffer_mib: usize,
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

/// Summary statistics over a set of samples.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Stats {
    count: usize,
    min: Duration,
    max: Duration,
    mean: Duration,
    /// Present when at least 2 samples.
    p50: Option<Duration>,
    /// Present when at least 100 samples.
    p99: Option<Duration>,
}

/// Nearest-rank percentile (`pct` in whole percent) over sorted samples.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn percentile(sorted: &[Duration], pct: usize) -> Duration {
    debug_assert!(!sorted.is_empty());
    // rank = ceil(pct/100 * n), 1-based, clamped into range.
    let rank = (pct * sorted.len()).div_ceil(100);
    sorted[rank.clamp(1, sorted.len()) - 1]
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn stats(samples: &[Duration]) -> Option<Stats> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let total: Duration = sorted.iter().sum();
    Some(Stats {
        count: sorted.len(),
        min: sorted[0],
        max: sorted[sorted.len() - 1],
        mean: total / u32::try_from(sorted.len()).unwrap_or(u32::MAX),
        p50: (sorted.len() >= 2).then(|| percentile(&sorted, 50)),
        p99: (sorted.len() >= 100).then(|| percentile(&sorted, 99)),
    })
}

fn build_payload(args: &Args) -> Result<Vec<u8>, String> {
    if let Some(n) = args.payload_size {
        if n == 0 || n > homa_rs::MAX_MESSAGE_LENGTH {
            return Err(format!(
                "--payload-size must be in 1..={}",
                homa_rs::MAX_MESSAGE_LENGTH
            ));
        }
        // Deterministic pattern including zero bytes so corruption and
        // truncation are detectable.
        return Ok((0..n).map(|i| (i % 253) as u8).collect());
    }
    if args.payload.is_empty() {
        return Err("--payload must not be empty (Homa rejects empty messages)".into());
    }
    Ok(args.payload.clone().into_bytes())
}

#[cfg(target_os = "linux")]
fn run(args: &Args) -> Result<(), String> {
    use std::time::Instant;

    use homa_rs::{AddressFamily, HomaSocket};
    use tracing::{error, info};

    let payload = build_payload(args)?;
    let family = if args.server.is_ipv4() {
        AddressFamily::V4
    } else {
        AddressFamily::V6
    };
    let socket = HomaSocket::config(family)
        .receive_region_bytes(args.buffer_mib << 20)
        .open()
        .map_err(|e| format!("could not open Homa socket: {e} (kind {:?})", e.kind()))?;
    let dest = std::net::SocketAddr::new(args.server, args.port);
    let timeout = (args.timeout_ms > 0).then(|| Duration::from_millis(args.timeout_ms));

    info!(
        local_port = socket.local_port(),
        %dest,
        payload_len = payload.len(),
        iterations = args.iterations,
        ?timeout,
        "sending requests"
    );

    let mut samples = Vec::with_capacity(args.iterations as usize);
    let mut failures = 0u32;
    for i in 0..args.iterations {
        let start = Instant::now();
        let id = match socket.send_request(dest, &payload, u64::from(i) + 1, true) {
            Ok(id) => id,
            Err(e) => {
                failures += 1;
                error!(iteration = i, error = %e, kind = ?e.kind(), "send failed");
                continue;
            }
        };
        let response = match timeout {
            Some(t) => socket.recv_response_timeout(id, t),
            None => socket.recv_response(id),
        };
        let elapsed = start.elapsed();
        match response {
            Ok(msg) => {
                if msg.payload == payload {
                    samples.push(elapsed);
                    if args.iterations == 1 || i < 3 || tracing::enabled!(tracing::Level::DEBUG) {
                        info!(
                            iteration = i,
                            id = %msg.id,
                            cookie = msg.completion_cookie,
                            response_len = msg.len(),
                            fragments = msg.fragments,
                            rtt_us = elapsed.as_micros(),
                            "response verified"
                        );
                    }
                } else {
                    failures += 1;
                    error!(
                        iteration = i,
                        id = %msg.id,
                        expected_len = payload.len(),
                        actual_len = msg.len(),
                        "response payload MISMATCH"
                    );
                }
            }
            Err(e) => {
                failures += 1;
                error!(iteration = i, %id, error = %e, kind = ?e.kind(), "request failed");
            }
        }
    }

    if let Some(s) = stats(&samples) {
        println!(
            "application-level RTT over {} ok / {} failed requests of {} bytes:",
            s.count,
            failures,
            payload.len()
        );
        println!("  min  {:>10.1} µs", s.min.as_secs_f64() * 1e6);
        println!("  mean {:>10.1} µs", s.mean.as_secs_f64() * 1e6);
        if let Some(p50) = s.p50 {
            println!("  p50  {:>10.1} µs", p50.as_secs_f64() * 1e6);
        }
        match s.p99 {
            Some(p99) => println!("  p99  {:>10.1} µs", p99.as_secs_f64() * 1e6),
            None => println!("  p99  (needs >= 100 samples)"),
        }
        println!("  max  {:>10.1} µs", s.max.as_secs_f64() * 1e6);
        println!(
            "(user-space round trip incl. sendmsg/recvmsg and payload copies; not wire latency)"
        );
    }

    if failures > 0 {
        return Err(format!("{failures} of {} requests failed", args.iterations));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn run(args: &Args) -> Result<(), String> {
    let _ = build_payload(args)?;
    Err(format!(
        "Homa sockets require Linux with the homa kernel module; this binary was built for {}",
        std::env::consts::OS
    ))
}

fn main() -> ExitCode {
    init_tracing();
    let args = Args::parse();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us(n: u64) -> Duration {
        Duration::from_micros(n)
    }

    #[test]
    fn stats_of_empty_is_none() {
        assert!(stats(&[]).is_none());
    }

    #[test]
    fn stats_single_sample_has_no_percentiles() {
        let s = stats(&[us(42)]).unwrap();
        assert_eq!((s.count, s.min, s.max, s.mean), (1, us(42), us(42), us(42)));
        assert_eq!(s.p50, None);
        assert_eq!(s.p99, None);
    }

    #[test]
    fn stats_percentiles() {
        let samples: Vec<Duration> = (1..=100).map(us).collect();
        let s = stats(&samples).unwrap();
        assert_eq!(s.min, us(1));
        assert_eq!(s.max, us(100));
        assert_eq!(s.mean, Duration::from_nanos(50_500)); // 5050 µs / 100
        assert_eq!(s.p50, Some(us(50)));
        assert_eq!(s.p99, Some(us(99)));
    }

    #[test]
    fn payload_generation_rules() {
        let mut args = Args::parse_from(["echo-client"]);
        assert_eq!(build_payload(&args).unwrap(), b"Hello Homa");
        args.payload_size = Some(300_000);
        let p = build_payload(&args).unwrap();
        assert_eq!(p.len(), 300_000);
        assert!(p.contains(&0));
        args.payload_size = Some(0);
        assert!(build_payload(&args).is_err());
        args.payload_size = Some(homa_rs::MAX_MESSAGE_LENGTH + 1);
        assert!(build_payload(&args).is_err());
        args.payload_size = None;
        args.payload = String::new();
        assert!(build_payload(&args).is_err());
    }
}
