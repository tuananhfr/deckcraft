//! Which regions differ between two states of a presentation.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use deckcraft_model::{Presentation, Shape, ShapeId, Slide};

use super::Region;

fn same<T: PartialEq>(a: &Arc<T>, b: &Arc<T>) -> bool {
    Arc::ptr_eq(a, b) || a == b
}

fn same_masters(a: &[Arc<deckcraft_model::Master>], b: &[Arc<deckcraft_model::Master>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
}

fn same_optional<T: PartialEq>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// Everything outside the slides, their order and sections, and the media list.
fn rest_differs(a: &Presentation, b: &Presentation) -> bool {
    // Media is added with what uses it and never taken away by a remote change, so only an item
    // whose content changed under the same id counts.
    let changed_media =
        b.media.iter().any(|m| a.media(m.id).is_some_and(|old| old.data != m.data || old.content_type != m.content_type || old.link != m.link));
    changed_media
        || !same_masters(&a.masters, &b.masters)
        || !same_optional(&a.notes_master, &b.notes_master)
        || !same_optional(&a.handout_master, &b.handout_master)
        || a.slide_size != b.slide_size
        || a.notes_size != b.notes_size
        || a.custom_shows != b.custom_shows
        || a.show != b.show
        || a.header_footer != b.header_footer
        || a.props != b.props
        || a.default_text_style != b.default_text_style
        || a.first_slide_number != b.first_slide_number
        || a.embedded_fonts != b.embedded_fonts
        || a.table_styles != b.table_styles
        || a.raw_ext != b.raw_ext
}

/// The slide without its shapes, and its shapes' ids in stacking order.
pub(super) fn slide_props(slide: &Slide) -> (Slide, Vec<ShapeId>) {
    let order = slide.shapes.iter().map(|s| s.id).collect();
    (Slide { shapes: Vec::new(), ..slide.clone() }, order)
}

/// `a`'s and `b`'s order of the shapes both have.
fn same_relative_order(a: &[Shape], b: &[Shape]) -> bool {
    let in_b: std::collections::HashSet<ShapeId> = b.iter().map(|s| s.id).collect();
    let in_a: std::collections::HashSet<ShapeId> = a.iter().map(|s| s.id).collect();
    a.iter().map(|s| s.id).filter(|id| in_b.contains(id)).eq(b.iter().map(|s| s.id).filter(|id| in_a.contains(id)))
}

fn slide_regions(a: &Slide, b: &Slide, out: &mut BTreeSet<Region>) {
    let props_differ = Slide { shapes: Vec::new(), ..a.clone() } != Slide { shapes: Vec::new(), ..b.clone() };
    if props_differ || !same_relative_order(&a.shapes, &b.shapes) {
        out.insert(Region::Slide(b.id));
    }
    let before: HashMap<ShapeId, &Shape> = a.shapes.iter().map(|s| (s.id, s)).collect();
    let after: HashMap<ShapeId, &Shape> = b.shapes.iter().map(|s| (s.id, s)).collect();
    for (id, shape) in &after {
        if before.get(id).is_none_or(|old| old != shape) {
            out.insert(Region::Shape(b.id, *id));
        }
    }
    for id in before.keys().filter(|id| !after.contains_key(id)) {
        out.insert(Region::Shape(b.id, *id));
    }
}

/// The regions that differ from `a` to `b`. A slide only one side has travels with `slides`.
pub fn changed(a: &Presentation, b: &Presentation) -> Vec<Region> {
    let mut out = BTreeSet::new();
    if rest_differs(a, b) {
        out.insert(Region::Document);
    }
    let order_a: Vec<_> = a.slides.iter().map(|s| s.id).collect();
    let order_b: Vec<_> = b.slides.iter().map(|s| s.id).collect();
    if order_a != order_b || a.sections != b.sections {
        out.insert(Region::Slides);
    }
    let before: HashMap<_, &Arc<Slide>> = a.slides.iter().map(|s| (s.id, s)).collect();
    for slide in &b.slides {
        if let Some(old) = before.get(&slide.id)
            && !Arc::ptr_eq(old, slide)
        {
            slide_regions(old, slide, &mut out);
        }
    }
    out.into_iter().collect()
}
