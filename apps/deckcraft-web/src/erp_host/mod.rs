//! The office site's online documents: `window.erpHost` v1 and v2 (`docs/erp-host.md` in
//! erpcons-office). A presentation the shell opens through it saves to the server: AutoSave, Save
//! and Ctrl+S call `erpHost.save` and only the server's acknowledgement marks it saved; it is never
//! a draft. v2 adds the view and comment modes, reloading in place, the account's name and the
//! client prefix new ids are mixed with; v3 (offered on a v2 object) co-editing by region
//! ([`deckcraft_ui_egui::live`]); the comment calls (M5, offered to every v2 shell) Cloud comments
//! anchored in the presentation ([`deckcraft_ui_egui::comments`]).
//! Without `erpHost` (version 1 or 2) none of this exists and the app is unchanged.

mod autosave;
#[cfg(target_arch = "wasm32")]
mod bridge;
#[cfg(target_arch = "wasm32")]
mod comments;
mod documents;
#[cfg(target_arch = "wasm32")]
mod live;
mod mode;
mod open;
mod outcome;
mod request;

use std::cell::RefCell;
use std::rc::Rc;

use deckcraft_ui_egui::comments::Comments;
use deckcraft_ui_egui::host_documents::HostDocuments;
use deckcraft_ui_egui::live::Live;
use deckcraft_ui_egui::{Services, SlideApp};

#[cfg(target_arch = "wasm32")]
pub(crate) use bridge::install;
use documents::{Apply, HostDocs};
use request::Request;

pub(crate) struct ErpHost {
    docs: HostDocs,
    /// What `open`, `setMode`, `setUser` and `setClientId` asked for, carried out in the next frame.
    opens: Rc<RefCell<Vec<Request>>>,
    /// Hands the shell its handlers (`open`, `flush`, `acknowledged`…), once the app can open
    /// documents.
    register: Option<Box<dyn FnOnce()>>,
    v2: bool,
    /// v3 co-editing, handed to the app on the first frame.
    live: Option<Live>,
    /// The comment calls (v2), handed to the app on the first frame.
    comments: Option<Comments>,
}

impl ErpHost {
    fn new(
        docs: HostDocs,
        opens: Rc<RefCell<Vec<Request>>>,
        register: Box<dyn FnOnce()>,
        v2: bool,
        live: Option<Live>,
        comments: Option<Comments>,
    ) -> Self {
        ErpHost { docs, opens, register: Some(register), v2, live, comments }
    }

    /// Hands the app the shell's documents. With v2 the app offers no Share… of its own: the
    /// shell's Share is the only one.
    pub(crate) fn attach(&self, services: &mut Services) {
        services.host_documents = Some(self.documents());
        if self.v2 {
            services.share = None;
        }
    }

    fn documents(&self) -> Box<dyn HostDocuments> {
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
            match request {
                Request::Open(request) => open::open(app, &self.docs, request),
                Request::SetMode { doc_id, mode, can_download } => mode::set_mode(app, &self.docs, &doc_id, mode, can_download),
                Request::SetUser(name) => app.session.host_author = name,
                Request::SetClientId(prefix) => app.session.set_client_prefix(prefix),
            }
        }
        if let Some(live) = self.live.take() {
            app.live = Some(live);
        }
        if let Some(comments) = self.comments.take() {
            app.comments = Some(comments);
        }
        // The room is about the host document opened last.
        let newest = app.session.documents().iter().map(|d| d.uid).filter(|uid| self.docs.owns(*uid)).max();
        app.live_bind(newest);
        app.comments_bind(newest);
        // Before `ui`, so what the shell sent is drawn this frame.
        app.comments_step();
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
    pub(crate) fn after_frame(&mut self, app: &mut SlideApp) -> Option<f64> {
        // After `ui`, where the user's changes were made: they go out this frame.
        let live_busy = app.live_step();
        // Again after `ui`: a marker clicked reaches the shell and `anchorOfSelection` sees the
        // selection just made.
        app.comments_step();
        // Encoding a deck takes long enough to stutter a slide show, so AutoSave waits for its end.
        let showing = app.show.is_some();
        for d in app.session.documents() {
            if self.docs.owns(d.uid) {
                self.docs.observe(d.uid, &d.doc, d.revision, showing || d.composition.is_some() || d.interaction.is_some());
            }
        }
        let wait = self.docs.tick();
        if live_busy { Some(wait.map_or(deckcraft_ui_egui::live::SELECTION_MS, |w| w.min(deckcraft_ui_egui::live::SELECTION_MS))) } else { wait }
    }
}

#[cfg(test)]
mod tests;
