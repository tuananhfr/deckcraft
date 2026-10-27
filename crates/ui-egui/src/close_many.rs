//! Closing several presentations at once (Close All, Close Others, closing the window): one prompt
//! lists every unsaved one with Save All, Don't Save and Cancel, instead of a prompt each. Save All
//! saves them one after another, each only once the one before has finished; a save that is
//! cancelled or fails stops there, and nothing is closed.

use egui::{Ui, vec2};
use serde_json::json;

use crate::SlideApp;
use crate::dialogs::Dialog;
use crate::theme::Tokens;

/// The dialog's id.
pub const DIALOG: &str = "closeMany";

/// What follows once the presentations are closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    Stay,
    /// The window closes too (the desktop app quits).
    Quit,
    #[cfg(not(target_arch = "wasm32"))]
    Update,
}

pub struct CloseBatch {
    /// The presentations to close, by uid, in tab order.
    pub targets: Vec<u64>,
    /// Those of them unsaved when the prompt opened: the ones it lists.
    unsaved: Vec<u64>,
    pub after: After,
    /// Save All was pressed: the unsaved presentations not saved yet, next first.
    queue: Option<Vec<u64>>,
    /// The presentation whose save was started last.
    started: Option<u64>,
    /// That save went through the host's save picker and ended later (the web). The next one
    /// waits for a click: a browser opens its save picker only during a click or key press.
    needs_click: bool,
}

/// Where Save All stands, after doing what it could this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Every unsaved presentation was saved.
    Done,
    /// A save is in the host's picker or being written.
    Waiting(u64),
    /// The next save needs a click of its own.
    NeedsClick(u64),
    /// A save was cancelled or failed.
    Stopped,
}

impl SlideApp {
    /// Presentation `uid` has unsaved changes, or a save of it hasn't finished.
    fn unsaved(&self, uid: u64) -> bool {
        self.session.documents().iter().any(|d| d.uid == uid && d.is_dirty()) || self.save_in_flight(uid)
    }

    /// Closes the presentations `uids`. When some have unsaved changes, one prompt asks about all
    /// of them first.
    pub fn close_documents(&mut self, uids: Vec<u64>, after: After) {
        let unsaved: Vec<u64> = uids.iter().copied().filter(|uid| self.unsaved(*uid)).collect();
        if !unsaved.is_empty() {
            self.close_batch = Some(CloseBatch { targets: uids, unsaved, after, queue: None, started: None, needs_click: false });
            self.dialog = Some(Dialog::new(DIALOG));
        } else {
            self.close_now(&uids);
        }
    }

    /// Close All: every open presentation.
    pub fn close_all(&mut self) {
        let all = self.session.documents().iter().map(|d| d.uid).collect();
        self.close_documents(all, After::Stay);
    }

    /// Close Others: every presentation but `keep`.
    pub fn close_others(&mut self, keep: u64) {
        let others = self.session.documents().iter().map(|d| d.uid).filter(|uid| *uid != keep).collect();
        self.close_documents(others, After::Stay);
    }

    fn close_now(&mut self, uids: &[u64]) {
        for uid in uids {
            if let Some(i) = self.session.documents().iter().position(|d| d.uid == *uid) {
                let _ = self.run("file.close", json!({"index": i}));
            }
        }
    }

    /// Saves the batch's unsaved presentations in order, as far as it can go without waiting.
    /// `clicked`: the user just asked for the next save, so a save picker may open.
    fn advance(&mut self, b: &mut CloseBatch, mut clicked: bool) -> Step {
        let Some(queue) = b.queue.as_mut() else { return Step::Stopped };
        loop {
            let Some(&uid) = queue.first() else { return Step::Done };
            let Some(i) = self.session.documents().iter().position(|d| d.uid == uid) else {
                queue.remove(0);
                continue;
            };
            if self.save_in_flight(uid) {
                b.needs_click = true;
                return Step::Waiting(uid);
            }
            if !self.session.documents().get(i).is_some_and(|d| d.is_dirty()) {
                queue.remove(0);
                continue;
            }
            if b.started == Some(uid) {
                return Step::Stopped;
            }
            if b.needs_click && !clicked {
                return Step::NeedsClick(uid);
            }
            clicked = false;
            b.needs_click = false;
            b.started = Some(uid);
            self.save_document(i);
            if self.awaiting_loss_confirmation(uid) {
                return Step::Waiting(uid);
            }
        }
    }

    /// Closes the batch's presentations and, for a quit, the window.
    fn finish(&mut self, b: &CloseBatch, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        if b.after == After::Update {
            if !self.has_unsaved_changes() {
                self.continue_update(ctx);
            }
            return;
        }
        // A presentation still unsaved here (edited from elsewhere since) stays open.
        let closing: Vec<u64> = b.targets.iter().copied().filter(|uid| !self.unsaved(*uid)).collect();
        self.close_now(&closing);
        if b.after == After::Quit && self.session.documents().is_empty() {
            self.quit_confirmed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn discard_and_finish(&mut self, b: &CloseBatch, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        if b.after == After::Update {
            self.continue_update(ctx);
            return;
        }
        let targets = b.targets.clone();
        self.close_now(&targets);
        if b.after == After::Quit {
            let _ = self.session.execute("file.recovery.discard", &json!({}));
            self.quit_confirmed = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// The prompt's body; true when it should close.
pub fn body(app: &mut SlideApp, ui: &mut Ui, _d: &mut Dialog) -> bool {
    let Some(mut b) = app.close_batch.take() else { return true };
    let close = body_with(app, ui, &mut b);
    if !close {
        app.close_batch = Some(b);
    }
    close
}

fn body_with(app: &mut SlideApp, ui: &mut Ui, b: &mut CloseBatch) -> bool {
    let ctx = ui.ctx().clone();
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let step = if b.queue.is_some() { Some(app.advance(b, false)) } else { None };
    match step {
        Some(Step::Done) => {
            app.finish(b, &ctx);
            return true;
        }
        Some(Step::Stopped) => return true,
        _ => {}
    }
    if step.is_none() && !b.targets.iter().any(|uid| app.unsaved(*uid)) {
        // Saved or closed from elsewhere since the prompt opened.
        app.finish(b, &ctx);
        return true;
    }
    let rows: Vec<(u64, String)> =
        b.unsaved.iter().filter_map(|uid| app.session.documents().iter().find(|d| d.uid == *uid)).map(|d| (d.uid, d.title())).collect();
    let question = match rows.as_slice() {
        [(_, name)] => crate::i18n::fmt(crate::tl!("Do you want to save the changes you made to {what}?"), &[("what", &format!("“{name}”"))]),
        _ => crate::tl!("Do you want to save the changes to these presentations?").to_string(),
    };
    ui.label(egui::RichText::new(question).strong());
    if rows.len() > 1 {
        ui.add_space(4.0);
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            for (uid, name) in &rows {
                let state = match app.save_state(*uid) {
                    Some(s @ crate::save_state::SaveState::Saving) => format!(" — {}", s.label()),
                    _ if step.is_some() && !app.unsaved(*uid) => format!(" — {}", crate::tl!("Saved")),
                    _ => String::new(),
                };
                ui.label(format!("•  “{name}”{state}"));
            }
        });
    }
    ui.add_space(4.0);
    ui.label(crate::tl!("Your changes will be lost if you don't save them."));
    ui.add_space(8.0);
    let t = Tokens::get(ui.ctx());
    let primary = |ui: &mut Ui, label: &str| {
        ui.add(egui::Button::new(egui::RichText::new(label).color(t.accent_text)).fill(t.accent).min_size(vec2(72.0, 24.0))).clicked()
    };
    let mut done = false;
    ui.horizontal(|ui| {
        if step.is_none() && ui.add(egui::Button::new(crate::tl!("Don't Save")).min_size(vec2(84.0, 24.0))).clicked() {
            app.discard_and_finish(b, &ctx);
            done = true;
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match step {
                None => {
                    let label = if rows.len() == 1 { crate::tl!("Save") } else { crate::tl!("Save All") };
                    if primary(ui, label) || enter {
                        b.queue = Some(b.targets.iter().copied().filter(|uid| app.unsaved(*uid)).collect());
                        done = settle(app, b, &ctx);
                    }
                }
                Some(Step::NeedsClick(uid)) => {
                    let name = app.session.documents().iter().find(|d| d.uid == uid).map(|d| d.title()).unwrap_or_default();
                    if primary(ui, &crate::i18n::fmt(crate::tl!("Save {name}"), &[("name", &format!("“{name}”"))])) || enter {
                        done = settle(app, b, &ctx);
                    }
                }
                _ => {
                    ui.add_enabled(false, egui::Button::new(crate::tl!("Saving…")).min_size(vec2(72.0, 24.0)));
                }
            }
            if !done && ui.add(egui::Button::new(crate::tl!("Cancel")).min_size(vec2(72.0, 24.0))).clicked() {
                done = true;
            }
        });
    });
    done
}

/// Runs the batch's saves after a click; true when the prompt is finished with.
fn settle(app: &mut SlideApp, b: &mut CloseBatch, ctx: &egui::Context) -> bool {
    match app.advance(b, true) {
        Step::Done => {
            app.finish(b, ctx);
            true
        }
        Step::Stopped => true,
        Step::Waiting(_) | Step::NeedsClick(_) => false,
    }
}
