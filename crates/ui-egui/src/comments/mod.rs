//! Cloud comments anchored in the presentation (ERPCons M5, the `erpHost` comment calls; web only,
//! `docs/erp-host.md` in erpcons-office). The shell keeps the threads on the server and shows
//! them in its panel; the app tells it where the selection is (`anchorOfSelection`), draws a
//! marker at each open thread's anchor on the slide shown (`showComments`), goes to an anchor
//! (`focusAnchor`), finds anchors again after the presentation changed (`resolveAnchors`) and
//! says which marker was clicked (`commentClicked`). Markers are drawn over the slide only: they
//! are never part of the presentation and never saved. Anchors are
//! [`deckcraft_engine::anchors::Anchor`]s. Without a shell calling these (the desktop, an older
//! shell) [`crate::SlideApp::comments`] is `None` and none of this runs.

mod frame;
mod markers;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;

use deckcraft_engine::Selection;
use deckcraft_engine::anchors::Anchor;

pub(crate) use markers::paint;

/// The shell's side of the comment calls, called by the app.
pub trait CommentHost {
    /// `erpHost.commentClicked(threadId)`.
    fn comment_clicked(&self, thread: &str);
}

/// One thread as `showComments` and `resolveAnchors` give it.
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub id: String,
    /// The anchor's JSON, as the shell stored it.
    pub anchor: String,
    /// `open`, `resolved` or `orphaned`: only open threads get a marker.
    pub status: String,
    pub count: u32,
}

/// Whether `resolveAnchors` found an anchor again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchorStatus {
    Open,
    Orphaned,
}

impl AnchorStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Orphaned => "orphaned",
        }
    }
}

/// One answer of `resolveAnchors`: the anchor as it is now (unchanged when orphaned).
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedThread {
    pub id: String,
    pub anchor: String,
    pub status: AnchorStatus,
}

/// Answers `resolveAnchors`, once.
pub type ResolveReply = Box<dyn FnOnce(Vec<ResolvedThread>)>;

/// What the shell called, applied on the next frame.
pub enum CommentEvent {
    Show(Vec<Thread>),
    /// `focusAnchor(anchor)`, the anchor's JSON.
    Focus(String),
    Resolve {
        threads: Vec<Thread>,
        reply: ResolveReply,
    },
}

pub type CommentInbox = Rc<RefCell<Vec<CommentEvent>>>;
/// The anchor of the selection as of the last frame, as JSON: `anchorOfSelection` answers with it
/// at once.
pub type SelectionAnchor = Rc<RefCell<Option<String>>>;

/// The app's side of the comment calls (web, `erpHost` v2 and later).
pub struct Comments {
    host: Box<dyn CommentHost>,
    inbox: CommentInbox,
    selection: SelectionAnchor,
    /// The presentation the threads are about: the host document opened last.
    doc: Option<u64>,
    /// What the selection anchor was made from: the presentation, its content (by pointer) and
    /// the selection.
    selection_key: Option<(u64, usize, Selection)>,
    /// The threads of the last `showComments`, with their anchors read.
    threads: Vec<(Thread, Option<Anchor>)>,
    /// The thread whose marker was clicked this frame.
    clicked: Option<String>,
}

impl Comments {
    pub fn new(host: Box<dyn CommentHost>) -> Self {
        Comments {
            host,
            inbox: CommentInbox::default(),
            selection: SelectionAnchor::default(),
            doc: None,
            selection_key: None,
            threads: Vec::new(),
            clicked: None,
        }
    }

    /// Where the shell's calls queue their events.
    pub fn inbox(&self) -> CommentInbox {
        self.inbox.clone()
    }

    /// What `anchorOfSelection` reads.
    pub fn selection(&self) -> SelectionAnchor {
        self.selection.clone()
    }
}
