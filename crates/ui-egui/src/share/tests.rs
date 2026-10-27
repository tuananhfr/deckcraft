#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use deckcraft_engine::Session;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::backstage::Page;
use crate::{Services, SlideApp};

type Calls = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// Web-like services: a share sheet answering `answer` and a download sink, each recorded.
fn sharing(answer: bool) -> (Services, Calls, Calls) {
    let (shared, downloaded): (Calls, Calls) = Default::default();
    let (share_sink, download_sink) = (shared.clone(), downloaded.clone());
    let services = Services {
        share: Some(Box::new(move |name: &str, bytes: &[u8]| {
            share_sink.borrow_mut().push((name.into(), bytes.into()));
            answer
        })),
        download: Some(Box::new(move |name: &str, bytes: &[u8]| download_sink.borrow_mut().push((name.into(), bytes.into())))),
        ..Default::default()
    };
    (services, shared, downloaded)
}

fn file_tab(services: Services) -> Harness<'static, SlideApp> {
    let mut app = SlideApp::new(Session::with_new(), services);
    app.run("slide.new", json!({})).unwrap();
    app.open_backstage(Page::Home);
    let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

fn assert_presentation(name: &str, bytes: &[u8], slides: usize) {
    assert!(name.ends_with(".pptx"), "shared as {name}");
    let reopened = deckcraft_engine::cmd::file::open_presentation(name, bytes).expect("the shared bytes open again");
    assert_eq!(reopened.slides.len(), slides);
}

#[test]
fn without_a_share_sheet_there_is_no_share_entry() {
    let mut h = file_tab(Services { download: Some(Box::new(|_: &str, _: &[u8]| {})), ..Default::default() });
    assert!(h.query_by_label("Share…").is_none());
    assert!(!h.state().can_share());
    assert!(h.state_mut().run("app.share", json!({})).is_err());
    let file_menu = |share| crate::menus::menu_tree(share).into_iter().find(|(menu, _)| *menu == "File").unwrap().1;
    assert!(!file_menu(false).iter().any(|(_, id)| *id == "app.share"));
    assert!(file_menu(true).contains(&("Share…", "app.share")));
}

#[test]
fn share_hands_the_presentation_as_pptx_to_the_share_sheet() {
    let (services, shared, downloaded) = sharing(true);
    let mut h = file_tab(services);
    let dirty = h.state().session.active().unwrap().is_dirty();
    let slides = h.state().session.active().unwrap().doc.slides.len();
    let title = h.state().session.active().unwrap().title();
    h.get_by_label("Share…").click();
    h.run_steps(3);
    let shared = shared.borrow();
    assert_eq!(shared.len(), 1, "one share per click");
    let (name, bytes) = shared.first().unwrap();
    assert_eq!(name, &format!("{title}.pptx"));
    assert_presentation(name, bytes, slides);
    assert!(downloaded.borrow().is_empty());
    assert!(h.state().backstage.is_none(), "back to the presentation");
    assert_eq!(h.state().session.active().unwrap().is_dirty(), dirty, "a shared copy is not a save");
}

#[test]
fn a_share_sheet_that_declines_falls_back_to_the_download() {
    let (services, shared, downloaded) = sharing(false);
    let mut h = file_tab(services);
    h.get_by_label("Share…").click();
    h.run_steps(3);
    assert_eq!(shared.borrow().len(), 1);
    assert_eq!(*downloaded.borrow(), *shared.borrow(), "the same name and bytes download");
}

#[test]
fn enter_on_share_downloads_since_a_key_cannot_open_the_share_sheet() {
    let (services, shared, downloaded) = sharing(true);
    let mut h = file_tab(services);
    let (top, _) = crate::backstage::entries(true, true);
    h.state_mut().backstage.as_mut().unwrap().focus = top.iter().position(|n| *n == crate::backstage::Nav::Share);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(shared.borrow().is_empty());
    assert_eq!(downloaded.borrow().len(), 1);
}

#[test]
fn a_loss_warning_comes_first_and_its_confirmation_shares() {
    let (services, shared, downloaded) = sharing(true);
    let mut h = file_tab(services);
    Arc::make_mut(&mut h.state_mut().session.active_mut().unwrap().doc).import_losses.macros = true;
    h.run_steps(2);
    h.get_by_label("Share…").click();
    h.run_steps(3);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.id.as_str()), Some(crate::loss_warning::DIALOG));
    assert!(shared.borrow().is_empty(), "nothing leaves before the confirmation");
    h.get_by_label("Save Anyway").click();
    h.run_steps(3);
    assert_eq!(shared.borrow().len(), 1);
    assert!(downloaded.borrow().is_empty());
    assert!(h.state().dialog.is_none());
}

#[test]
fn only_a_pointer_click_shares_keyboard_and_accessibility_clicks_download() {
    let (services, shared, downloaded) = sharing(true);
    let mut h = file_tab(services);
    h.get_by_label("Share…").click_accesskit();
    h.run_steps(3);
    assert!(shared.borrow().is_empty());
    assert_eq!(downloaded.borrow().len(), 1);

    // A command has no click at all.
    h.state_mut().run("app.share", json!({})).unwrap();
    assert!(shared.borrow().is_empty());
    assert_eq!(downloaded.borrow().len(), 2);

    // Tab to Save Anyway in the loss warning, then Enter.
    Arc::make_mut(&mut h.state_mut().session.active_mut().unwrap().doc).import_losses.macros = true;
    h.state_mut().open_backstage(Page::Home);
    h.run_steps(2);
    h.get_by_label("Share…").click();
    h.run_steps(3);
    h.get_by_label("Save Anyway").focus();
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().dialog.is_none());
    assert!(shared.borrow().is_empty(), "no share sheet without a pointer click");
    assert_eq!(downloaded.borrow().len(), 3);
}
