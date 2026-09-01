//! Every ugly case in `docs/text.md`, as table rows.

use voicecast_text::{Rejection, chunk, strip, validate};

// ---------------------------------------------------------------- validation

#[test]
fn rejects_markdown() {
    let cases = [
        ("Updated **3 files**.", "bold emphasis"),
        ("Updated __3 files__.", "bold emphasis"),
        ("See `foo.rs` for it.", "inline code"),
        ("Look at *this* bit.", "italic emphasis"),
        ("# Heading here", "heading"),
        ("## Another heading", "heading"),
        ("- a list item", "list marker"),
        ("1. an ordered item", "list marker"),
        ("> a quotation", "blockquote"),
        ("See [the docs](/x) now.", "link"),
        ("| a | b |\n| - | - |", "table row"),
        ("```\ncode\n```", "code block"),
    ];
    for (input, want_kind) in cases {
        let err = validate(input).expect_err(&format!("should reject: {input:?}"));
        assert_eq!(err.kind(), want_kind, "wrong kind for {input:?}");
    }
}

#[test]
fn rejects_bare_urls() {
    for input in [
        "See https://example.com/a?b=1 for details.",
        "Try http://localhost:8080 instead.",
        "Go to www.example.com now.",
    ] {
        let err = validate(input).expect_err(&format!("should reject: {input:?}"));
        assert!(
            matches!(err, Rejection::Url { .. }),
            "wrong variant for {input:?}"
        );
    }
}

#[test]
fn accepts_speakable_prose() {
    // None of these should trip the validator. The false positives that would
    // make strict validation intolerable all live here.
    for input in [
        "Build finished.",
        "Tests passed. Deploy when ready.",
        "It's the user's own device.",
        "Deploy to 10.0.0.1 finished.",
        "See src/main.rs:42 for the fix.",
        "Dr. Chen approved v1.2.3.",
        "The value is 2 * 3 * 4 apparently.", // arithmetic, not emphasis
        "The flag is snake_case_name in config.", // identifier, not emphasis
        "Done \u{1F389} nicely.",             // emoji is stripped, never rejected
        "Cost was $1.5M this quarter.",
        "Wait... that isn't right.",
    ] {
        assert!(validate(input).is_ok(), "should accept: {input:?}");
    }
}

#[test]
fn reports_the_first_problem_in_document_order() {
    // An agent fixing errors one at a time should make forward progress, so
    // whichever problem comes first in the text is the one reported.
    let md_first = validate("plain **bold** then https://x.com").unwrap_err();
    assert!(
        matches!(md_first, Rejection::Markdown { .. }),
        "got {md_first:?}"
    );

    let url_first = validate("see https://x.com then **bold**").unwrap_err();
    assert!(
        matches!(url_first, Rejection::Url { .. }),
        "got {url_first:?}"
    );
}

#[test]
fn span_points_at_the_offending_text() {
    let input = "Updated **3 files** today.";
    let err = validate(input).unwrap_err();
    let (s, e) = err.span();
    assert_eq!(&input[s..e], "**3 files**");
}

// ------------------------------------------------------------------ chunking

#[test]
fn does_not_split_inside_protected_spans() {
    // The naive split-on-period failures from docs/text.md.
    let cases = [
        ("Deploy to 10.0.0.1 finished.", 1),
        ("See src/main.rs:42 for the fix.", 1),
        ("Dr. Chen approved the change.", 1),
        ("Use v1.2.3 or later.", 1),
        ("The value is 3.14 exactly.", 1),
        ("Sort by e.g. name or date.", 1),
        ("Wait... it finished.", 1),
    ];
    for (input, want) in cases {
        let got = chunk(input);
        assert_eq!(got.len(), want, "{input:?} split into {got:?}");
    }
}

#[test]
fn splits_real_sentences() {
    let got = chunk("Build finished. Tests passed. Deploy when ready.");
    assert_eq!(got.len(), 3, "got {got:?}");
    assert!(got[0].starts_with("Build"));
    assert!(got[2].starts_with("Deploy"));
}

#[test]
fn handles_text_with_no_terminal_punctuation() {
    assert_eq!(chunk("build finished"), vec!["build finished"]);
}

#[test]
fn empty_input_yields_no_chunks() {
    assert!(chunk("").is_empty());
    assert!(chunk("   \n  ").is_empty());
}

#[test]
fn long_text_falls_back_and_never_splits_a_word() {
    let word = "supercalifragilistic";
    let input = std::iter::repeat_n(word, 40).collect::<Vec<_>>().join(" ");
    let got = chunk(&input);
    assert!(got.len() > 1, "should have split");
    for c in &got {
        assert!(
            c.chars().count() <= 200,
            "chunk too long: {}",
            c.chars().count()
        );
        for w in c.split_whitespace() {
            assert_eq!(w, word, "a word was split: {w:?}");
        }
    }
}

#[test]
fn long_sentence_prefers_clause_boundaries() {
    let clause = "the quick brown fox jumps over the lazy dog again and again, ";
    let input = clause.repeat(6);
    for c in chunk(&input) {
        assert!(c.chars().count() <= 200);
    }
}

// -------------------------------------------------------------------- strip

#[test]
fn strip_produces_speakable_text() {
    let cases = [
        ("Updated **3 files**.", "Updated 3 files."),
        ("See `foo.rs` now.", "See foo.rs now."),
        ("# Release notes", "Release notes"),
        ("- first item", "first item"),
        ("1. first item", "first item"),
        ("> quoted text", "quoted text"),
        ("See [the docs](https://x.com/y) now.", "See the docs now."),
        (
            "Visit https://example.com/a/b?c=1 today.",
            "Visit example.com today.",
        ),
        ("Done \u{1F389} nicely.", "Done nicely."),
    ];
    for (input, want) in cases {
        assert_eq!(strip(input), want, "stripping {input:?}");
    }
}

#[test]
fn strip_summarises_code_blocks() {
    let got = strip("Here:\n```\nlet x = 1;\nlet y = 2;\n```\nDone.");
    assert!(got.contains("code block, 2 lines"), "got {got:?}");
    assert!(
        !got.contains("let x"),
        "should not read code aloud: {got:?}"
    );
}

#[test]
fn stripped_output_passes_validation() {
    // The rewrite suggested in an error message must itself be acceptable,
    // or an agent following our advice would just get rejected again.
    for input in [
        "Updated **3 files**. See `src/main.rs` for details.",
        "# Heading\n- item one\n- item two",
        "See [docs](https://example.com/guide) for more.",
    ] {
        let stripped = strip(input);
        assert!(
            validate(&stripped).is_ok(),
            "strip({input:?}) = {stripped:?} which still fails validation"
        );
    }
}

#[test]
fn stripped_output_can_be_chunked() {
    let got = chunk(&strip("# Notes\n\nBuild **finished**. See `log.txt`."));
    assert!(!got.is_empty());
    assert!(got.iter().all(|c| !c.contains('*') && !c.contains('`')));
}
