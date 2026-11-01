//! Comment markers on the slide shown: a small bubble with the thread's count at the top right of
//! its shape (or of the slide), the shape outlined while the pointer is on the bubble, a click
//! reported to the shell. Drawn over the slide like the co-editing marks, never part of it.

use deckcraft_engine::Target;
use deckcraft_engine::anchors;
use deckcraft_geom::Point;
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::SlideApp;
use crate::canvas::Xf;

/// One open thread's marker on the slide shown.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub thread: String,
    pub count: u32,
    /// The anchor's corners on screen (a shape's, or the slide's).
    pub corners: [Pos2; 4],
    pub bubble: Rect,
}

const BUBBLE: egui::Vec2 = vec2(22.0, 16.0);
/// Placing a bubble checks every earlier one, each frame: threads piled on one slide beyond this
/// get no bubble (the shell's panel lists them all).
pub(crate) const MAX_MARKERS: usize = 64;
const FILL: Color32 = Color32::from_rgb(0xF5, 0xB4, 0x00);
const INK: Color32 = Color32::from_rgb(0x2B, 0x22, 0x00);

/// The markers of the open threads anchored on the slide shown, bubbles side by side where they
/// would overlap.
pub(crate) fn markers(app: &SlideApp, xf: Xf) -> Vec<Marker> {
    let Some(comments) = app.comments.as_ref() else { return Vec::new() };
    let Some(st) = app.session.active().filter(|d| Some(d.uid) == comments.doc && d.selection.target == Target::Slides) else { return Vec::new() };
    let Some(slide) = st.current_slide() else { return Vec::new() };
    let size = (st.doc.slide_size.width, st.doc.slide_size.height);
    let slide_rect = Rect::from_two_pos(xf.to_screen(Point::new(0.0, 0.0)), xf.to_screen(Point::new(size.0, size.1)));
    let mut out: Vec<Marker> = Vec::new();
    for (thread, anchor) in comments.threads.iter().filter(|(t, _)| t.status == "open") {
        if out.len() >= MAX_MARKERS {
            break;
        }
        let Some(anchor) = anchor else { continue };
        let Some((index, shape)) = anchors::place(&st.doc, anchor) else { continue };
        if index != st.selection.slide {
            continue;
        }
        let corners = match shape.and_then(|id| slide.shapes.iter().find(|s| s.id == id)) {
            Some(shape) => {
                let x = deckcraft_engine::cmd::xfrm_of(&st.doc, &st.selection, shape);
                let a = x.affine();
                [Point::new(0.0, 0.0), Point::new(x.w, 0.0), Point::new(x.w, x.h), Point::new(0.0, x.h)].map(|q| xf.to_screen(a * q))
            }
            None => [slide_rect.left_top(), slide_rect.right_top(), slide_rect.right_bottom(), slide_rect.left_bottom()],
        };
        let bounds = Rect::from_points(&corners);
        let x = (bounds.right() + 4.0).min(slide_rect.right() - BUBBLE.x).max(slide_rect.left());
        let y = (bounds.top() - BUBBLE.y / 2.0).clamp(slide_rect.top(), (slide_rect.bottom() - BUBBLE.y).max(slide_rect.top()));
        let mut bubble = Rect::from_min_size(pos2(x, y), BUBBLE);
        while out.iter().any(|m| m.bubble.intersects(bubble)) {
            bubble = bubble.translate(vec2(0.0, BUBBLE.y + 2.0));
        }
        out.push(Marker { thread: thread.id.clone(), count: thread.count, corners, bubble });
    }
    out
}

/// Draw the markers of the slide shown; a click on one is reported to the shell next step.
pub(crate) fn paint(app: &mut SlideApp, ui: &egui::Ui, painter: &egui::Painter, xf: Xf) {
    let font = FontId::proportional(10.5);
    let mut clicked = None;
    for marker in markers(app, xf) {
        let response = ui.interact(marker.bubble, ui.id().with(("comment-marker", &marker.thread)), Sense::click());
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            let c = marker.corners;
            painter.add(egui::Shape::convex_polygon(c.to_vec(), FILL.gamma_multiply(0.15), Stroke::new(1.5, FILL)));
        }
        painter.rect_filled(marker.bubble, CornerRadius::same(4), FILL);
        painter.rect_stroke(marker.bubble, CornerRadius::same(4), Stroke::new(1.0, INK.gamma_multiply(0.5)), StrokeKind::Inside);
        // The bubble's tail, towards the anchor.
        let b = marker.bubble.left_bottom();
        painter.add(egui::Shape::convex_polygon(vec![b + vec2(4.0, 0.0), b + vec2(10.0, 0.0), b + vec2(3.0, 5.0)], FILL, Stroke::NONE));
        let count = if marker.count > 99 { "99+".to_owned() } else { marker.count.max(1).to_string() };
        painter.text(marker.bubble.center(), Align2::CENTER_CENTER, count, font.clone(), INK);
        if response.clicked() {
            clicked = Some(marker.thread);
        }
    }
    if let (Some(thread), Some(c)) = (clicked, app.comments.as_mut()) {
        c.clicked = Some(thread);
    }
}
