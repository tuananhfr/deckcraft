#![allow(clippy::unwrap_used, clippy::expect_used)]

use deckcraft_engine::Session;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::{Services, SlideApp};

const OLE2: &[u8] = &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

fn harness() -> Harness<'static, SlideApp> {
    let mut app = SlideApp::new(Session::with_new(), Services::default());
    app.ui.language = "vi".into();
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

#[test]
fn opening_ole2_shows_a_dialog_without_a_status_message_and_ok_dismisses() {
    let mut h = harness();
    let before = h.state().status.clone();
    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().open_or_insert_bytes("renamed.pptx", OLE2);
    h.run_steps(2);
    assert!(h.query_by_label("Không mở được định dạng .ppt này").is_some());
    assert!(h.query_by_label("Hãy mở bằng PowerPoint hoặc LibreOffice, lưu thành .pptx rồi mở lại.").is_some());
    assert_eq!(h.state().status, before);
    assert_eq!(h.state().session.active().unwrap().uid, uid);
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    assert!(h.query_by_label("Không mở được định dạng .ppt này").is_none());
}

#[test]
fn odp_notice_is_dismissed_by_escape() {
    let mut h = harness();
    h.state_mut().open_or_insert_bytes("slides.ODP", b"PK");
    h.run_steps(2);
    assert!(h.query_by_label("Không mở được định dạng .odp này").is_some());
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    assert!(h.state().status.is_none());
}

#[test]
fn pptm_opens_with_a_macro_notice_and_save_and_save_as_suggest_pptx() {
    let mut h = harness();
    let bytes = deckcraft_engine::cmd::file::save_bytes(&deckcraft_model::defaults::new_presentation(None), "pptx").unwrap();
    h.state_mut().open_or_insert_bytes("slides.PPTM", &bytes);
    h.run_steps(2);
    assert!(h.query_by_label("Tệp .pptm đã được mở. Khi lưu, tệp sẽ được lưu thành .pptx.").is_some());
    assert!(h.query_by_label(crate::tl!("Macros will be removed.")).is_some());
    assert!(h.state().session.active().unwrap().save_as_pptx);
    assert!(h.state().status.is_none());
    h.get_by_label("OK").click();
    h.run_steps(2);
    let names = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = names.clone();
    h.state_mut().services.pick_save = Some(Box::new(move |name| {
        captured.borrow_mut().push(name.to_string());
        None
    }));
    h.state_mut().save();
    h.state_mut().run("app.saveAsDialog", json!({})).unwrap();
    assert_eq!(*names.borrow(), vec!["slides.pptx", "slides.pptx"]);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn desktop_ole2_open_uses_the_same_dialog() {
    let mut h = harness();
    let path = std::env::temp_dir().join(format!("deckcraft-legacy-ui-{}.ppt", h.state().session.active().unwrap().uid));
    std::fs::write(&path, OLE2).unwrap();
    h.state_mut().open_path(&path.to_string_lossy()).unwrap();
    h.run_steps(2);
    assert!(h.query_by_label("Không mở được định dạng .ppt này").is_some());
    assert!(h.state().status.is_none());
    std::fs::remove_file(path).unwrap();
}
