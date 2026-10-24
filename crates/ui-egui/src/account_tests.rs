//! The optional PI ID account UI against a fake hook: nothing changes without one; File › Account,
//! the title bar's button and the expiry notice call the hook and show its snapshot.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use deckcraft_engine::Session;
use egui::accesskit::Role;
use egui::{Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::account::{AccountError, AccountHook, AccountSnapshot, AccountView, LicenceView, OrgView, SignedInView, WarningView, date_text};
use crate::backstage::Page;
use crate::i18n::{self, Lang};
use crate::narrow_tests::{assert_apart, rects_in};
use crate::{Services, SlideApp};

const DESKTOP: egui::Vec2 = egui::Vec2::new(1366.0, 768.0);
const PHONE_H: f32 = 844.0;
/// 15 March 2027, noon UTC.
const EXPIRES: i64 = 1_805_112_000;
const NO_ACCOUNTS: &str = "DeckCraft has no accounts";

#[derive(Default)]
struct Fake {
    snapshot: Mutex<AccountSnapshot>,
    calls: Mutex<Vec<String>>,
    attached: AtomicUsize,
    deny: AtomicBool,
}

impl Fake {
    fn with(view: AccountView) -> Arc<Fake> {
        let fake = Arc::new(Fake::default());
        fake.set(view, None);
        fake
    }
    fn set(&self, view: AccountView, error: Option<AccountError>) {
        *self.snapshot.lock().unwrap() = AccountSnapshot { view, busy: false, error };
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

impl AccountHook for Fake {
    fn snapshot(&self) -> AccountSnapshot {
        self.snapshot.lock().unwrap().clone()
    }
    fn sign_in(&self) {
        self.record("sign_in".into());
    }
    fn cancel_sign_in(&self) {
        self.record("cancel_sign_in".into());
    }
    fn sign_out(&self) {
        self.record("sign_out".into());
    }
    fn select_org(&self, org_id: Option<String>) {
        self.record(format!("select_org:{org_id:?}"));
    }
    fn retry_activation(&self) {
        self.record("retry_activation".into());
    }
    fn allowed(&self, feature: &str) -> bool {
        self.record(format!("allowed:{feature}"));
        !self.deny.load(Ordering::SeqCst)
    }
    fn warning_shown(&self, days: u32) {
        self.record(format!("warning_shown:{days}"));
    }
    fn attach(&self, _ctx: &egui::Context) {
        self.attached.fetch_add(1, Ordering::SeqCst);
    }
}

fn signed_in(licence: LicenceView, warning: Option<WarningView>) -> AccountView {
    AccountView::SignedIn(SignedInView {
        name: "Nguyễn Văn An".into(),
        email: Some("an.nguyen@example.com".into()),
        orgs: vec![
            OrgView { id: "org_a".into(), name: "Công ty Xây dựng Ví dụ".into(), role: Some("member".into()) },
            OrgView { id: "org_b".into(), name: "Văn phòng Kiến trúc Mẫu".into(), role: None },
        ],
        active_org: None,
        licence,
        warning,
        online: true,
        session_only: false,
        reauth_required: false,
    })
}

fn harness(hook: Option<Arc<Fake>>, size: egui::Vec2, language: &str) -> Harness<'static, SlideApp> {
    let mut session = Session::new();
    deckcraft_engine::sample::open_sample(&mut session).unwrap();
    let mut app = SlideApp::new(session, Services::default());
    app.ui.language = language.into();
    app.account_hook = hook.map(|hook| hook as Arc<dyn AccountHook>);
    let mut h = Harness::builder().with_size(size).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

fn open_account_page(h: &mut Harness<'static, SlideApp>) {
    h.state_mut().run("app.backstage", json!({"page": "account"})).unwrap();
    h.run_steps(3);
}

fn page(h: &Harness<'static, SlideApp>) -> Option<Page> {
    h.state().backstage.as_ref().map(|b| b.page)
}

#[test]
fn without_a_hook_nothing_changes_and_everything_is_allowed() {
    let mut h = harness(None, DESKTOP, "en");
    assert!(h.query_by_label("Sign in").is_none(), "no title-bar button");
    assert!(h.state().allowed("trinhchieu.advanced"));
    open_account_page(&mut h);
    assert!(h.query_all_by_label_contains(NO_ACCOUNTS).next().is_some(), "the page is as before");
    assert!(h.query_by_label("User Information").is_some());
    assert!(h.query_by_label("Sign in with PI ID").is_none());
    assert!(h.query_by_label_contains("Paid features").is_none(), "no expiry notice");
}

#[test]
fn the_title_button_opens_the_guest_page_and_signs_in() {
    let fake = Fake::with(AccountView::Guest);
    let mut h = harness(Some(fake.clone()), DESKTOP, "en");
    h.get_by_label("Sign in").click();
    h.run_steps(3);
    assert_eq!(page(&h), Some(Page::Account));
    h.get_by_label("Signing in is optional: every feature works without an account.");
    assert!(h.query_all_by_label_contains(NO_ACCOUNTS).next().is_none(), "the PI ID section replaces the note");
    assert_eq!(h.get_all_by_label("User Information").count(), 1);
    h.get_by_label("Appearance");
    h.get_by_label("Product Information");
    h.get_by_label("Sign in with PI ID").click();
    h.run_steps(2);
    assert_eq!(fake.calls(), ["sign_in"]);
    assert_eq!(fake.attached.load(Ordering::SeqCst), 1, "attached once");
}

#[test]
fn signing_in_shows_the_browser_step_and_cancel_reaches_the_hook() {
    let fake = Fake::with(AccountView::SigningIn);
    let mut h = harness(Some(fake.clone()), DESKTOP, "en");
    h.get_by_label("Signing in…");
    open_account_page(&mut h);
    h.get_by_label("Continue in your browser…");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert_eq!(fake.calls(), ["cancel_sign_in"]);
}

#[test]
fn the_signed_in_page_switches_organization_and_signs_out() {
    let fake = Fake::with(signed_in(LicenceView::Free, None));
    let mut h = harness(Some(fake.clone()), DESKTOP, "vi");
    h.get_by_label("Nguyễn Văn An").click();
    h.run_steps(3);
    assert_eq!(page(&h), Some(Page::Account));
    h.get_by_label("an.nguyen@example.com");
    h.get_by_label("Miễn phí");
    h.get_by_label("Đã kết nối PI ID");
    // The organization picker is the first box on the page: the PI ID section is on top.
    let picker = h.get_all_by_role(Role::ComboBox).min_by(|a, b| a.rect().top().total_cmp(&b.rect().top())).expect("organization picker");
    picker.click();
    h.run_steps(3);
    h.get_by_label("Công ty Xây dựng Ví dụ (member)");
    h.get_by_label("Văn phòng Kiến trúc Mẫu").click();
    h.run_steps(3);
    assert_eq!(fake.calls(), ["select_org:Some(\"org_b\")"]);
    h.get_by_label("Đăng xuất").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("sign_out"));
}

#[test]
fn licence_lines_device_limit_retry_and_one_message_per_fact() {
    let fake = Fake::with(signed_in(LicenceView::Active { expires_at: EXPIRES, pro: true }, None));
    let mut h = harness(Some(fake.clone()), DESKTOP, "en");
    open_account_page(&mut h);
    h.get_by_label(&format!("Paid features are valid until {}.", date_text(EXPIRES)));
    fake.set(signed_in(LicenceView::Inactive { expires_at: EXPIRES }, None), None);
    h.run_steps(2);
    h.get_by_label(&format!("Your license expired on {}. Free features keep working.", date_text(EXPIRES)));
    fake.set(signed_in(LicenceView::DeviceCapReached, None), Some(AccountError::DeviceCap));
    h.run_steps(2);
    h.get_by_label("Your account has reached its device limit, so this device is not activated.");
    assert!(h.query_by_label_contains("Remove a device").is_none(), "the licence line already says it");
    h.get_by_label("Retry").click();
    h.run_steps(2);
    assert_eq!(fake.calls(), ["retry_activation"]);

    let mut session_only = signed_in(LicenceView::SessionOnly, None);
    if let AccountView::SignedIn(s) = &mut session_only {
        s.online = false;
        s.session_only = true;
        s.reauth_required = true;
    }
    fake.set(session_only, Some(AccountError::ReauthRequired));
    h.run_steps(2);
    h.get_by_label("Signed in for this session only: this computer has no credential store.");
    h.get_by_label("Offline. Your account syncs when you reconnect.");
    assert_eq!(h.get_all_by_label("Your session has expired. Sign in again.").count(), 1, "said once");
    assert!(h.query_by_label("You will need to sign in again the next time you open the app.").is_none());
    h.get_by_label("Sign in again").click();
    h.run_steps(2);
    assert_eq!(fake.calls().last().map(String::as_str), Some("sign_in"));
}

#[test]
fn the_lease_warning_shows_only_for_paid_features_and_closing_reports_it() {
    let warning = Some(WarningView { days: 3, expires_at: EXPIRES });
    let text = "Paid features expire on";
    for licence in [LicenceView::Free, LicenceView::Active { expires_at: EXPIRES, pro: false }, LicenceView::Online { pro: true }] {
        let h = harness(Some(Fake::with(signed_in(licence, warning))), DESKTOP, "en");
        assert!(h.query_by_label_contains(text).is_none(), "{licence:?}");
    }
    let h = harness(Some(Fake::with(signed_in(LicenceView::Active { expires_at: EXPIRES, pro: true }, None))), DESKTOP, "en");
    assert!(h.query_by_label_contains(text).is_none(), "no warning, no notice");

    let fake = Fake::with(signed_in(LicenceView::Active { expires_at: EXPIRES, pro: true }, warning));
    let mut h = harness(Some(fake.clone()), DESKTOP, "en");
    let notice = h.get_by_label_contains(&date_text(EXPIRES)).rect();
    let status = h.get_by_label("Fit slide to current window").rect();
    assert!(notice.bottom() <= status.top(), "above the status bar, clear of the chrome: {notice:?} {status:?}");
    h.state_mut().toast = Some(crate::toast::Toast { text: "Brief notice".into(), at: crate::now_ms() });
    h.run_steps(2);
    assert!(h.query_by_label_contains(text).is_none(), "a brief notice goes first");
    h.state_mut().toast = None;
    h.run_steps(2);
    h.get_by_label("Close").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains(text).is_none(), "hidden at once");
    assert_eq!(fake.calls(), ["warning_shown:3"]);

    let mut h = harness(Some(Fake::with(signed_in(LicenceView::Active { expires_at: EXPIRES, pro: true }, warning))), DESKTOP, "en");
    h.get_by_label("Account").click();
    h.run_steps(3);
    assert_eq!(page(&h), Some(Page::Account), "the notice leads to File › Account");
}

#[test]
fn errors_read_in_plain_language_and_a_cancel_is_silent() {
    let fake = Fake::with(AccountView::Guest);
    fake.set(AccountView::Guest, Some(AccountError::Network));
    let mut h = harness(Some(fake.clone()), DESKTOP, "vi");
    open_account_page(&mut h);
    h.get_by_label("Không kết nối được PI ID. Hãy kiểm tra mạng rồi thử lại.");
    fake.set(AccountView::Guest, Some(AccountError::RateLimited { retry_after_secs: Some(120) }));
    h.run_steps(2);
    h.get_by_label("PI ID đang bận. Hãy thử lại sau 2 phút.");
    fake.set(AccountView::Guest, Some(AccountError::Cancelled));
    h.run_steps(2);
    assert!(h.query_by_label_contains("thử lại").is_none(), "a cancelled sign-in is not an error");
    h.get_by_label("Đăng nhập bằng PI ID");
}

#[test]
fn allowed_asks_the_hook() {
    let fake = Fake::with(AccountView::Guest);
    fake.deny.store(true, Ordering::SeqCst);
    let h = harness(Some(fake.clone()), DESKTOP, "en");
    assert!(!h.state().allowed("trinhchieu.advanced"));
    assert_eq!(fake.calls(), ["allowed:trinhchieu.advanced"]);
}

#[test]
fn the_account_button_sits_beside_search_in_a_wide_title_bar() {
    let h = harness(Some(Fake::with(AccountView::Guest)), DESKTOP, "en");
    let button = h.get_by_label("Sign in").rect();
    let search = h.get_by_label("Search commands (⇧⌘P)").rect();
    assert!(button.left() >= search.right() && button.right() <= DESKTOP.x, "{button:?} {search:?}");
}

#[test]
fn the_account_button_never_crowds_a_narrow_title_bar() {
    for (view, label) in [(AccountView::Guest, "Sign in"), (signed_in(LicenceView::Free, None), "Nguyễn Văn An")] {
        for (width, code) in [(599.0, "vi"), (480.0, "vi"), (430.0, "en"), (390.0, "vi"), (360.0, "vi"), (320.0, "vi")] {
            let lang = Lang::from_code(code).unwrap();
            let mut h = harness(Some(Fake::with(view.clone())), vec2(width, PHONE_H), code);
            let top = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(width, crate::ribbon::NARROW_TITLE_H));
            let mut bar = rects_in(
                &h,
                lang,
                top,
                &[
                    "Home",
                    "Save (⌘S)",
                    "Undo (⌘Z)",
                    "Redo (Ctrl+Y)",
                    "Play from Start (F5)",
                    "Search commands (⇧⌘P)",
                    "Comments",
                    "Light/Dark appearance",
                    "Close document",
                    label,
                ],
            );
            let present = h.query_all_by_label_contains(i18n::tr(lang, "Present")).map(|n| n.rect()).find(|r| top.intersects(*r));
            bar.push(("Present".into(), present.expect("the Present button stays")));
            for must in ["Save (⌘S)", "Undo (⌘Z)", "Search commands (⇧⌘P)", "Close document"] {
                assert!(bar.iter().any(|(l, _)| l == i18n::tr(lang, must)), "{must} stays at {width}");
            }
            assert_apart("title bar", width, &bar);
            if width >= 599.0 {
                assert!(bar.iter().any(|(l, _)| l == i18n::tr(lang, label)), "the account button shows when there is room");
            }
            // File › Account stays reachable when the button gives way.
            open_account_page(&mut h);
            assert_eq!(page(&h), Some(Page::Account));
        }
    }
}

/// Renders of the account UI into `DECKCRAFT_SHOTS` (run with `--ignored`; never asserted).
#[test]
#[ignore]
fn account_screenshots() {
    let Some(dir) = std::env::var_os("DECKCRAFT_SHOTS").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let save = |h: &mut Harness<'static, SlideApp>, name: &str| h.render().unwrap().save(dir.join(name)).unwrap();

    let mut h = harness(Some(Fake::with(AccountView::Guest)), DESKTOP, "en");
    save(&mut h, "title-button-guest-en.png");
    open_account_page(&mut h);
    save(&mut h, "account-guest-en.png");

    let mut h = harness(Some(Fake::with(signed_in(LicenceView::Free, None))), DESKTOP, "vi");
    save(&mut h, "title-button-signed-in-vi.png");
    open_account_page(&mut h);
    save(&mut h, "account-signed-in-vi.png");

    let fake = Fake::with(AccountView::Guest);
    fake.set(AccountView::Guest, Some(AccountError::Network));
    let mut h = harness(Some(fake), DESKTOP, "vi");
    open_account_page(&mut h);
    save(&mut h, "account-error-vi.png");

    let warning = Some(WarningView { days: 3, expires_at: EXPIRES });
    let mut h = harness(Some(Fake::with(signed_in(LicenceView::Active { expires_at: EXPIRES, pro: true }, warning))), DESKTOP, "en");
    save(&mut h, "lease-warning-en.png");

    for width in [599.0, 390.0] {
        let mut h = harness(Some(Fake::with(AccountView::Guest)), vec2(width, PHONE_H), "vi");
        save(&mut h, &format!("title-bar-{width:.0}-vi.png"));
    }
    let mut h = harness(None, DESKTOP, "en");
    open_account_page(&mut h);
    save(&mut h, "account-no-hook-en.png");
}
