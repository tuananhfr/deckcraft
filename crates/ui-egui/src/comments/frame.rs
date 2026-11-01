//! One frame of the comment calls: the shell's events, the selection's anchor and the clicks on
//! markers.

use std::sync::Arc;

use deckcraft_engine::Target;
use deckcraft_engine::anchors::{Anchor, Resolution};

use super::{AnchorStatus, CommentEvent, MAX_THREADS, ResolvedThread, Thread};
use crate::SlideApp;

/// Characters of the slide title the label shows.
const LABEL_TITLE_CHARS: usize = 60;

/// `label` of an anchor: the slide as people count it, and its title.
fn label(anchor: &Anchor) -> String {
    let slide = crate::i18n::fmt(crate::tl!("Slide {n}"), &[("n", &anchor.index.saturating_add(1).to_string())]);
    if anchor.title.is_empty() {
        return slide;
    }
    let mut title: String = anchor.title.chars().take(LABEL_TITLE_CHARS).collect();
    if anchor.title.chars().count() > LABEL_TITLE_CHARS {
        title.push('…');
    }
    format!("{slide} · {title}")
}

fn anchor_json(anchor: &Anchor) -> String {
    anchor.to_json(&label(anchor))
}

impl SlideApp {
    /// Follow presentation `uid` (the host document opened last): the threads are about it.
    pub fn comments_bind(&mut self, uid: Option<u64>) {
        if let Some(c) = self.comments.as_mut()
            && c.doc != uid
        {
            c.doc = uid;
            c.selection_key = None;
        }
    }

    /// One step of the comment calls; `true` when the shell's calls changed what is shown.
    pub fn comments_step(&mut self) -> bool {
        let Some(comments) = self.comments.as_ref() else { return false };
        let uid = comments.doc;
        let events = std::mem::take(&mut *comments.inbox.borrow_mut());
        let changed = !events.is_empty();
        for event in events {
            match event {
                CommentEvent::Show(threads) => {
                    if let Some(c) = self.comments.as_mut() {
                        c.threads = threads
                            .into_iter()
                            .take(MAX_THREADS)
                            .map(|t| {
                                let anchor = Anchor::parse(&t.anchor).ok();
                                (t, anchor)
                            })
                            .collect();
                    }
                }
                CommentEvent::Focus(anchor) => {
                    if let (Some(uid), Ok(anchor)) = (uid, Anchor::parse(&anchor)) {
                        self.focus_anchor(uid, &anchor);
                    }
                }
                CommentEvent::Resolve { threads, reply } => reply(uid.map(|uid| self.resolve_threads(uid, &threads)).unwrap_or_default()),
            }
        }
        if let Some(c) = self.comments.as_mut()
            && let Some(thread) = c.clicked.take()
        {
            c.host.comment_clicked(&thread);
        }
        self.refresh_selection_anchor();
        changed
    }

    fn refresh_selection_anchor(&mut self) {
        let Some(comments) = self.comments.as_ref() else { return };
        let st = comments.doc.and_then(|uid| self.session.documents().iter().find(|d| d.uid == uid));
        let key = st.map(|d| (d.uid, Arc::as_ptr(&d.doc) as usize, d.selection.clone()));
        if comments.selection_key == key {
            return;
        }
        let anchor = key.as_ref().and_then(|(uid, _, _)| self.session.anchor_of_selection(*uid));
        if let Some(c) = self.comments.as_mut() {
            c.selection_key = key;
            *c.selection.borrow_mut() = anchor.as_ref().map(anchor_json);
        }
    }

    /// Show presentation `uid` at `anchor`: its tab, its slide, its shape selected.
    fn focus_anchor(&mut self, uid: u64, anchor: &Anchor) {
        let Some(st) = self.session.documents().iter().find(|d| d.uid == uid) else { return };
        let Some((index, shape)) = deckcraft_engine::anchors::place(&st.doc, anchor) else { return };
        if let Some(tab) = self.session.documents().iter().position(|d| d.uid == uid) {
            self.session.set_active(tab);
        }
        let _ = self.session.select(|_, sel| {
            sel.target = Target::Slides;
            sel.slide = index;
            sel.slides.clear();
            sel.shapes = shape.into_iter().collect();
            sel.text = None;
            sel.cells = None;
        });
    }

    /// `resolveAnchors`: each thread's anchor found again in presentation `uid`, in order; an
    /// orphaned one keeps its anchor. A thread whose anchor this editor can't read (a newer
    /// editor's) is left out: nothing here can tell whether it still points somewhere.
    fn resolve_threads(&self, uid: u64, threads: &[Thread]) -> Vec<ResolvedThread> {
        // Threads past the cap are left out of the answer, which the shell reads as unchanged.
        let readable: Vec<(&Thread, Anchor)> = threads.iter().take(MAX_THREADS).filter_map(|t| Some((t, Anchor::parse(&t.anchor).ok()?))).collect();
        let anchors: Vec<Anchor> = readable.iter().map(|(_, a)| a.clone()).collect();
        let resolved = self.session.resolve_anchors(uid, &anchors);
        readable
            .into_iter()
            .zip(resolved)
            .map(|((t, _), r)| match r {
                Resolution::Open(anchor) => ResolvedThread { id: t.id.clone(), anchor: anchor_json(&anchor), status: AnchorStatus::Open },
                Resolution::Orphaned => ResolvedThread { id: t.id.clone(), anchor: t.anchor.clone(), status: AnchorStatus::Orphaned },
            })
            .collect()
    }
}
