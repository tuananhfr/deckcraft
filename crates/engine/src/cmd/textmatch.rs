//! Text matching for Find and Replace that treats canonically equivalent text as equal: "ệ" typed
//! precomposed (NFC) and "e" + U+0323 + U+0302 pasted from another app (NFD) are the same letter.
//! Accents still count: "a" never matches "ả".

use unicode_normalization::char::{canonical_combining_class, decompose_canonical};

/// Text in canonical decomposition (NFD), case-folded on request, cut into clusters: a starter
/// and the combining marks after it. A match starts and ends on cluster boundaries, which map back
/// exactly to char offsets of the original text, however much longer the decomposed form is.
pub(crate) struct Folded {
    pub(crate) chars: Vec<char>,
    /// Per decomposed offset (and one past the end): the original char offset when a cluster
    /// starts there.
    boundary: Vec<Option<usize>>,
}

impl Folded {
    pub(crate) fn new(text: impl IntoIterator<Item = char>, match_case: bool) -> Folded {
        let mut chars = Vec::new();
        let mut boundary = Vec::new();
        let mut orig = 0;
        for c in text {
            let first = chars.len();
            decompose_canonical(c, |d| chars.push(if match_case { d } else { fold(d) }));
            boundary.resize(chars.len(), None);
            if let Some(b) = boundary.get_mut(first)
                && (first == 0 || chars.get(first).is_some_and(|d| canonical_combining_class(*d) == 0))
            {
                *b = Some(orig);
            }
            orig += 1;
        }
        boundary.push(Some(orig));
        canonical_order(&mut chars);
        Folded { chars, boundary }
    }

    /// Non-overlapping matches of `needle` (already folded), left to right, as original char
    /// ranges; `accept` can refuse a range of the original text.
    pub(crate) fn matches(&self, needle: &[char], whole_words: bool, accept: impl Fn(usize, usize) -> bool) -> Vec<(usize, usize)> {
        let n = needle.len();
        let mut out = Vec::new();
        let mut i = 0;
        while n > 0 && i + n <= self.chars.len() {
            let (Some(start), Some(&Some(end))) = (self.boundary.get(i).copied().flatten(), self.boundary.get(i + n)) else {
                i += 1;
                continue;
            };
            let hit =
                self.chars.get(i..i + n) == Some(needle) && (!whole_words || (!self.word_before(i) && !self.word_at(i + n))) && accept(start, end);
            if hit {
                out.push((start, end));
                i += n;
            } else {
                i += 1;
            }
        }
        out
    }

    /// Whether the cluster starting at `i` is part of a word, judged by its base letter.
    fn word_at(&self, i: usize) -> bool {
        self.chars.get(i).is_some_and(|c| c.is_alphanumeric() || *c == '_')
    }

    fn word_before(&self, i: usize) -> bool {
        let start = self.boundary.get(..i).and_then(|b| b.iter().rposition(Option::is_some));
        start.is_some_and(|s| self.word_at(s))
    }
}

/// Lower case as one char (`İ`, which lowers to two, stays itself).
fn fold(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

/// Unicode canonical ordering: each run of combining marks sorted, stably, by combining class, so
/// "ậ" as "â" + dot below and as "ạ" + circumflex decompose alike. A run never spans a starter, so
/// no mark leaves its cluster.
fn canonical_order(chars: &mut [char]) {
    let mut i = 0;
    while i < chars.len() {
        let run = chars.get(i..).map_or(0, |r| r.iter().take_while(|c| canonical_combining_class(**c) != 0).count());
        if let Some(r) = chars.get_mut(i..i + run) {
            r.sort_by_key(|c| canonical_combining_class(*c));
        }
        i += run.max(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NFC: &str = "Thiết kế nhà ở";
    const NFD: &str = "Thie\u{0302}\u{0301}t ke\u{0302}\u{0301} nha\u{0300} o\u{031B}\u{0309}";

    fn find(hay: &str, needle: &str, whole_words: bool) -> Vec<(usize, usize)> {
        Folded::new(hay.chars(), false).matches(&Folded::new(needle.chars(), false).chars, whole_words, |_, _| true)
    }

    #[test]
    fn nfc_and_nfd_find_each_other_with_original_offsets() {
        assert_eq!(find(NFD, "thiết kế", false), vec![(0, 12)], "decomposed: \"thiết\" is 7 chars, \"kế\" 4");
        assert_eq!(find(NFC, "THIE\u{0302}\u{0301}T KE\u{0302}\u{0301}", false), vec![(0, 8)]);
        assert_eq!(find(NFD, "ở", false), vec![(18, 21)], "o, horn, hook above");
        assert_eq!(find(NFC, "o\u{031B}\u{0309}", false), vec![(13, 14)]);
        assert!(find(NFC, "o\u{0309}", false).is_empty(), "\"ỏ\" is not \"ở\"");
    }

    #[test]
    fn accents_still_count() {
        assert!(find(NFD, "nha", false).is_empty(), "\"nha\" is not \"nhà\"");
        assert!(find(NFC, "nha", false).is_empty());
        assert!(find("ả", "a", false).is_empty());
        assert!(find("a\u{0309}", "a", false).is_empty());
        assert_eq!(find("a ả", "a", false), vec![(0, 1)]);
        assert!(find("ế", "ê", false).is_empty(), "a mark short");
    }

    #[test]
    fn marks_in_either_order_are_the_same_letter() {
        // "ậ": precomposed, "â" + dot below, "ạ" + circumflex, base + marks in non-canonical order.
        for hay in ["ậ", "â\u{0323}", "ạ\u{0302}", "a\u{0302}\u{0323}", "a\u{0323}\u{0302}"] {
            assert_eq!(find(hay, "ậ", false).len(), 1, "{hay:?}");
            assert_eq!(find("ậ", hay, false).len(), 1, "{hay:?}");
        }
    }

    #[test]
    fn whole_words_judge_a_decomposed_letter_by_its_base() {
        assert!(find("be\u{0301}ta", "ta", true).is_empty(), "\"ta\" inside \"béta\"");
        assert_eq!(find("be\u{0301} ta", "ta", true), vec![(4, 6)]);
        assert_eq!(find("nha\u{0300} ở", "nhà", true), vec![(0, 4)]);
        assert!(find("nha\u{0300}x", "nhà", true).is_empty());
    }

    #[test]
    fn match_case_keeps_case() {
        let hay = Folded::new("Ệ ệ".chars(), true);
        assert_eq!(hay.matches(&Folded::new("e\u{0323}\u{0302}".chars(), true).chars, false, |_, _| true), vec![(2, 3)]);
    }

    #[test]
    fn a_stray_leading_mark_does_not_panic() {
        assert_eq!(find("\u{0301}a", "a", false), vec![(1, 2)]);
        assert_eq!(find("\u{0301}a", "\u{0301}", false), vec![(0, 1)]);
    }
}
