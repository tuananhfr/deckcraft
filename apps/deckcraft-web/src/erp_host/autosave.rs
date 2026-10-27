//! When each host document is saved (`docs/erp-host.md` rules 1–6), with the time passed in: one
//! save in flight per document, later triggers coalescing behind it, AutoSave after a pause or
//! a long run of edits, and what an answer does to the document.

use std::collections::BTreeMap;

/// Quiet time after the last edit before AutoSave.
pub(crate) const IDLE_MS: f64 = 3_000.0;
/// Longest an edit waits for AutoSave while editing goes on without a pause.
pub(crate) const MAX_WAIT_MS: f64 = 30_000.0;
/// Unanswered saves remembered for `acknowledged`; the shell retries only its latest bytes.
const MAX_RETRIED: usize = 8;

/// A version of a document; `same` is identity, so undoing back to a saved version is clean.
pub(crate) trait Snapshot: Clone {
    fn same(&self, other: &Self) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    Auto,
    Manual,
    Flush,
}

impl Reason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Reason::Auto => "auto",
            Reason::Manual => "manual",
            Reason::Flush => "flush",
        }
    }
}

/// The shell's `SaveResult`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    Saved {
        version: Option<u64>,
    },
    Conflict,
    /// `offline`, or `failed` with `retryable`: the shell retries and shows the state.
    Retry,
    /// `failed` without `retryable`; the message is ready to show.
    Failed {
        message: String,
    },
}

/// A save to start now.
#[derive(Clone, Debug)]
pub(crate) struct Send<D> {
    pub uid: u64,
    pub revision: u64,
    pub snapshot: D,
    pub reason: Reason,
}

/// What an answer changes.
#[derive(Debug)]
pub(crate) struct Completion<D> {
    /// The version that is now durable.
    pub saved: Option<D>,
    /// The save ended unsaved; `Some` message to show.
    pub unsaved: Option<Option<String>>,
    /// The follow-up save that was waiting behind this one.
    pub next: Option<Send<D>>,
}

impl<D> Default for Completion<D> {
    fn default() -> Self {
        Completion { saved: None, unsaved: None, next: None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Halt {
    Conflict,
    Failed,
}

/// What the last frame saw of a document.
#[derive(Clone, Debug)]
pub(crate) struct Observation<D> {
    pub snapshot: D,
    pub revision: u64,
    /// AutoSave waits: a slide show, a drag or an input method composition is in progress.
    pub busy: bool,
    /// Saving would drop content the user hasn't agreed to lose (a Save confirms it).
    pub unconfirmed_losses: bool,
}

struct Doc<D> {
    seen: Observation<D>,
    acknowledged: D,
    /// The revision of `acknowledged`; an acknowledgement of an older one changes nothing.
    acknowledged_revision: u64,
    in_flight: Option<(u64, D)>,
    follow_up: Option<Reason>,
    /// Saves answered `offline` or retryable, which the shell may still store and acknowledge.
    retried: Vec<(u64, D)>,
    halt: Option<Halt>,
    first_edit: Option<f64>,
    last_edit: f64,
    reported_dirty: bool,
}

impl<D: Snapshot> Doc<D> {
    fn dirty(&self) -> bool {
        !self.seen.snapshot.same(&self.acknowledged)
    }

    /// False when a newer revision is already acknowledged.
    fn acknowledge(&mut self, revision: u64, written: D) -> bool {
        if revision < self.acknowledged_revision {
            return false;
        }
        self.acknowledged = written;
        self.acknowledged_revision = revision;
        self.halt = None;
        self.retried.retain(|(r, _)| *r > revision);
        true
    }
}

pub(crate) struct Scheduler<D> {
    docs: BTreeMap<u64, Doc<D>>,
}

impl<D> Default for Scheduler<D> {
    fn default() -> Self {
        Scheduler { docs: BTreeMap::new() }
    }
}

impl<D: Snapshot> Scheduler<D> {
    /// Starts tracking `uid`, opened at `snapshot` (what the server has).
    pub(crate) fn adopt(&mut self, uid: u64, snapshot: D, revision: u64) {
        let seen = Observation { snapshot: snapshot.clone(), revision, busy: false, unconfirmed_losses: false };
        let doc = Doc {
            seen,
            acknowledged: snapshot,
            acknowledged_revision: revision,
            in_flight: None,
            follow_up: None,
            retried: Vec::new(),
            halt: None,
            first_edit: None,
            last_edit: 0.0,
            reported_dirty: false,
        };
        self.docs.insert(uid, doc);
    }

    /// Stops tracking `uid`; returns whether it was tracked.
    pub(crate) fn remove(&mut self, uid: u64) -> bool {
        self.docs.remove(&uid).is_some()
    }

    pub(crate) fn uids(&self) -> Vec<u64> {
        self.docs.keys().copied().collect()
    }

    pub(crate) fn dirty(&self, uid: u64) -> bool {
        self.docs.get(&uid).is_some_and(Doc::dirty)
    }

    /// A save of `uid` is in flight or waiting behind one.
    pub(crate) fn saving(&self, uid: u64) -> bool {
        self.docs.get(&uid).is_some_and(|d| d.in_flight.is_some())
    }

    /// The dirty state `reported` last, for a document that takes `uid`'s place.
    pub(crate) fn inherit_reported(&mut self, uid: u64, reported: bool) {
        if let Some(d) = self.docs.get_mut(&uid) {
            d.reported_dirty = reported;
        }
    }

    pub(crate) fn reported_dirty(&self, uid: u64) -> bool {
        self.docs.get(&uid).is_some_and(|d| d.reported_dirty)
    }

    pub(crate) fn observe(&mut self, uid: u64, seen: Observation<D>, now: f64) {
        let Some(d) = self.docs.get_mut(&uid) else { return };
        if !seen.snapshot.same(&d.seen.snapshot) {
            d.last_edit = now;
            d.first_edit.get_or_insert(now);
        }
        d.seen = seen;
        if !d.dirty() {
            d.first_edit = None;
        }
    }

    /// The edit an observation missed (Save typed in the same frame as the edit).
    pub(crate) fn observe_snapshot(&mut self, uid: u64, snapshot: D, revision: u64, now: f64) {
        let Some(seen) = self.docs.get(&uid).map(|d| Observation { snapshot, revision, ..d.seen.clone() }) else { return };
        self.observe(uid, seen, now);
    }

    /// Asks for a save of `uid`; `Some` when it should start now.
    pub(crate) fn request(&mut self, uid: u64, reason: Reason) -> Option<Send<D>> {
        let d = self.docs.get_mut(&uid)?;
        if d.in_flight.is_some() {
            // A manual save outranks the follow-up reason it joins; the newest revision is sent either way.
            if d.follow_up != Some(Reason::Manual) {
                d.follow_up = Some(reason);
            }
            return None;
        }
        let allowed = match reason {
            Reason::Manual => d.dirty(),
            Reason::Auto => d.dirty() && d.halt.is_none() && !d.seen.busy && !d.seen.unconfirmed_losses,
            // A conflict is never retried by the editor; a failure only once there is something new to send.
            // A conflict or a failure is never retried by the editor on its own.
            Reason::Flush => d.dirty() && d.halt.is_none() && !d.seen.unconfirmed_losses,
        };
        if !allowed {
            return None;
        }
        d.in_flight = Some((d.seen.revision, d.seen.snapshot.clone()));
        d.first_edit = None;
        Some(Send { uid, revision: d.seen.revision, snapshot: d.seen.snapshot.clone(), reason })
    }

    /// AutoSaves due at `now`.
    pub(crate) fn due(&mut self, now: f64) -> Vec<Send<D>> {
        let due: Vec<u64> = self
            .docs
            .iter()
            .filter(|(_, d)| d.in_flight.is_none() && d.first_edit.is_some_and(|first| now - d.last_edit >= IDLE_MS || now - first >= MAX_WAIT_MS))
            .map(|(uid, _)| *uid)
            .collect();
        due.into_iter().filter_map(|uid| self.request(uid, Reason::Auto)).collect()
    }

    /// Milliseconds until the next AutoSave may be due, to wake the page then.
    pub(crate) fn next_wake(&self, now: f64) -> Option<f64> {
        self.docs
            .values()
            .filter(|d| d.halt.is_none() && d.in_flight.is_none() && !d.seen.busy && !d.seen.unconfirmed_losses)
            .filter_map(|d| d.first_edit.map(|first| (d.last_edit + IDLE_MS).min(first + MAX_WAIT_MS) - now))
            .reduce(f64::min)
            .map(|ms| ms.max(0.0))
    }

    /// Every document whose revision differs from its acknowledged one (`flush`).
    pub(crate) fn flush(&mut self) -> Vec<Send<D>> {
        self.uids().into_iter().filter_map(|uid| self.request(uid, Reason::Flush)).collect()
    }

    /// The host answered the save of `uid` at `revision`.
    pub(crate) fn complete(&mut self, uid: u64, revision: u64, outcome: Outcome) -> Completion<D> {
        let Some(d) = self.docs.get_mut(&uid) else { return Completion::default() };
        let Some(written) = d.in_flight.take_if(|(r, _)| *r == revision).map(|(_, written)| written) else { return Completion::default() };
        let mut completion = Completion::default();
        match outcome {
            Outcome::Saved { .. } => {
                if d.acknowledge(revision, written.clone()) {
                    completion.saved = Some(written.clone());
                }
            }
            Outcome::Conflict => {
                d.halt = Some(Halt::Conflict);
                d.follow_up = None;
                completion.unsaved = Some(None);
            }
            Outcome::Retry => {
                if d.retried.len() >= MAX_RETRIED {
                    d.retried.remove(0);
                }
                d.retried.push((revision, written.clone()));
                completion.unsaved = Some(None);
            }
            Outcome::Failed { message } => {
                d.halt = Some(Halt::Failed);
                d.follow_up = None;
                completion.unsaved = Some(Some(message));
            }
        }
        if !d.dirty() {
            d.first_edit = None;
        }
        // The newest revision may already be what was just sent; then nothing follows.
        let follow_up = d.follow_up.take().filter(|_| !d.seen.snapshot.same(&written));
        completion.next = follow_up.and_then(|reason| self.request(uid, reason));
        completion
    }

    /// The shell stored `revision` on its own retry (`acknowledged`): as `saved` for it, unless a
    /// newer revision was acknowledged since. Returns the version now saved.
    pub(crate) fn acknowledged(&mut self, uid: u64, revision: u64) -> Option<D> {
        let d = self.docs.get_mut(&uid)?;
        if revision <= d.acknowledged_revision {
            return None;
        }
        let written = d.retried.iter().find(|(r, _)| *r == revision).map(|(_, written)| written.clone())?;
        if !d.acknowledge(revision, written.clone()) {
            return None;
        }
        if !d.dirty() {
            d.first_edit = None;
        }
        Some(written)
    }

    /// A save of `uid` waits behind the one in flight, not yet handed to the shell.
    pub(crate) fn queued(&self, uid: u64) -> bool {
        self.docs.get(&uid).is_some_and(|d| d.follow_up.is_some())
    }

    /// `dirty(docId, …)` transitions since the last call, as (uid, dirty).
    pub(crate) fn dirty_changes(&mut self) -> Vec<(u64, bool)> {
        let mut changes = vec![];
        for (uid, d) in &mut self.docs {
            let dirty = d.dirty();
            if dirty != d.reported_dirty {
                d.reported_dirty = dirty;
                changes.push((*uid, dirty));
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests;
