//! Others on the slide: a shape someone else holds gets an outline in their colour with their name;
//! someone's selected shape a thinner one; someone holding the slide, or on it, a name tag at its
//! corner.

use deckcraft_engine::Target;
use deckcraft_engine::live::Region;
use deckcraft_geom::Point;
use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, pos2, vec2};

use super::Live;
use crate::SlideApp;
use crate::canvas::Xf;

/// `#rrggbb`, or a neutral grey.
fn colour(hex: Option<&String>) -> Color32 {
    let parsed = hex.and_then(|h| h.strip_prefix('#')).filter(|h| h.len() == 6).and_then(|h| u32::from_str_radix(h, 16).ok());
    match parsed {
        Some(v) => Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8),
        None => Color32::from_gray(120),
    }
}

/// Someone's presence on the slide shown, as drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    /// A shape's corners on screen; `None` for a name tag at the slide's corner.
    pub corners: Option<[Pos2; 4]>,
    pub colour: Color32,
    pub name: String,
    /// Someone holds it (a lock), rather than has it selected.
    pub lock: bool,
}

impl Live {
    /// Who holds or selects which region, with their name and colour: locks first.
    fn presence(&self) -> Vec<(String, String, Color32, bool)> {
        let mut locks: Vec<_> = self.others.values().collect();
        locks.sort_by(|a, b| a.region.cmp(&b.region));
        let locks = locks.into_iter().map(|l| {
            (
                l.region.clone(),
                self.name_of(&l.holder, l.name.as_ref()).unwrap_or_default(),
                colour(self.colour_of(&l.holder, l.colour.as_ref()).as_ref()),
                true,
            )
        });
        let cursors = self.cursors.iter().map(|c| {
            (
                c.region.clone(),
                self.name_of(&c.author, c.name.as_ref()).unwrap_or_default(),
                colour(self.colour_of(&c.author, c.colour.as_ref()).as_ref()),
                false,
            )
        });
        locks.chain(cursors).collect()
    }
}

/// The marks on the slide shown in the live presentation.
pub(crate) fn marks(app: &SlideApp, xf: Xf) -> Vec<Mark> {
    let Some(live) = app.live.as_ref() else { return Vec::new() };
    let Some(st) = app.session.active().filter(|d| Some(d.uid) == live.doc && d.selection.target == Target::Slides) else { return Vec::new() };
    let Some(slide) = st.current_slide() else { return Vec::new() };
    let mut out: Vec<Mark> = Vec::new();
    for (region, name, colour, lock) in live.presence() {
        let corners = match Region::parse(&region) {
            Some(Region::Shape(s, id)) if s == slide.id => {
                let Some(shape) = slide.shapes.iter().find(|sh| sh.id == id) else { continue };
                let x = deckcraft_engine::cmd::xfrm_of(&st.doc, &st.selection, shape);
                let a = x.affine();
                Some([Point::new(0.0, 0.0), Point::new(x.w, 0.0), Point::new(x.w, x.h), Point::new(0.0, x.h)].map(|q| xf.to_screen(a * q)))
            }
            Some(Region::Slide(s)) if s == slide.id => None,
            Some(Region::Document) => None,
            _ => continue,
        };
        // A lock already outlines it.
        if !lock && corners.is_some() && out.iter().any(|m| m.lock && m.corners == corners) {
            continue;
        }
        out.push(Mark { corners, colour, name, lock });
    }
    out
}

/// Draw the marks of the slide shown.
pub(crate) fn paint(app: &SlideApp, painter: &egui::Painter, xf: Xf) {
    let font = FontId::proportional(11.0);
    let mut tags = 0.0;
    for mark in marks(app, xf) {
        let anchor = match mark.corners {
            Some(c) => {
                let width = if mark.lock { 2.0 } else { 1.0 };
                for i in 0..4 {
                    painter.line_segment([c[i], c[(i + 1) % 4]], Stroke::new(width, mark.colour));
                }
                c.iter().fold(c[0], |m, p| if p.y < m.y || (p.y == m.y && p.x < m.x) { *p } else { m })
            }
            None => {
                let at = xf.origin + vec2(4.0, 4.0 + tags);
                tags += 18.0;
                at
            }
        };
        if mark.name.is_empty() {
            continue;
        }
        let galley = painter.layout_no_wrap(mark.name.clone(), font.clone(), Color32::WHITE);
        let size = galley.size() + vec2(8.0, 4.0);
        let top = if mark.corners.is_some() { anchor.y - size.y } else { anchor.y };
        let tag = Rect::from_min_size(pos2(anchor.x, top), size);
        painter.rect_filled(tag, CornerRadius::same(3), mark.colour);
        painter.text(tag.center(), Align2::CENTER_CENTER, &mark.name, font.clone(), Color32::WHITE);
    }
}
