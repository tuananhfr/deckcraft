//! Each file delivered through the inbox (the web app's drops and the office shell's handover)
//! reports once whether it was opened or inserted, after the editor decided.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::rc::Rc;

use deckcraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use crate::{Inbox, Services, SlideApp};

type Results = Rc<RefCell<Vec<(String, usize, bool)>>>;

fn harness(app: SlideApp) -> Harness<'static, SlideApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(2);
    h
}

fn reporting() -> (SlideApp, Inbox, Results) {
    let inbox = Inbox::default();
    let results: Results = Default::default();
    let sink = results.clone();
    let services = Services {
        inbox: Some(inbox.clone()),
        intake_result: Some(Box::new(move |name: &str, size: usize, ok: bool| sink.borrow_mut().push((name.into(), size, ok)))),
        ..Default::default()
    };
    (SlideApp::new(Session::with_new(), services), inbox, results)
}

fn deliver(h: &mut Harness<'_, SlideApp>, inbox: &Inbox, files: &[(&str, Vec<u8>)]) {
    inbox.lock().unwrap().extend(files.iter().map(|(name, bytes)| (name.to_string(), bytes.clone())));
    h.run_steps(2);
}

fn pptx() -> Vec<u8> {
    deckcraft_engine::cmd::file::save_bytes(&deckcraft_model::defaults::new_presentation(None), "pptx").unwrap()
}

#[test]
fn opened_inserted_and_refused_files_each_report_once() {
    let (app, inbox, results) = reporting();
    let mut h = harness(app);
    let picture = deckcraft_render::render_slide(
        &h.state().session.active().unwrap().doc,
        0,
        &deckcraft_render::RenderOpts { scale: 0.05, ..Default::default() },
    )
    .to_png();
    let files = [
        ("Báo cáo.pptx", pptx()),
        ("ảnh.png", picture),
        ("dàn ý.txt", b"Title\n\tPoint".to_vec()),
        ("broken.pptx", b"PK\x03\x04 not really".to_vec()),
        ("archive.zip", b"PK\x03\x04".to_vec()),
    ];
    deliver(&mut h, &inbox, &files);
    let expected: Vec<_> =
        files.iter().zip([true, true, true, false, false]).map(|((name, bytes), ok)| (name.to_string(), bytes.len(), ok)).collect();
    assert_eq!(*results.borrow(), expected);
    h.run_steps(3);
    assert_eq!(results.borrow().len(), files.len(), "reported once");
}

#[test]
fn legacy_formats_report_refused_and_still_show_their_notice() {
    let (app, inbox, results) = reporting();
    let mut h = harness(app);
    deliver(&mut h, &inbox, &[("old.ppt", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1".to_vec())]);
    assert_eq!(*results.borrow(), [("old.ppt".to_string(), 8, false)]);
    assert!(h.query_by_label("Cannot open this .ppt format").is_some());
    h.get_by_label("OK").click();
    h.run_steps(2);
    deliver(&mut h, &inbox, &[("slides.odp", b"PK".to_vec())]);
    assert_eq!(results.borrow().last(), Some(&("slides.odp".to_string(), 2, false)));
    assert!(h.query_by_label("Cannot open this .odp format").is_some());
}

#[test]
fn a_presentation_over_the_documents_limit_reports_refused() {
    let (mut app, inbox, results) = reporting();
    while app.session.documents().len() < deckcraft_engine::MAX_DOCUMENTS {
        app.run("file.new", serde_json::json!({})).unwrap();
    }
    let mut h = harness(app);
    deliver(&mut h, &inbox, &[("eleventh.pptx", pptx())]);
    assert_eq!(results.borrow().last().map(|r| r.2), Some(false));
    assert_eq!(h.state().session.documents().len(), deckcraft_engine::MAX_DOCUMENTS);
}
