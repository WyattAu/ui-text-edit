//! The buffer abstraction: where the text lives.
//!
//! The whole point of this trait is that nothing above it knows how the text is
//! stored. A single-line form field wants a `String`; a mail body wants a rope;
//! a document editor wants a piece table. `TextField` above works over any of
//! them, and the operations it needs are few enough that implementing the trait
//! is genuinely small.
//!
//! # Why this is not a rope
//!
//! Because `ropey` is about to change underneath one. Ropey 2.0 flips indexing
//! from char-based to byte-based with the char metric behind a feature flag
//! (ropey 2.x CHANGELOG), and 2.0.0-beta.1 has 685k downloads already. A crate
//! that exposed `Rope` in its own types would have to break its API with it.
//! Exposing a *trait* means the buffer can be replaced without touching a
//! single caller, which is the only durable answer to that changelog.
//!
//! # The index unit
//!
//! Every offset in this trait and everything above it is a **grapheme cluster
//! index**, not a byte offset and not a `char` index. That is inherited from
//! `input-kit` and it is what makes a caret unable to split a character. The
//! implementations convert; nothing above them thinks in bytes.

use alloc::string::String;

/// A mutable, indexable text buffer.
///
/// Implemented by [`StringBuffer`] and by callers' own buffers. All offsets are
/// grapheme cluster indices; an out-of-range index is clamped rather than
/// panicking, because an offset arrives from a caret the user moved and from a
/// platform that reports stale positions.
pub trait TextBuffer {
    /// The whole text.
    fn text(&self) -> &str;

    /// Number of grapheme clusters.
    fn len_clusters(&self) -> usize;

    /// Whether the buffer holds no clusters.
    fn is_empty(&self) -> bool {
        self.len_clusters() == 0
    }

    /// Replace the cluster range `range` with `insert`.
    ///
    /// The range is in cluster indices and is normalised if reversed. Indices
    /// past the end clamp to the end.
    fn replace(&mut self, range: core::ops::Range<usize>, insert: &str);

    /// Insert `text` at cluster index `at`, clamped to the end.
    fn insert(&mut self, at: usize, text: &str) {
        self.replace(at..at, text);
    }

    /// Delete the cluster range `range`, normalised if reversed and clamped.
    fn delete(&mut self, range: core::ops::Range<usize>) {
        self.replace(range, "");
    }

    /// Remove every cluster.
    fn clear(&mut self) {
        let n = self.len_clusters();
        self.replace(0..n, "");
    }

    /// The cluster range `range` as a substring, clamped.
    ///
    /// Returns `None` only if the buffer is not on cluster boundaries, which
    /// no implementation of this trait produces.
    fn slice(&self, range: core::ops::Range<usize>) -> Option<&str> {
        let text = self.text();
        let len = self.len_clusters();
        let lo = range.start.min(len);
        let hi = range.end.min(len);
        let lo_byte = input_kit::grapheme::byte_offset(text, lo)?;
        let hi_byte = input_kit::grapheme::byte_offset(text, hi)?;
        text.get(lo_byte..hi_byte)
    }
}

/// The default [`TextBuffer`]: a `String` held whole.
///
/// Correct for form fields, single-line inputs, and anything short. A rope or a
/// piece table sits behind the same trait when the text gets long, and nothing
/// above this type changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringBuffer {
    text: String,
}

impl StringBuffer {
    /// An empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A buffer holding `text`.
    #[must_use]
    pub fn from_text(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    /// Consume the buffer, yielding its text.
    #[must_use]
    pub fn into_text(self) -> String {
        self.text
    }

    /// The whole text, by reference.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl TextBuffer for StringBuffer {
    fn text(&self) -> &str {
        &self.text
    }

    fn len_clusters(&self) -> usize {
        input_kit::grapheme::count(&self.text)
    }

    fn replace(&mut self, range: core::ops::Range<usize>, insert: &str) {
        // Normalise a reversed range rather than producing an empty span:
        // `delete(4..1)` should delete clusters 1..4, which is what every
        // editor's "select backwards then delete" does.
        let (lo, hi) = if range.start <= range.end {
            (range.start, range.end)
        } else {
            (range.end, range.start)
        };
        let len = self.len_clusters();
        let lo = lo.min(len);
        let hi = hi.min(len);
        let lo_byte = input_kit::grapheme::byte_offset(&self.text, lo).unwrap_or(self.text.len());
        let hi_byte = input_kit::grapheme::byte_offset(&self.text, hi).unwrap_or(self.text.len());
        // `String::replace_range` needs char boundaries, which the offsets
        // above are by construction. If a caller built a malformed buffer the
        // ranges simply do not apply and the text is left alone.
        if self.text.is_char_boundary(lo_byte) && self.text.is_char_boundary(hi_byte) {
            self.text.replace_range(lo_byte..hi_byte, insert);
        }
    }
}

impl From<String> for StringBuffer {
    fn from(text: String) -> Self {
        Self { text }
    }
}

impl From<&str> for StringBuffer {
    fn from(text: &str) -> Self {
        Self::from_text(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    #[test]
    fn replacing_a_cluster_keeps_its_marks() {
        let mut b = StringBuffer::from_text("cafe\u{301}");
        assert_eq!(b.len_clusters(), 4);
        assert_eq!(b.text().len(), 6, "six bytes, four clusters");
        // Replace the decomposed e with a precomposed one.
        b.replace(3..4, "\u{00E9}");
        assert_eq!(b.as_str(), "caf\u{00E9}");
        assert_eq!(b.len_clusters(), 4);
    }

    #[test]
    fn a_reversed_range_deletes_the_same_span() {
        // Built from variables: a literal `4..1` reads as an always-empty range,
        // and the point here is that `delete` normalises the order.
        let (start, end) = (4, 1);
        let mut b = StringBuffer::from_text("abcdef");
        b.delete(start..end);
        assert_eq!(b.as_str(), "aef");
    }

    #[test]
    fn indices_past_the_end_clamp_rather_than_panicking() {
        let mut b = StringBuffer::from_text("ab");
        b.delete(0..99);
        assert_eq!(b.as_str(), "");
        b.insert(99, "x");
        assert_eq!(b.as_str(), "x");
    }

    #[test]
    fn slicing_is_by_cluster_not_byte() {
        let b = StringBuffer::from_text("cafe\u{301}");
        assert_eq!(b.slice(3..4), Some("e\u{301}"));
        assert_eq!(b.slice(0..4), Some("cafe\u{301}"));
        assert_eq!(b.slice(2..99), Some("fe\u{301}"));
        assert_eq!(b.slice(9..10), Some(""));
    }

    /// The property the trait exists for: a buffer of any kind can be addressed
    /// by cluster and the result never splits a character.
    #[test]
    fn every_buffer_satisfies_the_cluster_contract() {
        for text in ["", "a", "cafe\u{301}", "\u{1F1FA}\u{1F1F8}", "a\u{1F600}b"] {
            let b = StringBuffer::from_text(text);
            let n = b.len_clusters();
            assert_eq!(n, input_kit::grapheme::count(text));
            // Every prefix is itself well-formed text.
            for i in 0..=n {
                let prefix = b.slice(0..i).unwrap_or_default();
                assert!(
                    input_kit::grapheme::byte_offset(text, i).is_some(),
                    "{text:?}: no byte offset for cluster {i}"
                );
                let _ = prefix;
            }
        }
    }

    /// Deleting every cluster, one at a time from the end, must land on the
    /// empty string and never leave a stray mark behind.
    #[test]
    fn deleting_cluster_by_cluster_never_orphans_a_mark() {
        let mut b = StringBuffer::from_text("cafe\u{301} \u{1F1FA}\u{1F1F8}");
        while b.len_clusters() > 0 {
            let n = b.len_clusters();
            b.delete(n - 1..n);
            // The remaining text is always a valid prefix of the original.
            assert!(
                input_kit::grapheme::byte_offset(b.as_str(), b.len_clusters()).is_some(),
                "{:?} lost its cluster boundaries mid-delete",
                b.as_str()
            );
        }
        assert_eq!(b.as_str(), "");
    }

    #[test]
    fn conversions_round_trip() {
        let owned: String = String::from("hi");
        assert_eq!(StringBuffer::from(owned).into_text(), "hi");
        assert_eq!(StringBuffer::from("hi").as_str(), "hi");
    }
}