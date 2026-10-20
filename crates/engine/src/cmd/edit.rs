//! Edit menu: undo/redo, clipboard, duplicate, delete, select all.

use std::sync::Arc;

use deckcraft_model::{Shape, ShapeId, Slide, SlideId};
use serde_json::{Value, json};

use super::*;
use crate::{HistoryEntry, Result, Session, clipboard_html, clipboard_marker, clipboard_media};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(noundo "edit.undo", "Undo", ["Edit"], Some("Cmd+Z"), "{}", can_undo, undo),
        cmd!(noundo "edit.redo", "Redo", ["Edit"], Some("Cmd+Y"), "{}", can_redo, redo),
        cmd!("edit.cut", "Cut", ["Home", "Clipboard"], Some("Cmd+X"), "{scope?: slides}", has_doc, cut),
        cmd!(noundo "edit.copy", "Copy", ["Home", "Clipboard"], Some("Cmd+C"), "{scope?: slides} → {text, html?, copied}", has_doc, copy),
        cmd!(
            "edit.paste",
            "Paste",
            ["Home", "Clipboard"],
            Some("Cmd+V"),
            "{text?: plain text to paste, html?: string, scope?: slides}",
            has_doc,
            paste
        ),
        cmd!("edit.pasteText", "Paste and Match Formatting", ["Edit"], Some("Cmd+Shift+V"), "{text?}", has_doc, paste_text),
        cmd!("edit.duplicate", "Duplicate", ["Edit"], Some("Cmd+D"), "{ids?}", has_selection, duplicate),
        cmd!("edit.delete", "Delete", ["Edit"], Some("Delete"), "{ids?, scope?: slides}", has_doc, delete),
        cmd!(noundo "edit.selectAll", "Select All", ["Edit"], Some("Cmd+A"), "{}", has_slide, select_all),
        cmd!(noundo "edit.deselect", "Deselect", [], Some("Escape"), "{}", has_doc, deselect),
        cmd!(noundo "edit.select", "Select Objects", [], None, "{ids: [id], add?: bool, toggle?: bool}", has_slide, select),
    ]
}

fn restore(s: &mut Session, redo: bool) -> Result<Value> {
    let st = s.doc_mut()?;
    let (from, to) = if redo { (&mut st.history.redo, &mut st.history.undo) } else { (&mut st.history.undo, &mut st.history.redo) };
    let Some(e) = from.pop() else { return ok() };
    to.push(HistoryEntry { label: e.label.clone(), doc: st.doc.clone(), selection: st.selection.clone() });
    st.doc = e.doc;
    st.selection = e.selection;
    st.interaction = None;
    st.revision += 1;
    Ok(json!({"label": e.label}))
}

fn undo(s: &mut Session, _p: &Value) -> Result<Value> {
    restore(s, false)
}
fn redo(s: &mut Session, _p: &Value) -> Result<Value> {
    restore(s, true)
}

fn slides_scope(s: &Session, p: &Value) -> bool {
    str_param(p, "scope") == Some("slides")
        || s.active().is_some_and(|d| d.selection.shapes.is_empty() && d.selection.text.is_none() && !d.selection.slides.is_empty())
}

fn copy_inner(s: &mut Session, p: &Value) -> Result<(String, String, bool)> {
    let st = s.doc()?;
    // Text selection.
    if let Some(t) = &st.selection.text {
        if t.is_range() {
            let body = super::text::body_of(st, t).cloned().unwrap_or_default();
            let (a, b) = t.ordered();
            let plain = deckcraft_model::edit::text_range(&body, a, b);
            if plain.is_empty() {
                return Ok((String::new(), String::new(), false));
            }
            let sliced = deckcraft_model::edit::slice(&body, a, b);
            let ctx = super::text::ctx_for(&st.doc, &st.selection);
            let html = clipboard_html::text(&sliced, ctx.as_ref(), st.shape(t.shape));
            s.clipboard = crate::Clipboard { text: Some(sliced), plain: plain.clone(), ..Default::default() };
            return Ok((plain, html, true));
        }
        return Ok((String::new(), String::new(), false));
    }
    if slides_scope(s, p) {
        let ids: Vec<SlideId> =
            if st.selection.slides.is_empty() { st.current_slide().map(|x| vec![x.id]).unwrap_or_default() } else { st.selection.slides.clone() };
        let slides: Vec<Slide> = ids.iter().filter_map(|id| st.doc.slide(*id).cloned()).collect();
        let mids = clipboard_media::collect(&slides)?;
        let media: Vec<_> = st.doc.media.iter().filter(|m| mids.contains(&m.id)).cloned().collect();
        let mut texts = vec![];
        let mut html = String::new();
        for slide in &slides {
            let ctx = deckcraft_model::resolve::Ctx::for_slide(&st.doc, slide);
            let (plain, markup) = clipboard_html::shapes(&slide.shapes, &media, ctx.as_ref());
            if !plain.is_empty() {
                texts.push(plain);
            }
            html.push_str(&markup);
        }
        let plain = texts.join("\n");
        let copied = !slides.is_empty();
        s.clipboard = crate::Clipboard { slides, media, plain: plain.clone(), ..Default::default() };
        return Ok((plain, html, copied));
    }
    if let Some((id, from, to)) = st.selection.cells
        && let Some(shape) = st.shape(id)
        && let deckcraft_model::ShapeKind::Table(table) = &shape.kind
    {
        let Some(table) = clipboard_html::table_range(table, from, to) else {
            return Ok((String::new(), String::new(), false));
        };
        let ctx = super::text::ctx_for(&st.doc, &st.selection);
        let (plain, html) = clipboard_html::table(&table, ctx.as_ref());
        let mut shape = shape.clone();
        shape.xfrm = Some(xfrm_of(&st.doc, &st.selection, &shape));
        if let Some(xfrm) = &mut shape.xfrm {
            xfrm.w = table.width();
            xfrm.h = table.height();
        }
        shape.kind = deckcraft_model::ShapeKind::Table(table);
        let shapes = vec![shape];
        let mids = clipboard_media::collect(&shapes)?;
        let media = st.doc.media.iter().filter(|m| mids.contains(&m.id)).cloned().collect();
        s.clipboard = crate::Clipboard { shapes, media, plain: plain.clone(), ..Default::default() };
        let copied = !html.is_empty();
        return Ok((plain, html, copied));
    }
    let shapes: Vec<Shape> = st
        .selection
        .shapes
        .iter()
        .filter_map(|id| {
            st.shape(*id).cloned().map(|mut sh| {
                if sh.xfrm.is_none() {
                    sh.xfrm = Some(xfrm_of(&st.doc, &st.selection, &sh));
                }
                sh
            })
        })
        .collect();
    let mids = clipboard_media::collect(&shapes)?;
    let media: Vec<_> = st.doc.media.iter().filter(|m| mids.contains(&m.id)).cloned().collect();
    let ctx = super::text::ctx_for(&st.doc, &st.selection);
    let (plain, html) = clipboard_html::shapes(&shapes, &media, ctx.as_ref());
    let copied = !shapes.is_empty();
    s.clipboard = crate::Clipboard { shapes, media, plain: plain.clone(), ..Default::default() };
    Ok((plain, html, copied))
}

fn copy(s: &mut Session, p: &Value) -> Result<Value> {
    let (text, mut html, copied) = copy_inner(s, p)?;
    if copied
        && (!s.clipboard.shapes.is_empty() || !s.clipboard.slides.is_empty())
        && let Some(marker) = clipboard_marker::encode(&s.clipboard)
    {
        html.push_str(&marker);
    }
    let mut result = json!({"text": text, "copied": copied});
    if !html.is_empty() {
        result["html"] = Value::String(html);
    }
    Ok(result)
}

fn cut(s: &mut Session, p: &Value) -> Result<Value> {
    let result = copy(s, p)?;
    delete(s, p)?;
    Ok(result)
}

/// Give pasted shapes fresh ids (groups included) and map connector ends.
pub(crate) fn reid(doc: &mut deckcraft_model::Presentation, shapes: &mut [Shape]) {
    fn rec(doc: &mut deckcraft_model::Presentation, shapes: &mut [Shape], depth: usize) {
        if depth > 64 {
            return;
        }
        for s in shapes {
            s.id = ShapeId(doc.alloc_id());
            if let Some(ch) = s.children_mut() {
                rec(doc, ch, depth + 1);
            }
        }
    }
    rec(doc, shapes, 0);
}

fn paste(s: &mut Session, p: &Value) -> Result<Value> {
    let marker = str_param(p, "html").and_then(clipboard_marker::from_html);
    let internal = marker.is_some();
    if let Some(clip) = marker {
        s.clipboard = clip;
    }
    let html = str_param(p, "html");
    let text = str_param(p, "text");
    let external = html.is_some() || text.is_some_and(|text| text != s.clipboard.plain);
    if !internal
        && external
        && let Some(result) = crate::rich_paste::external(s, html, text)?
    {
        return Ok(result);
    }
    if crate::table_paste::has_target(s)
        && let [shape] = s.clipboard.shapes.as_slice()
        && let deckcraft_model::ShapeKind::Table(table) = &shape.kind
    {
        let table = table.clone();
        return crate::table_paste::paste(s, table);
    }
    let editing = s.doc()?.selection.text.is_some();
    if editing {
        if let Some(body) = s.clipboard.text.clone() {
            return super::text::paste_body(s, &body);
        }
        let plain = s.clipboard.plain.clone();
        return paste_plain(s, &plain);
    }
    let clip = s.clipboard.clone();
    if !clip.slides.is_empty() {
        let at = s.doc()?.selection.slide + 1;
        return s.edit(|doc, sel| {
            let media = clipboard_media::bring(doc, &clip.media);
            let mut new_ids = vec![];
            for (k, sl) in clip.slides.iter().enumerate() {
                let mut sl = clipboard_media::remap(sl, &media)?;
                sl.id = SlideId(doc.alloc_id());
                reid(doc, &mut sl.shapes);
                if doc.layout(sl.layout).is_none() {
                    sl.layout = doc.masters.first().and_then(|m| m.layouts.first()).map(|l| l.id).unwrap_or_default();
                }
                new_ids.push(sl.id);
                let i = (at + k).min(doc.slides.len());
                doc.slides.insert(i, Arc::new(sl));
            }
            sel.slide = (at + new_ids.len()).saturating_sub(1).min(doc.slides.len().saturating_sub(1));
            sel.slides = new_ids.clone();
            sel.shapes.clear();
            Ok(json!({"slides": new_ids.len()}))
        });
    }
    if !clip.shapes.is_empty() {
        s.clipboard.pastes = s.clipboard.pastes.saturating_add(1);
        let off = s.prefs.duplicate_offset * s.clipboard.pastes as f64;
        // Pasting on another slide keeps the position; on the same slide it offsets.
        let same_slide = s.doc().ok().is_some_and(|d| clip.shapes.iter().any(|c| d.shape(c.id).is_some()));
        let off = if same_slide { off } else { 0.0 };
        return s.edit(|doc, sel| {
            let media = clipboard_media::bring(doc, &clip.media);
            let mut shapes = clipboard_media::remap(&clip.shapes, &media)?;
            reid(doc, &mut shapes);
            for sh in &mut shapes {
                if let Some(x) = sh.xfrm.as_mut() {
                    x.x += off;
                    x.y += off;
                }
                // A pasted placeholder becomes a regular shape keeping its look.
                if sh.ph.is_some() && sel.target == crate::Target::Slides {
                    sh.ph = None;
                }
            }
            let ids: Vec<ShapeId> = shapes.iter().map(|x| x.id).collect();
            let list = crate::shapes_mut(doc, sel).ok_or_else(|| bad("edit.paste", "no slide"))?;
            list.extend(shapes);
            sel.shapes = ids.clone();
            sel.text = None;
            Ok(json!({"ids": ids}))
        });
    }
    if let Some(body) = &clip.text {
        return crate::rich_paste::body(s, body);
    }
    if !clip.plain.is_empty() {
        let t = clip.plain.clone();
        return paste_plain(s, &t);
    }
    ok()
}

fn paste_plain(s: &mut Session, text: &str) -> Result<Value> {
    crate::rich_paste::plain(s, text)
}

fn paste_text(s: &mut Session, p: &Value) -> Result<Value> {
    let t = str_param(p, "text").map(String::from).unwrap_or_else(|| s.clipboard.plain.clone());
    paste_plain(s, &t)
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = targets(s, p)?;
    let off = s.prefs.duplicate_offset;
    s.edit(|doc, sel| {
        let list = super::current_shapes(doc, sel).cloned().unwrap_or_default();
        let mut copies: Vec<Shape> = list.iter().filter(|x| ids.contains(&x.id)).cloned().collect();
        let boxes: Vec<_> = copies.iter().map(|c| xfrm_of(doc, sel, c)).collect();
        reid(doc, &mut copies);
        for (c, b) in copies.iter_mut().zip(boxes) {
            let mut x = b;
            x.x += off;
            x.y += off;
            c.xfrm = Some(x);
            if sel.target == crate::Target::Slides {
                c.ph = None;
            }
            if !c.name.is_empty() {
                c.name = format!("{} copy", c.name.trim_end_matches(" copy"));
            }
        }
        let new_ids: Vec<ShapeId> = copies.iter().map(|c| c.id).collect();
        let shapes = crate::shapes_mut(doc, sel).ok_or_else(|| bad("edit.duplicate", "no slide"))?;
        shapes.extend(copies);
        sel.shapes = new_ids.clone();
        Ok(json!({"ids": new_ids}))
    })
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    if s.doc()?.selection.text.is_some() && ids_param(p, "ids").is_none() {
        return super::text::delete_dir(s, "forward");
    }
    if slides_scope(s, p) && ids_param(p, "ids").is_none() {
        return s.execute("slide.delete", &json!({}));
    }
    let ids = targets(s, p)?;
    if ids.is_empty() {
        return ok();
    }
    s.edit(|doc, sel| {
        let shapes = crate::shapes_mut(doc, sel).ok_or_else(|| bad("edit.delete", "no slide"))?;
        let n = remove_shapes(shapes, &ids);
        // Animations of deleted shapes go too.
        if sel.target == crate::Target::Slides
            && let Some(sl) = doc.slides.get_mut(sel.slide)
        {
            Arc::make_mut(sl).animations.retain(|a| !ids.contains(&a.shape));
        }
        sel.shapes.retain(|i| !ids.contains(i));
        sel.text = None;
        Ok(json!({"deleted": n}))
    })
}

/// Remove shapes by id anywhere in the tree; empty groups go too.
pub(crate) fn remove_shapes(shapes: &mut Vec<Shape>, ids: &[ShapeId]) -> usize {
    let before = count(shapes);
    fn rec(v: &mut Vec<Shape>, ids: &[ShapeId], depth: usize) {
        v.retain(|s| !ids.contains(&s.id));
        if depth > 64 {
            return;
        }
        for s in v.iter_mut() {
            if let Some(ch) = s.children_mut() {
                rec(ch, ids, depth + 1);
            }
        }
        v.retain(|s| !s.is_group() || !s.children().is_empty());
    }
    rec(shapes, ids, 0);
    before.saturating_sub(count(shapes))
}

fn count(v: &[Shape]) -> usize {
    let mut n = 0;
    deckcraft_model::walk(v, &mut |_, _| n += 1);
    n
}

fn select_all(s: &mut Session, _p: &Value) -> Result<Value> {
    if s.doc()?.selection.text.is_some() {
        return s.execute("text.selectAll", &json!({}));
    }
    s.select(|_, sel| {
        sel.text = None;
    })?;
    let ids: Vec<ShapeId> = s.doc()?.shapes().iter().filter(|x| !x.hidden && !x.locked).map(|x| x.id).collect();
    s.select(|_, sel| sel.shapes = ids.clone())?;
    Ok(json!({"ids": ids}))
}

fn deselect(s: &mut Session, _p: &Value) -> Result<Value> {
    s.painter = None;
    s.anim_painter = None;
    s.select(|_, sel| {
        if let Some(t) = sel.text.take() {
            // Esc while editing text selects the shape.
            if !t.notes {
                sel.shapes = vec![t.shape];
            }
        } else {
            sel.shapes.clear();
            sel.cells = None;
        }
    })?;
    ok()
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = ids_param(p, "ids").or_else(|| id_param(p, "id").map(|i| vec![i])).unwrap_or_default();
    let add = bool_or(p, "add", false);
    let toggle = bool_or(p, "toggle", false);
    let valid: Vec<ShapeId> = ids.into_iter().filter(|i| s.active().is_some_and(|d| d.shape(*i).is_some())).collect();
    s.select(|_, sel| {
        sel.text = None;
        sel.cells = None;
        if toggle {
            for i in &valid {
                if let Some(k) = sel.shapes.iter().position(|x| x == i) {
                    sel.shapes.remove(k);
                } else {
                    sel.shapes.push(*i);
                }
            }
        } else if add {
            for i in &valid {
                if !sel.shapes.contains(i) {
                    sel.shapes.push(*i);
                }
            }
        } else {
            sel.shapes = valid.clone();
        }
    })?;
    Ok(json!({"selected": s.doc()?.selection.shapes}))
}
