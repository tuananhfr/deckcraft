//! The licence lock against a fake hook: the view mode's refusals with the licence's reason,
//! Save, Save As, Export and Print kept, editing given back without reopening, and a hook that
//! does not gate (the web's) never locking.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use deckcraft_engine::{Access, Session};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::EditLock;
use crate::account::{AccountHook, AccountSnapshot, AccountView, LicenceView, SignedInView};
use crate::backstage::{Nav, Page, offered_entries};
use crate::i18n::{self, Lang};
use crate::ribbon::enabled;
use crate::{Services, SlideApp};

#[derive(Default)]
struct Gate {
    view: Mutex<AccountView>,
    lock: Mutex<Option<EditLock>>,
    calls: Mutex<Vec<&'static str>>,
    refused: AtomicUsize,
}

impl Gate {
    fn set(&self, lock: Option<EditLock>) {
        *self.lock.lock().unwrap() = lock;
    }
}

impl AccountHook for Gate {
    fn snapshot(&self) -> AccountSnapshot {
        AccountSnapshot { view: self.view.lock().unwrap().clone(), busy: false, error: None }
    }
    fn sign_in(&self) {
        self.calls.lock().unwrap().push("sign_in");
    }
    fn cancel_sign_in(&self) {}
    fn sign_out(&self) {}
    fn select_org(&self, _org_id: Option<String>) {}
    fn retry_activation(&self) {
        self.calls.lock().unwrap().push("retry_activation");
    }
    fn allowed(&self, _feature: &str) -> bool {
        true
    }
    fn warning_shown(&self, _days: u32) {}
    fn attach(&self, _ctx: &egui::Context) {}
    fn edit_lock(&self) -> Option<EditLock> {
        *self.lock.lock().unwrap()
    }
    fn edit_refused(&self) {
        self.refused.fetch_add(1, Ordering::SeqCst);
    }
}

/// The web's hook: it has no say on editing.
struct Ungated;

impl AccountHook for Ungated {
    fn snapshot(&self) -> AccountSnapshot {
        AccountSnapshot::default()
    }
    fn sign_in(&self) {}
    fn cancel_sign_in(&self) {}
    fn sign_out(&self) {}
    fn select_org(&self, _org_id: Option<String>) {}
    fn retry_activation(&self) {}
    fn allowed(&self, feature: &str) -> bool {
        !crate::account::is_pro_feature(feature)
    }
    fn warning_shown(&self, _days: u32) {}
    fn attach(&self, _ctx: &egui::Context) {}
}

fn harness(hook: Arc<dyn AccountHook>, language: &str) -> Harness<'static, SlideApp> {
    let mut session = Session::new();
    deckcraft_engine::sample::open_sample(&mut session).unwrap();
    let mut app = SlideApp::new(session, Services::default());
    app.ui.language = language.into();
    app.account_hook = Some(hook);
    let mut h = Harness::builder().with_size(egui::vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

fn access(h: &Harness<'static, SlideApp>) -> Access {
    h.state().session.active().unwrap().access
}

fn status(h: &Harness<'static, SlideApp>) -> Option<String> {
    h.state().status.as_ref().map(|s| s.0.clone())
}

#[test]
fn a_lock_refuses_edits_with_its_reason_and_keeps_save_export_and_print() {
    let gate = Arc::new(Gate::default());
    let mut h = harness(gate.clone(), "en");
    assert_eq!(access(&h), Access::default());
    // Changed before the lock: the change must stay and still be saved.
    h.state_mut().run("slide.new", json!({})).unwrap();
    let uid = h.state().session.active().unwrap().uid;
    let undo_steps = h.state().session.active().unwrap().history.undo.len();

    gate.set(Some(EditLock::SignIn));
    h.run_steps(2);
    assert_eq!(access(&h), Access::edits_locked());
    assert_eq!(status(&h).as_deref(), Some("Sign in with PI ID to edit"), "said once when it starts");
    let app = h.state_mut();
    app.status = None;
    assert!(app.run("slide.new", json!({})).is_err());
    assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some("Sign in with PI ID to edit"), "the reason, not the view mode's notice");
    assert_eq!(gate.refused.load(Ordering::SeqCst), 1, "the hook hears of the refused edit");
    for id in ["slide.new", "edit.paste", "edit.undo", "format.bold", "insert.table", "app.insertPictureDialog"] {
        assert!(!enabled(app, id), "{id} is offered while locked");
    }
    for id in ["edit.copy", "app.saveAsDialog", "app.exportDialog", "app.print", "show.fromStart"] {
        assert!(enabled(app, id), "{id} is not offered while locked");
    }
    let (top, _) = offered_entries(app, true);
    for nav in [Nav::Save, Nav::Page(Page::SaveAs), Nav::Page(Page::Print), Nav::Page(Page::Export)] {
        assert!(top.contains(&nav), "{nav:?} is not offered while locked");
    }
    assert!(app.can_save());

    let dir = std::env::temp_dir().join(format!("deckcraft-licence-lock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("locked.pptx");
    app.session.active_mut().unwrap().path = Some(path.to_string_lossy().into_owned());
    app.save();
    let saved = path.is_file();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(saved, "Save writes the file: {:?}", app.status);
    let doc = app.session.active().unwrap();
    assert!(!doc.is_dirty() && doc.uid == uid && doc.history.undo.len() == undo_steps, "saved as it was, history kept");

    app.run("file.new", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().access, Access::edits_locked(), "a new presentation starts locked");
}

#[test]
fn a_new_lease_gives_editing_back_without_reopening() {
    let gate = Arc::new(Gate::default());
    gate.set(Some(EditLock::RenewOffline));
    let mut h = harness(gate.clone(), "en");
    let uid = h.state().session.active().unwrap().uid;
    assert_eq!(access(&h), Access::edits_locked());
    assert_eq!(status(&h).as_deref(), Some("Your license has expired. Connect to the internet to renew it"));

    gate.set(None);
    h.run_steps(2);
    assert_eq!(access(&h), Access::default());
    assert_eq!(h.state().session.active().unwrap().uid, uid, "the same presentation");
    assert_eq!(status(&h), None, "a reason that no longer holds is taken back");
    h.state_mut().run("slide.new", json!({})).unwrap();

    h.state_mut().run("file.new", json!({})).unwrap();
    assert_eq!(access(&h), Access::default());
}

#[test]
fn checking_locks_quietly_and_the_hosts_view_modes_are_left_alone() {
    let gate = Arc::new(Gate::default());
    gate.set(Some(EditLock::Checking));
    let mut h = harness(gate.clone(), "en");
    assert_eq!(access(&h), Access::edits_locked());
    assert_eq!(status(&h), None, "nothing to say until the account is read");

    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().set_access(uid, Access::read_only(false));
    gate.set(None);
    h.run_steps(2);
    assert_eq!(access(&h), Access::read_only(false), "lifting the lock never widens a host's mode");
    gate.set(Some(EditLock::SignIn));
    h.run_steps(2);
    assert_eq!(access(&h), Access::read_only(false));
    assert!(!h.state().can_save(), "the view mode still refuses Save");
}

#[test]
fn a_hook_that_does_not_gate_never_locks() {
    let mut h = harness(Arc::new(Ungated), "en");
    assert!(crate::account::is_pro_feature("erpconsoffice.use"), "even with editing a pro feature");
    h.run_steps(2);
    assert_eq!(access(&h), Access::default());
    h.state_mut().run("slide.new", json!({})).unwrap();
    h.state_mut().run("file.new", json!({})).unwrap();
    assert_eq!(access(&h), Access::default());
}

fn signed_in() -> AccountView {
    AccountView::SignedIn(SignedInView {
        name: "Nguyễn Văn An".into(),
        email: None,
        orgs: Vec::new(),
        active_org: Some("org".into()),
        licence: LicenceView::Free,
        warning: None,
        online: true,
        session_only: false,
        reauth_required: false,
    })
}

#[test]
fn the_account_page_says_why_and_offers_sign_in_or_retry() {
    let gate = Arc::new(Gate::default());
    gate.set(Some(EditLock::SignIn));
    let mut h = harness(gate.clone(), "vi");
    h.state_mut().run("app.backstage", json!({"page": "account"})).unwrap();
    h.run_steps(3);
    h.get_by_label("Đăng nhập PI ID để sửa");
    assert!(h.query_by_label_contains("Đăng nhập là tùy chọn").is_none(), "signing in is no longer optional");
    h.get_by_label("Đăng nhập bằng PI ID").click();
    h.run_steps(2);

    *gate.view.lock().unwrap() = signed_in();
    gate.set(Some(EditLock::NoPlan));
    h.run_steps(3);
    h.get_by_label("Tổ chức này chưa có gói ERPCons Office");
    h.get_by_label("Thử lại").click();
    h.run_steps(2);
    gate.set(Some(EditLock::Revoked));
    h.run_steps(3);
    assert!(h.query_by_label("Thử lại").is_none(), "a sync can't bring back a removed device");
    assert_eq!(*gate.calls.lock().unwrap(), ["sign_in", "retry_activation"]);
}

#[test]
fn every_reason_reads_in_vietnamese() {
    use EditLock::*;
    let all = [Checking, SignIn, ChooseOrg, Revoked, DeviceCap, NotActivated, SessionOnly, RenewOffline, RenewFailed, NoPlan, Unavailable];
    let vi = Lang::from_code("vi").unwrap();
    i18n::set_current(Lang::EN);
    for lock in all {
        assert_ne!(i18n::tr(vi, lock.text()), lock.text(), "untranslated: {}", lock.text());
    }
    assert_eq!(i18n::tr(vi, SignIn.text()), "Đăng nhập PI ID để sửa");
    assert_eq!(i18n::tr(vi, ChooseOrg.text()), "Chọn tổ chức để sửa");
    let retried: Vec<EditLock> = all.into_iter().filter(|l| l.retry_offered()).collect();
    assert_eq!(retried, [ChooseOrg, DeviceCap, NotActivated, RenewOffline, RenewFailed, NoPlan]);
    assert!(!DeviceCap.sync_may_lift(), "only the user's Retry lifts a device cap");
}

/// Renders of the licence lock into `DECKCRAFT_SHOTS` (run with `--ignored`; never asserted).
#[test]
#[ignore]
fn lock_screenshots() {
    let Some(dir) = std::env::var_os("DECKCRAFT_SHOTS").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let save = |h: &mut Harness<'static, SlideApp>, name: &str| h.render().unwrap().save(dir.join(name)).unwrap();
    let gate = Arc::new(Gate::default());
    gate.set(Some(EditLock::SignIn));
    let mut h = harness(gate.clone(), "vi");
    save(&mut h, "licence-lock-guest-vi.png");
    h.state_mut().run("app.backstage", json!({"page": "account"})).unwrap();
    h.run_steps(3);
    save(&mut h, "licence-lock-account-guest-vi.png");
    *gate.view.lock().unwrap() = signed_in();
    gate.set(Some(EditLock::NoPlan));
    h.run_steps(3);
    save(&mut h, "licence-lock-account-no-plan-vi.png");
}
