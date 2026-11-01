//! `regionConflict(regionId)`: after a rejoin, someone else had changed a region the user changed
//! while the page had no room. The user picks here: "Giữ bản của tôi" (the shell sends this page's
//! edit), "Dùng bản mới" (the shell drops it and applies the room's content).

use egui::{Align2, vec2};

use super::Choice;
use crate::SlideApp;

impl SlideApp {
    /// Answer the first conflict waiting with `choice`.
    pub fn answer_live_conflict(&mut self, choice: Choice) {
        let Some(live) = self.live.as_mut() else { return };
        if live.conflicts.is_empty() {
            return;
        }
        let prompt = live.conflicts.remove(0);
        if choice == Choice::Mine {
            self.restore_mine(&prompt.region);
        }
        (prompt.reply)(choice);
    }

    /// Put this page's last sent version of `region` back, in case the room's was applied here
    /// meanwhile; the shell sends it to the room itself.
    fn restore_mine(&mut self, region: &str) {
        let Some(live) = self.live.as_ref() else { return };
        let (Some(uid), Some(mine)) = (live.doc, live.sent.get(region).cloned()) else { return };
        let _ = self.session.apply_remote(uid, region, &mine);
    }

    /// The first conflict waiting for the user, as a dialog.
    pub fn live_conflict_prompt(&mut self, ctx: &egui::Context) {
        if self.live.as_ref().is_none_or(|l| l.conflicts.is_empty()) {
            return;
        }
        let mut choice = None;
        egui::Window::new(crate::tl!("Editing together"))
            .id(egui::Id::new("live-conflict"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, -40.0))
            .show(ctx, |ui| {
                ui.set_max_width((ctx.content_rect().width() - 48.0).clamp(160.0, 420.0));
                ui.label(crate::tl!("Someone else changed this part while you were offline."));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(crate::tl!("Keep mine")).clicked() {
                        choice = Some(Choice::Mine);
                    }
                    if ui.button(crate::tl!("Use theirs")).clicked() {
                        choice = Some(Choice::Theirs);
                    }
                });
            });
        if let Some(choice) = choice {
            self.answer_live_conflict(choice);
        }
    }
}
