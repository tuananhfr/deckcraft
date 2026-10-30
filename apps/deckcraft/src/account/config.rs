//! Where the desktop app finds PI and which keys verify its leases.

use erpcons_account::{Config, Product};

/// The account configuration for `issuer` (and PI's API under `api_base`, when it is elsewhere).
///
/// Release builds verify leases with the pinned `PLATFORM_KEYS` only. Debug builds also pin
/// `ERPCONS_PI_DEV_LEASE_KEY` (`kid:hex32`, read at run time) to run against pi-mock or a local PI.
pub(super) fn config_for(issuer: &str, api_base: Option<&str>, client_version: &str) -> Config {
    let mut config = Config::new(issuer, Product::TrinhChieu, client_version);
    config.api_base = api_base.map(Into::into);
    // Editing needs this device's `erpconsoffice` lease from PI's device protocol (ADR 0013).
    config.offline_licensing = true;
    #[cfg(debug_assertions)]
    config.platform_keys.extend(dev_lease_key());
    config
}

#[cfg(debug_assertions)]
fn dev_lease_key() -> Option<erpcons_account::PinnedKey> {
    let value = std::env::var("ERPCONS_PI_DEV_LEASE_KEY").ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())?;
    let key = parse_dev_lease_key(&value);
    if key.is_none() {
        log::warn!("ERPCONS_PI_DEV_LEASE_KEY is not kid:<64 hex digits>; ignored");
    }
    key
}

// A release that cannot verify a lease would lock every user out of editing.
const _: () = assert!(!erpcons_account::PLATFORM_KEYS.is_empty());

/// `kid:hex32`: a lease-signing key id and its Ed25519 public key in 64 hex digits.
#[cfg(debug_assertions)]
pub(super) fn parse_dev_lease_key(value: &str) -> Option<erpcons_account::PinnedKey> {
    let (kid, hex) = value.split_once(':')?;
    // `from_str_radix` alone would accept a leading `+`.
    if kid.is_empty() || hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = [0u8; 32];
    for (byte, pair) in key.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(erpcons_account::PinnedKey { kid: kid.to_owned(), key })
}
