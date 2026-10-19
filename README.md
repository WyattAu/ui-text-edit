# ui-text-edit

Text-field semantics for Rust GUIs: a buffer-agnostic `TextBuffer`, a
grapheme-correct caret/selection/edit model, and an a11y-state bridge.

`no_std` + `alloc`. L2 in the estate's layer model. **Not a code editor.**

```toml
[dependencies]
ui-text-edit = "0.1"

# The accessibility bridge (drives an a11y-kit node from a field):
ui-text-edit = { version = "0.1", features = ["a11y"] }
```

## The problem

Four GUI stacks in this estate need to let someone type a name into a box, and
each one does it by hand. The hard part of typing is not the letters. It is
everything around them:

- A backspace must remove a whole character and not orphan its combining mark.
- An IME must be able to show a half-formed syllable **without** that syllable
  reaching the value.
- A screen reader must not announce a value the user has not finished typing,
  and must not read a 10 KB mail body in full on every focus.

None of that is a widget. It is semantics, and every toolkit reimplements it
slightly differently.

## What is in the box

| Type | Purpose |
| --- | --- |
| `TextBuffer` | where the text lives, behind a trait |
| `StringBuffer` | the default: a `String` held whole |
| `TextField` | one caret, one optional selection, the editing operations a form field needs |
| `ComposingField` | a field wired to `input-kit`'s IME composer |
| `bridge` | drives an `a11y_kit` node's value and `busy` state from a field |

Deliberately **absent**: multiple carets, transactions, a revision tree, undo,
folds. Those belong to an editing engine, and this estate has no engine to
wrap. See [ADR-0006](https://github.com/WyattAu/engineering-standards/blob/main/docs/adr/0006-text-field-layer-not-editing-engine.md)
for the survey that established it, and for why building a sixth editing core
would have been the wrong answer.

## Every index is a grapheme cluster

This is the one decision everything else rests on. Offsets are cluster indices,
never byte or `char` indices, inherited from
[`input-kit`](https://crates.io/crates/input-kit). So a caret cannot land
between a letter and its combining mark, between the two halves of a flag, or
inside an emoji ZWJ sequence.

```rust
use ui_text_edit::field_from;

let mut f = field_from("cafe\u{301}");
assert_eq!(f.len(), 4, "six bytes, four characters to a reader");
f.move_to_end();
f.backspace();
assert_eq!(f.text(), "caf", "the combining mark went with its base");
```

## IME composition

A commit becomes an ordinary insert at the caret, so every caret, selection, and
backspace rule applies to IME text exactly as it does to typed text. There is
no second code path, which is where IME bugs usually live.

```rust
use ui_text_edit::compose::composing_field_from;
use ui_text_edit::FieldUpdate;
use input_kit::{InputEvent, Preedit};

let mut f = composing_field_from("");
f.handle(InputEvent::Preedit(Preedit::at_end("にほ")));
assert_eq!(f.text(), "", "the preedit is a display overlay, not the value");
assert!(f.is_composing());

f.handle(InputEvent::Commit("日本".into()));
assert_eq!(f.text(), "日本");
assert!(!f.is_composing());
```

`flush()` recovers a half-typed syllable on focus loss, because losing it is
worse than inserting it.

## Accessibility

With the `a11y` feature, a field drives an `a11y-kit` node:

```rust,ignore
let node = ui_text_edit::bridge::a11y_node_for(&field, "Body");
assert!(node.states.busy);  // composing: the value is not final
```

The value is truncated at a cluster boundary so a screen reader reads the field
rather than the whole document, and `busy` is driven from the composer rather
than from a flag the caller has to remember to set.

## Why not just depend on an editor engine

Because every existing one either owns its buffer, so it cannot be swapped when
`ropey` 2.0 flips its index model from char-based to byte-based, or is a code
editor this estate does not need. Putting `TextBuffer` behind a trait means the
storage can be replaced without touching a single caller.

## Licence

MIT OR Apache-2.0.