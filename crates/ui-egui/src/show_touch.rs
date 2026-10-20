//! A persistent exit target for touch slide shows, independent of the fading controls.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};

pub(crate) fn exit_rect(ui: &Ui, touch: bool) -> Option<Rect> {
    touch.then(|| {
        let full = ui.max_rect();
        Rect::from_min_size(pos2(full.right() - 52.0, full.top() + 8.0), vec2(44.0, 44.0))
    })
}

pub(crate) fn exit_button(ui: &mut Ui, rect: Rect) -> bool {
    let response = ui.interact(rect, ui.id().with("show_touch_exit"), Sense::click());
    let label = crate::tl!("Exit slide show");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(6), Color32::from_black_alpha(if response.hovered() { 220 } else { 170 }));
    let mark = rect.shrink(15.0);
    let stroke = Stroke::new(2.0, Color32::WHITE);
    painter.line_segment([mark.left_top(), mark.right_bottom()], stroke);
    painter.line_segment([mark.right_top(), mark.left_bottom()], stroke);
    response.on_hover_text(label).clicked()
}
