//! Find and Replace (Home › Editing): text in every slide's shapes, table cells and speaker notes,
//! in slide order. Masters and layouts are not searched, as in PowerPoint's Normal view.

use std::sync::Arc;

use deckcraft_model::edit as ed;
use deckcraft_model::text::{Paragraph, RunKind, TextBody};
use deckcraft_model::{Presentation, Shape, ShapeId, ShapeKind, Slide};
use serde_json::{Value, json};

use super::*;
use crate::{DocState, Result, Session, Target, TextSel};

const QUERY: &str = "{text, matchCase?: bool, wholeWords?: bool}";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            noundo "edit.find",
            "Find Next",
            ["Home", "Editing"],
            None,
            "{text, matchCase?: bool, wholeWords?: bool} → {found, count, slide, id?, cell?, notes, paragraph, range: [start, end]} — selects the next match after the selection, wrapping around to the first slide",
            has_slide,
            find
        ),
        cmd!(
            "edit.replace",
            "Replace",
            ["Home", "Editing"],
            None,
            "{text, with, matchCase?, wholeWords?} → {replaced, found, …} — replaces the selected match (if the selection is one), then selects the next",
            has_slide,
            replace
        ),
        cmd!("edit.replaceAll", "Replace All", ["Home", "Editing"], None, "{text, with, matchCase?, wholeWords?} → {count}", has_slide, replace_all),
    ]
}

/// One text body of a slide: a shape's, a table cell's, or the speaker notes (`shape: None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Place {
    shape: Option<ShapeId>,
    cell: Option<(usize, usize)>,
}

const NOTES: Place = Place { shape: None, cell: None };

impl Place {
    fn of(t: &TextSel) -> Place {
        if t.notes { NOTES } else { Place { shape: Some(t.shape), cell: t.cell } }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Hit {
    slide: usize,
    /// The body's index in its slide's search order ([`places`]).
    body: usize,
    place: Place,
    para: usize,
    start: usize,
    end: usize,
}

impl Hit {
    fn key(&self) -> (usize, usize, usize, usize) {
        (self.slide, self.body, self.para, self.start)
    }
}

struct Query {
    needle: Vec<char>,
    match_case: bool,
    whole_words: bool,
}

impl Query {
    fn from_params(p: &Value, cmd: &str) -> Result<Query> {
        let text = str_param(p, "text").filter(|t| !t.is_empty()).ok_or_else(|| bad(cmd, format!("missing `text`: {QUERY}")))?;
        let match_case = bool_or(p, "matchCase", false);
        let needle = text.chars().map(|c| if match_case { c } else { fold(c) }).collect();
        Ok(Query { needle, match_case, whole_words: bool_or(p, "wholeWords", false) })
    }

    /// Char ranges of the matches in a paragraph, left to right and not overlapping. A match lies
    /// in text runs only: fields (slide numbers, dates) and line breaks are not text to replace.
    fn matches(&self, para: &Paragraph) -> Vec<(usize, usize)> {
        let mut chars = Vec::new();
        let mut in_text = Vec::new();
        for r in &para.runs {
            for c in r.content().chars() {
                chars.push(if self.match_case { c } else { fold(c) });
                in_text.push(r.kind == RunKind::Text);
            }
        }
        let n = self.needle.len();
        let mut out = Vec::new();
        let mut i = 0;
        while n > 0 && i + n <= chars.len() {
            let hit = chars.get(i..i + n) == Some(self.needle.as_slice())
                && in_text.get(i..i + n).is_some_and(|t| t.iter().all(|x| *x))
                && (!self.whole_words || (!word_char_at(&chars, i.checked_sub(1)) && !word_char_at(&chars, Some(i + n))));
            if hit {
                out.push((i, i + n));
                i += n;
            } else {
                i += 1;
            }
        }
        out
    }
}

/// Lower case as one char, so offsets stay those of the original text (`İ` folds to itself).
fn fold(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

fn word_char_at(chars: &[char], i: Option<usize>) -> bool {
    i.and_then(|i| chars.get(i)).is_some_and(|c| c.is_alphanumeric() || *c == '_')
}

/// A slide's text bodies in search order: shapes back to front (into groups, table cells row by
/// row), then the notes.
fn places(slide: &Slide) -> Vec<Place> {
    fn walk(shapes: &[Shape], out: &mut Vec<Place>, depth: usize) {
        if depth > 64 {
            return;
        }
        for sh in shapes {
            match &sh.kind {
                ShapeKind::Group { children, .. } => walk(children, out, depth + 1),
                ShapeKind::Table(t) => {
                    for (r, row) in t.rows.iter().enumerate() {
                        for (c, cell) in row.cells.iter().enumerate() {
                            if !cell.is_covered() {
                                out.push(Place { shape: Some(sh.id), cell: Some((r, c)) });
                            }
                        }
                    }
                }
                ShapeKind::Shape | ShapeKind::Connector { .. } if sh.text.is_some() => out.push(Place { shape: Some(sh.id), cell: None }),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&slide.shapes, &mut out, 0);
    out.push(NOTES);
    out
}

fn body(slide: &Slide, place: Place) -> Option<&TextBody> {
    let Some(id) = place.shape else { return Some(&slide.notes) };
    let sh = deckcraft_model::find_shape(&slide.shapes, id)?;
    match (place.cell, &sh.kind) {
        (Some((r, c)), ShapeKind::Table(t)) => t.cell(r, c).map(|x| &x.text),
        (None, _) => sh.text.as_ref(),
        _ => None,
    }
}

fn body_mut(slide: &mut Slide, place: Place) -> Option<&mut TextBody> {
    let Some(id) = place.shape else { return Some(&mut slide.notes) };
    let sh = deckcraft_model::find_shape_mut(&mut slide.shapes, id)?;
    match (place.cell, &mut sh.kind) {
        (Some((r, c)), ShapeKind::Table(t)) => t.cell_mut(r, c).map(|x| &mut x.text),
        (None, _) => sh.text.as_mut(),
        _ => None,
    }
}

fn hits(doc: &Presentation, q: &Query) -> Vec<Hit> {
    let mut out = Vec::new();
    for (slide, sl) in doc.slides.iter().enumerate() {
        for (bi, place) in places(sl).into_iter().enumerate() {
            let Some(b) = body(sl, place) else { continue };
            for (para, p) in b.paragraphs.iter().enumerate() {
                out.extend(q.matches(p).into_iter().map(|(start, end)| Hit { slide, body: bi, place, para, start, end }));
            }
        }
    }
    out
}

/// Where Find Next starts: after the selected text, else at the selected shape, else at the start
/// of the current slide.
fn cursor(st: &DocState) -> (usize, usize, usize, usize) {
    let slide = st.selection.slide;
    let Some(sl) = st.current_slide().filter(|_| st.selection.target == Target::Slides) else { return (slide, 0, 0, 0) };
    let ps = places(sl);
    if let Some(t) = &st.selection.text {
        let (_, end) = t.ordered();
        return match ps.iter().position(|p| *p == Place::of(t)) {
            Some(bi) => (slide, bi, end.0, end.1),
            None => (slide, 0, 0, 0),
        };
    }
    let first = st.selection.shapes.first().and_then(|id| ps.iter().position(|p| p.shape == Some(*id)));
    (slide, first.unwrap_or(0), 0, 0)
}

/// The selection, when it is exactly one match of `q`.
fn selected_hit(st: &DocState, q: &Query) -> Option<Hit> {
    if st.selection.target != Target::Slides {
        return None;
    }
    let t = st.selection.text.as_ref()?;
    let ((p0, start), (p1, end)) = t.ordered();
    if p0 != p1 || start == end {
        return None;
    }
    let sl = st.current_slide()?;
    let place = Place::of(t);
    let bi = places(sl).iter().position(|p| *p == place)?;
    let para = body(sl, place)?.paragraphs.get(p0)?;
    q.matches(para).contains(&(start, end)).then_some(Hit { slide: st.selection.slide, body: bi, place, para: p0, start, end })
}

fn select_hit(s: &mut Session, h: &Hit) -> Result<()> {
    s.select(|doc, sel| {
        sel.slide = h.slide;
        sel.slides = doc.slides.get(h.slide).map(|x| vec![x.id]).unwrap_or_default();
        sel.target = Target::Slides;
        sel.cells = None;
        let (anchor, caret) = ((h.para, h.start), (h.para, h.end));
        match h.place.shape {
            Some(id) => {
                sel.shapes = vec![id];
                sel.text = Some(TextSel { shape: id, anchor, caret, cell: h.place.cell, notes: false, line_end: None });
            }
            None => {
                sel.shapes.clear();
                sel.text = Some(TextSel { anchor, caret, notes: true, ..Default::default() });
            }
        }
    })
}

fn hit_json(h: &Hit, count: usize) -> Value {
    let mut v =
        json!({"found": true, "count": count, "slide": h.slide, "notes": h.place.shape.is_none(), "paragraph": h.para, "range": [h.start, h.end]});
    if let Some(id) = h.place.shape {
        v["id"] = json!(id);
    }
    if let Some((r, c)) = h.place.cell {
        v["cell"] = json!([r, c]);
    }
    v
}

fn find_next(s: &mut Session, q: &Query) -> Result<Value> {
    let st = s.doc()?;
    let all = hits(&st.doc, q);
    let from = cursor(st);
    let Some(h) = all.iter().find(|h| h.key() >= from).or_else(|| all.first()).copied() else {
        return Ok(json!({"found": false, "count": 0}));
    };
    select_hit(s, &h)?;
    Ok(hit_json(&h, all.len()))
}

/// Replaces one match, keeping the formatting of its first character; returns the position after
/// the replacement.
fn replace_range(body: &mut TextBody, h: &Hit, with: &str) -> ed::Pos {
    let props = ed::props_at(body, (h.para, h.start + 1));
    let at = ed::delete(body, (h.para, h.start), (h.para, h.end));
    if with.is_empty() { at } else { ed::insert(body, at, with, Some(props)) }
}

fn with_text<'a>(p: &'a Value, cmd: &str) -> Result<&'a str> {
    str_param(p, "with").ok_or_else(|| bad(cmd, "missing `with`"))
}

/// Grows or shrinks the auto-fitting text boxes on any slide, as typing in them does.
fn refit(s: &mut Session, boxes: &[(usize, ShapeId)]) -> Result<()> {
    let saved = s.doc()?.selection.clone();
    let r = boxes.iter().try_for_each(|(slide, id)| {
        let st = s.doc_mut()?;
        st.selection.slide = *slide;
        st.selection.target = Target::Slides;
        text::fit_text_box(s, *id)
    });
    s.doc_mut()?.selection = saved;
    r
}

fn find(s: &mut Session, p: &Value) -> Result<Value> {
    let q = Query::from_params(p, "edit.find")?;
    find_next(s, &q)
}

fn replace(s: &mut Session, p: &Value) -> Result<Value> {
    let q = Query::from_params(p, "edit.replace")?;
    let with = with_text(p, "edit.replace")?;
    let current = selected_hit(s.doc()?, &q);
    if let Some(h) = current {
        s.edit(|doc, sel| {
            let body = doc
                .slides
                .get_mut(h.slide)
                .and_then(|sl| body_mut(Arc::make_mut(sl), h.place))
                .ok_or_else(|| bad("edit.replace", "the text is gone"))?;
            let end = replace_range(body, &h, with);
            if let Some(t) = sel.text.as_mut() {
                t.anchor = end;
                t.caret = end;
            }
            Ok(())
        })?;
        if let (Some(id), None) = (h.place.shape, h.place.cell) {
            refit(s, &[(h.slide, id)])?;
        }
    }
    let mut r = find_next(s, &q)?;
    r["replaced"] = json!(current.is_some());
    Ok(r)
}

fn replace_all(s: &mut Session, p: &Value) -> Result<Value> {
    let q = Query::from_params(p, "edit.replaceAll")?;
    let with = with_text(p, "edit.replaceAll")?;
    let all = hits(&s.doc()?.doc, &q);
    if all.is_empty() {
        return Ok(json!({"count": 0}));
    }
    s.edit(|doc, sel| {
        // Back to front, so the offsets of the matches still to replace stay valid.
        for h in all.iter().rev() {
            if let Some(body) = doc.slides.get_mut(h.slide).and_then(|sl| body_mut(Arc::make_mut(sl), h.place)) {
                replace_range(body, h, with);
            }
        }
        // The insertion point in a rewritten body no longer marks the same text.
        if sel.target == Target::Slides
            && let Some(t) = &sel.text
            && all.iter().any(|h| h.slide == sel.slide && h.place == Place::of(t))
        {
            sel.shapes = if t.notes { vec![] } else { vec![t.shape] };
            sel.text = None;
        }
        Ok(())
    })?;
    let mut boxes: Vec<(usize, ShapeId)> =
        all.iter().filter(|h| h.place.cell.is_none()).filter_map(|h| h.place.shape.map(|id| (h.slide, id))).collect();
    boxes.dedup();
    refit(s, &boxes)?;
    Ok(json!({"count": all.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Slide 1: the title "Beta alpha beta". Slide 2: a text box "alphabet BETA", a 1×2 table
    /// ["x", "beta"] and the notes "notes beta".
    fn deck() -> (Session, u32, u32, u32) {
        let mut s = Session::with_new();
        s.prefs.autocorrect = false;
        let title = s.doc().unwrap().current_slide().unwrap().shapes[0].id.0;
        s.execute("text.set", &json!({"id": title, "text": "Beta alpha beta"})).unwrap();
        s.execute("slide.new", &json!({"layout": "blank"})).unwrap();
        let tb = s.execute("insert.textBox", &json!({"rect": [40, 40, 300, 60], "text": "alphabet BETA"})).unwrap()["id"].as_u64().unwrap() as u32;
        let table = s.execute("insert.table", &json!({"rows": 1, "cols": 2})).unwrap()["id"].as_u64().unwrap() as u32;
        s.execute("text.set", &json!({"id": table, "cell": [0, 0], "text": "x"})).unwrap();
        s.execute("text.set", &json!({"id": table, "cell": [0, 1], "text": "beta"})).unwrap();
        s.execute("slide.notes", &json!({"text": "notes beta"})).unwrap();
        s.execute("slide.go", &json!({"index": 0})).unwrap();
        (s, title, tb, table)
    }

    fn found(r: &Value) -> (u64, Value, Value, bool, Value) {
        assert_eq!(r["found"], true, "{r}");
        (r["slide"].as_u64().unwrap(), r["id"].clone(), r["cell"].clone(), r["notes"].as_bool().unwrap(), r["range"].clone())
    }

    fn body_text(s: &Session, slide: usize, id: u32) -> String {
        let sl = &s.doc().unwrap().doc.slides[slide];
        deckcraft_model::find_shape(&sl.shapes, ShapeId(id)).unwrap().text.as_ref().unwrap().text()
    }

    #[test]
    fn find_next_walks_shapes_cells_and_notes_then_wraps() {
        let (mut s, title, tb, table) = deck();
        let undo = s.doc().unwrap().history.undo.len();
        let q = json!({"text": "beta"});
        let mut seen = vec![];
        for _ in 0..6 {
            seen.push(found(&s.execute("edit.find", &q).unwrap()));
        }
        let at = |slide, id: u32, cell: Value, notes, r: [usize; 2]| (slide, if notes { Value::Null } else { json!(id) }, cell, notes, json!(r));
        assert_eq!(
            seen,
            vec![
                at(0, title, Value::Null, false, [0, 4]),
                at(0, title, Value::Null, false, [11, 15]),
                at(1, tb, Value::Null, false, [9, 13]),
                at(1, table, json!([0, 1]), false, [0, 4]),
                at(1, 0, Value::Null, true, [6, 10]),
                at(0, title, Value::Null, false, [0, 4]),
            ]
        );
        // The match is selected as text, on its slide.
        s.execute("edit.find", &q).unwrap();
        s.execute("edit.find", &q).unwrap();
        let st = s.doc().unwrap();
        assert_eq!(st.selection.slide, 1);
        assert_eq!(st.selection.shapes, vec![ShapeId(tb)]);
        let t = st.selection.text.clone().unwrap();
        assert_eq!((t.shape, t.anchor, t.caret), (ShapeId(tb), (0, 9), (0, 13)));
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "finding is not an undo step");
    }

    #[test]
    fn match_case_and_whole_words_narrow_the_matches() {
        let (mut s, ..) = deck();
        assert_eq!(s.execute("edit.find", &json!({"text": "BETA"})).unwrap()["count"], 5);
        let r = s.execute("edit.find", &json!({"text": "BETA", "matchCase": true})).unwrap();
        assert_eq!((r["count"].clone(), r["slide"].clone(), r["range"].clone()), (json!(1), json!(1), json!([9, 13])));
        assert_eq!(s.execute("edit.find", &json!({"text": "alpha"})).unwrap()["count"], 2, "alphabet contains alpha");
        assert_eq!(s.execute("edit.find", &json!({"text": "alpha", "wholeWords": true})).unwrap()["count"], 1);
        let r = s.execute("edit.find", &json!({"text": "gamma"})).unwrap();
        assert_eq!(r, json!({"found": false, "count": 0}));
        assert!(s.execute("edit.find", &json!({"text": ""})).is_err());
    }

    #[test]
    fn replace_changes_the_selected_match_then_selects_the_next() {
        let (mut s, title, ..) = deck();
        let q = json!({"text": "beta", "with": "gamma"});
        // Nothing selected yet: Replace only finds.
        let r = s.execute("edit.replace", &q).unwrap();
        assert_eq!((r["replaced"].clone(), r["range"].clone()), (json!(false), json!([0, 4])));
        assert_eq!(body_text(&s, 0, title), "Beta alpha beta");
        let r = s.execute("edit.replace", &q).unwrap();
        assert_eq!(r["replaced"], true);
        assert_eq!(body_text(&s, 0, title), "gamma alpha beta");
        assert_eq!(r["range"], json!([12, 16]), "the next match, after the replacement");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(body_text(&s, 0, title), "Beta alpha beta");
        assert!(s.execute("edit.replace", &json!({"text": "beta"})).is_err(), "`with` is required");
    }

    #[test]
    fn replace_all_counts_and_is_one_undo_step() {
        let (mut s, title, tb, table) = deck();
        let r = s.execute("edit.replaceAll", &json!({"text": "beta", "with": "β", "wholeWords": true})).unwrap();
        assert_eq!(r["count"], 5);
        assert_eq!(body_text(&s, 0, title), "β alpha β");
        assert_eq!(body_text(&s, 1, tb), "alphabet β");
        let sl = s.doc().unwrap().doc.slides[1].clone();
        let ShapeKind::Table(t) = &deckcraft_model::find_shape(&sl.shapes, ShapeId(table)).unwrap().kind else { panic!("not a table") };
        assert_eq!(t.cell(0, 1).unwrap().text.text(), "β");
        assert_eq!(sl.notes.text(), "notes β");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(body_text(&s, 0, title), "Beta alpha beta");
        assert_eq!(s.doc().unwrap().doc.slides[1].notes.text(), "notes beta");
        assert_eq!(s.execute("edit.replaceAll", &json!({"text": "gamma", "with": "x"})).unwrap()["count"], 0);
    }
}
