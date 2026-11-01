//! Region ids: what one person locks and sends while editing together.

use std::fmt;

use deckcraft_model::{ShapeId, SlideId};

/// A part of the presentation that changes as a whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Region {
    /// `shape:<slideId>:<shapeId>`: a top-level shape of a slide (a group with all it holds).
    Shape(SlideId, ShapeId),
    /// `slide:<slideId>`: the slide's own properties and the stacking order of its shapes.
    Slide(SlideId),
    /// `slides`: which slides there are, their order and the sections.
    Slides,
    /// `*`: everything else (masters, layouts, theme, slide size, properties).
    Document,
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Region::Shape(slide, shape) => write!(f, "shape:{slide}:{shape}"),
            Region::Slide(slide) => write!(f, "slide:{slide}"),
            Region::Slides => f.write_str("slides"),
            Region::Document => f.write_str("*"),
        }
    }
}

impl Region {
    pub fn parse(s: &str) -> Option<Region> {
        let id = |s: &str| s.parse::<u32>().ok().filter(|_| !s.starts_with('+') && (s == "0" || !s.starts_with('0')));
        match s {
            "slides" => return Some(Region::Slides),
            "*" => return Some(Region::Document),
            _ => {}
        }
        if let Some(rest) = s.strip_prefix("shape:") {
            let (slide, shape) = rest.split_once(':')?;
            return Some(Region::Shape(SlideId(id(slide)?), ShapeId(id(shape)?)));
        }
        s.strip_prefix("slide:").and_then(id).map(|slide| Region::Slide(SlideId(slide)))
    }

    /// The slide it belongs to.
    pub fn slide(&self) -> Option<SlideId> {
        match self {
            Region::Shape(slide, _) | Region::Slide(slide) => Some(*slide),
            Region::Slides | Region::Document => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_ids_read_back() {
        for r in [Region::Shape(SlideId(256), ShapeId(7)), Region::Slide(SlideId(4_000_000)), Region::Slides, Region::Document] {
            assert_eq!(Region::parse(&r.to_string()), Some(r));
        }
        assert_eq!(Region::Shape(SlideId(256), ShapeId(7)).to_string(), "shape:256:7");
        for bad in ["", "shape:1", "shape:1:x", "slide:", "slide:01", "slide:+1", "slide:-1", "pages", "shape:1:2:3"] {
            assert_eq!(Region::parse(bad), None, "{bad:?}");
        }
    }
}
