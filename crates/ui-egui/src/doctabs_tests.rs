//! Document tabs: shown with two or more presentations, switching, closing (with the
//! unsaved-changes prompt), the open-documents limit on every UI path, and per-document zoom.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use deckcraft_engine::{MAX_DOCUMENTS, Session};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::{Services, SlideApp, doctabs};

fn harness(app: SlideApp) -> Harness<'static, SlideApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

fn app_with(n: usize, services: Services) -> SlideApp {
    let mut app = SlideApp::new(Session::new(), services);
    for _ in 0..n {
        app.run("file.new", json!({})).unwrap();
    }
    app
}

fn titles(app: &SlideApp) -> Vec<String> {
    app.session.documents().iter().map(|d| d.title()).collect()
}

fn active_title(app: &SlideApp) -> String {
    app.session.active().unwrap().title()
}

/// Gives presentation `i` an unsaved change without making it active.
fn make_dirty(app: &mut SlideApp, i: usize) {
    let back = app.session.active_index().unwrap();
    app.session.set_active(i);
    let d = app.session.active_mut().unwrap();
    d.doc = Arc::new((*d.doc).clone());
    app.session.set_active(back);
}

fn pptx_bytes() -> Vec<u8> {
    deckcraft_engine::cmd::file::save_bytes(&deckcraft_model::defaults::new_presentation(None), "pptx").unwrap()
}

#[test]
fn the_tab_row_shows_only_with_two_or_more_presentations() {
    let mut h = harness(app_with(1, Services::default()));
    assert!(!doctabs::visible(h.state()));
    assert!(h.query_by_label("Presentation1").is_none(), "one presentation: no tabs");
    assert!(h.query_by_label("Close Presentation1").is_none());
    let top_alone = h.state().canvas_rect.unwrap().min.y;

    h.state_mut().run("file.new", json!({})).unwrap();
    h.run_steps(3);
    assert!(doctabs::visible(h.state()));
    assert!(h.query_by_label("Presentation1").is_some());
    assert!(h.query_by_label("Presentation2").is_some());
    assert!(h.query_by_label("Close Presentation2").is_some());
    let top = h.state().canvas_rect.unwrap().min.y;
    assert!((top - top_alone - doctabs::HEIGHT).abs() < 0.5, "the row takes {top_alone} → {top}");

    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Presentation1").is_none(), "back to one: the row goes");
    assert!((h.state().canvas_rect.unwrap().min.y - top_alone).abs() < 0.5, "and the editor is laid out as before");
}

#[test]
fn clicking_a_tab_switches_through_window_next() {
    let mut h = harness(app_with(3, Services::default()));
    assert_eq!(active_title(h.state()), "Presentation3");
    h.get_by_label("Presentation1").click();
    h.run_steps(3);
    assert_eq!(active_title(h.state()), "Presentation1");
    assert_eq!(h.state().session.journal.last(), Some(&("window.next".to_string(), json!({"index": 0}))));
    h.get_by_label("Presentation2").click();
    h.run_steps(3);
    assert_eq!(active_title(h.state()), "Presentation2");
}

#[test]
fn the_close_button_closes_its_own_presentation_and_keeps_the_active_one() {
    let mut h = harness(app_with(3, Services::default()));
    h.get_by_label("Close Presentation1").click();
    h.run_steps(3);
    assert_eq!(titles(h.state()), ["Presentation2", "Presentation3"]);
    assert_eq!(active_title(h.state()), "Presentation3");
    assert!(h.state().dialog.is_none(), "nothing unsaved: no prompt");
    // Middle-click closes too.
    h.get_by_label("Presentation2").click_button(egui::PointerButton::Middle);
    h.run_steps(3);
    assert_eq!(titles(h.state()), ["Presentation3"]);
    assert!(h.query_by_label("Presentation3").is_none(), "one left: no tabs");
}

#[test]
fn closing_an_unsaved_tab_asks_first_without_switching() {
    let mut app = app_with(3, Services::default());
    make_dirty(&mut app, 0);
    let mut h = harness(app);
    h.get_by_label("Close Presentation1").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.id.as_str()), Some("close"));
    assert!(h.query_by_label_contains("“Presentation1”").is_some(), "the prompt names the tab's presentation");
    assert_eq!(active_title(h.state()), "Presentation3", "the prompt doesn't switch");
    assert_eq!(h.state().session.documents().len(), 3);

    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(h.state().session.documents().len(), 3, "Cancel keeps it");

    h.get_by_label("Close Presentation1").click();
    h.run_steps(3);
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert_eq!(titles(h.state()), ["Presentation2", "Presentation3"]);
    assert_eq!(active_title(h.state()), "Presentation3");
}

#[test]
fn save_in_the_prompt_saves_the_tabs_presentation_then_closes_it() {
    let saved: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
    let sink = saved.clone();
    let services = Services { download: Some(Box::new(move |name: &str, _: &[u8]| sink.borrow_mut().push(name.to_owned()))), ..Default::default() };
    let mut app = app_with(2, services);
    make_dirty(&mut app, 0);
    let mut h = harness(app);
    h.get_by_label("Close Presentation1").click();
    h.run_steps(3);
    h.get_by_label("Save").click();
    h.run_steps(3);
    assert_eq!(*saved.borrow(), ["Presentation1.pptx"], "the tab's presentation was saved, not the active one");
    assert_eq!(titles(h.state()), ["Presentation2"]);
    assert!(!h.state().session.active().unwrap().is_dirty());
}

#[test]
fn the_limit_holds_on_every_way_to_open() {
    let mut h = harness(app_with(MAX_DOCUMENTS, Services::default()));
    h.state_mut().ui.language = "vi".into();
    h.run_steps(2);
    let toast = |h: &Harness<'_, SlideApp>| h.state().toast.as_ref().map(|t| t.text.clone());
    let msg = deckcraft_engine::EngineError::TooManyDocuments.to_string();

    // New (ribbon, menus, File › New).
    assert!(h.state_mut().run("file.new", json!({})).is_err());
    assert_eq!(toast(&h), Some(msg.clone()));
    h.run_steps(2);
    assert!(h.query_by_label_contains("Chỉ mở được tối đa 10 tài liệu.").is_some(), "said in Vietnamese");
    h.state_mut().toast = None;

    // Ctrl+N.
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
    h.run_steps(2);
    assert_eq!(toast(&h), Some(msg.clone()));
    h.state_mut().toast = None;

    // A dropped file, the web shell's open, the browser's file picker (all through the inbox).
    h.state_mut().open_or_insert_bytes("Dropped.pptx", &pptx_bytes());
    assert_eq!(toast(&h), Some(msg.clone()));
    h.state_mut().toast = None;

    // A desktop path.
    assert!(h.state_mut().open_path("/no/such/file.pptx").is_err());
    assert_eq!(toast(&h), Some(msg.clone()));
    h.state_mut().toast = None;

    // A draft the store hands back.
    let deck = deckcraft_engine::cmd::file::save_bytes(&deckcraft_model::defaults::new_presentation(None), crate::drafts::FORMAT).unwrap();
    let mut drafts = crate::drafts::Drafts::default();
    assert!(drafts.restore(h.state_mut(), "k", "Draft.pptx", &deck, 0.0).is_err());
    assert_eq!(toast(&h), Some(msg.clone()));

    assert_eq!(h.state().session.documents().len(), MAX_DOCUMENTS);
    assert!(crate::i18n::has(crate::i18n::Lang::from_code("vi").unwrap(), &msg));

    // Closing one makes room.
    h.state_mut().run("file.close", json!({})).unwrap();
    h.state_mut().open_or_insert_bytes("Dropped.pptx", &pptx_bytes());
    assert_eq!(h.state().session.documents().len(), MAX_DOCUMENTS);
    assert_eq!(active_title(h.state()), "Dropped");
}

#[test]
fn several_drafts_restore_as_several_tabs() {
    let deck = deckcraft_engine::cmd::file::save_bytes(&deckcraft_model::defaults::new_presentation(None), crate::drafts::FORMAT).unwrap();
    let mut app = app_with(1, Services::default());
    let mut drafts = crate::drafts::Drafts::default();
    for (key, name) in [("k-1", "Một.pptx"), ("k-2", "Hai.pptx")] {
        drafts.restore(&mut app, key, name, &deck, 0.0).unwrap();
    }
    let h = harness(app);
    assert_eq!(titles(h.state()), ["Presentation1", "Một", "Hai"]);
    assert!(h.query_by_label("Một").is_some() && h.query_by_label("Hai").is_some());
    let inputs = h.state().draft_inputs();
    assert_eq!(inputs.iter().filter(|d| d.unsaved).count(), 2, "both restored drafts are drafted, not only the active one");
}

#[test]
fn each_presentation_keeps_its_zoom() {
    let mut h = harness(app_with(2, Services::default()));
    h.state_mut().run("view.zoom", json!({"percent": 200})).unwrap();
    h.run_steps(2);
    h.get_by_label("Presentation1").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.zoom, None, "the other presentation is still fitted");
    h.state_mut().run("view.zoom", json!({"percent": 50})).unwrap();
    // Switched by command (Ctrl+`, the control channel) rather than a click.
    h.state_mut().run("window.next", json!({"index": 1})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().ui.zoom, Some(2.0));
    h.get_by_label("Presentation1").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.zoom, Some(0.5));
}

/// Renders the tab row with 1, 2 and 10 presentations in Vietnamese into `DECKCRAFT_SHOTS`
/// (run with `--ignored`; needs a GPU or software wgpu adapter).
#[test]
#[ignore]
fn doc_tabs_screenshots() {
    let Some(dir) = std::env::var_os("DECKCRAFT_SHOTS").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let names = [
        "Báo cáo tài chính quý III năm 2026 — bản trình bày cho Hội đồng quản trị.pptx",
        "Kế hoạch thi công.pptx",
        "Đào tạo an toàn lao động.pptx",
        "Nghiệm thu giai đoạn 2.pptx",
        "Dự toán.pptx",
        "Họp giao ban tuần 42.pptx",
        "Giới thiệu công ty.pptx",
        "Hồ sơ năng lực ERPCons.pptx",
        "Tiến độ dự án cầu Nhật Tân mở rộng.pptx",
    ];
    for (n, file) in [(1, "w7a-1doc-vi.png"), (2, "w7a-2docs-vi.png"), (10, "w7a-10docs-vi.png")] {
        let mut app = app_with(1, Services::default());
        app.ui.language = "vi".into();
        for name in names.iter().take(n - 1) {
            app.open_or_insert_bytes(name, &pptx_bytes());
        }
        if n > 1 {
            make_dirty(&mut app, 1);
            app.session.set_active(0);
        }
        if n == 10 {
            make_dirty(&mut app, 4);
            assert!(app.run("file.new", json!({})).is_err());
        }
        let mut h = harness(app);
        h.run_steps(4);
        match h.render() {
            Ok(img) => img.save(dir.join(file)).unwrap(),
            Err(e) => eprintln!("no renderer: {e}"),
        }
    }
}
