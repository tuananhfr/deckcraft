//! Ids for live editing: slides and shapes created while editing carry the session's client prefix
//! (two sessions creating at once never collide), and ids stay put across save/open, edits, undo
//! and redo; ids in the file are never rewritten.

use std::collections::HashSet;

use deckcraft_model::walk;
use serde_json::json;

use crate::*;

fn prefix(s: &str) -> ClientPrefix {
    ClientPrefix::parse(s).unwrap()
}

/// A three-slide deck saved as PPTX.
fn deck() -> Vec<u8> {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    s.execute("slide.new", &json!({})).unwrap();
    s.execute("shape.insert", &json!({"preset": "rect", "rect": [10, 10, 50, 50]})).unwrap();
    s.execute("slide.new", &json!({"layout": "blank"})).unwrap();
    cmd::file::save_bytes(&s.active().unwrap().doc, "pptx").unwrap()
}

fn open(bytes: &[u8], client: &str) -> Session {
    let mut s = Session::new();
    s.set_client_prefix(prefix(client));
    s.execute("file.openBytes", &json!({"name": "deck.pptx", "data": cmd::base64_encode(bytes)})).unwrap();
    s
}

/// Each slide's id with its shapes' ids, in order.
fn ids(s: &Session) -> Vec<(u32, Vec<u32>)> {
    s.active()
        .unwrap()
        .doc
        .slides
        .iter()
        .map(|sl| {
            let mut shapes = vec![];
            walk(&sl.shapes, &mut |sh, _| shapes.push(sh.id.0));
            (sl.id.0, shapes)
        })
        .collect()
}

fn all(ids: &[(u32, Vec<u32>)]) -> HashSet<(bool, u32)> {
    ids.iter().flat_map(|(slide, shapes)| std::iter::once((true, *slide)).chain(shapes.iter().map(|s| (false, *s)))).collect()
}

fn reopen(s: &Session, client: &str) -> Session {
    open(&cmd::file::save_bytes(&s.active().unwrap().doc, "pptx").unwrap(), client)
}

fn add_shape(s: &mut Session) -> u32 {
    s.execute("shape.insert", &json!({"preset": "ellipse", "rect": [100, 100, 40, 40]})).unwrap()["id"].as_u64().unwrap() as u32
}

#[test]
fn client_prefixes_are_six_lowercase_hex_digits() {
    assert_eq!(prefix("00ff10").to_string(), "00ff10");
    for bad in ["", "00ff1", "00ff100", "00FF10", "00ff1g", "-0ff10"] {
        assert_eq!(ClientPrefix::parse(bad), None, "{bad:?}");
    }
    let mut s = Session::new();
    assert_eq!(s.client_prefix().to_string().len(), 6, "a session picks its own prefix");
    s.set_client_prefix(prefix("abcdef"));
    assert_eq!(s.client_prefix(), prefix("abcdef"));
}

#[test]
fn created_slides_and_shapes_carry_the_prefix() {
    let mut s = open(&deck(), "0a0b0c");
    let before = all(&ids(&s));
    s.execute("slide.new", &json!({})).unwrap();
    let shape = add_shape(&mut s);
    let created: Vec<(bool, u32)> = all(&ids(&s)).difference(&before).copied().collect();
    assert!(created.len() >= 2, "{created:?}");
    for (_, id) in &created {
        assert_eq!(id >> 6, 0x0a0b0c, "the first ids of a session are its prefix and a counter: {id:#x}");
    }
    assert!(created.contains(&(false, shape)));
}

#[test]
fn two_clients_creating_at_once_never_collide() {
    let bytes = deck();
    let (mut a, mut b) = (open(&bytes, "000001"), open(&bytes, "000002"));
    let file = all(&ids(&a));
    assert_eq!(file, all(&ids(&b)), "both read the file alike");
    // Past the first block of 64 ids too.
    for _ in 0..50 {
        for s in [&mut a, &mut b] {
            s.execute("slide.new", &json!({})).unwrap();
            add_shape(s);
        }
    }
    let new_a: HashSet<u32> = all(&ids(&a)).difference(&file).map(|x| x.1).collect();
    let new_b: HashSet<u32> = all(&ids(&b)).difference(&file).map(|x| x.1).collect();
    assert!(new_a.len() >= 100 && new_b.len() >= 100, "{} {}", new_a.len(), new_b.len());
    assert!(new_a.is_disjoint(&new_b), "{:?}", new_a.intersection(&new_b).collect::<Vec<_>>());
    let file_ids: HashSet<u32> = file.iter().map(|x| x.1).collect();
    assert!(new_a.is_disjoint(&file_ids) && new_b.is_disjoint(&file_ids));
}

#[test]
fn ids_survive_save_open_edits_undo_and_redo() {
    let mut s = open(&deck(), "123abc");
    s.execute("slide.new", &json!({})).unwrap();
    let shape = add_shape(&mut s);
    let edited = ids(&s);
    assert!(edited.iter().any(|(_, shapes)| shapes.contains(&shape)));

    s.execute("slide.move", &json!({"from": 3, "to": 0})).unwrap();
    let moved = ids(&s);
    assert_eq!(all(&moved), all(&edited), "reordering keeps every id");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(ids(&s), edited);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(ids(&s), moved);

    let reopened = reopen(&s, "fedcba");
    assert_eq!(ids(&reopened), moved, "saving as PPTX and opening again keeps slide and shape ids");
    let native = cmd::file::save_bytes(&s.active().unwrap().doc, "deckcraft").unwrap();
    assert_eq!(ids(&open(&native, "fedcba")), moved, "and so does the native format");
}

#[test]
fn an_undone_shape_does_not_lend_its_id_to_the_next_one() {
    let mut s = open(&deck(), "aaaaaa");
    let first = add_shape(&mut s);
    s.execute("edit.undo", &json!({})).unwrap();
    let second = add_shape(&mut s);
    assert_ne!(first, second);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(ids(&s).iter().any(|(_, shapes)| shapes.contains(&second)), "redo brings the same id back");
}

#[test]
fn ids_in_the_file_are_never_rewritten() {
    let mut source = open(&deck(), "bbbbbb");
    source.execute("slide.new", &json!({})).unwrap();
    source.execute("slide.move", &json!({"from": 3, "to": 1})).unwrap();
    let bytes = cmd::file::save_bytes(&source.active().unwrap().doc, "pptx").unwrap();
    let file = ids(&source);
    // Slide ids used to be written as 256 + position and read back from a counter.
    assert_eq!(ids(&open(&bytes, "eeeeee")), file, "opening keeps the file's slide and shape ids");
    let mut s = open(&bytes, "cccccc");
    s.execute("slide.move", &json!({"from": 0, "to": 2})).unwrap();
    s.execute("slide.duplicate", &json!({})).unwrap();
    let after = ids(&s);
    for (slide, shapes) in &file {
        assert!(after.iter().any(|(sl, sh)| sl == slide && sh == shapes), "slide {slide} keeps its id and its shapes' ids: {after:?}");
    }
    let reopened = reopen(&s, "dddddd");
    assert_eq!(ids(&reopened), after);
}
