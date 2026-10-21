use std::io::Write;
use std::path::Path;

use deckcraft_ui_egui::UiState;

pub(super) fn language(saved: Option<&str>) -> &str {
    saved.unwrap_or("vi")
}

pub(super) fn load_ui(bytes: Option<&[u8]>) -> UiState {
    let value = bytes.and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok());
    let saved = value.as_ref().and_then(|value| value.get("language")).and_then(serde_json::Value::as_str);
    let selected = language(saved).to_owned();
    let mut ui = value.and_then(|value| serde_json::from_value::<UiState>(value).ok()).unwrap_or_default();
    ui.language = selected;
    ui
}

pub(super) fn save_language_change(path: &Path, ui: &UiState, last_language: &mut String) -> Result<(), Box<dyn std::error::Error>> {
    if ui.language != *last_language {
        last_language.clone_from(&ui.language);
        save_ui(path, ui)?;
    }
    Ok(())
}

pub(super) fn save_ui(path: &Path, ui: &UiState) -> Result<(), Box<dyn std::error::Error>> {
    let parent = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec_pretty(ui)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use deckcraft_engine::Session;
    use deckcraft_ui_egui::{Services, SlideApp};

    #[test]
    fn saved_language_or_default() {
        assert_eq!(language(None), "vi");
        for saved in ["en", "vi", "auto", "fr"] {
            assert_eq!(language(Some(saved)), saved);
        }
    }

    #[test]
    fn missing_language_defaults_to_vietnamese() {
        assert_eq!(load_ui(None).language, "vi");
        let ui = load_ui(Some(br#"{"notes":false}"#));
        assert_eq!(ui.language, "vi");
        assert!(!ui.notes);
        assert_eq!(load_ui(Some(br#"{"language":"en"}"#)).language, "en");
        assert_eq!(load_ui(Some(br#"{"language":"auto"}"#)).language, "auto");
    }

    #[test]
    fn language_change_saves_before_exit() {
        let dir = tempfile::tempdir().expect("settings directory");
        let path = dir.path().join("ui.json");
        let mut app = SlideApp::new(Session::new(), Services::default());
        let mut last_language = app.ui.language.clone();
        save_ui(&path, &app.ui_state_to_save()).expect("initial save");
        app.run("view.language", serde_json::json!({"language":"vi"})).expect("change language");
        save_language_change(&path, &app.ui_state_to_save(), &mut last_language).expect("save changed language");
        assert_eq!(last_language, "vi");
        let saved = std::fs::read(&path).expect("read settings before exit");
        assert_eq!(load_ui(Some(&saved)).language, "vi");
    }

    #[test]
    fn failed_save_returns_error() {
        let dir = tempfile::tempdir().expect("settings directory");
        let ui = UiState::default();
        let mut last_language = "vi".to_owned();
        assert!(save_language_change(dir.path(), &ui, &mut last_language).is_err());
        assert_eq!(last_language, ui.language);
        assert!(save_language_change(dir.path(), &ui, &mut last_language).is_ok());
    }
}
