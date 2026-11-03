//! Error type and errno classification for Homa operations.
//!
//! Every failing system call is reported as [`HomaError::Os`] (or
//! [`HomaError::RpcFailed`] when the kernel attributed the failure to a
//! specific RPC). [`HomaError::kind`] folds the raw `errno` into a small
//! [`ErrorKind`] enum that callers can match on without caring about Linux
//! details; the original [`std::io::Error`] is always retained.

use std::io;

use crate::transport::RpcId;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, HomaError>;

/// Linux `asm-generic` errno numbers.
///
/// [`classify_errno`] uses these on every host. macOS `libc` uses different
/// values, and the Windows `libc` crate does not define the socket ones.
/// Only the Linux socket path observes a real errno.
mod linux_errno {
    pub const EPERM: i32 = 1;
    pub const EINTR: i32 = 4;
    // Not its own ErrorKind. Named so tests can assert it stays Other,
    // including on hosts whose libc crate has no EIO.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const EIO: i32 = 5;
    pub const EBADF: i32 = 9;
    pub const EAGAIN: i32 = 11;
    pub const ENOMEM: i32 = 12;
    pub const EACCES: i32 = 13;
    pub const EFAULT: i32 = 14;
    pub const EINVAL: i32 = 22;
    pub const ENOPROTOOPT: i32 = 92;
    pub const EPROTONOSUPPORT: i32 = 93;
    pub const ESOCKTNOSUPPORT: i32 = 94;
    pub const EAFNOSUPPORT: i32 = 97;
    pub const EADDRINUSE: i32 = 98;
    pub const ENETUNREACH: i32 = 101;
    pub const ENOBUFS: i32 = 105;
    pub const ENOTCONN: i32 = 107;
    pub const ESHUTDOWN: i32 = 108;
    pub const ETIMEDOUT: i32 = 110;
    pub const EHOSTUNREACH: i32 = 113;
}

/// Coarse classification of a [`HomaError`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The Homa kernel module is not loaded or the platform has no Homa
    /// (`EPROTONOSUPPORT`, `EAFNOSUPPORT` at socket creation, or non-Linux).
    Unavailable,
    /// Operation would block (`EAGAIN`/`EWOULDBLOCK`).
    WouldBlock,
    /// A timeout elapsed - either the kernel gave up on the peer
    /// (`ETIMEDOUT`) or a caller-supplied deadline passed.
    Timeout,
    /// The socket was shut down (`ESHUTDOWN`).
    Shutdown,
    /// No Homa socket is listening on the destination port (`ENOTCONN`).
    NoServer,
    /// The destination host could not be reached (`EHOSTUNREACH`,
    /// `ENETUNREACH`).
    Unreachable,
    /// The requested port is already in use (`EADDRINUSE`).
    AddressInUse,
    /// Insufficient permission (`EACCES`, `EPERM`).
    PermissionDenied,
    /// Out of memory, including an exhausted receive buffer pool (`ENOMEM`).
    NoMemory,
    /// A system call was interrupted by a signal (`EINTR`).
    Interrupted,
    /// Invalid arguments as judged by the kernel (`EINVAL`, `EFAULT`,
    /// `EAFNOSUPPORT`) or by this crate before issuing a system call.
    InvalidInput,
    /// Anything else.
    Other,
}

/// Errors produced by this crate.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HomaError {
    /// A system call failed.
    #[error("{op} failed: {source}{}", fmt_detail(.detail))]
    Os {
        /// Name of the operation, e.g. `"recvmsg"`.
        op: &'static str,
        /// The underlying OS error (carries `errno`).
        #[source]
        source: io::Error,
        /// Homa's own explanation from `HOMAIOCINFO.error_msg`, if available.
        detail: Option<String>,
    },

    /// `recvmsg` reported that a specific RPC failed (for example the peer
    /// timed out or there was no server on the port).
    #[error("RPC {id} failed: {source}{}", fmt_detail(.detail))]
    RpcFailed {
        /// The failed RPC.
        id: RpcId,
        /// Completion cookie supplied when the request was sent (responses
        /// only; `0` for requests).
        completion_cookie: u64,
        /// The underlying OS error.
        #[source]
        source: io::Error,
        /// Homa's own explanation, if available.
        detail: Option<String>,
    },

    /// Payload longer than `HOMA_MAX_MESSAGE_LENGTH`.
    #[error("message of {len} bytes exceeds Homa's maximum of {max} bytes")]
    MessageTooLarge {
        /// Offered length.
        len: usize,
        /// Maximum accepted by Homa.
        max: usize,
    },

    /// Homa rejects zero-length messages.
    #[error("Homa messages must contain at least one byte")]
    EmptyMessage,

    /// An address could not be resolved or converted.
    #[error("invalid address: {0}")]
    InvalidAddress(String),

    /// Address family of a destination does not match the socket's.
    #[error("address family mismatch: socket is {socket:?}, address is {address:?}")]
    AddressFamilyMismatch {
        /// Family the socket was created with.
        socket: crate::addr::AddressFamily,
        /// Family of the offered address.
        address: crate::addr::AddressFamily,
    },

    /// A bind port outside `1..HOMA_MIN_DEFAULT_PORT` was requested.
    #[error("port {0} is not bindable: Homa server ports must be in 1..=32767")]
    InvalidPort(u16),

    /// Receive-buffer configuration was rejected before reaching the kernel.
    #[error("invalid receive buffer configuration: {0}")]
    InvalidBufferConfig(String),

    /// The kernel returned fragment descriptors that do not describe a valid
    /// message inside the registered region. Treated as fatal for that
    /// message; nothing is read from the region.
    #[error("kernel returned inconsistent receive buffer descriptors: {0}")]
    MalformedReceive(String),

    /// A caller-supplied deadline elapsed.
    #[error("operation timed out after {0:?}")]
    Timeout(std::time::Duration),

    /// The id is not one the kernel could have issued for this operation
    /// (e.g. `0`, which Homa never assigns and which `recvmsg` interprets as
    /// "any shared message").
    #[error("invalid RPC id {0}")]
    InvalidRpcId(RpcId),

    /// The message received was not of the kind the caller asked for.
    #[error("expected a {expected:?} but received RPC {id} ({actual:?})")]
    UnexpectedMessage {
        /// Kind the caller asked for.
        expected: crate::transport::MessageKind,
        /// Kind actually received.
        actual: crate::transport::MessageKind,
        /// Id of the received message.
        id: RpcId,
    },
}

#[allow(clippy::ref_option)] // signature dictated by thiserror's field access
fn fmt_detail(detail: &Option<String>) -> String {
    match detail {
        Some(d) if !d.is_empty() => format!(" (homa: {d})"),
        _ => String::new(),
    }
}

impl HomaError {
    /// Build an [`HomaError::Os`] from the current `errno`-derived error.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn os(op: &'static str, source: io::Error) -> Self {
        Self::Os {
            op,
            source,
            detail: None,
        }
    }

    /// Attach Homa's `error_msg` explanation (if any) to an OS/RPC error.
    #[must_use]
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn with_detail(mut self, new_detail: Option<String>) -> Self {
        match &mut self {
            Self::Os { detail, .. } | Self::RpcFailed { detail, .. } => {
                if detail.is_none() {
                    *detail = new_detail.filter(|d| !d.is_empty());
                }
            }
            _ => {}
        }
        self
    }

    /// The raw `errno`, if this error wraps a system call failure.
    #[must_use]
    pub fn raw_os_error(&self) -> Option<i32> {
        match self {
            Self::Os { source, .. } | Self::RpcFailed { source, .. } => source.raw_os_error(),
            _ => None,
        }
    }

    /// Coarse classification; see [`ErrorKind`].
    #[must_use]
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Os { op, source, .. } => {
                let kind = classify_errno(source.raw_os_error());
                // EAFNOSUPPORT / EPROTONOSUPPORT at socket creation ⇒ no Homa.
                if *op == "socket" && kind == ErrorKind::InvalidInput {
                    ErrorKind::Unavailable
                } else {
                    kind
                }
            }
            Self::RpcFailed { source, .. } => classify_errno(source.raw_os_error()),
            Self::Timeout(_) => ErrorKind::Timeout,
            Self::MessageTooLarge { .. }
            | Self::EmptyMessage
            | Self::InvalidAddress(_)
            | Self::AddressFamilyMismatch { .. }
            | Self::InvalidPort(_)
            | Self::InvalidBufferConfig(_)
            | Self::InvalidRpcId(_)
            | Self::UnexpectedMessage { .. } => ErrorKind::InvalidInput,
            Self::MalformedReceive(_) => ErrorKind::Other,
        }
    }

    /// `true` for conditions a long-running server should log and continue
    /// from rather than exit on.
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        !matches!(self.kind(), ErrorKind::Shutdown | ErrorKind::Unavailable)
    }

    /// Whether a failed private `recvmsg` should be followed by `HOMAIOCABORT`.
    ///
    /// Abort only when this thread may have been the waiter and the kernel
    /// left the RPC alive. Do **not** abort when:
    /// * the kernel already ended the RPC ([`HomaError::RpcFailed`], shutdown);
    /// * the wait was rejected before this thread became the waiter
    ///   (`EINVAL` / [`ErrorKind::InvalidInput`]: a second waiter, or an id
    ///   that is not a private client RPC). Aborting there would cancel the
    ///   other thread's RPC or a shared response;
    /// * the result is `EAGAIN` or `EINTR`, which leave the RPC alive and are
    ///   retried by the socket layer.
    #[must_use]
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn private_wait_should_abort(&self) -> bool {
        !matches!(self, Self::RpcFailed { .. })
            && !matches!(
                self.kind(),
                ErrorKind::Shutdown
                    | ErrorKind::InvalidInput
                    | ErrorKind::WouldBlock
                    | ErrorKind::Interrupted
            )
    }
}

/// Map a Linux `errno` to an [`ErrorKind`].
#[must_use]
pub fn classify_errno(errno: Option<i32>) -> ErrorKind {
    let Some(e) = errno else {
        return ErrorKind::Other;
    };
    match e {
        // EWOULDBLOCK == EAGAIN on Linux.
        linux_errno::EAGAIN => ErrorKind::WouldBlock,
        linux_errno::ETIMEDOUT => ErrorKind::Timeout,
        linux_errno::ESHUTDOWN => ErrorKind::Shutdown,
        linux_errno::ENOTCONN => ErrorKind::NoServer,
        linux_errno::EHOSTUNREACH | linux_errno::ENETUNREACH => ErrorKind::Unreachable,
        linux_errno::EADDRINUSE => ErrorKind::AddressInUse,
        linux_errno::EACCES | linux_errno::EPERM => ErrorKind::PermissionDenied,
        linux_errno::ENOMEM | linux_errno::ENOBUFS => ErrorKind::NoMemory,
        linux_errno::EINTR => ErrorKind::Interrupted,
        linux_errno::EPROTONOSUPPORT | linux_errno::ESOCKTNOSUPPORT | linux_errno::ENOPROTOOPT => {
            ErrorKind::Unavailable
        }
        linux_errno::EINVAL
        | linux_errno::EFAULT
        | linux_errno::EAFNOSUPPORT
        | linux_errno::EBADF => ErrorKind::InvalidInput,
        _ => ErrorKind::Other,
    }
}

impl From<HomaError> for io::Error {
    fn from(err: HomaError) -> Self {
        match err {
            HomaError::Os { source, .. } | HomaError::RpcFailed { source, .. } => source,
            HomaError::Timeout(_) => io::Error::new(io::ErrorKind::TimedOut, err.to_string()),
            other => io::Error::new(io::ErrorKind::InvalidInput, other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(op: &'static str, errno: i32) -> HomaError {
        HomaError::os(op, io::Error::from_raw_os_error(errno))
    }

    #[test]
    fn errno_classification() {
        assert_eq!(
            classify_errno(Some(linux_errno::EAGAIN)),
            ErrorKind::WouldBlock
        );
        assert_eq!(
            classify_errno(Some(linux_errno::ETIMEDOUT)),
            ErrorKind::Timeout
        );
        assert_eq!(
            classify_errno(Some(linux_errno::ESHUTDOWN)),
            ErrorKind::Shutdown
        );
        assert_eq!(
            classify_errno(Some(linux_errno::ENOTCONN)),
            ErrorKind::NoServer
        );
        assert_eq!(
            classify_errno(Some(linux_errno::EHOSTUNREACH)),
            ErrorKind::Unreachable
        );
        assert_eq!(
            classify_errno(Some(linux_errno::EADDRINUSE)),
            ErrorKind::AddressInUse
        );
        assert_eq!(
            classify_errno(Some(linux_errno::ENOMEM)),
            ErrorKind::NoMemory
        );
        assert_eq!(
            classify_errno(Some(linux_errno::EINVAL)),
            ErrorKind::InvalidInput
        );
        assert_eq!(
            classify_errno(Some(linux_errno::EPROTONOSUPPORT)),
            ErrorKind::Unavailable
        );
        assert_eq!(classify_errno(Some(linux_errno::EIO)), ErrorKind::Other);
        assert_eq!(classify_errno(None), ErrorKind::Other);
    }

    #[test]
    fn socket_creation_errors_mean_unavailable() {
        assert_eq!(
            os("socket", linux_errno::EPROTONOSUPPORT).kind(),
            ErrorKind::Unavailable
        );
        assert_eq!(
            os("socket", linux_errno::EAFNOSUPPORT).kind(),
            ErrorKind::Unavailable
        );
        // ...but the same errno elsewhere is just bad input.
        assert_eq!(
            os("sendmsg", linux_errno::EAFNOSUPPORT).kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn rpc_failed_carries_id_and_kind() {
        let err = HomaError::RpcFailed {
            id: RpcId::from_raw(42),
            completion_cookie: 7,
            source: io::Error::from_raw_os_error(linux_errno::ETIMEDOUT),
            detail: Some("peer dead".into()),
        };
        assert_eq!(err.kind(), ErrorKind::Timeout);
        assert_eq!(err.raw_os_error(), Some(linux_errno::ETIMEDOUT));
        let text = err.to_string();
        assert!(text.contains("RPC 42"), "{text}");
        assert!(text.contains("peer dead"), "{text}");
    }

    #[test]
    fn detail_is_attached_once_and_empty_is_dropped() {
        let err = os("recvmsg", linux_errno::EINVAL).with_detail(Some(String::new()));
        assert!(!err.to_string().contains("homa:"));
        let err = err.with_detail(Some("invalid msg_controllen in recvmsg".into()));
        assert!(err.to_string().contains("homa: invalid msg_controllen"));
        let err = err.with_detail(Some("second".into()));
        assert!(!err.to_string().contains("second"));
    }

    #[test]
    fn private_wait_abort_policy() {
        let os = |errno| HomaError::os("recvmsg", io::Error::from_raw_os_error(errno));
        assert!(!os(linux_errno::EINVAL).private_wait_should_abort());
        assert!(!os(linux_errno::EAGAIN).private_wait_should_abort());
        assert!(!os(linux_errno::EINTR).private_wait_should_abort());
        assert!(!os(linux_errno::ESHUTDOWN).private_wait_should_abort());
        assert!(os(linux_errno::EIO).private_wait_should_abort());
        assert!(
            !HomaError::RpcFailed {
                id: RpcId::from_raw(2),
                completion_cookie: 0,
                source: io::Error::from_raw_os_error(linux_errno::ETIMEDOUT),
                detail: None,
            }
            .private_wait_should_abort()
        );
        assert!(!HomaError::InvalidRpcId(RpcId::from_raw(1)).private_wait_should_abort());
    }

    #[test]
    fn recoverability() {
        assert!(os("recvmsg", linux_errno::ENOMEM).is_recoverable());
        assert!(!os("recvmsg", linux_errno::ESHUTDOWN).is_recoverable());
        assert!(!os("socket", linux_errno::EPROTONOSUPPORT).is_recoverable());
        assert!(HomaError::Timeout(std::time::Duration::from_secs(1)).is_recoverable());
    }

    #[test]
    fn converts_to_io_error() {
        let io: io::Error = os("bind", linux_errno::EADDRINUSE).into();
        assert_eq!(io.raw_os_error(), Some(linux_errno::EADDRINUSE));
        let io: io::Error = HomaError::EmptyMessage.into();
        assert_eq!(io.kind(), io::ErrorKind::InvalidInput);
    }

    /// The numbers above are the Linux ones the kernel actually returns.
    #[cfg(target_os = "linux")]
    #[test]
    fn linux_errno_matches_libc() {
        assert_eq!(linux_errno::EPERM, libc::EPERM);
        assert_eq!(linux_errno::EINTR, libc::EINTR);
        assert_eq!(linux_errno::EIO, libc::EIO);
        assert_eq!(linux_errno::EBADF, libc::EBADF);
        assert_eq!(linux_errno::EAGAIN, libc::EAGAIN);
        assert_eq!(linux_errno::ENOMEM, libc::ENOMEM);
        assert_eq!(linux_errno::EACCES, libc::EACCES);
        assert_eq!(linux_errno::EFAULT, libc::EFAULT);
        assert_eq!(linux_errno::EINVAL, libc::EINVAL);
        assert_eq!(linux_errno::ENOPROTOOPT, libc::ENOPROTOOPT);
        assert_eq!(linux_errno::EPROTONOSUPPORT, libc::EPROTONOSUPPORT);
        assert_eq!(linux_errno::ESOCKTNOSUPPORT, libc::ESOCKTNOSUPPORT);
        assert_eq!(linux_errno::EAFNOSUPPORT, libc::EAFNOSUPPORT);
        assert_eq!(linux_errno::EADDRINUSE, libc::EADDRINUSE);
        assert_eq!(linux_errno::ENETUNREACH, libc::ENETUNREACH);
        assert_eq!(linux_errno::ENOBUFS, libc::ENOBUFS);
        assert_eq!(linux_errno::ENOTCONN, libc::ENOTCONN);
        assert_eq!(linux_errno::ESHUTDOWN, libc::ESHUTDOWN);
        assert_eq!(linux_errno::ETIMEDOUT, libc::ETIMEDOUT);
        assert_eq!(linux_errno::EHOSTUNREACH, libc::EHOSTUNREACH);
    }
}
