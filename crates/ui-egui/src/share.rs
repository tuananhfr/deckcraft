//! File › Share…: a copy of the presentation, made as Save As makes it, handed to the host's share
//! sheet (`Services::share`), or downloaded when the host doesn't take it.

use crate::{SlideApp, loss_warning::Action};

impl SlideApp {
    /// Share… is offered only by hosts with a share sheet, and only over a presentation.
    pub fn can_share(&self) -> bool {
        self.services.share.is_some() && self.session.active().is_some()
    }

    /// Shares the active presentation in the format Save As would write. Only a pointer click
    /// (`pointer_click`) runs inside the browser event that lets a page open the share sheet;
    /// from a key or a command the copy downloads instead.
    pub fn share_presentation(&mut self, pointer_click: bool) {
        let ext = if self.default_save_format() == "pptx" { "pptx" } else { "deckcraft" };
        if self.warn_save_losses(ext, Action::Share) {
            return;
        }
        let Some(doc) = self.session.active().map(|d| d.doc.clone()) else { return };
        let name = self.suggested_name(ext);
        let bytes = match deckcraft_engine::cmd::file::save_bytes(&doc, ext) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.set_status(e.to_string());
                return;
            }
        };
        let shared = pointer_click && self.services.share.as_mut().is_some_and(|share| share(&name, &bytes));
        if !shared {
            self.download(&name, Ok(bytes));
        }
    }
}

#[cfg(test)]
mod tests;
