//! Closing and reopening presentations and the save status: Reopen Closed Presentation
//! (Ctrl+Shift+T, the tab menu, the File tab), Close All / Close Others and quitting with one
//! prompt for every unsaved presentation, and "Saving…" / "Save failed" in the title bar.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use deckcraft_engine::Session;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::close_many::{After, DIALOG};
use crate::save_state::SaveState;
use crate::{SaveDone, SaveOutcome, Services, SlideApp};

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

fn make_dirty(app: &mut SlideApp, i: usize) {
    let back = app.session.active_index().unwrap();
    app.session.set_active(i);
    app.run("slide.new", json!({})).unwrap();
    app.session.set_active(back);
}

fn dirty(app: &SlideApp, title: &str) -> bool {
    app.session.documents().iter().find(|d| d.title() == title).unwrap().is_dirty()
}

fn uid(app: &SlideApp, title: &str) -> u64 {
    app.session.documents().iter().find(|d| d.title() == title).unwrap().uid
}

fn dialog(h: &Harness<'_, SlideApp>) -> Option<String> {
    h.state().dialog.as_ref().map(|d| d.id.clone())
}

fn disabled(h: &Harness<'_, SlideApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

type PickerCalls = Rc<RefCell<Vec<(String, Option<SaveDone>)>>>;

/// The web: a save picker that stays open until the test ends it.
fn picker_services() -> (Services, PickerCalls) {
    let calls: PickerCalls = Default::default();
    let sink = calls.clone();
    let services = Services {
        save_file: Some(Box::new(move |name: &str, _: &[u8], done: SaveDone| sink.borrow_mut().push((name.to_owned(), Some(done))))),
        download: Some(Box::new(|_: &str, _: &[u8]| {})),
        ..Default::default()
    };
    (services, calls)
}

fn settle(h: &mut Harness<'_, SlideApp>, calls: &PickerCalls, outcome: SaveOutcome) {
    let open: Vec<SaveDone> = calls.borrow_mut().iter_mut().filter_map(|c| c.1.take()).collect();
    assert_eq!(open.len(), 1, "one save picker is open at a time");
    for done in open {
        done(outcome.clone());
    }
    h.run_steps(3);
}

/// The desktop: a save dialog answering with the paths given, in turn (None = Cancel).
fn desktop_services(answers: Vec<Option<String>>) -> (Services, Rc<RefCell<Vec<String>>>) {
    let asked: Rc<RefCell<Vec<String>>> = Default::default();
    let seen = asked.clone();
    let answers = RefCell::new(answers.into_iter());
    let services = Services {
        pick_save: Some(Box::new(move |suggested: &str| {
            seen.borrow_mut().push(suggested.to_owned());
            answers.borrow_mut().next().flatten()
        })),
        ..Default::default()
    };
    (services, asked)
}

fn temp_path(name: &str) -> String {
    std::env::temp_dir().join(format!("deckcraft-closing-{}-{name}.deckcraft", std::process::id())).to_string_lossy().into_owned()
}

#[test]
fn ctrl_shift_t_reopens_a_presentation_closed_without_saving() {
    let mut app = app_with(2, Services::default());
    make_dirty(&mut app, 1);
    let mut h = harness(app);
    h.get_by_label("Close Presentation2").click();
    h.run_steps(3);
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert_eq!(titles(h.state()), ["Presentation1"]);

    h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::T);
    h.run_steps(3);
    assert_eq!(titles(h.state()), ["Presentation1", "Presentation2"], "reopened as a new tab");
    let d = h.state().session.active().unwrap();
    assert_eq!(d.title(), "Presentation2");
    assert!(d.is_dirty(), "with its unsaved changes, still unsaved");
    assert_eq!(d.doc.slides.len(), 2);

    // Listed in the Keyboard Shortcuts (F1) with the other commands.
    let rows = crate::shortcuts::rows();
    let row = rows.iter().find(|r| r.id == "file.reopenClosed").expect("listed");
    assert_eq!(row.keys(), crate::ribbon::pretty_shortcut("Cmd+Shift+T"));
}

#[test]
fn the_tab_menu_closes_others_or_all_and_reopens() {
    let mut h = harness(app_with(3, Services::default()));
    h.get_by_label("Presentation1").click_button(egui::PointerButton::Secondary);
    h.run_steps(3);
    assert!(h.query_by_label("Close Others").is_some() && h.query_by_label("Close All").is_some());
    assert!(disabled(&h, "Reopen Closed Presentation"), "nothing closed yet");
    h.get_by_label("Close Others").click();
    h.run_steps(3);
    assert_eq!(titles(h.state()), ["Presentation1"]);
    assert!(h.state().dialog.is_none(), "nothing unsaved: no prompt");

    // One presentation: no tabs, so the File tab offers it.
    h.state_mut().run("app.home", json!({"on": true})).unwrap();
    h.run_steps(3);
    assert!(!disabled(&h, "Reopen Closed Presentation"));
    h.get_by_label("Reopen Closed Presentation").click();
    h.run_steps(3);
    assert_eq!(titles(h.state()), ["Presentation1", "Presentation3"], "the last one closed comes back first");

    h.get_by_label("Presentation3").click_button(egui::PointerButton::Secondary);
    h.run_steps(3);
    assert!(!disabled(&h, "Reopen Closed Presentation"));
    h.get_by_label("Close All").click();
    h.run_steps(3);
    assert!(h.state().session.documents().is_empty());
    // The start screen, after closing the last one.
    h.get_by_label("Reopen Closed Presentation").click();
    h.run_steps(3);
    assert_eq!(titles(h.state()), ["Presentation3"]);
}

#[test]
fn close_all_asks_once_for_every_unsaved_presentation() {
    let mut app = app_with(3, Services::default());
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 2);
    let mut h = harness(app);
    h.state_mut().run("app.closeAll", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(dialog(&h).as_deref(), Some(DIALOG));
    assert!(h.query_by_label("Do you want to save the changes to these presentations?").is_some());
    assert!(h.query_by_label_contains("“Presentation1”").is_some() && h.query_by_label_contains("“Presentation3”").is_some());
    assert!(h.query_by_label_contains("“Presentation2”").is_none(), "only the unsaved ones are listed");
    assert!(h.query_by_label("Save All").is_some());

    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(titles(h.state()).len(), 3, "Cancel closes nothing");
    assert!(dialog(&h).is_none() && h.state().close_batch.is_none());

    h.state_mut().run("app.closeAll", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("Don't Save").click();
    h.run_steps(3);
    assert!(h.state().session.documents().is_empty(), "Don't Save closes them all");
    assert_eq!(h.state().session.closed.len(), 3);
    h.state_mut().run("file.reopenClosed", json!({})).unwrap();
    assert!(dirty(h.state(), "Presentation3"), "and they reopen unsaved");
}

#[test]
fn esc_on_the_prompt_closes_nothing() {
    let mut app = app_with(2, Services::default());
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 1);
    let mut h = harness(app);
    let keep = uid(h.state(), "Presentation1");
    h.state_mut().run("app.closeOthers", json!({"index": 0})).unwrap();
    h.run_steps(3);
    assert_eq!(dialog(&h).as_deref(), Some(DIALOG));
    assert!(h.query_by_label_contains("“Presentation2”").is_some());
    assert!(h.query_by_label("Save").is_some(), "one unsaved: Save, not Save All");
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert_eq!(titles(h.state()), ["Presentation1", "Presentation2"]);
    assert!(h.state().close_batch.is_none());
    assert_eq!(h.state().session.documents()[0].uid, keep);
}

/// The web: each save waits for its picker, and the next one opens only on a click of its own.
#[test]
fn save_all_on_the_web_saves_one_after_another() {
    let (services, calls) = picker_services();
    let mut app = app_with(3, services);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 2);
    let mut h = harness(app);
    h.state_mut().run("app.closeAll", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("Save All").click();
    h.run_steps(3);
    assert_eq!(calls.borrow().len(), 1, "one picker at a time");
    assert_eq!(calls.borrow()[0].0, "Presentation1.pptx");
    assert!(disabled(&h, "Saving…"));
    assert!(h.query_by_label_contains("“Presentation1” — Saving…").is_some());
    assert_eq!(h.state().save_state(uid(h.state(), "Presentation1")), Some(SaveState::Saving));

    settle(&mut h, &calls, SaveOutcome::Saved);
    assert!(!dirty(h.state(), "Presentation1"));
    assert_eq!(calls.borrow().len(), 1, "the next picker waits for a click");
    assert_eq!(titles(h.state()).len(), 3, "nothing is closed before all are saved");
    h.get_by_label("Save “Presentation3”").click();
    h.run_steps(3);
    assert_eq!(calls.borrow().len(), 2);
    assert_eq!(calls.borrow()[1].0, "Presentation3.pptx");
    settle(&mut h, &calls, SaveOutcome::Saved);
    assert!(h.state().session.documents().is_empty(), "all saved: all closed");
    assert!(dialog(&h).is_none());
}

#[test]
fn cancel_or_failure_in_a_picker_stops_and_closes_nothing() {
    for outcome in [SaveOutcome::Cancelled, SaveOutcome::Failed("disk full".into())] {
        let (services, calls) = picker_services();
        let mut app = app_with(3, services);
        make_dirty(&mut app, 0);
        make_dirty(&mut app, 1);
        let mut h = harness(app);
        h.state_mut().run("app.closeAll", json!({})).unwrap();
        h.run_steps(3);
        h.get_by_label("Save All").click();
        h.run_steps(3);
        settle(&mut h, &calls, outcome.clone());
        assert_eq!(titles(h.state()).len(), 3, "{outcome:?}: nothing is closed");
        assert!(dirty(h.state(), "Presentation1") && dirty(h.state(), "Presentation2"));
        assert_eq!(calls.borrow().len(), 1, "{outcome:?}: the rest aren't asked for");
        assert!(dialog(&h).is_none());
        let state = h.state().save_state(uid(h.state(), "Presentation1"));
        match outcome {
            SaveOutcome::Failed(_) => assert_eq!(state, Some(SaveState::Failed("disk full".into()))),
            _ => assert_eq!(state, None, "cancelling isn't a failure"),
        }
    }
}

/// The desktop: Save All runs every save dialog in a row, each finishing before the next.
#[test]
fn quitting_saves_all_in_turn_or_stops_at_a_cancel() {
    let (a, b) = (temp_path("a"), temp_path("b"));
    let (services, asked) = desktop_services(vec![Some(a.clone()), None]);
    let mut app = app_with(3, services);
    make_dirty(&mut app, 0);
    make_dirty(&mut app, 2);
    let mut h = harness(app);
    let all: Vec<u64> = h.state().session.documents().iter().map(|d| d.uid).collect();
    h.state_mut().close_documents(all, After::Quit);
    h.run_steps(3);
    assert!(h.query_by_label_contains("“Presentation1”").is_some() && h.query_by_label_contains("“Presentation3”").is_some());
    h.get_by_label("Save All").click();
    h.run_steps(3);
    assert_eq!(asked.borrow().len(), 2, "the second save dialog came after the first was written");
    assert!(std::path::Path::new(&a).exists());
    assert_eq!(h.state().session.documents().len(), 3, "the second was cancelled: nothing closed");
    assert!(!h.state().quit_confirmed);
    assert!(dialog(&h).is_none());

    let (services, asked) = desktop_services(vec![Some(b.clone())]);
    h.state_mut().services = services;
    let all: Vec<u64> = h.state().session.documents().iter().map(|d| d.uid).collect();
    h.state_mut().close_documents(all, After::Quit);
    h.run_steps(3);
    assert!(h.query_by_label_contains("“Presentation1”").is_none(), "already saved: not listed");
    h.get_by_label("Save").click();
    h.run_steps(3);
    assert_eq!(asked.borrow().len(), 1);
    assert!(h.state().session.documents().is_empty());
    assert!(h.state().quit_confirmed, "everything saved: the window closes");
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
}

#[test]
fn the_title_says_saving_then_save_failed_until_an_edit() {
    let (services, calls) = picker_services();
    let mut h = harness(app_with(1, services));
    h.state_mut().ui.language = "vi".into();
    h.state_mut().run("slide.new", json!({})).unwrap();
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    h.run_steps(2);
    assert_eq!(calls.borrow().len(), 1, "the web's picker opens within the key press");
    assert_eq!(crate::ribbon::document_title(h.state()), "Presentation1 — Đang lưu…");

    settle(&mut h, &calls, SaveOutcome::Failed("disk full".into()));
    assert_eq!(crate::ribbon::document_title(h.state()), "Presentation1 — Lưu không thành công");
    // The reason, on hovering (or tapping) the title.
    let reason = "Không lưu được “Presentation1”: disk full";
    assert!(h.query_by_label(reason).is_some());
    h.get_by_label(reason).click();
    h.run_steps(2);
    assert_eq!(h.state().toast.as_ref().map(|s| s.text.as_str()), Some(reason), "a tap shows it in full");

    h.state_mut().toast = None;
    h.state_mut().status = None;
    h.state_mut().run("slide.new", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(crate::ribbon::document_title(h.state()), "Presentation1 — Đã sửa", "an edit ends the failure");
    assert!(h.query_by_label(reason).is_none());
}

#[test]
fn a_failure_lasts_until_the_next_successful_save() {
    let (services, calls) = picker_services();
    let mut h = harness(app_with(1, services));
    h.state_mut().run("slide.new", json!({})).unwrap();
    h.state_mut().save();
    settle(&mut h, &calls, SaveOutcome::Failed("denied".into()));
    let id = uid(h.state(), "Presentation1");
    assert_eq!(h.state().save_state(id), Some(SaveState::Failed("denied".into())));
    h.state_mut().save();
    h.run_steps(1);
    assert_eq!(h.state().save_state(id), Some(SaveState::Saving), "a new save shows over the failure");
    settle(&mut h, &calls, SaveOutcome::Saved);
    assert_eq!(h.state().save_state(id), None);
    assert_eq!(crate::ribbon::document_title(h.state()), "Presentation1");
}

/// The desktop writes the file the frame after Ctrl+S, so "Saving…" is on screen while it does.
#[test]
fn desktop_save_shows_saving_before_it_writes() {
    let path = temp_path("ctrl-s");
    let mut app = app_with(1, Services::default());
    app.session.active_mut().unwrap().path = Some(path.clone());
    let mut h = harness(app);
    h.state_mut().run("slide.new", json!({})).unwrap();
    // What Ctrl+S and the Save button do (a key press takes several frames in the harness).
    h.state_mut().request_save();
    h.run_steps(1);
    let id = h.state().session.active().unwrap().uid;
    assert_eq!(h.state().save_state(id), Some(SaveState::Saving));
    assert!(h.state().session.active().unwrap().is_dirty(), "not written yet");
    h.run_steps(2);
    assert_eq!(h.state().save_state(id), None);
    assert!(!h.state().session.active().unwrap().is_dirty());
    assert!(std::path::Path::new(&path).exists());
    let _ = std::fs::remove_file(&path);

    // A path in a missing folder: the failure and why.
    h.state_mut().session.active_mut().unwrap().path = Some("/erpcons-no-such-dir/x.deckcraft".into());
    h.state_mut().run("slide.new", json!({})).unwrap();
    h.key_press_modifiers(Modifiers::COMMAND, Key::S);
    h.run_steps(3);
    assert!(matches!(h.state().save_state(id), Some(SaveState::Failed(_))));
    assert!(crate::ribbon::document_title(h.state()).ends_with("— Save failed"));
    assert!(h.query_by_label_contains("Couldn't save “x”").is_some());
}

/// Dirty presentations are marked as they were, before and after a reopen (pointer identity).
#[test]
fn a_reopened_saved_presentation_is_not_marked_edited() {
    let mut h = harness(app_with(2, Services::default()));
    h.state_mut().run("file.close", json!({})).unwrap();
    h.state_mut().run("file.reopenClosed", json!({})).unwrap();
    h.run_steps(2);
    let d = h.state().session.active().unwrap();
    assert!(Arc::ptr_eq(&d.doc, &d.saved_doc));
    assert_eq!(crate::ribbon::document_title(h.state()), "Presentation2");
}

/// Renders the tab menu, the Close All prompt (asking, saving, waiting for the next click), the
/// title's save states and the start screen's Reopen in Vietnamese into `DECKCRAFT_SHOTS` (run
/// with `--ignored`; needs a GPU or software wgpu adapter).
#[test]
#[ignore]
fn closing_screenshots() {
    let Some(dir) = std::env::var_os("DECKCRAFT_SHOTS").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let pptx = deckcraft_engine::cmd::file::save_bytes(&deckcraft_model::defaults::new_presentation(None), "pptx").unwrap();
    let names = ["Báo cáo tài chính quý III.pptx", "Kế hoạch thi công.pptx", "Đào tạo an toàn lao động.pptx", "Họp giao ban tuần 42.pptx"];
    let sized = |size: (f32, f32), services: Services| {
        let mut app = SlideApp::new(Session::new(), services);
        app.ui.language = "vi".into();
        for name in names {
            app.open_or_insert_bytes(name, &pptx);
        }
        let mut h = Harness::builder().with_size(egui::vec2(size.0, size.1)).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        h.run_steps(3);
        h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
        h
    };
    let save = |h: &mut Harness<'_, SlideApp>, file: &str| {
        h.run_steps(3);
        match h.render() {
            Ok(img) => img.save(dir.join(file)).unwrap(),
            Err(e) => eprintln!("no renderer: {e}"),
        }
    };

    let mut h = sized((1366.0, 768.0), Services::default());
    h.state_mut().run("file.close", json!({})).unwrap();
    h.run_steps(2);
    h.get_by_label("Kế hoạch thi công").click_button(egui::PointerButton::Secondary);
    save(&mut h, "we-tabmenu-1366x768-vi.png");

    for (size, file) in [((1366.0, 768.0), "we-closeall-1366x768-vi.png"), ((390.0, 844.0), "we-closeall-390x844-vi.png")] {
        let (services, calls) = picker_services();
        let mut h = sized(size, services);
        for i in [0, 1, 3] {
            make_dirty(h.state_mut(), i);
        }
        h.state_mut().run("app.closeAll", json!({})).unwrap();
        save(&mut h, file);
        if size.0 > 1000.0 {
            h.get_by_label("Lưu tất cả").click();
            save(&mut h, "we-closeall-saving-1366x768-vi.png");
            settle(&mut h, &calls, SaveOutcome::Saved);
            save(&mut h, "we-closeall-next-1366x768-vi.png");
        }
    }

    for (size, suffix) in [((1366.0, 768.0), "1366x768"), ((390.0, 844.0), "390x844")] {
        let (services, calls) = picker_services();
        let mut h = sized(size, services);
        make_dirty(h.state_mut(), 3);
        h.state_mut().save();
        save(&mut h, &format!("we-title-saving-{suffix}-vi.png"));
        settle(&mut h, &calls, SaveOutcome::Failed("Không đủ dung lượng trên ổ đĩa.".into()));
        let reason = h.state().save_failure_text(uid(h.state(), "Họp giao ban tuần 42")).unwrap();
        h.get_by_label(&reason).hover();
        save(&mut h, &format!("we-title-failed-{suffix}-vi.png"));
        h.hover_at(egui::pos2(size.0 / 2.0, size.1 / 2.0));
        h.run_steps(3);
        h.get_by_label(&reason).click();
        h.hover_at(egui::pos2(size.0 / 2.0, size.1 / 2.0));
        save(&mut h, &format!("we-title-failed-tapped-{suffix}-vi.png"));
    }

    let mut h = sized((1366.0, 768.0), Services::default());
    h.state_mut().close_all();
    save(&mut h, "we-start-reopen-1366x768-vi.png");
}
