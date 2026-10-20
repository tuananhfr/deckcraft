use serde_json::json;

use crate::SlideApp;
use crate::dialogs::Dialog;
use crate::save_state::SaveState;

impl SlideApp {
    /// Closes the active presentation, asking about unsaved changes first.
    pub fn close_presentation(&mut self) {
        if let Some(uid) = self.session.active().map(|d| d.uid) {
            self.close_document_by_uid(uid);
        }
    }

    /// Closes `uid` without changing which other presentation is active.
    pub fn close_document_by_uid(&mut self, uid: u64) {
        let Some((i, doc)) = self.session.documents().iter().enumerate().find(|(_, d)| d.uid == uid) else { return };
        let saving = self.save_state(uid) == Some(SaveState::Saving);
        if doc.is_dirty() || saving {
            let mut dialog = Dialog::new("close");
            dialog.params = json!({"uid": uid});
            if saving {
                dialog.fields.insert("lossSaveStarted".into(), "1".into());
            }
            self.dialog = Some(dialog);
        } else {
            let _ = self.run("file.close", json!({"index": i}));
        }
    }

    pub(crate) fn notify_editor_closed(&mut self, command: &str, open_before: usize, succeeded: bool) {
        if command == "file.close"
            && succeeded
            && open_before > 0
            && self.session.documents().is_empty()
            && let Some(notify) = self.services.editor_closed.as_mut()
        {
            notify();
        }
    }
}

#[cfg(test)]
mod tests;
