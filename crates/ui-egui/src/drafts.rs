//! Draft autosave for a host draft store (the ERPCons office site's `window.erpDrafts`): unsaved
//! presentations are handed to the store as they change, so a closed or crashed tab can offer
//! them back. The store owns storage and every message shown about it; this decides when to put
//! and remove, and reopens drafts the store hands back. Platform-independent: the host passes the
//! time in.

use std::collections::BTreeMap;
use std::sync::Arc;

use deckcraft_model::Presentation;

use crate::SlideApp;

/// Quiet time after the last change before a draft is put.
pub const IDLE_MS: f64 = 2_500.0;
/// Longest a change waits while editing goes on without a pause.
pub const CHECKPOINT_MS: f64 = 30_000.0;
/// Puts of a document are at least this many times its last serialization apart, so a large
/// deck doesn't spend the tab's time serializing.
pub const ADAPTIVE_FACTOR: f64 = 10.0;
/// Drafts are native `.deckcraft`: lossless and cheaper to write than PPTX.
pub const FORMAT: &str = "deckcraft";

/// The host's draft store.
pub trait DraftStore {
    fn put(&mut self, key: &str, name: &str, format: &str, bytes: &[u8]);
    fn remove(&mut self, key: &str);
}

/// One open presentation as drafting sees it in a frame.
#[derive(Clone)]
pub struct DocInput {
    pub uid: u64,
    pub doc: Arc<Presentation>,
    /// The name Save suggests on the web (`<title>.pptx`).
    pub name: String,
    /// Dirty, or a save of it hasn't finished.
    pub unsaved: bool,
    /// An input method is composing: the presentation holds its preedit.
    pub composing: bool,
}

/// What to do this frame.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Documents (uid) to put now.
    pub put: Vec<u64>,
    /// Keys to remove from the store.
    pub remove: Vec<String>,
    /// A change waits for its moment: tick again without input.
    pub waiting: bool,
}

struct Entry {
    key: String,
    /// The document as of the last frame; the hidden-page flush works from it.
    current: DocInput,
    /// What was last put (compared by pointer: `revision` also moves with the selection).
    drafted: Option<Arc<Presentation>>,
    /// The store holds a draft under `key`.
    stored: bool,
    last_change: f64,
    first_unput_change: Option<f64>,
    last_put: f64,
    serialize_ms: f64,
}

impl Entry {
    fn new(key: String, current: DocInput, now: f64) -> Self {
        Entry {
            key,
            current,
            drafted: None,
            stored: false,
            last_change: now,
            first_unput_change: None,
            last_put: f64::NEG_INFINITY,
            serialize_ms: 0.0,
        }
    }

    fn changed_since_put(&self) -> bool {
        self.drafted.as_ref().is_none_or(|d| !Arc::ptr_eq(d, &self.current.doc))
    }
}

/// Drafting state for the open presentations, by document uid.
#[derive(Default)]
pub struct Drafts {
    entries: BTreeMap<u64, Entry>,
}

impl Drafts {
    /// Decide what to put and remove at `now` (ms) given this frame's documents.
    pub fn plan(&mut self, docs: &[DocInput], now: f64) -> Plan {
        let mut plan = Plan::default();
        self.entries.retain(|uid, e| {
            let open = docs.iter().any(|d| d.uid == *uid);
            if !open && e.stored {
                plan.remove.push(e.key.clone());
            }
            open
        });
        for d in docs {
            let e = self.entries.entry(d.uid).or_insert_with(|| Entry::new(format!("doc-{}", d.uid), d.clone(), now));
            if !Arc::ptr_eq(&e.current.doc, &d.doc) {
                e.last_change = now;
            }
            e.current = d.clone();
            if !d.unsaved {
                if std::mem::take(&mut e.stored) {
                    plan.remove.push(e.key.clone());
                }
                e.drafted = None;
                e.first_unput_change = None;
                continue;
            }
            if !e.changed_since_put() {
                e.first_unput_change = None;
                continue;
            }
            let first = *e.first_unput_change.get_or_insert(now);
            let due = now - e.last_change >= IDLE_MS || now - first >= CHECKPOINT_MS;
            let spaced = now - e.last_put >= ADAPTIVE_FACTOR * e.serialize_ms;
            if due && spaced && !d.composing {
                plan.put.push(d.uid);
            } else {
                plan.waiting = true;
            }
        }
        plan
    }

    /// Every frame: put and remove what is due. Returns whether a change is waiting.
    pub fn tick(&mut self, app: &SlideApp, now: f64, clock: &dyn Fn() -> f64, store: &mut dyn DraftStore) -> bool {
        let plan = self.plan(&app.draft_inputs(), now);
        for key in &plan.remove {
            store.remove(key);
        }
        for uid in plan.put {
            self.put(uid, now, clock, store);
        }
        plan.waiting
    }

    /// The page is being hidden (maybe for good): put every changed unsaved document now, as of
    /// the last frame, without waiting for the idle moment.
    pub fn flush(&mut self, clock: &dyn Fn() -> f64, store: &mut dyn DraftStore) {
        let due: Vec<u64> =
            self.entries.iter().filter(|(_, e)| e.current.unsaved && !e.current.composing && e.changed_since_put()).map(|(uid, _)| *uid).collect();
        let now = clock();
        for uid in due {
            self.put(uid, now, clock, store);
        }
    }

    fn put(&mut self, uid: u64, now: f64, clock: &dyn Fn() -> f64, store: &mut dyn DraftStore) {
        let Some(e) = self.entries.get_mut(&uid) else { return };
        let start = clock();
        match deckcraft_engine::cmd::file::save_bytes(&e.current.doc, FORMAT) {
            Ok(bytes) => {
                e.serialize_ms = (clock() - start).max(0.0);
                store.put(&e.key, &e.current.name, FORMAT, &bytes);
                e.stored = true;
            }
            // Retried on the next change rather than every frame.
            Err(err) => {
                let name = &e.current.name;
                log::warn!(
                    "couldn't write a draft of a {}: {}",
                    deckcraft_engine::log_safe::log_name(name, None),
                    deckcraft_engine::log_safe::redact(&err.to_string(), name)
                );
            }
        }
        e.drafted = Some(e.current.doc.clone());
        e.last_put = now;
        e.first_unput_change = None;
    }

    /// Open a draft the store handed back (`.deckcraft` bytes kept as `name`): unsaved, titled
    /// after `name`, and drafted under `key` from now on.
    pub fn restore(&mut self, app: &mut SlideApp, key: &str, name: &str, bytes: &[u8], now: f64) -> Result<(), String> {
        let uid = app.open_draft(name, bytes)?;
        let Some(current) = app.draft_inputs().into_iter().find(|d| d.uid == uid) else { return Ok(()) };
        let mut e = Entry::new(key.to_string(), current, now);
        // The store already holds exactly this.
        e.drafted = Some(e.current.doc.clone());
        e.stored = true;
        self.entries.insert(uid, e);
        Ok(())
    }
}

impl SlideApp {
    /// The open presentations as drafting sees them.
    pub fn draft_inputs(&self) -> Vec<DocInput> {
        self.session
            .documents()
            .iter()
            // The host keeps its own documents' unsaved bytes, so a reload can't offer an old draft over them.
            .filter(|d| !self.host_owns(d.uid))
            .map(|d| DocInput {
                uid: d.uid,
                doc: d.doc.clone(),
                name: format!("{}.pptx", d.title()),
                unsaved: d.is_dirty() || self.pending_saves.iter().any(|p| p.uid == d.uid),
                composing: d.composition.is_some(),
            })
            .collect()
    }

    /// Open draft bytes alongside the open presentations, unsaved (they were never saved to a
    /// file), with no path and the title of `name`. Returns the document's uid.
    pub fn open_draft(&mut self, name: &str, bytes: &[u8]) -> Result<u64, String> {
        let refused = |app: &mut SlideApp, e: deckcraft_engine::EngineError| {
            app.note_refusal(&e);
            e.to_string()
        };
        self.session.ensure_room().map_err(|e| refused(self, e))?;
        let p = deckcraft_engine::cmd::file::open_presentation(name, bytes).map_err(|e| e.to_string())?;
        deckcraft_engine::cmd::file::add_opened(&mut self.session, name, None, p).map_err(|e| refused(self, e))?;
        let d = self.session.active_mut().ok_or("no document")?;
        d.saved_doc = Arc::new((*d.doc).clone());
        d.revision += 1;
        Ok(d.uid)
    }
}
