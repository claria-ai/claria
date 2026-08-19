//! A wrapper that makes leaking a value into a log an act of intent.
//!
//! Claria's audit trail is a PHI-access record, so some of what it carries —
//! a search query, a client name, a fragment of a note — must reach S3 in full
//! and must never reach the Claria Console the user can export and mail to
//! support. The console layer flattens every field of every admitted tracing
//! event into a message string, so there is no structured boundary downstream
//! to filter on: whatever a `tracing` macro is handed is what ends up in the
//! export.
//!
//! [`Sensitive`] moves the boundary to the type. A `Sensitive<String>` renders
//! as `[redacted]` through both `Display` and `Debug`, so the two ways a value
//! reaches a tracing field — `%value` and `?value` — both produce nothing
//! useful. Getting the real value out requires calling [`Sensitive::reveal`],
//! which is one word, greppable in review, and hard to type by accident.

use std::fmt;

use serde::{Deserialize, Serialize};

/// What is printed in place of a sensitive value.
const REDACTED: &str = "[redacted]";

/// A value that must not be rendered into logs.
///
/// Serialization is transparent — the wrapped value encodes and decodes
/// exactly as it would on its own, so putting a field behind `Sensitive` is
/// not a schema change. Only the human-readable renderings are suppressed.
///
/// ```
/// use claria_core::sensitive::Sensitive;
///
/// let query = Sensitive::new("patient with recurring migraines".to_string());
/// assert_eq!(format!("{query}"), "[redacted]");
/// assert_eq!(format!("{query:?}"), "[redacted]");
/// assert_eq!(query.reveal(), "patient with recurring migraines");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sensitive<T>(T);

impl<T> Sensitive<T> {
    /// Wrap a value so it cannot be rendered by accident.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Borrow the wrapped value.
    ///
    /// This is the only way to read a sensitive value, and it is deliberately
    /// one distinctive word: `grep -rn 'reveal()'` enumerates every place in
    /// the codebase where PHI leaves its wrapper.
    pub fn reveal(&self) -> &T {
        &self.0
    }

    /// Take ownership of the wrapped value. Same caveat as [`Self::reveal`].
    pub fn reveal_into(self) -> T {
        self.0
    }

    /// Apply a function to the wrapped value, keeping the result wrapped.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Sensitive<U> {
        Sensitive(f(self.0))
    }
}

impl<T: AsRef<str>> Sensitive<T> {
    /// Render as `[redacted; N chars]`.
    ///
    /// A length is not PHI, and it is often the only thing an operator needs
    /// to tell "the query was empty" from "the query was fine but the index
    /// was". Use this instead of reaching for [`Self::reveal`] when the
    /// question is about shape rather than content.
    pub fn redacted_with_len(&self) -> String {
        format!("[redacted; {} chars]", self.0.as_ref().chars().count())
    }
}

impl<T> fmt::Display for Sensitive<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl<T> fmt::Debug for Sensitive<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(REDACTED)
    }
}

impl<T> From<T> for Sensitive<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}
