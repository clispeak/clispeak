//! Validation and chunking.
//!
//! Pure functions: no I/O, no async, no platform code. This is where the
//! highest test density lives — every ugly case in `docs/text.md` is a table
//! row. Implemented at M2.

/// Why some text was refused.
///
/// Errors carry enough to build a message an agent can act on: the offending
/// span, and a suggested rewrite it can resend verbatim.
#[derive(Debug, thiserror::Error)]
pub enum Rejection {
    /// Markdown that will not read well aloud.
    #[error("text contains markdown that will not read well aloud")]
    Markdown {
        /// Byte range of the offending span.
        span: (usize, usize),
        /// What was found, e.g. "bold emphasis".
        kind: &'static str,
    },
    /// A bare URL. Reading one aloud is unbearable; describe the destination.
    #[error("text contains a URL that will not read well aloud")]
    Url {
        /// Byte range of the offending span.
        span: (usize, usize),
    },
}

/// Reject text that will not read well aloud.
///
/// Strict by default: markdown and bare URLs are errors, because an agent can
/// read the error and rewrite. Emoji and stray whitespace are handled quietly,
/// because there is no obviously better thing to suggest.
pub fn validate(_text: &str) -> Result<(), Rejection> {
    todo!("M2")
}

/// Split text into speakable chunks at safe sentence boundaries.
///
/// Masks spans that must never be split — URLs, paths, decimals, IPs,
/// abbreviations — then splits, then restores.
pub fn chunk(_text: &str) -> Vec<String> {
    todo!("M2")
}
