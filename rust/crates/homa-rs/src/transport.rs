//! Request/response vocabulary shared by the socket layer and callers.
//!
//! Homa assigns every RPC a 64-bit identifier. The client sees an **even** id;
//! the server sees the same id with the low bit **set**. A received message is
//! therefore a request exactly when its id is odd, and the id a server passes
//! back when replying is the odd one it received. [`RpcId`] encodes these
//! rules so callers never manipulate the raw bit.
//!
//! The kernel never issues id `0` (client ids start at 2 and advance by 2;
//! `homa_utils.c`/`homa_rpc.c`). `0` is the "any shared message" selector in
//! `homa_recvmsg_args`, so [`RpcId::is_valid`] is `false` for it and the
//! socket layer refuses to wait on it.

use std::fmt;
use std::net::SocketAddr;

/// Identifier of a Homa RPC as seen by *this* host.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RpcId(u64);

impl RpcId {
    /// Wrap a raw identifier from the kernel.
    #[must_use]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    /// The raw 64-bit value expected by `homa_sendmsg_args`/`homa_recvmsg_args`.
    #[must_use]
    pub const fn as_raw(self) -> u64 {
        self.0
    }

    /// `true` if this host is the **server** for the RPC (low bit set), i.e.
    /// the message carrying this id is a request that expects a reply.
    #[must_use]
    pub const fn is_server_side(self) -> bool {
        self.0 & 1 == 1
    }

    /// `true` if this host is the **client** for the RPC (low bit clear).
    ///
    /// Note that `RpcId(0)` also has the low bit clear but is not a valid
    /// id; check [`RpcId::is_valid`] before passing an id to the kernel.
    #[must_use]
    pub const fn is_client_side(self) -> bool {
        !self.is_server_side()
    }

    /// `false` for `0`, which the kernel never assigns to an RPC.
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }

    /// Kind of message that carries this id when received.
    #[must_use]
    pub const fn kind(self) -> MessageKind {
        if self.is_server_side() {
            MessageKind::Request
        } else {
            MessageKind::Response
        }
    }

    /// The client's view of the same RPC (low bit cleared). Useful for
    /// correlating server logs with client logs.
    #[must_use]
    pub const fn client_view(self) -> Self {
        Self(self.0 & !1)
    }
}

impl fmt::Debug for RpcId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RpcId({})", self.0)
    }
}

impl fmt::Display for RpcId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<RpcId> for u64 {
    fn from(id: RpcId) -> Self {
        id.0
    }
}

/// Whether a received message is a request (needs a reply) or a response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageKind {
    /// An incoming request; the receiver must eventually reply.
    Request,
    /// The response to a request this host sent earlier.
    Response,
}

/// A complete message received from a Homa socket.
///
/// The payload has been **copied** out of the kernel receive region; the
/// region pages were handed back to the kernel (or queued for hand-back) by
/// the time this value is returned, so `Message` has no lifetime ties to the
/// socket and can be moved freely.
#[derive(Clone, PartialEq, Eq)]
pub struct Message {
    /// RPC identifier (odd ⇒ request, even ⇒ response).
    pub id: RpcId,
    /// Address of the peer that sent this message. For a request this is the
    /// client to reply to; for a response it is the server.
    pub peer: SocketAddr,
    /// For responses, the cookie supplied with the original request; `0` for
    /// requests.
    pub completion_cookie: u64,
    /// The message bytes.
    pub payload: Vec<u8>,
    /// Number of receive buffer pages the kernel used for this message
    /// (diagnostic; messages > 64 KiB span several).
    pub fragments: usize,
}

impl Message {
    /// Request or response?
    #[must_use]
    pub const fn kind(&self) -> MessageKind {
        self.id.kind()
    }

    /// Shorthand for `kind() == MessageKind::Request`.
    #[must_use]
    pub const fn is_request(&self) -> bool {
        self.id.is_server_side()
    }

    /// Shorthand for `kind() == MessageKind::Response`.
    #[must_use]
    pub const fn is_response(&self) -> bool {
        self.id.is_client_side()
    }

    /// Payload length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.payload.len()
    }

    /// Always `false` for messages produced by Homa (zero-length messages are
    /// rejected by the kernel), but provided for API symmetry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Message")
            .field("id", &self.id)
            .field("kind", &self.kind())
            .field("peer", &self.peer)
            .field("completion_cookie", &self.completion_cookie)
            .field("len", &self.payload.len())
            .field("fragments", &self.fragments)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_id_parity_rules() {
        let server = RpcId::from_raw(1001);
        let client = RpcId::from_raw(1000);
        assert!(server.is_server_side());
        assert!(!server.is_client_side());
        assert_eq!(server.kind(), MessageKind::Request);
        assert!(client.is_client_side());
        assert_eq!(client.kind(), MessageKind::Response);
        assert_eq!(server.client_view(), client);
        assert_eq!(client.client_view(), client);
        assert_eq!(u64::from(server), 1001);
        assert_eq!(server.to_string(), "1001");
        assert_eq!(format!("{server:?}"), "RpcId(1001)");
        assert!(client.is_valid() && server.is_valid());
        assert!(!RpcId::from_raw(0).is_valid());
        assert!(RpcId::from_raw(0).is_client_side());
    }

    #[test]
    fn message_kind_follows_id() {
        let peer: SocketAddr = "10.0.0.1:5000".parse().unwrap();
        let req = Message {
            id: RpcId::from_raw(3),
            peer,
            completion_cookie: 0,
            payload: b"hi".to_vec(),
            fragments: 1,
        };
        assert!(req.is_request());
        assert!(!req.is_response());
        assert_eq!(req.len(), 2);
        assert!(!req.is_empty());
        let dbg = format!("{req:?}");
        assert!(dbg.contains("Request") && dbg.contains("len: 2"), "{dbg}");
    }
}
