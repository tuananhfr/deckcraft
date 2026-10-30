//! Status bar (slide counter, language, notes/comments toggles, view buttons, zoom slider) and the
//! notes pane under the slide.

use egui::{Align2, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::status_fit::{StatusFit, StatusItem};
use crate::theme::{self, Tokens};
use crate::{SlideApp, ViewMode};

pub fn status_bar(app: &mut SlideApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("status").exact_size(28.0).frame(egui::Frame::NONE.fill(t.chrome)).show(ui, |ui| {
        let rect = ui.max_rect();
        if app.layout.is_narrow() {
            narrow_status_bar(app, ui, rect);
            return;
        }
        if app.session.active().is_none() {
            return;
        }
        wide_status_bar(app, ui, rect);
    });
}

/// Status-bar spacing, in points: an icon button and the gap between items on the right.
const ICON_W: f32 = 26.0;
const GAP: f32 = 4.0;
/// Room a fresh message gets before lower-priority items give way to it.
const MESSAGE_MIN: f32 = 160.0;

/// The status bar of a window wider than a phone: the slide counter, the language and a fresh
/// message on the left; the Notes and Comments switches, view buttons and zoom on the right. When
/// they don't all fit, the lowest-priority ones give way ([`StatusFit`]) instead of overlapping.
fn wide_status_bar(app: &mut SlideApp, ui: &mut Ui, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let font = theme::font(12.0);
    let left = slide_counter(app);
    let message = fresh_message(app, ui);
    let pct = (app.ui.zoom.unwrap_or(app.canvas_scale) * 100.0).round();
    let language = crate::tl!("English (United States)");
    let fit = {
        let text_w = |s: &str| ui.fonts_mut(|f| f.layout_no_wrap(s.to_string(), font.clone(), t.text).size().x);
        let counter_w = text_w(&left);
        let label_w = |l: &str| labeled_width(ui, l) - ICON_W;
        let fixed = 14.0 + counter_w + 20.0 + (ICON_W + GAP) + text_w(&format!("{pct}%")) + GAP + 16.0 + 2.0 * (ICON_W + GAP) + 8.0;
        let fixed = fixed + if message.is_some() { 20.0 + MESSAGE_MIN } else { 0.0 };
        let language_w = (106.0 - counter_w).max(20.0) + text_w(language);
        let labels_w = label_w("Comments") + label_w("Notes");
        StatusFit::new(rect.width(), fixed, |item| match item {
            StatusItem::Language => language_w,
            StatusItem::ZoomSlider => 110.0 + GAP,
            StatusItem::ToggleLabels => labels_w,
            StatusItem::ViewButtons => 4.0 * (ICON_W + GAP),
            StatusItem::ZoomButtons => 2.0 * (ICON_W + GAP),
        })
    };
    let p = ui.painter().clone();
    let shown = p.text(pos2(rect.min.x + 14.0, rect.center().y), Align2::LEFT_CENTER, &left, font.clone(), t.text_dim);
    described(ui, shown, "status_counter", &left);
    let mut end = shown.max.x;
    if fit.shows(StatusItem::Language) {
        // The English columns are minimums: translated text is longer and pushes the next one right.
        let x = (rect.min.x + 120.0).max(shown.max.x + 20.0);
        let shown = p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, language, font.clone(), t.text_dim);
        described(ui, shown, "status_language", language);
        end = shown.max.x;
    }
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(rect.max.x - 560.0, rect.min.y), pos2(rect.max.x - 8.0, rect.max.y)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    right.spacing_mut().item_spacing.x = GAP;
    if small_icon(&mut right, Icon::FitSmall, "Fit slide to current window", false).clicked() {
        app.ui.zoom = None;
    }
    let lbl = right.add(egui::Label::new(egui::RichText::new(format!("{pct}%")).size(12.0).color(t.text_dim)).sense(Sense::click()));
    if lbl.clicked() {
        app.dialog = Some(crate::dialogs::Dialog::new("zoom"));
    }
    if fit.shows(StatusItem::ZoomButtons) && small_icon(&mut right, Icon::ZoomIn, "Zoom In", false).clicked() {
        let _ = app.run("view.zoomIn", json!({}));
    }
    if fit.shows(StatusItem::ZoomSlider) {
        // Slider 10%–400% (log scale).
        let mut z = app.ui.zoom.unwrap_or(app.canvas_scale).clamp(0.1, 4.0);
        let mut lz = z.ln();
        let s = right.add_sized(vec2(110.0, 18.0), egui::Slider::new(&mut lz, (0.1f32).ln()..=(4.0f32).ln()).show_value(false));
        if s.changed() {
            z = lz.exp();
            app.ui.zoom = Some(z);
        }
    }
    if fit.shows(StatusItem::ZoomButtons) && small_icon(&mut right, Icon::ZoomOut, "Zoom Out", false).clicked() {
        let _ = app.run("view.zoomOut", json!({}));
    }
    right.add_space(8.0);
    if fit.shows(StatusItem::ViewButtons) {
        let view = app.ui.view;
        if small_icon(&mut right, Icon::ViewShowSmall, "Slide Show", false).clicked() {
            let i = app.session.active().map(|d| d.selection.slide).unwrap_or(0);
            app.start_show(i, false);
        }
        if small_icon(&mut right, Icon::ViewReadingSmall, "Reading View", false).clicked() {
            let _ = app.run("view.reading", json!({}));
        }
        if small_icon(&mut right, Icon::ViewSorterSmall, "Slide Sorter", view == ViewMode::Sorter).clicked() {
            app.ui.view = ViewMode::Sorter;
        }
        if small_icon(&mut right, Icon::ViewNormalSmall, "Normal", view == ViewMode::Normal).clicked() {
            app.ui.view = ViewMode::Normal;
        }
        right.add_space(8.0);
    }
    let toggle = |ui: &mut Ui, icon: Icon, label: &str, on: bool| {
        if fit.shows(StatusItem::ToggleLabels) { labeled(ui, icon, label, on) } else { small_icon(ui, icon, label, on) }
    };
    let c_on = app.ui.pane.as_deref() == Some("comments");
    if toggle(&mut right, Icon::CommentsSmall, "Comments", c_on).clicked() {
        let _ = app.run("view.pane", json!({"pane": "comments", "toggle": true}));
    }
    let n_on = app.ui.notes;
    if toggle(&mut right, Icon::NotesSmall, "Notes", n_on).clicked() {
        app.ui.notes = !n_on;
    }
    if let Some(msg) = message {
        let right_start = right.min_rect().min.x - 12.0;
        let x = (rect.min.x + 290.0).max(end + 20.0);
        let x = if right_start - x < MESSAGE_MIN { end + 20.0 } else { x };
        if let Some(g) = crate::narrow::elide(ui, &msg, font, t.accent, right_start - x) {
            let shown = Rect::from_min_size(pos2(x, rect.center().y - g.size().y / 2.0), g.size());
            p.galley(shown.min, g, t.accent);
            described(ui, shown, "status_message", &msg);
        }
    }
}

/// The transient status message while it is fresh (shown for 6 s), in the interface language.
fn fresh_message(app: &mut SlideApp, ui: &Ui) -> Option<String> {
    match app.status.clone() {
        Some((msg, at)) if crate::now_ms() - at < 6000.0 => {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            Some(crate::tl!(&msg).to_string())
        }
        Some(_) => {
            app.status = None;
            None
        }
        None => None,
    }
}

/// Names painted status text for screen readers (and tests).
fn described(ui: &Ui, shown: Rect, id: &str, text: &str) {
    ui.interact(shown, ui.id().with(id), Sense::hover()).widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
}

/// "Slide 2 of 9" for the selection, or the master being edited.
fn slide_counter(app: &SlideApp) -> String {
    let Some(st) = app.session.active() else { return String::new() };
    let n = st.doc.slides.len();
    match st.selection.target {
        deckcraft_engine::Target::Slides => crate::i18n::fmt(
            crate::tl!("Slide {n} of {total}"),
            &[("n", &(st.selection.slide + 1).min(n.max(1)).to_string()), ("total", &n.to_string())],
        ),
        _ => crate::tl!("Slide Master").to_string(),
    }
}

/// A phone-width status bar: the thumbnails, notes and comments switches and the zoom on the
/// right; on the left the slide counter, or a fresh status message in its place, cut to the room
/// left. The language, the zoom slider and the view buttons give way (the View tab has them).
fn narrow_status_bar(app: &mut SlideApp, ui: &mut Ui, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(rect.min.x + 8.0, rect.min.y), pos2(rect.max.x - 8.0, rect.max.y)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    right.spacing_mut().item_spacing.x = 4.0;
    let named = small_icon;
    if named(&mut right, Icon::FitSmall, "Fit slide to current window", false).clicked() {
        app.ui.zoom = None;
    }
    let pct = (app.ui.zoom.unwrap_or(app.canvas_scale) * 100.0).round();
    let lbl = right.add(egui::Label::new(egui::RichText::new(format!("{pct}%")).size(12.0).color(t.text_dim)).sense(Sense::click()));
    if lbl.clicked() {
        app.dialog = Some(crate::dialogs::Dialog::new("zoom"));
    }
    right.add_space(4.0);
    let c_on = app.ui.pane.as_deref() == Some("comments");
    if named(&mut right, Icon::CommentsSmall, "Comments", c_on).clicked() {
        let _ = app.run("view.pane", json!({"pane": "comments", "toggle": true}));
    }
    if named(&mut right, Icon::NotesSmall, "Notes", app.ui.notes).clicked() {
        app.ui.notes = !app.ui.notes;
    }
    if app.ui.view == ViewMode::Normal && named(&mut right, Icon::ViewNormalSmall, "Slide Thumbnails", app.layout.thumbs).clicked() {
        let _ = app.run("view.thumbnails", json!({}));
    }
    let room = Rect::from_min_max(pos2(rect.min.x + 12.0, rect.min.y), pos2(right.min_rect().min.x - 8.0, rect.max.y));
    let (text, color) = match fresh_message(app, ui) {
        Some(m) => (m, t.accent),
        None => (slide_counter(app), t.text_dim),
    };
    if let Some(g) = crate::narrow::elide(ui, &text, theme::font(12.0), color, room.width()) {
        let at = pos2(room.min.x, room.center().y - g.size().y / 2.0);
        let shown = Rect::from_min_size(at, g.size()).intersect(room);
        ui.interact(shown, ui.id().with("status_text"), Sense::hover())
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
        ui.painter().with_clip_rect(room).galley(at, g, color);
    }
}

fn small_icon(ui: &mut Ui, icon: Icon, tip: &str, on: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 20.0), Sense::click());
    if on || resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), if on { t.selected } else { t.hover });
    }
    icons::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), icon, t.text_dim, false);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, crate::tl!(tip)));
    resp.on_hover_text(crate::tl!(tip))
}

fn labeled_width(ui: &Ui, label: &str) -> f32 {
    22.0 + ui.fonts_mut(|f| f.layout_no_wrap(crate::tl!(label).into(), theme::font(12.0), egui::Color32::PLACEHOLDER).size().x) + 8.0
}

fn labeled(ui: &mut Ui, icon: Icon, label: &str, on: bool) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let w = labeled_width(ui, label);
    let (r, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
    if on || resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(4), if on { t.selected } else { t.hover });
    }
    icons::paint(ui.painter(), Rect::from_center_size(pos2(r.min.x + 11.0, r.center().y), vec2(15.0, 15.0)), icon, t.text_dim, false);
    ui.painter().text(pos2(r.min.x + 22.0, r.center().y), Align2::LEFT_CENTER, crate::tl!(label), theme::font(12.0), t.text_dim);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, crate::tl!(label)));
    resp
}

/// The speaker notes editor under the slide.
pub fn notes_pane(app: &mut SlideApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else { return };
    if st.selection.target != deckcraft_engine::Target::Slides {
        return;
    }
    let idx = st.selection.slide;
    let uid = st.uid;
    let current = st.current_slide().map(|s| s.notes_text()).unwrap_or_default();
    egui::Panel::bottom("notes")
        .resizable(true)
        .default_size(app.ui.notes_height)
        .size_range(36.0..=400.0)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(8, 6)))
        .show(ui, |ui| {
            app.ui.notes_height = ui.available_height() + 12.0;
            let rect = ui.max_rect();
            ui.painter()
                .line_segment([pos2(rect.min.x - 8.0, rect.min.y - 6.0), pos2(rect.max.x + 8.0, rect.min.y - 6.0)], Stroke::new(1.0, t.separator));
            // Keep a local buffer per slide; commit when it changes and the field loses focus.
            let id = egui::Id::new("notes_edit");
            if !app.editable() {
                // Read-only notes can be read, and selected to copy when downloading is allowed.
                let mut shown = current.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut shown)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY)
                        .desired_rows(2)
                        .font(theme::font(13.0))
                        .interactive(app.can_download()),
                );
                app.notes_buf = (uid, idx, current.clone());
                return;
            }
            let focused = ui.ctx().memory(|m| m.has_focus(id));
            // Focus can leave in the frame before the field is drawn (Escape, a click on a
            // thumbnail): what was typed is still committed, to the slide it was typed for.
            let was_focused = ui.ctx().memory(|m| m.had_focus_last_frame(id));
            if app.notes_buf.0 != uid || app.notes_buf.1 != idx {
                let (old_uid, old_idx, typed) = std::mem::replace(&mut app.notes_buf, (uid, idx, current.clone()));
                let old = app.session.active().and_then(|d| d.doc.slides.get(old_idx)).map(|s| s.notes_text());
                if was_focused && old_uid == uid && old.is_some_and(|o| o != typed) {
                    let _ = app.run("slide.notes", json!({"text": typed, "index": old_idx}));
                }
            }
            if !focused && !was_focused && app.notes_buf.2 != current {
                app.notes_buf.2 = current.clone();
            }
            let mut buf = app.notes_buf.2.clone();
            let r = ui.add(
                egui::TextEdit::multiline(&mut buf)
                    .id(id)
                    .hint_text(crate::tl!("Click to add notes"))
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .desired_rows(2)
                    .font(theme::font(13.0)),
            );
            app.notes_buf.2 = buf.clone();
            if r.has_focus() && buf != current {
                app.track_inline_edit("slide.notes", json!({"text": buf, "index": idx}));
            }
            if r.lost_focus() && buf != current {
                let _ = app.run("slide.notes", json!({"text": buf, "index": idx}));
            }
        });
}
