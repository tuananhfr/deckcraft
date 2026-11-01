//! `setClientId` of the office shell's `erpHost` (live editing): a prefix of 24 random bits that
//! every id the session creates while editing is mixed with ([`deckcraft_model::IdSource`]), so
//! two people adding slides or shapes to one presentation at once never pick the same id. Without
//! one the session picks its own prefix.

use std::fmt;

use deckcraft_model::IdSource;

use crate::Session;

/// Six lowercase hex digits on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClientPrefix(u32);

impl ClientPrefix {
    /// Exactly six lowercase hex digits; anything else is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return None;
        }
        u32::from_str_radix(s, 16).ok().map(Self)
    }

    /// A prefix of the session's own.
    pub fn random() -> Self {
        Self(random_bits() & 0xff_ffff)
    }

    pub fn bits(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ClientPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:06x}", self.0)
    }
}

#[cfg(target_arch = "wasm32")]
fn random_bits() -> u32 {
    // `as` saturates; Math.random() is in [0, 1).
    (js_sys::Math::random() * f64::from(u32::MAX)) as u32
}

#[cfg(not(target_arch = "wasm32"))]
fn random_bits() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    // The OS seeds RandomState; the clock adds to it.
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let x = h.finish();
    (x ^ (x >> 32)) as u32
}

pub(crate) fn new_source() -> IdSource {
    IdSource::new(ClientPrefix::random().bits())
}

impl Session {
    /// Mix `prefix` into every id created from now on; ids created before keep theirs.
    pub fn set_client_prefix(&mut self, prefix: ClientPrefix) {
        self.ids.set_prefix(prefix.bits());
    }

    pub fn client_prefix(&self) -> ClientPrefix {
        ClientPrefix(self.ids.prefix().unwrap_or_default())
    }
}
