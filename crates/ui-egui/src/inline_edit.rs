//! Text typed into a field that commits only when it loses focus (speaker notes, the outline,
//! the notes page, alt text, a shape's or a property's name), kept so it can be committed before
//! the presentation turns read-only instead of being dropped.

use serde_json::Value;

use crate::SlideApp;

/// The command the focused field would run on losing focus.
pub(crate) struct InlineEdit {
    uid: u64,
    command: &'static str,
    params: Value,
}

impl SlideApp {
    /// Records what the focused field would commit to the active presentation. Recorded again
    /// every frame the field is drawn, so a field gone from the screen leaves nothing behind.
    pub(crate) fn track_inline_edit(&mut self, command: &'static str, params: Value) {
        if let Some(uid) = self.session.active().map(|d| d.uid) {
            self.inline_edit = Some(InlineEdit { uid, command, params });
        }
    }

    /// Commits what is still typed into a field of presentation `uid`, as one undo step.
    pub(crate) fn commit_inline_edit(&mut self, uid: u64) {
        let Some(edit) = self.inline_edit.take_if(|e| e.uid == uid) else { return };
        // The field belongs to the active tab; another tab's would have committed when it lost focus.
        if self.session.active().is_some_and(|d| d.uid == uid) {
            let _ = self.run(edit.command, edit.params);
        }
    }
}
