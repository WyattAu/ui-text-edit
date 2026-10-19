//! The accessibility bridge: a text field as an `a11y_kit` node.
//!
//! A form field has three things to say to a screen reader: what it is called,
//! what its value is, and whether that value is final. The third is the one
//! that is always forgotten, and it is the one an IME breaks: while a syllable
//! is half-formed the field's value is transient, and announcing it reads a
//! character the user has not finished typing.
//!
//! `a11y_kit::States::busy` exists for exactly that, added in `a11y-kit` 0.2.0
//! after this crate's composition layer was composed against it and the gap
//! was found (see `estate-integration`'s `input_composition` suite). This
//! module is the wiring that closes the loop.
//!
//! ```
//! # #[cfg(feature = "a11y")] {
//! use ui_text_edit::{bridge::a11y_node_for, compose::composing_field_from};
//! use input_kit::{InputEvent, Preedit};
//!
//! let mut field = composing_field_from("Page text");
//! let node = a11y_node_for(&field, "Page");
//! assert!(!node.states.busy, "a settled field is not busy");
//!
//! field.handle(InputEvent::Preedit(Preedit::at_end("にほ")));
//! let node = a11y_node_for(&field, "Page");
//! assert!(node.states.busy, "a composing field must not have its value announced");
//! # }
//! ```

use crate::buffer::TextBuffer;
use crate::compose::ComposingField;
use crate::field::TextField;
use alloc::borrow::ToOwned;
use alloc::format;
use alloc::string::String;

/// The `a11y_kit` types this bridge needs, imported only when the feature is on.
#[cfg(feature = "a11y")]
use a11y_kit::{A11yNode, A11yRole, NodeId, States};

/// The most characters of a field value worth announcing.
///
/// A screen reader reads the accessible name on every focus. A 10 KB mail body
/// read in full is worse than useless: the user cannot act on it and cannot
/// skip it. Measured by convention rather than by a standard: WAI-ARIA does
/// not mandate a length, but every production tree truncates.
pub const MAX_ANNOUNCED_CHARS: usize = 80;

/// Marks a value that was cut short.
pub const ELLIPSIS: char = '\u{2026}';

/// Truncate to [`MAX_ANNOUNCED_CHARS`] clusters, cut on a cluster boundary.
///
/// The cut is the important part. Truncating by bytes or by `char` would split
/// a combining mark from its base or cut a flag in half, and the screen reader
/// would then read something the field does not contain.
#[must_use]
pub fn truncate_for_announcement(text: &str) -> String {
    let total = input_kit::grapheme::count(text);
    if total <= MAX_ANNOUNCED_CHARS {
        return text.to_owned();
    }
    let cut = input_kit::grapheme::byte_offset(text, MAX_ANNOUNCED_CHARS).unwrap_or(text.len());
    let mut out = String::with_capacity(cut + ELLIPSIS.len_utf8());
    // `byte_offset` returns a cluster boundary, so this slice is always valid.
    out.push_str(text.get(..cut).unwrap_or(""));
    out.push(ELLIPSIS);
    out
}

/// Whether a value was cut by [`truncate_for_announcement`].
#[must_use]
pub fn is_truncated(text: &str) -> bool {
    input_kit::grapheme::count(text) > MAX_ANNOUNCED_CHARS
}

/// The accessible value of a field: its text, or a placeholder when empty.
///
/// An empty textbox must still announce something; a screen reader reading an
/// empty name reports nothing at all, which is worse than a generic "edit".
/// A long value is truncated by [`truncate_for_announcement`], because the
/// alternative is reading the whole field on every focus.
#[must_use]
pub fn accessible_value<B: TextBuffer>(field: &TextField<B>) -> String {
    let text = field.text();
    if text.is_empty() {
        return "(empty)".to_owned();
    }
    truncate_for_announcement(text)
}

/// Whether the field's value is transient and must not be announced.
///
/// This is the single definition of "busy" in this crate: composing, or
/// flushing pending text. It reads from the composer rather than taking a
/// flag, so the tree cannot drift from the field.
#[must_use]
pub fn is_busy<B: TextBuffer>(field: &ComposingField<B>) -> bool {
    field.is_composing()
}

/// The `a11y_kit` node for a plain field, settled.
#[cfg(feature = "a11y")]
#[must_use]
pub fn a11y_node_for_field<B: TextBuffer>(
    field: &TextField<B>,
    name: &str,
    id: NodeId,
) -> A11yNode {
    let mut node = A11yNode::new(id, role_for(field), name)
        .with_states(States {
            required: false,
            busy: false,
            ..States::default()
        });
    node.name = format!("{name}: {}", accessible_value(field));
    node
}

/// The `a11y_kit` node for a composing field.
///
/// `busy` is driven by the composer, so a field that is mid-syllable is marked
/// and one that has committed is not, with no way for the two to disagree.
#[cfg(feature = "a11y")]
#[must_use]
pub fn a11y_node_for<B: TextBuffer>(
    field: &ComposingField<B>,
    name: &str,
) -> A11yNode {
    let busy = is_busy(field);
    let mut node = A11yNode::new(NodeId(0), role_for(field.field()), name)
        .with_states(States {
            busy,
            ..States::default()
        });
    node.name = format!("{name}: {}", accessible_value(field.field()));
    node
}

/// `MultilineTextbox` when the text contains a newline, `Textbox` otherwise.
#[cfg(feature = "a11y")]
#[must_use]
pub fn role_for<B: TextBuffer>(field: &TextField<B>) -> A11yRole {
    if field.text().contains('\n') {
        A11yRole::MultilineTextbox
    } else {
        A11yRole::Textbox
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::StringBuffer;
    use input_kit::{InputEvent, Preedit};

    #[test]
    fn an_empty_field_announces_something() {
        let f = crate::field::field_from("");
        assert_eq!(accessible_value(&f), "(empty)");
        let f = crate::field::field_from("hello");
        assert_eq!(accessible_value(&f), "hello");
    }

    #[test]
    fn busy_is_read_from_the_composer_not_from_a_flag() {
        let mut f = crate::compose::composing_field_from("");
        assert!(!is_busy(&f));
        f.handle(InputEvent::Preedit(Preedit::at_end("か")));
        assert!(is_busy(&f), "composing means busy");
        f.handle(InputEvent::Commit("か".into()));
        assert!(!is_busy(&f), "committed means not busy");
    }

    #[cfg(feature = "a11y")]
    #[test]
    fn the_node_marks_a_composing_field_busy() {
        let mut f = crate::compose::composing_field_from("Page text");
        let node = a11y_node_for(&f, "Page");
        assert!(!node.states.busy);
        assert_eq!(node.role, A11yRole::Textbox);
        assert!(node.name.contains("Page text"));

        f.handle(InputEvent::Preedit(Preedit::at_end("にほ")));
        let node = a11y_node_for(&f, "Page");
        assert!(node.states.busy, "the preedit must not be announced as the value");
    }

    #[cfg(feature = "a11y")]
    #[test]
    fn a_multiline_field_is_a_multiline_textbox() {
        let one = crate::field::field_from("single");
        assert_eq!(role_for(&one), A11yRole::Textbox);
        let many = crate::field::field_from("line one\nline two");
        assert_eq!(role_for(&many), A11yRole::MultilineTextbox);
    }

    #[cfg(feature = "a11y")]
    #[test]
    fn a_settled_node_still_audits_clean() {
        use a11y_kit::A11yTree;
        let f = crate::field::TextField::new(StringBuffer::from_text("hi"));
        let node = a11y_node_for_field(&f, "Label", a11y_kit::NodeId(1));
        let mut tree = A11yTree::new();
        tree.insert(a11y_kit::A11yNode::new(a11y_kit::NodeId(0), a11y_kit::A11yRole::Window, "Win"));
        let mut node = node;
        node.parent = Some(a11y_kit::NodeId(0));
        node.states.focused = true;
        tree.insert(node);
        assert!(tree.audit().is_ok());
    }
}