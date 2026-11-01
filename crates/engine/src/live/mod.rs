//! Editing together by region (the office shell's `erpHost` v3; only the web app turns it on).
//!
//! In live mode every undo step of a presentation names the regions it changed ([`Region`]), found
//! by comparing the presentation before and after it, and waits in an outbox for the app to send
//! ([`Session::take_live_changes`], [`Session::live_payload`]). Other people's regions come in
//! through [`Session::apply_remote`], which changes nothing else and is not an undo step. A change
//! to a region someone else holds is refused before it lands ([`Session::set_live_blocked`]), and
//! one whose lock the room refused afterwards is taken back ([`Session::revert_live_change`]).
//! Undo and redo are per user: a step whose region someone else changed since is skipped, and
//! once anyone else changed anything, a step puts back only its own regions.

mod diff;
mod payload;
mod region;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use deckcraft_model::{Presentation, SlideId};
use serde_json::{Value, json};

pub use payload::{MAX_PAYLOAD_BYTES, VERSION};
pub use region::Region;

use crate::{DocState, EngineError, HistoryEntry, Session};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LiveError {
    #[error("the presentation isn't being edited together")]
    NotLive,
    #[error("no such presentation")]
    NoDocument,
    #[error("payload version {0} is newer than this app")]
    Version(u64),
    #[error("malformed payload: {0}")]
    Malformed(String),
    #[error("this change is too large to share while editing together")]
    TooLarge,
    #[error("couldn't apply the change: {0}")]
    Apply(String),
}

/// The regions other people hold.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blocked {
    pub regions: HashSet<Region>,
    /// Someone holds `*`.
    pub document: bool,
}

impl Blocked {
    /// `*` clashes with every region, any other region only with itself.
    pub fn blocks(&self, region: &Region) -> bool {
        self.document || self.regions.contains(region) || (*region == Region::Document && !self.regions.is_empty())
    }

    fn is_empty(&self) -> bool {
        !self.document && self.regions.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Edit,
    Undo,
    Redo,
}

/// One of the user's steps, for the app to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalChange {
    /// The undo step's transaction id.
    pub step: u64,
    pub kind: ChangeKind,
    pub regions: Vec<Region>,
}

/// What [`Session::apply_remote`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    pub region: Region,
    /// `false` when the presentation already had that content.
    pub changed: bool,
}

#[derive(Clone, Debug)]
struct Step {
    regions: Vec<Region>,
    /// [`LiveState::epoch`] when the step was taken (or last done or undone).
    epoch: u64,
}

/// A presentation's co-editing state.
#[derive(Clone, Debug)]
pub struct LiveState {
    /// Bumped by every change of someone else's (and by a refused change taken back).
    epoch: u64,
    /// When someone else last changed each region.
    touched: HashMap<Region, u64>,
    /// When someone else last replaced the whole presentation.
    everything: u64,
    steps: HashMap<u64, Step>,
    outbox: Vec<LocalChange>,
    blocked: Blocked,
    /// The slides the room has, as far as this side knows: others travel whole with `slides`.
    room_slides: HashSet<SlideId>,
    skipped: usize,
    /// The last change refused because someone else holds its region.
    refused: Option<Region>,
}

impl LiveState {
    fn new(doc: &Presentation) -> Self {
        LiveState {
            epoch: 0,
            touched: HashMap::new(),
            everything: 0,
            steps: HashMap::new(),
            outbox: Vec::new(),
            blocked: Blocked::default(),
            room_slides: doc.slides.iter().map(|s| s.id).collect(),
            skipped: 0,
            refused: None,
        }
    }

    /// Steps from before live mode count as changing everything.
    fn step(&self, transaction: u64) -> Step {
        self.steps.get(&transaction).cloned().unwrap_or(Step { regions: vec![Region::Document], epoch: 0 })
    }

    /// Someone else changed one of the step's regions since it was taken.
    fn is_stale(&self, step: &Step) -> bool {
        self.everything > step.epoch
            || step.regions.iter().any(|r| *r == Region::Document && self.epoch > step.epoch)
            || step.regions.iter().any(|r| self.touched.get(r).is_some_and(|e| *e > step.epoch))
    }

    fn region_is_stale(&self, region: Region, epoch: u64) -> bool {
        self.is_stale(&Step { regions: vec![region], epoch })
    }

    fn record(&mut self, step: u64, kind: ChangeKind, regions: Vec<Region>) {
        self.steps.insert(step, Step { regions: regions.clone(), epoch: self.epoch });
        if !regions.is_empty() {
            self.outbox.push(LocalChange { step, kind, regions });
        }
    }
}

/// Before the undo step `step` that took `st` from `before` to its current presentation lands.
pub(crate) fn record(st: &mut DocState, before: &Presentation, step: u64) {
    let Some(live) = st.live.as_mut() else { return };
    let regions = diff::changed(before, &st.doc);
    live.record(step, ChangeKind::Edit, regions);
}

/// After a command: a change to a region someone else holds is taken back at once (with the
/// interaction or composition it belongs to). Returns the region refused.
pub(crate) fn refuse_blocked(st: &mut DocState, old: &Arc<Presentation>, old_selection: &crate::Selection) -> Option<Region> {
    let live = st.live.as_ref()?;
    if live.blocked.is_empty() || Arc::ptr_eq(old, &st.doc) {
        return None;
    }
    let (base, selection) = match (&st.composition, &st.interaction) {
        (Some(c), _) => (c.before.clone(), c.before_selection.clone()),
        (None, Some(i)) => (i.doc.clone(), i.selection.clone()),
        (None, None) => (old.clone(), old_selection.clone()),
    };
    let refused = diff::changed(&base, &st.doc).into_iter().find(|r| live.blocked.blocks(r))?;
    if let Some(live) = st.live.as_mut() {
        live.refused = Some(refused);
    }
    st.doc = base;
    st.selection = selection;
    st.composition = None;
    st.interaction = None;
    st.revision += 1;
    Some(refused)
}

/// Regions applied in an order where each finds what it needs: the whole presentation, then
/// which slides there are, then slides and shapes.
fn ordered(regions: &[Region]) -> Vec<Region> {
    let mut out = regions.to_vec();
    out.sort_by_key(|r| match r {
        Region::Document => 0,
        Region::Slides => 1,
        Region::Slide(_) => 2,
        Region::Shape(..) => 3,
    });
    out.dedup();
    out
}

/// `target` with `regions` as `source` (an earlier or later state of it) has them.
fn copy_regions(target: &Presentation, source: &Presentation, regions: &[Region]) -> Result<Presentation, LiveError> {
    let mut next = target.clone();
    for region in ordered(regions) {
        let known: HashSet<SlideId> = next.slides.iter().map(|s| s.id).collect();
        let text = payload::build(source, region, &|id| known.contains(&id))?;
        payload::apply(&mut next, region, &payload::parse(&text)?)?;
    }
    Ok(next)
}

fn locked(region: Region) -> EngineError {
    EngineError::RegionLocked(region.to_string())
}

impl Session {
    fn live_doc(&mut self, uid: u64) -> Result<&mut DocState, LiveError> {
        self.document_mut(uid).ok_or(LiveError::NoDocument)
    }

    /// Turn co-editing on or off for presentation `uid`.
    pub fn set_live(&mut self, uid: u64, on: bool) -> Result<(), LiveError> {
        let st = self.live_doc(uid)?;
        st.live = on.then(|| LiveState::new(&st.doc));
        Ok(())
    }

    pub fn is_live(&self, uid: u64) -> bool {
        self.documents().iter().any(|d| d.uid == uid && d.live.is_some())
    }

    /// The regions other people hold now; changes to them are refused.
    pub fn set_live_blocked(&mut self, uid: u64, blocked: Blocked) {
        if let Some(live) = self.document_mut(uid).and_then(|st| st.live.as_mut()) {
            live.blocked = blocked;
        }
    }

    /// The user's steps since the last call, in order.
    pub fn take_live_changes(&mut self, uid: u64) -> Vec<LocalChange> {
        self.document_mut(uid).and_then(|st| st.live.as_mut()).map(|l| std::mem::take(&mut l.outbox)).unwrap_or_default()
    }

    /// How many undo or redo steps were skipped since the last call.
    pub fn take_live_skipped(&mut self, uid: u64) -> usize {
        self.document_mut(uid).and_then(|st| st.live.as_mut()).map(|l| std::mem::take(&mut l.skipped)).unwrap_or(0)
    }

    /// The region of the last change refused because someone else holds it, since the last call.
    pub fn take_live_refused(&mut self, uid: u64) -> Option<Region> {
        self.document_mut(uid).and_then(|st| st.live.as_mut()).and_then(|l| l.refused.take())
    }

    /// Region `region` as it is now, to send.
    pub fn live_payload(&mut self, uid: u64, region: &Region) -> Result<String, LiveError> {
        let st = self.live_doc(uid)?;
        let doc = st.doc.clone();
        let live = st.live.as_mut().ok_or(LiveError::NotLive)?;
        let room = &live.room_slides;
        let text = payload::build(&doc, *region, &|id| room.contains(&id))?;
        if matches!(region, Region::Slides | Region::Document) {
            live.room_slides.extend(doc.slides.iter().map(|s| s.id));
        }
        Ok(text)
    }

    /// Someone else's region: applied exactly, nothing else touched, not an undo step; the
    /// current slide stays the same slide.
    pub fn apply_remote(&mut self, uid: u64, region: &str, payload: &str) -> Result<Applied, LiveError> {
        // No editor sends more; reading a larger one would only cost memory.
        if payload.len() > payload::MAX_PAYLOAD_BYTES {
            return Err(LiveError::TooLarge);
        }
        let region = Region::parse(region).ok_or_else(|| payload::malformed("the region"))?;
        let parsed = payload::parse(payload)?;
        let st = self.live_doc(uid)?;
        if st.live.is_none() {
            return Err(LiveError::NotLive);
        }
        let prev = st.doc.clone();
        let mut next = (*prev).clone();
        // Catches only natively: the web build aborts on a panic, so the apply path must not panic.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| payload::apply(&mut next, region, &parsed)))
            .map_err(|p| LiveError::Apply(crate::guard::panic_message(p.as_ref())))??;
        let touched = diff::changed(&prev, &next);
        if touched.is_empty() {
            return Ok(Applied { region, changed: false });
        }
        // What the user is in the middle of starts from the new content, so its step stays theirs.
        // A whole presentation is unpacked once: the bases take the result, keeping their ids.
        let rebase = |base: &Presentation| -> Option<Presentation> {
            if region == Region::Document {
                return Some(Presentation { ids: base.ids.clone(), ..next.clone() });
            }
            let mut base = base.clone();
            payload::apply(&mut base, region, &parsed).is_ok().then_some(base)
        };
        if let Some(i) = st.interaction.as_mut()
            && let Some(base) = rebase(&i.doc)
        {
            i.doc = Arc::new(base);
        }
        if let Some(c) = st.composition.as_mut()
            && let Some(base) = rebase(&c.before)
        {
            c.before = Arc::new(base);
        }
        let current = prev.slides.get(st.selection.slide).map(|s| s.id);
        st.doc = Arc::new(next);
        if let Some(i) = current.and_then(|id| st.doc.slide_index(id)) {
            st.selection.slide = i;
        }
        if st
            .selection
            .text
            .as_ref()
            .is_some_and(|t| touched.iter().any(|r| matches!(r, Region::Shape(_, s) if *s == t.shape) || *r == Region::Document))
        {
            st.selection.text = None;
        }
        crate::clamp_selection(st);
        st.revision += 1;
        let doc = st.doc.clone();
        if let Some(live) = st.live.as_mut() {
            live.epoch += 1;
            let epoch = live.epoch;
            if region == Region::Document {
                live.everything = epoch;
            }
            if matches!(region, Region::Slides | Region::Document) {
                live.room_slides.extend(doc.slides.iter().map(|s| s.id));
            }
            for r in touched.into_iter().chain(std::iter::once(region)) {
                live.touched.insert(r, epoch);
            }
        }
        Ok(Applied { region, changed: true })
    }

    /// Take back a change the room refused (its lock was denied): its regions go back to how they
    /// were before it, except those someone else changed since, and the step leaves the history.
    pub fn revert_live_change(&mut self, uid: u64, change: &LocalChange) -> Result<(), LiveError> {
        let st = self.live_doc(uid)?;
        let Some(live) = st.live.as_mut() else { return Ok(()) };
        live.outbox.retain(|c| !(c.step == change.step && c.kind == change.kind));
        let step = live.step(change.step);
        let epoch = live.epoch;
        let fresh: Vec<Region> = step.regions.iter().copied().filter(|r| !live.region_is_stale(*r, step.epoch)).collect();
        // An edit or a redo sits on the undo stack (its snapshot is the state before it); an undo
        // on the redo stack (its snapshot is the state after the step).
        let on_undo = change.kind != ChangeKind::Undo;
        let stack = if on_undo { &mut st.history.undo } else { &mut st.history.redo };
        let Some(at) = stack.iter().rposition(|e| e.transaction_id == change.step) else { return Ok(()) };
        let swapped_back = at + 1 == stack.len() && step.epoch == epoch;
        let before = st.doc.clone();
        if swapped_back {
            st.doc = stack[at].doc.clone();
            st.selection = stack[at].selection.clone();
        } else if !fresh.is_empty() {
            st.doc = Arc::new(copy_regions(&st.doc, &stack[at].doc, &fresh)?);
        }
        let entry = stack.remove(at);
        // An undone undo goes back to the undo stack: the step is still the user's.
        if !on_undo {
            st.history.undo.push(HistoryEntry { doc: before, selection: st.selection.clone(), ..entry });
        }
        if let Some(live) = st.live.as_mut() {
            if !swapped_back {
                live.epoch += 1;
            }
            if on_undo {
                live.steps.remove(&change.step);
            } else if let Some(s) = live.steps.get_mut(&change.step) {
                s.epoch = live.epoch;
            }
        }
        st.interaction = None;
        crate::clamp_selection(st);
        st.revision += 1;
        Ok(())
    }
}

/// Undo or redo in live mode: skip the user's steps someone else changed since, swap whole
/// states while nobody else changed anything since the step, and otherwise put back only the
/// step's regions.
pub(crate) fn history(s: &mut Session, redo: bool) -> crate::Result<Value> {
    let st = s.doc_mut()?;
    let Some(live) = st.live.as_mut() else { return Ok(json!({})) };
    let (from, to) = if redo { (&mut st.history.redo, &mut st.history.undo) } else { (&mut st.history.undo, &mut st.history.redo) };
    let step = loop {
        let Some(top) = from.last() else { return Ok(json!({})) };
        let step = live.step(top.transaction_id);
        if live.is_stale(&step) {
            let tid = top.transaction_id;
            from.pop();
            live.steps.remove(&tid);
            live.skipped += 1;
            continue;
        }
        if let Some(r) = step.regions.iter().copied().find(|r| live.blocked.blocks(r)) {
            live.refused = Some(r);
            return Err(locked(r));
        }
        break step;
    };
    let Some(e) = from.pop() else { return Ok(json!({})) };
    let whole = step.epoch == live.epoch;
    let next = if whole {
        e.doc.clone()
    } else {
        match copy_regions(&st.doc, &e.doc, &step.regions) {
            Ok(next) => Arc::new(next),
            Err(err) => {
                from.push(e);
                return Err(EngineError::Other(err.to_string()));
            }
        }
    };
    to.push(HistoryEntry {
        transaction_id: e.transaction_id,
        timestamp: e.timestamp,
        label: e.label.clone(),
        doc: st.doc.clone(),
        selection: st.selection.clone(),
    });
    st.doc = next;
    if whole {
        st.selection = e.selection;
    }
    st.interaction = None;
    live.record(e.transaction_id, if redo { ChangeKind::Redo } else { ChangeKind::Undo }, step.regions);
    crate::clamp_selection(st);
    st.revision += 1;
    Ok(json!({"label": e.label, "transaction_id": e.transaction_id, "timestamp": e.timestamp}))
}
