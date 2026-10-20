//! What the title bar says about saving a presentation: "Saving…" while a save is in flight (the
//! browser's save picker and its write, or a desktop write), and "Save failed" with the reason
//! until the next successful save or edit. Unsaved changes ("Edited") are the document's own.

use std::collections::HashMap;
use std::sync::Arc;

use deckcraft_model::Presentation;

use crate::SlideApp;

struct Failure {
    reason: String,
    /// The presentation as it was when the save failed; an edit since ends the failure.
    doc: Arc<Presentation>,
}

#[derive(Default)]
pub struct SaveStatus {
    failures: HashMap<u64, Failure>,
    /// A desktop Save that waits for the frame showing "Saving…" to be painted before the write
    /// blocks the window: the presentation's uid and that frame's number, once drawn.
    queued: Option<(u64, Option<u64>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveState {
    Saving,
    /// The reason, as the host or the file system gave it.
    Failed(String),
}

impl SaveState {
    /// The word the title bar adds after the presentation's name.
    pub fn label(&self) -> &'static str {
        match self {
            SaveState::Saving => crate::tl!("Saving…"),
            SaveState::Failed(_) => crate::tl!("Save failed"),
        }
    }
}

impl SlideApp {
    /// Saving or a failed save of presentation `uid`; None when neither.
    pub fn save_state(&self, uid: u64) -> Option<SaveState> {
        if self.save_status.queued.is_some_and(|q| q.0 == uid) || self.pending_saves.iter().any(|p| p.uid == uid) {
            return Some(SaveState::Saving);
        }
        let f = self.save_status.failures.get(&uid)?;
        let current = self.session.documents().iter().find(|d| d.uid == uid)?;
        Arc::ptr_eq(&f.doc, &current.doc).then(|| SaveState::Failed(f.reason.clone()))
    }

    /// "Couldn't save “name”: reason" for a failed save, for its tooltip and the status bar.
    pub fn save_failure_text(&self, uid: u64) -> Option<String> {
        let Some(SaveState::Failed(reason)) = self.save_state(uid) else { return None };
        let name = self.session.documents().iter().find(|d| d.uid == uid).map(|d| d.title()).unwrap_or_default();
        Some(crate::i18n::fmt(crate::tl!("Couldn't save {name}: {error}"), &[("name", &format!("“{name}”")), ("error", &reason)]))
    }

    pub(crate) fn note_saved(&mut self, uid: u64) {
        self.save_status.failures.remove(&uid);
    }

    pub(crate) fn note_save_failed(&mut self, uid: u64, reason: &str) {
        let Some(doc) = self.session.documents().iter().find(|d| d.uid == uid).map(|d| d.doc.clone()) else { return };
        self.save_status.failures.insert(uid, Failure { reason: reason.to_string(), doc });
    }

    /// Save, the button and Ctrl+S. A host that writes synchronously (the desktop) writes on the
    /// next frame, after "Saving…" is shown; the web's save picker must open within the click or
    /// key that asked for it, so it opens now.
    pub fn request_save(&mut self) {
        let Some(uid) = self.session.active().map(|d| d.uid) else { return };
        if self.services.save_file.is_some() {
            self.save();
        } else {
            self.save_status.queued = Some((uid, None));
        }
    }

    /// The title bar was drawn in frame `frame`: a queued Save runs once that frame is painted.
    pub(crate) fn saving_shown(&mut self, ctx: &egui::Context) {
        if let Some((_, shown)) = &mut self.save_status.queued {
            shown.get_or_insert(ctx.cumulative_frame_nr());
            ctx.request_repaint();
        }
    }

    /// Runs the Save [`request_save`](Self::request_save) queued, in a frame after the one that
    /// showed "Saving…" (a frame can take several passes before it is painted).
    pub(crate) fn run_queued_save(&mut self, ctx: &egui::Context) {
        let Some((uid, Some(shown))) = self.save_status.queued else { return };
        if ctx.cumulative_frame_nr() <= shown {
            return;
        }
        self.save_status.queued = None;
        if let Some(i) = self.session.documents().iter().position(|d| d.uid == uid) {
            self.save_document(i);
        }
    }

    /// Forgets failures of presentations that are no longer open.
    pub(crate) fn prune_save_failures(&mut self) {
        let docs = self.session.documents();
        self.save_status.failures.retain(|uid, _| docs.iter().any(|d| d.uid == *uid));
    }
}
