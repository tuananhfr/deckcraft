//! Where an office Cloud comment points in a presentation (ERPCons M5, the `erpHost` comment
//! calls). The anchor is JSON the shell stores on the server with the comment; nothing of it is
//! ever written into the file, and the file's own comments are untouched.
//!
//! ```json
//! {"v":1,
//!  "region":"shape:<slideId>:<shapeId>" | "slide:<slideId>",
//!  "pos":{"box":[x,y,w,h], "text":{"from":[p,c],"to":[p,c]}} | null,
//!  "soft":{"index":<0-based slide>,"title":"<slide title>","name":"<shape name>"},
//!  "label":"Trang chiếu 2 · Tiêu đề"}
//! ```
//!
//! `region` is the co-editing region of the selected shape (its top-level shape, as locked) or, with
//! no shape selected, of the slide. `pos` is where in the region: the shape's box in slide points
//! and the selected text, if any (paragraph, character). `soft` finds it again when the slide's id
//! is gone (another version of the file): the slide by its title, nearest its old position, then
//! the shape by its id. `label` is the shell's hint for people, made by the app and ignored when
//! an anchor is read.

use deckcraft_model::{Presentation, Shape, ShapeId, Slide, SlideId, walk};
use serde_json::{Value, json};

use crate::live::Region;
use crate::{Selection, Session, Target};

pub const ANCHOR_VERSION: u64 = 1;
/// An anchor is a small hint; anything larger is refused unread.
pub const MAX_ANCHOR_BYTES: usize = 16 * 1024;
/// The title and name kept to find an anchor again.
pub const MAX_TEXT_CHARS: usize = 200;
const MAX_INDEX: u64 = 1 << 24;

#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub slide: SlideId,
    /// A top-level shape of the slide; `None` for the slide itself.
    pub shape: Option<ShapeId>,
    /// The shape's box (x, y, w, h) in slide points, unrotated.
    pub bounds: Option<[f64; 4]>,
    /// Selected text in the shape: from and to (paragraph, character).
    pub text: Option<((usize, usize), (usize, usize))>,
    /// 0-based position of the slide when made or last resolved.
    pub index: usize,
    pub title: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AnchorError {
    #[error("the anchor comes from a newer version of the editor")]
    Version,
    #[error("the anchor can't be read ({0})")]
    Malformed(&'static str),
}

/// Whether an anchor was found again ([`Session::resolve_anchors`]).
#[derive(Clone, Debug, PartialEq)]
pub enum Resolution {
    Open(Anchor),
    Orphaned,
}

/// Whitespace collapsed to single spaces, trimmed, cut to [`MAX_TEXT_CHARS`] characters.
pub fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_TEXT_CHARS).collect()
}

fn round(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn pair(v: &Value) -> Option<(usize, usize)> {
    let a = v.as_array().filter(|a| a.len() == 2)?;
    let n = |v: &Value| v.as_u64().filter(|n| *n < MAX_INDEX).and_then(|n| usize::try_from(n).ok());
    Some((n(a.first()?)?, n(a.get(1)?)?))
}

impl Anchor {
    pub fn region(&self) -> Region {
        match self.shape {
            Some(shape) => Region::Shape(self.slide, shape),
            None => Region::Slide(self.slide),
        }
    }

    /// The anchor from the shell's JSON (any `label` is ignored).
    pub fn parse(json: &str) -> Result<Self, AnchorError> {
        if json.len() > MAX_ANCHOR_BYTES {
            return Err(AnchorError::Malformed("too large"));
        }
        let value: Value = serde_json::from_str(json).map_err(|_| AnchorError::Malformed("not JSON"))?;
        Self::from_value(&value)
    }

    pub fn from_value(value: &Value) -> Result<Self, AnchorError> {
        let obj = value.as_object().ok_or(AnchorError::Malformed("not an object"))?;
        match obj.get("v").and_then(Value::as_u64) {
            Some(ANCHOR_VERSION) => {}
            Some(_) => return Err(AnchorError::Version),
            None => return Err(AnchorError::Malformed("no version")),
        }
        let (slide, shape) = match obj.get("region").and_then(Value::as_str).and_then(Region::parse) {
            Some(Region::Shape(slide, shape)) => (slide, Some(shape)),
            Some(Region::Slide(slide)) => (slide, None),
            _ => return Err(AnchorError::Malformed("no shape or slide region")),
        };
        let pos = obj.get("pos").and_then(Value::as_object);
        let bounds = pos.and_then(|p| p.get("box")).and_then(Value::as_array).filter(|b| b.len() == 4).and_then(|b| {
            let mut out = [0.0; 4];
            for (slot, v) in out.iter_mut().zip(b) {
                *slot = v.as_f64().filter(|f| f.is_finite() && f.abs() < 1e9)?;
            }
            Some(out)
        });
        let text = pos.and_then(|p| p.get("text")).and_then(Value::as_object).and_then(|t| Some((pair(t.get("from")?)?, pair(t.get("to")?)?)));
        let soft = obj.get("soft").and_then(Value::as_object).ok_or(AnchorError::Malformed("no soft anchor"))?;
        let index = soft.get("index").and_then(Value::as_u64).filter(|i| *i < MAX_INDEX).ok_or(AnchorError::Malformed("no slide index"))?;
        let text_of = |key: &str| soft.get(key).and_then(Value::as_str).map(clean_text).unwrap_or_default();
        Ok(Self {
            slide,
            shape,
            bounds: bounds.filter(|_| shape.is_some()),
            text: text.filter(|_| shape.is_some()),
            index: usize::try_from(index).map_err(|_| AnchorError::Malformed("slide index"))?,
            title: text_of("title"),
            name: text_of("name"),
        })
    }

    /// The shell's JSON, with `label` for people.
    pub fn to_json(&self, label: &str) -> String {
        self.to_value(label).to_string()
    }

    pub fn to_value(&self, label: &str) -> Value {
        let pos = self.shape.map_or(Value::Null, |_| {
            let mut pos = serde_json::Map::new();
            pos.insert("box".into(), self.bounds.map_or(Value::Null, |b| json!(b.map(round))));
            if let Some((from, to)) = self.text {
                pos.insert("text".into(), json!({ "from": [from.0, from.1], "to": [to.0, to.1] }));
            }
            Value::Object(pos)
        });
        json!({
            "v": ANCHOR_VERSION,
            "region": self.region().to_string(),
            "pos": pos,
            "soft": { "index": self.index, "title": self.title, "name": self.name },
            "label": label,
        })
    }
}

/// The top-level shape of `slide` that is or holds shape `id`.
pub fn top_level(slide: &Slide, id: ShapeId) -> Option<&Shape> {
    slide.shapes.iter().find(|top| {
        let mut found = top.id == id;
        walk(std::slice::from_ref(*top), &mut |s: &Shape, _| found |= s.id == id);
        found
    })
}

/// Shape `shape`'s box on slide `index` as drawn (a placeholder's from its layout).
fn bounds(doc: &Presentation, index: usize, shape: &Shape) -> [f64; 4] {
    let sel = Selection { slide: index, target: Target::Slides, ..Selection::default() };
    let x = crate::cmd::xfrm_of(doc, &sel, shape);
    [x.x, x.y, x.w, x.h].map(round)
}

/// The anchor of shape `shape` (or the slide) on slide `index` as it is now.
fn anchor_at(doc: &Presentation, index: usize, shape: Option<ShapeId>, text: Option<((usize, usize), (usize, usize))>) -> Option<Anchor> {
    let slide = doc.slides.get(index)?;
    let top = match shape {
        Some(id) => Some(top_level(slide, id)?),
        None => None,
    };
    Some(Anchor {
        slide: slide.id,
        shape: top.map(|s| s.id),
        bounds: top.map(|s| bounds(doc, index, s)),
        text: text.filter(|_| top.is_some()),
        index,
        title: clean_text(&slide.title()),
        name: top.map(|s| clean_text(&s.name)).unwrap_or_default(),
    })
}

/// Find `anchor` again in `doc`: its slide by id (the shape by id on it, or orphaned when the
/// shape is gone), else a slide with its title nearest its old position holding its shape, else
/// orphaned.
pub fn resolve(doc: &Presentation, anchor: &Anchor) -> Resolution {
    let found = |index: usize| anchor_at(doc, index, anchor.shape, anchor.text);
    if let Some(index) = doc.slides.iter().position(|s| s.id == anchor.slide) {
        return found(index).map_or(Resolution::Orphaned, Resolution::Open);
    }
    if anchor.title.is_empty() {
        return Resolution::Orphaned;
    }
    let mut titled: Vec<usize> = doc.slides.iter().enumerate().filter(|(_, s)| clean_text(&s.title()) == anchor.title).map(|(i, _)| i).collect();
    titled.sort_by_key(|i| i.abs_diff(anchor.index));
    titled.into_iter().find_map(found).map_or(Resolution::Orphaned, Resolution::Open)
}

/// Where `anchor` is now, without the title fallback: the slide's position and its top-level
/// shape, if any. `None` when the slide or the shape is gone.
pub fn place(doc: &Presentation, anchor: &Anchor) -> Option<(usize, Option<ShapeId>)> {
    let index = doc.slides.iter().position(|s| s.id == anchor.slide)?;
    match anchor.shape {
        Some(id) => Some((index, Some(top_level(doc.slides.get(index)?, id)?.id))),
        None => Some((index, None)),
    }
}

impl Session {
    /// The anchor of the selection in presentation `uid`: the first selected shape (as its
    /// top-level shape, with the text being selected in it), else the current slide; `None` in
    /// Slide Master view or without slides.
    pub fn anchor_of_selection(&self, uid: u64) -> Option<Anchor> {
        let st = self.documents().iter().find(|d| d.uid == uid)?;
        if st.selection.target != Target::Slides {
            return None;
        }
        let index = st.selection.slide;
        let text = st.selection.text.as_ref().filter(|t| !t.notes && t.cell.is_none());
        let shape = text.map(|t| t.shape).or_else(|| st.selection.shapes.first().copied());
        let range = text.filter(|t| Some(t.shape) == shape).map(|t| t.ordered());
        anchor_at(&st.doc, index, shape, range).or_else(|| anchor_at(&st.doc, index, None, None))
    }

    /// [`resolve`] for each anchor in presentation `uid`; every one orphaned without it.
    pub fn resolve_anchors(&self, uid: u64, anchors: &[Anchor]) -> Vec<Resolution> {
        match self.documents().iter().find(|d| d.uid == uid) {
            Some(st) => anchors.iter().map(|a| resolve(&st.doc, a)).collect(),
            None => vec![Resolution::Orphaned; anchors.len()],
        }
    }
}
