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

fn error(original: &str, body: &str) -> String {
    updated(original, body).unwrap_err().to_string()
}

#[test]
fn chunk_matching_several_places_is_rejected_with_their_lines() {
    assert_eq!(
        error("x = 1\ny = 2\nx = 1\n", "@@\n-x = 1\n+x = 3"),
        "f.txt: chunk 1 of 1 matches 2 places, at lines 1, 3\n  \
         add context lines or an `@@` anchor line so that it matches only one"
    );
}

#[test]
fn anchor_selects_one_of_several_matching_places() {
    assert_eq!(
        updated(
            "def a():\n    return 1\ndef b():\n    return 1\n",
            "@@ def b():\n-    return 1\n+    return 2"
        )
        .unwrap(),
        "def a():\n    return 1\ndef b():\n    return 2\n"
    );
}

#[test]
fn chunk_matching_nothing_reports_the_closest_place_and_its_first_difference() {
    assert_eq!(
        error(
            "fn main() {\n    let x = foo(baz);\n    run(x);\n}\n",
            "@@\n fn main() {\n-    let x = foo(bar);\n+    let x = foo(qux);\n     run(x);"
        ),
        "f.txt: chunk 1 of 1 matches nothing\n  \
         closest is at line 1 (2 of 3 lines match); line 2 differs:\n    \
         patch:     let x = foo(bar);\n    \
         file:      let x = foo(baz);"
    );

    assert_eq!(
        error("alpha\nbeta\n", "@@\n-something else entirely\n+x"),
        "f.txt: chunk 1 of 1 matches nothing, and no similar lines were found"
    );
}

#[test]
fn chunk_matching_only_above_its_search_start_is_reported_as_out_of_order() {
    assert_eq!(
        error("a\nb\nc\n", "@@\n-c\n+C\n@@\n-a\n+A"),
        "f.txt: chunk 2 of 2 matches only at line 1, above line 4 where its search starts\n  \
         chunks and `@@` anchors must follow file order"
    );
}

#[test]
fn missing_anchor_is_reported() {
    assert_eq!(
        error("a\nb\n", "@@ def missing():\n-a\n+A"),
        "f.txt: chunk 1 of 1: anchor `@@ def missing():` not found"
    );

    assert_eq!(
        error("a\nb\n", "@@\n-a\n+A\n@@ def missing():\n-b\n+B"),
        "f.txt: chunk 2 of 2: anchor `@@ def missing():` not found below line 1"
    );
}
