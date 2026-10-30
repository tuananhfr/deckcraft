//! A read-only presentation (the office site's view and comment modes, the desktop's licence lock)
//! refuses every change, by command, key, typing, input method and pointer, and keeps its content
//! and undo history when its access changes either way.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::tools::{Mods, PointerEvent, PointerKind, ToolKind};
use crate::*;

/// The sample deck with a rectangle selected, and that rectangle's id and box.
fn deck() -> (Session, u64) {
    let mut s = Session::new();
    sample::open_sample(&mut s).unwrap();
    s.prefs.snap_to_grid = false;
    s.prefs.smart_guides = false;
    let id = s.execute("shape.insert", &json!({"preset": "rect", "rect": [100, 100, 100, 80]})).unwrap()["id"].as_u64().unwrap();
    s.execute("edit.select", &json!({"ids": [id]})).unwrap();
    (s, id)
}

fn lock(s: &mut Session, can_download: bool) {
    let uid = s.active().unwrap().uid;
    assert!(s.set_access(uid, Access::read_only(can_download)));
}

/// What a refused change must leave as it was.
fn snapshot(s: &Session) -> (Arc<model::Presentation>, usize, usize) {
    let d = s.active().unwrap();
    (d.doc.clone(), d.history.undo.len(), d.history.redo.len())
}

fn unchanged(s: &Session, before: &(Arc<model::Presentation>, usize, usize), what: &str) {
    let now = snapshot(s);
    assert!(Arc::ptr_eq(&before.0, &now.0), "{what} changed a read-only presentation");
    assert_eq!((before.1, before.2), (now.1, now.2), "{what} touched the undo history");
}

fn read_only_refusal(r: Result<Value>) -> bool {
    matches!(r, Err(EngineError::Disabled(_, why)) if why == access::READ_ONLY)
}

fn png() -> String {
    let doc = model::defaults::new_presentation(None);
    let img = render::render_slide(&doc, 0, &render::RenderOpts { scale: 0.05, ..Default::default() });
    cmd::base64_encode(&img.to_png())
}

/// One command of each family, with parameters that change an editable presentation.
fn one_per_family(shape: u64) -> Vec<(&'static str, Value)> {
    vec![
        ("slide.new", json!({})),
        ("slide.move", json!({"from": 0, "to": 1})),
        ("slide.notes", json!({"text": "notes"})),
        ("section.add", json!({"name": "S"})),
        ("shape.insert", json!({"preset": "star5", "rect": [10, 10, 50, 50]})),
        ("shape.fill", json!({"color": "#FF0000", "ids": [shape]})),
        ("shape.move", json!({"dx": 5, "dy": 5, "ids": [shape]})),
        ("arrange.sendToBack", json!({"ids": [shape]})),
        ("format.bold", json!({"ids": [shape]})),
        ("text.set", json!({"id": shape, "text": "changed"})),
        ("insert.textBox", json!({"rect": [20, 20, 100, 30]})),
        ("insert.table", json!({"rows": 2, "cols": 2})),
        ("insert.picture", json!({"name": "p.png", "data": png()})),
        ("design.theme", json!({"name": "DeckCraft"})),
        ("design.background", json!({"color": "#00FF00"})),
        ("transition.set", json!({"kind": "fade"})),
        ("animation.add", json!({"effect": "fade", "ids": [shape]})),
        ("comment.add", json!({"text": "hi"})),
        ("edit.paste", json!({"text": "pasted"})),
        ("edit.duplicate", json!({"ids": [shape]})),
        ("edit.delete", json!({"ids": [shape]})),
        ("edit.replaceAll", json!({"text": "Northern", "with": "Southern"})),
        ("master.insertLayout", json!({"name": "L"})),
        ("show.setup", json!({"loop": true})),
        ("file.properties", json!({"title": "T"})),
    ]
}

#[test]
fn every_command_family_is_refused_read_only() {
    let shape = deck().1;
    for (id, params) in one_per_family(shape) {
        // The same command changes the presentation when it may be edited…
        let (mut s, _) = deck();
        let before = s.active().unwrap().doc.clone();
        let r = s.execute(id, &params);
        assert!(r.is_ok() && !Arc::ptr_eq(&before, &s.active().unwrap().doc), "{id} should change an editable deck: {r:?}");
        // …and nothing when it is read-only, whatever the mode was called.
        let (mut s, _) = deck();
        lock(&mut s, true);
        let before = snapshot(&s);
        assert!(read_only_refusal(s.execute(id, &params)), "{id} wasn't refused");
        unchanged(&s, &before, id);
    }
}

#[test]
fn every_command_is_refused_or_harmless_read_only() {
    let (mut s, shape) = deck();
    s.execute("text.edit", &json!({"id": shape})).unwrap();
    s.execute("edit.deselect", &json!({})).unwrap();
    s.execute("edit.select", &json!({"ids": [shape]})).unwrap();
    lock(&mut s, true);
    let uid = s.active().unwrap().uid;
    for spec in command_specs() {
        // These act on other presentations or write files, never on the read-only one.
        let skipped = ["file.close", "file.new", "file.open", "file.openBytes", "file.reopenClosed", "window.next", "file.recovery.open"];
        if skipped.contains(&spec.id) || matches!(spec.id, "file.save" | "file.saveAs" | "file.export" | "file.saveTemplate") {
            continue;
        }
        for params in [json!({}), json!({"ids": [shape], "id": shape, "text": "x", "index": 0, "name": "x"})] {
            s.ui_requests.clear();
            let before = snapshot(&s);
            let r = s.execute(spec.id, &params);
            assert_eq!(s.active().unwrap().uid, uid);
            unchanged(&s, &before, spec.id);
            if access::refusal(Access::read_only(true), spec).is_some() {
                assert!(read_only_refusal(r), "{} wasn't refused", spec.id);
            }
        }
    }
}

#[test]
fn undo_and_redo_are_refused_read_only() {
    let (mut s, shape) = deck();
    s.execute("shape.move", &json!({"dx": 5, "dy": 0, "ids": [shape]})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    lock(&mut s, true);
    let before = snapshot(&s);
    assert!(read_only_refusal(s.execute("edit.undo", &json!({}))));
    assert!(read_only_refusal(s.execute("edit.redo", &json!({}))));
    assert!(read_only_refusal(s.execute("file.revert", &json!({}))));
    assert!(read_only_refusal(s.execute("file.save", &json!({}))));
    unchanged(&s, &before, "undo/redo");
    let undo = find_command("edit.undo").unwrap();
    assert_eq!(s.available(undo), Err(access::READ_ONLY.to_string()), "the ribbon greys Undo out");
}

#[test]
fn keys_typing_and_input_methods_change_nothing_read_only() {
    let (mut s, shape) = deck();
    lock(&mut s, true);
    let before = snapshot(&s);
    // A selected shape: typing would start editing it, Delete would delete it, arrows nudge it.
    assert_eq!(s.type_text("x").unwrap()["handled"], false);
    assert_eq!(s.compose_text("â", None).unwrap()["handled"], false);
    assert_eq!(s.commit_text("â").unwrap()["handled"], false);
    assert!(read_only_refusal(s.key("Delete", Mods::default())));
    s.key("ArrowLeft", Mods::default()).unwrap();
    assert!(read_only_refusal(s.key("d", Mods { cmd: true, ..Default::default() })), "Ctrl+D duplicates");
    unchanged(&s, &before, "keys on a selected shape");
    // No caret to type at: Enter, F2 and `text.edit` don't enter the text.
    assert!(read_only_refusal(s.execute("text.edit", &json!({"id": shape}))));
    assert!(read_only_refusal(s.key("Enter", Mods::default())));
    assert!(read_only_refusal(s.key("F2", Mods::default())));
    assert!(s.active().unwrap().selection.text.is_none());
    assert_eq!(s.delete_surrounding(1, 0).unwrap()["handled"], false);
    assert!(read_only_refusal(s.key("Backspace", Mods::default())));
    assert!(read_only_refusal(s.key("b", Mods { cmd: true, ..Default::default() })), "Ctrl+B formats");
    assert!(read_only_refusal(s.execute("text.compose", &json!({"text": "a"}))));
    assert!(s.active().unwrap().composition.is_none());
    unchanged(&s, &before, "keys and text commands");
}

fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
    PointerEvent { kind, x, y, mods: Mods::default(), tol: 3.0 }
}

#[test]
fn canvas_drags_select_but_never_move_size_or_rotate_read_only() {
    let (mut s, shape) = deck();
    lock(&mut s, true);
    let before = snapshot(&s);
    // Move: drag the middle.
    s.pointer(ev(PointerKind::Down, 150.0, 140.0)).unwrap();
    s.pointer(ev(PointerKind::Drag, 220.0, 200.0)).unwrap();
    s.pointer(ev(PointerKind::Up, 220.0, 200.0)).unwrap();
    // Resize: the SE handle is only part of the shape.
    s.execute("edit.select", &json!({"ids": [shape]})).unwrap();
    assert!(matches!(s.hit_test(geom::Point::new(200.0, 180.0), 3.0), Some(tools::Hit::Shape { .. })));
    s.pointer(ev(PointerKind::Down, 200.0, 180.0)).unwrap();
    s.pointer(ev(PointerKind::Drag, 260.0, 230.0)).unwrap();
    s.pointer(ev(PointerKind::Up, 260.0, 230.0)).unwrap();
    // Alt-drag would duplicate.
    let alt = Mods { alt: true, ..Default::default() };
    s.pointer(PointerEvent { mods: alt, ..ev(PointerKind::Down, 150.0, 140.0) }).unwrap();
    s.pointer(PointerEvent { mods: alt, ..ev(PointerKind::Drag, 250.0, 240.0) }).unwrap();
    s.pointer(PointerEvent { mods: alt, ..ev(PointerKind::Up, 250.0, 240.0) }).unwrap();
    unchanged(&s, &before, "a canvas drag");
    assert_eq!(s.active().unwrap().selection.shapes, vec![model::ShapeId(shape as u32)], "clicking still selects");
    // Drawing tools fall back to selecting.
    s.set_tool(ToolKind::Shape { preset: "rect".into() });
    assert_eq!(s.tool.kind, ToolKind::Select);
    s.pointer(ev(PointerKind::Down, 400.0, 300.0)).unwrap();
    s.pointer(ev(PointerKind::Drag, 450.0, 350.0)).unwrap();
    s.pointer(ev(PointerKind::Up, 450.0, 350.0)).unwrap();
    unchanged(&s, &before, "drawing");
}

#[test]
fn a_click_or_double_click_on_text_selects_the_shape_without_a_caret_read_only() {
    let (mut s, shape) = deck();
    s.execute("text.set", &json!({"id": shape, "text": "Northern"})).unwrap();
    s.execute("edit.deselect", &json!({})).unwrap();
    lock(&mut s, true);
    let before = snapshot(&s);
    assert!(matches!(s.hit_test(geom::Point::new(150.0, 140.0), 3.0), Some(tools::Hit::Shape { text: false, .. })));
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
        s.pointer(ev(kind, 150.0, 140.0)).unwrap();
        let sel = &s.active().unwrap().selection;
        assert_eq!(sel.shapes, vec![model::ShapeId(shape as u32)], "{kind:?} selects");
        assert!(sel.text.is_none(), "{kind:?} placed a caret");
    }
    assert!(s.ui_requests.is_empty(), "a double-click opens no Format pane");
    unchanged(&s, &before, "clicks");
}

#[test]
fn going_read_only_and_back_keeps_the_document_and_its_undo_history() {
    let (mut s, shape) = deck();
    s.execute("shape.move", &json!({"dx": 10, "dy": 0, "ids": [shape]})).unwrap();
    s.execute("slide.new", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let uid = s.active().unwrap().uid;
    let before = snapshot(&s);
    let revision = s.active().unwrap().revision;
    assert!(s.set_access(uid, Access::read_only(true)));
    unchanged(&s, &before, "going read-only");
    assert!(s.set_access(uid, Access::default()));
    unchanged(&s, &before, "going back to editing");
    assert_eq!(s.active().unwrap().revision, revision, "nothing to save");
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.undo.len(), before.1 - 1, "the history from before is still there");
    assert!(!s.set_access(u64::MAX, Access::default()), "an unknown presentation");
}

#[test]
fn going_read_only_ends_a_composition_and_a_drag_as_undo_steps() {
    let (mut s, shape) = deck();
    s.execute("text.edit", &json!({"id": shape})).unwrap();
    s.compose_text("â", None).unwrap();
    let uid = s.active().unwrap().uid;
    let undo = s.active().unwrap().history.undo.len();
    lock(&mut s, true);
    let d = s.active().unwrap();
    assert!(d.composition.is_none());
    assert!(d.selection.text.is_none(), "no caret once read-only");
    assert_eq!(d.history.undo.len(), undo + 1, "the composed text is kept as one step");
    assert!(s.set_access(uid, Access::default()));
    s.execute("edit.select", &json!({"ids": [shape]})).unwrap();
    s.pointer(ev(PointerKind::Down, 150.0, 140.0)).unwrap();
    s.pointer(ev(PointerKind::Drag, 200.0, 140.0)).unwrap();
    let undo = s.active().unwrap().history.undo.len();
    lock(&mut s, true);
    assert!(s.active().unwrap().interaction.is_none());
    assert_eq!(s.active().unwrap().history.undo.len(), undo + 1, "the move so far is one step");
    assert!(!s.tool.dragging());
    let before = snapshot(&s);
    s.pointer(ev(PointerKind::Drag, 260.0, 140.0)).unwrap();
    s.pointer(ev(PointerKind::Up, 260.0, 140.0)).unwrap();
    unchanged(&s, &before, "the rest of the drag");
}

#[test]
fn viewing_keeps_navigation_find_selection_and_the_show() {
    let (mut s, shape) = deck();
    s.execute("slide.new", &json!({})).unwrap();
    s.execute("slide.go", &json!({"index": 0})).unwrap();
    lock(&mut s, true);
    let before = snapshot(&s);
    s.execute("slide.next", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().selection.slide, 1);
    s.key("PageUp", Mods::default()).unwrap();
    assert_eq!(s.active().unwrap().selection.slide, 0);
    s.execute("edit.select", &json!({"ids": [shape]})).unwrap();
    s.execute("edit.selectAll", &json!({})).unwrap();
    s.execute("edit.find", &json!({"text": "Northern"})).unwrap();
    s.execute("show.fromStart", &json!({})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    unchanged(&s, &before, "viewing");
}

#[test]
fn a_hidden_object_picked_from_the_selection_pane_is_selected_not_shown_read_only() {
    let (mut s, shape) = deck();
    s.execute("shape.visible", &json!({"visible": false, "ids": [shape]})).unwrap();
    s.execute("edit.deselect", &json!({})).unwrap();
    lock(&mut s, true);
    let before = snapshot(&s);
    s.execute("selection.select", &json!({"id": shape})).unwrap();
    assert_eq!(s.active().unwrap().selection.shapes, vec![model::ShapeId(shape as u32)]);
    assert!(s.active().unwrap().current_slide().unwrap().shapes.iter().any(|x| u64::from(x.id.0) == shape && x.hidden), "still hidden");
    unchanged(&s, &before, "picking a hidden object");
}

#[test]
fn no_download_refuses_copy_save_as_and_export_even_when_editable() {
    let (mut s, shape) = deck();
    let uid = s.active().unwrap().uid;
    for access in [Access::read_only(false), Access { editable: true, can_download: false, can_save: true }] {
        assert!(s.set_access(uid, access));
        for id in ["edit.copy", "edit.cut", "file.saveAs", "file.export", "file.saveTemplate", "file.saveBytes", "file.render"] {
            let r = s.execute(id, &json!({"ids": [shape], "path": "x.pptx", "format": "pptx"}));
            assert!(matches!(r, Err(EngineError::Disabled(_, ref why)) if why == access::NO_DOWNLOAD), "{id}: {r:?}");
        }
    }
    assert!(s.set_access(uid, Access::read_only(true)));
    s.execute("edit.copy", &json!({"ids": [shape]})).unwrap();
    assert!(!s.clipboard.shapes.is_empty(), "copying works when downloading is allowed");
}

#[test]
fn locked_edits_still_save_and_new_presentations_start_locked() {
    let (mut s, shape) = deck();
    let uid = s.active().unwrap().uid;
    assert!(s.set_access(uid, Access::edits_locked()));
    assert!(s.active().unwrap().is_dirty(), "the shape inserted before the lock is still unsaved");
    for (id, params) in one_per_family(shape) {
        let before = snapshot(&s);
        assert!(read_only_refusal(s.execute(id, &params)), "{id} wasn't refused");
        unchanged(&s, &before, id);
    }
    let dir = std::env::temp_dir().join(format!("deckcraft-edits-locked-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    s.execute("file.save", &json!({"path": path("saved.pptx")})).unwrap();
    assert!(!s.active().unwrap().is_dirty(), "Save keeps what was changed before the lock");
    s.execute("file.saveAs", &json!({"path": path("copy.pptx")})).unwrap();
    s.execute("file.export", &json!({"path": path("slide.png"), "format": "png"})).unwrap();
    s.execute("edit.copy", &json!({"ids": [shape]})).unwrap();
    assert!(dir.join("saved.pptx").is_file() && dir.join("copy.pptx").is_file());
    let _ = std::fs::remove_dir_all(&dir);
    // The host view modes keep refusing Save.
    assert!(s.set_access(uid, Access::read_only(true)));
    assert!(read_only_refusal(s.execute("file.save", &json!({"path": path("view.pptx")}))));

    s.set_new_document_access(Access::edits_locked());
    s.execute("file.new", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().access, Access::edits_locked(), "a new presentation is never editable for a moment");
    assert!(read_only_refusal(s.execute("slide.new", &json!({}))));
    s.set_new_document_access(Access::default());
    s.execute("file.new", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().access, Access::default());
}

#[test]
fn the_hosts_account_names_new_comments() {
    let mut s = Session::with_new();
    s.prefs.author = "Local Name".into();
    s.execute("comment.add", &json!({"text": "a"})).unwrap();
    s.host_author = Some("Nguyễn Văn An".into());
    s.execute("comment.add", &json!({"text": "b"})).unwrap();
    s.execute("comment.reply", &json!({"index": 1, "text": "c"})).unwrap();
    s.host_author = None;
    s.execute("comment.add", &json!({"text": "d"})).unwrap();
    let comments = &s.active().unwrap().current_slide().unwrap().comments;
    let authors: Vec<&str> = comments.iter().map(|c| c.author.as_str()).collect();
    assert_eq!(authors, ["Local Name", "Nguyễn Văn An", "Local Name"]);
    assert_eq!(comments[1].initials, "NVA");
    assert_eq!(comments[1].replies[0].author, "Nguyễn Văn An");
    assert_eq!(s.prefs.author, "Local Name", "the preference is untouched");
}
