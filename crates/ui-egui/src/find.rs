//! The Find and Replace window (Ctrl+F, Ctrl+H). It stays open beside the slide while you work,
//! and runs `edit.find`, `edit.replace` and `edit.replaceAll`.

use deckcraft_engine::Session;
use egui::{Align2, Key, vec2};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{SlideApp, ViewMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Find,
    With,
}

#[derive(Debug, Default)]
pub struct FindState {
    pub visible: bool,
    /// Replace mode adds the Replace with field and its buttons.
    pub replace: bool,
    pub text: String,
    pub with: String,
    pub match_case: bool,
    pub whole_words: bool,
    /// The last search's outcome (nothing found, how many were replaced).
    pub message: Option<String>,
    focus: Option<Field>,
}

impl FindState {
    /// Opens (or switches) the window, with `text` to look for when given.
    pub fn open(&mut self, replace: bool, text: Option<String>) {
        self.visible = true;
        self.replace = replace;
        if let Some(t) = text {
            self.text = t;
        }
        self.message = None;
        self.focus = Some(Field::Find);
    }

    fn params(&self) -> Value {
        json!({"text": self.text, "with": self.with, "matchCase": self.match_case, "wholeWords": self.whole_words})
    }
}

/// Selected text within one paragraph, which Find starts with (as PowerPoint does).
pub fn selected_text(session: &Session) -> Option<String> {
    let st = session.active()?;
    let t = st.selection.text.as_ref()?;
    let (a, b) = t.ordered();
    if a.0 != b.0 || a == b {
        return None;
    }
    let body = deckcraft_engine::cmd::text::body_of(st, t)?;
    Some(deckcraft_engine::model::edit::text_range(body, a, b))
}

pub fn show(app: &mut SlideApp, ctx: &egui::Context) {
    if !app.find.visible || app.session.active().is_none() || app.backstage_open() {
        return;
    }
    let mut f = std::mem::take(&mut app.find);
    // Replacing would change a read-only presentation; finding still works.
    let editable = app.editable();
    f.replace &= editable;
    let t = Tokens::get(ctx);
    let mut open = true;
    let mut close = false;
    let mut action = None;
    egui::Window::new(crate::tl!(if f.replace { "Replace" } else { "Find" }))
        .id(egui::Id::new("find_replace"))
        .collapsible(false)
        .resizable(false)
        .pivot(Align2::RIGHT_TOP)
        .default_pos(ctx.content_rect().right_top() + vec2(-24.0, 150.0))
        .open(&mut open)
        .show(ctx, |ui| {
            ui.set_min_width(360.0);
            egui::Grid::new("find_fields").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label(crate::tl!("Find what:"));
                let r = ui.add(egui::TextEdit::singleline(&mut f.text).id(egui::Id::new("find_text")).desired_width(240.0));
                if f.focus == Some(Field::Find) {
                    r.request_focus();
                    f.focus = None;
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    action = Some("edit.find");
                    f.focus = Some(Field::Find);
                }
                close |= r.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape));
                ui.end_row();
                if f.replace {
                    ui.label(crate::tl!("Replace with:"));
                    let r = ui.add(egui::TextEdit::singleline(&mut f.with).id(egui::Id::new("find_with")).desired_width(240.0));
                    if f.focus == Some(Field::With) {
                        r.request_focus();
                        f.focus = None;
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        action = Some("edit.replace");
                        f.focus = Some(Field::With);
                    }
                    close |= r.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape));
                    ui.end_row();
                }
            });
            ui.checkbox(&mut f.match_case, crate::tl!("Match case"));
            ui.checkbox(&mut f.whole_words, crate::tl!("Find whole words only"));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let has_text = !f.text.is_empty();
                if ui.add_enabled(has_text, egui::Button::new(crate::tl!("Find Next"))).clicked() {
                    action = Some("edit.find");
                }
                if f.replace {
                    if ui.add_enabled(has_text, egui::Button::new(crate::tl!("Replace"))).clicked() {
                        action = Some("edit.replace");
                    }
                    if ui.add_enabled(has_text, egui::Button::new(crate::tl!("Replace All"))).clicked() {
                        action = Some("edit.replaceAll");
                    }
                } else if editable && ui.button(crate::tl!("Replace…")).clicked() {
                    f.replace = true;
                    f.focus = Some(Field::With);
                }
                close |= ui.button(crate::tl!("Close")).clicked();
            });
            if let Some(m) = &f.message {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(m).color(t.text_faint));
            }
        });
    if let Some(id) = action {
        run(app, &mut f, id);
    }
    f.visible = open && !close;
    app.find = f;
}

fn run(app: &mut SlideApp, f: &mut FindState, id: &str) {
    if f.text.is_empty() {
        return;
    }
    let r = match app.run(id, f.params()) {
        Ok(r) => r,
        Err(e) => {
            f.message = Some(e);
            return;
        }
    };
    if id == "edit.replaceAll" {
        let n = r.get("count").and_then(Value::as_u64).unwrap_or(0);
        f.message = Some(if n == 0 {
            crate::tl!("The search text wasn't found.").to_string()
        } else {
            crate::i18n::fmt(crate::tl!("Replaced {count} occurrences."), &[("count", &n.to_string())])
        });
        return;
    }
    if r.get("found").and_then(Value::as_bool) != Some(true) {
        f.message = Some(crate::tl!("The search text wasn't found.").to_string());
        return;
    }
    // The match is selected on its slide: show it there.
    if matches!(app.ui.view, ViewMode::Sorter | ViewMode::Reading) {
        app.ui.view = ViewMode::Normal;
    }
    f.message = None;
    if r.get("notes").and_then(Value::as_bool) == Some(true) {
        app.ui.notes = true;
        let n = r.get("slide").and_then(Value::as_u64).unwrap_or(0) + 1;
        f.message = Some(crate::i18n::fmt(crate::tl!("Found in the notes of slide {n}."), &[("n", &n.to_string())]));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use egui::{Key, Modifiers};
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    /// Two slides: the title "beta alpha beta" and a text box "Beta"; Find and Replace open.
    fn harness() -> (Harness<'static, SlideApp>, u32, u32) {
        let mut s = Session::with_new();
        s.prefs.autocorrect = false;
        let title = s.doc().unwrap().current_slide().unwrap().shapes[0].id.0;
        s.execute("text.set", &json!({"id": title, "text": "beta alpha beta"})).unwrap();
        s.execute("slide.new", &json!({"layout": "blank"})).unwrap();
        let tb = s.execute("insert.textBox", &json!({"rect": [40, 40, 300, 60], "text": "Beta"})).unwrap()["id"].as_u64().unwrap() as u32;
        s.execute("slide.go", &json!({"index": 0})).unwrap();
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            SlideApp::new(s, crate::Services::default()),
        );
        h.run_steps(2);
        h.key_press_modifiers(Modifiers::COMMAND, Key::H);
        h.run_steps(2);
        (h, title, tb)
    }

    fn button<'a>(h: &'a Harness<'_, SlideApp>, label: &'a str) -> egui_kittest::Node<'a> {
        h.get_by_role_and_label(egui::accesskit::Role::Button, label)
    }

    fn shape_text(h: &Harness<'_, SlideApp>, slide: usize, id: u32) -> String {
        let sl = &h.state().session.active().unwrap().doc.slides[slide];
        deckcraft_engine::model::find_shape(&sl.shapes, deckcraft_engine::model::ShapeId(id)).unwrap().text.as_ref().unwrap().text()
    }

    #[test]
    fn enter_finds_the_next_match_and_the_window_stays_open() {
        let (mut h, title, tb) = harness();
        h.event(egui::Event::Text("beta".into()));
        h.run_steps(1);
        let sel = |h: &Harness<'_, SlideApp>| {
            let st = h.state().session.active().unwrap();
            let t = st.selection.text.clone().unwrap();
            (st.selection.slide, t.shape.0, t.anchor, t.caret)
        };
        h.key_press(Key::Enter);
        h.run_steps(2);
        assert_eq!(sel(&h), (0, title, (0, 0), (0, 4)));
        h.key_press(Key::Enter);
        h.run_steps(2);
        assert_eq!(sel(&h), (0, title, (0, 11), (0, 15)), "the Find field keeps the keyboard");
        button(&h, "Find Next").click();
        h.run_steps(2);
        assert_eq!(sel(&h), (1, tb, (0, 0), (0, 4)));
        assert!(h.state().find.visible);
        h.get_by_label("Match case").click();
        h.run_steps(1);
        button(&h, "Find Next").click();
        h.run_steps(2);
        assert_eq!(sel(&h), (0, title, (0, 0), (0, 4)), "Beta no longer matches: wrapped to slide 1");
    }

    #[test]
    fn replace_and_replace_all_report_what_they_did() {
        let (mut h, title, tb) = harness();
        h.event(egui::Event::Text("beta".into()));
        h.run_steps(1);
        h.state_mut().find.with = "gamma".into();
        button(&h, "Replace").click();
        h.run_steps(2);
        assert_eq!(shape_text(&h, 0, title), "beta alpha beta", "the first Replace finds the match");
        button(&h, "Replace").click();
        h.run_steps(2);
        assert_eq!(shape_text(&h, 0, title), "gamma alpha beta");
        button(&h, "Replace All").click();
        h.run_steps(2);
        assert_eq!(shape_text(&h, 0, title), "gamma alpha gamma");
        assert_eq!(shape_text(&h, 1, tb), "gamma");
        assert_eq!(h.state().find.message.as_deref(), Some("Replaced 2 occurrences."));
        button(&h, "Replace All").click();
        h.run_steps(2);
        assert_eq!(h.state().find.message.as_deref(), Some("The search text wasn't found."));
    }
}
