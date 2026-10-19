//! Help › Keyboard Shortcuts (F1): every shortcut and the command it runs, built from the tables
//! the key dispatch reads, so the list can't drift from what the keys do.

use egui::{Ui, vec2};

use crate::SlideApp;
use crate::dialogs::Dialog;
use crate::theme::{self, Tokens};

pub struct Row {
    /// In the engine's notation (`Cmd+Shift+S`).
    pub shortcut: &'static str,
    pub id: &'static str,
    /// English; drawn translated.
    pub label: &'static str,
}

impl Row {
    /// The keys as this platform writes them (`Ctrl+Shift+S` on Windows).
    pub fn keys(&self) -> String {
        crate::ribbon::pretty_shortcut(self.shortcut)
    }
}

fn label_of(id: &str) -> &'static str {
    deckcraft_engine::find_command(id).map(|c| c.label).or_else(|| crate::UI_COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1)).unwrap_or("")
}

/// Every shortcut in dispatch order: the UI's own, then the engine's registry and its extra
/// shortcuts. A key an earlier row already takes is left out, since its command never sees it.
pub fn rows() -> Vec<Row> {
    use deckcraft_engine::keys::{EXTRA_SHORTCUTS, parse_shortcut};
    let engine = deckcraft_engine::command_specs().iter().filter_map(|c| c.shortcut.map(|s| (s, c.id)));
    let mut out: Vec<Row> = Vec::new();
    let mut taken = Vec::new();
    for (shortcut, id) in crate::menus::UI_SHORTCUTS.iter().copied().chain(engine).chain(EXTRA_SHORTCUTS.iter().copied()) {
        let Some(keys) = parse_shortcut(shortcut) else { continue };
        if taken.contains(&keys) {
            continue;
        }
        taken.push(keys);
        out.push(Row { shortcut, id, label: label_of(id) });
    }
    out
}

/// The rows whose label (English or translated) or keys contain `query`, ignoring case.
pub fn filter<'a>(rows: &'a [Row], query: &str) -> Vec<&'a Row> {
    let q = query.trim().to_lowercase();
    rows.iter()
        .filter(|r| {
            q.is_empty()
                || r.label.to_lowercase().contains(&q)
                || crate::tl!(r.label).to_lowercase().contains(&q)
                || r.keys().to_lowercase().contains(&q)
                || r.shortcut.to_lowercase().contains(&q)
        })
        .collect()
}

/// The dialog's body; true when it should close.
pub fn body(_app: &mut SlideApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let first = !d.fields.contains_key("query");
    let mut query = d.fields.get("query").cloned().unwrap_or_default();
    let r = ui.add(egui::TextEdit::singleline(&mut query).hint_text(crate::tl!("Search shortcuts…")).desired_width(f32::INFINITY));
    if first {
        r.request_focus();
    }
    let all = rows();
    let shown = filter(&all, &query);
    d.fields.insert("query".into(), query);
    ui.add_space(6.0);
    // A fixed list height keeps Close in place while the search narrows the rows.
    egui::ScrollArea::vertical().max_height(420.0).auto_shrink([true, false]).show(ui, |ui| {
        ui.set_min_width(420.0);
        if shown.is_empty() {
            ui.label(egui::RichText::new(crate::tl!("No shortcuts match.")).color(t.text_faint));
        }
        egui::Grid::new("shortcut_rows").num_columns(2).striped(true).spacing([24.0, 4.0]).show(ui, |ui| {
            for row in shown {
                ui.label(egui::RichText::new(row.keys()).font(theme::bold(12.5)));
                ui.label(crate::tl!(row.label));
                ui.end_row();
            }
        });
    });
    ui.add_space(8.0);
    let mut close = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        close = ui.add(egui::Button::new(crate::tl!("Close")).min_size(vec2(72.0, 24.0))).clicked();
    });
    close
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use deckcraft_engine::keys::{key_token, parse_shortcut, shortcut_command};

    use super::*;

    /// Each listed shortcut, pressed as an egui key, reaches the command the list names: the UI's
    /// table first, then the engine's registry, as `SlideApp::keyboard` and `Session::key` do.
    /// F1, F12, Ctrl+M, Ctrl+F, Ctrl+H and Alt+Q pressed as real egui keys on the slide.
    #[test]
    fn office_keys_reach_their_commands() {
        use egui::{Key, Modifiers};
        let picked: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
        let sink = picked.clone();
        let services = crate::Services {
            pick_save: Some(Box::new(move |name: &str| {
                sink.borrow_mut().push(name.to_string());
                None
            })),
            ..Default::default()
        };
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(
            |ui, app: &mut SlideApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            SlideApp::new(deckcraft_engine::Session::with_new(), services),
        );
        h.run_steps(2);
        let dialog = |h: &egui_kittest::Harness<'_, SlideApp>| h.state().dialog.as_ref().map(|d| d.id.clone());

        h.key_press(Key::F1);
        h.run_steps(2);
        assert_eq!(dialog(&h).as_deref(), Some("shortcuts"));
        h.event(egui::Event::Text("zoom".into()));
        h.run_steps(2);
        assert_eq!(h.state().dialog.as_ref().unwrap().fields["query"], "zoom", "typing goes to the search field");
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert_eq!(dialog(&h), None);

        h.key_press(Key::F12);
        h.run_steps(1);
        assert_eq!(picked.borrow().len(), 1, "F12 is Save As");

        h.key_press_modifiers(Modifiers::COMMAND, Key::M);
        h.run_steps(1);
        assert_eq!(h.state().session.active().unwrap().doc.slides.len(), 2, "Ctrl+M adds a slide");
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
        h.run_steps(1);
        assert_eq!(h.state().session.active().unwrap().doc.slides.len(), 3, "Ctrl+Shift+N still does");

        h.key_press_modifiers(Modifiers::COMMAND, Key::F);
        h.run_steps(2);
        assert!(h.state().find.visible && !h.state().find.replace, "Ctrl+F opens Find");
        assert!(h.state().palette.is_none(), "Ctrl+F no longer opens the command palette");
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert!(!h.state().find.visible, "Esc in the Find field closes it");
        h.key_press_modifiers(Modifiers::COMMAND, Key::H);
        h.run_steps(2);
        assert!(h.state().find.visible && h.state().find.replace, "Ctrl+H opens Replace");
        h.key_press(Key::Escape);
        h.run_steps(2);
        h.key_press_modifiers(Modifiers::ALT, Key::Q);
        h.run_steps(1);
        assert!(h.state().palette.is_some(), "Alt+Q opens the command palette");
    }

    #[test]
    fn every_listed_shortcut_runs_its_command() {
        let rows = rows();
        assert!(rows.len() > 40, "only {} shortcuts", rows.len());
        for r in &rows {
            assert!(!r.label.is_empty(), "{} runs `{}`, which is no command", r.shortcut, r.id);
            let (token, mods) = parse_shortcut(r.shortcut).unwrap();
            let key = egui::Key::ALL
                .iter()
                .copied()
                .find(|k| key_token(crate::key_name(*k)) == token)
                .unwrap_or_else(|| panic!("{} names no key", r.shortcut));
            let runs = crate::menus::ui_shortcut_command(key, mods).or_else(|| shortcut_command(crate::key_name(key), mods));
            assert_eq!(runs, Some(r.id), "{}", r.shortcut);
        }
    }

    #[test]
    fn the_list_has_the_office_keys_and_no_shadowed_ones() {
        let rows = rows();
        let runs = |keys: &str| rows.iter().filter(|r| r.keys() == crate::ribbon::pretty_shortcut(keys)).map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(runs("F1"), ["app.shortcuts"]);
        assert_eq!(runs("F12"), ["app.saveAsDialog"]);
        assert_eq!(runs("Cmd+M"), ["slide.new"]);
        assert_eq!(runs("Cmd+Shift+N"), ["slide.new"]);
        assert_eq!(runs("Cmd+F"), ["app.find"]);
        assert_eq!(runs("Cmd+H"), ["app.replace"]);
        assert_eq!(runs("Alt+Q"), ["app.palette"]);
        assert_eq!(runs("Cmd+="), ["view.zoomIn"], "the UI's zoom shadows Subscript");
        assert_eq!(runs("Cmd+Shift+V"), ["edit.pasteText"], "the registry's first command wins");
    }

    #[test]
    fn the_search_matches_english_and_translated_labels_and_keys() {
        let rows = rows();
        let ids = |q: &str| filter(&rows, q).iter().map(|r| r.id).collect::<Vec<_>>();
        assert!(ids("new slide").contains(&"slide.new"));
        assert!(ids("f12").contains(&"app.saveAsDialog"));
        crate::i18n::set_current(crate::i18n::Lang::from_code("vi").unwrap());
        assert!(ids("phím tắt").contains(&"app.shortcuts"));
        assert!(ids("Bảng Lệnh").contains(&"app.palette"));
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert!(ids("no such command anywhere").is_empty());
    }
}
