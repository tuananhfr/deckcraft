//! `open(doc)`: a document from the server becomes a tab, or takes the place of the tab already
//! showing it (the shell reloads the latest version after a conflict, or for a viewer catching up
//! with `preserveView`), and the shell hears how it went (`opened`, the rules of `erpIntakeResult`).

use deckcraft_ui_egui::SlideApp;

use super::documents::HostDocs;
use super::mode::{self, Mode};

/// The only format a presentation is stored in on the server in v1.
const FORMAT: &str = "pptx";

/// `doc` as `open` received it.
pub(crate) struct OpenRequest {
    pub doc_id: String,
    pub version: u64,
    pub title: String,
    pub format: String,
    /// `None` when `bytes` was not a `Uint8Array`.
    pub bytes: Option<Vec<u8>>,
    /// v2; v1 shells get `edit`, `true` and `false`.
    pub mode: Mode,
    pub can_download: bool,
    /// Replace the open tab's content in place, keeping the slide and selection.
    pub preserve_view: bool,
}

/// `opened`'s `reason` when `ok` is false: keys of the shell's intake messages, never text.
pub(crate) mod reason {
    /// Not a format this editor takes: not pptx, a legacy or a password-protected file.
    pub const UNSUPPORTED: &str = "unsupported";
    /// Unreadable bytes, or no room under the open-documents limit.
    pub const PARSER_FAILED: &str = "parserFailed";
}

pub(crate) fn open(app: &mut SlideApp, docs: &HostDocs, request: OpenRequest) {
    let result = open_document(app, docs, &request);
    if let Some(mut host) = docs.host() {
        match result {
            Ok(()) => host.opened(&request.doc_id, true, None),
            Err(reason) => host.opened(&request.doc_id, false, Some(reason)),
        }
    }
}

fn open_document(app: &mut SlideApp, docs: &HostDocs, request: &OpenRequest) -> Result<(), &'static str> {
    if !request.format.eq_ignore_ascii_case(FORMAT) {
        return Err(reason::UNSUPPORTED);
    }
    let Some(bytes) = request.bytes.as_deref() else { return Err(reason::PARSER_FAILED) };
    let previous = docs.uid_for(&request.doc_id).and_then(|uid| index_of(app, uid).map(|i| (uid, i)));
    if let Some((uid, i)) = previous
        && docs.version(uid) == Some(request.version)
        && !docs.dirty(uid)
    {
        // Already showing that version, unchanged: nothing to reload.
        mode::apply(app, docs, uid, request.mode, request.can_download);
        if !request.preserve_view {
            app.session.set_active(i);
        }
        return Ok(());
    }
    if let Some((uid, _)) = previous
        && request.preserve_view
    {
        return reload_in_place(app, docs, request, uid, bytes);
    }
    // At the documents limit the old tab must go first, or the new one has no room.
    let mut replaced = None;
    if let Some((uid, i)) = previous
        && app.session.ensure_room().is_err()
    {
        app.session.close_document(i);
        replaced = Some(docs.forget(uid));
    }
    let name = file_name(&request.title);
    let before: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    let opened = app.open_or_insert_bytes(&name, bytes);
    let new = app.session.active().map(|d| d.uid).filter(|uid| !before.contains(uid));
    let (true, Some(uid)) = (opened, new) else {
        if replaced.is_some()
            && let Some(mut host) = docs.host()
        {
            host.closed(&request.doc_id);
        }
        // Password-protected OOXML is an OLE2 container, which this detects as a legacy file.
        return Err(if deckcraft_engine::legacy_format::detect(&name, bytes).is_some() { reason::UNSUPPORTED } else { reason::PARSER_FAILED });
    };
    let Some(d) = app.session.active_mut() else { return Err(reason::PARSER_FAILED) };
    if let Some(title) = display_title(&request.title) {
        d.title = title;
    }
    // Reopened, it would be a local copy easily taken for the server's document.
    d.reopenable = false;
    let (doc, revision) = (d.saved_doc.clone(), d.revision);
    docs.adopt(uid, &request.doc_id, request.version, FORMAT, doc, revision);
    mode::apply(app, docs, uid, request.mode, request.can_download);
    // Closed without asking: the shell reloads only when the user chose the server's version.
    if replaced.is_none()
        && let Some((old, _)) = previous
        && let Some(i) = index_of(app, old)
    {
        app.session.close_document(i);
        replaced = Some(docs.forget(old));
    }
    if let Some(reported) = replaced {
        docs.inherit_reported(uid, reported);
    }
    Ok(())
}

/// `preserveView`: the newer version replaces tab `uid`'s content, which keeps its place, zoom,
/// slide and selection; the active tab stays the active one.
fn reload_in_place(app: &mut SlideApp, docs: &HostDocs, request: &OpenRequest, uid: u64, bytes: &[u8]) -> Result<(), &'static str> {
    let name = file_name(&request.title);
    let doc = deckcraft_engine::cmd::file::open_presentation(&name, bytes)
        .map_err(|e| if matches!(e, deckcraft_engine::EngineError::LegacyFormat(_)) { reason::UNSUPPORTED } else { reason::PARSER_FAILED })?;
    if !app.session.replace_content(uid, doc) {
        return Err(reason::PARSER_FAILED);
    }
    let Some(d) = app.session.document_mut(uid) else { return Err(reason::PARSER_FAILED) };
    if let Some(title) = display_title(&request.title) {
        d.title = title;
    }
    let (doc, revision) = (d.saved_doc.clone(), d.revision);
    // Saved state starts over at the new version; the shell hears it is clean again if it wasn't.
    let reported = docs.forget(uid);
    docs.adopt(uid, &request.doc_id, request.version, FORMAT, doc, revision);
    docs.inherit_reported(uid, reported);
    mode::apply(app, docs, uid, request.mode, request.can_download);
    Ok(())
}

fn index_of(app: &SlideApp, uid: u64) -> Option<usize> {
    app.session.documents().iter().position(|d| d.uid == uid)
}

/// The name the document opens under: its title with the server's format, so errors name it.
fn file_name(title: &str) -> String {
    match display_title(title) {
        Some(title) => format!("{title}.{FORMAT}"),
        None => format!("Presentation.{FORMAT}"),
    }
}

/// The title without a `.pptx` the server may have kept; `None` when nothing is left.
fn display_title(title: &str) -> Option<String> {
    let title = title.trim();
    let stem = match title.len().checked_sub(FORMAT.len() + 1) {
        Some(at) if title.is_char_boundary(at) && title[at..].eq_ignore_ascii_case(&format!(".{FORMAT}")) => &title[..at],
        _ => title,
    };
    let stem = stem.trim();
    (!stem.is_empty()).then(|| stem.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_drop_only_a_pptx_extension() {
        assert_eq!(display_title("Báo cáo quý.pptx").as_deref(), Some("Báo cáo quý"));
        assert_eq!(display_title("Kế hoạch.PPTX").as_deref(), Some("Kế hoạch"));
        assert_eq!(display_title("v1.2 kế hoạch").as_deref(), Some("v1.2 kế hoạch"));
        assert_eq!(display_title("a/b"), Some("a/b".into()), "a title is never a path");
        assert_eq!(display_title(" .pptx "), None);
        assert_eq!(display_title("ạ"), Some("ạ".into()));
        assert_eq!(file_name("Kế hoạch.pptx"), "Kế hoạch.pptx");
    }
}
