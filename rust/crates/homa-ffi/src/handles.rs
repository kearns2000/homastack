//! Socket handles: a generation-checked registry instead of raw pointers.
//!
//! A C caller cannot be trusted to never use a handle after closing it, and a
//! handle that is a raw heap pointer turns that mistake into a use-after-free.
//! Instead, `homa_socket_t *` values are **opaque keys**: the upper 32 bits are
//! a generation counter and the lower 32 bits a slot index into a
//! process-wide table. Every exported function looks the key up; a key that
//! was closed (or never issued) simply fails with
//! `HOMA_ERR_INVALID_ARGUMENT`. Keys are never dereferenced.
//!
//! Each slot holds an `Arc<Socket>`. Lookups clone the `Arc` while holding the
//! table lock for a few instructions, then release it, so blocking receives
//! run without any global lock held and sockets can be used from many threads
//! at once (the restrictions that remain are the kernel's; see the crate docs).
//! `close` removes the entry; the socket is destroyed when the last in-flight
//! operation drops its clone. Because that could be delayed by a thread that
//! is blocked in a receive, `close` first calls `shutdown` when it sees other
//! references, which wakes those threads with `HOMA_ERR_SHUTDOWN`.

use std::sync::{Arc, Mutex, PoisonError};

/// The socket type behind a handle: the real one on Linux, an uninhabitable
/// stand-in elsewhere so the registry (and every signature) compiles
/// identically on all platforms.
#[cfg(target_os = "linux")]
pub type Socket = homa_rs::HomaSocket;

/// See [`Socket`]. Never constructed.
#[cfg(not(target_os = "linux"))]
#[derive(Debug)]
pub enum Socket {}

/// Opaque C-side type. Only ever used behind a pointer whose *value* is a
/// registry key; it has no fields and is never dereferenced.
#[repr(C)]
pub struct homa_socket_t {
    _private: [u8; 0],
}

/// Registry key as carried inside a `*mut homa_socket_t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HandleKey(u64);

impl HandleKey {
    const fn new(index: u32, generation: u32) -> Self {
        Self((generation as u64) << 32 | index as u64)
    }

    const fn index(self) -> u32 {
        self.0 as u32
    }

    const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    /// Encode as the opaque pointer handed to C.
    #[must_use]
    pub const fn to_ptr(self) -> *mut homa_socket_t {
        self.0 as usize as *mut homa_socket_t
    }

    /// Decode a pointer received from C (`None` for null).
    #[must_use]
    pub fn from_ptr(ptr: *const homa_socket_t) -> Option<Self> {
        if ptr.is_null() {
            None
        } else {
            Some(Self(ptr as usize as u64))
        }
    }
}

#[derive(Debug)]
struct Slot {
    generation: u32,
    socket: Option<Arc<Socket>>,
}

#[derive(Debug, Default)]
struct Table {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

/// The process-wide handle table.
#[derive(Debug, Default)]
pub struct Registry {
    table: Mutex<Table>,
}

impl Registry {
    /// Empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            table: Mutex::new(Table {
                slots: Vec::new(),
                free: Vec::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Table> {
        // A panic while holding this lock can only happen between two
        // self-consistent Vec operations; the data is still valid.
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Store `socket` and return its key, or `None` if 2^32 handles are live
    /// (each one holds an open file descriptor, so this cannot happen in
    /// practice).
    pub fn insert(&self, socket: Arc<Socket>) -> Option<HandleKey> {
        let mut t = self.lock();
        if let Some(index) = t.free.pop() {
            let slot = &mut t.slots[index as usize];
            // Generation was already advanced on removal and is never 0.
            slot.socket = Some(socket);
            Some(HandleKey::new(index, slot.generation))
        } else {
            let index = u32::try_from(t.slots.len()).ok()?;
            t.slots.push(Slot {
                generation: 1,
                socket: Some(socket),
            });
            Some(HandleKey::new(index, 1))
        }
    }

    /// Clone the socket behind `key`, if it is live.
    #[must_use]
    pub fn get(&self, key: HandleKey) -> Option<Arc<Socket>> {
        let t = self.lock();
        let slot = t.slots.get(key.index() as usize)?;
        if slot.generation != key.generation() {
            return None;
        }
        slot.socket.clone()
    }

    /// Remove `key`, returning the socket if it was live. The slot's
    /// generation is advanced so the old key can never match again.
    pub fn remove(&self, key: HandleKey) -> Option<Arc<Socket>> {
        let mut t = self.lock();
        let index = key.index() as usize;
        let slot = t.slots.get_mut(index)?;
        if slot.generation != key.generation() {
            return None;
        }
        let socket = slot.socket.take()?;
        // Wrap to 1, never 0, so a key of all-zero bits is always invalid.
        slot.generation = slot.generation.checked_add(1).unwrap_or(1);
        t.free.push(key.index());
        Some(socket)
    }

    /// Number of live handles (diagnostics / tests).
    #[must_use]
    pub fn live(&self) -> usize {
        self.lock()
            .slots
            .iter()
            .filter(|s| s.socket.is_some())
            .count()
    }
}

/// The one registry used by the exported functions.
pub static REGISTRY: Registry = Registry::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_encoding_round_trips_and_null_is_invalid() {
        let k = HandleKey::new(7, 3);
        assert_eq!(k.index(), 7);
        assert_eq!(k.generation(), 3);
        assert_eq!(HandleKey::from_ptr(k.to_ptr()), Some(k));
        assert!(HandleKey::from_ptr(std::ptr::null()).is_none());
        assert!(!HandleKey::new(0, 1).to_ptr().is_null());
    }

    // The registry itself can only be exercised with a constructible socket
    // type, i.e. on Linux (tests/ffi_kernel.rs) - except for the lookup of
    // keys that were never issued, which is platform independent.
    #[test]
    fn unknown_keys_are_rejected() {
        let r = Registry::new();
        assert!(r.get(HandleKey::new(0, 1)).is_none());
        assert!(r.remove(HandleKey::new(0, 1)).is_none());
        assert!(
            r.get(HandleKey::from_ptr(0xdead_beef as *const homa_socket_t).unwrap())
                .is_none()
        );
        assert_eq!(r.live(), 0);
    }
}
