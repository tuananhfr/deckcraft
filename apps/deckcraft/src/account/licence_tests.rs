//! The edit gate against pi-mock's Office free plan: locked for a guest and in the personal
//! context, editable with this device's `erpconsoffice` lease, locked again when the device is
//! revoked or the lease runs out offline, and Retry or a refused edit's resync lifting a block.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64};
use std::time::{Duration, Instant};

use erpcons_account::{Clock, MemorySecretStore, OFFICE_PI_PRODUCT, PinnedKey, Product};
use pi_mock::{Fault, ORG_A, ORG_OFFICE, Scenario, Server, Tenant};

use super::tests::{follow_redirect, mock_config, opened, wait_for};
use super::*;

const T0: i64 = 1_792_828_800;

struct TestClock {
    unix: AtomicI64,
    mono_ms: AtomicU64,
}

impl Clock for TestClock {
    fn unix_now(&self) -> i64 {
        self.unix.load(Ordering::SeqCst)
    }
    fn monotonic(&self) -> Duration {
        Duration::from_millis(self.mono_ms.load(Ordering::SeqCst))
    }
}

struct Env {
    server: Server,
    dir: tempfile::TempDir,
    clock: Arc<TestClock>,
}

impl Env {
    fn new(mut scenario: Scenario) -> Self {
        scenario.now.get_or_insert(T0);
        Self {
            server: Server::start(scenario),
            dir: tempfile::tempdir().unwrap(),
            clock: Arc::new(TestClock { unix: AtomicI64::new(T0), mono_ms: AtomicU64::new(1_000_000) }),
        }
    }

    /// The hook as `install` makes it, with the mock's lease key, a test clock and resyncs
    /// within milliseconds.
    fn hook(&self) -> DesktopAccount {
        let mut config = mock_config(&self.server, &self.dir);
        config.clock = self.clock.clone();
        // PI held back longer than this reads as offline.
        config.timeout = Duration::from_secs(1);
        let timing = ResyncTiming { min: Duration::from_millis(20), max: Duration::from_millis(50) };
        opened(DesktopAccount::with_timing(config, Arc::new(MemorySecretStore::new()), Box::new(follow_redirect), timing))
    }

    /// Moves the client's wall and monotonic clocks and PI's time together.
    fn advance(&self, secs: i64) {
        self.clock.unix.fetch_add(secs, Ordering::SeqCst);
        self.clock.mono_ms.fetch_add(secs.unsigned_abs() * 1000, Ordering::SeqCst);
        let now = self.clock.unix_now();
        self.server.update_scenario(|s| s.now = Some(now));
    }
}

fn account(hook: &DesktopAccount) -> Arc<Account> {
    hook.shared.account.get().cloned().expect("opened")
}

fn wait_lock(hook: &DesktopAccount, expected: Option<EditLock>) {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let lock = hook.edit_lock();
        if lock == expected && !hook.snapshot().busy {
            return;
        }
        assert!(Instant::now() < until, "timed out waiting for {expected:?}, still {lock:?}: {:?}", hook.snapshot());
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn signed_in(hook: &DesktopAccount) {
    hook.sign_in();
    wait_for(hook, "sign-in", |s| matches!(s.view, AccountView::SignedIn(_)));
}

#[test]
fn a_guest_is_locked_and_signing_in_to_the_free_plan_unlocks_editing() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    wait_for(&hook, "guest", |s| s.view == AccountView::Guest);
    assert_eq!(hook.edit_lock(), Some(EditLock::SignIn));
    signed_in(&hook);
    wait_lock(&hook, None);
    let AccountView::SignedIn(view) = hook.snapshot().view else { panic!("signed in") };
    assert_eq!(view.active_org.as_deref(), Some(ORG_OFFICE));
    assert!(matches!(view.licence, LicenceView::Active { pro: false, .. }), "the free plan is no paid plan: {:?}", view.licence);
    let activations = env.server.requests().into_iter().filter(|r| r.path == "/api/v1/id/devices/activate").count();
    assert_eq!(activations, 1, "one activation, for the one Office product");
    assert_eq!(env.server.device_rows().len(), 1);
}

#[test]
fn the_free_plan_lease_raises_no_expiry_warning() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    signed_in(&hook);
    wait_lock(&hook, None);
    // Two days left on the 7-day lease: the library's 3-day stage.
    env.advance(5 * 86_400);
    let status = account(&hook).status();
    let Status::SignedIn(s) = &status else { panic!("signed in") };
    assert!(s.warning.is_some(), "the library still raises it for `use`");
    let AccountView::SignedIn(view) = view_of(&status) else { panic!("signed in") };
    assert!(matches!(view.licence, LicenceView::Active { pro: false, .. }), "read as Free, so no notice: {:?}", view.licence);
    assert_eq!(hook.edit_lock(), None, "still editable");
}

#[test]
fn the_personal_context_asks_for_an_organization() {
    let env = Env::new(Scenario { tenants: vec![Tenant::office_free("ten_a", ORG_A)], ..Scenario::default() });
    let hook = env.hook();
    signed_in(&hook);
    wait_lock(&hook, Some(EditLock::ChooseOrg));
    hook.select_org(Some(ORG_A.into()));
    wait_lock(&hook, None);
}

#[test]
fn a_revoked_device_is_locked_after_the_next_sync() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    signed_in(&hook);
    wait_lock(&hook, None);
    let account = account(&hook);
    let device = account.diagnostics().device_id.expect("activated");
    assert!(env.server.revoke_device(&device));
    env.advance(13 * 3600);
    let _ = account.refresh();
    assert_eq!(hook.edit_lock(), Some(EditLock::Revoked));
    assert!(!EditLock::Revoked.retry_offered());
}

#[test]
fn a_lease_that_runs_out_offline_locks_until_it_is_renewed() {
    let env = Env::new(Scenario::office_free());
    let hook = env.hook();
    signed_in(&hook);
    wait_lock(&hook, None);
    let account = account(&hook);
    env.advance(8 * 86_400);
    // The connection drops after the token call (held back past the timeout, which reads as
    // offline); a dropped token call would cost the rotated refresh token. Added after
    // `advance`, which resets the scenario's faults.
    env.server.add_fault(Fault::delay("GET", "/api/v1/id/me", Duration::from_secs(2)));
    assert!(account.refresh().is_err(), "PI is unreachable");
    assert_eq!(hook.edit_lock(), Some(EditLock::RenewOffline));

    // Back online: a refused edit's resync renews the lease.
    env.server.clear_faults();
    hook.edit_refused();
    wait_lock(&hook, None);
}

#[test]
fn retry_activates_once_the_organization_has_a_plan() {
    let env = Env::new(Scenario { tenants: Vec::new(), ..Scenario::office_free() });
    let hook = env.hook();
    signed_in(&hook);
    wait_lock(&hook, Some(EditLock::NoPlan));
    env.server.update_scenario(|scenario| scenario.tenants = vec![Tenant::office_free("ten_office", ORG_OFFICE)]);
    // A plain sync keeps the not-entitled block (ADR 0013); only Retry activates again.
    let _ = account(&hook).refresh();
    assert_eq!(hook.edit_lock(), Some(EditLock::NoPlan));
    hook.retry_activation();
    wait_lock(&hook, None);
}

#[test]
fn a_refused_edit_syncs_again_without_waiting_for_the_hourly_refresh() {
    let env = Env::new(Scenario { faults: vec![Fault::problem("POST", "/api/v1/id/devices/activate", 422, "")], ..Scenario::office_free() });
    let hook = env.hook();
    signed_in(&hook);
    wait_lock(&hook, Some(EditLock::NotActivated));
    env.server.clear_faults();
    hook.edit_refused();
    wait_lock(&hook, None);
}

#[test]
fn without_an_account_editing_stays_locked() {
    let hook = DesktopAccount::unavailable();
    assert_eq!(hook.edit_lock(), Some(EditLock::Unavailable));
    hook.sign_in();
    assert_eq!(hook.edit_lock(), Some(EditLock::Unavailable), "nothing can open it later");
    assert_eq!(hook.snapshot().error, Some(AccountError::Other));
}

#[test]
fn licensing_is_on_for_the_office_product_and_the_dev_lease_key_parses() {
    let config = config::config_for("http://127.0.0.1:9", Some("http://127.0.0.1:9/api"), "1.2.3");
    assert!(config.offline_licensing);
    assert_eq!(config.product, Product::TrinhChieu);
    assert_eq!(config.profile.pi_product, OFFICE_PI_PRODUCT);
    assert_eq!(config.client_id, "erpcons-office-desktop");
    assert_eq!(config.api_base.as_deref(), Some("http://127.0.0.1:9/api"));
    assert!(config.platform_keys.iter().any(|k| k.kid == "lease-2026-10"), "production's pinned key");
    let hex: String = pi_mock::dev_lease_public_key().iter().map(|b| format!("{b:02x}")).collect();
    let key = config::parse_dev_lease_key(&format!("{}:{hex}", pi_mock::DEV_LEASE_KID));
    assert_eq!(key, Some(PinnedKey { kid: pi_mock::DEV_LEASE_KID.into(), key: pi_mock::dev_lease_public_key() }));
    for bad in ["", "kid", ":00", "kid:00", &format!("kid:{hex}0"), &format!("kid:{}", "zz".repeat(32)), &format!("kid:+{}", &hex[1..])] {
        assert_eq!(config::parse_dev_lease_key(bad), None, "{bad:?}");
    }
}
