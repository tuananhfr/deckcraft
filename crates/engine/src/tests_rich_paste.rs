#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use deckcraft_model::text::{Align, LevelStyle, ParaProps, RunProps, TextBody};
use deckcraft_model::{ColorRef, Fill, Presentation, ShapeId, ShapeKind};
use serde_json::{Value, json};

use crate::{Session, tests_html_clip::MERGED_TABLE};

fn snapshot(s: &Session) -> (Arc<Presentation>, usize) {
    (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len())
}

fn undo_once(s: &mut Session, before: (Arc<Presentation>, usize)) {
    assert_eq!(s.doc().unwrap().history.undo.len(), before.1 + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(Arc::ptr_eq(&s.doc().unwrap().doc, &before.0));
}

fn table(s: &Session, id: ShapeId) -> &deckcraft_model::Table {
    let ShapeKind::Table(table) = &s.doc().unwrap().shape(id).unwrap().kind else { panic!("expected table") };
    table
}

fn text_shape(s: &mut Session, text: &str) -> ShapeId {
    s.execute("shape.insert", &json!({"preset": "rect", "text": text})).unwrap();
    s.doc().unwrap().selection.shapes[0]
}

#[test]
fn html_creates_formatted_text_box_with_one_undo() {
    let mut s = Session::with_new();
    let before = snapshot(&s);
    s.execute("edit.paste", &json!({"html": "<p><b>Rich</b> text</p><h3>Heading</h3>", "text": "fallback"})).unwrap();
    let state = s.doc().unwrap();
    let shape = state.shape(state.selection.shapes[0]).unwrap();
    assert!(shape.text_box);
    let text = shape.text.as_ref().unwrap();
    assert_eq!(text.text(), "Rich text\nHeading");
    assert_eq!(text.paragraphs[0].runs[0].props.bold, Some(true));
    assert_eq!(text.paragraphs[1].runs[0].props.size, Some(20.0));
    undo_once(&mut s, before);
}

#[test]
fn html_replaces_selected_shape_text_and_keeps_first_list_paragraph() {
    let mut s = Session::with_new();
    let id = text_shape(&mut s, "old");
    s.execute("text.edit", &json!({"id": id})).unwrap();
    s.execute("text.selectAll", &json!({})).unwrap();
    let before = snapshot(&s);
    s.execute("edit.paste", &json!({"html": "<ul><li><b>A</b></li><li>B</li></ul>"})).unwrap();
    let text = s.doc().unwrap().shape(id).unwrap().text.as_ref().unwrap();
    assert_eq!(text.text(), "A\nB");
    assert_eq!(text.paragraphs[0].runs[0].props.bold, Some(true));
    assert!(text.paragraphs[0].props.bullet.is_some());
    undo_once(&mut s, before);
}

#[test]
fn html_table_pasted_into_shape_text_keeps_runs_instead_of_inserting_table() {
    let mut s = Session::with_new();
    let id = text_shape(&mut s, "");
    s.execute("text.edit", &json!({"id": id})).unwrap();
    let before = snapshot(&s);
    s.execute("edit.paste", &json!({"html": "<table><tr><td><b>A</b></td><td>B</td></tr></table>"})).unwrap();
    let shape = s.doc().unwrap().shape(id).unwrap();
    assert!(matches!(shape.kind, ShapeKind::Shape));
    let text = shape.text.as_ref().unwrap();
    assert_eq!(text.text(), "A\nB");
    assert_eq!(text.paragraphs[0].runs[0].props.bold, Some(true));
    undo_once(&mut s, before);
}

#[test]
fn paste_text_ignores_html_and_keeps_destination_formatting() {
    let mut s = Session::with_new();
    let id = text_shape(&mut s, "old");
    s.execute("format.italic", &json!({})).unwrap();
    s.execute("text.edit", &json!({"id": id})).unwrap();
    s.execute("text.selectAll", &json!({})).unwrap();
    let before = snapshot(&s);
    s.execute("edit.pasteText", &json!({"text": "plain\ttext\nsecond\tline", "html": "<b>not used</b>"})).unwrap();
    let text = s.doc().unwrap().shape(id).unwrap().text.as_ref().unwrap();
    assert_eq!(text.text(), "plain\ttext\nsecond\tline");
    assert_eq!(text.paragraphs[0].runs[0].props.italic, Some(true));
    assert_ne!(text.paragraphs[0].runs[0].props.bold, Some(true));
    undo_once(&mut s, before);
}

#[test]
fn tsv_and_merged_html_create_real_tables_sized_to_slide_with_one_undo() {
    for payload in [json!({"text": "A\tB\nC\tD\nE\tF"}), json!({"html": MERGED_TABLE})] {
        let mut s = Session::with_new();
        let before = snapshot(&s);
        s.execute("edit.paste", &payload).unwrap();
        let state = s.doc().unwrap();
        let id = state.selection.shapes[0];
        let table = table(&s, id);
        assert_eq!(table.n_rows(), 3);
        assert_eq!(table.n_cols(), if payload.get("html").is_some() { 3 } else { 2 });
        let x = state.shape(id).unwrap().xfrm.unwrap();
        assert!(x.x >= 0.0 && x.y >= 0.0);
        assert!(x.x + x.w <= state.doc.slide_size.width);
        assert!(x.y + x.h <= state.doc.slide_size.height);
        if payload.get("html").is_some() {
            assert_eq!(table.cell(0, 0).unwrap().row_span, 2);
        }
        undo_once(&mut s, before);
    }
}

#[test]
fn selected_table_paste_starts_at_selected_cell_and_extends_once() {
    let mut s = Session::with_new();
    s.execute("insert.table", &json!({"rows": 2, "cols": 2, "data": [["kept", "x"], ["y", "z"]]})).unwrap();
    let id = s.doc().unwrap().selection.shapes[0];
    s.execute("table.selectCells", &json!({"from": [1, 1], "to": [1, 1]})).unwrap();
    let before = snapshot(&s);
    let shapes_before = s.doc().unwrap().current_slide().unwrap().shapes.len();
    s.execute("edit.paste", &json!({"text": "A\tB\nC\tD\nE\tF"})).unwrap();
    let table = table(&s, id);
    assert_eq!((table.n_rows(), table.n_cols()), (4, 3));
    assert_eq!(table.cell(0, 0).unwrap().text.text(), "kept");
    assert_eq!(table.cell(1, 1).unwrap().text.text(), "A");
    assert_eq!(table.cell(3, 2).unwrap().text.text(), "F");
    assert_eq!(s.doc().unwrap().current_slide().unwrap().shapes.len(), shapes_before);
    undo_once(&mut s, before);
}

#[test]
fn edited_table_cell_paste_fills_grid_and_keeps_html_merge() {
    let mut s = Session::with_new();
    s.execute("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    let id = s.doc().unwrap().selection.shapes[0];
    s.execute("text.edit", &json!({"id": id, "cell": [1, 1]})).unwrap();
    let before = snapshot(&s);
    s.execute("edit.paste", &json!({"html": MERGED_TABLE})).unwrap();
    let table = table(&s, id);
    assert_eq!((table.n_rows(), table.n_cols()), (4, 4));
    assert_eq!(table.cell(1, 1).unwrap().grid_span, 2);
    assert_eq!(table.cell(1, 1).unwrap().row_span, 2);
    assert!(table.cell(2, 2).unwrap().is_covered());
    assert_eq!(table.cell(3, 3).unwrap().text.text(), "I");
    undo_once(&mut s, before);
}

#[test]
fn copied_internal_table_fills_selected_table_with_one_undo() {
    let mut source = Session::with_new();
    source.execute("insert.table", &json!({"rows": 2, "cols": 2, "data": [["A", "B"], ["C", "D"]]})).unwrap();
    let payload = source.execute("edit.copy", &json!({})).unwrap();
    let mut s = Session::with_new();
    s.execute("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    let id = s.doc().unwrap().selection.shapes[0];
    s.execute("table.selectCells", &json!({"from": [1, 1], "to": [1, 1]})).unwrap();
    let before = snapshot(&s);
    s.execute("edit.paste", &payload).unwrap();
    assert_eq!(table(&s, id).cell(2, 2).unwrap().text.text(), "D");
    undo_once(&mut s, before);
}

#[test]
fn local_clipboard_preserves_rich_text_and_fills_selected_table() {
    let mut s = Session::with_new();
    let id = text_shape(&mut s, "Rich");
    s.execute("format.bold", &json!({})).unwrap();
    s.execute("text.edit", &json!({"id": id})).unwrap();
    s.execute("text.selectAll", &json!({})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("text.exit", &json!({})).unwrap();
    let before = snapshot(&s);
    s.execute("edit.paste", &json!({})).unwrap();
    let state = s.doc().unwrap();
    let pasted = state.shape(state.selection.shapes[0]).unwrap().text.as_ref().unwrap();
    assert_eq!(pasted.paragraphs[0].runs[0].props.bold, Some(true));
    undo_once(&mut s, before);
    s.execute("insert.table", &json!({"rows": 2, "cols": 2, "data": [["A", "B"], ["C", "D"]]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("insert.table", &json!({"rows": 2, "cols": 2})).unwrap();
    let id = s.doc().unwrap().selection.shapes[0];
    let before = snapshot(&s);
    s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(table(&s, id).cell(1, 1).unwrap().text.text(), "D");
    undo_once(&mut s, before);
}

#[test]
fn paste_splits_intersecting_old_merge_without_leaving_covered_cells() {
    let mut s = Session::with_new();
    s.execute("insert.table", &json!({"rows": 3, "cols": 3})).unwrap();
    let id = s.doc().unwrap().selection.shapes[0];
    s.execute("table.merge", &json!({"from": [0, 1], "to": [2, 2]})).unwrap();
    s.execute("table.selectCells", &json!({"from": [0, 0], "to": [0, 0]})).unwrap();
    let before = snapshot(&s);
    s.execute("edit.paste", &json!({"text": "A\tB\nC\tD"})).unwrap();
    assert!(!table(&s, id).cell(2, 2).unwrap().is_covered());
    assert_eq!(table(&s, id).cell(0, 1).unwrap().grid_span, 1);
    undo_once(&mut s, before);
}

fn inherited_placeholder(s: &mut Session, old: &str) -> (ShapeId, RunProps) {
    let id = s.doc().unwrap().current_slide().unwrap().shapes[0].id;
    let layout_id = s.doc().unwrap().current_slide().unwrap().layout;
    let ph = s.doc().unwrap().shape(id).unwrap().ph.clone().unwrap();
    let run = RunProps {
        font: Some("Inter".into()),
        size: Some(37.0),
        fill: Some(Fill::solid(ColorRef::rgb(deckcraft_color::Rgba::rgb(18, 52, 86)))),
        ..Default::default()
    };
    s.edit(|doc, sel| {
        let master = Arc::make_mut(&mut doc.masters[0]);
        let layout = master.layouts.iter_mut().find(|l| l.id == layout_id).unwrap();
        let shape = layout.shapes.iter_mut().find(|shape| shape.ph.as_ref().is_some_and(|p| p.idx == ph.idx)).unwrap();
        shape
            .text
            .as_mut()
            .unwrap()
            .list_style
            .set(0, LevelStyle { run: run.clone(), para: ParaProps { align: Some(Align::Right), ..Default::default() } });
        let shapes = crate::shapes_mut(doc, sel).unwrap();
        deckcraft_model::find_shape_mut(shapes, id).unwrap().text = Some(TextBody::from_text(old));
        Ok(())
    })
    .unwrap();
    s.execute("text.edit", &json!({"id": id})).unwrap();
    (id, run)
}

#[test]
fn placeholder_replacement_materializes_layout_props_for_rich_plain_and_match_paste() {
    for old in ["", "replace me"] {
        for (command, payload) in [
            ("edit.paste", json!({"html": "<p>New</p><p>Text</p>"})),
            ("edit.paste", json!({"text": "New\nText"})),
            ("edit.pasteText", json!({"text": "New\nText", "html": "<b>ignored</b>"})),
        ] {
            let mut s = Session::with_new();
            let (id, expected) = inherited_placeholder(&mut s, old);
            s.execute("text.selectAll", &json!({})).unwrap();
            let before = snapshot(&s);
            s.execute(command, &payload).unwrap();
            let shape = s.doc().unwrap().shape(id).unwrap();
            assert!(shape.ph.is_none());
            assert!(shape.xfrm.is_some());
            let text = shape.text.as_ref().unwrap();
            assert_eq!(text.text(), "New\nText");
            for para in &text.paragraphs {
                assert_eq!(para.props.align, Some(Align::Right));
                for run in &para.runs {
                    assert_eq!(run.props.font, expected.font);
                    assert_eq!(run.props.size, expected.size);
                    assert_eq!(run.props.fill, expected.fill);
                }
            }
            undo_once(&mut s, before);
            assert!(s.doc().unwrap().shape(id).unwrap().ph.is_some());
        }
    }
}

#[test]
fn partial_placeholder_paste_preserves_placeholder_and_explicit_source_style() {
    let mut s = Session::with_new();
    let (id, expected) = inherited_placeholder(&mut s, "Old ");
    let before = snapshot(&s);
    s.execute("edit.paste", &json!({"html": "<b>new</b>"})).unwrap();
    let shape = s.doc().unwrap().shape(id).unwrap();
    assert!(shape.ph.is_some());
    let text = shape.text.as_ref().unwrap();
    assert_eq!(text.text(), "Old new");
    let run = text.paragraphs[0].runs.last().unwrap();
    assert_eq!(run.props.bold, Some(true));
    assert_eq!(run.props.size, expected.size);
    undo_once(&mut s, before);
}

#[test]
fn rejected_html_uses_plain_flavor_without_stale_clipboard_and_one_undo() {
    let payloads: [Value; 3] = [
        json!({"html": format!("{}X{}", "<div>".repeat(65), "</div>".repeat(65)), "text": "safe"}),
        json!({"html": format!("<p style='{}'>X</p>", "x".repeat(8193)), "text": "safe"}),
        json!({"html": format!("<table><tr>{}</tr></table>", "<td>x</td>".repeat(100_000)), "text": "safe"}),
    ];
    for payload in payloads {
        let mut s = Session::with_new();
        s.clipboard.plain = "safe".into();
        let before = snapshot(&s);
        s.execute("edit.paste", &payload).unwrap();
        let state = s.doc().unwrap();
        assert_eq!(state.shape(state.selection.shapes[0]).unwrap().text.as_ref().unwrap().text(), "safe");
        undo_once(&mut s, before);
    }
}
