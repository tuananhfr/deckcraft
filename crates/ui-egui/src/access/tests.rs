//! A read-only presentation in the interface: typing, input methods, paste, cut and keys change
//! nothing, editing commands are greyed out while viewing ones stay, Save only says why, and
//! without download Save As, Export, Print and copying are gone.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use deckcraft_engine::{Access, Session};
use egui::{Event, ImeEvent, Key, Modifiers};
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::backstage::{Nav, Page, offered_entries};
use crate::ribbon::{enabled, offered};
use crate::{Services, SlideApp};

fn harness(access: Access) -> egui_kittest::Harness<'static, SlideApp> {
    let mut session = Session::with_new();
    let uid = session.active().unwrap().uid;
    session.set_access(uid, access);
    let services = Services { download: Some(Box::new(|_: &str, _: &[u8]| {})), ..Default::default() };
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

fn doc(app: &SlideApp) -> Arc<deckcraft_engine::model::Presentation> {
    app.session.active().unwrap().doc.clone()
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

#[test]
fn keys_typing_input_methods_paste_and_cut_change_nothing() {
    let mut h = harness(Access::read_only(true));
    let title = h.state().session.active().unwrap().doc.slides[0].shapes[0].id.0;
    h.state_mut().run("edit.select", json!({"ids": [title]})).unwrap();
    h.run_steps(1);
    let before = doc(h.state());
    let events = [
        Event::Text("x".into()),
        Event::Ime(ImeEvent::Preedit { text: "â".into(), active_range_chars: None }),
        Event::Ime(ImeEvent::Commit("â".into())),
        Event::Paste("pasted".into()),
        Event::Cut,
        key(Key::Delete, Modifiers::NONE),
        key(Key::ArrowRight, Modifiers::NONE),
        key(Key::D, Modifiers::COMMAND),
        key(Key::Z, Modifiers::COMMAND),
        key(Key::S, Modifiers::COMMAND),
    ];
    for e in events.clone() {
        h.event(e);
        h.run_steps(1);
    }
    // And again after trying to put a caret in the title.
    assert!(h.state_mut().run("text.edit", json!({"id": title})).is_err(), "no caret read-only");
    for e in events {
        h.event(e);
        h.run_steps(1);
    }
    let app = h.state();
    assert!(Arc::ptr_eq(&before, &doc(app)), "a read-only presentation changed");
    assert!(app.session.active().unwrap().history.undo.is_empty());
    assert!(app.session.active().unwrap().composition.is_none());
    assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some("You can only view this presentation."), "the usual read-only notice");
}

#[test]
fn editing_commands_are_greyed_out_and_viewing_ones_stay() {
    let mut h = harness(Access::read_only(true));
    let app = h.state_mut();
    for id in ["slide.new", "edit.paste", "edit.undo", "format.bold", "shape.insert", "insert.table", "app.insertPictureDialog", "comment.add"] {
        assert!(!enabled(app, id), "{id} is offered read-only");
    }
    for id in ["view.zoomIn", "show.start", "show.fromStart", "app.find", "edit.copy", "app.saveAsDialog", "app.print", "view.outline"] {
        assert!(enabled(app, id), "{id} is not offered read-only");
    }
    let (top, _) = offered_entries(app, true);
    assert!(!top.contains(&Nav::Save) && top.contains(&Nav::Page(Page::SaveAs)) && top.contains(&Nav::Page(Page::Print)));
    // Replace opens Find, which still searches.
    app.run("app.replace", json!({})).unwrap();
    assert!(app.find.visible && !app.find.replace);
    app.status = None;
    app.save();
    assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some("You can only view this presentation."));
}

#[test]
fn without_download_the_file_tab_offers_no_save_as_print_or_export() {
    let mut h = harness(Access::read_only(false));
    let app = h.state_mut();
    let (top, _) = offered_entries(app, true);
    for page in [Page::SaveAs, Page::Print, Page::Export] {
        assert!(!top.contains(&Nav::Page(page)), "{page:?}");
    }
    assert!(top.contains(&Nav::Page(Page::Info)) && top.contains(&Nav::Close));
    for id in ["edit.copy", "app.saveAsDialog", "app.exportDialog", "app.print"] {
        assert!(!enabled(app, id), "{id}");
    }
    let before = app.status.clone();
    app.save_as_dialog("pptx");
    assert_ne!(app.status, before, "Save As says why it does nothing");
    // The File tab open on Print when downloading is turned off falls back to Info.
    let uid = app.session.active().unwrap().uid;
    app.session.set_access(uid, Access::default());
    app.run("app.backstage", json!({"page": "print"})).unwrap();
    app.session.set_access(uid, Access::read_only(false));
    h.run_steps(2);
    assert_eq!(h.state().backstage.as_ref().map(|b| b.page), Some(Page::Info));
}

#[test]
fn copying_follows_can_download() {
    let mut h = harness(Access::read_only(false));
    let title = h.state().session.active().unwrap().doc.slides[0].shapes[0].id.0;
    h.state_mut().run("edit.select", json!({"ids": [title]})).unwrap();
    h.event(Event::Copy);
    h.run_steps(1);
    assert!(h.state().session.clipboard.shapes.is_empty(), "nothing copied without download");
    let uid = h.state().session.active().unwrap().uid;
    h.state_mut().session.set_access(uid, Access::read_only(true));
    h.event(Event::Copy);
    h.run_steps(1);
    assert!(!h.state().session.clipboard.shapes.is_empty(), "a viewer who may download can copy");
}

#[test]
fn dialogs_and_panes_that_change_the_presentation_are_not_offered() {
    let mut h = harness(Access::read_only(false));
    let app = h.state_mut();
    for dialog in ["table", "spelling", "comment", "customShow", "headerFooter", "export"] {
        assert!(!offered(app, "app.dialog", &json!({"id": dialog})), "{dialog} is offered read-only");
        assert!(app.run("app.dialog", json!({"id": dialog})).is_err(), "{dialog} opens read-only");
    }
    for dialog in ["accessibility", "zoom", "shortcuts"] {
        assert!(offered(app, "app.dialog", &json!({"id": dialog})), "{dialog} is not offered read-only");
    }
    for pane in ["format", "background", "animation", "designer"] {
        assert!(!offered(app, "view.pane", &json!({"pane": pane})), "{pane} is offered read-only");
    }
    assert!(offered(app, "view.pane", &json!({"pane": "comments"})));
    // A dialog opened while the presentation could still be changed closes when it turns read-only.
    let uid = app.session.active().unwrap().uid;
    app.set_access(uid, Access::default());
    app.run("app.dialog", json!({"id": "table"})).unwrap();
    app.set_access(uid, Access::read_only(true));
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
}

#[test]
fn going_read_only_commits_the_notes_being_typed_as_one_undo_step() {
    let mut h = harness(Access::default());
    let uid = h.state().session.active().unwrap().uid;
    h.ctx.memory_mut(|m| m.request_focus(egui::Id::new("notes_edit")));
    h.run_steps(2);
    h.event(Event::Text("Speaker notes".into()));
    h.run_steps(1);
    let notes = |app: &SlideApp| app.session.active().unwrap().doc.slides[0].notes_text();
    assert_eq!(notes(h.state()), "", "typed, not yet committed");
    let undo = h.state().session.active().unwrap().history.undo.len();
    h.state_mut().set_access(uid, Access::read_only(true));
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.slides[0].notes_text(), "Speaker notes");
    assert!(st.is_dirty());
    assert_eq!(st.history.undo.len(), undo + 1);
    h.run_steps(2);
    assert_eq!(notes(h.state()), "Speaker notes", "the read-only notes show what was typed");
    h.state_mut().set_access(uid, Access::default());
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(notes(h.state()), "");
}

/// Where `shape` of the current slide is drawn, in screen points.
fn on_screen(app: &SlideApp, shape: usize) -> egui::Pos2 {
    let st = app.session.active().unwrap();
    let sh = &st.doc.slides[st.selection.slide].shapes[shape];
    let x = deckcraft_engine::cmd::xfrm_of(&st.doc, &st.selection, sh);
    let r = app.slide_rect.unwrap();
    let k = r.width() / st.doc.slide_size.width as f32;
    r.min + egui::vec2((x.x + x.w / 2.0) as f32, (x.y + x.h / 2.0) as f32) * k
}

fn click(h: &mut egui_kittest::Harness<'_, SlideApp>, at: egui::Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.input_mut().events.extend([Event::PointerMoved(at), button(true)]);
    h.run_steps(1);
    h.input_mut().events.push(button(false));
    h.run_steps(3);
}

#[test]
fn clicking_text_read_only_selects_the_shape_without_a_caret() {
    let mut h = harness(Access::read_only(true));
    let title = h.state().session.active().unwrap().doc.slides[0].shapes[0].id;
    let at = on_screen(h.state(), 0);
    click(&mut h, at);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.selection.shapes, vec![title], "the click selects, to copy");
    assert!(st.selection.text.is_none(), "no caret");
    // The same click on an editable slide puts the caret in the empty title, as before.
    let mut h = harness(Access::default());
    click(&mut h, at);
    assert!(h.state().session.active().unwrap().selection.text.is_some());
}

#[test]
fn read_only_slides_draw_empty_placeholders_as_the_show_does() {
    let draw = |app: &SlideApp| {
        let doc = doc(app);
        let opts = deckcraft_render::RenderOpts { scale: 0.25, edit: crate::canvas::placeholder_prompts(app), ..Default::default() };
        deckcraft_render::render_slide(&doc, 0, &opts)
    };
    let uniform = |img: &deckcraft_render::Image| img.pixels.chunks(4).all(|p| p == &img.pixels[..4]);
    let h = harness(Access::read_only(false));
    assert!(uniform(&draw(h.state())), "no prompt or dashed outline on an empty read-only slide");
    let h = harness(Access::default());
    assert!(!uniform(&draw(h.state())), "an editable slide still prompts");
}

#[test]
fn comment_mode_says_comments_on_the_web_come_later() {
    let mut h = harness(Access::read_only(true));
    h.state_mut().ui.language = "vi".into();
    h.run_steps(1);
    let uid = h.state().session.active().unwrap().uid;
    let app = h.state_mut();
    app.set_read_only_notice(uid, crate::ReadOnlyNotice::CommentsLater);
    assert!(app.run("slide.new", json!({})).is_err());
    assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some("Nhận xét trên web sẽ có sau, hiện chỉ xem được"));
    app.set_read_only_notice(uid, crate::ReadOnlyNotice::View);
    assert!(app.run("slide.new", json!({})).is_err());
    assert_eq!(app.status.as_ref().map(|s| s.0.as_str()), Some("Bạn chỉ có quyền xem bản trình bày này."), "view mode is unchanged");
}

#[test]
fn going_read_only_commits_what_is_typed_in_the_outline_and_the_notes_page() {
    for (view, field_text) in [(crate::ViewMode::Outline, "Outline title"), (crate::ViewMode::NotesPage, "Notes page text")] {
        let mut h = harness(Access::default());
        h.state_mut().ui.view = view;
        h.run_steps(2);
        // The view's first field under the ribbon: slide 1's title, or the page's notes.
        let fields: Vec<_> =
            h.query_all_by_role(egui::accesskit::Role::TextInput).chain(h.query_all_by_role(egui::accesskit::Role::MultilineTextInput)).collect();
        let rects: Vec<egui::Rect> = fields.iter().map(|f| f.rect()).collect();
        let field = fields.into_iter().filter(|f| f.rect().min.y > 120.0).min_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y));
        field.unwrap_or_else(|| panic!("{view:?}: no field under the ribbon among {rects:?}")).focus();
        h.run_steps(2);
        h.event(Event::Text(field_text.into()));
        h.run_steps(1);
        let uid = h.state().session.active().unwrap().uid;
        let before = doc(h.state());
        h.state_mut().set_access(uid, Access::read_only(true));
        let after = doc(h.state());
        assert!(!Arc::ptr_eq(&before, &after), "{view:?}: what was typed is committed");
        assert!(h.state().session.active().unwrap().is_dirty());
        h.state_mut().set_access(uid, Access::default());
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(*doc(h.state()), *before, "{view:?}: one undo step");
    }
}
