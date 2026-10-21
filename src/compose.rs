//! The IME bridge: a text field that composes.
//!
//! `input-kit`'s [`Composer`] knows what the platform is typing. A [`TextField`]
//! knows where the text is. Neither knows about the other, and that is correct:
//! one is platform protocol, the other is text editing.
//!
//! This module is the seam, and it is small on purpose: commit goes in as an
//! insert, a preedit is rendered by the caller, and nothing about the field's
//! caret depends on the platform's cursor unit. That last part is the point of
//! routing through `input-kit` rather than storing a preedit offset here.

use crate::buffer::TextBuffer;
use crate::field::TextField;
use input_kit::{Composer, InputEvent, Output, Preedit};

/// What the caller should do after handing an event to the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldUpdate {
    /// Nothing changed.
    Idle,
    /// A preedit is open: render `preedit` inline at the caret. The field's
    /// text is unchanged.
    Composing(Preedit),
    /// The field's text changed by one insert. Commit, or rebuild the value.
    Edited,
    /// The preedit was abandoned; remove any rendered inline text.
    Cancelled,
}

/// A text field plus an IME composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposingField<B> {
    field: TextField<B>,
    composer: Composer,
}

impl<B: TextBuffer> ComposingField<B> {
    /// A field with no composition active.
    #[must_use]
    pub fn new(field: TextField<B>) -> Self {
        Self {
            field,
            composer: Composer::new(),
        }
    }

    /// A field over `buffer`, caret at the end.
    #[must_use]
    pub fn with_caret_at_end(buffer: B) -> Self {
        Self::new(TextField::with_caret_at_end(buffer))
    }

    /// The underlying field.
    #[must_use]
    pub fn field(&self) -> &TextField<B> {
        &self.field
    }

    /// The underlying field, mutably. Editing it directly while a composition
    /// is open will leave the two out of step; prefer [`Self::handle`].
    pub fn field_mut(&mut self) -> &mut TextField<B> {
        &mut self.field
    }

    /// Consume and yield the field.
    #[must_use]
    pub fn into_field(self) -> TextField<B> {
        self.field
    }

    /// Whether an IME composition is in progress.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.composer.is_composing()
    }

    /// The preedit to render inline, if any.
    #[must_use]
    pub fn preedit(&self) -> Option<&Preedit> {
        self.composer.preedit()
    }

    /// The field's text. While composing this is the committed text only; the
    /// preedit is not part of it.
    #[must_use]
    pub fn text(&self) -> &str {
        self.field.text()
    }

    /// Feed a platform event.
    ///
    /// A commit or a direct insert becomes an ordinary insert at the caret, so
    /// every caret, selection, and backspace rule applies to IME text exactly
    /// as it does to typed text. There is no second code path, which is where
    /// IME bugs usually live.
    pub fn handle(&mut self, event: InputEvent) -> FieldUpdate {
        match self.composer.handle(event) {
            Output::Idle => FieldUpdate::Idle,
            Output::Preedit(p) => FieldUpdate::Composing(p),
            Output::Commit(text) => {
                self.field.insert(&text);
                FieldUpdate::Edited
            }
            Output::Cancelled => FieldUpdate::Cancelled,
        }
    }

    /// Feed a platform event whose preedit caret is a UTF-16 code unit index
    /// (Win32, macOS, the web).
    pub fn handle_utf16_preedit(&mut self, text: &str, caret: u16) -> FieldUpdate {
        self.handle(InputEvent::preedit_utf16(text, caret))
    }

    /// Feed a platform event whose preedit caret is a byte offset (Wayland,
    /// X11).
    pub fn handle_byte_preedit(&mut self, text: &str, caret: usize) -> FieldUpdate {
        self.handle(InputEvent::preedit_bytes(text, caret))
    }

    /// End composition because the widget lost focus, recovering the text.
    ///
    /// The default is to commit what the user typed, because losing a
    /// half-typed syllable is worse than inserting it. Returns `false` when
    /// there was nothing pending.
    pub fn flush(&mut self) -> bool {
        match self.composer.flush() {
            Some(text) => {
                self.field.insert(&text);
                true
            }
            None => false,
        }
    }
}

/// A convenience alias for the common case.
pub type ComposingStringField = ComposingField<crate::buffer::StringBuffer>;

/// Build a composing field over an owned `String`, caret at the end.
#[must_use]
pub fn composing_field_from(text: &str) -> ComposingStringField {
    ComposingField::with_caret_at_end(crate::buffer::StringBuffer::from_text(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_commit_becomes_an_ordinary_insert() {
        let mut f = composing_field_from("hello ");
        let u = f.handle(InputEvent::Preedit(Preedit::at_end("か")));
        assert!(matches!(u, FieldUpdate::Composing(_)));
        assert_eq!(f.text(), "hello ", "the preedit is not in the field yet");
        assert!(f.is_composing());

        let u = f.handle(InputEvent::Commit("ん".into()));
        assert_eq!(u, FieldUpdate::Edited);
        assert_eq!(f.text(), "hello ん");
        assert!(!f.is_composing());
    }

    /// A commit REPLACES the preedit; it does not append to it.
    ///
    /// This is the semantic that is easy to get wrong, because the preedit is
    /// on screen right up until the commit arrives and a naive implementation
    /// treats it as already-inserted text. It is not: the preedit is a
    /// display-only overlay, and the commit is the whole final value. Inserting
    /// both would duplicate the text.
    #[test]
    fn a_commit_replaces_the_preedit_rather_than_appending_to_it() {
        let mut f = composing_field_from("");
        for preedit in ["に", "にほ", "にほんご"] {
            f.handle(InputEvent::Preedit(Preedit::at_end(preedit)));
        }
        assert_eq!(f.text(), "", "the preedit never entered the value");
        f.handle(InputEvent::Commit("日本語".into()));
        assert_eq!(f.text(), "日本語", "the commit is the entire value");
        assert_eq!(f.field().len(), 3, "three characters, not nine");
    }

    /// The caret must be in clusters, so committing a decomposed character
    /// moves it by one.
    #[test]
    fn committing_moves_the_caret_by_clusters() {
        let mut f = composing_field_from("");
        f.handle(InputEvent::Commit("e\u{301}".into()));
        assert_eq!(f.text(), "e\u{301}");
        assert_eq!(f.field().caret(), 1);
    }

    #[test]
    fn backspace_after_a_commit_deletes_the_whole_character() {
        let mut f = composing_field_from("");
        f.handle(InputEvent::Commit("cafe\u{301}".into()));
        assert_eq!(f.field().len(), 4);
        assert!(f.field_mut().backspace());
        assert_eq!(
            f.text(),
            "caf",
            "the e and its combining mark went together"
        );
    }

    #[test]
    fn focus_loss_recovers_the_half_typed_syllable() {
        let mut f = composing_field_from("");
        f.handle(InputEvent::Preedit(Preedit::at_end("にほ")));
        assert!(f.is_composing());
        assert!(f.flush());
        assert_eq!(f.text(), "にほ");
        assert!(!f.is_composing());
        assert!(!f.flush(), "nothing left to flush");
    }

    #[test]
    fn a_cancelled_composition_inserts_nothing() {
        let mut f = composing_field_from("");
        f.handle(InputEvent::Preedit(Preedit::at_end("か")));
        assert_eq!(f.handle(InputEvent::Reset), FieldUpdate::Cancelled);
        assert_eq!(f.text(), "");
        assert!(!f.is_composing());
        assert!(!f.flush());
    }

    /// Both platform dialects must produce the same field state.
    #[test]
    fn both_platform_preedit_dialects_reach_the_same_state() {
        let mut via_utf16 = composing_field_from("");
        let mut via_bytes = composing_field_from("");
        let _ = via_utf16.handle_utf16_preedit("にほんご", 3);
        let _ = via_bytes.handle_byte_preedit("にほんご", 9);
        assert_eq!(via_utf16.text(), via_bytes.text());
        assert_eq!(
            via_utf16.preedit().map(|p| p.caret()),
            via_bytes.preedit().map(|p| p.caret()),
            "the two units disagreed about where the caret is"
        );
        assert_eq!(via_utf16.preedit().map(|p| p.caret()), Some(3));
    }

    /// The whole point: text typed through the IME obeys the same caret rules
    /// as text typed on the keyboard.
    #[test]
    fn ime_text_obeys_the_same_caret_rules_as_typed_text() {
        let mut f = composing_field_from("");
        // Type "日本語" entirely through composition.
        for pre in ["に", "にほ", "にほんご"] {
            let _ = f.handle(InputEvent::Preedit(Preedit::at_end(pre)));
        }
        f.handle(InputEvent::Commit("日本語".into()));
        assert_eq!(f.text(), "日本語");
        assert_eq!(f.field().caret(), 3);

        // Backspace removes one whole cluster, same as any other text.
        assert!(f.field_mut().backspace());
        assert_eq!(f.text(), "日本");
        assert_eq!(f.field().caret(), 2);

        // And the selection works across it.
        f.field_mut().select_all();
        assert_eq!(f.field().selection_text(), "日本");
    }

    #[test]
    fn a_direct_insert_while_composing_aborts_the_composition() {
        let mut f = composing_field_from("");
        f.handle(InputEvent::Preedit(Preedit::at_end("か")));
        assert_eq!(
            f.handle(InputEvent::Insert("x".into())),
            FieldUpdate::Edited
        );
        assert_eq!(f.text(), "x");
        assert!(!f.is_composing(), "the preedit did not interleave");
    }

    #[test]
    fn an_empty_preedit_does_not_disturb_the_field() {
        let mut f = composing_field_from("ab");
        assert_eq!(
            f.handle(InputEvent::Preedit(Preedit::empty())),
            FieldUpdate::Idle
        );
        assert_eq!(
            f.text(),
            "ab",
            "an empty preedit must not disturb the field"
        );
        assert!(!f.is_composing());
    }
}
