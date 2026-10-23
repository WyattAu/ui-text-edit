//! Text-field semantics for Rust GUIs.
//!
//! Four GUI stacks in the estate need to let someone type a name into a box, and
//! each one is doing it by hand. The hard part of typing is not the letters; it
//! is everything around them. A backspace must remove a whole character and not
//! orphan its combining mark. An IME must be able to show a half-formed
//! syllable without that syllable reaching the value. A screen reader must not
//! announce a value the user has not finished typing.
//!
//! `ui-text-edit` is that missing middle layer, and it is deliberately *not* a
//! code editor:
//!
//! - [`StringBuffer`] / [`TextBuffer`] — where the text lives, behind a trait,
//!   so a rope or a piece table can replace it without touching a caller.
//! - [`TextField`] — one caret, one optional selection, and the editing
//!   operations a form field needs. No multiple carets, no revision tree:
//!   those are an editing engine's job and the estate has no engine to wrap.
//! - [`journal::JournaledField`] — optional linear undo/redo over any
//!   field, snapshot-based so combining marks, IME merges, and every other
//!   segmentation surprise cannot corrupt the history.
//! - [`ComposingField`] — a field wired to `input-kit`'s IME composer, so a
//!   commit is an ordinary insert and there is no second code path.
//! - [`bridge`] — drives an `a11y_kit` node's value and `busy` state from a
//!   field, behind the `a11y` feature.
//!
//! # Every index is a grapheme cluster
//!
//! This is the one decision everything else rests on. Offsets are cluster
//! indices, never byte or `char` indices, inherited from `input-kit`. So a
//! caret cannot land between a letter and its combining mark, between the two
//! halves of a flag, or inside an emoji ZWJ sequence.
//!
//! ```
//! use ui_text_edit::field_from;
//!
//! let mut f = field_from("cafe\u{301}");
//! assert_eq!(f.len(), 4, "six bytes, four characters to a reader");
//! f.move_to_end();
//! f.backspace();
//! assert_eq!(f.text(), "caf", "the combining mark went with its base");
//! ```
//!
//! # Why not just depend on an editor engine
//!
//! Because every existing one either owns its buffer (so it cannot be swapped
//! when `ropey` 2.0 flips its index model) or is a code editor the estate does
//! not need. See ADR-0006 for the survey and the measurements.
//!
//! # Layer
//!
//! L2 in the estate's model. Estate deps: `input-kit` L1 (composition) and,
//! optionally, `a11y-kit` L1 (the tree). `no_std` + `alloc`.

#![no_std]
#![deny(missing_docs)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod bridge;
pub mod buffer;
pub mod compose;
pub mod field;
pub mod journal;
pub mod validate;

pub use buffer::{StringBuffer, TextBuffer};
pub use compose::{ComposingField, FieldUpdate};
pub use field::{field_from, field_from_start, Selection, StringField, TextField};
pub use journal::{Edit, Journal, JournaledField};
pub use validate::{Chain, MaxLength, Reject, Validator};

/// The estate layer this crate occupies.
pub const LAYER: &str = "L2";

#[cfg(all(test, feature = "a11y"))]
mod integration {
    //! Cross-module tests with the `a11y` feature on. These are the properties
    //! a consumer actually depends on, and none of them is visible from a
    //! single module: an IME commit landing in the field, then reaching a
    //! screen reader with the right `busy` state at each step.

    use super::*;
    use alloc::string::String;
    use input_kit::{InputEvent, Preedit};

    /// The whole stack, end to end: compose Japanese into a field, and check
    /// that every observable state is correct at every step.
    #[test]
    fn compose_then_announce_without_announcing_a_half_word() {
        let mut f = compose::composing_field_from("");

        // Step 1: the user opens a preedit. The value is still empty, so the
        // screen reader must be told the value is not final.
        f.handle(InputEvent::Preedit(Preedit::at_end("に")));
        assert_eq!(f.text(), "", "the preedit is not the value");
        let node = bridge::a11y_node_for(&f, "Body");
        assert!(node.states.busy, "half a syllable must not be announced");
        assert!(node.name.contains("(empty)"));

        // Step 2: the preedit grows. Still not announced.
        f.handle(InputEvent::Preedit(Preedit::at_end("にほ")));
        assert!(bridge::is_busy(&f));
        assert_eq!(f.text(), "");

        // Step 3: commit. The value appears and the field settles.
        f.handle(InputEvent::Commit("日本".into()));
        assert_eq!(f.text(), "日本");
        let node = bridge::a11y_node_for(&f, "Body");
        assert!(!node.states.busy, "a committed value is final");
        assert!(node.name.contains("日本"));
        assert!(!node.states.disabled, "busy never became unusable");
    }

    /// The IME path must not become a second code path. Every editing rule
    /// that holds for typed text has to hold for composed text.
    #[test]
    fn composed_text_is_ordinary_text_after_it_lands() {
        let mut f = compose::composing_field_from("");
        f.handle(InputEvent::Commit("cafe\u{301}".into()));
        let mut field = f.into_field();

        assert_eq!(field.len(), 4, "four clusters, six bytes");
        field.move_to_end();
        assert!(field.backspace());
        assert_eq!(field.text(), "caf", "the combining mark went with its base");

        // Selection across composed text behaves.
        let mut g = compose::composing_field_from("");
        g.handle(InputEvent::Commit("日本語".into()));
        let mut g = g.into_field();
        g.select_all();
        assert_eq!(g.selection_text(), "日本語");
        g.delete_selection();
        assert_eq!(g.text(), "");
    }

    /// Focus loss at any point must leave the field in a usable, consistent
    /// state: either the typed text is in the value and the field is settled,
    /// or nothing happened.
    #[test]
    fn focus_loss_always_leaves_the_field_consistent() {
        for preedit in ["", "か", "にほんご"] {
            let mut f = compose::composing_field_from("");
            if !preedit.is_empty() {
                f.handle(InputEvent::Preedit(Preedit::at_end(preedit)));
            }
            let flushed = f.flush();
            let node = bridge::a11y_node_for(&f, "Body");

            assert!(!node.states.busy, "a flushed field is never busy");
            if flushed {
                assert_eq!(f.text(), preedit, "flushed text must be the value");
            } else {
                assert_eq!(f.text(), "", "nothing flushed means nothing typed");
            }
            // And it is still editable.
            f.handle(InputEvent::Insert("z".into()));
            assert!(f.text().ends_with('z'));
        }
        let _ = String::new();
    }
}
