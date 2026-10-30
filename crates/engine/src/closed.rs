//! Presentations closed in this session, kept so `file.reopenClosed` (Ctrl+Shift+T) can bring
//! the last one back as it was, unsaved changes included: closing with Don't Save by mistake
//! loses nothing until the entry ages out. Native `.deckcraft` bytes, like a draft.

use std::collections::VecDeque;

use crate::DocState;

/// Most closed presentations kept.
pub const MAX_CLOSED: usize = 10;
/// Most bytes kept for them together; the oldest go first.
pub const MAX_CLOSED_BYTES: usize = 64 * 1024 * 1024;

/// A closed presentation as it was when it was closed.
#[derive(Clone, Debug)]
pub struct ClosedDoc {
    pub title: String,
    pub path: Option<String>,
    /// It had changes that weren't saved: it reopens unsaved.
    pub unsaved: bool,
    pub bytes: Vec<u8>,
}

/// The closed presentations, most recent last.
#[derive(Debug)]
pub struct ClosedDocs {
    entries: VecDeque<ClosedDoc>,
    max_entries: usize,
    max_bytes: usize,
}

impl Default for ClosedDocs {
    fn default() -> Self {
        Self::with_caps(MAX_CLOSED, MAX_CLOSED_BYTES)
    }
}

impl ClosedDocs {
    pub fn with_caps(max_entries: usize, max_bytes: usize) -> Self {
        ClosedDocs { entries: VecDeque::new(), max_entries, max_bytes }
    }

    /// Keeps `d` as it is now. An untouched placeholder loses nothing by closing and isn't kept;
    /// neither is one that can't be written, is larger than the whole allowance or isn't
    /// [`reopenable`](DocState::reopenable).
    pub fn record(&mut self, d: &DocState) {
        if d.is_untouched_placeholder() || !d.reopenable {
            return;
        }
        match deckcraft_format::save(&d.doc) {
            Ok(bytes) => self.push(ClosedDoc { title: d.title(), path: d.path.clone(), unsaved: d.is_dirty(), bytes }),
            Err(e) => log::warn!("couldn't keep a closed presentation: {e}"),
        }
    }

    pub fn push(&mut self, c: ClosedDoc) {
        if c.bytes.len() > self.max_bytes {
            log::info!("a closed presentation of {} bytes is too large to keep for reopening", c.bytes.len());
            return;
        }
        self.entries.push_back(c);
        while self.entries.len() > self.max_entries || self.total_bytes() > self.max_bytes {
            self.entries.pop_front();
        }
    }

    /// The most recently closed one.
    pub fn last(&self) -> Option<&ClosedDoc> {
        self.entries.back()
    }

    pub fn pop(&mut self) -> Option<ClosedDoc> {
        self.entries.pop_back()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn total_bytes(&self) -> usize {
        self.entries.iter().map(|c| c.bytes.len()).sum()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn entry(title: &str, len: usize) -> ClosedDoc {
        ClosedDoc { title: title.into(), path: None, unsaved: true, bytes: vec![0; len] }
    }

    fn titles(c: &ClosedDocs) -> Vec<String> {
        c.entries.iter().map(|e| e.title.clone()).collect()
    }

    #[test]
    fn keeps_the_ten_most_recent() {
        let mut c = ClosedDocs::default();
        for i in 0..12 {
            c.push(entry(&format!("p{i}"), 10));
        }
        assert_eq!(c.len(), MAX_CLOSED);
        assert_eq!(titles(&c).first().map(String::as_str), Some("p2"), "the oldest went first");
        assert_eq!(c.last().unwrap().title, "p11");
        assert_eq!(c.pop().unwrap().title, "p11");
        assert_eq!(c.last().unwrap().title, "p10");
    }

    #[test]
    fn a_document_that_is_not_reopenable_is_not_kept() {
        let session = crate::Session::with_new();
        let mut d = session.active().unwrap().clone();
        d.doc = std::sync::Arc::new((*d.doc).clone());
        let mut c = ClosedDocs::default();
        c.record(&d);
        assert_eq!(c.len(), 1);
        d.reopenable = false;
        c.record(&d);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn the_byte_allowance_drops_the_oldest() {
        let mut c = ClosedDocs::with_caps(MAX_CLOSED, 100);
        c.push(entry("a", 40));
        c.push(entry("b", 40));
        c.push(entry("c", 40));
        assert_eq!(titles(&c), ["b", "c"]);
        assert!(c.total_bytes() <= 100);
        // One larger than the whole allowance isn't kept, and doesn't push the others out.
        c.push(entry("huge", 101));
        assert_eq!(titles(&c), ["b", "c"]);
    }

    #[test]
    fn the_default_allowance_is_64_mib() {
        let mut c = ClosedDocs::default();
        for i in 0..4 {
            c.push(entry(&format!("p{i}"), 20 * 1024 * 1024));
        }
        assert_eq!(titles(&c), ["p1", "p2", "p3"], "four 20 MiB presentations don't fit in 64 MiB");
        c.push(entry("too big", MAX_CLOSED_BYTES + 1));
        assert_eq!(c.len(), 3);
    }
}
