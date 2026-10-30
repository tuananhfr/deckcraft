//! The host's access to the active presentation in the interface (the office site's view and
//! comment modes, the desktop's licence lock): editing is not offered, Save only says why unless
//! the access keeps it, and without download Save As, Export, Print, Share and copying are gone.
//! The engine refuses the same changes on its own.

use deckcraft_engine::access::{self, Access, NO_DOWNLOAD, READ_ONLY};
use deckcraft_engine::{EngineError, find_command};

use crate::SlideApp;

/// Interface commands that would change the presentation.
const EDITING: &[&str] = &["app.insertPictureDialog", "app.insertAudioDialog", "app.insertVideoDialog"];
/// Interface commands that hand the presentation out of the app.
const DOWNLOADING: &[&str] = &["app.saveAsDialog", "app.exportDialog", "app.print", "app.share"];
/// Dialogs that change the presentation; the others show it, or the app's own settings.
const EDITING_DIALOGS: &[&str] = &[
    "altText",
    "chartData",
    "comment",
    "customShow",
    "equation",
    "headerFooter",
    "hyperlink",
    "language",
    "outline",
    "paragraph",
    "renameLayout",
    "renameSection",
    "renameShape",
    "setupShow",
    "slideSize",
    "smartart",
    "spelling",
    "symbol",
    "table",
    "trim",
];

impl SlideApp {
    /// The active presentation's access (everything with none open).
    pub fn access(&self) -> Access {
        self.session.active().map(|d| d.access).unwrap_or_default()
    }

    pub fn editable(&self) -> bool {
        self.access().editable
    }

    pub fn can_download(&self) -> bool {
        self.access().can_download
    }

    /// Save to the presentation's own file: editable, or locked by the licence only.
    pub fn can_save(&self) -> bool {
        let a = self.access();
        a.editable || a.can_save
    }

    /// Sets presentation `uid`'s access. Going read-only, what is still typed into a field is
    /// committed first, as one undo step: the editor never drops an edit on its own.
    pub fn set_access(&mut self, uid: u64, access: Access) -> bool {
        if !access.editable {
            self.commit_inline_edit(uid);
        }
        self.session.set_access(uid, access)
    }

    /// Why command `id` (engine or interface) is refused on the active presentation, if it is.
    pub(crate) fn access_refusal(&self, id: &str) -> Option<&'static str> {
        let a = self.access();
        match find_command(id) {
            Some(spec) => access::refusal(a, spec),
            None if !a.can_download && DOWNLOADING.contains(&id) => Some(NO_DOWNLOAD),
            None if !a.editable && EDITING.contains(&id) => Some(READ_ONLY),
            None => None,
        }
    }

    /// Why dialog `id` (`app.dialog`'s `id`) is refused on the active presentation, if it is.
    pub(crate) fn dialog_refusal(&self, id: &str) -> Option<&'static str> {
        let a = self.access();
        if id == "export" && !a.can_download {
            return Some(NO_DOWNLOAD);
        }
        (!a.editable && EDITING_DIALOGS.contains(&id)).then_some(READ_ONLY)
    }

    /// Says in the status bar why an action is refused (`why` is [`READ_ONLY`] or [`NO_DOWNLOAD`]).
    pub(crate) fn note_access(&mut self, why: &str) {
        self.set_status(EngineError::Disabled(String::new(), why.into()).to_string());
    }

    /// Save As, Export, Print and Share are allowed; otherwise says why. True when allowed.
    pub(crate) fn may_download(&mut self) -> bool {
        let ok = self.can_download();
        if !ok {
            self.note_access(NO_DOWNLOAD);
        }
        ok
    }
}

/// What a read-only presentation says when a change is refused.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReadOnlyNotice {
    #[default]
    View,
    /// The office site's comment mode, view-only until comments on the web come.
    CommentsLater,
}

impl SlideApp {
    /// Sets what presentation `uid` says when a change is refused while it is read-only.
    pub fn set_read_only_notice(&mut self, uid: u64, notice: ReadOnlyNotice) {
        if notice == ReadOnlyNotice::View {
            self.read_only_notices.remove(&uid);
        } else {
            self.read_only_notices.insert(uid, notice);
        }
    }

    /// The translated notice for a refusal's message, which names the command in English. A
    /// refusal by the licence lock gives its reason and tells the account hook.
    pub(crate) fn access_notice(&self, message: &str) -> Option<&'static str> {
        if message.ends_with(READ_ONLY)
            && let Some(lock) = self.licence_refusal()
        {
            if let Some(hook) = &self.account_hook {
                hook.edit_refused();
            }
            Some(lock.text())
        } else if message.ends_with(READ_ONLY) {
            let uid = self.session.active().map(|d| d.uid);
            Some(match uid.and_then(|uid| self.read_only_notices.get(&uid)) {
                Some(ReadOnlyNotice::CommentsLater) => crate::tl!("Comments on the web are coming later; for now you can only view."),
                _ => crate::tl!("You can only view this presentation."),
            })
        } else if message.ends_with(NO_DOWNLOAD) {
            Some(crate::tl!("Downloading, printing and copying are turned off for this presentation."))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests;
