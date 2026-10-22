//! Several open presentations: the open-documents limit on every path that adds one, switching,
//! and closing one that isn't active.

use serde_json::json;

use crate::*;

fn full() -> Session {
    let mut s = Session::new();
    for _ in 0..MAX_DOCUMENTS {
        s.execute("file.new", &json!({})).unwrap();
    }
    s
}

fn titles(s: &Session) -> Vec<String> {
    s.documents().iter().map(DocState::title).collect()
}

fn refused(r: Result<serde_json::Value>) -> bool {
    matches!(r, Err(EngineError::TooManyDocuments))
}

#[test]
fn new_is_refused_past_the_limit() {
    let mut s = full();
    assert_eq!(s.documents().len(), 10);
    s.set_active(3);
    assert!(refused(s.execute("file.new", &json!({}))));
    assert_eq!(s.documents().len(), 10);
    assert_eq!(s.active_index(), Some(3), "the refusal leaves the active presentation");
    s.close_document(0);
    s.execute("file.new", &json!({})).unwrap();
    assert_eq!(s.documents().last().unwrap().title(), "Presentation11", "no untitled number was used by the refusal");
    assert_eq!(EngineError::TooManyDocuments.to_string(), "You can have up to 10 documents open. Close one to open another.");
}

#[test]
fn opening_bytes_or_a_file_is_refused_past_the_limit() {
    let bytes = cmd::file::save_bytes(&model::defaults::new_presentation(None), "pptx").unwrap();
    let mut s = full();
    assert!(refused(s.execute("file.openBytes", &json!({"name": "a.pptx", "data": cmd::base64_encode(&bytes)}))));
    let path = std::env::temp_dir().join(format!("deckcraft-limit-{}.pptx", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    assert!(refused(s.execute("file.open", &json!({"path": path.to_string_lossy()}))));
    let _ = std::fs::remove_file(&path);
    assert!(refused(sample::open_sample(&mut s).map(|_| json!(null))));
    assert!(matches!(s.add_document(DocState::new(model::defaults::new_presentation(None), None, "x".into())), Err(EngineError::TooManyDocuments)));
    assert_eq!(s.documents().len(), 10);
}

#[test]
fn recovery_opens_up_to_the_limit_and_keeps_the_rest() {
    let dir = std::env::temp_dir().join(format!("deckcraft-limit-recovery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut a = Session::new();
    a.recovery_dir = Some(dir.clone());
    for _ in 0..3 {
        a.execute("file.new", &json!({})).unwrap();
        a.execute("slide.new", &json!({})).unwrap();
    }
    a.execute("file.recovery.save", &json!({})).unwrap();
    let mut b = Session::new();
    b.recovery_dir = Some(dir.clone());
    for _ in 0..MAX_DOCUMENTS - 2 {
        b.execute("file.new", &json!({})).unwrap();
    }
    assert!(refused(b.execute("file.recovery.open", &json!({}))));
    assert_eq!(b.documents().len(), 10);
    // Opened entries stay on disk until a durable checkpoint replaces them (DECK-05).
    let open_sources: Vec<_> = b.documents().iter().filter_map(|d| d.recovery_source).collect();
    assert_eq!(open_sources.len(), 2);
    let left: Vec<_> = recovery::list(&dir).into_iter().filter(|(uid, _)| !open_sources.contains(uid)).collect();
    assert_eq!(left.len(), 1, "the one that didn't fit stays recoverable");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closing_another_presentation_keeps_the_active_one() {
    let mut s = Session::new();
    for _ in 0..4 {
        s.execute("file.new", &json!({})).unwrap();
    }
    s.execute("window.next", &json!({"index": 2})).unwrap();
    assert_eq!(s.doc().unwrap().title(), "Presentation3");
    s.execute("file.close", &json!({"index": 0})).unwrap();
    assert_eq!(s.doc().unwrap().title(), "Presentation3", "a tab before the active one");
    s.execute("file.close", &json!({"index": 2})).unwrap();
    assert_eq!(s.doc().unwrap().title(), "Presentation3", "a tab after it");
    assert_eq!(titles(&s), ["Presentation2", "Presentation3"]);
    assert!(s.execute("file.close", &json!({"index": 5})).is_err());
    s.execute("file.close", &json!({})).unwrap();
    assert_eq!(titles(&s), ["Presentation2"], "no index closes the active one");
    assert_eq!(s.active_index(), Some(0));
    let journal: Vec<&str> = s.journal.iter().map(|(id, _)| id.as_str()).collect();
    assert!(journal.contains(&"window.next") && journal.contains(&"file.close"), "{journal:?}");
}

fn open_named(s: &mut Session, name: &str) -> Result<serde_json::Value> {
    let bytes = cmd::file::save_bytes(&model::defaults::new_presentation(None), "pptx").unwrap();
    s.execute("file.openBytes", &json!({"name": format!("{name}.pptx"), "data": cmd::base64_encode(&bytes)}))
}

fn names(prefix: &str, n: usize) -> Vec<String> {
    (0..n).map(|i| format!("{prefix}{i}")).collect()
}

#[test]
fn an_untouched_sample_never_takes_a_slot() {
    let mut s = Session::new();
    sample::open_sample(&mut s).unwrap();
    for name in names("f", MAX_DOCUMENTS) {
        open_named(&mut s, &name).unwrap();
    }
    assert_eq!(titles(&s), names("f", MAX_DOCUMENTS), "the first one opened replaced the sample");

    // Beside other presentations it stays until the limit, then gives way.
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    sample::open_sample(&mut s).unwrap();
    for name in names("f", MAX_DOCUMENTS - 2) {
        open_named(&mut s, &name).unwrap();
    }
    assert_eq!(s.documents().len(), MAX_DOCUMENTS);
    assert_eq!(titles(&s)[1], "Northern Lights");
    s.set_active(0);
    open_named(&mut s, "last").unwrap();
    assert_eq!(s.documents().len(), MAX_DOCUMENTS);
    assert!(!titles(&s).contains(&"Northern Lights".to_string()));
    assert_eq!(s.doc().unwrap().title(), "last", "the new one is active");
    assert!(refused(s.execute("file.new", &json!({}))), "{} user presentations fill every slot", MAX_DOCUMENTS);
}

#[test]
fn an_edited_sample_counts() {
    let mut s = Session::new();
    sample::open_sample(&mut s).unwrap();
    s.execute("slide.new", &json!({})).unwrap();
    for name in names("f", MAX_DOCUMENTS - 1) {
        open_named(&mut s, &name).unwrap();
    }
    assert!(refused(open_named(&mut s, "more")));
    assert_eq!(titles(&s)[0], "Northern Lights");

    // An undone edit leaves something to redo, so it still counts.
    let mut s = Session::new();
    sample::open_sample(&mut s).unwrap();
    s.execute("slide.new", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
    open_named(&mut s, "f").unwrap();
    assert_eq!(titles(&s), ["Northern Lights", "f"]);
}

#[test]
fn only_a_placeholder_blank_is_replaced() {
    let mut s = Session::with_new();
    open_named(&mut s, "f").unwrap();
    assert_eq!(titles(&s), ["Presentation1", "f"], "a blank the user made stays");

    let mut s = Session::with_new();
    s.active_mut().unwrap().placeholder = true;
    s.execute("file.new", &json!({})).unwrap();
    assert_eq!(titles(&s), ["Presentation2"], "the start blank gives way to a new one too");
}

fn reopen(s: &mut Session) -> Result<serde_json::Value> {
    s.execute("file.reopenClosed", &json!({}))
}

#[test]
fn a_presentation_closed_without_saving_reopens_with_its_changes_unsaved() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("slide.new", &json!({"layout": "blank"})).unwrap();
    assert!(s.doc().unwrap().is_dirty());
    assert!(reopen(&mut s).is_err(), "nothing closed yet");
    s.execute("file.close", &json!({})).unwrap();
    assert_eq!(titles(&s), ["Presentation1"]);
    assert_eq!(s.closed.last().map(|c| (c.title.as_str(), c.unsaved)), Some(("Presentation2", true)));

    let v = reopen(&mut s).unwrap();
    assert_eq!(v, json!({"document": 1, "title": "Presentation2", "unsaved": true}));
    assert_eq!(titles(&s), ["Presentation1", "Presentation2"]);
    let d = s.doc().unwrap();
    assert_eq!(d.doc.slides.len(), 2, "the slide added before closing came back");
    assert!(d.is_dirty(), "and is still unsaved");
    assert!(s.closed.is_empty());
    assert!(reopen(&mut s).is_err());
}

#[test]
fn a_saved_presentation_reopens_saved_with_its_file() {
    let path = std::env::temp_dir().join(format!("deckcraft-reopen-{}.deckcraft", std::process::id()));
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("slide.new", &json!({})).unwrap();
    s.execute("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
    let title = s.doc().unwrap().title();
    s.execute("file.close", &json!({})).unwrap();
    assert!(s.active().is_none());
    reopen(&mut s).unwrap();
    let d = s.doc().unwrap();
    assert_eq!(d.title(), title);
    assert_eq!(d.path.as_deref(), Some(path.to_string_lossy().as_ref()), "Save writes to the same file again");
    assert!(!d.is_dirty());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn reopening_goes_most_recent_first_and_keeps_to_the_limit() {
    let mut s = full();
    s.close_document(2);
    s.close_document(0);
    assert_eq!(s.closed.len(), 2);
    s.execute("file.new", &json!({})).unwrap();
    s.execute("file.new", &json!({})).unwrap();
    assert!(refused(reopen(&mut s)), "the limit holds");
    assert_eq!(s.closed.last().map(|c| c.title.as_str()), Some("Presentation1"), "the refused one stays to reopen");
    s.close_document(9);
    reopen(&mut s).unwrap();
    assert_eq!(s.doc().unwrap().title(), "Presentation12", "the most recently closed comes back first");
    for _ in 0..3 {
        s.close_document(0);
    }
    for _ in 0..3 {
        reopen(&mut s).unwrap();
    }
    let back: Vec<String> = titles(&s).into_iter().rev().take(3).collect();
    assert_eq!(back, ["Presentation2", "Presentation4", "Presentation5"], "newest closed first");
    assert_eq!(s.closed.last().map(|c| c.title.as_str()), Some("Presentation1"));
}

#[test]
fn an_untouched_sample_is_not_kept_and_ctrl_shift_t_reopens() {
    let mut s = Session::new();
    sample::open_sample(&mut s).unwrap();
    s.execute("file.close", &json!({})).unwrap();
    assert!(s.closed.is_empty(), "closing the untouched sample loses nothing");
    s.execute("file.new", &json!({})).unwrap();
    let title = s.doc().unwrap().title();
    s.execute("file.close", &json!({})).unwrap();
    // With nothing open, the key still reaches the command.
    let v = s.key("T", Mods { shift: true, alt: false, cmd: true }).unwrap();
    assert_eq!(v["command"], "file.reopenClosed", "{v}");
    assert_eq!(titles(&s), [title]);
}
