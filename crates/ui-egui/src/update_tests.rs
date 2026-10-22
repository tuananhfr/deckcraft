#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Native update notices use a fake service and never contact a server or run installers.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::{
    SlideApp,
    updates::{CheckOutcome, Offer, PreviousResult, RELEASES_PAGE, UpdateHook},
};

use egui_kittest::{Harness, kittest::Queryable};

struct Fake {
    page_only: bool,
    msi: bool,
    auto: AtomicBool,
    calls: AtomicUsize,
    forced: AtomicBool,
    skipped: Mutex<Option<String>>,
    downloads: AtomicUsize,
    installed: AtomicBool,
    installs: AtomicUsize,
    fail_download: AtomicBool,
    wait_download: AtomicBool,
    check_error: AtomicBool,
    up_to_date: AtomicBool,
    previous: Mutex<Option<PreviousResult>>,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            page_only: false,
            msi: false,
            auto: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
            forced: AtomicBool::new(false),
            skipped: Mutex::new(None),
            downloads: AtomicUsize::new(0),
            installed: AtomicBool::new(false),
            installs: AtomicUsize::new(0),
            fail_download: AtomicBool::new(false),
            wait_download: AtomicBool::new(false),
            check_error: AtomicBool::new(false),
            up_to_date: AtomicBool::new(false),
            previous: Mutex::new(None),
        }
    }
}

impl UpdateHook for Fake {
    fn check(&self, forced: bool) -> Result<CheckOutcome, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.forced.store(forced, Ordering::SeqCst);
        if self.check_error.load(Ordering::SeqCst) {
            return Err("test check failure".into());
        }
        if self.up_to_date.load(Ordering::SeqCst) {
            return Ok(CheckOutcome::UpToDate);
        }
        Ok(CheckOutcome::Available(Offer {
            version: "99.0.0".into(),
            notes_url: format!("{RELEASES_PAGE}/tag/erpcons-v99.0.0"),
            download_page_only: self.page_only,
            windows_msi: self.msi,
        }))
    }
    fn download(&self, progress: &dyn Fn(u64, u64), cancel: &AtomicBool) -> Result<(), String> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        progress(5, 10);
        while self.wait_download.load(Ordering::SeqCst) && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if self.fail_download.load(Ordering::SeqCst) {
            return Err("test download failure".into());
        }
        if cancel.load(Ordering::Relaxed) { Err("cancelled".into()) } else { Ok(()) }
    }
    fn install_and_restart(&self) -> Result<(), String> {
        self.installs.fetch_add(1, Ordering::SeqCst);
        self.installed.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn skip(&self, version: &str) -> Result<(), String> {
        *self.skipped.lock().unwrap() = Some(version.into());
        Ok(())
    }
    fn auto_check(&self) -> bool {
        self.auto.load(Ordering::SeqCst)
    }
    fn set_auto_check(&self, on: bool) -> Result<(), String> {
        self.auto.store(on, Ordering::SeqCst);
        Ok(())
    }
    fn take_previous_result(&self) -> Option<PreviousResult> {
        self.previous.lock().unwrap().take()
    }
}

fn harness(fake: Arc<Fake>) -> Harness<'static, SlideApp> {
    let mut app = SlideApp::new(deckcraft_engine::Session::new(), crate::Services::default());
    app.product_name = "Trình chiếu – ERPCons";
    app.ui.language = "en".into();
    app.update_hook = Some(fake);
    Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    )
}

fn wait_for(h: &mut Harness<'static, SlideApp>, label: &str) {
    for _ in 0..200 {
        h.run_steps(2);
        if h.query_by_label_contains(label).is_some() {
            h.run_steps(3);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.get_by_label_contains(label);
}

#[test]
fn available_at_start_later_hides_and_forced_check_restores_notice() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Trình chiếu – ERPCons 99.0.0 is available");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert!(!fake.forced.load(Ordering::SeqCst));
    h.get_by_label("Remind me later").click();
    h.run_steps(3);
    assert!(h.query_by_label_contains("is available").is_none());
    h.state_mut().check_for_updates();
    wait_for(&mut h, "Trình chiếu – ERPCons 99.0.0 is available");
    assert!(fake.forced.load(Ordering::SeqCst));
}

#[test]
fn skip_passes_version_and_hides_notice() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Skip this version");
    h.get_by_label("Skip this version").click();
    h.run_steps(3);
    assert_eq!(fake.skipped.lock().unwrap().as_deref(), Some("99.0.0"));
    assert!(h.query_by_label_contains("is available").is_none());
}

#[test]
fn download_page_only_opens_notes_without_downloading() {
    let fake = Arc::new(Fake { page_only: true, ..Fake::default() });
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Download new version");
    assert!(h.query_by_label("Update now").is_none());
    h.get_by_label("Download new version").click();
    let mut opened = false;
    for _ in 0..5 {
        h.step();
        opened |= h
            .output()
            .platform_output
            .commands
            .iter()
            .any(|command| matches!(command, egui::OutputCommand::OpenUrl(url) if url.url == format!("{RELEASES_PAGE}/tag/erpcons-v99.0.0")));
    }
    assert!(opened);
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 0);
}

#[test]
fn download_failure_offers_retry_and_success_launches_installer() {
    let fake = Arc::new(Fake::default());
    fake.fail_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "test download failure");
    h.get_by_label("Retry");
    assert!(!fake.installed.load(Ordering::SeqCst));
    fake.fail_download.store(false, Ordering::SeqCst);
    h.get_by_label("Retry").click();
    for _ in 0..400 {
        h.run_steps(2);
        if fake.installed.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(fake.installed.load(Ordering::SeqCst));
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);
}

#[test]
fn previous_results_are_shown_and_failure_retry_forces_check() {
    for (previous, label, failed) in [
        (PreviousResult::Installed { version: "1.2.3".into() }, "Updated to version 1.2.3", false),
        (PreviousResult::Failed { version: "1.2.3".into(), reason: "test reason".into() }, "Update to 1.2.3 failed: test reason", true),
    ] {
        let fake = Arc::new(Fake::default());
        fake.auto.store(false, Ordering::SeqCst);
        *fake.previous.lock().unwrap() = Some(previous);
        let mut h = harness(fake.clone());
        wait_for(&mut h, label);
        assert!(fake.previous.lock().unwrap().is_none());
        assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
        if failed {
            h.get_by_label("Retry").click();
            wait_for(&mut h, "is available");
            assert!(fake.forced.load(Ordering::SeqCst));
        }
    }
}

#[test]
fn no_hook_opens_releases_page() {
    let mut app = SlideApp::new(deckcraft_engine::Session::new(), crate::Services::default());
    app.check_for_updates();
    assert_eq!(app.pending_urls.last().map(String::as_str), Some(RELEASES_PAGE));
}

fn dirty(app: &mut SlideApp) {
    app.run("slide.new", serde_json::json!({})).unwrap();
}

fn document(app: &mut SlideApp) {
    app.run("file.new", serde_json::json!({})).unwrap();
}

#[test]
fn unsaved_document_cancel_aborts_update_without_discarding_work() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    document(h.state_mut());
    dirty(h.state_mut());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    h.run_steps(3);
    h.get_by_label_contains("Do you want to save");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().close_batch.is_none());
    assert!(h.state().has_unsaved_changes());
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 0);
    assert!(!fake.installed.load(Ordering::SeqCst));
}

#[test]
fn forced_checks_report_up_to_date_and_errors_when_auto_is_off() {
    let fake = Arc::new(Fake::default());
    fake.auto.store(false, Ordering::SeqCst);
    fake.up_to_date.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    h.run_steps(3);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    h.state_mut().check_for_updates();
    wait_for(&mut h, "Trình chiếu – ERPCons is up to date.");
    fake.check_error.store(true, Ordering::SeqCst);
    h.state_mut().check_for_updates();
    wait_for(&mut h, "test check failure");
    h.get_by_label("Retry");
}

#[test]
fn cancel_download_keeps_app_open_and_never_installs() {
    let fake = Arc::new(Fake::default());
    fake.wait_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Cancel");
    h.get_by_label("Cancel").click();
    wait_for(&mut h, "Update now");
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 1);
    assert!(!fake.installed.load(Ordering::SeqCst));
}

#[test]
fn auto_check_checkbox_updates_the_service_preference() {
    let fake = Arc::new(Fake::default());
    fake.auto.store(false, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    h.state_mut().run("app.preferences", serde_json::json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("Automatically check for updates").click();
    h.run_steps(3);
    assert!(fake.auto.load(Ordering::SeqCst));
}

#[test]
fn update_notice_allows_document_shortcuts() {
    let mut h = harness(Arc::new(Fake::default()));
    document(h.state_mut());
    dirty(h.state_mut());
    wait_for(&mut h, "Update now");
    let before = h.state().session.active().unwrap().doc.slides.len();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.slides.len(), before - 1);
}

#[test]
fn update_notice_screenshots_in_both_themes() {
    let Some(dir) = std::env::var_os("DECKCRAFT_UPDATE_SCREENSHOTS") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for theme in ["light", "dark"] {
        let mut h = harness(Arc::new(Fake { msi: true, ..Fake::default() }));
        h.state_mut().ui.language = "vi".into();
        h.state_mut().run("view.dark", serde_json::json!({"on": theme == "dark"})).unwrap();
        wait_for(&mut h, "Đã có Trình chiếu – ERPCons");
        h.render().unwrap().save(dir.join(format!("update-{theme}.png"))).unwrap();
        h.get_by_label("Cập nhật ngay").click();
        wait_for(&mut h, "Windows sẽ hỏi quyền cài đặt");
        h.render().unwrap().save(dir.join(format!("update-msi-ready-{theme}.png"))).unwrap();
    }
}

#[test]
fn edits_made_during_download_are_guarded_before_install() {
    let fake = Arc::new(Fake::default());
    fake.wait_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    document(h.state_mut());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Cancel");
    dirty(h.state_mut());
    fake.wait_download.store(false, Ordering::SeqCst);
    wait_for(&mut h, "Do you want to save");
    assert!(!fake.installed.load(Ordering::SeqCst));
    h.get_by_label("Cancel").click();
    h.run_steps(5);
    assert!(h.state().has_unsaved_changes());
    assert!(h.state().close_batch.is_none());
    assert!(!fake.installed.load(Ordering::SeqCst));
}

#[test]
fn discarding_unsaved_changes_allows_download() {
    let fake = Arc::new(Fake::default());
    fake.fail_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    document(h.state_mut());
    dirty(h.state_mut());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Do you want to save");
    h.get_by_label("Don't Save").click();
    wait_for(&mut h, "test download failure");
    assert!(h.state().has_unsaved_changes(), "failed updates keep the presentation available");
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 1);
}

#[test]
fn windows_msi_ready_notice_explains_the_permission_prompt() {
    let fake = Arc::new(Fake { msi: true, ..Fake::default() });
    let mut h = harness(fake);
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Windows will ask for installation permission");
}

#[test]
fn cancelling_save_picker_aborts_update() {
    let fake = Arc::new(Fake::default());
    let mut h = harness(fake.clone());
    h.state_mut().services.pick_save = Some(Box::new(|_| None));
    document(h.state_mut());
    dirty(h.state_mut());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Do you want to save");
    h.get_by_label("Save").click();
    h.run_steps(5);
    assert!(h.state().has_unsaved_changes());
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 0);
    assert!(!fake.installed.load(Ordering::SeqCst));
}

#[test]
fn saving_before_update_keeps_the_presentation_open_during_download() {
    let dir = std::env::temp_dir().join(format!("deckcraft-update-save-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("saved.deckcraft").to_string_lossy().into_owned();
    let fake = Arc::new(Fake::default());
    fake.wait_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    h.state_mut().services.pick_save = Some(Box::new(move |_| Some(path.clone())));
    document(h.state_mut());
    dirty(h.state_mut());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Do you want to save");
    h.get_by_label("Save").click();
    for _ in 0..200 {
        h.run_steps(2);
        if fake.downloads.load(Ordering::SeqCst) > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 1);
    assert_eq!(h.state().session.documents().len(), 1);
    assert!(!h.state().has_unsaved_changes());
    h.get_by_label("Cancel").click();
    wait_for(&mut h, "Update now");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn msi_waits_for_install_click_and_installs_once() {
    let fake = Arc::new(Fake { msi: true, ..Fake::default() });
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Install now");
    std::thread::sleep(std::time::Duration::from_millis(850));
    h.run_steps(5);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
    h.get_by_label("Install now").click();
    h.run_steps(3);
    h.state_mut().request_update(&egui::Context::default());
    h.run_steps(3);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 1);
    assert!(h.state().quit_confirmed);
}

#[test]
fn msi_cancel_allows_forced_check_and_download_again() {
    let fake = Arc::new(Fake { msi: true, ..Fake::default() });
    let mut h = harness(fake.clone());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Install now");
    h.get_by_label("Cancel").click();
    h.run_steps(5);
    assert!(h.query_by_label("Install now").is_none());
    h.state_mut().check_for_updates();
    wait_for(&mut h, "Update now");
    assert!(fake.forced.load(Ordering::SeqCst));
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Install now");
    assert_eq!(fake.downloads.load(Ordering::SeqCst), 2);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
    drop(h);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
}

#[test]
fn msi_install_click_guards_edits_made_during_download() {
    let fake = Arc::new(Fake { msi: true, ..Fake::default() });
    fake.wait_download.store(true, Ordering::SeqCst);
    let mut h = harness(fake.clone());
    document(h.state_mut());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Cancel");
    dirty(h.state_mut());
    fake.wait_download.store(false, Ordering::SeqCst);
    wait_for(&mut h, "Install now");
    h.get_by_label("Install now").click();
    wait_for(&mut h, "Do you want to save");
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
    h.get_by_label("Cancel").click();
    h.run_steps(5);
    assert!(h.state().has_unsaved_changes());
    assert!(h.state().close_batch.is_none());
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
    h.get_by_label("Install now").click();
    wait_for(&mut h, "Do you want to save");
    h.get_by_label("Don't Save").click();
    h.run_steps(5);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 1);
}

#[test]
fn quitting_while_msi_waits_never_installs() {
    let fake = Arc::new(Fake { msi: true, ..Fake::default() });
    let mut h = harness(fake.clone());
    document(h.state_mut());
    wait_for(&mut h, "Update now");
    h.get_by_label("Update now").click();
    wait_for(&mut h, "Install now");
    dirty(h.state_mut());
    let all = h.state().session.documents().iter().map(|doc| doc.uid).collect();
    h.state_mut().close_documents(all, crate::close_many::After::Quit);
    wait_for(&mut h, "Do you want to save");
    h.get_by_label("Don't Save").click();
    h.run_steps(5);
    assert!(h.state().quit_confirmed);
    assert_eq!(fake.installs.load(Ordering::SeqCst), 0);
}
