//! Presentations a host keeps itself (the office site's online documents, web only): Save, Ctrl+S
//! and the close prompts' Save hand them to the host, which says later whether the save is
//! durable. Save As, Export and Print stay local files, and these presentations are never drafts.
//! Without a host (the desktop, standalone pages) nothing here applies.

use std::sync::Arc;

use deckcraft_model::Presentation;

use crate::SlideApp;

/// The host side of [`crate::Services::host_documents`].
pub trait HostDocuments {
    /// The host keeps presentation `uid`.
    fn owns(&self, uid: u64) -> bool;
    /// The user saved `uid` as `doc` at `revision` (loss warnings already confirmed). False when
    /// there was nothing to send.
    fn save(&mut self, uid: u64, doc: &Arc<Presentation>, revision: u64) -> bool;
    /// A save of `uid` has not been answered yet.
    fn saving(&self, uid: u64) -> bool;
}

impl SlideApp {
    pub fn host_owns(&self, uid: u64) -> bool {
        self.services.host_documents.as_ref().is_some_and(|h| h.owns(uid))
    }

    /// A save of `uid` is still being written: the browser's save picker or the host.
    pub(crate) fn save_in_flight(&self, uid: u64) -> bool {
        self.pending_saves.iter().any(|p| p.uid == uid) || self.services.host_documents.as_ref().is_some_and(|h| h.saving(uid))
    }

    /// [`has_unsaved_changes`](Self::has_unsaved_changes) without the host's documents: the host
    /// guards leaving the page for those, and hears of their saves before a frame here does.
    pub fn has_unsaved_local_changes(&self) -> bool {
        self.session.documents().iter().any(|d| d.is_dirty() && !self.host_owns(d.uid)) || !self.pending_saves.is_empty()
    }

    pub(crate) fn save_to_host(&mut self, uid: u64) {
        if self.warn_save_losses("pptx", crate::loss_warning::Action::Save) {
            return;
        }
        let Some((doc, revision)) = self.session.documents().iter().find(|d| d.uid == uid).map(|d| (d.doc.clone(), d.revision)) else { return };
        let sent = self.services.host_documents.as_mut().is_some_and(|h| h.save(uid, &doc, revision));
        if !sent {
            self.note_saved(uid);
        }
    }

    /// The host made `written` (what a save of `uid` sent) durable: that is the saved state, so
    /// edits made while it was in flight stay unsaved.
    pub fn host_saved(&mut self, uid: u64, written: Arc<Presentation>) {
        self.mark_saved(uid, written);
        self.note_saved(uid);
    }

    /// A save of `uid` ended without being durable. `error` is shown where save errors are; the
    /// host shows its own state otherwise (offline, conflict).
    pub fn host_save_ended(&mut self, uid: u64, error: Option<&str>) {
        let Some(error) = error else {
            self.note_save_cancelled(uid);
            return;
        };
        self.note_save_failed(uid, error);
        let name = self.session.documents().iter().find(|d| d.uid == uid).map(|d| d.title()).unwrap_or_default();
        self.set_status(crate::i18n::fmt(crate::tl!("Couldn't save {name}: {error}"), &[("name", name.as_str()), ("error", error)]));
    }
}

#[cfg(test)]
mod tests;
