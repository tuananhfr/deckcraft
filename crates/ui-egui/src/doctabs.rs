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

/// The tab row shows only with more than one presentation open.
pub fn visible(app: &SlideApp) -> bool {
    app.session.documents().len() > 1
}

enum Action {
    Activate(usize),
    Close(u64),
}

pub fn show(app: &mut SlideApp, ui: &mut Ui) {
    if !visible(app) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let tabs: Vec<(u64, String, bool)> = app.session.documents().iter().map(|d| (d.uid, d.title(), d.is_dirty())).collect();
    let active = app.session.active_index();
    let mut action = None;
    egui::Panel::top("doc_tabs").exact_size(HEIGHT).frame(egui::Frame::NONE.fill(t.chrome)).show(ui, |ui| {
        let rect = ui.max_rect();
        let x0 = rect.min.x + 8.0;
        let w = ((rect.width() - 16.0) / tabs.len() as f32).clamp(MIN_W, MAX_W);
        ui.painter().line_segment([rect.left_bottom() - vec2(0.0, 0.5), rect.right_bottom() - vec2(0.0, 0.5)], Stroke::new(1.0, t.separator));
        for (i, (uid, title, dirty)) in tabs.iter().enumerate() {
            let is_active = active == Some(i);
            let r = Rect::from_min_size(pos2(x0 + i as f32 * w, rect.min.y + 3.0), vec2(w - 2.0, HEIGHT - 3.0));
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

            let tip = if *dirty { format!("{title} — {}", crate::tl!("Edited")) } else { title.clone() };
            let close = close.on_hover_text(crate::tl!("Close"));
            let tab = tab.on_hover_text(tip);
            if close.clicked() || tab.middle_clicked() {
                action = Some(Action::Close(*uid));
            } else if tab.clicked() && !is_active {
                action = Some(Action::Activate(i));
            }
        }
    });
    match action {
        Some(Action::Activate(i)) => {
            let _ = app.run("window.next", json!({"index": i}));
        }
        Some(Action::Close(uid)) => app.close_document_by_uid(uid),
        None => {}
    }
}
