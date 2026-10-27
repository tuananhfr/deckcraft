//! The office site's online documents: `window.erpHost` v1 (`docs/erp-host.md` in erpcons-office).
//! A presentation the shell opens through it saves to the server: AutoSave, Save and Ctrl+S
//! call `erpHost.save` and only the server's acknowledgement marks it saved; it is never a draft.
//! Without `erpHost` (version 1) none of this exists and the app is unchanged.

mod autosave;
#[cfg(target_arch = "wasm32")]
mod bridge;
mod documents;
mod open;
mod outcome;

use std::cell::RefCell;
use std::rc::Rc;

use deckcraft_ui_egui::SlideApp;
use deckcraft_ui_egui::host_documents::HostDocuments;

#[cfg(target_arch = "wasm32")]
pub(crate) use bridge::install;
use documents::{Apply, HostDocs};
use open::OpenRequest;

pub(crate) struct ErpHost {
    docs: HostDocs,
    /// Documents `open` received, opened in the next frame (not inside the shell's call).
    opens: Rc<RefCell<Vec<OpenRequest>>>,
    /// Hands the shell `open`, `flush` and `acknowledged`, once the app can open documents.
    register: Option<Box<dyn FnOnce()>>,
}

impl ErpHost {
    fn new(docs: HostDocs, opens: Rc<RefCell<Vec<OpenRequest>>>, register: Box<dyn FnOnce()>) -> Self {
        ErpHost { docs, opens, register: Some(register) }
    }

    /// For `Services::host_documents`.
    pub(crate) fn documents(&self) -> Box<dyn HostDocuments> {
        Box::new(self.docs.clone())
    }

    /// After the app's `logic`: closed tabs, opened documents and the shell's answers.
    pub(crate) fn logic(&mut self, app: &mut SlideApp) {
        if app.fonts_ready()
            && let Some(register) = self.register.take()
        {
            register();
        }
        // Before opening, so a tab closed and opened again is reported in that order.
        let open: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
        self.docs.retain_open(&open);
        let requests = self.opens.try_borrow_mut().map(|mut r| std::mem::take(&mut *r)).unwrap_or_default();
        for request in requests {
            open::open(app, &self.docs, request);
        }
        self.docs.pump();
        for apply in self.docs.take_applies() {
            match apply {
                Apply::Saved(uid, written) => app.host_saved(uid, written),
                Apply::Unsaved(uid, message) => app.host_save_ended(uid, message.as_deref()),
            }
        }
    }

    /// At the end of every frame, after edits: starts what is due; returns ms until the next
    /// AutoSave may be.
    pub(crate) fn after_frame(&mut self, app: &SlideApp) -> Option<f64> {
        // Encoding a deck takes long enough to stutter a slide show, so AutoSave waits for its end.
        let showing = app.show.is_some();
        for d in app.session.documents() {
            if self.docs.owns(d.uid) {
                self.docs.observe(d.uid, &d.doc, d.revision, showing || d.composition.is_some() || d.interaction.is_some());
            }
        }
        self.docs.tick()
    }
}

#[cfg(test)]
mod tests;
