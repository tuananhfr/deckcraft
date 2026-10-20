#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use deckcraft_engine::Session;
use deckcraft_model::{Shape, ShapeKind};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::{SaveDone, SaveOutcome, Services, SlideApp};

fn lossy(services: Services) -> SlideApp {
    let mut app = SlideApp::new(Session::with_new(), services);
    let d = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(&mut d.doc);
    doc.import_losses.signatures = true;
    doc.import_losses.macros = true;
    doc.import_losses.parts = vec!["customData".into()];
    Arc::make_mut(doc.slides.first_mut().unwrap()).shapes.push(Shape {
        kind: ShapeKind::Opaque {
            xml: r#"<p:graphicFrame xmlns:p="urn:synthetic" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:oleObj r:id="object"/></p:graphicFrame>"#.into(),
            preview: None,
            label: "Synthetic OLE".into(),
        },
        ..Default::default()
    });
    app
}

fn harness(app: SlideApp, width: f32) -> Harness<'static, SlideApp> {
    let mut h = Harness::builder().with_size(egui::vec2(width, 800.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

type Calls = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

fn downloads() -> (Services, Calls) {
    let calls: Calls = Default::default();
    let sink = calls.clone();
    (Services { download: Some(Box::new(move |name, bytes| sink.borrow_mut().push((name.into(), bytes.into())))), ..Default::default() }, calls)
}

struct TestDir(std::path::PathBuf);

impl TestDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("deckcraft-loss-warning-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn save_with_losses_shows_one_dialog_and_cancel_writes_nothing() {
    let dir = TestDir::new("cancel");
    let source = dir.file("source.pptx");
    std::fs::write(&source, b"original bytes").unwrap();
    let mut app = lossy(Services::default());
    app.session.active_mut().unwrap().path = Some(source.clone());
    let saved = app.session.active().unwrap().saved_doc.clone();
    let mut h = harness(app, 1200.0);
    h.state_mut().request_save();
    h.run_steps(4);
    assert_eq!(h.state().dialog.as_ref().unwrap().id, super::DIALOG);
    h.get_by_label("Saving will lose some content");
    h.get_by_label("Save Anyway");
    h.get_by_label("Save a Copy");
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert_eq!(std::fs::read(&source).unwrap(), b"original bytes");
    assert!(Arc::ptr_eq(&saved, &h.state().session.active().unwrap().saved_doc));
    assert!(h.state().session.active().unwrap().is_dirty());
}

#[test]
fn save_anyway_starts_the_browser_save_once_after_confirmation() {
    let calls: Calls = Default::default();
    let sink = calls.clone();
    let services = Services {
        save_file: Some(Box::new(move |name, bytes, done: SaveDone| {
            sink.borrow_mut().push((name.into(), bytes.into()));
            done(SaveOutcome::Saved);
        })),
        download: Some(Box::new(|_, _| {})),
        ..Default::default()
    };
    let mut h = harness(lossy(services), 1200.0);
    h.state_mut().request_save();
    h.run_steps(3);
    assert!(calls.borrow().is_empty());
    h.get_by_label("Save Anyway").click();
    h.run_steps(5);
    assert_eq!(calls.borrow().len(), 1);
    assert!(calls.borrow().first().unwrap().0.ends_with(".pptx"));
    assert!(h.state().dialog.is_none());
    assert!(!h.state().session.active().unwrap().is_dirty());
    h.run_steps(5);
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn desktop_save_anyway_writes_only_after_the_click() {
    let dir = TestDir::new("anyway");
    let path = dir.file("source.pptx");
    std::fs::write(&path, b"original bytes").unwrap();
    let mut app = lossy(Services::default());
    app.session.active_mut().unwrap().path = Some(path.clone());
    let mut h = harness(app, 1200.0);
    h.state_mut().request_save();
    h.run_steps(4);
    assert_eq!(std::fs::read(&path).unwrap(), b"original bytes");
    h.get_by_label("Save Anyway").click();
    h.run_steps(4);
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.starts_with(b"PK"));
    assert!(!h.state().session.active().unwrap().is_dirty());
    h.run_steps(4);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn save_a_copy_keeps_the_name_path_saved_snapshot_and_dirty_state() {
    let dir = TestDir::new("copy");
    let source = dir.file("source.pptx");
    let copy = dir.file("copy.pptx");
    std::fs::write(&source, b"original bytes").unwrap();
    let destination = copy.clone();
    let services = Services { pick_save: Some(Box::new(move |_| Some(destination.clone()))), ..Default::default() };
    let mut app = lossy(services);
    app.session.active_mut().unwrap().path = Some(source.clone());
    let before = app.session.active().unwrap();
    let (saved, title, revision) = (before.saved_doc.clone(), before.title(), before.revision);
    let mut h = harness(app, 1200.0);
    h.state_mut().request_save();
    h.run_steps(4);
    h.get_by_label("Save a Copy").click();
    h.run_steps(4);
    let after = h.state().session.active().unwrap();
    assert_eq!(after.path.as_deref(), Some(source.as_str()));
    assert_eq!(after.title(), title);
    assert_eq!(after.revision, revision);
    assert!(Arc::ptr_eq(&saved, &after.saved_doc));
    assert!(after.is_dirty());
    assert_eq!(std::fs::read(&source).unwrap(), b"original bytes");
    assert!(std::fs::read(copy).unwrap().starts_with(b"PK"));
}

#[test]
fn a_copy_cannot_overwrite_the_source() {
    let dir = TestDir::new("same-path");
    let source = dir.file("source.pptx");
    std::fs::write(&source, b"original bytes").unwrap();
    let destination = source.clone();
    let services = Services { pick_save: Some(Box::new(move |_| Some(destination.clone()))), ..Default::default() };
    let mut app = lossy(services);
    app.session.active_mut().unwrap().path = Some(source.clone());
    let mut h = harness(app, 1200.0);
    h.state_mut().request_save();
    h.run_steps(4);
    h.get_by_label("Save a Copy").click();
    h.run_steps(3);
    assert_eq!(std::fs::read(source).unwrap(), b"original bytes");
    assert!(h.state().session.active().unwrap().is_dirty());
}

#[test]
fn browser_copy_success_keeps_the_open_document_unsaved() {
    let calls: Calls = Default::default();
    let sink = calls.clone();
    let services = Services {
        save_file: Some(Box::new(move |name, bytes, done: SaveDone| {
            sink.borrow_mut().push((name.into(), bytes.into()));
            done(SaveOutcome::Saved);
        })),
        download: Some(Box::new(|_, _| {})),
        ..Default::default()
    };
    let app = lossy(services);
    let before = app.session.active().unwrap();
    let (saved, title) = (before.saved_doc.clone(), before.title());
    let mut h = harness(app, 1200.0);
    h.state_mut().request_save();
    h.run_steps(3);
    h.get_by_label("Save a Copy").click();
    h.run_steps(4);
    assert_eq!(calls.borrow().len(), 1);
    let after = h.state().session.active().unwrap();
    assert!(after.is_dirty());
    assert_eq!(after.title(), title);
    assert!(after.path.is_none());
    assert!(Arc::ptr_eq(&saved, &after.saved_doc));
}

#[test]
fn export_and_save_as_warn_before_the_browser_download() {
    for export in [false, true] {
        let (services, calls) = downloads();
        let mut h = harness(lossy(services), 390.0);
        if export {
            assert!(!h.state_mut().export_as("pptx", json!({})));
        } else {
            h.state_mut().save_as_dialog("pptx");
        }
        h.run_steps(3);
        assert!(calls.borrow().is_empty());
        h.get_by_label("Save Anyway").click();
        h.run_steps(4);
        assert_eq!(calls.borrow().len(), 1);
        assert!(h.state().dialog.is_none());
    }
}

#[test]
fn saving_without_losses_needs_no_dialog() {
    let (services, calls) = downloads();
    let mut h = harness(SlideApp::new(Session::with_new(), services), 1200.0);
    h.state_mut().request_save();
    h.run_steps(4);
    assert!(h.state().dialog.is_none());
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn direct_ui_save_commands_cannot_bypass_the_warning() {
    let dir = TestDir::new("command");
    for command in ["file.save", "file.saveAs", "file.export"] {
        let path = dir.file(&format!("{command}.pptx"));
        let mut h = harness(lossy(Services::default()), 1200.0);
        let result = h.state_mut().run(command, json!({"path": path, "format": "pptx"})).unwrap();
        assert_eq!(result["needsConfirmation"], true);
        h.run_steps(3);
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert!(!std::path::Path::new(&path).exists());
    }
}

#[test]
fn vietnamese_loss_dialog_strings_are_translated() {
    let vi = crate::i18n::Lang::from_code("vi").unwrap();
    for (source, expected) in [
        ("Saving will lose some content", "Lưu sẽ mất một số nội dung"),
        ("Save Anyway", "Vẫn lưu"),
        ("Save a Copy", "Lưu bản sao"),
        ("Cancel", "Hủy"),
    ] {
        assert_eq!(crate::i18n::tr(vi, source), expected);
    }
}

#[test]
fn closing_one_or_all_resumes_after_the_confirmed_save() {
    for all in [false, true] {
        let (services, calls) = downloads();
        let mut h = harness(lossy(services), 1200.0);
        if all {
            h.state_mut().close_all();
        } else {
            h.state_mut().close_presentation();
        }
        h.run_steps(3);
        h.get_by_label("Save").click();
        h.run_steps(3);
        assert_eq!(h.state().dialog.as_ref().unwrap().id, super::DIALOG);
        assert!(calls.borrow().is_empty());
        h.get_by_label("Save Anyway").click();
        h.run_steps(5);
        assert_eq!(calls.borrow().len(), 1);
        assert!(h.state().session.documents().is_empty());
    }
}

#[test]
fn cancelling_a_close_batch_loss_warning_keeps_the_document_open() {
    let (services, calls) = downloads();
    let mut h = harness(lossy(services), 1200.0);
    h.state_mut().close_all();
    h.run_steps(3);
    h.get_by_label("Save").click();
    h.run_steps(3);
    h.get_by_label("Cancel").click();
    h.run_steps(4);
    assert_eq!(h.state().session.documents().len(), 1);
    assert!(h.state().session.active().unwrap().is_dirty());
    assert!(h.state().close_batch.is_none());
    assert!(h.state().dialog.is_none());
    assert!(calls.borrow().is_empty());
}

#[test]
fn escape_cancels_without_a_download() {
    let (services, calls) = downloads();
    let mut h = harness(lossy(services), 1200.0);
    h.state_mut().request_save();
    h.run_steps(4);
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert!(h.state().dialog.is_none());
    assert!(calls.borrow().is_empty());
}

#[test]
fn editing_during_confirmation_requires_fresh_consent() {
    let (services, calls) = downloads();
    let mut h = harness(lossy(services), 1200.0);
    h.state_mut().request_save();
    h.run_steps(4);
    h.state_mut().run("slide.new", json!({})).unwrap();
    h.get_by_label("Save Anyway").click();
    h.run_steps(4);
    assert!(calls.borrow().is_empty());
    assert_eq!(h.state().dialog.as_ref().unwrap().id, super::DIALOG);
    h.get_by_label("Save Anyway").click();
    h.run_steps(4);
    assert_eq!(calls.borrow().len(), 1);
}
