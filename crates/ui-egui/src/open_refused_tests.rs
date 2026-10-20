//! A file refused by the decompression limits says why in the interface language, on every way
//! to open, and opens nothing.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use deckcraft_engine::{EngineError, Session};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::{Services, SlideApp};

const VI: &str = "Tệp quá lớn hoặc bị nén bất thường nên không mở được.";

/// A presentation whose one medium is 64 MiB of zeros, deflated to a few dozen KiB.
fn bomb() -> Vec<u8> {
    let mut p = deckcraft_model::defaults::new_presentation(None);
    p.add_media("a.bmp", "image/bmp", vec![0; 64 << 20]);
    deckcraft_engine::cmd::file::save_bytes(&p, "deckcraft").unwrap()
}

#[test]
fn a_zip_bomb_shows_the_translated_reason_and_opens_nothing() {
    let mut app = SlideApp::new(Session::new(), Services::default());
    app.run("file.new", json!({})).unwrap();
    app.ui.language = "vi".into();
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(2);
    let msg = EngineError::FileTooLarge.to_string();
    assert_eq!(crate::i18n::tr(crate::i18n::Lang::from_code("vi").unwrap(), &msg), VI);
    let toast = |h: &Harness<'_, SlideApp>| h.state().toast.as_ref().map(|t| t.text.clone());

    // Dropped, or handed over by the web shell's open.
    h.state_mut().open_or_insert_bytes("Bomb.deckcraft", &bomb());
    assert_eq!(toast(&h), Some(msg.clone()));
    h.run_steps(2);
    assert!(h.query_all_by_label_contains(VI).next().is_some(), "said in Vietnamese");
    h.state_mut().toast = None;

    // A desktop path.
    let dir = std::env::temp_dir().join(format!("deckcraft-bomb-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Bomb.deckcraft");
    std::fs::write(&path, bomb()).unwrap();
    assert_eq!(h.state_mut().open_path(&path.to_string_lossy()), Err(msg.clone()));
    assert_eq!(toast(&h), Some(msg));
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(h.state().session.documents().len(), 1, "the blank presentation stays, nothing else opened");
}
