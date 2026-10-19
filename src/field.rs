//! The editing model: caret, selection, and the operations on them.
//!
//! [`TextField`] is a text field, not a code editor. It has one caret, an
//! optional selection anchored at it, and the handful of operations a form
//! field, a mail body, or a single-line input needs. It does not have multiple
//! carets, transactions, a revision tree, undo, or folds: those belong to an
//! editing engine, and the estate does not have one to wrap.
//!
//! Every index here is a grapheme cluster index, inherited from
//! `input_kit`. That is the one decision the whole crate rests on: a caret
//! cannot be placed between a letter and its combining mark, between the two
//! halves of a flag, or inside an emoji ZWJ sequence, because those are one
//! cluster and this type only counts clusters.
//!
//! ```
//! use ui_text_edit::{TextField, StringBuffer};
//!
//! let mut f = TextField::with_caret_at_end(StringBuffer::from_text("hello"));
//! f.select_left(2);                              // the caret is at the end
//! assert_eq!(f.selection_text(), "lo");          // so the last two clusters
//! f.delete_selection();
//! assert_eq!(f.text(), "hel");
//! ```

use crate::buffer::TextBuffer;
use input_kit::grapheme;

/// A caret and its selection anchor, both in cluster indices.
///
/// `anchor` and `caret` may be in either order; that is what makes a
/// backwards selection representable without a direction flag that every
/// operation then has to consult.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Selection {
    /// The fixed end, in clusters.
    pub anchor: usize,
    /// The moving end, in clusters.
    pub caret: usize,
}

impl Selection {
    /// A collapsed caret at `at`.
    #[must_use]
    pub fn caret(at: usize) -> Self {
        Self { anchor: at, caret: at }
    }

    /// The selection from `from` to `to`, in either order.
    ///
    /// Named `between` rather than `range` so that [`Selection::range`], the
    /// accessor that turns a selection into an ascending range, keeps its
    /// name.
    #[must_use]
    pub fn between(from: usize, to: usize) -> Self {
        Self {
            anchor: from,
            caret: to,
        }
    }

    /// Whether there is no selection.
    #[must_use]
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.caret
    }

    /// The selection as an ascending range.
    #[must_use]
    pub fn range(&self) -> core::ops::Range<usize> {
        if self.anchor <= self.caret {
            self.anchor..self.caret
        } else {
            self.caret..self.anchor
        }
    }

    /// Length in clusters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.caret.abs_diff(self.anchor)
    }

    /// Whether the selection spans nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.is_collapsed()
    }
}

/// A text field: a buffer plus a caret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextField<B> {
    buffer: B,
    selection: Selection,
}

impl<B: TextBuffer> TextField<B> {
    /// A field over `buffer`, caret at the start.
    #[must_use]
    pub fn new(buffer: B) -> Self {
        Self {
            buffer,
            selection: Selection::caret(0),
        }
    }

    /// A field over `buffer`, caret at the end.
    #[must_use]
    pub fn with_caret_at_end(buffer: B) -> Self {
        let end = buffer.len_clusters();
        Self {
            buffer,
            selection: Selection::caret(end),
        }
    }

    /// The buffer.
    #[must_use]
    pub fn buffer(&self) -> &B {
        &self.buffer
    }

    /// The buffer, mutably. Mutating it directly may leave the caret out of
    /// range; the next read clamps it.
    pub fn buffer_mut(&mut self) -> &mut B {
        &mut self.buffer
    }

    /// Consume the field, yielding the buffer.
    #[must_use]
    pub fn into_buffer(self) -> B {
        self.buffer
    }

    /// The whole text.
    #[must_use]
    pub fn text(&self) -> &str {
        self.buffer.text()
    }

    /// Length in clusters.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len_clusters()
    }

    /// Whether the field is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// The current selection, clamped to the buffer.
    #[must_use]
    pub fn selection(&self) -> Selection {
        self.clamped(self.selection)
    }

    /// The caret, clamped to the buffer.
    #[must_use]
    pub fn caret(&self) -> usize {
        self.selection().caret
    }

    fn clamped(&self, s: Selection) -> Selection {
        let len = self.buffer.len_clusters();
        Selection {
            anchor: s.anchor.min(len),
            caret: s.caret.min(len),
        }
    }

    /// Set the selection outright, clamped to the buffer.
    ///
    /// A platform reports a caret in its own unit; this is where it becomes a
    /// cluster index. An index past the end clamps rather than panicking,
    /// because a stale caret from a buffer that shrank must not be fatal.
    pub fn set_selection(&mut self, s: Selection) {
        self.selection = self.clamped(s);
    }

    // --- editing ------------------------------------------------------------

    /// Insert `text` at the caret, replacing the selection if there is one.
    ///
    /// Returns `false` for an empty insertion with a collapsed caret, which is
    /// the "nothing happened" case a caller may want to skip.
    pub fn insert(&mut self, text: &str) -> bool {
        let sel = self.selection();
        let range = sel.range();
        self.buffer.replace(range.clone(), text);
        // The caret lands after the inserted text. Counting clusters, not
        // bytes: inserting "é" as two code points moves the caret by one.
        let inserted = grapheme::count(text);
        self.set_selection(Selection::caret(range.start + inserted));
        !text.is_empty() || !sel.is_collapsed()
    }

    /// Insert `text` at an explicit cluster index, leaving the caret put.
    pub fn insert_at(&mut self, at: usize, text: &str) {
        self.buffer.insert(at, text);
    }

    /// Delete the selection. A collapsed caret deletes nothing.
    pub fn delete_selection(&mut self) -> bool {
        let sel = self.selection();
        if sel.is_collapsed() {
            return false;
        }
        let range = sel.range();
        self.buffer.replace(range.clone(), "");
        self.set_selection(Selection::caret(range.start));
        true
    }

    /// Delete the cluster before the caret, or the selection if there is one.
    ///
    /// This is Backspace, and it is the operation that most often corrupts
    /// text in a hand-rolled implementation, because deleting "one character"
    /// by bytes orphans the combining mark. It deletes one *cluster*.
    pub fn backspace(&mut self) -> bool {
        let sel = self.selection();
        if !sel.is_collapsed() {
            return self.delete_selection();
        }
        let at = sel.caret;
        if at == 0 {
            return false;
        }
        self.buffer.delete(at - 1..at);
        self.set_selection(Selection::caret(at - 1));
        true
    }

    /// Delete the cluster at the caret, or the selection if there is one.
    ///
    /// This is Delete (forward).
    pub fn delete_forward(&mut self) -> bool {
        let sel = self.selection();
        if !sel.is_collapsed() {
            return self.delete_selection();
        }
        let at = sel.caret;
        let len = self.buffer.len_clusters();
        if at >= len {
            return false;
        }
        self.buffer.delete(at..at + 1);
        self.set_selection(Selection::caret(at));
        true
    }

    /// Replace the selection with `text`, or insert at the caret if collapsed.
    pub fn replace_selection(&mut self, text: &str) {
        self.delete_selection();
        self.insert(text);
    }

    // --- movement -----------------------------------------------------------

    /// Move the caret one cluster left, collapsing the selection.
    pub fn move_left(&mut self) -> bool {
        let at = self.selection().caret;
        if at == 0 {
            self.set_selection(Selection::caret(0));
            return false;
        }
        self.set_selection(Selection::caret(at - 1));
        true
    }

    /// Move the caret one cluster right, collapsing the selection.
    pub fn move_right(&mut self) -> bool {
        let at = self.selection().caret;
        let len = self.buffer.len_clusters();
        if at >= len {
            self.set_selection(Selection::caret(len));
            return false;
        }
        self.set_selection(Selection::caret(at + 1));
        true
    }

    /// Move the caret to the start, collapsing the selection.
    pub fn move_to_start(&mut self) {
        self.set_selection(Selection::caret(0));
    }

    /// Move the caret to the end, collapsing the selection.
    pub fn move_to_end(&mut self) {
        let n = self.buffer.len_clusters();
        self.set_selection(Selection::caret(n));
    }

    /// Extend the selection left by `n` clusters from the anchor's side.
    ///
    /// The anchor stays put and the caret moves, which is what Shift+Arrow
    /// means and what makes a backwards selection fall out for free.
    pub fn select_left(&mut self, n: usize) {
        let sel = self.selection();
        let caret = sel.caret.saturating_sub(n);
        self.set_selection(Selection {
            anchor: sel.anchor,
            caret,
        });
    }

    /// Extend the selection right by `n` clusters.
    pub fn select_right(&mut self, n: usize) {
        let sel = self.selection();
        let caret = sel.caret.saturating_add(n);
        self.set_selection(Selection {
            anchor: sel.anchor,
            caret,
        });
    }

    /// Select everything.
    pub fn select_all(&mut self) {
        let n = self.buffer.len_clusters();
        self.set_selection(Selection::between(0, n));
    }

    /// Collapse the selection to its caret, keeping the caret.
    pub fn collapse(&mut self) {
        let at = self.selection().caret;
        self.set_selection(Selection::caret(at));
    }

    /// Move the caret to `at` clusters, collapsing any selection.
    ///
    /// Clamped to the buffer, because `at` comes from a platform that reports
    /// positions in its own unit and may be stale.
    pub fn set_caret(&mut self, at: usize) {
        self.set_selection(Selection::caret(at));
    }

    /// The selected text, or `""` when the selection is collapsed.
    #[must_use]
    pub fn selection_text(&self) -> &str {
        let sel = self.selection();
        if sel.is_collapsed() {
            return "";
        }
        self.buffer.slice(sel.range()).unwrap_or("")
    }

    /// Whether there is a non-empty selection.
    #[must_use]
    pub fn has_selection(&self) -> bool {
        !self.selection().is_collapsed()
    }
}

/// A convenience alias for the common case.
pub type StringField = TextField<crate::buffer::StringBuffer>;

/// Build a field over an owned `String`.
#[must_use]
pub fn field_from(text: &str) -> StringField {
    TextField::with_caret_at_end(crate::buffer::StringBuffer::from_text(text))
}

/// Build a field over an owned `String`, caret at the start.
#[must_use]
pub fn field_from_start(text: &str) -> StringField {
    TextField::new(crate::buffer::StringBuffer::from_text(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::StringBuffer;
    use alloc::string::ToString;

    #[test]
    fn a_backwards_selection_is_representable() {
        let s = Selection::between(4, 1);
        assert!(!s.is_collapsed());
        assert_eq!(s.range(), 1..4);
        assert_eq!(s.len(), 3);
        let mut f = TextField::with_caret_at_end(StringBuffer::from_text("abcdef"));
        f.set_selection(s);
        assert_eq!(f.selection_text(), "bcd");
        f.delete_selection();
        assert_eq!(f.text(), "aef");
        assert_eq!(f.caret(), 1, "caret lands at the start of the removed span");
    }

    /// THE property: backspace removes a whole cluster, never half of one.
    ///
    /// `cafe` + U+0301 is four clusters: c, a, f, and the pair e+mark. One
    /// backspace at the end removes the e *and* its mark, leaving `caf`.
    #[test]
    fn backspace_takes_the_whole_cluster() {
        let mut f = field_from("cafe\u{301}");
        assert_eq!(f.len(), 4, "four clusters, six bytes");
        f.move_to_end();
        assert!(f.backspace());
        assert_eq!(f.text(), "caf", "the combining mark went with its base");
        assert_eq!(f.caret(), 3);
        // A second backspace takes the f, leaving two clusters.
        assert!(f.backspace());
        assert_eq!(f.text(), "ca");

        // A flag is one cluster, so one backspace removes both halves.
        let mut g = field_from("\u{1F1FA}\u{1F1F8}");
        assert_eq!(g.len(), 1);
        g.move_to_end();
        assert!(g.backspace());
        assert_eq!(g.text(), "");
    }

    #[test]
    fn delete_forward_takes_the_whole_cluster() {
        let mut f = field_from_start("cafe\u{301}");
        assert!(f.delete_forward());
        assert_eq!(f.text(), "afe\u{301}", "the e and its mark went together");
        assert_eq!(f.caret(), 0, "the caret does not move when deleting forward");
        // At the end there is nothing forward to delete.
        f.move_to_end();
        assert!(!f.delete_forward());
    }

    #[test]
    fn backspace_at_the_start_is_a_no_op_not_a_panic() {
        let mut f = field_from_start("ab");
        assert!(!f.backspace());
        assert_eq!(f.text(), "ab");
        assert_eq!(f.caret(), 0);
        let mut empty = field_from("");
        assert!(!empty.backspace());
        assert!(!empty.delete_forward());
    }

    #[test]
    fn inserting_moves_the_caret_by_clusters_not_bytes() {
        let mut f = field_from_start("");
        f.insert("e\u{301}");
        assert_eq!(f.text(), "e\u{301}");
        assert_eq!(f.caret(), 1, "two code points, one cluster");
        // The caret is after the cluster, so the next insert lands after it.
        f.insert("x");
        assert_eq!(f.text(), "e\u{301}x");
        assert_eq!(f.caret(), 2);
    }

    #[test]
    fn shift_movement_extends_from_the_anchor() {
        let mut f = field_from_start("hello");
        f.move_right();
        f.move_right();
        assert_eq!(f.caret(), 2);
        f.select_left(2);
        let sel = f.selection();
        assert_eq!((sel.anchor, sel.caret), (2, 0));
        assert_eq!(f.selection_text(), "he", "a backwards selection reads correctly");

        // Extending left again is a no-op, because the caret is already at the
        // leftmost position and the anchor is the fixed end. This asymmetry is
        // the point of an anchor: you grow a selection from one end.
        f.select_left(1);
        assert_eq!(f.selection_text(), "he");
        assert_eq!(f.selection().anchor, 2, "the anchor stays where it was");

        // Growing back to the right works, because the caret can move again.
        // Reaching the anchor collapses the selection.
        f.select_right(2);
        assert_eq!(f.selection().caret, 2);
        assert_eq!(f.selection_text(), "", "the caret reached the anchor");
        f.select_right(1);
        assert_eq!(f.selection_text(), "l");
        assert_eq!(f.selection().anchor, 2, "still anchored at 2");
    }

    #[test]
    fn a_selection_replaced_by_an_insert_leaves_no_trace() {
        let mut f = field_from("hello world");
        f.move_to_start();
        f.select_right(6);
        assert_eq!(f.selection_text(), "hello ");
        f.insert("goodbye");
        // The selection included the space, so it went with the rest.
        assert_eq!(f.text(), "goodbyeworld");
        assert_eq!(f.caret(), 7);
        assert!(!f.has_selection(), "the caret collapses after the insert");
    }

    #[test]
    fn select_all_and_collapse() {
        let mut f = field_from("cafe\u{301}");
        f.select_all();
        assert_eq!(f.selection_text(), "cafe\u{301}");
        assert!(f.has_selection());
        f.collapse();
        assert!(!f.has_selection());
        assert_eq!(f.caret(), 4);
    }

    #[test]
    fn movement_clamps_at_both_ends() {
        let mut f = field_from("ab");
        assert_eq!(f.caret(), 2, "field_from leaves the caret at the end");
        assert!(f.move_left());
        assert!(f.move_left());
        assert_eq!(f.caret(), 0);
        assert!(!f.move_left(), "already at the start");
        assert_eq!(f.caret(), 0);
        f.move_to_end();
        assert_eq!(f.caret(), 2);
        assert!(!f.move_right(), "already at the end");
        assert_eq!(f.caret(), 2);
    }

    #[test]
    fn extending_past_the_end_clamps_rather_than_panicking() {
        let mut f = field_from_start("ab");
        f.select_right(99);
        assert_eq!(f.selection_text(), "ab", "a huge selection is the whole field");
        assert_eq!(f.selection().caret, 2, "the caret clamped to the end");
        // Collapsing to the start again, then past it.
        f.select_left(99);
        assert_eq!(f.selection_text(), "");
        assert_eq!(f.caret(), 0, "the caret clamped to the start");
        // And back out again.
        f.select_right(99);
        assert_eq!(f.selection_text(), "ab");
        assert_eq!(f.selection().caret, 2);
    }

    /// A field whose buffer was mutated underneath it must not hand out a
    /// caret past the end.
    #[test]
    fn the_caret_survives_an_out_of_band_buffer_edit() {
        let mut f = field_from("abcdef");
        f.move_to_end();
        assert_eq!(f.caret(), 6);
        f.buffer_mut().clear();
        assert_eq!(f.caret(), 0, "a stale caret clamps rather than panicking");
        assert_eq!(f.selection_text(), "");
        assert!(!f.backspace());
        assert!(f.insert("z"));
        assert_eq!(f.text(), "z");
    }

    #[test]
    fn replace_selection_handles_a_collapsed_caret() {
        let mut f = field_from_start("ab");
        f.replace_selection("X");
        assert_eq!(f.text(), "Xab");
    }

    #[test]
    fn insert_at_leaves_the_caret_alone() {
        let mut f = field_from_start("ab");
        f.insert_at(1, "-");
        assert_eq!(f.text(), "a-b");
        assert_eq!(f.caret(), 0, "insert_at is a buffer operation, not an edit");
    }

    #[test]
    fn to_string_round_trips_through_the_buffer() {
        let mut f = field_from("");
        for c in "typing".chars() {
            f.insert(&c.to_string());
        }
        assert_eq!(f.text(), "typing");
        assert_eq!(f.len(), 6);
        assert_eq!(f.caret(), 6);
    }
}