use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;

use erpcons_account::{ApiError, ExpiryWarning, Membership, MemorySecretStore, PinnedKey, SignedIn, User};
use pi_mock::{ORG_A, ORG_B, Scenario, Server};

use super::*;

fn membership(id: &str, status: MembershipStatus) -> Membership {
    Membership { org_id: id.into(), name: format!("Org {id}"), role: "member".into(), status }
}

fn signed_in_status(licence: LicenceState) -> Status {
    Status::SignedIn(Box::new(SignedIn {
        user: Some(User { user_id: "usr_1".into(), display_name: "Nguyễn An".into(), email: Some("an@example.com".into()) }),
        orgs: vec![
            membership("org_a", MembershipStatus::Active),
            Membership { role: String::new(), ..membership("org_b", MembershipStatus::Active) },
            membership("org_s", MembershipStatus::Suspended),
        ],
        active_org: Some("org_a".into()),
        licence,
        licences: BTreeMap::new(),
        warning: Some(ExpiryWarning { stage: WarningStage::Days3, expires_at: 1_800_000_000 }),
        online: false,
        session_only: false,
        reauth_required: true,
    }))
}

#[test]
fn status_maps_to_the_view() {
    assert_eq!(view_of(&Status::Guest), AccountView::Guest);
    assert_eq!(view_of(&Status::SigningIn), AccountView::SigningIn);
    let licence =
        LicenceState::Active { lease_id: "l".into(), features: vec!["trinhchieu.view".into(), "trinhchieu.use".into()], expires_at: 1_800_000_000 };
    let AccountView::SignedIn(view) = view_of(&signed_in_status(licence)) else { panic!("signed in") };
    assert_eq!(view.name, "Nguyễn An");
    assert_eq!(view.email.as_deref(), Some("an@example.com"));
    assert_eq!(view.orgs.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["org_a", "org_b"], "only active memberships can be picked");
    assert_eq!(view.orgs[0].role.as_deref(), Some("member"));
    assert_eq!(view.orgs[1].role, None, "an empty role is no role");
    assert_eq!(view.active_org.as_deref(), Some("org_a"));
    assert_eq!(view.licence, LicenceView::Active { expires_at: 1_800_000_000, pro: false });
    assert_eq!(view.warning, Some(WarningView { days: 3, expires_at: 1_800_000_000 }));
    assert!(!view.online && view.reauth_required && !view.session_only);
    let mut no_user = signed_in_status(LicenceState::NotActivated);
    if let Status::SignedIn(s) = &mut no_user {
        s.user = None;
    }
    let AccountView::SignedIn(view) = view_of(&no_user) else { panic!("signed in") };
    assert_eq!((view.name.as_str(), view.email), ("", None), "before /v1/me answered");
}

#[test]
fn licence_states_map_to_the_view() {
    let active =
        |features: &[&str]| LicenceState::Active { lease_id: "l".into(), features: features.iter().map(|f| f.to_string()).collect(), expires_at: 5 };
    assert_eq!(licence_of(&LicenceState::NotActivated), LicenceView::Free);
    assert_eq!(licence_of(&active(&["trinhchieu.view", "trinhchieu.export"])), LicenceView::Active { expires_at: 5, pro: false });
    assert_eq!(licence_of(&active(&["trinhchieu.view", "trinhchieu.advanced"])), LicenceView::Active { expires_at: 5, pro: true });
    assert_eq!(licence_of(&LicenceState::Inactive { lease_id: "l".into(), expires_at: 9 }), LicenceView::Inactive { expires_at: 9 });
    assert_eq!(licence_of(&LicenceState::DeviceRevoked), LicenceView::DeviceRevoked);
    assert_eq!(licence_of(&LicenceState::DeviceCapReached), LicenceView::DeviceCapReached);
    assert_eq!(licence_of(&LicenceState::SessionOnly), LicenceView::SessionOnly);
}

#[test]
fn library_errors_map_to_the_error_line() {
    let api = |status: u16, code: &str, retry: Option<u64>| {
        Error::Api(ApiError {
            status,
            code: ApiCode::from_parts(status, Some(code)),
            correlation_id: None,
            retry_after: retry.map(Duration::from_secs),
            max_devices: None,
            active_devices: None,
        })
    };
    let cases = [
        (Error::Cancelled, Some(AccountError::Cancelled)),
        (Error::SignInInProgress, None),
        (Error::Network("down".into()), Some(AccountError::Network)),
        (Error::SignInTimedOut, Some(AccountError::Timeout)),
        (Error::Timeout, Some(AccountError::Timeout)),
        (Error::AuthorizationDenied("access_denied".into()), Some(AccountError::Denied)),
        (Error::InvalidGrant, Some(AccountError::ReauthRequired)),
        (Error::NotSignedIn, Some(AccountError::ReauthRequired)),
        (Error::OrgUnavailable, Some(AccountError::OrgUnavailable)),
        (Error::UnknownOrg, Some(AccountError::OrgUnavailable)),
        (Error::Revoked(RevocationReason::DeviceRevoked), Some(AccountError::DeviceRevoked)),
        (Error::Revoked(RevocationReason::MembershipRevoked), Some(AccountError::OrgUnavailable)),
        (Error::Revoked(RevocationReason::UserSuspended), Some(AccountError::Suspended)),
        (Error::ActivationBlocked, None),
        (Error::CredentialStoreUnavailable, None),
        (Error::Browser("no browser".into()), Some(AccountError::Other)),
        (Error::StateMismatch, Some(AccountError::Other)),
        (api(401, "unauthenticated", None), Some(AccountError::ReauthRequired)),
        (api(403, "insufficient_scope", None), Some(AccountError::ReauthRequired)),
        (api(403, "user_suspended", None), Some(AccountError::Suspended)),
        (api(403, "device_revoked", None), Some(AccountError::DeviceRevoked)),
        (api(403, "membership_inactive", None), Some(AccountError::OrgUnavailable)),
        (api(403, "forbidden", None), Some(AccountError::Denied)),
        (api(409, "device_cap_reached", None), Some(AccountError::DeviceCap)),
        (api(412, "client_version_unsupported", None), Some(AccountError::ClientTooOld)),
        (api(429, "rate_limited", Some(120)), Some(AccountError::RateLimited { retry_after_secs: Some(120) })),
        (api(429, "quota_exceeded", None), Some(AccountError::Denied)),
        (api(503, "dependency_unavailable", None), Some(AccountError::ServiceUnavailable)),
        (api(422, "validation_failed", None), Some(AccountError::Other)),
    ];
    for (error, expected) in cases {
        assert_eq!(error_view(&error), expected, "{error:?}");
    }
}

#[test]
fn the_browser_pages_are_translated_and_escaped() {
    i18n::set_current(i18n::Lang::from_pref("vi"));
    assert!(callback_page("Signed in to PI ID. You can close this tab and go back to the app.").contains("Đã đăng nhập PI ID."));
    assert!(callback_page("Sign-in did not finish. Close this tab and try again in the app.").contains("Chưa đăng nhập xong."));
    i18n::set_current(i18n::Lang::EN);
    assert!(callback_page("a <b> & c").contains("a &lt;b&gt; &amp; c"));
}

#[test]
fn no_issuer_no_account() {
    // Test builds never set the issuer, so the app's normal start installs nothing.
    assert!(ISSUER.is_none_or(|issuer| issuer.trim().is_empty()), "tests must be built without ERPCONS_PI_ISSUER");
    assert!(install().is_none());
}

/// The system browser in tests: follows the authorization redirect to the app's loopback listener.
fn follow_redirect(url: &str) -> Result<(), String> {
    let url = url.to_owned();
    std::thread::spawn(move || {
        if let Some(location) = http_get(&url).and_then(|response| location_of(&response)) {
            let _ = http_get(&location);
        }
    });
    Ok(())
}

fn http_get(url: &str) -> Option<String> {
    let rest = url.strip_prefix("http://")?;
    let (host, path) = rest.split_once('/').map_or((rest, String::from("/")), |(host, path)| (host, format!("/{path}")));
    let mut stream = TcpStream::connect(host).ok()?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").ok()?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn location_of(response: &str) -> Option<String> {
    response.lines().take_while(|line| !line.is_empty()).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case("location").then(|| value.trim().to_owned())
    })
}

fn mock_config(server: &Server, dir: &tempfile::TempDir) -> Config {
    let mut config = Config::new(server.issuer(), Product::TrinhChieu, "0.6.5");
    // The mock's development key is pinned here only, never in a build.
    config.platform_keys = vec![PinnedKey { kid: server.lease_key_id(), key: server.lease_public_key() }];
    config.state_dir = Some(dir.path().to_path_buf());
    config.timeout = Duration::from_secs(5);
    config.max_retry_wait = Duration::from_secs(3);
    config.sign_in_timeout = Duration::from_secs(20);
    config
}

fn wait_for(hook: &DesktopAccount, what: &str, done: impl Fn(&AccountSnapshot) -> bool) -> AccountSnapshot {
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        let snapshot = hook.snapshot();
        if done(&snapshot) && !snapshot.busy {
            return snapshot;
        }
        assert!(Instant::now() < until, "timed out waiting for {what}: {snapshot:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn started(server: &Server, dir: &tempfile::TempDir, opener: Opener) -> DesktopAccount {
    let hook = DesktopAccount::new(mock_config(server, dir), Arc::new(MemorySecretStore::new()), opener).unwrap();
    hook.attach(&egui::Context::default());
    let until = Instant::now() + Duration::from_secs(10);
    while hook.shared.account.get().is_none() {
        assert!(Instant::now() < until, "the worker did not open the account");
        std::thread::sleep(Duration::from_millis(10));
    }
    hook
}

#[test]
fn signs_in_switches_organization_and_signs_out_against_pi_mock() {
    let server = Server::start(Scenario::default());
    let dir = tempfile::tempdir().unwrap();
    let hook = started(&server, &dir, Box::new(follow_redirect));
    assert_eq!(wait_for(&hook, "guest", |s| s.view == AccountView::Guest).error, None);
    assert!(hook.allowed("trinhchieu.use"));

    hook.sign_in();
    assert_eq!(hook.snapshot().view, AccountView::SigningIn, "shown at once");
    let snapshot = wait_for(&hook, "sign-in", |s| matches!(&s.view, AccountView::SignedIn(v) if v.orgs.len() == 2));
    assert_eq!(snapshot.error, None);
    let AccountView::SignedIn(view) = snapshot.view else { panic!("signed in") };
    assert_eq!(view.email.as_deref(), Some("an.nguyen@example.com"));
    assert!(!view.name.is_empty());
    assert_eq!(view.active_org, None);
    assert!(matches!(view.licence, LicenceView::Active { pro: false, .. }), "{:?}", view.licence);
    assert!(view.online && !view.session_only && !view.reauth_required);
    assert_eq!(view.warning, None);
    assert!(hook.allowed("trinhchieu.use"));

    hook.select_org(Some(ORG_B.into()));
    wait_for(&hook, "org switch", |s| matches!(&s.view, AccountView::SignedIn(v) if v.active_org.as_deref() == Some(ORG_B)));
    hook.select_org(Some(ORG_A.into()));
    wait_for(&hook, "second org switch", |s| matches!(&s.view, AccountView::SignedIn(v) if v.active_org.as_deref() == Some(ORG_A)));
    hook.select_org(None);
    let snapshot = wait_for(&hook, "personal context", |s| matches!(&s.view, AccountView::SignedIn(v) if v.active_org.is_none()));
    assert_eq!(snapshot.error, None);

    hook.sign_out();
    let snapshot = wait_for(&hook, "sign-out", |s| s.view == AccountView::Guest);
    assert_eq!(snapshot.error, None);

    let dir = dir.path().to_string_lossy().into_owned();
    for request in server.requests() {
        assert!(!request.body.contains(&dir) && !request.query.contains(&dir), "a local path left the machine: {} {}", request.method, request.path);
    }
}

#[test]
fn cancel_unblocks_a_sign_in_waiting_for_the_browser() {
    let server = Server::start(Scenario::default());
    let dir = tempfile::tempdir().unwrap();
    // A browser that never comes back.
    let hook = started(&server, &dir, Box::new(|_| Ok(())));
    wait_for(&hook, "guest", |s| s.view == AccountView::Guest);
    hook.sign_in();
    assert_eq!(hook.snapshot().view, AccountView::SigningIn, "shown at once");
    std::thread::sleep(Duration::from_millis(200));
    hook.cancel_sign_in();
    let snapshot = wait_for(&hook, "cancel", |s| s.view == AccountView::Guest);
    assert_eq!(snapshot.error, Some(AccountError::Cancelled));
    assert_eq!(deckcraft_ui_egui::account::error_text(AccountError::Cancelled), None, "shown as nothing");
    assert_eq!(server.requests().iter().filter(|r| r.path == "/token").count(), 0);
}

#[test]
fn cancel_before_the_worker_starts_signing_in_is_silent() {
    let server = Server::start(Scenario::default());
    let dir = tempfile::tempdir().unwrap();
    let hook = started(&server, &dir, Box::new(|_| Err("the browser must not open".into())));
    hook.sign_in();
    hook.cancel_sign_in();
    let snapshot = wait_for(&hook, "the cancelled sign-in", |s| s.view == AccountView::Guest);
    assert_eq!(snapshot.error, Some(AccountError::Cancelled));
}
