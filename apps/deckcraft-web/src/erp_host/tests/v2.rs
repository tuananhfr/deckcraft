//! `erpHost` v2: view and comment modes, `setMode`, `canDownload`, `preserveView` and `setUser`.

use super::*;

fn png() -> Vec<u8> {
    let doc = deckcraft_engine::model::defaults::new_presentation(None);
    deckcraft_engine::render::render_slide(&doc, 0, &deckcraft_engine::render::RenderOpts { scale: 0.05, ..Default::default() }).to_png()
}

impl World {
    fn open_as(&mut self, doc_id: &str, mode: Mode, can_download: bool) -> u64 {
        let mut r = request(doc_id, 1);
        r.mode = mode;
        r.can_download = can_download;
        self.opens.borrow_mut().push(r.into());
        let now = self.clock.get();
        self.frame(now);
        self.app.session.active().unwrap().uid
    }

    fn set_mode(&mut self, doc_id: &str, mode: Mode, can_download: bool) {
        self.opens.borrow_mut().push(Request::SetMode { doc_id: doc_id.into(), mode, can_download });
        let now = self.clock.get();
        self.frame(now);
    }

    fn flushed_at_once(&self) -> bool {
        let resolved = Rc::new(Cell::new(false));
        let flag = resolved.clone();
        self.host.docs.flush(Box::new(move || flag.set(true)));
        resolved.get()
    }

    fn undo_len(&self) -> usize {
        self.app.session.active().unwrap().history.undo.len()
    }

    fn status(&self) -> Option<&str> {
        self.app.status.as_ref().map(|s| s.0.as_str())
    }
}

#[test]
fn view_and_comment_open_read_only_and_never_save() {
    for mode in [Mode::View, Mode::Comment] {
        let mut w = World::new();
        let uid = w.open_as("d1", mode, true);
        assert_eq!(w.calls.0.borrow().opened, vec![("d1".into(), true, None)]);
        assert!(!w.app.editable(), "{mode:?}");
        let doc = w.app.session.active().unwrap().doc.clone();
        // One per kind of entry point; the engine's own tests go through every command.
        assert!(w.app.run("slide.new", json!({})).is_err());
        assert!(w.app.run("format.bold", json!({})).is_err());
        assert!(w.app.run("edit.paste", json!({"text": "pasted"})).is_err());
        assert!(w.app.run("edit.undo", json!({})).is_err());
        assert!(w.app.run("app.insertPictureDialog", json!({})).is_err());
        assert!(!w.app.open_or_insert_bytes("dropped.png", &png()), "a dropped picture is refused");
        assert_eq!(w.app.session.type_text("x").unwrap()["handled"], false);
        assert!(std::sync::Arc::ptr_eq(&doc, &w.app.session.active().unwrap().doc), "{mode:?} changed the document");
        // Comment mode is view-only until comments on the web come, and says so.
        let notice = match mode {
            Mode::Comment => "Comments on the web are coming later; for now you can only view.",
            _ => "You can only view this presentation.",
        };
        assert_eq!(w.status(), Some(notice));
        // Ctrl+S says so and sends nothing; neither do AutoSave and flush.
        w.app.status = None;
        w.app.request_save();
        assert_eq!(w.status(), Some(notice));
        w.frame(100_000.0);
        assert!(w.flushed_at_once());
        assert!(w.saves().is_empty());
        assert!(w.dirty_calls().is_empty(), "dirty stays false");
        assert_eq!(w.app.save_state(uid), None);
    }
}

#[test]
fn set_mode_both_ways_keeps_the_document_and_its_undo_history() {
    let mut w = World::new();
    let uid = w.open("d1");
    w.edit(0.0);
    w.frame(IDLE_MS);
    w.answer(0, Outcome::Saved { version: Some(2) });
    w.edit(IDLE_MS + 10.0);
    let (doc, undo) = (w.app.session.active().unwrap().doc.clone(), w.undo_len());
    // The shell flushes before downgrading a dirty document; the editor never drops the edits.
    w.set_mode("d1", Mode::View, true);
    assert!(!w.app.editable());
    assert!(std::sync::Arc::ptr_eq(&doc, &w.app.session.active().unwrap().doc));
    assert_eq!(w.undo_len(), undo);
    assert!(w.dirty(), "edits made before are kept");
    assert!(w.app.run("edit.undo", json!({})).is_err());
    w.frame(100_000.0);
    assert_eq!(w.saves().len(), 1, "no AutoSave while viewing");
    w.set_mode("d1", Mode::Edit, true);
    assert!(w.app.editable());
    assert_eq!(w.undo_len(), undo);
    w.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(w.undo_len(), undo - 1, "undo goes on from where it was");
    w.frame(200_000.0);
    assert_eq!(w.saves().len(), 2, "AutoSave runs again");
    assert_eq!(w.app.session.active().unwrap().uid, uid);
    // setMode for a document that isn't open does nothing.
    w.set_mode("other", Mode::View, false);
    assert!(w.app.editable());
}

#[test]
fn going_read_only_stops_autosave_at_once_and_flush_still_saves_the_edits_from_before() {
    let mut w = World::new();
    let uid = w.open("d1");
    w.edit(0.0);
    // As the bridge does inside the shell's `setMode`, before the next frame.
    w.host.docs.set_read_only(uid, true);
    w.frame(IDLE_MS);
    assert!(w.saves().is_empty(), "no AutoSave");
    w.app.request_save();
    assert!(w.saves().is_empty(), "no Save");
    assert!(w.flushed_at_once(), "handed over at once");
    assert_eq!(w.saves(), vec![("d1".into(), w.revision(), Reason::Flush)]);
}

#[test]
fn without_download_save_as_export_print_share_and_copy_are_gone() {
    for mode in [Mode::Edit, Mode::View, Mode::Comment] {
        let mut w = World::new();
        w.open_as("d1", mode, false);
        assert!(!w.app.can_download());
        for id in ["app.saveAsDialog", "app.exportDialog", "app.print", "app.share", "edit.copy", "edit.cut"] {
            assert!(!deckcraft_ui_egui::ribbon::enabled(&w.app, id), "{id} offered with {mode:?}");
            assert!(w.app.run(id, json!({})).is_err(), "{id} ran with {mode:?}");
        }
        assert!(!w.app.export_as("pdf", json!({})));
        assert!(!w.app.print_pdf(json!({})));
        assert!(!w.app.can_share());
        assert!(w.app.run("app.backstage", json!({"page": "export"})).is_err());
        assert_eq!(w.status(), Some("Downloading, printing and copying are turned off for this presentation."));
    }
    let mut w = World::new();
    w.open_as("d1", Mode::View, true);
    for id in ["app.saveAsDialog", "app.exportDialog", "app.print", "edit.copy"] {
        assert!(deckcraft_ui_egui::ribbon::enabled(&w.app, id), "{id} is offered to a viewer who may download");
    }
    w.set_mode("d1", Mode::View, false);
    assert!(!deckcraft_ui_egui::ribbon::enabled(&w.app, "app.print"), "setMode changes canDownload too");
}

/// A deck of four slides, the last three with a rectangle; `text` goes on the last one's title.
fn deck(text: &str) -> Vec<u8> {
    let mut s = Session::with_new();
    for _ in 0..3 {
        s.execute("slide.new", &json!({})).unwrap();
        s.execute("shape.insert", &json!({"preset": "rect", "rect": [10, 10, 50, 50]})).unwrap();
    }
    let title = s.active().unwrap().current_slide().unwrap().shapes[0].id.0;
    s.execute("text.set", &json!({"id": title, "text": text})).unwrap();
    deckcraft_engine::cmd::file::save_bytes(&s.active().unwrap().doc, "pptx").unwrap()
}

#[test]
fn preserve_view_reloads_in_the_same_tab_keeping_the_slide_and_selection() {
    let mut w = World::new();
    let mut first = request("d1", 1);
    first.mode = Mode::View;
    first.bytes = Some(deck("old"));
    w.opens.borrow_mut().push(first.into());
    w.frame(0.0);
    let uid = w.app.session.active().unwrap().uid;
    w.app.run("slide.go", json!({"index": 2})).unwrap();
    let shape = w.app.session.active().unwrap().current_slide().unwrap().shapes.last().unwrap().id;
    w.app.run("edit.select", json!({"ids": [shape.0]})).unwrap();
    // Another tab is active when the newer version arrives.
    w.app.session.set_active(0);
    let mut newer = request("d1", 5);
    newer.mode = Mode::View;
    newer.preserve_view = true;
    newer.title = "Báo cáo quý (sửa).pptx".into();
    newer.bytes = Some(deck("new"));
    w.opens.borrow_mut().push(newer.into());
    w.frame(0.0);
    assert_eq!(w.calls.0.borrow().opened.last().unwrap(), &("d1".into(), true, None));
    assert_eq!(w.app.session.documents().len(), 2, "no new tab");
    assert_eq!(w.app.session.active_index(), Some(0), "the active tab stays");
    let d = w.app.session.documents().iter().find(|d| d.uid == uid).unwrap();
    assert_eq!((d.selection.slide, d.selection.shapes.clone()), (2, vec![shape]));
    assert_eq!(d.title(), "Báo cáo quý (sửa)");
    assert_eq!(d.doc.slides.last().unwrap().title(), "new", "the newer content");
    assert!(!d.is_dirty() && !d.access.editable);
    assert_eq!(w.host.docs.version(uid), Some(5));
    assert!(w.calls.0.borrow().closed.is_empty());
    assert!(w.saves().is_empty());
}

#[test]
fn set_user_names_new_comments_in_edit_mode() {
    let mut w = World::new();
    w.app.session.prefs.author = "Máy này".into();
    w.opens.borrow_mut().push(Request::SetUser(Some("Trần Thị Bình".into())));
    w.open("d1");
    w.app.run("comment.add", json!({"text": "Xem lại"})).unwrap();
    w.opens.borrow_mut().push(Request::SetUser(None));
    w.frame(0.0);
    w.app.run("comment.add", json!({"text": "Ẩn danh"})).unwrap();
    let authors: Vec<String> = w.app.session.active().unwrap().current_slide().unwrap().comments.iter().map(|c| c.author.clone()).collect();
    assert_eq!(authors, ["Trần Thị Bình", "Máy này"]);
}

/// One frame of the interface, as the page draws it, with `events` as input.
fn draw(w: &mut World, ctx: &egui::Context, events: Vec<egui::Event>) {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
    let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| {
        w.app.logic(ui.ctx());
        w.app.ui(ui);
    });
    // No renderer here to take the textures.
    output.textures_delta.clear();
}

#[test]
fn downgrading_keeps_the_notes_being_typed_as_one_undo_step() {
    let mut w = World::new();
    w.open("d1");
    let ctx = egui::Context::default();
    for _ in 0..3 {
        draw(&mut w, &ctx, vec![]);
    }
    ctx.memory_mut(|m| m.request_focus(egui::Id::new("notes_edit")));
    draw(&mut w, &ctx, vec![]);
    draw(&mut w, &ctx, vec![egui::Event::Text("Speaker notes".into())]);
    let notes = |w: &World| w.app.session.active().unwrap().doc.slides[0].notes_text();
    assert_eq!(notes(&w), "", "typed into the notes field, not yet committed");
    let undo = w.undo_len();
    w.set_mode("d1", Mode::View, true);
    assert!(!w.app.editable());
    assert_eq!(notes(&w), "Speaker notes", "the fork never drops an edit on its own");
    assert!(w.dirty());
    assert_eq!(w.undo_len(), undo + 1);
    assert_eq!(w.dirty_calls(), vec![("d1".into(), true)], "the shell learns there is something unsaved");
    w.frame(100_000.0);
    assert!(w.saves().is_empty(), "no AutoSave while viewing");
    // The shell flushes a document left dirty by a downgrade.
    assert!(w.flushed_at_once());
    assert_eq!(w.saves(), vec![("d1".into(), w.revision(), Reason::Flush)]);
    let bytes = w.calls.0.borrow().saves[0].bytes.clone();
    let saved = deckcraft_engine::cmd::file::open_presentation("d1.pptx", &bytes).unwrap();
    assert_eq!(saved.slides[0].notes_text(), "Speaker notes");
    w.answer(0, Outcome::Saved { version: Some(2) });
    assert!(!w.dirty());
    assert_eq!(w.dirty_calls().last(), Some(&("d1".into(), false)));
    w.frame(200_000.0);
    assert!(w.flushed_at_once());
    assert_eq!(w.saves().len(), 1, "saved: nothing more is sent");
    w.set_mode("d1", Mode::Edit, true);
    w.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(notes(&w), "");
}

#[test]
fn comment_mode_reads_in_vietnamese_as_the_owner_asked() {
    deckcraft_ui_egui::i18n::set_current(deckcraft_ui_egui::i18n::Lang::from_pref("vi"));
    let mut w = World::new();
    w.open_as("d1", Mode::Comment, true);
    assert!(w.app.run("slide.new", json!({})).is_err());
    assert_eq!(w.status(), Some("Nhận xét trên web sẽ có sau, hiện chỉ xem được"));
    // setMode to view says the usual notice, and back to comment the comment one.
    w.set_mode("d1", Mode::View, true);
    assert!(w.app.run("slide.new", json!({})).is_err());
    assert_eq!(w.status(), Some("Bạn chỉ có quyền xem bản trình bày này."));
    w.set_mode("d1", Mode::Comment, true);
    assert!(w.app.run("slide.new", json!({})).is_err());
    assert_eq!(w.status(), Some("Nhận xét trên web sẽ có sau, hiện chỉ xem được"));
    deckcraft_ui_egui::i18n::set_current(deckcraft_ui_egui::i18n::Lang::EN);
}

#[test]
fn with_erp_host_v2_the_shells_share_is_the_only_one() {
    let mut w = World::with_v2(true);
    w.open("d1");
    assert!(!w.app.can_share(), "no Share… of the app's own next to the shell's");
    assert!(w.app.run("app.share", json!({})).is_err());
    // v1 (and the free web build, which has no erpHost) keep it.
    let mut w = World::with_v2(false);
    w.open("d1");
    assert!(w.app.can_share());
}
