#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use deckcraft_engine::Session;
use deckcraft_model::Presentation;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use super::HostDocuments;
use crate::save_state::{CloseSaveStep, SaveState};
use crate::{SaveDone, SaveOutcome, Services, SlideApp};

#[derive(Default)]
struct Fake {
    owned: BTreeSet<u64>,
    saving: BTreeSet<u64>,
    /// (uid, revision, the presentation sent)
    sent: Vec<(u64, u64, Arc<Presentation>)>,
}

#[derive(Clone, Default)]
struct Host(Rc<RefCell<Fake>>);

impl HostDocuments for Host {
    fn owns(&self, uid: u64) -> bool {
        self.0.borrow().owned.contains(&uid)
    }
    fn save(&mut self, uid: u64, doc: &Arc<Presentation>, revision: u64) -> bool {
        let mut f = self.0.borrow_mut();
        f.sent.push((uid, revision, doc.clone()));
        f.saving.insert(uid);
        true
    }
    fn saving(&self, uid: u64) -> bool {
        self.0.borrow().saving.contains(&uid)
    }
}

type Picker = Rc<RefCell<Vec<(String, Option<SaveDone>)>>>;

/// Two presentations, the active one kept by the host; the browser's save picker stays open
/// until the test settles it.
fn app() -> (SlideApp, Host, Picker, u64) {
    let host = Host::default();
    let picker: Picker = Default::default();
    let sink = picker.clone();
    let services = Services {
        save_file: Some(Box::new(move |name: &str, _: &[u8], done: SaveDone| sink.borrow_mut().push((name.into(), Some(done))))),
        download: Some(Box::new(|_: &str, _: &[u8]| {})),
        host_documents: Some(Box::new(host.clone())),
        ..Default::default()
    };
    let mut app = SlideApp::new(Session::with_new(), services);
    app.run("file.new", json!({})).unwrap();
    app.run("slide.new", json!({})).unwrap();
    let uid = app.session.active().unwrap().uid;
    host.0.borrow_mut().owned.insert(uid);
    (app, host, picker, uid)
}

fn dirty(app: &SlideApp) -> bool {
    app.session.active().unwrap().is_dirty()
}

#[test]
fn save_hands_a_host_document_to_the_host_and_only_its_answer_saves_it() {
    let (mut app, host, picker, uid) = app();
    let revision = app.session.active().unwrap().revision;
    app.request_save();
    assert!(picker.borrow().is_empty(), "no save picker for a host document");
    let (sent_uid, sent_revision, written) = host.0.borrow().sent.last().cloned().unwrap();
    assert_eq!((sent_uid, sent_revision), (uid, revision));
    assert!(dirty(&app), "sent is not saved");
    assert_eq!(app.save_state(uid), Some(SaveState::Saving));
    app.run("slide.new", json!({})).unwrap();
    host.0.borrow_mut().saving.clear();
    app.host_saved(uid, written);
    assert!(dirty(&app), "the edit made while the save was in flight stays unsaved");
    app.run("edit.undo", json!({})).unwrap();
    assert!(!dirty(&app), "saved as of the revision that was sent");
    assert_eq!(app.close_save_step(uid), CloseSaveStep::Ready);
}

#[test]
fn a_save_while_one_is_in_flight_still_reaches_the_host() {
    let (mut app, host, _, uid) = app();
    app.save();
    app.run("slide.new", json!({})).unwrap();
    app.save_document(app.session.active_index().unwrap());
    assert_eq!(host.0.borrow().sent.len(), 2, "the host coalesces; the UI never drops a Save");
    assert_eq!(app.close_save_step(uid), CloseSaveStep::Waiting);
}

#[test]
fn unsaved_answers_keep_the_document_dirty_and_only_errors_are_shown() {
    let (mut app, host, _, uid) = app();
    app.save();
    host.0.borrow_mut().saving.clear();
    app.status = None;
    app.host_save_ended(uid, None);
    assert!(dirty(&app));
    assert!(app.status.is_none(), "offline and conflicts are the host's to show");
    assert_eq!(app.save_state(uid), None);
    assert_eq!(app.close_save_step(uid), CloseSaveStep::Stopped);
    app.host_save_ended(uid, Some("Tệp quá lớn"));
    assert_eq!(app.save_state(uid), Some(SaveState::Failed("Tệp quá lớn".into())));
    assert!(app.status.as_ref().is_some_and(|(s, _)| s.contains("Tệp quá lớn")));
}

#[test]
fn save_as_of_a_host_document_is_a_local_copy() {
    let (mut app, host, picker, _) = app();
    app.save_as_dialog("pptx");
    assert!(host.0.borrow().sent.is_empty());
    assert_eq!(picker.borrow().len(), 1);
    let done = picker.borrow_mut()[0].1.take().unwrap();
    done(SaveOutcome::Saved);
    app.settle_saves();
    assert!(dirty(&app), "a local copy doesn't save the server's document");
}

#[test]
fn host_documents_are_never_drafts_and_other_documents_save_as_before() {
    let (mut app, host, picker, uid) = app();
    let inputs = app.draft_inputs();
    assert!(inputs.iter().all(|d| d.uid != uid));
    assert_eq!(inputs.len(), 1, "the other presentation still drafts");
    app.session.set_active(0);
    app.run("slide.new", json!({})).unwrap();
    app.save();
    assert_eq!(picker.borrow().len(), 1, "a local presentation still uses the save picker");
    assert!(host.0.borrow().sent.is_empty());
}

#[test]
fn without_a_host_nothing_is_owned() {
    let mut app = SlideApp::new(Session::with_new(), Services::default());
    let uid = app.session.active().unwrap().uid;
    assert!(!app.host_owns(uid));
    assert!(!app.save_in_flight(uid));
    assert_eq!(app.draft_inputs().len(), 1);
    app.run("slide.new", json!({})).unwrap();
    assert!(app.has_unsaved_local_changes() && app.has_unsaved_changes());
}

#[test]
fn leaving_the_page_is_guarded_by_the_host_for_its_documents() {
    let (mut app, _, _, _) = app();
    assert!(app.has_unsaved_changes());
    assert!(!app.has_unsaved_local_changes(), "only the host document is edited");
    app.session.set_active(0);
    app.run("slide.new", json!({})).unwrap();
    assert!(app.has_unsaved_local_changes());
}

#[test]
fn a_lossy_host_document_is_sent_only_after_save_anyway() {
    let (mut app, host, _, _) = app();
    {
        let d = app.session.active_mut().unwrap();
        Arc::make_mut(&mut d.doc).import_losses.macros = true;
    }
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h.state_mut().request_save();
    h.run_steps(3);
    assert!(host.0.borrow().sent.is_empty(), "nothing is sent over a loss warning");
    h.get_by_label("Save Anyway").click();
    h.run_steps(3);
    assert_eq!(host.0.borrow().sent.len(), 1);
}
