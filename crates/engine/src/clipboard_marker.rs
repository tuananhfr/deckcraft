//! Versioned clipboard transport with bounded serialization and untrusted-input validation.

use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::Arc;

use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Clipboard;
use crate::cmd::{base64_decode, base64_encode};

pub(crate) const MAX_MARKER: usize = 4 * 1024 * 1024;
// Base64 expansion must fit inside the marker budget too.
const MAX_JSON: usize = MAX_MARKER / 4 * 3;
const MAX_HTML: usize = 8 * 1024 * 1024;
const MAX_CELLS: usize = 20_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fragment {
    v: u8,
    editor: String,
    clipboard: Clipboard,
    // Native manifests omit media bytes; the clipboard has no accompanying archive.
    media_data: Vec<String>,
}

#[derive(Serialize)]
struct FragmentRef<'a> {
    v: u8,
    editor: &'a str,
    clipboard: &'a Clipboard,
    media_data: Vec<String>,
}

struct BoundedJson(Vec<u8>);

impl Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_JSON.saturating_sub(self.0.len()) || self.0.try_reserve(bytes.len()).is_err() {
            return Err(io::Error::other("clipboard limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn encode(clip: &Clipboard) -> Option<String> {
    let total = clip.media.iter().try_fold(0usize, |n, m| n.checked_add(m.data.len()))?;
    if total > MAX_JSON || clip.plain.len() > MAX_JSON {
        return None;
    }
    let fragment =
        FragmentRef { v: 1, editor: "deckcraft", clipboard: clip, media_data: clip.media.iter().map(|m| base64_encode(&m.data)).collect() };
    let mut out = BoundedJson(vec![]);
    serde_json::to_writer(&mut out, &fragment).ok()?;
    let encoded = base64_encode(&out.0);
    Some(format!("<meta name=\"erpcons-clip\" content=\"{encoded}\">"))
}

fn bounded_json(value: &Value, depth: usize, nodes: &mut usize) -> bool {
    if depth > 64 || *nodes >= 100_000 {
        return false;
    }
    *nodes += 1;
    match value {
        Value::Array(items) => items.len() <= MAX_CELLS && items.iter().all(|v| bounded_json(v, depth + 1, nodes)),
        Value::Object(items) => items.values().all(|v| bounded_json(v, depth + 1, nodes)),
        Value::Number(n) => n.as_f64().is_some_and(|v| v.is_finite() && v.abs() <= u32::MAX as f64),
        _ => true,
    }
}

fn valid_shapes(shapes: &[deckcraft_model::Shape], cells: &mut usize, ids: &mut HashSet<deckcraft_model::ShapeId>) -> bool {
    for shape in shapes {
        if !ids.insert(shape.id) {
            return false;
        }
        if let deckcraft_model::ShapeKind::Table(t) = &shape.kind {
            let Some(count) = t.n_rows().checked_mul(t.n_cols()) else { return false };
            *cells = cells.saturating_add(count);
            if *cells > MAX_CELLS || t.n_rows() == 0 || t.n_cols() == 0 {
                return false;
            }
            for (r, row) in t.rows.iter().enumerate() {
                if row.cells.len() != t.n_cols() {
                    return false;
                }
                for (c, cell) in row.cells.iter().enumerate() {
                    if cell.grid_span.max(1) as usize > t.n_cols() - c || cell.row_span.max(1) as usize > t.n_rows() - r {
                        return false;
                    }
                }
            }
        }
        if !valid_shapes(shape.children(), cells, ids) {
            return false;
        }
    }
    true
}

fn decode(encoded: &str) -> Option<Clipboard> {
    if encoded.is_empty() || encoded.len() > MAX_MARKER {
        return None;
    }
    let bytes = base64_decode(encoded)?;
    if bytes.len() > MAX_JSON {
        return None;
    }
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    if !bounded_json(&value, 0, &mut 0) {
        return None;
    }
    let fragment: Fragment = serde_json::from_value(value).ok()?;
    if fragment.v != 1 || fragment.editor != "deckcraft" || fragment.media_data.len() != fragment.clipboard.media.len() {
        return None;
    }
    let mut clip = fragment.clipboard;
    let mut cells = 0;
    if !valid_shapes(&clip.shapes, &mut cells, &mut HashSet::new()) {
        return None;
    }
    let mut slides = HashSet::new();
    for slide in &clip.slides {
        if !slides.insert(slide.id) || !valid_shapes(&slide.shapes, &mut cells, &mut HashSet::new()) {
            return None;
        }
    }
    let mut media_ids = HashSet::new();
    for (item, data) in clip.media.iter_mut().zip(fragment.media_data) {
        if !media_ids.insert(item.id) {
            return None;
        }
        item.data = Arc::new(base64_decode(&data)?);
    }
    let mut referenced = crate::clipboard_media::collect(&clip.shapes).ok()?;
    referenced.extend(crate::clipboard_media::collect(&clip.slides).ok()?);
    if referenced.iter().any(|id| !media_ids.contains(id)) {
        return None;
    }
    clip.pastes = 0;
    Some(clip)
}

pub(crate) fn from_html(html: &str) -> Option<Clipboard> {
    if html.len() > MAX_HTML {
        return None;
    }
    let mut reader = Reader::from_str(html);
    reader.config_mut().check_end_names = false;
    let mut raw_text = None;
    loop {
        match reader.read_event().ok()? {
            Event::Start(tag) | Event::Empty(tag) => {
                let name = tag.name().as_ref().to_ascii_lowercase();
                if raw_text.is_some() {
                    continue;
                }
                if matches!(name.as_slice(), b"script" | b"style" | b"textarea" | b"title") {
                    raw_text = Some(name);
                    continue;
                }
                if name != b"meta" {
                    continue;
                }
                let mut marker = false;
                let mut content = None;
                for attr in tag.attributes() {
                    let attr = attr.ok()?;
                    if attr.key.as_ref().eq_ignore_ascii_case(b"name") {
                        marker = attr.decode_and_unescape_value(reader.decoder()).ok()?.eq_ignore_ascii_case("erpcons-clip");
                    } else if attr.key.as_ref().eq_ignore_ascii_case(b"content") {
                        content = Some(attr.decode_and_unescape_value(reader.decoder()).ok()?.into_owned());
                    }
                }
                if marker && let Some(clip) = content.as_deref().and_then(decode) {
                    return Some(clip);
                }
            }
            Event::End(tag) => {
                if raw_text.as_ref().is_some_and(|name| tag.name().as_ref().eq_ignore_ascii_case(name)) {
                    raw_text = None;
                }
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use deckcraft_model::{MediaId, MediaItem, Shape, ShapeId, ShapeKind, Table, TextBody};
    use serde_json::json;

    fn value() -> Value {
        json!({"v": 1, "editor": "deckcraft", "clipboard": Clipboard {
            shapes: vec![Shape { id: ShapeId(10), text: Some(TextBody::from_text("fragment")), ..Default::default() }],
            plain: "fragment".into(),
            ..Default::default()
        }, "media_data": []})
    }

    fn marker(value: &Value) -> String {
        format!("<meta name='erpcons-clip' content='{}'>", base64_encode(&serde_json::to_vec(value).unwrap()))
    }

    #[test]
    fn marker_accepts_browser_wrappers_attribute_order_and_case() {
        let payload = base64_encode(&serde_json::to_vec(&value()).unwrap());
        let html = format!("<html><head><META CONTENT='{payload}' NAME='erpcons-clip'></head><body><p>fallback</p></body></html>");
        assert_eq!(from_html(&html).unwrap().plain, "fragment");
    }

    #[test]
    fn marker_rejects_unknown_version_editor_and_missing_fields() {
        let mut version = value();
        version["v"] = json!(2);
        let mut editor = value();
        editor["editor"] = json!("another-editor");
        for bad in [version, editor, json!({"v": 1}), json!({})] {
            assert!(from_html(&marker(&bad)).is_none());
        }
    }

    #[test]
    fn marker_rejects_extreme_numbers_deep_nesting_and_duplicate_ids() {
        let mut number = value();
        number["clipboard"]["shapes"][0]["xfrm"] = json!({"x": 1e300, "y": 0, "w": 1, "h": 1});
        assert!(from_html(&marker(&number)).is_none());
        let mut duplicate = value();
        let shape = duplicate["clipboard"]["shapes"][0].clone();
        duplicate["clipboard"]["shapes"].as_array_mut().unwrap().push(shape);
        assert!(from_html(&marker(&duplicate)).is_none());
        let mut nested = json!(null);
        for _ in 0..66 {
            nested = json!([nested]);
        }
        let mut deep = value();
        deep["extra"] = nested;
        assert!(from_html(&marker(&deep)).is_none());
    }

    #[test]
    fn marker_rejects_out_of_bounds_table_merges_and_missing_media() {
        let mut table = Table::new(1, 1, 10.0, 10.0);
        table.cell_mut(0, 0).unwrap().grid_span = u32::MAX;
        let mut bad = value();
        bad["clipboard"]["shapes"][0]["kind"] = serde_json::to_value(ShapeKind::Table(table)).unwrap();
        assert!(from_html(&marker(&bad)).is_none());
        let mut missing = value();
        missing["clipboard"]["shapes"][0]["kind"] =
            serde_json::to_value(ShapeKind::Picture { fill: deckcraft_model::style::PictureFill { media: MediaId(77), ..Default::default() } })
                .unwrap();
        assert!(from_html(&marker(&missing)).is_none());
    }

    #[test]
    fn marker_round_trip_preserves_media_bytes_and_resets_paste_counter() {
        let clip = Clipboard {
            media: vec![MediaItem { id: MediaId(4), data: Arc::new(vec![1, 2, 3]), ..Default::default() }],
            pastes: 12,
            ..Default::default()
        };
        let copy = from_html(&encode(&clip).unwrap()).unwrap();
        assert_eq!(copy.media[0].data, clip.media[0].data);
        assert_eq!(copy.pastes, 0);
    }

    #[test]
    fn marker_limits_apply_to_both_directions_and_media() {
        let oversize = "A".repeat(MAX_MARKER + 1);
        assert!(decode(&oversize).is_none());
        let clip = Clipboard { media: vec![MediaItem { data: Arc::new(vec![0; MAX_JSON + 1]), ..Default::default() }], ..Default::default() };
        assert!(encode(&clip).is_none());
        assert!(from_html(&" ".repeat(MAX_HTML + 1)).is_none());
    }

    #[test]
    fn marker_in_script_style_or_comment_is_ignored() {
        let marker = marker(&value());
        for html in [format!("<script>{marker}</script>"), format!("<style>{marker}</style>"), format!("<!--{marker}-->")] {
            assert!(from_html(&html).is_none());
        }
    }
}
