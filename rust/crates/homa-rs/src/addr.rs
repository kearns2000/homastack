//! Conversions between [`std::net::SocketAddr`] and the C `sockaddr` family
//! used by the kernel.
//!
//! Homa accepts `sockaddr_in` for `AF_INET` sockets and `sockaddr_in6` for
//! `AF_INET6` sockets; the family must match the socket's. [`RawSocketAddr`]
//! mirrors the host libc layout, so it is compiled only on Unix.
//! [`AddressFamily`] and [`resolve`] are available on every host.

#[cfg(unix)]
use core::mem::{size_of, zeroed};
#[cfg(unix)]
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::net::{SocketAddr, ToSocketAddrs};

use crate::error::{HomaError, Result};

/// IP address family of a Homa socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddressFamily {
    /// `AF_INET`.
    V4,
    /// `AF_INET6`.
    V6,
}

impl AddressFamily {
    /// The libc `AF_*` constant.
    #[cfg(unix)]
    #[must_use]
    pub const fn to_raw(self) -> libc::c_int {
        match self {
            Self::V4 => libc::AF_INET,
            Self::V6 => libc::AF_INET6,
        }
    }

    /// Family of a [`SocketAddr`].
    #[must_use]
    pub const fn of(addr: &SocketAddr) -> Self {
        match addr {
            SocketAddr::V4(_) => Self::V4,
            SocketAddr::V6(_) => Self::V6,
        }
    }
}

/// A `sockaddr_storage` plus the length of the address actually stored.
///
/// Built from a [`SocketAddr`] for `bind`/`sendmsg`, or filled by `recvmsg`.
#[cfg(unix)]
#[derive(Clone, Copy)]
pub struct RawSocketAddr {
    storage: libc::sockaddr_storage,
    len: libc::socklen_t,
}

#[cfg(unix)]
impl core::fmt::Debug for RawSocketAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.to_socket_addr() {
            Ok(addr) => write!(f, "RawSocketAddr({addr})"),
            Err(_) => write!(f, "RawSocketAddr(<family {}>)", self.family_raw()),
        }
    }
}

#[cfg(unix)]
impl RawSocketAddr {
    /// Zeroed storage ready to be filled by the kernel.
    #[must_use]
    pub fn empty() -> Self {
        // SAFETY: sockaddr_storage is plain-old-data; all-zero is a valid
        // (AF_UNSPEC) value.
        let storage: libc::sockaddr_storage = unsafe { zeroed() };
        Self { storage, len: 0 }
    }

    /// Encode a [`SocketAddr`].
    #[must_use]
    pub fn from_socket_addr(addr: &SocketAddr) -> Self {
        // SAFETY: as in `empty`.
        let mut storage: libc::sockaddr_storage = unsafe { zeroed() };
        let len = match addr {
            SocketAddr::V4(v4) => {
                // SAFETY: sockaddr_in is POD; zeroed then every field we care
                // about is assigned below. Any platform-specific extra fields
                // (e.g. `sin_len`, `sin_zero`) stay zero, which is valid.
                let mut sin: libc::sockaddr_in = unsafe { zeroed() };
                sin.sin_family = libc::AF_INET as libc::sa_family_t;
                sin.sin_port = v4.port().to_be();
                sin.sin_addr = libc::in_addr {
                    s_addr: u32::from_ne_bytes(v4.ip().octets()),
                };
                // SAFETY: sockaddr_storage is at least as large and as aligned
                // as sockaddr_in; writing the smaller struct into its prefix
                // is the standard C idiom for this union.
                unsafe { core::ptr::write((&raw mut storage).cast::<libc::sockaddr_in>(), sin) };
                size_of::<libc::sockaddr_in>()
            }
            SocketAddr::V6(v6) => {
                // SAFETY: as above, for sockaddr_in6.
                let mut sin6: libc::sockaddr_in6 = unsafe { zeroed() };
                sin6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
                sin6.sin6_port = v6.port().to_be();
                sin6.sin6_flowinfo = v6.flowinfo();
                sin6.sin6_scope_id = v6.scope_id();
                sin6.sin6_addr = libc::in6_addr {
                    s6_addr: v6.ip().octets(),
                };
                // SAFETY: as above.
                unsafe { core::ptr::write((&raw mut storage).cast::<libc::sockaddr_in6>(), sin6) };
                size_of::<libc::sockaddr_in6>()
            }
        };
        Self {
            storage,
            len: len as libc::socklen_t,
        }
    }

    /// The raw storage (for passing to the kernel).
    #[must_use]
    pub fn as_storage(&self) -> &libc::sockaddr_storage {
        &self.storage
    }

    /// Mutable raw storage (for the kernel to fill).
    pub fn as_storage_mut(&mut self) -> &mut libc::sockaddr_storage {
        &mut self.storage
    }

    /// Number of meaningful bytes in the storage.
    #[must_use]
    pub fn len(&self) -> libc::socklen_t {
        self.len
    }

    /// `true` if no address has been stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Record the length the kernel reported after `recvmsg`.
    pub fn set_len(&mut self, len: libc::socklen_t) {
        self.len = len;
    }

    fn family_raw(&self) -> libc::c_int {
        libc::c_int::from(self.storage.ss_family)
    }

    /// Decode to a [`SocketAddr`].
    ///
    /// # Errors
    /// [`HomaError::InvalidAddress`] if the family is not `AF_INET`/`AF_INET6`
    /// or the stored length is too short for it.
    pub fn to_socket_addr(&self) -> Result<SocketAddr> {
        match self.family_raw() {
            f if f == libc::AF_INET => {
                if (self.len as usize) < size_of::<libc::sockaddr_in>() {
                    return Err(HomaError::InvalidAddress(format!(
                        "AF_INET address too short ({} bytes)",
                        self.len
                    )));
                }
                // SAFETY: family and length checked; storage is aligned for
                // sockaddr_in and read by value (no reference escapes).
                let sin = unsafe { (&raw const self.storage).cast::<libc::sockaddr_in>().read() };
                let ip = Ipv4Addr::from(sin.sin_addr.s_addr.to_ne_bytes());
                Ok(SocketAddr::V4(SocketAddrV4::new(
                    ip,
                    u16::from_be(sin.sin_port),
                )))
            }
            f if f == libc::AF_INET6 => {
                if (self.len as usize) < size_of::<libc::sockaddr_in6>() {
                    return Err(HomaError::InvalidAddress(format!(
                        "AF_INET6 address too short ({} bytes)",
                        self.len
                    )));
                }
                // SAFETY: as above, for sockaddr_in6.
                let sin6 = unsafe {
                    (&raw const self.storage)
                        .cast::<libc::sockaddr_in6>()
                        .read()
                };
                let ip = Ipv6Addr::from(sin6.sin6_addr.s6_addr);
                Ok(SocketAddr::V6(SocketAddrV6::new(
                    ip,
                    u16::from_be(sin6.sin6_port),
                    sin6.sin6_flowinfo,
                    sin6.sin6_scope_id,
                )))
            }
            other => Err(HomaError::InvalidAddress(format!(
                "unsupported address family {other}"
            ))),
        }
    }
}

/// Resolve `addr` to a single [`SocketAddr`] of the given family.
///
/// Accepts anything `std` accepts (`"127.0.0.1:5000"`, `("host", 5000)`,
/// [`SocketAddr`], ...). If `family` is `Some`, the first address of that
/// family is chosen; otherwise the first address wins.
///
/// # Errors
/// [`HomaError::InvalidAddress`] if resolution fails or yields no address, or
/// [`HomaError::AddressFamilyMismatch`] if nothing matches `family`.
pub fn resolve<A: ToSocketAddrs>(addr: A, family: Option<AddressFamily>) -> Result<SocketAddr> {
    let addrs: Vec<SocketAddr> = addr
        .to_socket_addrs()
        .map_err(|e| HomaError::InvalidAddress(e.to_string()))?
        .collect();
    let Some(first) = addrs.first().copied() else {
        return Err(HomaError::InvalidAddress(
            "address resolved to nothing".into(),
        ));
    };
    match family {
        None => Ok(first),
        Some(want) => addrs
            .into_iter()
            .find(|a| AddressFamily::of(a) == want)
            .ok_or(HomaError::AddressFamilyMismatch {
                socket: want,
                address: AddressFamily::of(&first),
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn v4_round_trip() {
        let addr: SocketAddr = "192.168.1.20:5001".parse().unwrap();
        let raw = RawSocketAddr::from_socket_addr(&addr);
        assert_eq!(raw.len() as usize, size_of::<libc::sockaddr_in>());
        assert_eq!(
            raw.as_storage().ss_family,
            libc::AF_INET as libc::sa_family_t
        );
        assert_eq!(raw.to_socket_addr().unwrap(), addr);
        assert_eq!(format!("{raw:?}"), "RawSocketAddr(192.168.1.20:5001)");
    }

    #[cfg(unix)]
    #[test]
    fn v6_round_trip_keeps_scope_and_flow() {
        let addr = SocketAddr::V6(SocketAddrV6::new(
            "fe80::1".parse().unwrap(),
            4000,
            0x1234,
            7,
        ));
        let raw = RawSocketAddr::from_socket_addr(&addr);
        assert_eq!(raw.len() as usize, size_of::<libc::sockaddr_in6>());
        assert_eq!(raw.to_socket_addr().unwrap(), addr);
    }

    #[cfg(unix)]
    #[test]
    fn port_is_network_byte_order() {
        let addr: SocketAddr = "10.0.0.1:4660".parse().unwrap();
        let raw = RawSocketAddr::from_socket_addr(&addr);
        // SAFETY: test-only read of a sockaddr_in we just wrote.
        let sin = unsafe {
            (&raw const *raw.as_storage())
                .cast::<libc::sockaddr_in>()
                .read()
        };
        assert_eq!(sin.sin_port.to_ne_bytes(), 4660u16.to_be_bytes());
        assert_eq!(sin.sin_addr.s_addr.to_ne_bytes(), [10, 0, 0, 1]);
    }

    #[cfg(unix)]
    #[test]
    fn empty_and_wrong_family_are_rejected() {
        let raw = RawSocketAddr::empty();
        assert!(raw.is_empty());
        assert!(matches!(
            raw.to_socket_addr(),
            Err(HomaError::InvalidAddress(_))
        ));

        let mut raw = RawSocketAddr::from_socket_addr(&"1.2.3.4:5".parse().unwrap());
        raw.set_len(2); // too short
        assert!(matches!(
            raw.to_socket_addr(),
            Err(HomaError::InvalidAddress(_))
        ));
    }

    #[test]
    fn resolve_filters_by_family() {
        let v4 = resolve("127.0.0.1:5000", Some(AddressFamily::V4)).unwrap();
        assert!(v4.is_ipv4());
        let err = resolve("127.0.0.1:5000", Some(AddressFamily::V6)).unwrap_err();
        assert!(matches!(err, HomaError::AddressFamilyMismatch { .. }));
        let any = resolve("[::1]:5000", None).unwrap();
        assert!(any.is_ipv6());
        assert!(matches!(
            resolve("not an address", None),
            Err(HomaError::InvalidAddress(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn family_to_raw_matches_libc() {
        assert_eq!(AddressFamily::V4.to_raw(), libc::AF_INET);
        assert_eq!(AddressFamily::V6.to_raw(), libc::AF_INET6);
    }

    #[test]
    fn family_helpers() {
        assert_eq!(
            AddressFamily::of(&"[::1]:1".parse().unwrap()),
            AddressFamily::V6
        );
    }
}
