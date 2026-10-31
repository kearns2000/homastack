//! Homa echo server.
//!
//! Binds a Homa server port, receives requests and returns each payload
//! unchanged as the response. Single-threaded: Homa's `recvmsg` returns one
//! complete message at a time and `sendmsg` returns as soon as the response is
//! queued, so one thread is sufficient for a proof of concept.
//!
//! ```text
//! echo-server --bind 0.0.0.0 --port 5000
//! ```
//!
//! Runs until SIGINT/SIGTERM, then drops the socket, which returns any
//! outstanding receive buffers and unmaps the region.

use std::net::IpAddr;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;

/// Homa echo server (Phase 1 proof of concept).
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Local IP address. Homa binds by port only; the address selects the
    /// family (IPv4 vs IPv6) and is otherwise informational.
    #[arg(long, default_value = "0.0.0.0")]
    bind: IpAddr,

    /// Homa server port (1..=32767). Port 0 is rejected: Homa treats it as
    /// "no bind", which never marks the socket as a server.
    #[arg(long, default_value_t = 5000, value_parser = clap::value_parser!(u16).range(1..=32767))]
    port: u16,

    /// Receive buffer region size in MiB (default 64). The kernel minimum is
    /// `nr_cpu_ids * 64 KiB + 2 MiB`, where `nr_cpu_ids` is the number of
    /// *possible* CPUs (often larger than `nproc`); a too-small region is
    /// rejected with EINVAL at bind time.
    #[arg(long, default_value_t = 64)]
    buffer_mib: usize,

    /// How often (ms) the loop wakes to check for shutdown.
    #[arg(long, default_value_t = 500)]
    poll_interval_ms: u64,
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

#[cfg(target_os = "linux")]
fn run(args: &Args, stop: &AtomicBool) -> Result<(), homa_rs::HomaError> {
    use std::time::Duration;

    use homa_rs::{AddressFamily, ErrorKind, HomaSocket};
    use tracing::{error, info, warn};

    let family = if args.bind.is_ipv4() {
        AddressFamily::V4
    } else {
        AddressFamily::V6
    };
    let socket = HomaSocket::config(family)
        .receive_region_bytes(args.buffer_mib << 20)
        .bind((args.bind, args.port))?;
    info!(
        bind = %args.bind,
        port = socket.local_port(),
        region_mib = socket.receive_region_len() >> 20,
        "echo server listening on Homa port"
    );

    let interval = Duration::from_millis(args.poll_interval_ms);
    let mut served: u64 = 0;
    let mut consecutive_errors: u32 = 0;

    while !stop.load(Ordering::Relaxed) {
        let msg = match socket.recv_timeout(interval) {
            Ok(Some(msg)) => msg,
            Ok(None) => continue,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) if !e.is_recoverable() => {
                error!(error = %e, "fatal socket error");
                return Err(e);
            }
            Err(e) => {
                consecutive_errors += 1;
                warn!(error = %e, kind = ?e.kind(), consecutive_errors, "recv failed; continuing");
                if consecutive_errors >= 100 {
                    error!("too many consecutive receive errors; giving up");
                    return Err(e);
                }
                // Avoid a hot spin if the kernel keeps failing immediately.
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
        };
        consecutive_errors = 0;

        if !msg.is_request() {
            // A non-private response addressed to this socket: nothing to do.
            warn!(id = %msg.id, peer = %msg.peer, "ignoring unexpected response message");
            continue;
        }

        info!(
            peer = %msg.peer,
            id = %msg.id,
            len = msg.len(),
            fragments = msg.fragments,
            "request received"
        );
        match socket.reply(&msg, &msg.payload) {
            Ok(()) => served += 1,
            Err(e) if !e.is_recoverable() => {
                error!(error = %e, "fatal error sending reply");
                return Err(e);
            }
            Err(e) => warn!(id = %msg.id, error = %e, "reply failed; continuing"),
        }
    }

    info!(
        served,
        pending_bpages = socket.pending_bpages(),
        "shutting down"
    );
    drop(socket); // explicit: returns bpages, closes fd, unmaps region
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn run(_args: &Args, _stop: &AtomicBool) -> Result<(), homa_rs::HomaError> {
    tracing::error!(
        "Homa sockets require Linux with the homa kernel module; this binary was built for {}",
        std::env::consts::OS
    );
    Err(homa_rs::HomaError::InvalidBufferConfig(
        "unsupported platform".into(),
    ))
}

fn main() -> ExitCode {
    init_tracing();
    let args = Args::parse();

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop);
        if let Err(e) = ctrlc::set_handler(move || stop.store(true, Ordering::Relaxed)) {
            tracing::warn!(error = %e, "could not install signal handler");
        }
    }

    match run(&args, &stop) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, kind = ?e.kind(), "echo server failed");
            ExitCode::FAILURE
        }
    }
}
