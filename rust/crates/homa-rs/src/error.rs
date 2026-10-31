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
}

/// Map a Linux `errno` to an [`ErrorKind`].
#[must_use]
pub fn classify_errno(errno: Option<i32>) -> ErrorKind {
    let Some(e) = errno else {
        return ErrorKind::Other;
    };
    match e {
        // EWOULDBLOCK == EAGAIN on Linux and every other supported Unix.
        libc::EAGAIN => ErrorKind::WouldBlock,
        libc::ETIMEDOUT => ErrorKind::Timeout,
        libc::ESHUTDOWN => ErrorKind::Shutdown,
        libc::ENOTCONN => ErrorKind::NoServer,
        libc::EHOSTUNREACH | libc::ENETUNREACH => ErrorKind::Unreachable,
        libc::EADDRINUSE => ErrorKind::AddressInUse,
        libc::EACCES | libc::EPERM => ErrorKind::PermissionDenied,
        libc::ENOMEM | libc::ENOBUFS => ErrorKind::NoMemory,
        libc::EINTR => ErrorKind::Interrupted,
        libc::EPROTONOSUPPORT | libc::ESOCKTNOSUPPORT | libc::ENOPROTOOPT => ErrorKind::Unavailable,
        libc::EINVAL | libc::EFAULT | libc::EAFNOSUPPORT | libc::EBADF => ErrorKind::InvalidInput,
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
        assert_eq!(classify_errno(Some(libc::EAGAIN)), ErrorKind::WouldBlock);
        assert_eq!(classify_errno(Some(libc::ETIMEDOUT)), ErrorKind::Timeout);
        assert_eq!(classify_errno(Some(libc::ESHUTDOWN)), ErrorKind::Shutdown);
        assert_eq!(classify_errno(Some(libc::ENOTCONN)), ErrorKind::NoServer);
        assert_eq!(
            classify_errno(Some(libc::EHOSTUNREACH)),
            ErrorKind::Unreachable
        );
        assert_eq!(
            classify_errno(Some(libc::EADDRINUSE)),
            ErrorKind::AddressInUse
        );
        assert_eq!(classify_errno(Some(libc::ENOMEM)), ErrorKind::NoMemory);
        assert_eq!(classify_errno(Some(libc::EINVAL)), ErrorKind::InvalidInput);
        assert_eq!(
            classify_errno(Some(libc::EPROTONOSUPPORT)),
            ErrorKind::Unavailable
        );
        assert_eq!(classify_errno(Some(libc::EIO)), ErrorKind::Other);
        assert_eq!(classify_errno(None), ErrorKind::Other);
    }

    #[test]
    fn socket_creation_errors_mean_unavailable() {
        assert_eq!(
            os("socket", libc::EPROTONOSUPPORT).kind(),
            ErrorKind::Unavailable
        );
        assert_eq!(
            os("socket", libc::EAFNOSUPPORT).kind(),
            ErrorKind::Unavailable
        );
        // ...but the same errno elsewhere is just bad input.
        assert_eq!(
            os("sendmsg", libc::EAFNOSUPPORT).kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn rpc_failed_carries_id_and_kind() {
        let err = HomaError::RpcFailed {
            id: RpcId::from_raw(42),
            completion_cookie: 7,
            source: io::Error::from_raw_os_error(libc::ETIMEDOUT),
            detail: Some("peer dead".into()),
        };
        assert_eq!(err.kind(), ErrorKind::Timeout);
        assert_eq!(err.raw_os_error(), Some(libc::ETIMEDOUT));
        let text = err.to_string();
        assert!(text.contains("RPC 42"), "{text}");
        assert!(text.contains("peer dead"), "{text}");
    }

    #[test]
    fn detail_is_attached_once_and_empty_is_dropped() {
        let err = os("recvmsg", libc::EINVAL).with_detail(Some(String::new()));
        assert!(!err.to_string().contains("homa:"));
        let err = err.with_detail(Some("invalid msg_controllen in recvmsg".into()));
        assert!(err.to_string().contains("homa: invalid msg_controllen"));
        let err = err.with_detail(Some("second".into()));
        assert!(!err.to_string().contains("second"));
    }

    #[test]
    fn recoverability() {
        assert!(os("recvmsg", libc::ENOMEM).is_recoverable());
        assert!(!os("recvmsg", libc::ESHUTDOWN).is_recoverable());
        assert!(!os("socket", libc::EPROTONOSUPPORT).is_recoverable());
        assert!(HomaError::Timeout(std::time::Duration::from_secs(1)).is_recoverable());
    }

    #[test]
    fn converts_to_io_error() {
        let io: io::Error = os("bind", libc::EADDRINUSE).into();
        assert_eq!(io.raw_os_error(), Some(libc::EADDRINUSE));
        let io: io::Error = HomaError::EmptyMessage.into();
        assert_eq!(io.kind(), io::ErrorKind::InvalidInput);
    }
}
