#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use deckcraft_model::text::Strike;
use deckcraft_model::{ColorRef, Fill, MediaItem, ShapeKind};
use serde_json::json;

use crate::{Session, TextSel, clipboard_marker};

fn text_shape(s: &mut Session, text: &str) -> deckcraft_model::ShapeId {
    s.execute("shape.insert", &json!({"preset": "rect", "rect": [10, 10, 80, 40], "text": text})).unwrap();
    s.doc().unwrap().selection.shapes[0]
}

#[test]
fn copy_table_is_tsv_and_html() {
    let mut s = Session::with_new();
    s.execute("insert.table", &json!({"rows": 2, "cols": 2, "data": [["A", "B"], ["C", "D"]]})).unwrap();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(result["text"], "A\tB\nC\tD");
    let html = result["html"].as_str().unwrap();
    assert!(html.contains("<table"));
    assert_eq!(html.matches("<td ").count(), 4);
    assert!(html.contains("name=\"erpcons-clip\""));
}

#[test]
fn copy_table_quotes_tabs_newlines_and_quotes() {
    let mut s = Session::with_new();
    s.execute("insert.table", &json!({"rows": 2, "cols": 2, "data": [["a\tb", "say \"hi\""], ["two\nlines", "<&"]]})).unwrap();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(result["text"], "\"a\tb\"\t\"say \"\"hi\"\"\"\n\"two\nlines\"\t<&");
    assert!(result["html"].as_str().unwrap().contains("&lt;&amp;"));
}

#[test]
fn copy_cell_range_contains_only_selected_cells_and_cross_tab_table() {
    let mut s = Session::with_new();
    s.execute("insert.table", &json!({"rows": 2, "cols": 2, "data": [["A", "B"], ["C", "D"]]})).unwrap();
    s.execute("table.selectCells", &json!({"from": [1, 1], "to": [0, 1]})).unwrap();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(result["text"], "B\nD");
    assert_eq!(result["html"].as_str().unwrap().matches("<td ").count(), 2);
    let mut other = Session::with_new();
    other.execute("edit.paste", &result).unwrap();
    let shape = other.doc().unwrap().current_slide().unwrap().shapes.last().unwrap();
    let ShapeKind::Table(table) = &shape.kind else { panic!("expected table") };
    assert_eq!((table.n_rows(), table.n_cols()), (2, 1));
    assert_eq!(table.cell(1, 0).unwrap().text.text(), "D");
}

#[test]
fn copy_merged_table_keeps_spans_and_basic_cell_styles() {
    let mut s = Session::with_new();
    s.execute("insert.table", &json!({"rows": 2, "cols": 2, "data": [["A", "B"], ["C", "D"]]})).unwrap();
    s.execute("table.cellFill", &json!({"color": "#123456"})).unwrap();
    s.execute("table.merge", &json!({"from": [0, 0], "to": [1, 1]})).unwrap();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    let html = result["html"].as_str().unwrap();
    assert!(html.contains("colspan=\"2\" rowspan=\"2\""));
    assert!(html.contains("background-color:#123456;"));
    assert_eq!(html.matches("<td ").count(), 1);
    s.execute("table.selectCells", &json!({"from": [1, 1], "to": [1, 1]})).unwrap();
    let selected = s.execute("edit.copy", &json!({})).unwrap();
    assert!(selected["text"].as_str().unwrap().contains('A'));
    assert!(selected["html"].as_str().unwrap().contains("colspan=\"1\" rowspan=\"1\""));
}

#[test]
fn copy_shape_marker_pastes_in_fresh_session_with_one_undo() {
    let mut s = Session::with_new();
    let id = text_shape(&mut s, "Cross-tab <&");
    s.execute("format.bold", &json!({})).unwrap();
    let expected = s.doc().unwrap().shape(id).unwrap().text.clone();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    let html = result["html"].as_str().unwrap();
    assert!(html.contains("name=\"erpcons-clip\""));
    assert!(html.contains("Cross-tab &lt;&amp;"));
    let mut other = Session::with_new();
    let before = other.doc().unwrap().doc.clone();
    let undo_before = other.doc().unwrap().history.undo.len();
    other.execute("edit.paste", &json!({"html": html, "text": "different fallback"})).unwrap();
    let state = other.doc().unwrap();
    assert_eq!(state.current_slide().unwrap().shapes.len(), before.slides[0].shapes.len() + 1);
    assert_eq!(state.current_slide().unwrap().shapes.last().unwrap().text.as_ref().unwrap().text(), "Cross-tab <&");
    assert_eq!(state.current_slide().unwrap().shapes.last().unwrap().text, expected);
    assert_eq!(state.history.undo.len(), undo_before + 1);
    other.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&other.doc().unwrap().doc, &before));
}

#[test]
fn garbage_marker_falls_back_to_plain_text() {
    for marker in ["not-base64!", "e30=", "eyJ2Ijo5OX0="] {
        let mut s = Session::with_new();
        let before = s.doc().unwrap().current_slide().unwrap().shapes.len();
        s.execute("edit.paste", &json!({"html": format!("<meta name='erpcons-clip' content='{marker}'>"), "text": "fallback"})).unwrap();
        let shapes = &s.doc().unwrap().current_slide().unwrap().shapes;
        assert_eq!(shapes.len(), before + 1);
        assert_eq!(shapes.last().unwrap().text.as_ref().unwrap().text(), "fallback");
    }
}

#[test]
fn oversized_marker_is_omitted_but_portable_content_remains() {
    let mut s = Session::with_new();
    let id = text_shape(&mut s, "kept");
    s.edit(|doc, sel| {
        let shapes = crate::shapes_mut(doc, sel).unwrap();
        deckcraft_model::find_shape_mut(shapes, id).unwrap().descr = "x".repeat(clipboard_marker::MAX_MARKER);
        Ok(())
    })
    .unwrap();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(result["text"], "kept");
    assert!(result["html"].as_str().unwrap().contains("kept"));
    assert!(!result["html"].as_str().unwrap().contains("erpcons-clip"));
}

#[test]
fn oversized_textless_fragment_has_no_clipboard_payload() {
    let mut s = Session::with_new();
    let id = text_shape(&mut s, "");
    s.edit(|doc, sel| {
        let shape = deckcraft_model::find_shape_mut(crate::shapes_mut(doc, sel).unwrap(), id).unwrap();
        shape.descr = "x".repeat(clipboard_marker::MAX_MARKER);
        Ok(())
    })
    .unwrap();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(result["text"], "");
    assert!(result.get("html").is_none());
}

#[test]
fn empty_selection_and_caret_copy_have_no_html_or_text() {
    let mut s = Session::with_new();
    let id = text_shape(&mut s, "text");
    s.select(|_, sel| {
        sel.shapes.clear();
        sel.slides.clear();
    })
    .unwrap();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(result["text"], "");
    assert_eq!(result["copied"], false);
    assert!(result.get("html").is_none());
    s.select(|_, sel| sel.text = Some(TextSel { shape: id, ..Default::default() })).unwrap();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(result["text"], "");
    assert!(result.get("html").is_none());
}

#[test]
fn textless_shape_has_no_name_placeholder_and_has_internal_marker() {
    let mut s = Session::with_new();
    text_shape(&mut s, "");
    let result = s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(result["text"], "");
    assert!(result["html"].as_str().unwrap().contains("erpcons-clip"));
}

#[test]
fn copy_text_selection_exports_run_formatting_and_paragraphs() {
    let mut s = Session::with_new();
    let id = text_shape(&mut s, "hello\nworld");
    s.edit(|doc, sel| {
        let shape = deckcraft_model::find_shape_mut(crate::shapes_mut(doc, sel).unwrap(), id).unwrap();
        let props = &mut shape.text.as_mut().unwrap().paragraphs[0].runs[0].props;
        props.bold = Some(true);
        props.italic = Some(true);
        props.underline = Some("sng".into());
        props.strike = Some(Strike::Single);
        props.size = Some(23.0);
        props.font = Some("Inter".into());
        props.fill = Some(Fill::solid(ColorRef::rgb(deckcraft_color::Rgba::rgb(18, 52, 86))));
        sel.text = Some(TextSel { shape: id, anchor: (0, 1), caret: (1, 3), ..Default::default() });
        Ok(())
    })
    .unwrap();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(result["text"], "ello\nwor");
    let html = result["html"].as_str().unwrap();
    for css in ["font-weight:bold", "font-style:italic", "underline line-through", "font-size:23pt", "Inter", "color:#123456"] {
        assert!(html.contains(css), "{css}: {html}");
    }
    assert_eq!(html.matches("<p ").count(), 2);
}

#[test]
fn copy_slide_includes_all_shape_text_and_pastes_in_fresh_session() {
    let mut s = Session::with_new();
    text_shape(&mut s, "body one");
    text_shape(&mut s, "body two");
    let result = s.execute("edit.copy", &json!({"scope": "slides"})).unwrap();
    let plain = result["text"].as_str().unwrap();
    assert!(plain.contains("body one") && plain.contains("body two"));
    let mut other = Session::with_new();
    let before = other.doc().unwrap().doc.clone();
    other.execute("edit.paste", &result).unwrap();
    assert_eq!(other.doc().unwrap().doc.slides.len(), before.slides.len() + 1);
    other.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&other.doc().unwrap().doc, &before));
}

#[test]
fn picture_marker_transfers_bytes_and_remaps_colliding_media_ids() {
    let mut s = Session::with_new();
    let id = text_shape(&mut s, "");
    let media = s
        .edit(|doc, sel| {
            let media = doc.add_media("pixel.png", "image/png", vec![1, 2, 3]);
            let shape = deckcraft_model::find_shape_mut(crate::shapes_mut(doc, sel).unwrap(), id).unwrap();
            shape.text = None;
            shape.kind = ShapeKind::Picture { fill: deckcraft_model::style::PictureFill { media, ..Default::default() } };
            Ok(media)
        })
        .unwrap();
    let result = s.execute("edit.copy", &json!({})).unwrap();
    assert!(result["html"].as_str().unwrap().contains("data:image/png;base64,AQID"));
    let mut other = Session::with_new();
    other
        .edit(|doc, _| {
            doc.media.push(MediaItem { id: media, content_type: "image/png".into(), data: Arc::new(vec![9]), ..Default::default() });
            doc.fix_next_id();
            Ok(())
        })
        .unwrap();
    let before = other.doc().unwrap().doc.clone();
    other.execute("edit.paste", &result).unwrap();
    let state = other.doc().unwrap();
    let ShapeKind::Picture { fill } = &state.current_slide().unwrap().shapes.last().unwrap().kind else { panic!("expected picture") };
    assert_ne!(fill.media, media);
    assert_eq!(state.doc.media(fill.media).unwrap().data.as_slice(), &[1, 2, 3]);
    assert_eq!(state.doc.media(media).unwrap().data.as_slice(), &[9]);
    other.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&other.doc().unwrap().doc, &before));
}

#[test]
fn cut_also_returns_the_internal_marker() {
    let mut s = Session::with_new();
    let id = text_shape(&mut s, "cut text");
    let result = s.execute("edit.cut", &json!({})).unwrap();
    assert!(result["html"].as_str().unwrap().contains("erpcons-clip"));
    assert!(s.doc().unwrap().shape(id).is_none());
}
