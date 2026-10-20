//! Empty placeholders show their prompt ("Click to add title") in the interface language on the
//! canvas, and only there: the saved file keeps them empty.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use deckcraft_engine::Session;
use deckcraft_engine::cmd::file::{open_presentation, save_bytes};
use egui::vec2;
use egui_kittest::Harness;
use serde_json::json;

use crate::{Services, SlideApp};

#[test]
fn prompts_redraw_in_the_new_language_and_never_reach_the_saved_file() {
    let app = SlideApp::new(Session::with_new(), Services::default());
    let mut h = Harness::builder().with_size(vec2(1366.0, 768.0)).build_ui_state(
        |ui, app: &mut SlideApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    let doc = &h.state().session.active().unwrap().doc;
    let empty = |d: &deckcraft_model::Presentation| {
        let slide = &d.slides[0];
        let ph: Vec<_> = slide.shapes.iter().filter(|s| s.ph.is_some()).collect();
        !ph.is_empty() && ph.iter().all(|s| s.text.as_ref().is_none_or(|t| t.is_empty()))
    };
    assert!(empty(doc), "a new presentation's title slide has empty placeholders");
    let renders = h.state().textures.canvas_renders;
    h.state_mut().run("view.language", json!({"language": "vi"})).unwrap();
    h.run_steps(2);
    assert!(h.state().textures.canvas_renders > renders, "the canvas draws its prompts again in Vietnamese");
    let doc = &h.state().session.active().unwrap().doc;
    assert!(empty(doc), "drawing a prompt leaves the placeholder empty");
    let saved = open_presentation("saved.pptx", &save_bytes(doc, "pptx").unwrap()).unwrap();
    assert!(empty(&saved), "the saved PPTX has no prompt text");
}

/// Renders a new presentation's title slide in Vietnamese into `DECKCRAFT_SHOTS` (run with
/// `--ignored`; needs a GPU or software wgpu adapter).
#[test]
#[ignore]
fn prompt_screenshots() {
    let Some(dir) = std::env::var_os("DECKCRAFT_SHOTS").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    for (size, file) in [(vec2(1366.0, 768.0), "prompts-1366x768-vi.png"), (vec2(390.0, 844.0), "prompts-390x844-vi.png")] {
        let mut app = SlideApp::new(Session::with_new(), Services::default());
        app.ui.language = "vi".into();
        let mut h = Harness::builder().with_size(size).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        h.run_steps(4);
        match h.render() {
            Ok(img) => img.save(dir.join(file)).unwrap(),
            Err(e) => eprintln!("no renderer: {e}"),
        }
    }
}
