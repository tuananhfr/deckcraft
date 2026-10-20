use deckcraft_engine::Session;
use serde_json::json;

#[test]
fn empty_copy_is_distinguished_from_a_selected_textless_shape() {
    let mut session = Session::with_new();
    let selection = &mut session.active_mut().unwrap().selection;
    selection.shapes.clear();
    selection.slides.clear();
    selection.text = None;
    let empty = session.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(empty["copied"], false);
    assert_eq!(empty["text"], "");
    session.execute("shape.insert", &json!({"preset": "rect", "rect": [10, 10, 100, 40]})).unwrap();
    let shape = session.execute("edit.copy", &json!({})).unwrap();
    assert_eq!(shape["copied"], true);
    assert_eq!(shape["text"], "");
}

#[test]
fn unusable_html_pastes_supplied_text_once_with_one_undo() {
    let mut session = Session::with_new();
    let before = session.active().unwrap().doc.clone();
    let undo_before = session.active().unwrap().history.undo.len();
    session.execute("edit.paste", &json!({"html": "<meta charset='utf-8'>", "text": "fallback once"})).unwrap();
    let state = session.active().unwrap();
    assert_eq!(state.history.undo.len(), undo_before + 1);
    assert_eq!(state.doc.slides[0].shapes.len(), before.slides[0].shapes.len() + 1);
    assert_eq!(state.doc.slides[0].shapes.last().unwrap().text.as_ref().unwrap().text(), "fallback once");
    session.execute("edit.undo", &json!({})).unwrap();
    assert!(std::sync::Arc::ptr_eq(&session.active().unwrap().doc, &before));
}

#[test]
fn unusable_html_falls_back_to_text_and_usable_html_is_imported() {
    for (html, expected) in [
        ("<!-- no usable content -->", "fallback once"),
        ("<body></body>", "fallback once"),
        ("<p>Imported HTML</p>", "Imported HTML"),
    ] {
        let mut session = Session::with_new();
        let before = session.active().unwrap().doc.clone();
        let undo_before = session.active().unwrap().history.undo.len();
        session.execute("edit.paste", &json!({"html": html, "text": "fallback once"})).unwrap();
        let state = session.active().unwrap();
        assert_eq!(state.history.undo.len(), undo_before + 1);
        assert_eq!(state.doc.slides[0].shapes.len(), before.slides[0].shapes.len() + 1);
        let shape = state.doc.slides[0].shapes.last().unwrap();
        assert!(shape.text_box);
        assert_eq!(shape.text.as_ref().unwrap().text(), expected);
        session.execute("edit.undo", &json!({})).unwrap();
        assert!(std::sync::Arc::ptr_eq(&session.active().unwrap().doc, &before));
        assert_eq!(session.active().unwrap().history.undo.len(), undo_before);
    }
}
