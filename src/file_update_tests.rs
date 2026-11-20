use super::*;
use crate::Hunk;
use crate::parse_patch;
use pretty_assertions::assert_eq;

/// Applies the chunks of an `Update File` hunk with the given body to `original`.
fn updated(original: &str, body: &str) -> Result<String, ApplyPatchError> {
    let patch = format!("*** Begin Patch\n*** Update File: f.txt\n{body}\n*** End Patch");
    let hunks = parse_patch(&patch).unwrap();
    let [Hunk::UpdateFile { chunks, .. }] = hunks.as_slice() else {
        panic!("expected a single UpdateFile hunk");
    };
    derive_new_contents_from_chunks("f.txt", original, chunks)
}

#[test]
fn chunks_with_leading_context_replace_separate_lines() {
    assert_eq!(
        updated(
            "foo\nbar\nbaz\nqux\n",
            "@@\n foo\n-bar\n+BAR\n@@\n baz\n-qux\n+QUX"
        )
        .unwrap(),
        "foo\nBAR\nbaz\nQUX\n"
    );
}

#[test]
fn replacement_reaches_the_first_and_the_last_line() {
    assert_eq!(
        updated("foo\nbar\nbaz\n", "@@\n-foo\n+FOO\n bar").unwrap(),
        "FOO\nbar\nbaz\n"
    );

    assert_eq!(
        updated("foo\nbar\nbaz\n", "@@\n foo\n bar\n-baz\n+BAZ").unwrap(),
        "foo\nbar\nBAZ\n"
    );
}

#[test]
fn addition_only_chunk_appends_at_end_of_file() {
    assert_eq!(
        updated("foo\nbar\nbaz\n", "@@\n+quux\n*** End of File").unwrap(),
        "foo\nbar\nbaz\nquux\n"
    );
}

#[test]
fn end_of_file_chunk_with_context_follows_earlier_replacements() {
    assert_eq!(
        updated(
            "a\nb\nc\nd\ne\nf\n",
            "@@\n a\n-b\n+B\n@@\n c\n d\n-e\n+E\n@@\n f\n+g\n*** End of File"
        )
        .unwrap(),
        "a\nB\nc\nd\nE\nf\ng\n"
    );
}

#[test]
fn addition_only_chunk_before_a_removal_still_appends() {
    assert_eq!(
        updated(
            "line1\nline2\nline3\n",
            "@@\n+after-context\n+second-line\n@@\n line1\n-line2\n-line3\n+line2-replacement"
        )
        .unwrap(),
        "line1\nline2-replacement\nafter-context\nsecond-line\n"
    );
}

#[test]
fn ascii_punctuation_matches_typographic_punctuation() {
    assert_eq!(
        updated(
            "import asyncio  # local import \u{2013} avoids top\u{2011}level dep\n",
            "@@\n-import asyncio  # local import - avoids top-level dep\n+import asyncio  # HELLO"
        )
        .unwrap(),
        "import asyncio  # HELLO\n"
    );
}
