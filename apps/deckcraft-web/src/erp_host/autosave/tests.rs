use super::*;

/// A document version: equal numbers are the same version.
#[derive(Clone, Copy, Debug, PartialEq)]
struct V(u32);

impl Snapshot for V {
    fn same(&self, other: &Self) -> bool {
        self == other
    }
}

const UID: u64 = 7;

fn scheduler() -> Scheduler<V> {
    let mut s = Scheduler::default();
    s.adopt(UID, V(0), 1);
    s
}

fn edit(s: &mut Scheduler<V>, version: u32, now: f64) {
    s.observe(UID, Observation { snapshot: V(version), revision: u64::from(version) + 1, busy: false, unconfirmed_losses: false }, now);
}

fn sent(send: Option<Send<V>>) -> (u64, V, Reason) {
    let send = send.expect("a save starts");
    (send.revision, send.snapshot, send.reason)
}

#[test]
fn autosave_waits_for_a_pause_then_at_most_the_long_wait() {
    let mut s = scheduler();
    assert!(s.due(100_000.0).is_empty(), "nothing to save while clean");
    edit(&mut s, 1, 0.0);
    assert!(s.due(2_999.0).is_empty());
    assert_eq!(s.next_wake(1_000.0), Some(2_000.0));
    let due = s.due(3_000.0);
    assert_eq!(due.len(), 1);
    assert_eq!((due[0].revision, due[0].reason), (2, Reason::Auto));

    let mut s = scheduler();
    // Edits every second never leave a 3 s pause: the save comes 30 s after the first one.
    for t in 0..30 {
        edit(&mut s, t + 1, f64::from(t) * 1_000.0);
        assert!(s.due(f64::from(t) * 1_000.0 + 999.0).is_empty(), "at {t} s");
    }
    edit(&mut s, 31, 30_000.0);
    assert_eq!(s.due(30_000.0).len(), 1);
}

#[test]
fn clean_only_as_of_the_revision_sent() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    let (revision, written, _) = sent(s.request(UID, Reason::Manual));
    assert_eq!((revision, written), (2, V(1)));
    edit(&mut s, 2, 10.0);
    let c = s.complete(UID, revision, Outcome::Saved { version: Some(5) });
    assert_eq!(c.saved, Some(V(1)));
    assert!(s.dirty(UID), "the edit typed while the save was in flight stays dirty");
    assert!(c.next.is_none(), "AutoSave picks it up after its pause");
    assert_eq!(s.due(3_010.0).len(), 1, "and the next AutoSave sends it");
}

#[test]
fn one_save_in_flight_and_later_triggers_coalesce_into_one_follow_up() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    let (first, _, _) = sent(s.request(UID, Reason::Auto));
    edit(&mut s, 2, 10.0);
    assert!(s.request(UID, Reason::Manual).is_none(), "queued behind the save in flight");
    edit(&mut s, 3, 20.0);
    assert!(s.request(UID, Reason::Auto).is_none());
    assert!(s.due(100_000.0).is_empty(), "no AutoSave while one is in flight");
    assert!(s.flush().is_empty());
    assert!(s.saving(UID));
    let c = s.complete(UID, first, Outcome::Saved { version: None });
    let (revision, snapshot, reason) = sent(c.next);
    assert_eq!((revision, snapshot, reason), (4, V(3), Reason::Manual), "one follow-up of the newest revision");
    assert!(s.complete(UID, revision, Outcome::Saved { version: None }).next.is_none());
    assert!(!s.dirty(UID));
}

#[test]
fn a_follow_up_of_what_is_already_in_flight_sends_nothing() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    let (revision, _, _) = sent(s.request(UID, Reason::Auto));
    assert!(s.request(UID, Reason::Manual).is_none());
    assert!(s.complete(UID, revision, Outcome::Retry).next.is_none(), "the shell owns retrying those bytes");
}

#[test]
fn a_stale_answer_changes_nothing() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    let (revision, _, _) = sent(s.request(UID, Reason::Auto));
    let c = s.complete(UID, revision + 1, Outcome::Saved { version: None });
    assert!(c.saved.is_none() && c.unsaved.is_none());
    assert!(s.saving(UID));
}

#[test]
fn conflict_stops_autosave_keeps_dirty_and_is_never_retried() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    let (revision, _, _) = sent(s.request(UID, Reason::Auto));
    edit(&mut s, 2, 10.0);
    assert!(s.request(UID, Reason::Manual).is_none());
    let c = s.complete(UID, revision, Outcome::Conflict);
    assert_eq!(c.unsaved, Some(None), "nothing for the editor to show");
    assert!(c.next.is_none(), "the queued follow-up is dropped");
    assert!(s.dirty(UID));
    edit(&mut s, 3, 20.0);
    assert!(s.due(100_000.0).is_empty(), "AutoSave stopped");
    assert!(s.flush().is_empty(), "flush doesn't retry a conflict");
    assert_eq!(s.next_wake(100.0), None);
    // Only the user's own Save goes again; the shell answers it.
    assert_eq!(sent(s.request(UID, Reason::Manual)).2, Reason::Manual);
}

#[test]
fn offline_and_retryable_keep_dirty_and_autosave() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    let (revision, _, _) = sent(s.request(UID, Reason::Auto));
    let c = s.complete(UID, revision, Outcome::Retry);
    assert_eq!(c.unsaved, Some(None));
    assert!(s.dirty(UID));
    assert!(s.due(100_000.0).is_empty(), "the editor doesn't resend the same bytes");
    edit(&mut s, 2, 200_000.0);
    assert_eq!(s.due(203_000.0).len(), 1, "AutoSave keeps running for new edits");
}

#[test]
fn a_failure_stops_autosave_shows_its_message_and_a_save_restarts_it() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    let (revision, _, _) = sent(s.request(UID, Reason::Auto));
    let c = s.complete(UID, revision, Outcome::Failed { message: "Tệp quá lớn".into() });
    assert_eq!(c.unsaved, Some(Some("Tệp quá lớn".into())));
    assert!(s.flush().is_empty(), "the same bytes would fail the same way");
    edit(&mut s, 2, 10.0);
    assert!(s.due(100_000.0).is_empty(), "AutoSave stopped");
    assert!(s.flush().is_empty(), "flush skips a stopped document even after an edit");
    edit(&mut s, 3, 200_000.0);
    let (revision, _, _) = sent(s.request(UID, Reason::Manual));
    s.complete(UID, revision, Outcome::Saved { version: Some(2) });
    edit(&mut s, 4, 300_000.0);
    assert_eq!(s.due(303_000.0).len(), 1, "a successful Save restarts AutoSave");
}

#[test]
fn flush_saves_every_dirty_document_and_ignores_busy() {
    let mut s = scheduler();
    s.adopt(8, V(0), 1);
    s.adopt(9, V(0), 1);
    edit(&mut s, 1, 0.0);
    s.observe(8, Observation { snapshot: V(5), revision: 6, busy: true, unconfirmed_losses: false }, 0.0);
    assert!(s.due(100_000.0).iter().all(|send| send.uid == UID), "a slide show holds AutoSave");
    assert_eq!(s.next_wake(100_000.0), None);
    let mut s2 = scheduler();
    s2.adopt(8, V(0), 1);
    s2.adopt(9, V(0), 1);
    edit(&mut s2, 1, 0.0);
    s2.observe(8, Observation { snapshot: V(5), revision: 6, busy: true, unconfirmed_losses: false }, 0.0);
    let uids: Vec<u64> = s2.flush().iter().map(|send| send.uid).collect();
    assert_eq!(uids, vec![UID, 8], "flush leaves out only the clean document");
    assert!(s2.flush().is_empty(), "already in flight");
}

#[test]
fn unconfirmed_losses_hold_autosave_and_flush() {
    let mut s = scheduler();
    s.observe(UID, Observation { snapshot: V(1), revision: 2, busy: false, unconfirmed_losses: true }, 0.0);
    assert!(s.due(100_000.0).is_empty());
    assert!(s.flush().is_empty());
    assert!(s.request(UID, Reason::Manual).is_some(), "a Save asks first, then sends");
}

#[test]
fn undoing_back_to_the_saved_version_is_clean_and_reported_once() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    assert_eq!(s.dirty_changes(), vec![(UID, true)]);
    assert!(s.dirty_changes().is_empty(), "only transitions");
    edit(&mut s, 0, 10.0);
    assert_eq!(s.dirty_changes(), vec![(UID, false)]);
    assert!(s.due(100_000.0).is_empty());
}

#[test]
fn save_reasons_match_the_contract() {
    assert_eq!([Reason::Auto, Reason::Manual, Reason::Flush].map(Reason::as_str), ["auto", "manual", "flush"]);
}

#[test]
fn acknowledged_after_offline_saves_that_revision() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    let (revision, _, _) = sent(s.request(UID, Reason::Auto));
    s.complete(UID, revision, Outcome::Retry);
    assert_eq!(s.acknowledged(UID, revision), Some(V(1)));
    assert!(!s.dirty(UID));
    assert_eq!(s.acknowledged(UID, revision), None, "only once");
    assert_eq!(s.acknowledged(UID, revision + 5), None, "a revision never sent");
}

#[test]
fn acknowledged_leaves_later_edits_dirty_and_older_ones_are_ignored() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    let (first, _, _) = sent(s.request(UID, Reason::Auto));
    s.complete(UID, first, Outcome::Retry);
    edit(&mut s, 2, 10.0);
    let (second, _, _) = sent(s.request(UID, Reason::Manual));
    s.complete(UID, second, Outcome::Retry);
    edit(&mut s, 3, 20.0);
    assert_eq!(s.acknowledged(UID, second), Some(V(2)));
    assert!(s.dirty(UID), "the edit after the save stays dirty");
    assert_eq!(s.acknowledged(UID, first), None, "older than the one acknowledged");
    let (third, _, _) = sent(s.request(UID, Reason::Manual));
    assert!(s.complete(UID, third, Outcome::Saved { version: Some(9) }).saved.is_some());
    assert!(!s.dirty(UID));
}

#[test]
fn acknowledged_restarts_a_stopped_autosave_like_saved() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    let (revision, _, _) = sent(s.request(UID, Reason::Auto));
    s.complete(UID, revision, Outcome::Retry);
    edit(&mut s, 2, 10.0);
    let (failed, _, _) = sent(s.request(UID, Reason::Auto));
    s.complete(UID, failed, Outcome::Failed { message: "x".into() });
    assert!(s.acknowledged(UID, revision).is_some());
    edit(&mut s, 3, 100.0);
    assert_eq!(s.due(3_100.0).len(), 1);
}

#[test]
fn a_flush_behind_a_save_in_flight_is_queued_until_handed_over() {
    let mut s = scheduler();
    edit(&mut s, 1, 0.0);
    let (revision, _, _) = sent(s.request(UID, Reason::Auto));
    edit(&mut s, 2, 10.0);
    assert!(s.flush().is_empty());
    assert!(s.queued(UID));
    let next = s.complete(UID, revision, Outcome::Saved { version: None }).next;
    assert_eq!(next.map(|n| n.reason), Some(Reason::Flush));
    assert!(!s.queued(UID), "handed over");
}
