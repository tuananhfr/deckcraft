//! Legacy open notices use the shared modal dialog host.

use deckcraft_engine::EngineError;
use serde_json::{Value, json};

use crate::{SlideApp, dialogs::Dialog};

pub(crate) const DIALOG: &str = "legacyFormat";

impl SlideApp {
    pub(crate) fn execute_open(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        match self.session.execute(command, params) {
            Ok(value) => {
                self.drain_requests();
                Ok(value)
            }
            Err(EngineError::LegacyFormat(format)) => {
                let mut dialog = Dialog::new(DIALOG);
                dialog.params = json!({"extension": format.extension(), "opened": false});
                self.dialog = Some(dialog);
                Ok(json!({"opened": false}))
            }
            Err(error) => {
                self.note_refusal(&error);
                Err(error.to_string())
            }
        }
    }
}

pub(crate) fn title(dialog: &Dialog) -> String {
    if dialog.params.get("opened").and_then(Value::as_bool) == Some(true) {
        crate::tl!("Presentation opened").into()
    } else {
        crate::i18n::fmt(crate::tl!("Cannot open this {ext} format"), &[("ext", extension(dialog))])
    }
}

fn extension(dialog: &Dialog) -> &str {
    dialog.params.get("extension").and_then(Value::as_str).unwrap_or("")
}

pub(crate) fn body(ui: &mut egui::Ui, dialog: &Dialog) -> bool {
    if dialog.params.get("opened").and_then(Value::as_bool) == Some(true) {
        ui.label(crate::i18n::fmt(
            crate::tl!("The {ext} file has been opened. When saved, it will be saved as .pptx."),
            &[("ext", extension(dialog))],
        ));
        if extension(dialog) == ".pptm" {
            ui.label(crate::tl!("Macros will be removed."));
        }
    } else {
        ui.label(crate::tl!("Open it in PowerPoint or LibreOffice, save it as .pptx, then open it again."));
    }
    ui.add_space(8.0);
    ui.button(crate::tl!("OK")).clicked() || ui.input(|input| input.key_pressed(egui::Key::Enter))
}

#[cfg(test)]
mod tests;
