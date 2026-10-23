//! Validation for proposed field text, applied before an edit is accepted.
//!
//! A field's job is not just to store what was typed; a name field has a
//! length cap, a slug field has a charset, a port field has a numeric
//! range. Every GUI toolkit expresses this as a *validator* — a check that
//! runs on proposed input before the value changes. This module is that
//! check, toolkit-agnostic: a [`Validator`] receives the proposed text and
//! returns the sanitized text to accept, or `None` to reject it outright.
//!
//! Validators compose: a [`Chain`] runs each in order and short-circuits
//! on the first rejection.
//!
//! ```
//! use ui_text_edit::validate::{Chain, MaxLength, Reject, Validator};
//!
//! // A slug: lowercase ASCII and dashes, at most 12 clusters.
//! let slug = Chain::new()
//!     .reject_chars(|c: char| !(c.is_ascii_lowercase() || c == '-'))
//!     .push(MaxLength(12));
//!
//! let accepted = slug.accept("Hello World 123").unwrap();
//! assert_eq!(accepted, "elloorld");
//! ```

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use input_kit::grapheme;

/// A check applied to proposed field text before it is accepted.
///
/// Return `Some(accepted)` with the (possibly sanitized) text to insert —
/// it may differ from `proposed` — or `None` to reject the input whole.
pub trait Validator {
    /// The text to accept for `proposed`, or `None` to reject.
    fn accept(&self, proposed: &str) -> Option<String>;
}

impl<F: Fn(&str) -> Option<String>> Validator for F {
    fn accept(&self, proposed: &str) -> Option<String> {
        self(proposed)
    }
}

/// Reject text longer than `max` clusters, truncating to fit.
///
/// Truncation is grapheme-correct: a 12-cluster cap never splits a
/// combining mark from its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaxLength(pub usize);

impl Validator for MaxLength {
    fn accept(&self, proposed: &str) -> Option<String> {
        if grapheme::count(proposed) <= self.0 {
            return Some(proposed.to_owned());
        }
        let byte = grapheme::byte_offset(proposed, self.0)?;
        Some(proposed[..byte].to_owned())
    }
}

/// Drop every cluster containing a character `reject` disallows.
///
/// The sanitizing validator: what survives is the input minus its
/// forbidden clusters, so "abc<script>" pasted into an alphanumeric field
/// degrades to "abcscript" rather than being refused wholesale.
#[derive(Debug, Clone)]
pub struct Reject<F: Fn(char) -> bool>(pub F);

impl<F: Fn(char) -> bool> Validator for Reject<F> {
    fn accept(&self, proposed: &str) -> Option<String> {
        let kept: String = proposed.chars().filter(|c| !(self.0)(*c)).collect();
        if kept.is_empty() && !proposed.is_empty() {
            return None;
        }
        Some(kept)
    }
}

/// Run validators in order; the first rejection wins, the last
/// sanitization is what is accepted.
#[derive(Default)]
pub struct Chain {
    steps: Vec<Box<dyn Validator>>,
}

impl Chain {
    /// An empty chain (accepts everything unchanged).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a step.
    #[must_use]
    pub fn push<V: Validator + 'static>(mut self, validator: V) -> Self {
        self.steps.push(Box::new(validator));
        self
    }

    /// Append a per-character rejection step.
    #[must_use]
    pub fn reject_chars<F: Fn(char) -> bool + 'static>(self, reject: F) -> Self {
        self.push(Reject(reject))
    }
}

impl Validator for Chain {
    fn accept(&self, proposed: &str) -> Option<String> {
        let mut current = proposed.to_owned();
        for step in &self.steps {
            current = step.accept(&current)?;
        }
        Some(current)
    }
}
