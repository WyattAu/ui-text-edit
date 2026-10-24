//! Undo/redo for a [`TextField`], as a linear journal of snapshots.
//!
//! A form field without undo is not finished: every GUI toolkit's line
//! edit carries a history, and the estate's field layer initially shipped
//! without one because "the estate has no engine to wrap". It does not
//! need an engine. A single-caret field's history is a *linear journal*,
//! and the journal stores the field's full text before and after each
//! edit.
//!
//! Snapshots over spans is a deliberate choice. A span-based journal
//! (start, removed, inserted) is smaller, but it breaks on the first
//! grapheme subtlety: insert a combining mark after a base and the
//! cluster count does not change — the mark *merged* — so a span derived
//! from the inserted text's own cluster count survives the undo that
//! should remove it. Measured spans fix that and break on the next
//! segmentation surprise. Whole-text snapshots cannot lie. Fields are
//! short and the history is capped, so the memory cost is bounded by
//! `cap × len`; a revision tree, branches, and folds remain an editing
//! engine's job (see ADR-0006).
//!
//! [`JournaledField`] wraps any [`TextField`] and journals every mutating
//! call. `replace_selection` is recorded as one edit even though it runs
//! as a delete and an insert, so an undo restores the field in a single
//! step.

use crate::buffer::TextBuffer;
use crate::field::{Selection, TextField};
use alloc::borrow::ToOwned;
use alloc::string::String;
use alloc::vec::Vec;

/// One reversible edit: the field's full text before and after, plus the
/// selections to restore on undo and redo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The field's text before the edit.
    pub text_before: String,
    /// The field's text after the edit.
    pub text_after: String,
    /// The selection to restore when this edit is undone.
    pub selection_before: Selection,
    /// The selection to restore when this edit is redone.
    pub selection_after: Selection,
}

/// A linear undo/redo journal, oldest edit first.
#[derive(Debug, Default)]
pub struct Journal {
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    limit: usize,
}

impl Journal {
    /// An empty journal holding at most `limit` edits (oldest dropped).
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Record an applied edit. Clears the redo stack: the journal is
    /// linear, and a new edit invalidates the future.
    pub fn push(&mut self, edit: Edit) {
        self.undo.push(edit);
        self.redo.clear();
        if self.undo.len() > self.limit {
            let drop = self.undo.len() - self.limit;
            self.undo.drain(0..drop);
        }
    }

    /// Whether an undo is available.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Whether a redo is available.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Pop the edit to undo, if any.
    #[must_use]
    pub fn pop_undo(&mut self) -> Option<Edit> {
        self.undo.pop()
    }

    /// Pop the edit to redo, if any.
    #[must_use]
    pub fn pop_redo(&mut self) -> Option<Edit> {
        self.redo.pop()
    }

    /// Push an edit onto the redo stack (the inverse of an undo).
    pub fn push_redo(&mut self, edit: Edit) {
        self.redo.push(edit);
    }

    /// Forget all history.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

/// A [`TextField`] that journals every mutation for undo/redo.
///
/// The wrapped field is reachable through [`Deref`]/[`DerefMut`], so
/// non-mutating calls (reading the text, moving the caret) pass through
/// unjournaled. Mutations have explicit journaling methods here because a
/// journaled edit must capture the field's pre-state, which a blanket
/// pass-through cannot see.
#[derive(Debug)]
pub struct JournaledField<B: TextBuffer> {
    field: TextField<B>,
    journal: Journal,
}

impl<B: TextBuffer> core::ops::Deref for JournaledField<B> {
    type Target = TextField<B>;
    fn deref(&self) -> &Self::Target {
        &self.field
    }
}

impl<B: TextBuffer> core::ops::DerefMut for JournaledField<B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.field
    }
}

impl<B: TextBuffer> JournaledField<B> {
    /// Wrap `field` with a journal holding at most `limit` edits.
    #[must_use]
    pub fn with_limit(field: TextField<B>, limit: usize) -> Self {
        Self {
            field,
            journal: Journal::new(limit),
        }
    }

    /// The journal, for `can_undo`-style checks by a host UI.
    #[must_use]
    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    /// Snapshot the current state and journal it as an applied edit.
    fn record(&mut self, text_before: String, before: Selection) {
        let edit = Edit {
            text_before,
            text_after: self.field.text().to_owned(),
            selection_before: before,
            selection_after: self.field.selection(),
        };
        self.journal.push(edit);
    }

    /// Insert `text` at the caret, replacing the selection, journaled.
    ///
    /// Returns `false` when nothing changed (empty insertion over a
    /// collapsed caret), mirroring [`TextField::insert`].
    pub fn insert(&mut self, text: &str) -> bool {
        let before = self.field.selection();
        let text_before = self.field.text().to_owned();
        let changed = self.field.insert(text);
        if changed {
            self.record(text_before, before);
        }
        changed
    }

    /// Backspace: delete the selection, or the cluster before the caret.
    /// Journaled.
    pub fn backspace(&mut self) -> bool {
        let before = self.field.selection();
        let text_before = self.field.text().to_owned();
        let changed = self.field.backspace();
        if changed {
            self.record(text_before, before);
        }
        changed
    }

    /// Delete (forward): the selection, or the cluster at the caret.
    /// Journaled.
    pub fn delete_forward(&mut self) -> bool {
        let before = self.field.selection();
        let text_before = self.field.text().to_owned();
        let changed = self.field.delete_forward();
        if changed {
            self.record(text_before, before);
        }
        changed
    }

    /// Replace the selection with `text` — journaled as *one* edit, so an
    /// undo restores the field in a single step.
    pub fn replace_selection(&mut self, text: &str) {
        let before = self.field.selection();
        let text_before = self.field.text().to_owned();
        self.field.replace_selection(text);
        self.record(text_before, before);
    }

    /// Replace the entire value with `text` — journaled.
    ///
    /// The sync-point for hosts that own an input surface (a toolkit's own
    /// text widget) and hand this crate the *accepted* value after
    /// validation: the whole value is one edit, so an undo removes the
    /// accepted batch whole.
    pub fn replace_all(&mut self, text: &str) {
        let before = self.field.selection();
        let text_before = self.field.text().to_owned();
        let len = self.field.len_clusters();
        self.field.buffer_mut().replace(0..len, text);
        self.record(text_before, before);
    }

    /// Undo the most recent edit. Returns whether anything was undone.
    pub fn undo(&mut self) -> bool {
        let Some(edit) = self.journal.pop_undo() else {
            return false;
        };
        let len = self.field.len();
        self.field.buffer_mut().replace(0..len, &edit.text_before);
        self.field.set_selection(edit.selection_before);
        self.journal.push_redo(edit);
        true
    }

    /// Redo the most recently undone edit. Returns whether anything was
    /// redone.
    pub fn redo(&mut self) -> bool {
        let Some(edit) = self.journal.pop_redo() else {
            return false;
        };
        let len = self.field.len();
        self.field.buffer_mut().replace(0..len, &edit.text_after);
        self.field.set_selection(edit.selection_after);
        self.journal.push(edit);
        true
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::field::field_from;

    #[test]
    fn insert_then_undo_restores_exactly() {
        let mut f = JournaledField::with_limit(field_from("cafe"), 64);
        f.insert("\u{301}"); // combining acute, one cluster attaching to the "e"
        assert_eq!(f.text(), "cafe\u{301}");
        assert_eq!(f.len(), 4, "base plus mark is still four clusters");
        assert!(f.undo());
        assert_eq!(
            f.text(),
            "cafe",
            "the combining mark went back with its base"
        );
    }

    #[test]
    fn backspace_undo_round_trips_a_cluster() {
        let mut f = JournaledField::with_limit(field_from("abc"), 64);
        f.move_to_end();
        assert!(f.backspace());
        assert_eq!(f.text(), "ab");
        assert!(f.undo());
        assert_eq!(f.text(), "abc");
        assert!(f.redo());
        assert_eq!(f.text(), "ab");
    }

    #[test]
    fn replace_selection_is_one_edit() {
        let mut f = JournaledField::with_limit(field_from("hello world"), 64);
        f.set_selection(Selection::between(0, 5));
        f.replace_selection("goodbye");
        assert_eq!(f.text(), "goodbye world");
        assert!(f.undo());
        assert_eq!(f.text(), "hello world", "one undo, not two");
    }

    #[test]
    fn redo_is_invalidated_by_a_new_edit() {
        let mut f = JournaledField::with_limit(field_from(""), 64);
        f.insert("one");
        assert!(f.undo());
        f.insert("two");
        assert!(!f.journal().can_redo(), "a new edit clears the future");
        assert_eq!(f.text(), "two");
    }

    #[test]
    fn undo_restores_the_pre_edit_selection() {
        let mut f = JournaledField::with_limit(field_from("abcdef"), 64);
        f.set_selection(Selection::between(1, 4)); // select "bcd"
        f.replace_selection("X");
        assert_eq!(f.text(), "aXef");
        assert!(f.undo());
        assert_eq!(
            f.selection(),
            Selection::between(1, 4),
            "the selection comes back with the text"
        );
    }

    #[test]
    fn the_journal_respects_its_limit() {
        let mut f = JournaledField::with_limit(field_from(""), 2);
        f.insert("a");
        f.insert("b");
        f.insert("c");
        assert!(f.undo());
        assert!(f.undo());
        assert!(
            !f.journal().can_undo(),
            "the oldest edit was dropped at the cap"
        );
        assert_eq!(f.text(), "a");
    }

    #[test]
    fn undo_across_ime_composition() {
        // A ComposingField commit is an ordinary insert, so it journals like
        // one: undoing after an IME commit removes the whole syllable,
        // combining mark and all.
        let mut f = JournaledField::with_limit(field_from(""), 64);
        f.insert("commit\u{308}ed");
        assert!(f.undo());
        assert_eq!(f.text(), "");
    }
}
