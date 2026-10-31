//! Received messages as native-owned allocations.
//!
//! `homa_rs::Message` already owns its payload (a `Vec<u8>` copied out of the
//! kernel receive region). A `homa_message_t *` is that message moved into a
//! `Box`, so the payload pointer handed to C stays valid, at a fixed address,
//! until `homa_message_free`. Nothing is zero-copy and nothing points at the
//! kernel region or at a Rust stack frame.
//!
//! Ownership: each successfully returned message must be freed exactly once
//! with `homa_message_free`; null is accepted and ignored. Freeing twice or
//! passing a pointer that did not come from this library is undefined
//! behaviour, exactly as with `free(3)`; the `.NET` `SafeHandle` wrapper and
//! the C example both make the release unconditional and single.

use homa_rs::Message;

use crate::types::{
    HOMA_MESSAGE_REQUEST, HOMA_MESSAGE_RESPONSE, homa_endpoint_t, homa_message_info_t,
};

/// Opaque C-side type; only ever used behind a pointer produced by this crate.
#[repr(C)]
pub struct homa_message_t {
    _private: [u8; 0],
}

/// Move a message to the heap and hand ownership to C.
#[must_use]
pub fn into_raw(msg: Message) -> *mut homa_message_t {
    Box::into_raw(Box::new(msg)).cast()
}

/// Borrow the message behind a pointer produced by [`into_raw`].
///
/// # Safety
///
/// `ptr` must be non-null, come from [`into_raw`], and not have been freed.
pub unsafe fn borrow<'a>(ptr: *const homa_message_t) -> &'a Message {
    // SAFETY: caller contract; `into_raw` produced a valid `Box<Message>`.
    unsafe { &*ptr.cast::<Message>() }
}

/// Take ownership back from C and drop the message. Null is a no-op.
///
/// # Safety
///
/// `ptr` must be null or come from [`into_raw`] and not have been freed.
pub unsafe fn free(ptr: *mut homa_message_t) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: caller contract; this is the inverse of `into_raw`.
    drop(unsafe { Box::from_raw(ptr.cast::<Message>()) });
}

/// Fill a C info struct from a message.
pub fn fill_info(msg: &Message, out: &mut homa_message_info_t) {
    out.kind = if msg.is_request() {
        HOMA_MESSAGE_REQUEST
    } else {
        HOMA_MESSAGE_RESPONSE
    };
    out.rpc_id = msg.id.as_raw();
    out.completion_cookie = msg.completion_cookie;
    out.peer = homa_endpoint_t::from_socket_addr(&msg.peer);
    out.payload = msg.payload.as_ptr();
    out.payload_len = msg.payload.len() as u64;
    out.fragments = msg.fragments as u32;
    out.reserved = 0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use homa_rs::RpcId;

    fn sample() -> Message {
        Message {
            id: RpcId::from_raw(43),
            peer: "10.1.2.3:4567".parse().unwrap(),
            completion_cookie: 0,
            payload: b"a\0b".to_vec(),
            fragments: 1,
        }
    }

    #[test]
    fn payload_pointer_is_stable_until_free() {
        let msg = sample();
        let expected = msg.payload.clone();
        let raw = into_raw(msg);
        let mut info = homa_message_info_t::default();
        // SAFETY: `raw` is live.
        fill_info(unsafe { borrow(raw) }, &mut info);
        assert_eq!(info.kind, HOMA_MESSAGE_REQUEST);
        assert_eq!(info.rpc_id, 43);
        assert_eq!(info.payload_len, 3);
        assert_eq!(info.peer.port, 4567);
        // SAFETY: payload is valid for payload_len bytes while `raw` lives.
        let bytes = unsafe { std::slice::from_raw_parts(info.payload, info.payload_len as usize) };
        assert_eq!(bytes, expected.as_slice());
        // SAFETY: freeing exactly once.
        unsafe { free(raw) };
        // SAFETY: null is explicitly allowed.
        unsafe { free(std::ptr::null_mut()) };
    }
}
