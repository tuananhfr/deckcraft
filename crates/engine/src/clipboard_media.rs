//! Media references in serialized model fragments, including nested fills and picture bullets.

use std::collections::HashMap;

use deckcraft_model::{MediaId, MediaItem, Presentation};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{EngineError, Result};

fn references(value: &mut Value, depth: usize, visit: &mut impl FnMut(&mut Value)) {
    if depth > 64 {
        return;
    }
    match value {
        Value::Object(items) => {
            for (key, value) in items {
                if matches!(key.as_str(), "media" | "poster" | "preview") && value.is_u64() {
                    visit(value);
                } else {
                    references(value, depth + 1, visit);
                }
            }
        }
        Value::Array(items) => {
            for value in items {
                references(value, depth + 1, visit);
            }
        }
        _ => {}
    }
}

pub(crate) fn collect(fragment: &impl Serialize) -> Result<Vec<MediaId>> {
    let mut value = serde_json::to_value(fragment).map_err(|e| EngineError::Other(e.to_string()))?;
    let mut ids = vec![];
    references(&mut value, 0, &mut |value| {
        if let Some(id) = value.as_u64().and_then(|id| u32::try_from(id).ok()) {
            ids.push(MediaId(id));
        }
    });
    Ok(ids)
}

pub(crate) fn bring(doc: &mut Presentation, media: &[MediaItem]) -> HashMap<MediaId, MediaId> {
    let mut mapping = HashMap::new();
    for item in media {
        let id = match doc.media(item.id) {
            Some(existing) if existing.data == item.data && existing.content_type == item.content_type && existing.link == item.link => item.id,
            _ => {
                let mut incoming = item.clone();
                incoming.id = MediaId(doc.alloc_id());
                let id = incoming.id;
                doc.media.push(incoming);
                id
            }
        };
        mapping.insert(item.id, id);
    }
    mapping
}

pub(crate) fn remap<T: Clone + Serialize + DeserializeOwned>(fragment: &T, mapping: &HashMap<MediaId, MediaId>) -> Result<T> {
    if mapping.iter().all(|(old, new)| old == new) {
        return Ok(fragment.clone());
    }
    let mut value = serde_json::to_value(fragment).map_err(|e| EngineError::Other(e.to_string()))?;
    references(&mut value, 0, &mut |value| {
        if let Some(id) = value.as_u64().and_then(|id| u32::try_from(id).ok())
            && let Some(mapped) = mapping.get(&MediaId(id))
        {
            *value = serde_json::json!(mapped.0);
        }
    });
    serde_json::from_value(value).map_err(|e| EngineError::Other(e.to_string()))
}
