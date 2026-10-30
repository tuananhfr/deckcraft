//! `erpHost` v2 access modes (`doc.mode`, `doc.canDownload`, `setMode`): what the shell lets the
//! user do with a document, applied to its tab and to its saves.

use deckcraft_engine::Access;
use deckcraft_ui_egui::{ReadOnlyNotice, SlideApp};

use super::documents::HostDocs;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    #[default]
    Edit,
    Comment,
    View,
}

impl Mode {
    /// `doc.mode` or `setMode`'s `mode`: absent is `edit`; one this editor doesn't know is `view`,
    /// so a newer shell never gets more than it gave.
    pub(crate) fn parse(mode: Option<&str>) -> Mode {
        match mode {
            None | Some("edit") => Mode::Edit,
            Some("comment") => Mode::Comment,
            Some(_) => Mode::View,
        }
    }

    /// In phase 2 `comment` is `view` for the content: server comments come in a later milestone.
    pub(crate) fn access(self, can_download: bool) -> Access {
        match self {
            Mode::Edit => Access { editable: true, can_download },
            Mode::Comment | Mode::View => Access::read_only(can_download),
        }
    }
}

/// Applies `mode` to the open document `uid`: its tab, what it says when a change is refused,
/// and whether it is saved.
pub(crate) fn apply(app: &mut SlideApp, docs: &HostDocs, uid: u64, mode: Mode, can_download: bool) {
    let access = mode.access(can_download);
    let notice = if mode == Mode::Comment { ReadOnlyNotice::CommentsLater } else { ReadOnlyNotice::View };
    app.set_read_only_notice(uid, notice);
    app.set_access(uid, access);
    docs.set_read_only(uid, !access.editable);
}

/// `setMode(docId, mode, canDownload)` on an open document, keeping its content and undo history.
pub(crate) fn set_mode(app: &mut SlideApp, docs: &HostDocs, doc_id: &str, mode: Mode, can_download: bool) {
    if let Some(uid) = docs.uid_for(doc_id) {
        apply(app, docs, uid, mode, can_download);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse_and_comment_is_view_for_the_content() {
        assert_eq!(Mode::parse(None), Mode::Edit);
        assert_eq!(Mode::parse(Some("edit")), Mode::Edit);
        assert_eq!(Mode::parse(Some("comment")), Mode::Comment);
        assert_eq!(Mode::parse(Some("view")), Mode::View);
        assert_eq!(Mode::parse(Some("owner")), Mode::View, "unknown modes give the least");
        assert_eq!(Mode::Comment.access(false), Mode::View.access(false));
        assert_eq!(Mode::Edit.access(true), Access::default());
        assert!(!Mode::Comment.access(true).editable);
    }
}
