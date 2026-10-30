//! What a host lets the user do with one open presentation (the office site's view and comment
//! modes, `erpHost` v2, and the desktop's licence lock). It lives on the open document only and is
//! never written into a file.

use std::sync::Arc;

use deckcraft_model::Presentation;

use crate::cmd::CommandSpec;
use crate::{Selection, Session};

/// Why a command is refused on a presentation that can't be changed.
pub const READ_ONLY: &str = "the presentation is read-only";
/// Why a command is refused on a presentation whose content may not leave the app.
pub const NO_DOWNLOAD: &str = "downloading, printing and copying are turned off for this presentation";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    pub editable: bool,
    /// Save As, Export, Print and copying content to the clipboard.
    pub can_download: bool,
    /// Save to the presentation's own file, even when it can't be changed.
    pub can_save: bool,
}

impl Default for Access {
    fn default() -> Self {
        Access { editable: true, can_download: true, can_save: true }
    }
}

impl Access {
    /// The host's view modes: the host keeps the file, so there is nothing to save either.
    pub fn read_only(can_download: bool) -> Self {
        Access { editable: false, can_download, can_save: false }
    }

    /// No changes, but Save, Save As, Export, Print and copying stay (the desktop's licence lock):
    /// what was changed before the lock can still be saved.
    pub fn edits_locked() -> Self {
        Access { editable: false, can_download: true, can_save: true }
    }
}

/// Commands that change nothing in the presentation. An allowlist, so a command added later is
/// refused on a read-only presentation until it is known to be safe there.
const VIEWING: &[&str] = &[
    // Queries.
    "animation.get",
    "animation.list",
    "commands.list",
    "comment.list",
    "design.themes",
    "document.inspect",
    "file.info",
    "file.recovery.list",
    "format.state",
    "media.info",
    "review.accessibility",
    "review.spelling",
    "shape.inspect",
    "shape.presets",
    "shape.sites",
    "slide.inspect",
    "text.get",
    "transition.list",
    // Selection, navigation and find. No `text.edit`: a read-only presentation shows no caret, so
    // the text commands below only ever meet find's matches (copying needs `can_download`).
    "edit.deselect",
    "edit.find",
    "edit.select",
    "edit.selectAll",
    "selection.select",
    "slide.first",
    "slide.go",
    "slide.last",
    "slide.next",
    "slide.previous",
    "slide.selectSlides",
    "table.selectCells",
    "text.exit",
    "text.move",
    "text.select",
    "text.selectAll",
    "text.selectParagraph",
    "text.selectWord",
    "view.closeMaster",
    "view.slideMaster",
    // Playback and the slide show.
    "media.gifPlay",
    "media.pause",
    "media.play",
    "media.seek",
    "media.stop",
    "media.toggle",
    "show.fromCurrent",
    "show.fromStart",
    // Other presentations: these never change the read-only one.
    "file.close",
    "file.new",
    "file.open",
    "file.openBytes",
    "file.recovery.discard",
    "file.recovery.open",
    "file.recovery.save",
    "file.reopenClosed",
    "window.next",
];

/// Commands that hand the presentation's content out of the app.
const DOWNLOADING: &[&str] = &["edit.copy", "edit.cut", "file.export", "file.render", "file.saveAs", "file.saveBytes", "file.saveTemplate"];

/// Why `spec` is refused under `access`, if it is.
pub fn refusal(access: Access, spec: &CommandSpec) -> Option<&'static str> {
    if !access.can_download && DOWNLOADING.contains(&spec.id) {
        return Some(NO_DOWNLOAD);
    }
    // Copying, Save As and Save change nothing; Cut does.
    let viewing =
        VIEWING.contains(&spec.id) || (DOWNLOADING.contains(&spec.id) && spec.id != "edit.cut") || (spec.id == "file.save" && access.can_save);
    (!access.editable && !viewing).then_some(READ_ONLY)
}

impl Session {
    /// Whether `spec` can run now: the active presentation's access, then the command's own
    /// condition. The ribbon and menus ask this, so they offer only what would run.
    pub fn available(&self, spec: &CommandSpec) -> Result<(), String> {
        if let Some(why) = self.active().and_then(|d| refusal(d.access, spec)) {
            return Err(why.into());
        }
        (spec.enabled)(self)
    }

    /// The active presentation can be changed (true with none open).
    pub fn editable(&self) -> bool {
        self.active().is_none_or(|d| d.access.editable)
    }

    /// The access every presentation opened or created from now on starts with, instead of full
    /// access, so none is editable for a moment before the host applies its own.
    pub fn set_new_document_access(&mut self, access: Access) {
        self.new_document_access = access;
    }

    /// Sets presentation `uid`'s access, keeping its content and undo history. Going read-only
    /// keeps what a gesture or an input method already changed, as one undo step, and ends them.
    /// False when no such presentation is open.
    pub fn set_access(&mut self, uid: u64, access: Access) -> bool {
        let Some(i) = self.documents().iter().position(|d| d.uid == uid) else { return false };
        let active = self.active_index() == Some(i);
        if !access.editable {
            if active {
                self.end_composition();
                self.tool = crate::tools::ToolState::default();
            }
            if let Some(st) = self.docs.get_mut(i) {
                if let Some(c) = st.composition.take()
                    && !Arc::ptr_eq(&c.before, &st.doc)
                {
                    crate::push_undo(st, "Type Text".into(), c.before, c.before_selection);
                }
                if let Some(g) = st.interaction.take()
                    && !Arc::ptr_eq(&g.doc, &st.doc)
                {
                    crate::push_undo(st, g.label, g.doc, g.selection);
                }
                // No caret in a presentation that can't be typed into; the shape stays selected.
                st.selection.text = None;
                st.selection.cells = None;
            }
        }
        if let Some(st) = self.docs.get_mut(i) {
            st.access = access;
        }
        true
    }

    /// A command changed read-only presentation `uid` behind [`Session::edit`]: puts back `doc`
    /// and `selection` from before it. True when it did.
    pub(crate) fn undo_read_only_change(&mut self, uid: u64, doc: &Arc<Presentation>, selection: &Selection) -> bool {
        let Some(st) = self.active_mut() else { return false };
        if st.uid != uid || st.access.editable || Arc::ptr_eq(doc, &st.doc) {
            return false;
        }
        st.doc = doc.clone();
        st.selection = selection.clone();
        st.interaction = None;
        st.composition = None;
        st.revision += 1;
        log::warn!("a command changed a read-only presentation; the change was undone");
        true
    }
}

#[cfg(test)]
mod tests;
