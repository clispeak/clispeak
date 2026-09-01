//! Converting markdown into something worth hearing.
//!
//! Used two ways: by `--strip`, and to build the suggested rewrite inside a
//! rejection message. The second matters most — an agent can resend the
//! suggestion verbatim, which is what makes strict validation a feedback
//! channel rather than a wall.

/// Render markdown-ish text as plain speakable prose.
///
/// Emphasis and code markers are dropped, links become their text, headings
/// and list markers are removed, code blocks are summarised by line count, and
/// URLs are reduced to their host. Emoji are dropped silently — there is no
/// correct way to say them, which is exactly why they are stripped rather than
/// rejected.
pub fn strip(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();

        // Fenced code block: summarise rather than read the symbols aloud.
        if trimmed.starts_with("```") {
            let mut count = 0;
            for body in lines.by_ref() {
                if body.trim_start().starts_with("```") {
                    break;
                }
                count += 1;
            }
            out.push_str(&format!("code block, {count} lines. "));
            continue;
        }

        let line = strip_line_markers(trimmed);
        out.push_str(&strip_inline(&line));
        out.push(' ');
    }

    collapse_whitespace(&out)
}

/// Remove heading hashes, list bullets, and blockquote markers.
fn strip_line_markers(t: &str) -> String {
    let b = t.as_bytes();
    if b.first() == Some(&b'#') {
        let n = b.iter().take_while(|&&c| c == b'#').count();
        if n <= 6 {
            return t[n..].trim_start().to_string();
        }
    }
    if b.first() == Some(&b'>') {
        return t[1..].trim_start().to_string();
    }
    if matches!(b.first(), Some(b'-' | b'*' | b'+')) && b.get(1) == Some(&b' ') {
        return t[2..].to_string();
    }
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && b.get(digits) == Some(&b'.') && b.get(digits + 1) == Some(&b' ') {
        return t[digits + 2..].to_string();
    }
    t.to_string()
}

/// Drop emphasis and code markers, unwrap links, shorten URLs, drop emoji.
fn strip_inline(line: &str) -> String {
    let mut s = unwrap_links(line);
    s = shorten_urls(&s);

    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '_' | '`' => {
                // Swallow a doubled marker as one.
                if chars.peek() == Some(&c) {
                    chars.next();
                }
            }
            c if is_emoji(c) => {}
            c => out.push(c),
        }
    }
    out
}

/// `[text](target)` becomes `text`.
fn unwrap_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        if let Some(mid) = rest[open..].find("](")
            && let Some(close) = rest[open + mid..].find(')')
        {
            out.push_str(&rest[..open]);
            out.push_str(&rest[open + 1..open + mid]);
            rest = &rest[open + mid + close + 1..];
            continue;
        }
        out.push_str(&rest[..=open]);
        rest = &rest[open + 1..];
    }
    out.push_str(rest);
    out
}

/// `https://example.com/a/b?c=1` becomes `example.com`.
///
/// A spoken URL fails at conveying anything in every case worth having.
fn shorten_urls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, word) in s.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let host = word
            .strip_prefix("https://")
            .or_else(|| word.strip_prefix("http://"))
            .or_else(|| word.strip_prefix("www."));
        match host {
            Some(h) => out.push_str(h.split(['/', '?', '#']).next().unwrap_or(h)),
            None => out.push_str(word),
        }
    }
    out
}

/// Pictographic characters, variation selectors, and joiners.
fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F300..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF
        | 0x1F1E6..=0x1F1FF | 0xFE0F | 0x200D | 0x2190..=0x21FF)
}

/// Squeeze runs of whitespace and trim.
fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
