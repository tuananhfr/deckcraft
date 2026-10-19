//! View settings each open presentation keeps for itself while another one is shown: the zoom.
//! (The canvas and slide-list scroll positions are keyed by document where they are drawn.)

use std::collections::HashMap;

use crate::SlideApp;

#[derive(Default)]
pub struct DocViews {
    /// The presentation `UiState::zoom` describes.
    shown: Option<u64>,
    /// The zoom of the presentations not shown, by uid (`None` = fit to window).
    zooms: HashMap<u64, Option<f32>>,
}

impl SlideApp {
    /// Swaps the zoom when another presentation became active since the last call.
    pub(crate) fn sync_doc_view(&mut self) {
        let active = self.session.active().map(|d| d.uid);
        let views = &mut self.doc_views;
        if views.shown == active {
            return;
        }
        if let Some(prev) = views.shown {
            views.zooms.insert(prev, self.ui.zoom);
            // A presentation opened next to others starts fitted; the first one keeps the saved zoom.
            if let Some(uid) = active {
                self.ui.zoom = views.zooms.remove(&uid).unwrap_or(None);
            }
        }
        views.shown = active;
        let open: Vec<u64> = self.session.documents().iter().map(|d| d.uid).collect();
        views.zooms.retain(|uid, _| open.contains(uid));
    }
}
