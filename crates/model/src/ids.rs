//! Where ids created while editing come from (ERPCons co-editing): mixed with the editing
//! session's client prefix, so two people adding slides or shapes to one presentation at once
//! don't pick the same id. Reading a file keeps its ids and numbers what it must the same way in
//! every session.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::{Presentation, walk};

/// Ids per block: a session's first block is its prefix itself.
const BLOCK_BITS: u32 = 6;
/// Created ids stay below this: the PPTX reader keeps shape ids under 2^30 as they are, and slide
/// ids must stay under 2^31.
const LIMIT: u32 = 1 << 30;
/// Slide ids start here (`ST_SlideId`); no created id goes lower.
const MIN_ID: u32 = 256;

#[derive(Debug)]
struct Shared {
    prefix: AtomicU32,
    seq: AtomicU64,
}

/// The editing session's id source, shared by every copy of a presentation it edits (undo steps
/// included), so its counter never runs back. Empty for a presentation no session edits (reading
/// a file, building a new one): ids then come from the presentation's own counter. Never saved.
#[derive(Clone, Debug, Default)]
pub struct IdSource(Option<Arc<Shared>>);

impl PartialEq for IdSource {
    /// Where ids come from is no part of the presentation.
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl IdSource {
    /// A source for `prefix` (24 bits; higher bits are dropped).
    pub fn new(prefix: u32) -> Self {
        Self(Some(Arc::new(Shared { prefix: AtomicU32::new(prefix & 0xff_ffff), seq: AtomicU64::new(0) })))
    }

    /// Mix `prefix` into the ids created from now on; earlier ones keep theirs.
    pub fn set_prefix(&self, prefix: u32) {
        let Some(s) = &self.0 else { return };
        let prefix = prefix & 0xff_ffff;
        // The shell says it again after a reconnect: carry on rather than repeat ids.
        if s.prefix.swap(prefix, Ordering::Relaxed) != prefix {
            // A new prefix starts at its own first block.
            s.seq.store(0, Ordering::Relaxed);
        }
    }

    pub fn prefix(&self) -> Option<u32> {
        self.0.as_ref().map(|s| s.prefix.load(Ordering::Relaxed))
    }

    /// The next candidate: the first 64 ids of a prefix are `prefix << 6 | n`, so two prefixes
    /// never share them; later blocks are spread by a hash of the prefix.
    fn next(&self) -> Option<u32> {
        let s = self.0.as_ref()?;
        let prefix = s.prefix.load(Ordering::Relaxed);
        let n = s.seq.fetch_add(1, Ordering::Relaxed);
        let block = match n >> BLOCK_BITS {
            0 => prefix,
            k => (mix((u64::from(prefix) << 32) ^ k) & 0xff_ffff) as u32,
        };
        Some((block << BLOCK_BITS) | (n & ((1 << BLOCK_BITS) - 1)) as u32)
    }
}

/// splitmix64's finaliser.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Presentation {
    /// Whether a slide, layout, master, medium or any shape has `id`.
    pub fn uses_id(&self, id: u32) -> bool {
        let in_shapes = |shapes: &[crate::Shape]| {
            let mut found = false;
            walk(shapes, &mut |sh, _| found |= sh.id.0 == id);
            found
        };
        self.slides.iter().any(|s| s.id.0 == id || in_shapes(&s.shapes))
            || self
                .masters
                .iter()
                .chain(&self.notes_master)
                .chain(&self.handout_master)
                .any(|m| m.id.0 == id || in_shapes(&m.shapes) || m.layouts.iter().any(|l| l.id.0 == id || in_shapes(&l.shapes)))
            || self.media.iter().any(|m| m.id.0 == id)
    }

    /// A created id from the session's source, unused in this presentation; `None` without a
    /// source.
    pub(crate) fn alloc_session_id(&mut self) -> Option<u32> {
        // Bounded: blocks spread over 2^24 values, so a free id turns up within a few tries.
        for _ in 0..4096 {
            let id = self.ids.next()?;
            if (MIN_ID..LIMIT).contains(&id) && !self.uses_id(id) {
                self.next_id = self.next_id.max(id);
                return Some(id);
            }
        }
        None
    }
}
