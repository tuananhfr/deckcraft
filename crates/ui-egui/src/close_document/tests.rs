#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::rc::Rc;

use deckcraft_engine::Session;
use serde_json::json;

use crate::{Services, SlideApp};

#[test]
fn closed_hook_runs_once_per_last_document_close() {
    let calls = Rc::new(Cell::new(0));
    let sink = calls.clone();
    let services = Services { editor_closed: Some(Box::new(move || sink.set(sink.get() + 1))), ..Default::default() };
    let mut app = SlideApp::new(Session::new(), services);
    assert!(app.run("file.close", json!({})).is_err());
    assert_eq!(calls.get(), 0);
    app.run("file.new", json!({})).unwrap();
    app.run("file.new", json!({})).unwrap();
    assert!(app.run("file.close", json!({"index": 99})).is_err());
    app.close_presentation();
    assert_eq!(calls.get(), 0);
    app.close_presentation();
    assert_eq!(calls.get(), 1);
    app.close_presentation();
    app.run("app.closeAll", json!({})).unwrap();
    assert_eq!(calls.get(), 1);
    app.run("file.reopenClosed", json!({})).unwrap();
    app.close_presentation();
    assert_eq!(calls.get(), 2);
    app.run("file.new", json!({})).unwrap();
    app.run("file.new", json!({})).unwrap();
    app.run("app.closeAll", json!({})).unwrap();
    assert_eq!(calls.get(), 3);
}

#[test]
fn closing_the_last_document_needs_no_host_hook() {
    let mut app = SlideApp::new(Session::with_new(), Services::default());
    app.close_presentation();
    assert!(app.session.documents().is_empty());
}
