//! One frame of co-editing: the shell's calls, lock answers, the user's changes, letting regions
//! go, and the selection.

use std::collections::HashSet;

use deckcraft_engine::live::{Blocked, LiveError, LocalChange, Region};
use deckcraft_engine::model::{Shape, ShapeId, Slide, walk};
use deckcraft_engine::{DocState, Target};

use super::{ConflictPrompt, FOCUS_IDLE_MS, IDLE_END_MS, Live, LiveEvent, SELECTION_MS};
use crate::SlideApp;

/// The top-level shape of `slide` that is or holds shape `id`.
fn top_level(slide: &Slide, id: ShapeId) -> Option<ShapeId> {
    slide
        .shapes
        .iter()
        .find(|top| {
            let mut found = top.id == id;
            walk(std::slice::from_ref(*top), &mut |s: &Shape, _| found |= s.id == id);
            found
        })
        .map(|s| s.id)
}

/// The region shape `id` of the edited slide belongs to (`None` in Slide Master view).
fn shape_region(st: &DocState, id: ShapeId) -> Option<String> {
    if st.selection.target != Target::Slides {
        return None;
    }
    let slide = st.current_slide()?;
    top_level(slide, id).map(|top| Region::Shape(slide.id, top).to_string())
}

impl Live {
    fn blocked(&self) -> Blocked {
        let mut blocked = Blocked::default();
        for region in self.others.keys() {
            match Region::parse(region) {
                Some(Region::Document) => blocked.document = true,
                Some(r) => {
                    blocked.regions.insert(r);
                }
                None => {}
            }
        }
        blocked
    }

    fn busy(&self) -> HashSet<String> {
        self.waiting.iter().flat_map(|c| c.regions.iter().map(Region::to_string)).collect()
    }
}

impl SlideApp {
    /// Follow presentation `uid` (the host document opened last): live mode goes with it.
    pub fn live_bind(&mut self, uid: Option<u64>) {
        let Some(live) = self.live.as_mut() else { return };
        if live.doc == uid {
            return;
        }
        let old = live.doc;
        live.reset();
        live.doc = uid;
        if let Some(old) = old {
            let _ = self.session.set_live(old, false);
        }
        if let Some(new) = uid {
            let _ = self.session.set_live(new, true);
        }
    }

    /// One step of co-editing; `true` while something waits on time (a lock to let go, a
    /// selection to send) or the presentation changed.
    pub fn live_step(&mut self) -> bool {
        let Some(live) = self.live.as_ref() else { return false };
        let Some(uid) = live.doc else { return false };
        let now = live.host.now_ms();
        let applied = self.live_events(uid);
        self.live_outgoing(uid, now);
        self.live_release(uid, now);
        let selection_pending = self.live_selection(uid, now);
        if let Some(region) = self.session.take_live_refused(uid) {
            self.live_lock_notice(&region.to_string());
        }
        if self.session.take_live_skipped(uid) > 0 {
            self.toast =
                Some(crate::toast::Toast { text: crate::tl!("Skipped a change someone else has edited since.").to_string(), at: crate::now_ms() });
        }
        applied || selection_pending || self.live.as_ref().is_some_and(|l| !l.held.is_empty() || !l.waiting.is_empty() || !l.conflicts.is_empty())
    }

    fn live_events(&mut self, uid: u64) -> bool {
        let Some(live) = self.live.as_mut() else { return false };
        let events = std::mem::take(&mut *live.inbox.borrow_mut());
        let mut changed = false;
        for event in events {
            match event {
                LiveEvent::ApplyRemote { region, payload, .. } => {
                    // One this side can't apply is dropped, leaving the presentation as it was here.
                    changed |= self.session.apply_remote(uid, &region, &payload).is_ok_and(|a| a.changed);
                }
                LiveEvent::LockDenied(region) => self.live_denied(uid, &region),
                other => {
                    let Some(live) = self.live.as_mut() else { return changed };
                    changed = true;
                    match other {
                        LiveEvent::Presence(members) => live.members = members,
                        LiveEvent::Locks(locks) => {
                            let me = live.me().map(str::to_owned);
                            live.others = locks
                                .into_iter()
                                .filter(|l| !l.mine && me.as_deref() != Some(l.holder.as_str()) && !live.held.contains_key(&l.region))
                                .map(|l| (l.region.clone(), l))
                                .collect();
                            let blocked = live.blocked();
                            self.session.set_live_blocked(uid, blocked);
                        }
                        LiveEvent::Cursors(cursors) => {
                            let me = live.me().map(str::to_owned);
                            live.cursors = cursors.into_iter().filter(|c| me.as_deref() != Some(c.author.as_str())).collect();
                        }
                        LiveEvent::Leader(leader) => live.leader = leader,
                        LiveEvent::Conflict { region, reply } => live.conflicts.push(ConflictPrompt { region, reply }),
                        LiveEvent::ApplyRemote { .. } | LiveEvent::LockDenied(_) => {}
                    }
                }
            }
        }
        changed
    }

    fn live_answers(&mut self, uid: u64, now: f64) {
        let Some(live) = self.live.as_mut() else { return };
        let answers = std::mem::take(&mut *live.answers.borrow_mut());
        let mut denied = Vec::new();
        for (region, granted) in answers {
            live.asking.remove(&region);
            if granted {
                live.held.insert(region, now);
            } else {
                denied.push(region);
            }
        }
        for region in denied {
            self.live_denied(uid, &region);
        }
    }

    /// The room refused `region` (or took it back): the user's steps on it waiting to be sent are
    /// taken back, the inline edit in it ends (what was sent stays), and the user hears who holds
    /// it.
    pub(crate) fn live_denied(&mut self, uid: u64, region: &str) {
        let Some(live) = self.live.as_mut() else { return };
        live.held.remove(region);
        live.asking.remove(region);
        let (doomed, kept): (Vec<LocalChange>, Vec<LocalChange>) =
            std::mem::take(&mut live.waiting).into_iter().partition(|c| c.regions.iter().any(|r| r.to_string() == region));
        live.waiting = kept;
        for change in doomed.iter().rev() {
            let _ = self.session.revert_live_change(uid, change);
        }
        if self.live_intent(uid).contains(region) && self.session.active().is_some_and(|d| d.selection.text.is_some()) {
            let _ = self.session.execute("text.exit", &serde_json::json!({}));
        }
        self.live_lock_notice(region);
    }

    /// "<name> is editing this part." for a region someone else holds.
    pub(crate) fn live_lock_notice(&mut self, region: &str) {
        let name = self
            .live
            .as_ref()
            .and_then(|l| l.others.get(region).or_else(|| l.others.get("*")).and_then(|lock| l.name_of(&lock.holder, lock.name.as_ref())));
        let text = match name {
            Some(name) => crate::i18n::fmt(crate::tl!("{name} is editing this part."), &[("name", &name)]),
            None => crate::tl!("Someone else is editing this part.").to_string(),
        };
        self.toast = Some(crate::toast::Toast { text, at: crate::now_ms() });
    }

    /// The regions the user is about to change: the text being edited (a shape's, or the notes',
    /// which are the slide's), the shapes being dragged. Their locks are asked for once per focus,
    /// before the change normally.
    fn live_intent(&self, uid: u64) -> HashSet<String> {
        let mut out = HashSet::new();
        let Some(st) = self.session.documents().iter().find(|d| d.uid == uid) else { return out };
        if let Some(text) = &st.selection.text {
            if text.notes {
                out.extend(st.current_slide().map(|s| Region::Slide(s.id).to_string()));
            } else {
                out.extend(shape_region(st, text.shape));
            }
        }
        if st.interaction.is_some() {
            out.extend(st.selection.shapes.iter().filter_map(|id| shape_region(st, *id)));
        }
        out
    }

    fn live_outgoing(&mut self, uid: u64, now: f64) {
        let changes = self.session.take_live_changes(uid);
        let intent = self.live_intent(uid);
        let Some(live) = self.live.as_mut() else { return };
        live.waiting.extend(changes);
        let fresh: Vec<String> = intent.iter().filter(|r| !live.intent.contains(*r)).cloned().collect();
        live.intent = intent;
        for region in live.busy().into_iter().chain(fresh) {
            live.ask(&region);
        }
        // A shell may answer at once.
        self.live_answers(uid, now);
        let Some(live) = self.live.as_mut() else { return };
        let (ready, waiting): (Vec<LocalChange>, Vec<LocalChange>) =
            std::mem::take(&mut live.waiting).into_iter().partition(|c| c.regions.iter().all(|r| live.held.contains_key(&r.to_string())));
        live.waiting = waiting;
        let mut sent: HashSet<Region> = HashSet::new();
        let mut too_large = false;
        for change in ready {
            let mut payloads = Vec::new();
            let mut refused = false;
            for region in change.regions.iter().filter(|r| !sent.contains(*r)) {
                match self.session.live_payload(uid, region) {
                    Ok(p) => payloads.push((*region, p)),
                    Err(LiveError::TooLarge) => {
                        refused = true;
                        break;
                    }
                    // Nothing to send for it (the presentation closed meanwhile).
                    Err(_) => {}
                }
            }
            if refused {
                too_large = true;
                let _ = self.session.revert_live_change(uid, &change);
                continue;
            }
            let Some(live) = self.live.as_mut() else { return };
            for (region, payload) in payloads {
                let key = region.to_string();
                live.host.region_changed(&key, &payload);
                live.remember_sent(&key, &payload);
                live.held.insert(key, now);
                sent.insert(region);
            }
        }
        if too_large {
            self.toast = Some(crate::toast::Toast {
                text: crate::tl!("This change is too large to share while editing together.").to_string(),
                at: crate::now_ms(),
            });
        }
    }

    /// Let go of the regions the user left: idle past [`IDLE_END_MS`], or past [`FOCUS_IDLE_MS`]
    /// while still focused.
    fn live_release(&mut self, uid: u64, now: f64) {
        if self.live.as_ref().is_none_or(|l| l.held.is_empty()) {
            return;
        }
        let focus = self.live_intent(uid);
        let Some(live) = self.live.as_mut() else { return };
        let busy = live.busy();
        let ended: Vec<String> = live
            .held
            .iter()
            .filter(|(r, last)| !busy.contains(*r) && now - **last > if focus.contains(*r) { FOCUS_IDLE_MS } else { IDLE_END_MS })
            .map(|(r, _)| r.clone())
            .collect();
        for region in ended {
            live.held.remove(&region);
            live.host.region_did_end_edit(&region);
        }
    }

    /// Report where the user is: the shape being edited or selected (range: the text selection as
    /// `paragraph.char-paragraph.char`), else the slide shown. `true` while a change waits for the
    /// throttle.
    fn live_selection(&mut self, uid: u64, now: f64) -> bool {
        let Some(st) = self.session.documents().iter().find(|d| d.uid == uid) else { return false };
        let selection = match (&st.selection.text, st.selection.shapes.first()) {
            (Some(t), _) if !t.notes => shape_region(st, t.shape).map(|r| (r, format!("{}.{}-{}.{}", t.anchor.0, t.anchor.1, t.caret.0, t.caret.1))),
            (_, Some(id)) => shape_region(st, *id).map(|r| (r, String::new())),
            _ => None,
        };
        let Some(selection) = selection.or_else(|| st.current_slide().map(|s| (Region::Slide(s.id).to_string(), String::new()))) else {
            return false;
        };
        let Some(live) = self.live.as_mut() else { return false };
        if live.selection.as_ref() == Some(&selection) {
            return false;
        }
        if now - live.selection_sent_ms < SELECTION_MS {
            return true;
        }
        live.host.selection_changed(&selection.0, &selection.1);
        live.selection = Some(selection);
        live.selection_sent_ms = now;
        false
    }
}
