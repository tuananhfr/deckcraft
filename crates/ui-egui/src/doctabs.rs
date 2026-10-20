//! Document tabs under the title bar, one per open presentation, shown once two or more are open:
//! without them a second file would hide the first, unsaved work and all.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};
use serde_json::json;

use crate::SlideApp;
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};

/// Height of the tab row.
pub const HEIGHT: f32 = 26.0;
const MAX_W: f32 = 200.0;
const MIN_W: f32 = 56.0;
/// Tab width once the row scrolls: wide enough to read a name.
const SCROLL_W: f32 = 120.0;

/// The tab row shows only with more than one presentation open.
pub fn visible(app: &SlideApp) -> bool {
    app.session.documents().len() > 1
}

enum Action {
    Activate(usize),
    Close(u64),
    CloseOthers(u64),
    CloseAll,
    Reopen,
}

pub fn show(app: &mut SlideApp, ui: &mut Ui) {
    if !visible(app) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let tabs: Vec<Tab> = app
        .session
        .documents()
        .iter()
        .map(|d| Tab { uid: d.uid, title: d.title(), dirty: d.is_dirty(), state: app.save_state(d.uid).map(|s| s.label()) })
        .collect();
    let active = app.session.active_index();
    let can_reopen = !app.session.closed.is_empty();
    let mut action = None;
    egui::Panel::top("doc_tabs").exact_size(HEIGHT).frame(egui::Frame::NONE.fill(t.chrome)).show(ui, |ui| {
        let rect = ui.max_rect();
        let w = ((rect.width() - 16.0) / tabs.len() as f32).clamp(MIN_W, MAX_W);
        ui.painter().line_segment([rect.left_bottom() - vec2(0.0, 0.5), rect.right_bottom() - vec2(0.0, 0.5)], Stroke::new(1.0, t.separator));
        let row = Rect::from_min_max(pos2(rect.min.x + 8.0, rect.min.y), pos2(rect.max.x - 8.0, rect.max.y));
        if w * tabs.len() as f32 <= row.width() + 0.5 {
            action = tab_row(ui, row.min, w, &tabs, active, can_reopen, &t);
            return;
        }
        // More tabs than fit at their narrowest (a phone): the row scrolls sideways, and each tab
        // gets room for its name.
        let w = SCROLL_W;
        let strip = vec2(w * tabs.len() as f32, HEIGHT);
        // A newly active presentation's tab scrolls into view; otherwise the row stays where the
        // user scrolled it.
        let seen = ui.id().with("doc_tabs_active");
        let active_uid = active.and_then(|i| tabs.get(i)).map(|tab| tab.uid);
        let reveal = ui.data(|d| d.get_temp::<Option<u64>>(seen)) != Some(active_uid);
        ui.data_mut(|d| d.insert_temp(seen, active_uid));
        let mut sui = ui.new_child(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Min)));
        egui::ScrollArea::horizontal().id_salt("doc_tabs_scroll").show(&mut sui, |ui| {
            let (r, _) = ui.allocate_exact_size(strip, Sense::hover());
            action = tab_row(ui, r.min, w, &tabs, active, can_reopen, &t);
            if let Some(i) = active.filter(|_| reveal) {
                ui.scroll_to_rect(Rect::from_min_size(pos2(r.min.x + i as f32 * w, r.min.y), vec2(w, HEIGHT)), None);
            }
        });
    });
    match action {
        Some(Action::Activate(i)) => {
            let _ = app.run("window.next", json!({"index": i}));
        }
        Some(Action::Close(uid)) => app.close_document_by_uid(uid),
        Some(Action::CloseOthers(uid)) => app.close_others(uid),
        Some(Action::CloseAll) => app.close_all(),
        Some(Action::Reopen) => {
            let _ = app.run("file.reopenClosed", json!({}));
        }
        None => {}
    }
}

struct Tab {
    uid: u64,
    title: String,
    dirty: bool,
    /// "Saving…" or "Save failed".
    state: Option<&'static str>,
}

/// The tabs, `w` wide each, from `origin`; returns what a click or the context menu asked for.
fn tab_row(ui: &mut Ui, origin: egui::Pos2, w: f32, tabs: &[Tab], active: Option<usize>, can_reopen: bool, t: &Tokens) -> Option<Action> {
    let mut action = None;
    for (i, Tab { uid, title, dirty, state }) in tabs.iter().enumerate() {
        let is_active = active == Some(i);
        let r = Rect::from_min_size(pos2(origin.x + i as f32 * w, origin.y + 3.0), vec2(w - 2.0, HEIGHT - 3.0));
        let tab = ui.interact(r, ui.id().with(("doc_tab", uid)), Sense::click());
        tab.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, is_active, title));
        let close_r = Rect::from_center_size(pos2(r.max.x - 13.0, r.center().y), vec2(18.0, 18.0));
        let close = ui.interact(close_r, ui.id().with(("doc_tab_close", uid)), Sense::click());
        let close_label = crate::i18n::fmt(crate::tl!("Close {name}"), &[("name", title)]);
        close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &close_label));

        let p = ui.painter();
        let fill = if is_active {
            t.card
        } else if tab.hovered() {
            t.hover
        } else {
            Color32::TRANSPARENT
        };
        p.rect_filled(r, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, fill);
        if is_active {
            p.rect_filled(Rect::from_min_max(pos2(r.min.x, r.max.y - 2.0), r.max), CornerRadius::ZERO, t.accent);
        }
        let mut x = r.min.x + 10.0;
        if *dirty {
            p.circle_filled(pos2(x + 3.0, r.center().y), 3.0, t.accent);
            x += 12.0;
        }
        let ink = if is_active { t.text } else { t.text_dim };
        let mut job = egui::text::LayoutJob::simple_singleline(title.clone(), theme::font(12.0), ink);
        job.wrap = egui::text::TextWrapping::truncate_at_width((close_r.min.x - 4.0 - x).max(0.0));
        let galley = p.layout_job(job);
        p.galley(pos2(x, r.center().y - galley.size().y / 2.0), galley, ink);
        if close.hovered() {
            p.rect_filled(close_r, CornerRadius::same(4), t.pressed);
        }
        icons::paint(p, close_r.shrink(4.0), Icon::Close, t.text_dim, false);

        let tip = match state {
            Some(state) => format!("{title} — {state}"),
            None if *dirty => format!("{title} — {}", crate::tl!("Edited")),
            None => title.clone(),
        };
        let close = close.on_hover_text(crate::tl!("Close"));
        let tab = tab.on_hover_text(tip);
        if close.clicked() || tab.middle_clicked() {
            action = Some(Action::Close(*uid));
        } else if tab.clicked() && !is_active {
            action = Some(Action::Activate(i));
        }
        // Right-click, or a long press on a touch screen.
        tab.context_menu(|ui| {
            let mut item = |ui: &mut Ui, label: &str, enabled: bool, then: Action| {
                if ui.add_enabled(enabled, egui::Button::new(crate::tl!(label))).clicked() {
                    action = Some(then);
                    ui.close();
                }
            };
            item(ui, "Close", true, Action::Close(*uid));
            item(ui, "Close Others", tabs.len() > 1, Action::CloseOthers(*uid));
            item(ui, "Close All", true, Action::CloseAll);
            ui.separator();
            item(ui, "Reopen Closed Presentation", can_reopen, Action::Reopen);
        });
    }
    action
}
