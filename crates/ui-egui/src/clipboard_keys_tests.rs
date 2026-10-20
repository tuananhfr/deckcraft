#![allow(clippy::unwrap_used, clippy::expect_used)]

use deckcraft_engine::Session;
use egui::{Event, Key, Modifiers};
use serde_json::json;

use crate::{Services, SlideApp, clipboard_keys::drop_duplicate_clipboard_keys};

#[test]
fn raw_input_hook_preserves_synthetic_clipboard_keys() {
    let mut app = SlideApp::new(Session::with_new(), Services::default());
    let events = vec![
        Event::Key { key: Key::V, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::CTRL | Modifiers::COMMAND },
        Event::Key { key: Key::V, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::CTRL | Modifiers::COMMAND },
    ];
    app.synthetic = events.clone();
    let mut raw = egui::RawInput::default();
    app.raw_input_hook(&mut raw);
    assert_eq!(raw.events, events);
    assert!(app.synthetic.is_empty());
}

#[test]
fn web_key_and_paste_duplicate_internal_shape_once_and_one_undo_restores_it() {
    let mut session = Session::with_new();
    let id = session.execute("shape.insert", &json!({"preset": "rect", "rect": [100.0, 100.0, 120.0, 80.0]})).unwrap()["id"].clone();
    session.execute("edit.select", &json!({"ids": [id]})).unwrap();
    let copied = session.execute("edit.copy", &json!({})).unwrap();
    let text = copied["text"].as_str().unwrap().to_owned();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SlideApp::new(session, Services::default()),
    );
    h.run_steps(3);
    let before = h.state().session.active().unwrap().doc.slides[0].shapes.len();
    let undo_before = h.state().session.active().unwrap().history.undo.len();
    let mut events = vec![
        Event::Key { key: Key::V, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::CTRL | Modifiers::COMMAND },
        Event::Paste(text),
    ];
    drop_duplicate_clipboard_keys(&mut events, true);
    for event in events {
        h.event(event);
    }
    h.run_steps(1);
    assert_eq!(h.state().session.active().unwrap().doc.slides[0].shapes.len(), before + 1);
    assert_eq!(h.state().session.active().unwrap().history.undo.len(), undo_before + 1);
    assert_eq!(h.state().session.clipboard.pastes, 1);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.run_steps(1);
    assert_eq!(h.state().session.active().unwrap().doc.slides[0].shapes.len(), before);
    assert_eq!(h.state().session.active().unwrap().history.undo.len(), undo_before);
}
