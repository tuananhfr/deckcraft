//! One confirmation before a save hands lossy bytes to the filesystem or browser.

use std::sync::Arc;

use deckcraft_engine::save_losses::LossItem;
use deckcraft_model::Presentation;
use serde_json::Value;

use crate::{SlideApp, dialogs::Dialog};

pub(crate) const DIALOG: &str = "saveLosses";

#[derive(Clone)]
pub(crate) enum Action {
    Save,
    SaveAs(String),
    SavePath(String),
    Export(String, Value),
    Command(String, Value),
    Print(Value),
    Share,
}

struct Pending {
    uid: u64,
    doc: Arc<Presentation>,
    format: String,
    losses: Vec<LossItem>,
    action: Action,
    return_dialog: Option<Dialog>,
}

#[derive(Default)]
pub(crate) struct LossWarning {
    pending: Option<Pending>,
    approved: bool,
}

impl SlideApp {
    pub(crate) fn awaiting_loss_confirmation(&self, uid: u64) -> bool {
        self.loss_warning.pending.as_ref().is_some_and(|p| p.uid == uid)
    }

    pub(crate) fn suspend_for_loss_warning(&mut self, mut dialog: Dialog) {
        if dialog.id != "close" && dialog.id != crate::close_many::DIALOG {
            return;
        }
        if let Some(pending) = self.loss_warning.pending.as_mut() {
            if dialog.id == "close" {
                dialog.params = serde_json::json!({"uid": pending.uid});
            }
            pending.return_dialog = Some(dialog);
        }
    }

    pub(crate) fn warn_save_command(&mut self, id: &str, params: &Value) -> bool {
        if !matches!(id, "file.save" | "file.saveAs" | "file.export") {
            return false;
        }
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .or_else(|| if id == "file.save" { self.session.active().and_then(|d| d.path.as_deref()) } else { None });
        let Some(path) = path else { return false };
        let format = params.get("format").and_then(Value::as_str).unwrap_or_else(|| deckcraft_engine::cmd::file::format_for_path(path));
        self.warn_save_losses(format, Action::Command(id.into(), params.clone()))
    }

    pub(crate) fn warn_save_losses(&mut self, format: &str, action: Action) -> bool {
        if self.loss_warning.approved {
            return false;
        }
        let losses = self.session.save_losses(format);
        if losses.is_empty() {
            return false;
        }
        let Some(d) = self.session.active() else { return false };
        self.loss_warning.pending = Some(Pending { uid: d.uid, doc: d.doc.clone(), format: format.into(), losses, action, return_dialog: None });
        self.dialog = Some(Dialog::new(DIALOG));
        true
    }

    /// `pointer`: the confirmation was a pointer click, which a share sheet needs.
    fn resume_lossy_save(&mut self, copy: bool, pointer: bool) {
        let Some(pending) = self.loss_warning.pending.take() else { return };
        let Some(index) = self.session.documents().iter().position(|d| d.uid == pending.uid) else { return };
        let active = self.session.active().map(|d| d.uid);
        self.session.set_active(index);
        // An edit while the prompt was open requires consent to the new document's losses.
        if self.session.active().is_some_and(|d| !Arc::ptr_eq(&d.doc, &pending.doc)) && self.warn_save_losses(&pending.format, pending.action.clone())
        {
            if let Some(next) = self.loss_warning.pending.as_mut() {
                next.return_dialog = pending.return_dialog;
            }
            if let Some(i) = active.and_then(|uid| self.session.documents().iter().position(|d| d.uid == uid)) {
                self.session.set_active(i);
            }
            return;
        }
        self.loss_warning.approved = true;
        if copy {
            match pending.action {
                // Sharing never renames or marks the presentation saved: a copy is the same share.
                Action::Share => self.share_presentation(pointer),
                Action::Export(fmt, extra) => self.export_loss_copy_format(&fmt, extra),
                Action::Print(extra) => self.export_loss_copy_format("pdf", extra),
                Action::Command(id, params) if id == "file.export" => self.export_loss_copy(params, &pending.format),
                _ => self.save_loss_copy(&pending.format),
            }
        } else {
            match pending.action {
                Action::Save => self.save(),
                Action::SaveAs(fmt) => self.save_as_dialog(&fmt),
                Action::SavePath(path) => self.save_as_path(&path),
                Action::Export(fmt, extra) => {
                    self.export_as(&fmt, extra);
                }
                Action::Command(id, params) => {
                    let _ = self.run(&id, params);
                }
                Action::Print(params) => {
                    self.print_pdf(params);
                }
                Action::Share => self.share_presentation(pointer),
            }
        }
        self.loss_warning.approved = false;
        if !copy && let Some(mut dialog) = pending.return_dialog {
            if dialog.id == "close" {
                dialog.fields.insert("lossSaveStarted".into(), "1".into());
            }
            self.dialog = Some(dialog);
        } else if copy {
            self.close_batch = None;
        }
        if let Some(i) = active.and_then(|uid| self.session.documents().iter().position(|d| d.uid == uid)) {
            self.session.set_active(i);
        }
    }
}

fn label(loss: &LossItem) -> String {
    match loss {
        LossItem::OleObjects { count } => crate::i18n::fmt(
            crate::tl!("{count} OLE objects will be replaced by their preview images (or empty placeholders if no preview is available)."),
            &[("count", &count.to_string())],
        ),
        LossItem::DigitalSignatures => crate::tl!("Digital signatures will be removed and will no longer be valid.").into(),
        LossItem::Macros => crate::tl!("Macros will be removed.").into(),
        LossItem::UnsupportedParts { kinds } => {
            crate::i18n::fmt(crate::tl!("Unsupported parts will be removed: {kinds}"), &[("kinds", &kinds.join(", "))])
        }
    }
}

pub(crate) fn cancel(app: &mut SlideApp) {
    app.loss_warning.pending = None;
    app.close_batch = None;
}

pub(crate) fn body(app: &mut SlideApp, ui: &mut egui::Ui, _dialog: &mut Dialog) -> bool {
    let Some(pending) = app.loss_warning.pending.as_ref() else { return true };
    egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
        for loss in &pending.losses {
            ui.label(format!("• {}", label(loss)));
        }
    });
    ui.add_space(8.0);
    let mut choice = None;
    let mut pointer = false;
    ui.horizontal_wrapped(|ui| {
        let anyway = ui.button(crate::tl!("Save Anyway"));
        if anyway.clicked() {
            choice = Some(false);
            pointer = anyway.clicked_by(egui::PointerButton::Primary);
        }
        let copy = ui.button(crate::tl!("Save a Copy"));
        if copy.clicked() {
            choice = Some(true);
            pointer = copy.clicked_by(egui::PointerButton::Primary);
        }
        if ui.button(crate::tl!("Cancel")).clicked() {
            cancel(app);
            choice = Some(false);
        }
    });
    if let Some(copy) = choice {
        app.resume_lossy_save(copy, pointer);
        return true;
    }
    false
}

#[cfg(test)]
mod tests;
