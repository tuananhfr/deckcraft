//! Save destinations and copies that leave the open presentation's identity untouched.

use serde_json::{Value, json};

use crate::{PendingSave, SlideApp, loss_warning::Action};

impl SlideApp {
    pub(crate) fn export_loss_copy_format(&mut self, format: &str, extra: Value) {
        if self.services.pick_save.is_none() {
            self.export_as(format, extra);
            return;
        }
        let mut params = json!({"format": format, "all": format == "pngAll"});
        if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
            params.extend(extra.clone());
        }
        let format = match format {
            "pngAll" => "png",
            "jpg" => "jpeg",
            "txt" => "outline",
            other => other,
        };
        self.export_loss_copy(params, format);
    }

    pub(crate) fn export_loss_copy(&mut self, mut params: Value, format: &str) {
        let Some(d) = self.session.active() else { return };
        let source = d.path.clone();
        let name = crate::i18n::fmt(crate::tl!("{name} - Copy.{ext}"), &[("name", &d.title()), ("ext", format)]);
        let Some(pick) = self.services.pick_save.as_mut() else { return };
        let Some(path) = pick(&name) else { return };
        if source.as_deref().is_some_and(|source| same_path(source, &path)) {
            self.set_status(crate::tl!("Choose a different file to save a copy."));
            return;
        }
        if let Some(object) = params.as_object_mut() {
            object.insert("path".into(), json!(path));
            object.insert("format".into(), json!(format));
            if let Err(e) = self.session.execute("file.export", &params) {
                self.set_status(e.to_string());
            }
        }
    }

    pub(crate) fn save_as_path(&mut self, path: &str) {
        let format = deckcraft_engine::cmd::file::format_for_path(path);
        if self.warn_save_losses(format, Action::SavePath(path.into())) {
            return;
        }
        let Some(uid) = self.session.active().map(|d| d.uid) else { return };
        match self.session.execute("file.saveAs", &json!({"path": path})) {
            Ok(_) => {
                self.set_status(crate::i18n::fmt(crate::tl!("Saved {path}"), &[("path", path)]));
                self.ui.recent.retain(|p| p != path);
                self.ui.recent.insert(0, path.into());
                self.note_saved(uid);
            }
            Err(e) => {
                self.note_save_failed(uid, &e.to_string());
                self.set_status(e.to_string());
            }
        }
    }

    pub(crate) fn save_loss_copy(&mut self, format: &str) {
        let Some(d) = self.session.active() else { return };
        let (uid, doc, source) = (d.uid, d.doc.clone(), d.path.clone());
        let name = crate::i18n::fmt(crate::tl!("{name} - Copy.{ext}"), &[("name", &d.title()), ("ext", format)]);
        if let Some(pick) = self.services.pick_save.as_mut() {
            let Some(path) = pick(&name) else { return };
            if source.as_deref().is_some_and(|source| same_path(source, &path)) {
                self.set_status(crate::tl!("Choose a different file to save a copy."));
                return;
            }
            // An explicit format keeps the approved writer even if the chosen suffix differs.
            if let Err(e) = self.session.execute("file.export", &json!({"path": path, "format": format})) {
                self.set_status(e.to_string());
            }
            return;
        }
        match deckcraft_engine::cmd::file::save_bytes(&doc, format) {
            Ok(bytes) => {
                if let Some(save) = self.services.save_file.as_mut() {
                    self.next_save += 1;
                    let id = self.next_save;
                    self.pending_saves.push(PendingSave { id, uid, written: doc, name: name.clone(), copy: true });
                    let results = self.save_results.clone();
                    save(
                        &name,
                        &bytes,
                        Box::new(move |outcome| {
                            if let Ok(mut r) = results.try_borrow_mut() {
                                r.push((id, outcome));
                            }
                        }),
                    );
                    self.settle_saves();
                } else if let Some(download) = self.services.download.as_mut() {
                    download(&name, &bytes);
                }
            }
            Err(e) => self.set_status(e.to_string()),
        }
    }
}

fn same_path(a: &str, b: &str) -> bool {
    let a = std::fs::canonicalize(a).unwrap_or_else(|_| std::path::PathBuf::from(a));
    let b = std::fs::canonicalize(b).unwrap_or_else(|_| std::path::PathBuf::from(b));
    if cfg!(target_os = "windows") { a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()) } else { a == b }
}
