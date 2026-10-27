//! The online-document flow end to end with a fake `erpHost`: the shell's calls in, the app's
//! frames, and the editor's calls out.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use deckcraft_engine::Session;
use deckcraft_ui_egui::save_state::SaveState;
use deckcraft_ui_egui::{Services, SlideApp};
use serde_json::json;

use super::ErpHost;
use super::autosave::{IDLE_MS, Outcome, Reason};
use super::documents::{HostApi, HostDocs};
use super::open::OpenRequest;

struct Save {
    doc_id: String,
    revision: u64,
    reason: Reason,
    bytes: Vec<u8>,
    done: Option<Box<dyn FnOnce(Outcome)>>,
}

#[derive(Default)]
struct Calls {
    saves: Vec<Save>,
    dirty: Vec<(String, bool)>,
    opened: Vec<(String, bool, Option<String>)>,
    closed: Vec<String>,
}

#[derive(Clone, Default)]
struct FakeHost(Rc<RefCell<Calls>>);

impl HostApi for FakeHost {
    fn save(&mut self, doc_id: &str, bytes: Vec<u8>, revision: u64, reason: Reason, done: Box<dyn FnOnce(Outcome)>) {
        self.0.borrow_mut().saves.push(Save { doc_id: doc_id.into(), revision, reason, bytes, done: Some(done) });
    }
    fn dirty(&mut self, doc_id: &str, dirty: bool) {
        self.0.borrow_mut().dirty.push((doc_id.into(), dirty));
    }
    fn opened(&mut self, doc_id: &str, ok: bool, reason: Option<&str>) {
        self.0.borrow_mut().opened.push((doc_id.into(), ok, reason.map(str::to_string)));
    }
    fn closed(&mut self, doc_id: &str) {
        self.0.borrow_mut().closed.push(doc_id.into());
    }
}

struct World {
    app: SlideApp,
    host: ErpHost,
    calls: FakeHost,
    clock: Rc<Cell<f64>>,
    opens: Rc<RefCell<Vec<OpenRequest>>>,
}

fn pptx() -> Vec<u8> {
    let session = Session::with_new();
    deckcraft_engine::cmd::file::save_bytes(&session.active().unwrap().doc, "pptx").unwrap()
}

fn request(doc_id: &str, version: u64) -> OpenRequest {
    OpenRequest { doc_id: doc_id.into(), version, title: "Báo cáo quý.pptx".into(), format: "pptx".into(), bytes: Some(pptx()) }
}

impl World {
    /// The app with one local presentation open, as the web build starts.
    fn new() -> Self {
        let calls = FakeHost::default();
        let clock = Rc::new(Cell::new(0.0));
        let now = clock.clone();
        let docs = HostDocs::new(Box::new(calls.clone()), Box::new(move || now.get()), Box::new(|| {}));
        let opens = Rc::new(RefCell::new(Vec::new()));
        let host = ErpHost::new(docs, opens.clone(), Box::new(|| {}));
        let services = Services {
            host_documents: Some(host.documents()),
            // The web build's save picker; local Save As isn't exercised here.
            save_file: Some(Box::new(|_: &str, _: &[u8], _: deckcraft_ui_egui::SaveDone| {})),
            download: Some(Box::new(|_: &str, _: &[u8]| {})),
            ..Default::default()
        };
        let app = SlideApp::new(Session::with_new(), services);
        World { app, host, calls, clock, opens }
    }

    fn frame(&mut self, now: f64) {
        self.clock.set(now);
        self.host.logic(&mut self.app);
        self.host.after_frame(&self.app);
    }

    /// Opens `doc_id` through `open` and returns its uid.
    fn open(&mut self, doc_id: &str) -> u64 {
        self.opens.borrow_mut().push(request(doc_id, 1));
        let now = self.clock.get();
        self.frame(now);
        self.app.session.active().unwrap().uid
    }

    fn edit(&mut self, now: f64) {
        self.app.run("slide.new", json!({})).unwrap();
        self.frame(now);
    }

    fn saves(&self) -> Vec<(String, u64, Reason)> {
        self.calls.0.borrow().saves.iter().map(|s| (s.doc_id.clone(), s.revision, s.reason)).collect()
    }

    /// The shell answers save `i`.
    fn answer(&mut self, i: usize, outcome: Outcome) {
        let done = self.calls.0.borrow_mut().saves[i].done.take().unwrap();
        done(outcome);
        let now = self.clock.get();
        self.frame(now);
    }

    fn revision(&self) -> u64 {
        self.app.session.active().unwrap().revision
    }

    fn dirty(&self) -> bool {
        self.app.session.active().unwrap().is_dirty()
    }

    fn dirty_calls(&self) -> Vec<(String, bool)> {
        self.calls.0.borrow().dirty.clone()
    }
}

#[test]
fn open_adopts_the_document_under_its_title_and_reports_it_once() {
    let mut w = World::new();
    let uid = w.open("d1");
    assert_eq!(w.calls.0.borrow().opened, vec![("d1".into(), true, None)]);
    let d = w.app.session.active().unwrap();
    assert_eq!(d.title(), "Báo cáo quý");
    assert!(!d.is_dirty());
    assert!(w.app.host_owns(uid));
    assert!(w.app.draft_inputs().iter().all(|d| d.uid != uid), "never an erpDrafts draft");
    assert_eq!(w.app.draft_inputs().len(), 1, "the local presentation still is");
    w.frame(100_000.0);
    assert!(w.saves().is_empty(), "nothing to save until it is edited");
}

#[test]
fn refused_opens_say_why() {
    let mut w = World::new();
    let mut docx = request("a", 1);
    docx.format = "docx".into();
    let mut garbage = request("b", 1);
    garbage.bytes = Some(b"not a presentation".to_vec());
    let mut missing = request("c", 1);
    missing.bytes = None;
    // A legacy .ppt, or a password-protected .pptx: both are OLE2 containers.
    let mut ole = request("e", 1);
    ole.bytes = Some(vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1, 0, 0]);
    w.opens.borrow_mut().extend([docx, garbage, missing, ole]);
    w.frame(0.0);
    let opened = w.calls.0.borrow().opened.clone();
    assert_eq!(
        opened,
        vec![
            ("a".into(), false, Some("unsupported".into())),
            ("b".into(), false, Some("parserFailed".into())),
            ("c".into(), false, Some("parserFailed".into())),
            ("e".into(), false, Some("unsupported".into()))
        ]
    );
    for i in 0..9 {
        w.open(&format!("full-{i}"));
    }
    assert_eq!(w.app.session.documents().len(), 10);
    w.opens.borrow_mut().push(request("one-too-many", 1));
    w.frame(0.0);
    assert_eq!(w.calls.0.borrow().opened.last().unwrap(), &("one-too-many".into(), false, Some("parserFailed".into())));
}

#[test]
fn autosave_sends_the_revision_and_only_its_acknowledgement_saves_it() {
    let mut w = World::new();
    w.open("d1");
    w.edit(0.0);
    assert_eq!(w.dirty_calls(), vec![("d1".into(), true)]);
    w.frame(IDLE_MS - 1.0);
    assert!(w.saves().is_empty());
    let sent = w.revision();
    w.frame(IDLE_MS);
    assert_eq!(w.saves(), vec![("d1".into(), sent, Reason::Auto)]);
    assert!(w.calls.0.borrow().saves[0].bytes.starts_with(b"PK"), "the .pptx Save writes");
    w.edit(IDLE_MS + 10.0);
    w.answer(0, Outcome::Saved { version: Some(2) });
    assert!(w.dirty(), "the edit made while the save was in flight stays unsaved");
    assert_eq!(w.dirty_calls().len(), 1, "still dirty, nothing new to report");
    w.frame(2.0 * IDLE_MS + 10.0);
    assert_eq!(w.saves().len(), 2);
    assert_eq!(w.saves()[1].1, w.revision());
    w.answer(1, Outcome::Saved { version: Some(3) });
    assert!(!w.dirty());
    assert_eq!(w.dirty_calls().last().unwrap(), &("d1".into(), false));
}

#[test]
fn ctrl_s_saves_at_once_and_coalesces_behind_a_save_in_flight() {
    let mut w = World::new();
    let uid = w.open("d1");
    w.edit(0.0);
    w.app.request_save();
    assert_eq!(w.saves(), vec![("d1".into(), w.revision(), Reason::Manual)]);
    assert_eq!(w.app.save_state(uid), Some(SaveState::Saving));
    w.edit(10.0);
    w.app.request_save();
    w.edit(20.0);
    w.app.request_save();
    w.frame(100_000.0);
    assert_eq!(w.saves().len(), 1, "one save in flight per document");
    w.answer(0, Outcome::Saved { version: Some(2) });
    assert_eq!(w.saves().len(), 2, "one follow-up");
    assert_eq!(w.saves()[1].1, w.revision(), "of the newest revision");
    assert_eq!(w.saves()[1].2, Reason::Manual);
}

#[test]
fn a_conflict_stops_autosave_and_is_never_retried() {
    let mut w = World::new();
    let uid = w.open("d1");
    w.edit(0.0);
    w.frame(IDLE_MS);
    w.answer(0, Outcome::Conflict);
    assert!(w.dirty());
    assert_eq!(w.app.save_state(uid), None, "the shell shows the conflict");
    w.edit(10_000.0);
    w.frame(100_000.0);
    assert_eq!(w.saves().len(), 1, "no AutoSave after a conflict");
    // The shell reloads the latest version in its place.
    let mut latest = request("d1", 7);
    latest.title = "Báo cáo quý".into();
    w.opens.borrow_mut().push(latest);
    w.frame(100_000.0);
    let documents: Vec<String> = w.app.session.documents().iter().map(|d| d.title()).collect();
    assert_eq!(documents.iter().filter(|t| *t == "Báo cáo quý").count(), 1, "the tab is replaced: {documents:?}");
    assert!(w.calls.0.borrow().closed.is_empty(), "the document is still open");
    assert_eq!(w.dirty_calls().last().unwrap(), &("d1".into(), false));
    w.edit(200_000.0);
    w.frame(300_000.0);
    assert_eq!(w.saves().len(), 2, "AutoSave runs for the reloaded version");
}

#[test]
fn offline_keeps_the_document_dirty_and_autosave_running() {
    let mut w = World::new();
    let uid = w.open("d1");
    w.edit(0.0);
    w.frame(IDLE_MS);
    w.app.status = None;
    w.answer(0, Outcome::Retry);
    assert!(w.dirty());
    assert!(w.app.status.is_none(), "the shell shows the offline state");
    assert_eq!(w.app.save_state(uid), None);
    w.frame(100_000.0);
    assert_eq!(w.saves().len(), 1, "the shell retries those bytes, not the editor");
    w.edit(200_000.0);
    w.frame(200_000.0 + IDLE_MS);
    assert_eq!(w.saves().len(), 2);
}

#[test]
fn a_failure_shows_its_message_where_save_errors_are() {
    let mut w = World::new();
    let uid = w.open("d1");
    w.edit(0.0);
    w.frame(IDLE_MS);
    w.answer(0, Outcome::Failed { message: "Tệp vượt quá dung lượng cho phép.".into() });
    assert_eq!(w.app.save_state(uid), Some(SaveState::Failed("Tệp vượt quá dung lượng cho phép.".into())));
    assert!(w.app.status.as_ref().is_some_and(|(s, _)| s.contains("Tệp vượt quá dung lượng cho phép.")));
    w.edit(10_000.0);
    w.frame(100_000.0);
    assert_eq!(w.saves().len(), 1, "AutoSave stopped");
}

#[test]
fn flush_hands_every_edited_document_over_without_waiting_for_answers() {
    let mut w = World::new();
    w.open("d1");
    w.edit(0.0);
    w.open("d2");
    w.edit(0.0);
    w.open("d3");
    let resolved = Rc::new(Cell::new(false));
    let flag = resolved.clone();
    w.host.docs.flush(Box::new(move || flag.set(true)));
    let saves = w.saves();
    assert_eq!(saves.iter().map(|s| (s.0.as_str(), s.2)).collect::<Vec<_>>(), vec![("d1", Reason::Flush), ("d2", Reason::Flush)]);
    assert!(resolved.get(), "resolved once handed over; the shell tracks the answers");
    w.answer(0, Outcome::Saved { version: Some(2) });
    w.answer(1, Outcome::Retry);
    let clean = Rc::new(Cell::new(false));
    let flag = clean.clone();
    w.host.docs.flush(Box::new(move || flag.set(true)));
    assert!(clean.get());
    assert_eq!(w.saves().len(), 3, "d2 still differs from its acknowledged revision");
    assert_eq!(w.saves()[2].0, "d2");
}

#[test]
fn flush_behind_a_save_in_flight_resolves_when_its_follow_up_is_handed_over() {
    let mut w = World::new();
    w.open("d1");
    w.edit(0.0);
    w.frame(IDLE_MS);
    w.edit(IDLE_MS + 10.0);
    let resolved = Rc::new(Cell::new(false));
    let flag = resolved.clone();
    w.host.docs.flush(Box::new(move || flag.set(true)));
    assert_eq!(w.saves().len(), 1, "queued behind the AutoSave in flight");
    assert!(!resolved.get());
    w.answer(0, Outcome::Saved { version: Some(2) });
    assert_eq!(w.saves().len(), 2);
    assert_eq!(w.saves()[1].2, Reason::Flush);
    assert!(resolved.get(), "without waiting for the follow-up's answer");
}

#[test]
fn acknowledged_after_offline_saves_that_revision_only() {
    let mut w = World::new();
    let uid = w.open("d1");
    w.edit(0.0);
    w.frame(IDLE_MS);
    let sent = w.saves()[0].1;
    w.answer(0, Outcome::Retry);
    assert!(w.dirty());
    w.host.docs.acknowledged("unknown", sent, Some(3));
    w.frame(IDLE_MS);
    assert!(w.dirty(), "an unknown docId changes nothing");
    w.host.docs.acknowledged("d1", sent, Some(3));
    w.frame(IDLE_MS);
    assert!(!w.dirty(), "the shell's retry stored it");
    assert_eq!(w.dirty_calls().last().unwrap(), &("d1".into(), false));
    assert_eq!(w.host.docs.version(uid), Some(3));
    assert_eq!(w.app.save_state(uid), None);
}

#[test]
fn acknowledged_keeps_edits_made_after_the_save_and_ignores_older_revisions() {
    let mut w = World::new();
    w.open("d1");
    w.edit(0.0);
    w.frame(IDLE_MS);
    let first = w.saves()[0].1;
    w.answer(0, Outcome::Retry);
    w.edit(10_000.0);
    w.frame(10_000.0 + IDLE_MS);
    let second = w.saves()[1].1;
    w.answer(1, Outcome::Retry);
    w.edit(20_000.0);
    w.host.docs.acknowledged("d1", second, Some(4));
    w.frame(20_000.0);
    assert!(w.dirty(), "edited after the save that was acknowledged");
    w.app.run("edit.undo", json!({})).unwrap();
    w.frame(20_001.0);
    assert!(!w.dirty(), "clean as of the acknowledged revision");
    w.host.docs.acknowledged("d1", first, Some(3));
    w.frame(20_002.0);
    assert!(!w.dirty(), "an older acknowledgement doesn't roll back the saved state");
    assert_eq!(w.host.docs.version(w.app.session.active().unwrap().uid), Some(4));
}

#[test]
fn host_documents_never_enter_reopen_closed() {
    let mut w = World::new();
    w.open("d1");
    w.edit(0.0);
    let host_tab = w.app.session.active_index().unwrap();
    w.app.run("file.close", json!({"index": host_tab})).unwrap();
    w.frame(0.0);
    assert!(w.app.session.closed.is_empty(), "a host document never reopens as a local copy");
    w.app.run("slide.new", json!({})).unwrap();
    w.app.run("file.close", json!({"index": 0})).unwrap();
    assert_eq!(w.app.session.closed.len(), 1, "a local presentation still does");
}

#[test]
fn closing_a_host_tab_tells_the_shell_and_local_documents_are_untouched() {
    let mut w = World::new();
    w.open("d1");
    w.app.session.set_active(0);
    w.edit(0.0);
    w.frame(100_000.0);
    assert!(w.saves().is_empty(), "a local presentation never goes to the server");
    assert!(w.dirty_calls().is_empty());
    let host_tab = w.app.session.documents().iter().position(|d| w.app.host_owns(d.uid)).unwrap();
    w.app.run("file.close", json!({"index": host_tab})).unwrap();
    w.frame(100_000.0);
    assert_eq!(w.calls.0.borrow().closed, vec!["d1".to_string()]);
}

#[test]
fn a_slide_show_holds_autosave_until_it_ends() {
    let mut w = World::new();
    w.open("d1");
    w.edit(0.0);
    w.app.start_show(0, false);
    w.frame(100_000.0);
    assert!(w.saves().is_empty());
    w.app.end_show();
    w.frame(100_001.0);
    assert_eq!(w.saves().len(), 1);
}

#[test]
fn unconfirmed_losses_hold_autosave() {
    let mut w = World::new();
    w.open("d1");
    {
        let d = w.app.session.active_mut().unwrap();
        std::sync::Arc::make_mut(&mut d.doc).import_losses.macros = true;
        d.revision += 1;
    }
    w.frame(0.0);
    w.frame(100_000.0);
    assert!(w.saves().is_empty(), "never saved silently over a loss warning");
    w.host.docs.flush(Box::new(|| {}));
    assert!(w.saves().is_empty());
}

#[test]
fn reopening_the_open_version_keeps_the_tab() {
    let mut w = World::new();
    let uid = w.open("d1");
    w.app.session.set_active(0);
    w.opens.borrow_mut().push(request("d1", 1));
    w.frame(0.0);
    assert_eq!(w.app.session.active().unwrap().uid, uid, "switched to it, not reloaded");
    assert_eq!(w.app.session.documents().len(), 2);
}
