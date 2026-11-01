//! Editing together by region through the office shell's `erpHost` v3 (web only; `docs/erp-host.md`
//! in erpcons-office). The engine says which regions each of the user's steps changed
//! ([`deckcraft_engine::live`]); here the app asks the shell for their locks (`regionWillEdit`),
//! sends their content once granted (`regionChanged`), lets them go when the user leaves them
//! (`regionDidEndEdit`), reports the selection, applies other people's regions (`applyRemote`),
//! takes back a step whose lock was refused, and draws others' locks and selections on the slide.
//! Without v3 (the desktop, a shell without it) [`crate::SlideApp::live`] is `None` and none of this
//! runs.

mod conflict;
mod frame;
mod marks;

#[cfg(test)]
pub(crate) mod tests;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use deckcraft_engine::live::LocalChange;

pub(crate) use marks::paint;

/// A region the user holds stays held this long after its last change, unless still focused.
pub const IDLE_END_MS: f64 = 2_000.0;
/// A focused region (the text being edited, the shape being dragged) is let go after this long
/// without a change, so a selection left alone doesn't lock others out.
pub const FOCUS_IDLE_MS: f64 = 15_000.0;
/// `selectionChanged` goes out at most this often.
pub const SELECTION_MS: f64 = 250.0;
/// Payloads kept to put back when the user keeps their version of a conflict ("Giữ bản của tôi").
const SENT_BYTES: usize = 8 << 20;

/// The shell's side of `erpHost` v3, called by the app.
pub trait RegionHost {
    /// `regionWillEdit(regionId)`; `reply` gets whether the lock was granted (possibly later).
    fn region_will_edit(&self, region: &str, reply: Box<dyn FnOnce(bool)>);
    /// `regionChanged(regionId, payload)`.
    fn region_changed(&self, region: &str, payload: &str);
    /// `regionDidEndEdit(regionId)`.
    fn region_did_end_edit(&self, region: &str);
    /// `selectionChanged(regionId, range)`.
    fn selection_changed(&self, region: &str, range: &str);
    /// Wall-clock milliseconds (`Date.now()` on the web).
    fn now_ms(&self) -> f64;
}

/// A member of the live room (`showPresence`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Member {
    pub id: String,
    pub name: Option<String>,
    pub colour: Option<String>,
    /// This page's own member (`you`).
    pub me: bool,
}

/// One entry of the lock table (`showLocks`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LockView {
    pub region: String,
    pub holder: String,
    pub colour: Option<String>,
    pub name: Option<String>,
    /// The shell says this page holds it.
    pub mine: bool,
}

/// Someone else's selection (`showCursors`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cursor {
    pub author: String,
    pub region: String,
    pub range: String,
    pub name: Option<String>,
    pub colour: Option<String>,
}

/// The answer to `regionConflict`: "Giữ bản của tôi" keeps this page's edit, "Dùng bản mới" the
/// room's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Mine,
    Theirs,
}

impl Choice {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mine => "mine",
            Self::Theirs => "theirs",
        }
    }
}

/// Answers `regionConflict`, once.
pub type ChoiceReply = Box<dyn FnOnce(Choice)>;

/// What the shell called, applied on the next frame.
pub enum LiveEvent {
    ApplyRemote {
        region: String,
        payload: String,
        author: Option<String>,
    },
    Presence(Vec<Member>),
    Locks(Vec<LockView>),
    Cursors(Vec<Cursor>),
    LockDenied(String),
    Leader(bool),
    /// `regionConflict(regionId)`, answered by the user.
    Conflict {
        region: String,
        reply: ChoiceReply,
    },
}

pub type LiveInbox = Rc<RefCell<Vec<LiveEvent>>>;
type Answers = Rc<RefCell<Vec<(String, bool)>>>;

/// A conflict waiting for the user's choice.
struct ConflictPrompt {
    region: String,
    reply: ChoiceReply,
}

/// The app's co-editing state (web, `erpHost` v3).
pub struct Live {
    host: Box<dyn RegionHost>,
    inbox: LiveInbox,
    answers: Answers,
    /// The presentation of the live room: the host document opened last.
    doc: Option<u64>,
    /// Regions the user holds, with when they last changed.
    held: HashMap<String, f64>,
    asking: HashSet<String>,
    /// The user's steps waiting for their locks, in order.
    waiting: Vec<LocalChange>,
    /// Others' locks by region.
    others: HashMap<String, LockView>,
    members: Vec<Member>,
    cursors: Vec<Cursor>,
    leader: bool,
    sent: HashMap<String, String>,
    selection: Option<(String, String)>,
    selection_sent_ms: f64,
    /// The regions the user was about to change last frame.
    intent: HashSet<String>,
    conflicts: Vec<ConflictPrompt>,
}

impl Live {
    pub fn new(host: Box<dyn RegionHost>) -> Self {
        Live {
            host,
            inbox: LiveInbox::default(),
            answers: Answers::default(),
            doc: None,
            held: HashMap::new(),
            asking: HashSet::new(),
            waiting: Vec::new(),
            others: HashMap::new(),
            members: Vec::new(),
            cursors: Vec::new(),
            leader: false,
            sent: HashMap::new(),
            selection: None,
            selection_sent_ms: f64::NEG_INFINITY,
            intent: HashSet::new(),
            conflicts: Vec::new(),
        }
    }

    /// Where the shell's calls queue their events.
    pub fn inbox(&self) -> LiveInbox {
        self.inbox.clone()
    }

    /// Whether the shell says this page leads the room (it saves; informative only here).
    pub fn is_leader(&self) -> bool {
        self.leader
    }

    /// The presentation the room is about.
    pub fn doc(&self) -> Option<u64> {
        self.doc
    }

    /// A display name for member `id`: the lock's or cursor's own, else the room's.
    fn name_of(&self, id: &str, own: Option<&String>) -> Option<String> {
        own.cloned().or_else(|| self.members.iter().find(|m| m.id == id).and_then(|m| m.name.clone())).filter(|n| !n.trim().is_empty())
    }

    fn colour_of(&self, id: &str, own: Option<&String>) -> Option<String> {
        own.cloned().or_else(|| self.members.iter().find(|m| m.id == id).and_then(|m| m.colour.clone()))
    }

    fn me(&self) -> Option<&str> {
        self.members.iter().find(|m| m.me).map(|m| m.id.as_str())
    }

    fn remember_sent(&mut self, region: &str, payload: &str) {
        self.sent.insert(region.to_owned(), payload.to_owned());
        // Over budget the largest go first; a conflict about one of them keeps what this page has.
        while self.sent.values().map(String::len).sum::<usize>() > SENT_BYTES {
            let Some(largest) = self.sent.iter().max_by_key(|(_, p)| p.len()).map(|(r, _)| r.clone()) else { break };
            self.sent.remove(&largest);
        }
    }

    /// Ask the shell for `region`'s lock, unless it is held or asked for already.
    fn ask(&mut self, region: &str) {
        if self.held.contains_key(region) || !self.asking.insert(region.to_owned()) {
            return;
        }
        let answers = self.answers.clone();
        let owned = region.to_owned();
        self.host.region_will_edit(region, Box::new(move |granted| answers.borrow_mut().push((owned, granted))));
    }

    /// Forget the room's state when its presentation changes.
    fn reset(&mut self) {
        for region in self.held.keys() {
            self.host.region_did_end_edit(region);
        }
        self.held.clear();
        self.asking.clear();
        self.waiting.clear();
        self.sent.clear();
        self.selection = None;
        self.intent.clear();
    }
}
