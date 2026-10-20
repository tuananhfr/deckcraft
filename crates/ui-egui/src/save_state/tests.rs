#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::rc::Rc;

use deckcraft_engine::Session;
use serde_json::json;

use super::CloseSaveStep;
use crate::{SaveDone, SaveOutcome, Services, SlideApp};

type Completion = Rc<RefCell<Option<SaveDone>>>;

fn app() -> (SlideApp, Completion, u64) {
    let completion: Completion = Default::default();
    let sink = completion.clone();
    let services = Services {
        save_file: Some(Box::new(move |_, _, done| *sink.borrow_mut() = Some(done))),
        download: Some(Box::new(|_, _| {})),
        ..Default::default()
    };
    let mut app = SlideApp::new(Session::new(), services);
    app.run("file.new", json!({})).unwrap();
    app.run("slide.new", json!({})).unwrap();
    let uid = app.session.active().unwrap().uid;
    (app, completion, uid)
}

fn finish(app: &mut SlideApp, completion: &Completion, outcome: SaveOutcome) {
    completion.borrow_mut().take().unwrap()(outcome);
    app.settle_saves();
}

#[test]
fn close_save_waits_until_the_write_result_is_settled() {
    let (mut app, completion, uid) = app();
    app.save_document(0);
    assert_eq!(app.close_save_step(uid), CloseSaveStep::Waiting);
    completion.borrow_mut().take().unwrap()(SaveOutcome::Saved);
    assert_eq!(app.close_save_step(uid), CloseSaveStep::Waiting);
    assert!(app.session.active().unwrap().is_dirty());
    app.settle_saves();
    assert_eq!(app.close_save_step(uid), CloseSaveStep::Ready);
    assert!(!app.session.active().unwrap().is_dirty());
}

#[test]
fn cancelled_or_failed_close_saves_keep_changes_unsaved() {
    for outcome in [SaveOutcome::Cancelled, SaveOutcome::Failed("permission denied".into())] {
        let (mut app, completion, uid) = app();
        app.save_document(0);
        finish(&mut app, &completion, outcome);
        assert_eq!(app.close_save_step(uid), CloseSaveStep::Stopped);
        assert!(app.session.active().unwrap().is_dirty());
    }
}

#[test]
fn a_cancelled_write_does_not_reuse_an_earlier_success() {
    let (mut app, completion, uid) = app();
    app.save_document(0);
    finish(&mut app, &completion, SaveOutcome::Saved);
    assert_eq!(app.close_save_step(uid), CloseSaveStep::Ready);
    app.save_document(0);
    app.close_document_by_uid(uid);
    assert!(app.dialog.as_ref().unwrap().fields.contains_key("lossSaveStarted"));
    finish(&mut app, &completion, SaveOutcome::Cancelled);
    assert_eq!(app.close_save_step(uid), CloseSaveStep::Stopped);
    assert_eq!(app.session.documents().len(), 1);
}

#[test]
fn edits_during_a_close_save_prevent_closing() {
    let (mut app, completion, uid) = app();
    app.save_document(0);
    app.run("slide.new", json!({})).unwrap();
    finish(&mut app, &completion, SaveOutcome::Saved);
    assert_eq!(app.close_save_step(uid), CloseSaveStep::Stopped);
    assert!(app.session.active().unwrap().is_dirty());
}

#[test]
fn close_reuses_an_in_flight_save_and_keeps_its_target() {
    let (mut app, completion, uid) = app();
    app.save_document(0);
    let id = app.pending_saves.first().unwrap().id;
    app.close_document_by_uid(uid);
    app.save_document(0);
    assert_eq!(app.pending_saves.len(), 1);
    assert_eq!(app.pending_saves.first().unwrap().id, id);
    app.run("file.new", json!({})).unwrap();
    assert_ne!(app.session.active().unwrap().uid, uid);
    finish(&mut app, &completion, SaveOutcome::Saved);
    assert_eq!(app.close_save_step(uid), CloseSaveStep::Ready);
    assert_ne!(app.session.active().unwrap().uid, uid);
}

#[test]
fn close_waits_for_a_queued_desktop_save() {
    let mut app = SlideApp::new(Session::with_new(), Services::default());
    let uid = app.session.active().unwrap().uid;
    app.request_save();
    app.close_document_by_uid(uid);
    assert_eq!(app.close_save_step(uid), CloseSaveStep::Waiting);
    assert_eq!(app.session.documents().len(), 1);
    assert!(app.dialog.as_ref().unwrap().fields.contains_key("lossSaveStarted"));
}
