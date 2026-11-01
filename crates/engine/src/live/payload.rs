//! A region's whole content as compact JSON, versioned (`"v":1`):
//!
//! - `shape:<slide>:<shape>`: `{"v":1,"at":<index among the slide's shapes>,"shape":<shape>,
//!   "media":[<media>…]}`, or `{"v":1,"del":1}` once the shape (or its slide) is gone;
//! - `slide:<slide>`: `{"v":1,"slide":<the slide without its shapes>,"order":[<shape id>…],
//!   "media":[…]}` (`{"v":1,"del":1}` once the slide is gone: `slides` removes it);
//! - `slides`: `{"v":1,"order":[<slide id>…],"sections":[…],"new":[<slide>…],"media":[…]}`, `new`
//!   holding the whole slides the other side may not have;
//! - `*`: `{"v":1,"deck":<base64 of the native .deckcraft file>}`.
//!
//! Shapes and slides are the model's own JSON (`deckcraft_model`, camelCase); media is
//! `{"id","name","contentType","link","data":<base64>}` for the media they use.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use deckcraft_model::{MediaId, MediaItem, Presentation, Section, Shape, ShapeId, Slide, SlideId};
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::{LiveError, Region};
use crate::cmd::{base64_decode, base64_encode};

/// The room takes at most 1 MiB per message; the shell's envelope needs the rest.
pub const MAX_PAYLOAD_BYTES: usize = 1_000_000;
pub const VERSION: u64 = 1;

pub(super) fn malformed(what: &str) -> LiveError {
    LiveError::Malformed(what.into())
}

fn to_value(v: &impl Serialize) -> Result<Value, LiveError> {
    serde_json::to_value(v).map_err(|e| LiveError::Malformed(e.to_string()))
}

fn media_of(doc: &Presentation, fragment: &impl Serialize) -> Result<Value, LiveError> {
    let ids = crate::clipboard_media::collect(fragment).map_err(|e| LiveError::Malformed(e.to_string()))?;
    let mut seen = HashSet::new();
    let items = ids
        .into_iter()
        .filter(|id| seen.insert(*id))
        .filter_map(|id| doc.media(id))
        .map(|m| json!({"id": m.id, "name": m.name, "contentType": m.content_type, "link": m.link, "data": base64_encode(&m.data)}))
        .collect();
    Ok(Value::Array(items))
}

/// Add the payload's media this side lacks; media in use is never replaced.
fn bring_media(doc: &mut Presentation, payload: &Map<String, Value>) -> Result<(), LiveError> {
    let Some(items) = payload.get("media") else { return Ok(()) };
    let items = items.as_array().ok_or_else(|| malformed("the media"))?;
    for item in items {
        let id = item.get("id").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok()).map(MediaId).ok_or_else(|| malformed("a media id"))?;
        if doc.media(id).is_some() {
            continue;
        }
        let text = |key: &str| item.get(key).and_then(Value::as_str).map(str::to_owned);
        let data = text("data").and_then(|d| base64_decode(&d)).ok_or_else(|| malformed("media data"))?;
        doc.media.push(MediaItem {
            id,
            name: text("name").unwrap_or_default(),
            content_type: text("contentType").unwrap_or_default(),
            data: Arc::new(data),
            link: text("link"),
        });
        doc.next_id = doc.next_id.max(id.0);
    }
    Ok(())
}

fn finish(map: Value) -> Result<String, LiveError> {
    let text = serde_json::to_string(&map).map_err(|e| LiveError::Malformed(e.to_string()))?;
    if text.len() > MAX_PAYLOAD_BYTES {
        return Err(LiveError::TooLarge);
    }
    Ok(text)
}

fn gone() -> Value {
    json!({"v": VERSION, "del": 1})
}

/// Region `region` of `doc`; `known` says which slides the receiver has (others go whole).
pub fn build(doc: &Presentation, region: Region, known: &dyn Fn(SlideId) -> bool) -> Result<String, LiveError> {
    let value = match region {
        Region::Shape(slide, shape) => {
            match doc.slide(slide).and_then(|s| s.shapes.iter().position(|sh| sh.id == shape).map(|at| (at, &s.shapes[at]))) {
                Some((at, shape)) => json!({"v": VERSION, "at": at, "shape": to_value(shape)?, "media": media_of(doc, shape)?}),
                None => gone(),
            }
        }
        Region::Slide(slide) => match doc.slide(slide) {
            Some(slide) => {
                let (props, order) = super::diff::slide_props(slide);
                json!({"v": VERSION, "slide": to_value(&props)?, "order": order, "media": media_of(doc, &props)?})
            }
            None => gone(),
        },
        Region::Slides => {
            let order: Vec<SlideId> = doc.slides.iter().map(|s| s.id).collect();
            let new: Vec<&Slide> = doc.slides.iter().filter(|s| !known(s.id)).map(|s| s.as_ref()).collect();
            json!({"v": VERSION, "order": order, "sections": to_value(&doc.sections)?, "new": to_value(&new)?, "media": media_of(doc, &new)?})
        }
        Region::Document => {
            let bytes = deckcraft_format::save(doc).map_err(|e| LiveError::Malformed(e.to_string()))?;
            // Base64 grows it by a third: check before encoding a file that can't be sent anyway.
            if bytes.len() / 3 * 4 > MAX_PAYLOAD_BYTES {
                return Err(LiveError::TooLarge);
            }
            json!({"v": VERSION, "deck": base64_encode(&bytes)})
        }
    };
    finish(value)
}

/// The payload's JSON object, of a version this build reads.
pub fn parse(payload: &str) -> Result<Map<String, Value>, LiveError> {
    let value: Value = serde_json::from_str(payload).map_err(|_| malformed("the payload"))?;
    let Value::Object(map) = value else { return Err(malformed("the payload")) };
    match map.get("v").and_then(Value::as_u64) {
        Some(VERSION) => Ok(map),
        Some(v) => Err(LiveError::Version(v)),
        None => Err(malformed("the version")),
    }
}

fn ids<T: serde::de::DeserializeOwned>(payload: &Map<String, Value>, key: &str) -> Result<Vec<T>, LiveError> {
    serde_json::from_value(payload.get(key).cloned().ok_or_else(|| malformed(key))?).map_err(|_| malformed(key))
}

fn slide_mut(doc: &mut Presentation, id: SlideId) -> Result<&mut Slide, LiveError> {
    doc.slides.iter_mut().find(|s| s.id == id).map(Arc::make_mut).ok_or_else(|| LiveError::Apply(format!("slide {id} isn't here")))
}

/// Put the shapes listed in `order` in that order, in the places they hold among the others.
fn restack(shapes: &mut [Shape], order: &[ShapeId]) {
    let rank: HashMap<ShapeId, usize> = order.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let places: Vec<usize> = shapes.iter().enumerate().filter(|(_, s)| rank.contains_key(&s.id)).map(|(i, _)| i).collect();
    let mut listed: Vec<Shape> = places.iter().map(|i| shapes[*i].clone()).collect();
    listed.sort_by_key(|s| rank.get(&s.id).copied());
    for (place, shape) in places.into_iter().zip(listed) {
        shapes[place] = shape;
    }
}

/// Make region `region` of `doc` what `payload` says, touching nothing else.
pub fn apply(doc: &mut Presentation, region: Region, payload: &Map<String, Value>) -> Result<(), LiveError> {
    let deleted = payload.contains_key("del");
    match region {
        Region::Shape(slide, id) => {
            if deleted {
                // A slide that is gone took its shapes along.
                if doc.slide(slide).is_some() {
                    slide_mut(doc, slide)?.shapes.retain(|s| s.id != id);
                }
                return Ok(());
            }
            let shape: Shape =
                serde_json::from_value(payload.get("shape").cloned().ok_or_else(|| malformed("the shape"))?).map_err(|_| malformed("the shape"))?;
            if shape.id != id {
                return Err(malformed("the shape id"));
            }
            let at = payload.get("at").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).unwrap_or(usize::MAX);
            slide_mut(doc, slide)?;
            bring_media(doc, payload)?;
            let shapes = &mut slide_mut(doc, slide)?.shapes;
            match shapes.iter().position(|s| s.id == id) {
                Some(i) => shapes[i] = shape,
                None => shapes.insert(at.min(shapes.len()), shape),
            }
        }
        Region::Slide(id) => {
            if deleted {
                return Ok(());
            }
            let props: Slide =
                serde_json::from_value(payload.get("slide").cloned().ok_or_else(|| malformed("the slide"))?).map_err(|_| malformed("the slide"))?;
            if props.id != id {
                return Err(malformed("the slide id"));
            }
            let order: Vec<ShapeId> = ids(payload, "order")?;
            slide_mut(doc, id)?;
            bring_media(doc, payload)?;
            let slide = slide_mut(doc, id)?;
            let mut shapes = std::mem::take(&mut slide.shapes);
            restack(&mut shapes, &order);
            *slide = Slide { shapes, ..props };
        }
        Region::Slides => {
            let order: Vec<SlideId> = ids(payload, "order")?;
            let sections: Vec<Section> = ids(payload, "sections")?;
            let new: Vec<Slide> = ids(payload, "new")?;
            bring_media(doc, payload)?;
            let mut have: HashMap<SlideId, Arc<Slide>> = doc.slides.iter().map(|s| (s.id, s.clone())).collect();
            for slide in new {
                have.entry(slide.id).or_insert_with(|| Arc::new(slide));
            }
            doc.slides = order.iter().filter_map(|id| have.remove(id)).collect();
            doc.sections = sections;
        }
        Region::Document => {
            let bytes = payload.get("deck").and_then(Value::as_str).and_then(base64_decode).ok_or_else(|| malformed("the presentation"))?;
            let loaded = deckcraft_format::load(&bytes).map_err(|e| LiveError::Apply(e.to_string()))?;
            let ids = doc.ids.clone();
            *doc = Presentation { ids, ..loaded };
        }
    }
    doc.fix_next_id();
    Ok(())
}
