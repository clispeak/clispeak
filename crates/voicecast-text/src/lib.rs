//! Validation and chunking.
//!
//! Pure functions: no I/O, no async, no platform code. This is where the
//! project's highest test density lives — every ugly case in `docs/text.md`
//! is a table row below.
//!
//! Three stages, in order:
//!
//! ```text
//!   validate  ->  protect  ->  split
//!   (reject)      (mask)       (chunk)
//! ```

mod chunk;
mod protect;
mod strip;
mod validate;

pub use chunk::chunk;
pub use strip::strip;
pub use validate::validate;

/// Why some text was refused.
///
/// Carries enough to build a message an agent can act on: the offending span,
/// and — via [`strip`] — a rewrite it can resend verbatim.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
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

impl Rejection {
    /// Byte range of the offending text.
    pub fn span(&self) -> (usize, usize) {
        match self {
            Self::Markdown { span, .. } | Self::Url { span } => *span,
        }
    }

    /// Short description of what was found, for the caret annotation.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Markdown { kind, .. } => kind,
            Self::Url { .. } => "URL",
        }
    }
}
