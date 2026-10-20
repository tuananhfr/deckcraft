#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use deckcraft_engine::Session;
use egui::Event;
use serde_json::json;

use crate::{Services, SlideApp};

fn harness(services: Services) -> egui_kittest::Harness<'static, SlideApp> {
    let mut session = Session::with_new();
    let selection = &mut session.active_mut().unwrap().selection;
    selection.shapes.clear();
    selection.slides.clear();
    selection.text = None;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        SlideApp::new(session, services),
    );
    h.run_steps(3);
    h
}

#[test]
fn empty_copy_and_cut_write_nothing() {
    let calls = Rc::new(Cell::new(0));
    let sink = calls.clone();
    let mut h = harness(Services {
        set_clipboard_html: Some(Box::new(move |_, _| {
            sink.set(sink.get() + 1);
            true
        })),
        ..Default::default()
    });
    for event in [Event::Copy, Event::Cut] {
        h.event(event);
        h.run_steps(1);
        assert_eq!(calls.get(), 0);
        assert!(!h.output().platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(_))));
    }
    h.state_mut().run("edit.copy", json!({})).unwrap();
    h.run_steps(1);
    assert_eq!(calls.get(), 0, "ribbon/context copy is also empty");
    assert!(h.state().clipboard_fallback.is_none());
}

#[test]
fn rich_copy_writes_html_and_text_without_eframe_plain_write() {
    let writes = Rc::new(RefCell::new(Vec::new()));
    let sink = writes.clone();
    let mut h = harness(Services {
        set_clipboard_html: Some(Box::new(move |html, text| {
            sink.borrow_mut().push((html.to_owned(), text.to_owned()));
            true
        })),
        ..Default::default()
    });
    h.state_mut().run("insert.textBox", json!({"rect": [10, 10, 100, 40], "text": "A & B"})).unwrap();
    for event in [Event::Copy, Event::Cut] {
        h.event(event);
        h.run_steps(1);
        assert!(!h.output().platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(_))));
    }
    assert_eq!(
        &*writes.borrow(),
        &[
            ("<meta charset=\"utf-8\"><p>A &amp; B</p>".to_owned(), "A & B".to_owned()),
            ("<meta charset=\"utf-8\"><p>A &amp; B</p>".to_owned(), "A & B".to_owned()),
        ]
    );
}

#[test]
fn unsupported_rich_write_falls_back_to_plain_text() {
    let mut h = harness(Services { set_clipboard_html: Some(Box::new(|_, _| false)), ..Default::default() });
    h.state_mut().run("insert.textBox", json!({"rect": [10, 10, 100, 40], "text": "fallback"})).unwrap();
    h.event(Event::Copy);
    h.run_steps(1);
    assert!(h.output().platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == "fallback")));
    h.state_mut().run("edit.copy", json!({})).unwrap();
    h.run_steps(1);
    assert!(h.output().platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(text) if text == "fallback")));
}

#[test]
fn rich_copy_cancels_an_earlier_plain_fallback_in_the_same_frame() {
    let attempt = Rc::new(Cell::new(false));
    let next = attempt.clone();
    let mut h = harness(Services { set_clipboard_html: Some(Box::new(move |_, _| next.replace(true))), ..Default::default() });
    h.state_mut().run("insert.textBox", json!({"rect": [10, 10, 100, 40], "text": "last copy"})).unwrap();
    h.event(Event::Copy);
    h.event(Event::Copy);
    h.run_steps(1);
    assert!(attempt.get());
    assert!(!h.output().platform_output.commands.iter().any(|command| matches!(command, egui::OutputCommand::CopyText(_))));
}

#[test]
fn paste_button_requests_system_read_without_pasting_stale_content() {
    let reads = Rc::new(Cell::new(0));
    let sink = reads.clone();
    let mut h = harness(Services {
        request_clipboard: Some(Box::new(move || sink.set(sink.get() + 1))),
        clipboard_html: Some(Box::new(|| Some("<p>stale</p>".into()))),
        ..Default::default()
    });
    let before = h.state().session.active().unwrap().history.undo.len();
    h.state_mut().run("edit.paste", json!({})).unwrap();
    assert_eq!(reads.get(), 1);
    assert_eq!(h.state().session.active().unwrap().history.undo.len(), before);
    h.state_mut().run("edit.paste", json!({"text": "explicit command"})).unwrap();
    assert_eq!(reads.get(), 1, "programmatic paste does not request browser permission");
}

#[test]
fn unusable_html_pastes_text_once_and_one_undo_restores_the_slide() {
    let mut h = harness(Services {
        clipboard_html: Some(Box::new(|| Some("<meta charset='utf-8'><script>ignored</script>".into()))),
        clipboard_text: Some(Box::new(|| Some("fallback once".into()))),
        ..Default::default()
    });
    let before = h.state().session.active().unwrap().doc.clone();
    let undo_before = h.state().session.active().unwrap().history.undo.len();
    h.event(Event::Paste("fallback once".into()));
    h.run_steps(1);
    let state = h.state().session.active().unwrap();
    assert_eq!(state.history.undo.len(), undo_before + 1);
    assert_eq!(state.doc.slides[0].shapes.len(), before.slides[0].shapes.len() + 1);
    assert_eq!(state.doc.slides[0].shapes.last().unwrap().text.as_ref().unwrap().text(), "fallback once");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &before));
}

#[test]
fn textless_shape_with_minimal_html_keeps_internal_paste() {
    let mut h = harness(Services {
        clipboard_html: Some(Box::new(|| Some("<meta charset='utf-8'><p></p>".into()))),
        clipboard_text: Some(Box::new(|| Some(String::new()))),
        ..Default::default()
    });
    h.state_mut().run("shape.insert", json!({"preset": "rect", "rect": [10, 10, 100, 40]})).unwrap();
    h.state_mut().run("edit.copy", json!({})).unwrap();
    let before = h.state().session.active().unwrap().doc.clone();
    let undo_before = h.state().session.active().unwrap().history.undo.len();
    h.event(Event::Paste(String::new()));
    h.run_steps(1);
    assert_eq!(h.state().session.active().unwrap().doc.slides[0].shapes.len(), before.slides[0].shapes.len() + 1);
    assert_eq!(h.state().session.active().unwrap().history.undo.len(), undo_before + 1);
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&h.state().session.active().unwrap().doc, &before));
}

#[test]
fn image_only_paste_uses_the_existing_picture_path_once() {
    let mut h = harness(Services::default());
    let image = deckcraft_render::render_slide(
        &h.state().session.active().unwrap().doc,
        0,
        &deckcraft_render::RenderOpts { scale: 0.05, ..Default::default() },
    )
    .to_png();
    let image = Rc::new(RefCell::new(Some(image)));
    let source = image.clone();
    h.state_mut().services.clipboard_image = Some(Box::new(move || source.borrow_mut().take()));
    let before = h.state().session.active().unwrap().history.undo.len();
    h.state_mut().paste_clipboard(String::new());
    assert!(image.borrow().is_none());
    assert_eq!(h.state().session.active().unwrap().history.undo.len(), before + 1);
    assert!(matches!(h.state().session.active().unwrap().doc.slides[0].shapes.last().unwrap().kind, deckcraft_model::ShapeKind::Picture { .. }));
}
