//! Splitting text into speakable chunks.
//!
//! Chunks do three jobs: they let speech start before the whole message has
//! arrived, they keep input inside what engines handle, and they are the
//! **resume points** after an interrupt — so chunk quality is audible.

use unicode_segmentation::UnicodeSegmentation;

use crate::protect::{is_protected, protected_ranges};

/// Longest chunk before the fallback cascade kicks in.
const MAX_CHUNK: usize = 200;

/// Split text at safe sentence boundaries.
///
/// Sentence boundaries come from a UAX #29 segmenter, then any boundary
/// landing inside a protected range is discarded and its segments rejoined.
/// Over-long results fall back to clause, then word boundaries — never
/// mid-word.
pub fn chunk(text: &str) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }

    let ranges = protected_ranges(text);
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut offset = 0;

    for sentence in text.unicode_sentences() {
        let end = offset + sentence.len();
        current.push_str(sentence);

        // A boundary inside a protected span isn't a real sentence end, so
        // keep accumulating rather than cutting "10." off from "0.0.1".
        if !is_protected(text, &ranges, end) {
            push_trimmed(&mut chunks, &current);
            current.clear();
        }
        offset = end;
    }
    push_trimmed(&mut chunks, &current);

    chunks.into_iter().flat_map(|c| split_long(&c)).collect()
}

/// Push `s` if it has content once trimmed.
fn push_trimmed(out: &mut Vec<String>, s: &str) {
    let t = s.trim();
    if !t.is_empty() {
        out.push(t.to_string());
    }
}

/// Break an over-long chunk at clause boundaries, then at words.
fn split_long(chunk: &str) -> Vec<String> {
    if chunk.chars().count() <= MAX_CHUNK {
        return vec![chunk.to_string()];
    }

    let mut out = Vec::new();
    let mut current = String::new();

    // Clause boundaries first: a comma or semicolon is a natural breath.
    for part in split_keeping(chunk, &[',', ';', '—']) {
        if current.chars().count() + part.chars().count() > MAX_CHUNK && !current.is_empty() {
            push_trimmed(&mut out, &current);
            current.clear();
        }
        current.push_str(&part);
    }
    push_trimmed(&mut out, &current);

    // Anything still too long has no punctuation to help. Fall back to words.
    out.into_iter().flat_map(|p| split_words(&p)).collect()
}

/// Split after each of `seps`, keeping the separator with its clause.
fn split_keeping(s: &str, seps: &[char]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        cur.push(c);
        if seps.contains(&c) {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Last resort: pack whole words up to the cap. Never splits a word.
fn split_words(s: &str) -> Vec<String> {
    if s.chars().count() <= MAX_CHUNK {
        return vec![s.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        if !cur.is_empty() && cur.chars().count() + 1 + word.chars().count() > MAX_CHUNK {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
