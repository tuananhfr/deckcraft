//! UI-level shortcuts, the command palette, and the menu structure (shared with the native macOS
//! menu bar in the desktop app).

use deckcraft_engine::Mods;
use serde_json::json;

use crate::SlideApp;
use crate::theme::{self, Tokens};

/// Shortcuts the UI handles itself (views, show, zoom, palette, file dialogs), in the engine's
/// notation (`Cmd` is Ctrl off the Mac). The first matching row wins and the engine's registry
/// never sees the key; the Keyboard Shortcuts dialog lists these rows.
pub const UI_SHORTCUTS: &[(&str, &str)] = &[
    ("F1", "app.shortcuts"),
    ("F5", "show.start"),
    ("Shift+F5", "show.fromCurrent"),
    ("Cmd+Enter", "show.fromCurrent"),
    ("Cmd+Shift+Enter", "show.fromStart"),
    ("Cmd+S", "file.save"),
    ("Cmd+Shift+S", "app.saveAsDialog"),
    ("F12", "app.saveAsDialog"),
    // File tab: Alt+F opens it on Home with the keyboard in its side menu; Ctrl+P on Print.
    ("Alt+F", "app.backstage"),
    ("Cmd+P", "app.print"),
    ("Cmd+W", "app.closePresentation"),
    ("Cmd+O", "app.openDialog"),
    ("Cmd+F", "app.find"),
    ("Cmd+H", "app.replace"),
    ("Cmd+Shift+P", "app.palette"),
    // Office's "Tell me" search box.
    ("Alt+Q", "app.palette"),
    ("Cmd+,", "app.preferences"),
    ("Cmd+=", "view.zoomIn"),
    ("Cmd++", "view.zoomIn"),
    ("Cmd+-", "view.zoomOut"),
    ("Cmd+0", "view.fit"),
    ("Cmd+F1", "view.collapseRibbon"),
    ("Shift+F9", "view.gridlines"),
    ("Alt+F9", "view.guides"),
    ("Cmd+K", "insert.hyperlink"),
];

/// The command a key runs through [`UI_SHORTCUTS`], if the UI takes it.
pub fn ui_shortcut_command(key: egui::Key, m: Mods) -> Option<&'static str> {
    let name = crate::key_name(key);
    UI_SHORTCUTS.iter().find(|(sc, _)| deckcraft_engine::keys::shortcut_matches(sc, name, m)).map(|(_, id)| *id)
}

/// Runs a [`UI_SHORTCUTS`] key. True when consumed.
pub fn ui_shortcut(app: &mut SlideApp, key: egui::Key, m: Mods) -> bool {
    let Some(id) = ui_shortcut_command(key, m) else { return false };
    match id {
        // Save writes through the host (a download on the web), which `file.save` alone can't.
        "file.save" => app.request_save(),
        "app.backstage" => {
            app.open_backstage(crate::backstage::Page::Home);
            if let Some(b) = app.backstage.as_mut() {
                b.focus = Some(0);
            }
        }
        "app.print" => {
            if app.session.active().is_none() {
                return false;
            }
            app.open_backstage(crate::backstage::Page::Print);
        }
        "insert.hyperlink" => app.dialog = Some(crate::dialogs::Dialog::new("hyperlink")),
        _ => {
            let _ = app.run(id, json!({}));
        }
    }
    true
}

/// ⇧⌘P: search every command by name and run it.
pub fn palette(app: &mut SlideApp, ctx: &egui::Context) {
    let Some((mut query, mut sel)) = app.palette.take() else { return };
    let t = Tokens::get(ctx);
    let mut close = false;
    let mut run: Option<String> = None;
    egui::Window::new("command_palette")
        .title_bar(false)
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 90.0))
        .fixed_size(egui::vec2(520.0, 380.0))
        .show(ctx, |ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut query).hint_text(crate::tl!("Search commands…")).desired_width(f32::INFINITY).font(theme::font(15.0)),
            );
            r.request_focus();
            let q = query.to_lowercase();
            let mut items: Vec<(String, String, Option<&'static str>)> = app
                .session
                .commands()
                .into_iter()
                .filter(|c| c.enabled)
                .map(|c| (c.id.to_string(), c.label.to_string(), c.shortcut))
                .chain(crate::UI_COMMANDS.iter().map(|c| (c.0.to_string(), c.1.to_string(), c.2)))
                .filter(|(id, label, _)| {
                    q.is_empty()
                        || label.to_lowercase().contains(&q)
                        || crate::tl!(label).to_lowercase().contains(&q)
                        || id.to_lowercase().contains(&q)
                })
                .collect();
            items.sort_by_key(|(_, l, _)| !l.to_lowercase().starts_with(&q));
            items.truncate(60);
            let (down, up, enter, esc) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::ArrowDown),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Escape),
                )
            });
            if down {
                sel = (sel + 1).min(items.len().saturating_sub(1));
            }
            if up {
                sel = sel.saturating_sub(1);
            }
            if esc {
                close = true;
            }
            if enter && let Some((id, ..)) = items.get(sel) {
                run = Some(id.clone());
            }
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for (k, (id, label, sc)) in items.iter().enumerate() {
                    let r = ui.horizontal(|ui| {
                        let resp = ui.selectable_label(k == sel, crate::tl!(label));
                        if let Some(s) = sc {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(egui::RichText::new(crate::ribbon::pretty_shortcut(s)).color(t.text_faint));
                            });
                        }
                        resp
                    });
                    if r.inner.clicked() {
                        run = Some(id.clone());
                    }
                }
            });
        });
    if let Some(id) = run {
        let _ = app.run(&id, json!({}));
        close = true;
    }
    if !close {
        app.palette = Some((query, sel));
    }
}

/// The application menu tree: (menu, [(label, command id, shortcut)]), "-" = separator. Used by the
/// native macOS menu bar and as the parity catalogue's live side.
pub fn menu_tree() -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
    vec![
        (
            "File",
            vec![
                ("New Presentation", "file.new"),
                ("Open…", "app.openDialog"),
                ("-", ""),
                ("Close", "app.closePresentation"),
                ("Reopen Closed Presentation", "file.reopenClosed"),
                ("Save", "file.save"),
                ("Save As…", "app.saveAsDialog"),
                ("Export…", "app.exportDialog"),
                ("Print…", "app.print"),
                ("-", ""),
                ("Properties", "file.properties"),
            ],
        ),
        (
            "Edit",
            vec![
                ("Undo", "edit.undo"),
                ("Redo", "edit.redo"),
                ("-", ""),
                ("Cut", "edit.cut"),
                ("Copy", "edit.copy"),
                ("Paste", "edit.paste"),
                ("Paste and Match Formatting", "edit.pasteText"),
                ("-", ""),
                ("Select All", "edit.selectAll"),
                ("Duplicate", "edit.duplicate"),
                ("Delete Slide", "slide.delete"),
                ("-", ""),
                ("Find…", "app.find"),
                ("Replace…", "app.replace"),
            ],
        ),
        (
            "View",
            vec![
                ("Normal", "view.normal"),
                ("Slide Sorter", "view.sorter"),
                ("Notes Page", "view.notesPage"),
                ("Outline View", "view.outline"),
                ("Reading View", "view.reading"),
                ("Slide Show", "show.start"),
                ("-", ""),
                ("Slide Master", "view.slideMaster"),
                ("-", ""),
                ("Ruler", "view.ruler"),
                ("Gridlines", "view.gridlines"),
                ("Guides", "view.guides"),
                ("-", ""),
                ("Zoom In", "view.zoomIn"),
                ("Zoom Out", "view.zoomOut"),
                ("Fit to Window", "view.fit"),
            ],
        ),
        (
            "Insert",
            vec![
                ("New Slide", "slide.new"),
                ("Duplicate Slide", "slide.duplicate"),
                ("-", ""),
                ("Section", "section.add"),
                ("Text Box", "insert.textBox"),
                ("WordArt", "insert.wordArt"),
                ("Date and Time", "insert.dateTime"),
                ("Slide Number", "insert.slideNumber"),
                ("-", ""),
                ("Table…", "insert.table"),
                ("Chart", "insert.chart"),
                ("Picture from File…", "app.insertPictureDialog"),
                ("Audio from File…", "app.insertAudioDialog"),
                ("Video from File…", "app.insertVideoDialog"),
            ],
        ),
        (
            "Format",
            vec![
                ("Bold", "format.bold"),
                ("Italic", "format.italic"),
                ("Underline", "format.underline"),
                ("-", ""),
                ("Align Left", "format.alignLeft"),
                ("Center", "format.alignCenter"),
                ("Align Right", "format.alignRight"),
                ("Justify", "format.justify"),
                ("-", ""),
                ("Right-to-Left Text Direction", "format.rtl"),
            ],
        ),
        (
            "Arrange",
            vec![
                ("Bring to Front", "arrange.bringToFront"),
                ("Send to Back", "arrange.sendToBack"),
                ("Bring Forward", "arrange.bringForward"),
                ("Send Backward", "arrange.sendBackward"),
                ("-", ""),
                ("Group", "arrange.group"),
                ("Ungroup", "arrange.ungroup"),
                ("Regroup", "arrange.regroup"),
                ("-", ""),
                ("Rotate Left 90°", "arrange.rotateLeft"),
                ("Rotate Right 90°", "arrange.rotateRight"),
                ("Flip Horizontal", "arrange.flipHorizontal"),
                ("Flip Vertical", "arrange.flipVertical"),
            ],
        ),
        (
            "Slide Show",
            vec![
                ("Play from Start", "show.fromStart"),
                ("Play from Current Slide", "show.fromCurrent"),
                ("-", ""),
                ("Hide Slide", "slide.hide"),
                ("Set Up Show…", "show.setup"),
            ],
        ),
        ("Help", vec![("About DeckCraft", "app.about"), ("Keyboard Shortcuts", "app.shortcuts"), ("Command Palette", "app.palette")]),
    ]
}
