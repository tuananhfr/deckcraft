//! A newer version of an open presentation taking its place in the same tab (the office site
//! reloading a document a viewer has open), keeping what the user was looking at.

use std::sync::Arc;

use deckcraft_model::Presentation;

use crate::{Selection, Session, Target};

impl Session {
    /// Replaces presentation `uid`'s content with `doc`, as saved. The current slide stays (by
    /// position) and so do the selected shapes that still exist there. Its undo history is
    /// dropped: undoing would bring back the old version. False when `uid` isn't open.
    pub fn replace_content(&mut self, uid: u64, doc: Presentation) -> bool {
        let Some(i) = self.docs.iter().position(|d| d.uid == uid) else { return false };
        if self.active_index() == Some(i) {
            self.end_composition();
            self.tool = crate::tools::ToolState::default();
        }
        let missing_fonts = crate::fonts::missing_fonts(&doc);
        let Some(st) = self.docs.get_mut(i) else { return false };
        let doc = Arc::new(doc);
        st.selection = kept_selection(&st.selection, &doc);
        st.doc = doc.clone();
        st.saved_doc = doc;
        st.history = crate::History { limit: st.history.limit, ..Default::default() };
        st.interaction = None;
        st.composition = None;
        st.missing_fonts = missing_fonts;
        st.revision += 1;
        crate::clamp_selection(st);
        true
    }
}

/// The selection moved onto `new`: the slide at the same position and the selected shapes
/// still on it (shape ids come from the file, slide ids don't survive a PPTX import). Carets and
/// table cells are dropped, as the paragraph, character or cell they point at may be gone.
fn kept_selection(sel: &Selection, new: &Presentation) -> Selection {
    let slide = sel.slide.min(new.slides.len().saturating_sub(1));
    let shapes = if sel.target == Target::Slides { sel.shapes.clone() } else { vec![] };
    Selection { slide, shapes, ..Default::default() }
}

#[cfg(test)]
mod tests;
