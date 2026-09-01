//! Finding spans that must never be split.
//!
//! Sentence boundaries are not where periods are. A naive split on `.?!`
//! shreds real agent output:
//!
//! ```text
//! Deploy to 10.0.0.1 finished.     -> "Deploy to 10." / "0." / "0." / "1 finished."
//! See src/main.rs:42 for the fix.  -> shredded
//! Dr. Chen approved v1.2.3.        -> four fragments
//! ```
//!
//! Rather than an ever-growing regex, we mark the ranges that must stay
//! intact and refuse any split point inside one. That is testable: every case
//! above is a table row with an obvious expected output.

/// Abbreviations whose trailing period does not end a sentence.
const ABBREVIATIONS: &[&str] = &[
    "Dr.", "Mr.", "Mrs.", "Ms.", "Prof.", "St.", "Jr.", "Sr.", "Inc.", "Ltd.", "Co.", "Corp.",
    "e.g.", "i.e.", "etc.", "vs.", "No.", "Fig.", "Vol.", "cf.", "al.", "approx.", "min.", "max.",
];

/// Byte ranges that must not contain a split point.
pub fn protected_ranges(text: &str) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let mut out = Vec::new();

    // Dotted or colon-separated runs: 10.0.0.1, 3.14, v1.2.3, src/main.rs:42,
    // foo.rs. Anything where a period sits between two non-space characters.
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'.' && i > 0 && i + 1 < b.len() {
            let prev = b[i - 1];
            let next = b[i + 1];
            if !prev.is_ascii_whitespace() && !next.is_ascii_whitespace() {
                let start = token_start(b, i);
                let end = token_end(b, i);
                out.push((start, end));
                i = end;
                continue;
            }
        }
        i += 1;
    }

    // Known abbreviations. Case-sensitive: "No." is an abbreviation, "no." at
    // the end of a sentence is a word.
    for abbr in ABBREVIATIONS {
        let mut from = 0;
        while let Some(at) = find_at(b, abbr.as_bytes(), from) {
            let starts_token = at == 0 || b[at - 1].is_ascii_whitespace();
            if starts_token {
                out.push((at, at + abbr.len()));
            }
            from = at + 1;
        }
    }

    // Ellipses: three or more dots are one gesture, not three sentence ends.
    let mut i = 0;
    while i + 2 < b.len() {
        if &b[i..i + 3] == b"..." {
            let mut end = i + 3;
            while end < b.len() && b[end] == b'.' {
                end += 1;
            }
            out.push((i, end));
            i = end;
        } else {
            i += 1;
        }
    }

    out.sort_unstable();
    out
}

/// Whether a split at `pos` would break a protected span.
///
/// Two ways it can: the position sits strictly inside a range, or — once
/// trailing whitespace is skipped — it sits at the *end* of one. The second
/// case is the common one, because a UAX #29 segmenter keeps the trailing
/// space inside the segment: `"Dr. "` ends at 4 while the protected range for
/// `"Dr."` ends at 3. Without the skip, every abbreviation would end a
/// sentence.
pub fn is_protected(text: &str, ranges: &[(usize, usize)], pos: usize) -> bool {
    if ranges.iter().any(|&(s, e)| pos > s && pos < e) {
        return true;
    }
    let b = text.as_bytes();
    let mut p = pos.min(b.len());
    while p > 0 && b[p - 1].is_ascii_whitespace() {
        p -= 1;
    }
    p > 0 && ranges.iter().any(|&(_, e)| p == e)
}

/// Start of the whitespace-delimited token containing `i`.
fn token_start(b: &[u8], i: usize) -> usize {
    let mut s = i;
    while s > 0 && !b[s - 1].is_ascii_whitespace() {
        s -= 1;
    }
    s
}

/// End of the whitespace-delimited token containing `i`.
///
/// Trailing sentence punctuation is excluded, so `finished.` protects
/// `finished` but leaves its final period available as a boundary.
fn token_end(b: &[u8], i: usize) -> usize {
    let mut e = i;
    while e < b.len() && !b[e].is_ascii_whitespace() {
        e += 1;
    }
    while e > i && matches!(b[e - 1], b'.' | b'!' | b'?' | b',' | b';' | b':') {
        e -= 1;
    }
    e
}

/// Index of `needle` in `hay` at or after `from`.
fn find_at(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}
