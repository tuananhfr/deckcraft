//! The edit gate: editing needs `erpconsoffice.use` in an active lease of this device (ADR 0013
//! of `erpcons-account`), read from cached state only.

use std::time::{Duration, Instant};

use deckcraft_ui_egui::account::EditLock;
use erpcons_account::{Error, LicenceState, OFFICE_USE, Status};

/// Why edits are refused for `status`, `None` when they are allowed; `allowed` is asked only
/// while the lease lists editing.
pub(super) fn lock_of(status: &Status, allowed: impl FnOnce(&str) -> bool) -> Option<EditLock> {
    let signed_in = match status {
        Status::Guest | Status::SigningIn => return Some(EditLock::SignIn),
        Status::SignedIn(signed_in) => signed_in,
    };
    // PI leases to organizations only; each Office user gets a one-person one (D578).
    if signed_in.active_org.is_none() {
        return Some(EditLock::ChooseOrg);
    }
    // `allowed()` alone also grants on fresh online entitlements; only this device's lease
    // reflects a revoked or capped device, so it must grant too.
    match &signed_in.licence {
        LicenceState::DeviceRevoked => Some(EditLock::Revoked),
        LicenceState::DeviceCapReached => Some(EditLock::DeviceCap),
        LicenceState::NotEntitled => Some(EditLock::NoPlan),
        LicenceState::NotActivated => Some(EditLock::NotActivated),
        LicenceState::SessionOnly => Some(EditLock::SessionOnly),
        LicenceState::Inactive { .. } if !signed_in.online => Some(EditLock::RenewOffline),
        LicenceState::Inactive { .. } => Some(EditLock::RenewFailed),
        LicenceState::Active { features, .. } if features.iter().any(|f| f == OFFICE_USE) && allowed(OFFICE_USE) => None,
        LicenceState::Active { .. } => Some(EditLock::NoPlan),
    }
}

/// Syncs asked for by refused edits: at most one a minute, doubling to an hour while the refusal
/// stays (the background schedule of lease.md §6).
#[derive(Clone, Copy, Debug)]
pub(super) struct ResyncTiming {
    pub min: Duration,
    pub max: Duration,
}

impl Default for ResyncTiming {
    fn default() -> Self {
        Self { min: Duration::from_secs(60), max: Duration::from_secs(3600) }
    }
}

/// When the next sync for a refused edit may run.
pub(super) struct Resync {
    pub timing: ResyncTiming,
    pub not_before: Instant,
    pub failures: u32,
}

impl Resync {
    pub fn new(timing: ResyncTiming, now: Instant) -> Self {
        Self { timing, not_before: now, failures: 0 }
    }

    /// Schedules the next one after a sync that ended with `result` and left the refusal in place
    /// (`still_refused`) or not.
    pub fn after(&mut self, now: Instant, result: &Result<(), Error>, still_refused: bool) {
        let delay = if still_refused || result.is_err() {
            let doubled = self.timing.min.saturating_mul(1 << self.failures.min(6)).min(self.timing.max);
            self.failures = self.failures.saturating_add(1);
            result.as_ref().err().and_then(Error::retry_after).map_or(doubled, |after| after.max(doubled))
        } else {
            self.failures = 0;
            self.timing.min
        };
        self.not_before = now.checked_add(delay).unwrap_or(now);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use deckcraft_ui_egui::account::EditLock;
    use erpcons_account::{Error, LicenceState, SignedIn, Status};

    use super::{Resync, ResyncTiming, lock_of};

    fn signed_in(active_org: Option<&str>, licence: LicenceState, online: bool) -> Status {
        Status::SignedIn(Box::new(SignedIn {
            user: None,
            orgs: Vec::new(),
            active_org: active_org.map(str::to_owned),
            licence,
            licences: Default::default(),
            warning: None,
            online,
            session_only: false,
            reauth_required: false,
        }))
    }

    #[test]
    fn denial_order_follows_sign_in_organization_then_the_lease() {
        let active = |features: &[&str]| LicenceState::Active {
            lease_id: "l".into(),
            features: features.iter().map(|f| f.to_string()).collect(),
            expires_at: 9,
        };
        let expired = LicenceState::Inactive { lease_id: "l".into(), expires_at: 9 };
        let cases = [
            (Status::Guest, Some(EditLock::SignIn)),
            (Status::SigningIn, Some(EditLock::SignIn)),
            (signed_in(None, active(&["erpconsoffice.use"]), true), Some(EditLock::ChooseOrg)),
            (signed_in(Some("o"), LicenceState::DeviceRevoked, true), Some(EditLock::Revoked)),
            (signed_in(Some("o"), LicenceState::DeviceCapReached, true), Some(EditLock::DeviceCap)),
            (signed_in(Some("o"), LicenceState::NotEntitled, true), Some(EditLock::NoPlan)),
            (signed_in(Some("o"), LicenceState::NotActivated, true), Some(EditLock::NotActivated)),
            (signed_in(Some("o"), LicenceState::SessionOnly, true), Some(EditLock::SessionOnly)),
            (signed_in(Some("o"), expired.clone(), false), Some(EditLock::RenewOffline)),
            (signed_in(Some("o"), expired, true), Some(EditLock::RenewFailed)),
            (signed_in(Some("o"), active(&["erpconsoffice.view", "erpconsoffice.export"]), true), Some(EditLock::NoPlan)),
            (signed_in(Some("o"), active(&["trinhchieu.use"]), true), Some(EditLock::NoPlan)),
            (signed_in(Some("o"), active(&["erpconsoffice.use"]), true), None),
        ];
        for (status, expected) in cases {
            assert_eq!(lock_of(&status, |_| true), expected, "{status:?}");
        }
        let lease = signed_in(Some("o"), active(&["erpconsoffice.use"]), false);
        assert_eq!(lock_of(&lease, |_| false), Some(EditLock::NoPlan), "the library has the last word");
        let mut asked = false;
        lock_of(&signed_in(Some("o"), LicenceState::NotActivated, true), |_| {
            asked = true;
            true
        });
        assert!(!asked, "allowed() is asked only while the lease lists editing");
    }

    #[test]
    fn resync_waits_a_minute_and_backs_off_to_an_hour_while_refused() {
        let now = Instant::now();
        let mut r = Resync::new(ResyncTiming::default(), now);
        let delays: Vec<u64> = (0..8)
            .map(|_| {
                r.after(now, &Ok(()), true);
                r.not_before.duration_since(now).as_secs()
            })
            .collect();
        assert_eq!(delays, [60, 120, 240, 480, 960, 1920, 3600, 3600]);
        r.after(now, &Err(Error::Timeout), false);
        assert_eq!(r.not_before.duration_since(now), Duration::from_secs(3600));
        r.after(now, &Ok(()), false);
        assert_eq!(r.not_before.duration_since(now), Duration::from_secs(60), "a lifted refusal resets the schedule");
    }
}
