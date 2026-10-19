//! Draft autosave for the office site's draft store: when drafts are put and removed, the
//! hidden-page flush, and reopening a draft the store hands back.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::sync::Arc;

use deckcraft_engine::Session;
use serde_json::json;

use crate::drafts::{CHECKPOINT_MS, DocInput, DraftStore, Drafts, FORMAT, IDLE_MS};
use crate::{Services, SlideApp};

#[derive(Default)]
struct Store {
    puts: Vec<(String, String, String, Vec<u8>)>,
    removes: Vec<String>,
}

impl DraftStore for Store {
    fn put(&mut self, key: &str, name: &str, format: &str, bytes: &[u8]) {
        self.puts.push((key.into(), name.into(), format.into(), bytes.to_vec()));
    }
    fn remove(&mut self, key: &str) {
        self.removes.push(key.into());
    }
}

fn app() -> SlideApp {
    let mut app = SlideApp::new(Session::new(), Services::default());
    app.session.execute("file.new", &json!({})).unwrap();
    app
}

fn edit(app: &mut SlideApp) {
    app.session.execute("slide.new", &json!({"layout": "blank"})).unwrap();
}

fn uid(app: &SlideApp) -> u64 {
    app.session.active().unwrap().uid
}

const CLOCK: &dyn Fn() -> f64 = &|| 0.0;

/// An edit is put once the deck has been quiet for the idle time, as `.deckcraft` bytes under
/// `doc-<uid>` and the name Save would suggest; nothing is put while it is unchanged.
#[test]
fn puts_after_idle() {
    let mut app = app();
    let mut d = Drafts::default();
    let mut store = Store::default();
    assert!(!d.tick(&app, 0.0, CLOCK, &mut store), "a clean deck waits for nothing");
    edit(&mut app);
    assert!(d.tick(&app, 1000.0, CLOCK, &mut store), "waiting for the idle moment");
    assert!(d.tick(&app, 1000.0 + IDLE_MS - 1.0, CLOCK, &mut store));
    assert!(store.puts.is_empty());
    assert!(!d.tick(&app, 1000.0 + IDLE_MS, CLOCK, &mut store));
    assert_eq!(store.puts.len(), 1);
    let (key, name, format, bytes) = &store.puts[0];
    assert_eq!(key, &format!("doc-{}", uid(&app)));
    assert_eq!(name, "Presentation1.pptx");
    assert_eq!(format, FORMAT);
    let back = deckcraft_engine::cmd::file::open_presentation("x.deckcraft", bytes).unwrap();
    assert_eq!(back.slides.len(), app.session.active().unwrap().doc.slides.len());
    assert!(!d.tick(&app, 99_000.0, CLOCK, &mut store));
    assert_eq!(store.puts.len(), 1, "an unchanged deck is not put again");
}

/// Editing that never pauses is still put at the checkpoint.
#[test]
fn checkpoint_during_continuous_editing() {
    let mut app = app();
    let mut d = Drafts::default();
    let mut store = Store::default();
    d.tick(&app, 0.0, CLOCK, &mut store);
    let mut t = 0.0;
    while store.puts.is_empty() && t < 2.0 * CHECKPOINT_MS {
        t += 1000.0;
        edit(&mut app);
        d.tick(&app, t, CLOCK, &mut store);
    }
    assert_eq!(store.puts.len(), 1);
    assert!((CHECKPOINT_MS..CHECKPOINT_MS + 1001.0).contains(&t), "put at {t}");
}

/// A deck that took long to serialize is put at most once per ten serializations.
#[test]
fn slow_serialization_spaces_puts() {
    let mut app = app();
    let mut d = Drafts::default();
    let mut store = Store::default();
    // Each serialization appears to take 1 s.
    let t = Cell::new(0.0);
    let clock = || {
        t.set(t.get() + 1000.0);
        t.get()
    };
    edit(&mut app);
    d.tick(&app, 0.0, &clock, &mut store);
    d.tick(&app, IDLE_MS, &clock, &mut store);
    assert_eq!(store.puts.len(), 1);
    edit(&mut app);
    let first = IDLE_MS + 1.0;
    d.tick(&app, first, &clock, &mut store);
    assert!(d.tick(&app, first + IDLE_MS, &clock, &mut store), "idle, but too soon after a slow put");
    assert_eq!(store.puts.len(), 1);
    d.tick(&app, IDLE_MS + 10_000.0, &clock, &mut store);
    assert_eq!(store.puts.len(), 2);
}

/// Saved, undone back to saved, or closed without saving: the draft is removed, once.
#[test]
fn removes_when_no_longer_unsaved() {
    let mut app = app();
    app.session.execute("file.new", &json!({})).unwrap();
    let mut d = Drafts::default();
    let mut store = Store::default();
    // Two decks drafted.
    edit(&mut app);
    app.session.set_active(0);
    edit(&mut app);
    d.tick(&app, 0.0, CLOCK, &mut store);
    d.tick(&app, IDLE_MS, CLOCK, &mut store);
    assert_eq!(store.puts.len(), 2);
    let keys: Vec<String> = app.session.documents().iter().map(|s| format!("doc-{}", s.uid)).collect();
    // The first is saved.
    let st = app.session.active_mut().unwrap();
    st.saved_doc = st.doc.clone();
    d.tick(&app, IDLE_MS + 1.0, CLOCK, &mut store);
    assert_eq!(store.removes, vec![keys[0].clone()]);
    d.tick(&app, IDLE_MS + 2.0, CLOCK, &mut store);
    assert_eq!(store.removes.len(), 1, "removed once");
    // The second is closed with Don't Save.
    app.session.close_document(1);
    d.tick(&app, IDLE_MS + 3.0, CLOCK, &mut store);
    assert_eq!(store.removes, keys);
    // Undo back to the saved state also removes it.
    edit(&mut app);
    d.tick(&app, 10_000.0, CLOCK, &mut store);
    d.tick(&app, 10_000.0 + IDLE_MS, CLOCK, &mut store);
    assert_eq!(store.puts.len(), 3);
    app.session.execute("edit.undo", &json!({})).unwrap();
    assert!(!app.session.active().unwrap().is_dirty());
    d.tick(&app, 20_000.0, CLOCK, &mut store);
    assert_eq!(store.removes.len(), 3);
    assert_eq!(store.removes[2], keys[0]);
}

/// Nothing is put while an input method composes (the deck holds its preedit); the commit is put.
#[test]
fn waits_while_composing() {
    let doc = Arc::new(deckcraft_engine::cmd::file::open_presentation("x.txt", b"Title").unwrap());
    let input = |doc: &Arc<_>, composing| DocInput { uid: 7, doc: Arc::clone(doc), name: "A.pptx".into(), unsaved: true, composing };
    let mut d = Drafts::default();
    let plan = d.plan(&[input(&doc, true)], 0.0);
    assert!(plan.put.is_empty() && plan.waiting);
    let plan = d.plan(&[input(&doc, true)], CHECKPOINT_MS * 2.0);
    assert!(plan.put.is_empty() && plan.waiting, "not even at the checkpoint");
    let committed = Arc::new((*doc).clone());
    let plan = d.plan(&[input(&committed, false)], CHECKPOINT_MS * 2.0);
    assert_eq!(plan.put, vec![7], "the commit is past the checkpoint");
}

/// Hiding the page puts every changed, unsaved deck at once; the next frame doesn't put it again.
#[test]
fn flush_ignores_the_idle_wait() {
    let mut app = app();
    let mut d = Drafts::default();
    let mut store = Store::default();
    edit(&mut app);
    d.tick(&app, 0.0, CLOCK, &mut store);
    d.flush(CLOCK, &mut store);
    assert_eq!(store.puts.len(), 1);
    d.flush(CLOCK, &mut store);
    assert_eq!(store.puts.len(), 1, "unchanged since");
    assert!(!d.tick(&app, IDLE_MS * 4.0, CLOCK, &mut store));
    assert_eq!(store.puts.len(), 1);
}

/// A draft the store hands back opens unsaved under the name it was kept as, keeps the store's
/// key, and isn't put again until it changes.
#[test]
fn restored_draft_keeps_its_key() {
    let mut source = app();
    edit(&mut source);
    let bytes = deckcraft_engine::cmd::file::save_bytes(&source.session.active().unwrap().doc, FORMAT).unwrap();
    let mut app = app();
    let mut d = Drafts::default();
    let mut store = Store::default();
    d.restore(&mut app, "k-1", "Northern Lights.pptx", &bytes, 0.0).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(app.session.documents().len(), 2, "opened alongside");
    assert!(st.is_dirty());
    assert!(st.path.is_none());
    assert_eq!(st.title(), "Northern Lights");
    assert_eq!(st.doc.slides.len(), 2);
    d.tick(&app, IDLE_MS * 4.0, CLOCK, &mut store);
    assert!(store.puts.is_empty(), "already drafted");
    edit(&mut app);
    d.tick(&app, IDLE_MS * 5.0, CLOCK, &mut store);
    d.tick(&app, IDLE_MS * 6.0, CLOCK, &mut store);
    assert_eq!(store.puts.len(), 1);
    assert_eq!(store.puts[0].0, "k-1");
    assert_eq!(store.puts[0].1, "Northern Lights.pptx");
    let st = app.session.active_mut().unwrap();
    st.saved_doc = st.doc.clone();
    d.tick(&app, IDLE_MS * 7.0, CLOCK, &mut store);
    assert_eq!(store.removes, vec!["k-1".to_string()]);
    assert!(d.restore(&mut app, "k-2", "Junk.pptx", b"not a deck", 0.0).is_err());
}
