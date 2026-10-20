//! Route external clipboard data to text editing or real slide objects.

use deckcraft_geom::Xfrm;
use deckcraft_model::text::TextBody;
use serde_json::{Value, json};

use crate::{Result, Session, html_clip, table_paste};

pub(crate) fn external(s: &mut Session, html: Option<&str>, text: Option<&str>) -> Result<Option<Value>> {
    if let Some(html) = html.filter(|s| !s.is_empty()) {
        let Some(clip) = html_clip::read(html) else {
            let fallback = text.map(str::to_owned).unwrap_or_else(|| html_clip::plain(html));
            return plain(s, &fallback).map(Some);
        };
        let editing_shape = s.doc()?.selection.text.as_ref().is_some_and(|t| t.notes || t.cell.is_none());
        if !editing_shape && let Some(table) = clip.table {
            return table_paste::paste(s, table).map(Some);
        }
        if !editing_shape && let Some(table) = text.and_then(table_paste::tsv) {
            return table_paste::paste(s, table).map(Some);
        }
        if !clip.text.is_empty() {
            return body(s, &clip.text).map(Some);
        }
        if let Some(text) = text {
            return plain(s, text).map(Some);
        }
        return Ok(Some(json!({})));
    }
    if let Some(text) = text {
        let editing_shape = s.doc()?.selection.text.as_ref().is_some_and(|t| t.notes || t.cell.is_none());
        if !editing_shape && let Some(table) = table_paste::tsv(text) {
            return table_paste::paste(s, table).map(Some);
        }
        return plain(s, text).map(Some);
    }
    Ok(None)
}

pub(crate) fn body(s: &mut Session, source: &TextBody) -> Result<Value> {
    if s.doc()?.selection.text.is_some() {
        return crate::cmd::text::paste_body(s, source);
    }
    let size = s.doc()?.doc.slide_size;
    let rect = Xfrm::new(size.width * 0.1, size.height * 0.1, size.width * 0.8, size.height * 0.8);
    let mut shape = crate::cmd::insert::new_text_box(rect, "");
    let mut text = source.clone();
    if let Some(defaults) = shape.text.as_ref() {
        text.body.inherit(&defaults.body);
    }
    shape.text = Some(text);
    let id = crate::cmd::insert::add_shape(s, shape, true)?;
    s.select(|_, sel| sel.cells = None)?;
    crate::cmd::text::fit_text_box(s, id)?;
    Ok(json!({"id": id}))
}

pub(crate) fn plain(s: &mut Session, text: &str) -> Result<Value> {
    if s.doc()?.selection.text.is_some() {
        let st = s.doc()?;
        let mut source = TextBody::from_text(text);
        if let Some(t) = &st.selection.text
            && let Some(current) = crate::cmd::text::body_of(st, t)
        {
            let props = deckcraft_model::edit::props_at(current, t.ordered().0);
            for para in &mut source.paragraphs {
                if let Some(original) = current.paragraphs.get(t.ordered().0.0) {
                    para.level = original.level;
                    para.props = original.props.clone();
                }
                para.end_props = props.clone();
                for run in &mut para.runs {
                    run.props = props.clone();
                }
            }
        }
        return crate::cmd::text::paste_body(s, &source);
    }
    let size = s.doc()?.doc.slide_size;
    let rect = [size.width * 0.25, size.height * 0.4, size.width * 0.5, 40.0];
    s.execute("insert.textBox", &json!({"rect": rect, "text": text}))
}
