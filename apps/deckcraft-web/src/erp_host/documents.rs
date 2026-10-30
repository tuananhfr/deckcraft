//! The open host documents, shared by the frame, the shell's `flush` and the save answers that
//! arrive between frames (a hidden page draws no frames, so those can't wait for one).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use deckcraft_engine::LossItem;
use deckcraft_engine::model::Presentation;

use super::autosave::{Observation, Outcome, Reason, Scheduler, Send, Snapshot};

impl Snapshot for Arc<Presentation> {
    fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(self, other)
    }
}

/// `window.erpHost`'s editor → shell calls.
pub(crate) trait HostApi {
    /// `erpHost.save`; `done` gets the answer, never before `save` returns.
    fn save(&mut self, doc_id: &str, bytes: Vec<u8>, revision: u64, reason: Reason, done: Box<dyn FnOnce(Outcome)>);
    fn dirty(&mut self, doc_id: &str, dirty: bool);
    fn opened(&mut self, doc_id: &str, ok: bool, reason: Option<&str>);
    fn closed(&mut self, doc_id: &str);
}

/// What the app must apply in its next frame.
pub(crate) enum Apply {
    Saved(u64, Arc<Presentation>),
    Unsaved(u64, Option<String>),
}

struct Meta {
    doc_id: String,
    version: u64,
    format: String,
    /// The losses the user last saved over; AutoSave goes on while they are the document's losses.
    accepted: Vec<LossItem>,
    /// The losses of the document as last seen, by version.
    losses: Option<(Arc<Presentation>, Vec<LossItem>)>,
}

#[derive(Default)]
struct State {
    scheduler: Scheduler<Arc<Presentation>>,
    meta: BTreeMap<u64, Meta>,
    applies: Vec<Apply>,
    /// `flush` promises waiting for these documents' queued saves to be handed to the shell.
    waiters: Vec<(Vec<u64>, Box<dyn FnOnce()>)>,
}

struct Inner {
    state: RefCell<State>,
    host: RefCell<Box<dyn HostApi>>,
    answers: RefCell<Vec<(u64, u64, Outcome)>>,
    clock: Box<dyn Fn() -> f64>,
    wake: Box<dyn Fn()>,
}

#[derive(Clone)]
pub(crate) struct HostDocs(Rc<Inner>);

impl HostDocs {
    /// `clock` in ms; `wake` asks for a frame.
    pub(crate) fn new(host: Box<dyn HostApi>, clock: Box<dyn Fn() -> f64>, wake: Box<dyn Fn()>) -> Self {
        HostDocs(Rc::new(Inner { state: RefCell::default(), host: RefCell::new(host), answers: RefCell::default(), clock, wake }))
    }

    pub(crate) fn host(&self) -> Option<std::cell::RefMut<'_, Box<dyn HostApi>>> {
        self.0.host.try_borrow_mut().ok()
    }

    pub(crate) fn adopt(&self, uid: u64, doc_id: &str, version: u64, format: &str, doc: Arc<Presentation>, revision: u64) {
        let Ok(mut s) = self.0.state.try_borrow_mut() else { return };
        s.scheduler.adopt(uid, doc, revision);
        s.meta.insert(uid, Meta { doc_id: doc_id.into(), version, format: format.into(), accepted: vec![], losses: None });
    }

    /// Forgets `uid` without telling the shell it closed (its document is open again elsewhere).
    /// Returns the dirty state last reported for it.
    pub(crate) fn forget(&self, uid: u64) -> bool {
        let Ok(mut s) = self.0.state.try_borrow_mut() else { return false };
        let reported = s.scheduler.reported_dirty(uid);
        s.scheduler.remove(uid);
        s.meta.remove(&uid);
        reported
    }

    pub(crate) fn inherit_reported(&self, uid: u64, reported: bool) {
        if let Ok(mut s) = self.0.state.try_borrow_mut() {
            s.scheduler.inherit_reported(uid, reported);
        }
    }

    /// Read-only (view and comment modes), `uid` gets no AutoSave or Save, only `flush`; a pending
    /// `flush` stops waiting for a follow-up that is dropped.
    pub(crate) fn set_read_only(&self, uid: u64, read_only: bool) {
        if let Ok(mut s) = self.0.state.try_borrow_mut() {
            s.scheduler.set_read_only(uid, read_only);
        }
        self.resolve_waiters();
    }

    pub(crate) fn uid_for(&self, doc_id: &str) -> Option<u64> {
        self.0.state.try_borrow().ok()?.meta.iter().find(|(_, m)| m.doc_id == doc_id).map(|(uid, _)| *uid)
    }

    pub(crate) fn version(&self, uid: u64) -> Option<u64> {
        self.0.state.try_borrow().ok()?.meta.get(&uid).map(|m| m.version)
    }

    pub(crate) fn dirty(&self, uid: u64) -> bool {
        self.0.state.try_borrow().is_ok_and(|s| s.scheduler.dirty(uid))
    }

    /// The frame's view of `uid`; `busy` holds AutoSave.
    pub(crate) fn observe(&self, uid: u64, doc: &Arc<Presentation>, revision: u64, busy: bool) {
        let now = (self.0.clock)();
        let Ok(mut s) = self.0.state.try_borrow_mut() else { return };
        let State { scheduler, meta, .. } = &mut *s;
        let Some(m) = meta.get_mut(&uid) else { return };
        if m.losses.as_ref().is_none_or(|(seen, _)| !Arc::ptr_eq(seen, doc)) {
            let losses = deckcraft_engine::save_losses::losses_for(doc, &m.format);
            m.losses = Some((doc.clone(), losses));
        }
        let unconfirmed_losses = m.losses.as_ref().is_some_and(|(_, losses)| !losses.is_empty() && *losses != m.accepted);
        scheduler.observe(uid, Observation { snapshot: doc.clone(), revision, busy, unconfirmed_losses }, now);
    }

    /// Save, Ctrl+S or a close prompt's Save (any loss warning was confirmed). False when there is
    /// nothing to send or wait for.
    pub(crate) fn manual(&self, uid: u64, doc: &Arc<Presentation>, revision: u64) -> bool {
        let now = (self.0.clock)();
        let send = {
            let Ok(mut s) = self.0.state.try_borrow_mut() else { return false };
            let State { scheduler, meta, .. } = &mut *s;
            if let Some(m) = meta.get_mut(&uid) {
                m.accepted = deckcraft_engine::save_losses::losses_for(doc, &m.format);
            }
            scheduler.observe_snapshot(uid, doc.clone(), revision, now);
            let send = scheduler.request(uid, Reason::Manual);
            if send.is_none() && !scheduler.saving(uid) {
                return false;
            }
            send
        };
        self.send(send.into_iter().collect());
        self.pump();
        self.report_dirty();
        true
    }

    pub(crate) fn saving(&self, uid: u64) -> bool {
        self.0.state.try_borrow().is_ok_and(|s| s.scheduler.saving(uid))
    }

    /// Starts the AutoSaves that are due; returns ms until the next one may be.
    pub(crate) fn tick(&self) -> Option<f64> {
        let now = (self.0.clock)();
        let (due, wake) = {
            let Ok(mut s) = self.0.state.try_borrow_mut() else { return None };
            let due = s.scheduler.due(now);
            (due, s.scheduler.next_wake(now))
        };
        self.send(due);
        self.pump();
        self.report_dirty();
        wake
    }

    /// `flush()`: saves every document with unacknowledged edits; `resolve` runs once each save is
    /// handed to the shell, without waiting for its answer (the bridge also resolves after 10 s).
    pub(crate) fn flush(&self, resolve: Box<dyn FnOnce()>) {
        let sends = match self.0.state.try_borrow_mut() {
            Ok(mut s) => s.scheduler.flush(),
            Err(_) => vec![],
        };
        self.send(sends);
        self.pump();
        let waiting: Vec<u64> = match self.0.state.try_borrow() {
            Ok(s) => s.scheduler.uids().into_iter().filter(|uid| s.scheduler.queued(*uid)).collect(),
            Err(_) => vec![],
        };
        match self.0.state.try_borrow_mut() {
            Ok(mut s) if !waiting.is_empty() => s.waiters.push((waiting, resolve)),
            _ => resolve(),
        }
        self.report_dirty();
    }

    /// `acknowledged(docId, revision, version)`: the shell's own retry stored `revision`.
    pub(crate) fn acknowledged(&self, doc_id: &str, revision: u64, version: Option<u64>) {
        let Some(uid) = self.uid_for(doc_id) else { return };
        {
            let Ok(mut s) = self.0.state.try_borrow_mut() else { return };
            let Some(written) = s.scheduler.acknowledged(uid, revision) else { return };
            if let (Some(m), Some(v)) = (s.meta.get_mut(&uid), version) {
                m.version = v;
            }
            s.applies.push(Apply::Saved(uid, written));
        }
        self.report_dirty();
        (self.0.wake)();
    }

    /// The shell answered a save: applies it and starts the follow-up waiting behind it.
    pub(crate) fn answer(&self, uid: u64, revision: u64, outcome: Outcome) {
        if let Ok(mut answers) = self.0.answers.try_borrow_mut() {
            answers.push((uid, revision, outcome));
        }
        self.pump();
        (self.0.wake)();
    }

    /// Applies queued answers; a frame calls it too, for any that arrived while state was busy.
    pub(crate) fn pump(&self) {
        loop {
            let answers = match self.0.answers.try_borrow_mut() {
                Ok(mut a) => std::mem::take(&mut *a),
                Err(_) => return,
            };
            if answers.is_empty() {
                break;
            }
            let mut sends = vec![];
            {
                let Ok(mut s) = self.0.state.try_borrow_mut() else {
                    if let Ok(mut a) = self.0.answers.try_borrow_mut() {
                        a.extend(answers);
                    }
                    return;
                };
                for (uid, revision, outcome) in answers {
                    let version = match &outcome {
                        Outcome::Saved { version } => *version,
                        _ => None,
                    };
                    let c = s.scheduler.complete(uid, revision, outcome);
                    if let Some(written) = c.saved {
                        if let (Some(m), Some(v)) = (s.meta.get_mut(&uid), version) {
                            m.version = v;
                        }
                        s.applies.push(Apply::Saved(uid, written));
                    }
                    if let Some(message) = c.unsaved {
                        s.applies.push(Apply::Unsaved(uid, message));
                    }
                    sends.extend(c.next);
                }
            }
            self.send(sends);
        }
        self.resolve_waiters();
        self.report_dirty();
    }

    pub(crate) fn take_applies(&self) -> Vec<Apply> {
        self.0.state.try_borrow_mut().map(|mut s| std::mem::take(&mut s.applies)).unwrap_or_default()
    }

    /// Tells the shell about documents no longer open (their tab was closed).
    pub(crate) fn retain_open(&self, open: &[u64]) {
        let closed: Vec<String> = {
            let Ok(mut s) = self.0.state.try_borrow_mut() else { return };
            let gone: Vec<u64> = s.meta.keys().copied().filter(|uid| !open.contains(uid)).collect();
            gone.into_iter()
                .filter_map(|uid| {
                    s.scheduler.remove(uid);
                    s.meta.remove(&uid).map(|m| m.doc_id)
                })
                .collect()
        };
        if closed.is_empty() {
            return;
        }
        if let Some(mut host) = self.host() {
            for doc_id in &closed {
                host.closed(doc_id);
            }
        }
        self.resolve_waiters();
    }

    pub(crate) fn report_dirty(&self) {
        let changes: Vec<(String, bool)> = {
            let Ok(mut s) = self.0.state.try_borrow_mut() else { return };
            let changes = s.scheduler.dirty_changes();
            changes.into_iter().filter_map(|(uid, dirty)| s.meta.get(&uid).map(|m| (m.doc_id.clone(), dirty))).collect()
        };
        if changes.is_empty() {
            return;
        }
        if let Some(mut host) = self.host() {
            for (doc_id, dirty) in changes {
                host.dirty(&doc_id, dirty);
            }
        }
    }

    fn resolve_waiters(&self) {
        let ready: Vec<Box<dyn FnOnce()>> = {
            let Ok(mut s) = self.0.state.try_borrow_mut() else { return };
            let State { scheduler, waiters, .. } = &mut *s;
            let (ready, waiting): (Vec<_>, Vec<_>) =
                std::mem::take(waiters).into_iter().partition(|(uids, _)| !uids.iter().any(|uid| scheduler.queued(*uid)));
            *waiters = waiting;
            ready.into_iter().map(|(_, resolve)| resolve).collect()
        };
        for resolve in ready {
            resolve();
        }
    }

    /// Encodes each save and hands it to the shell; one that can't be encoded fails at once.
    fn send(&self, sends: Vec<Send<Arc<Presentation>>>) {
        for send in sends {
            let Some((doc_id, format)) =
                self.0.state.try_borrow().ok().and_then(|s| s.meta.get(&send.uid).map(|m| (m.doc_id.clone(), m.format.clone())))
            else {
                continue;
            };
            let (uid, revision) = (send.uid, send.revision);
            match deckcraft_engine::cmd::file::save_bytes(&send.snapshot, &format) {
                Ok(bytes) => {
                    let docs = self.clone();
                    let done = Box::new(move |outcome| docs.answer(uid, revision, outcome));
                    match self.host() {
                        Some(mut host) => host.save(&doc_id, bytes, revision, send.reason, done),
                        None => self.queue_answer(uid, revision, Outcome::Retry),
                    }
                }
                Err(e) => {
                    log::error!("couldn't encode a presentation for the office site: {e}");
                    self.queue_answer(uid, revision, Outcome::Failed { message: super::outcome::failed_message().into() });
                }
            }
        }
    }

    /// An answer known before the save could start, applied by the running pump or the next frame.
    fn queue_answer(&self, uid: u64, revision: u64, outcome: Outcome) {
        if let Ok(mut answers) = self.0.answers.try_borrow_mut() {
            answers.push((uid, revision, outcome));
        }
        (self.0.wake)();
    }
}

impl deckcraft_ui_egui::host_documents::HostDocuments for HostDocs {
    fn owns(&self, uid: u64) -> bool {
        self.0.state.try_borrow().is_ok_and(|s| s.meta.contains_key(&uid))
    }

    fn save(&mut self, uid: u64, doc: &Arc<Presentation>, revision: u64) -> bool {
        self.manual(uid, doc, revision)
    }

    fn saving(&self, uid: u64) -> bool {
        HostDocs::saving(self, uid)
    }
}
