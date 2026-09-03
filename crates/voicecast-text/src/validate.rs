//! Rejecting text that will not read well aloud.
//!
//! Strict by default. The caller is an agent, and an agent can act on an error
//! message — read it, fix the text, retry. That makes rejection a working
//! feedback channel rather than an obstacle, and it produces better speech
//! than stripping ever could: text *written* to be spoken beats markdown
//! cleaned up afterwards.
//!
//! The dividing line: **error when there is an obvious correction, handle it
//! quietly when there is not.** Markdown has a plain-prose alternative. Emoji
//! does not, so an error on it would be a puzzle with no answer.

use crate::Rejection;

/// Longest run we will look ahead for a closing delimiter.
///
/// Bounds the scan so a lone `*` in ordinary prose cannot make us walk the
/// rest of a long document looking for a partner it never had.
const MAX_SPAN: usize = 200;

/// Reject text containing markdown or a bare URL.
///
/// Returns the **first** problem in document order, so an agent fixing errors
/// one at a time makes forward progress rather than jumping around.
pub fn validate(text: &str) -> Result<(), Rejection> {
    let md = detect_markdown(text);
    let url = detect_url(text);

    match (md, url) {
        (Some(m), Some(u)) => Err(if m.span().0 <= u.span().0 { m } else { u }),
        (Some(m), None) => Err(m),
        (None, Some(u)) => Err(u),
        (None, None) => Ok(()),
    }
}

/// First markdown construct in the text, if any.
fn detect_markdown(text: &str) -> Option<Rejection> {
    let b = text.as_bytes();
    let mut best: Option<Rejection> = None;

    let mut consider = |r: Rejection| {
        if best.as_ref().is_none_or(|b| r.span().0 < b.span().0) {
            best = Some(r);
        }
    };

    // Line-anchored constructs: headings, list markers, blockquotes, tables.
    for (start, line) in line_spans(text) {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let at = start + indent;

        if let Some(kind) = line_marker(trimmed) {
            let len = marker_len(trimmed);
            consider(Rejection::Markdown {
                span: (at, at + len),
                kind,
            });
        } else if is_table_row(trimmed) {
            consider(Rejection::Markdown {
                span: (start, start + line.len()),
                kind: "table row",
            });
        }
    }

    // Paired delimiters. Fences before inline code, and doubles before
    // singles, so ``` is never reported as ` and ** is never reported as *.
    for (delim, kind) in [
        ("```", "code block"),
        ("**", "bold emphasis"),
        ("__", "bold emphasis"),
        ("`", "inline code"),
    ] {
        if let Some(span) = find_paired(b, delim.as_bytes()) {
            consider(Rejection::Markdown { span, kind });
        }
    }

    // Single-character emphasis needs word-boundary care: `foo_bar_baz` is an
    // identifier, and `2 * 3 * 4` is arithmetic. Only flag when the delimiters
    // hug the enclosed text.
    for (delim, kind) in [(b'*', "italic emphasis"), (b'_', "italic emphasis")] {
        if let Some(span) = find_emphasis(b, delim) {
            consider(Rejection::Markdown { span, kind });
        }
    }

    if let Some(span) = find_link(b) {
        consider(Rejection::Markdown { span, kind: "link" });
    }

    best
}

/// Byte offset and content of each line.
fn line_spans(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut off = 0;
    text.split('\n').map(move |line| {
        let start = off;
        off += line.len() + 1;
        (start, line)
    })
}

/// Heading, list, or blockquote marker at the start of a trimmed line.
fn line_marker(t: &str) -> Option<&'static str> {
    let b = t.as_bytes();
    if b.first() == Some(&b'#') {
        let hashes = b.iter().take_while(|&&c| c == b'#').count();
        if hashes <= 6 && b.get(hashes) == Some(&b' ') {
            return Some("heading");
        }
    }
    if b.first() == Some(&b'>') && matches!(b.get(1), Some(b' ') | None) {
        return Some("blockquote");
    }
    if matches!(b.first(), Some(b'-' | b'*' | b'+')) && b.get(1) == Some(&b' ') {
        return Some("list marker");
    }
    // Ordered list: one or two digits, then `. `.
    //
    // Capped at two because a longer run of digits is a number, not a
    // position in a list: "2024. was a good year" is a sentence somebody
    // means to have read aloud, and refusing it with "list marker" sent
    // them looking for markdown that was never there (#65). Lists past
    // ninety-nine items exist; they are not what anyone dictates.
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if (1..=2).contains(&digits) && b.get(digits) == Some(&b'.') && b.get(digits + 1) == Some(&b' ')
    {
        return Some("list marker");
    }
    None
}

/// Length of the marker `line_marker` matched.
fn marker_len(t: &str) -> usize {
    let b = t.as_bytes();
    if b.first() == Some(&b'#') {
        return b.iter().take_while(|&&c| c == b'#').count() + 1;
    }
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        return digits + 2;
    }
    2
}

/// A markdown table row: pipes around at least two cells.
fn is_table_row(t: &str) -> bool {
    t.starts_with('|') && t.matches('|').count() >= 3
}

/// Opening and closing run of `delim`, if both are present.
fn find_paired(b: &[u8], delim: &[u8]) -> Option<(usize, usize)> {
    let open = find_at(b, delim, 0)?;
    let close = find_at(b, delim, open + delim.len())?;
    if close - open > MAX_SPAN {
        return None;
    }
    Some((open, close + delim.len()))
}

/// `*text*` or `_text_`, where the delimiters hug non-space content.
///
/// Rejects `2 * 3` (spaces inside) and `snake_case_name` (no space before the
/// opener), which are the two ways this check earns false positives.
fn find_emphasis(b: &[u8], d: u8) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < b.len() {
        if b[i] == d
            && (i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\n')
            && b.get(i + 1).is_some_and(|c| *c != b' ' && *c != d)
        {
            let mut j = i + 1;
            while j < b.len() && j - i <= MAX_SPAN {
                if b[j] == d && b[j - 1] != b' ' {
                    return Some((i, j + 1));
                }
                if b[j] == b'\n' {
                    break;
                }
                j += 1;
            }
        }
        i += 1;
    }
    None
}

/// A `[text](target)` link.
fn find_link(b: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'['
            && let Some(close) = find_at(b, b"](", i)
            && let Some(end) = find_at(b, b")", close)
            && end - i <= MAX_SPAN
        {
            return Some((i, end + 1));
        }
        i += 1;
    }
    None
}

/// First bare URL, if any.
///
/// Not markdown, but rejected on the same logic: reading
/// `https://example.com/a/b?c=1#d` aloud is unbearable, and "the deploy
/// dashboard" is the obvious thing the agent should have written instead.
fn detect_url(text: &str) -> Option<Rejection> {
    let b = text.as_bytes();
    for prefix in [&b"https://"[..], b"http://", b"www."] {
        let mut from = 0;
        while let Some(start) = find_at(b, prefix, from) {
            from = start + 1;
            // `www.` has to begin a word and be followed by a host label.
            // Matching it anywhere rejected "Awww. Cute." as a URL, and
            // `strip` only removes a `www.` *prefix*, so the suggested
            // rewrite came back identical to the input and the caller was
            // told to fix something with no way to fix it (#65). The two
            // scheme prefixes need neither test: nothing says "https://"
            // by accident.
            if prefix == b"www." {
                let starts_word = start == 0 || !b[start - 1].is_ascii_alphanumeric();
                let has_host = b.get(start + 4).is_some_and(u8::is_ascii_alphanumeric);
                if !starts_word || !has_host {
                    continue;
                }
            }
            let end = b[start..]
                .iter()
                .position(|c| matches!(c, b' ' | b'\n' | b'\t'))
                .map_or(b.len(), |p| start + p);
            return Some(Rejection::Url { span: (start, end) });
        }
    }
    None
}

/// Index of `needle` in `hay` at or after `from`.
fn find_at(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}
