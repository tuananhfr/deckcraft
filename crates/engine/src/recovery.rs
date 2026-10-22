//! Crash recovery (PowerPoint's AutoRecover): unsaved changes of every open presentation are
//! written to a recovery folder (`file.recovery.save`, called on a timer by the app and after a
//! command panics); saving or closing a presentation removes its entry; after a crash
//! `file.recovery.open` reopens what was left there as unsaved presentations that remember where
//! they were saved.
//!
//! An entry is `<uid>.deckcraft` (the presentation) plus `<uid>.json` (`{"path", "title",
//! "saved"}`: the original file, the title and when it was written, Unix seconds).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::{DocState, EngineError, Result, Session};

/// The platform's per-user recovery folder.
pub fn default_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Application Support/DeckCraft/Recovery"));
    }
    if cfg!(target_os = "windows") {
        return std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("DeckCraft").join("Recovery"));
    }
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| home.map(|h| h.join(".local/share"))).map(|d| d.join("deckcraft/recovery"))
}

fn files(dir: &Path, uid: u64) -> (PathBuf, PathBuf) {
    (dir.join(format!("{uid}.deckcraft")), dir.join(format!("{uid}.json")))
}

fn now_secs() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
}

/// Remove a presentation's recovery entry (it was saved or closed).
pub fn discard(dir: &Path, uid: u64) {
    let entries = list(dir);
    let source = entries.iter().find(|(id, _)| *id == uid).and_then(|(_, m)| m.get("source")).and_then(Value::as_u64).unwrap_or(uid);
    for (id, m) in entries {
        if id == uid || id == source || m.get("source").and_then(Value::as_u64) == Some(source) {
            remove_entry(dir, id);
        }
    }
}

fn remove_entry(dir: &Path, uid: u64) {
    let (d, m) = files(dir, uid);
    let _ = std::fs::remove_file(d);
    let _ = std::fs::remove_file(m);
}

pub(crate) fn discard_document(dir: &Path, d: &DocState) {
    discard(dir, d.uid);
    if let Some(source) = d.recovery_source {
        discard(dir, source);
    }
}

fn durable_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("{}.tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(tmp, path)?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// Write entries for the unsaved presentations (and drop those of saved ones). Returns how many
/// were written.
pub fn save(s: &Session, dir: &Path) -> Result<usize> {
    std::fs::create_dir_all(dir).map_err(|e| EngineError::Other(format!("{}: {e}", dir.display())))?;
    let mut n = 0;
    for d in s.documents() {
        if !d.is_dirty() {
            discard_document(dir, d);
            continue;
        }
        let (doc, meta) = files(dir, d.uid);
        let bytes = deckcraft_format::save(&d.doc).map_err(|e| EngineError::Other(e.to_string()))?;
        let source = d.recovery_source.unwrap_or(d.uid);
        let m = json!({"path": d.path, "title": d.title(), "saved": now_secs(), "source": source});
        // Publish lineage first so a crash between the two renames cannot create duplicates.
        durable_write(&meta, m.to_string().as_bytes()).map_err(|e| EngineError::Other(format!("{}: {e}", meta.display())))?;
        durable_write(&doc, &bytes).map_err(|e| EngineError::Other(format!("{}: {e}", doc.display())))?;
        for (uid, meta) in list(dir) {
            if uid != d.uid && (uid == source || meta.get("source").and_then(Value::as_u64) == Some(source)) {
                remove_entry(dir, uid);
            }
        }
        n += 1;
    }
    Ok(n)
}

/// Entries in `dir`: (uid, metadata).
pub fn list(dir: &Path) -> Vec<(u64, Value)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("deckcraft") {
            continue;
        }
        let Some(uid) = p.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u64>().ok()) else { continue };
        // Metadata is a file on disk: anything but an object (corrupt, hand-edited) counts as none.
        let meta = std::fs::read_to_string(p.with_extension("json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .filter(Value::is_object)
            .unwrap_or(json!({}));
        out.push((uid, meta));
    }
    out.sort_by_key(|e| e.0);
    out
}

/// Reopen the newest readable entry per presentation, retaining it until a durable checkpoint
/// or explicit save/discard. Unreadable entries are left in place, and so are those past [`crate::MAX_DOCUMENTS`]
/// (the error says so; the ones opened stay open). Returns the opened documents' indices.
pub fn open(s: &mut Session, dir: &Path) -> Result<Vec<usize>> {
    let mut opened = Vec::new();
    let mut entries = list(dir);
    if let Some(next) = entries.iter().map(|(uid, _)| *uid).max().and_then(|uid| uid.checked_add(1)) {
        crate::NEXT_UID.fetch_max(next, std::sync::atomic::Ordering::Relaxed);
    } else if !entries.is_empty() {
        return Err(EngineError::Other("recovery identifier exhausted".into()));
    }
    entries.sort_by_key(|(uid, meta)| std::cmp::Reverse((meta.get("saved").and_then(Value::as_u64).unwrap_or(0), *uid)));
    let mut recovered = std::collections::HashSet::new();
    for (uid, meta) in &entries {
        let uid = *uid;
        let source = meta.get("source").and_then(Value::as_u64).unwrap_or(uid);
        if recovered.contains(&source) || s.documents().iter().any(|d| d.recovery_source == Some(source)) {
            continue;
        }
        let (doc, _) = files(dir, uid);
        let Ok(bytes) = std::fs::read(&doc) else { continue };
        let Ok(mut d) = deckcraft_format::load(&bytes) else { continue };
        s.ensure_room()?;
        deckcraft_format::repair(&mut d);
        let path = meta.get("path").and_then(Value::as_str).map(str::to_string);
        let title = meta.get("title").and_then(Value::as_str).filter(|t| !t.is_empty()).unwrap_or("Presentation");
        let mut st = DocState::new(d, path, format!("{title} (Recovered)"));
        // Unsaved: the copy on disk (if any) is older than what was recovered.
        st.recovery_source = Some(source);
        st.saved_doc = std::sync::Arc::new((*st.doc).clone());
        st.revision += 1;
        opened.push(s.add_document(st)?);
        recovered.insert(source);
        for (older, m) in &entries {
            if *older != uid && (*older == source || m.get("source").and_then(Value::as_u64) == Some(source)) {
                let older_key = (m.get("saved").and_then(Value::as_u64).unwrap_or(0), *older);
                let loaded_key = (meta.get("saved").and_then(Value::as_u64).unwrap_or(0), uid);
                if older_key < loaded_key {
                    remove_entry(dir, *older);
                }
            }
        }
    }
    Ok(opened)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("deckcraft-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn recovers_unsaved_presentations() {
        let dir = temp("recovery");
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("file.new", &json!({})).unwrap();
        // Only the edited presentation is written.
        s.execute("slide.new", &json!({"layout": "blank"})).unwrap();
        assert_eq!(save(&s, &dir).unwrap(), 1);
        assert_eq!(list(&dir).len(), 1);
        // "Crash": a new session reopens it, unsaved, with its slides.
        let mut s2 = Session::new();
        assert_eq!(open(&mut s2, &dir).unwrap().len(), 1);
        let st = s2.doc().unwrap();
        assert!(st.is_dirty());
        assert_eq!(st.doc.slides.len(), 2);
        assert!(st.title().ends_with("(Recovered)"));
        assert_eq!(list(&dir).len(), 1, "source stays until a durable replacement exists");
        // Saving drops the entry.
        s.recovery_dir = Some(dir.clone());
        assert_eq!(save(&s, &dir).unwrap(), 1);
        let path = dir.join("saved.deckcraft");
        s.execute("file.saveAs", &json!({"path": path.to_string_lossy()})).unwrap();
        assert!(list(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn checkpoint(name: &str) -> (PathBuf, Vec<u8>) {
        let dir = temp(name);
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        s.execute("slide.new", &json!({"layout": "blank"})).unwrap();
        save(&s, &dir).unwrap();
        let bytes = deckcraft_format::save(&s.doc().unwrap().doc).unwrap();
        (dir, bytes)
    }

    #[test]
    fn recovered_content_survives_death_before_timer() {
        let (dir, bytes) = checkpoint("death-before-timer");
        let mut s = Session::new();
        assert_eq!(open(&mut s, &dir).unwrap().len(), 1);
        drop(s);
        let mut s = Session::new();
        assert_eq!(open(&mut s, &dir).unwrap().len(), 1);
        assert_eq!(deckcraft_format::save(&s.doc().unwrap().doc).unwrap(), bytes);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn recovered_checkpoint_replaces_source() {
        let (dir, _) = checkpoint("checkpoint-replaces-source");
        let old = list(&dir).first().unwrap().0;
        let mut s = Session::new();
        open(&mut s, &dir).unwrap();
        assert!(files(&dir, old).0.exists());
        save(&s, &dir).unwrap();
        assert!(!files(&dir, old).0.exists());
        assert_eq!(list(&dir).len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saving_recovered_document_removes_records() {
        let (dir, _) = checkpoint("saved-recovery");
        let mut s = Session::new();
        s.recovery_dir = Some(dir.clone());
        open(&mut s, &dir).unwrap();
        assert_eq!(list(&dir).len(), 1);
        let path = dir.with_extension("deckcraft");
        s.execute("file.saveAs", &json!({"path": path})).unwrap();
        assert!(list(&dir).is_empty());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn newer_corrupt_record_falls_back_without_duplicates() {
        let (dir, bytes) = checkpoint("corrupt-newer");
        let old = list(&dir).first().unwrap().0;
        let newer = old + 10000;
        let (doc, meta) = files(&dir, newer);
        std::fs::write(&doc, b"broken").unwrap();
        std::fs::write(&meta, json!({"source": old, "saved": u64::MAX}).to_string()).unwrap();
        let mut s = Session::new();
        assert_eq!(open(&mut s, &dir).unwrap().len(), 1);
        assert_eq!(deckcraft_format::save(&s.doc().unwrap().doc).unwrap(), bytes);
        assert!(files(&dir, old).0.exists());
        s.execute("slide.new", &json!({"layout": "blank"})).unwrap();
        let newer_bytes = deckcraft_format::save(&s.doc().unwrap().doc).unwrap();
        std::fs::write(&doc, &newer_bytes).unwrap();
        drop(s);
        let mut s = Session::new();
        assert_eq!(open(&mut s, &dir).unwrap().len(), 1);
        assert!(!files(&dir, old).0.exists());
        assert!(doc.exists());
        assert_eq!(deckcraft_format::save(&s.doc().unwrap().doc).unwrap(), newer_bytes);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_checkpoint_keeps_source() {
        let (dir, bytes) = checkpoint("failed-checkpoint");
        let old = list(&dir).first().unwrap().0;
        let mut s = Session::new();
        open(&mut s, &dir).unwrap();
        let doc = files(&dir, s.doc().unwrap().uid).0;
        std::fs::create_dir(doc.with_extension("deckcraft.tmp")).unwrap();
        assert!(save(&s, &dir).is_err());
        assert_eq!(std::fs::read(files(&dir, old).0).unwrap(), bytes);
        drop(s);
        let mut s = Session::new();
        assert_eq!(open(&mut s, &dir).unwrap().len(), 1);
        s.recovery_dir = Some(dir.clone());
        s.execute("file.close", &json!({})).unwrap();
        assert!(list(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_entries_are_skipped() {
        let dir = temp("recovery-corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("7.deckcraft"), b"not a presentation").unwrap();
        std::fs::write(dir.join("7.json"), b"[1, 2]").unwrap();
        let mut s = Session::new();
        assert!(open(&mut s, &dir).unwrap().is_empty());
        s.recovery_dir = Some(dir.clone());
        let v = s.execute("file.recovery.list", &json!({})).unwrap();
        assert_eq!(v.as_array().map(Vec::len), Some(1));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
