use super::*;
use crate::Hunk;
use crate::parse_patch;
use pretty_assertions::assert_eq;

/// Applies the chunks of an `Update File` hunk with the given body to `original`.
fn updated(original: &str, body: &str) -> Result<String, ApplyPatchError> {
    applied(original, body, Matching::default()).map(|(contents, _)| contents)
}

/// Like `updated`, with the given leniency, and also returning the notes.
fn applied(
    original: &str,
    body: &str,
    matching: Matching,
) -> Result<(String, Vec<String>), ApplyPatchError> {
    let patch = format!("*** Begin Patch\n*** Update File: f.txt\n{body}\n*** End Patch");
    let hunks = parse_patch(&patch).unwrap();
    let [Hunk::UpdateFile { chunks, .. }] = hunks.as_slice() else {
        panic!("expected a single UpdateFile hunk");
    };
    derive_new_contents_from_chunks("f.txt", original, chunks, matching)
}

const FRAGMENTS: Matching = Matching {
    fragments: true,
    all: false,
};
const ALL: Matching = Matching {
    fragments: false,
    all: true,
};

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
fn long_lines_that_differ_are_cut_down_to_the_part_that_differs() {
    let (before, after) = ("word ".repeat(40), " tail".repeat(40));
    assert_eq!(
        error(
            &format!("{before}OMEGX{after}\n"),
            &format!("@@\n-{before}ALPHA{after}\n+x")
        ),
        "f.txt: chunk 1 of 1 matches nothing\n  \
         closest is at line 1 (0 of 1 lines match); line 1 differs at column 201:\n    \
         patch: …word word word word word word ALPHA tail tail tail tail tail tail…\n    \
         file:  …word word word word word word OMEGX tail tail tail tail tail tail…"
    );
}

#[test]
fn chunk_above_the_previous_one_applies_where_it_matches_exactly_once() {
    assert_eq!(
        updated("a\nb\nc\n", "@@\n-c\n+C\n@@\n-a\n+A").unwrap(),
        "A\nb\nC\n"
    );

    assert_eq!(
        error("a\nb\n", "@@\n-a\n+A\n@@\n-a\n+AA"),
        "f.txt: chunk 2 of 2 matches only line 1, which an earlier chunk already matched"
    );

    assert_eq!(
        error("x\ny\nx\nz\n", "@@\n-z\n+Z\n@@\n-x\n+X"),
        "f.txt: chunk 2 of 2 matches 2 places, at lines 1, 3\n  \
         add context lines or an `@@` anchor line so that it matches only one"
    );
}

#[test]
fn chunk_above_the_previous_one_adds_lines_ahead_of_the_line_that_one_changed() {
    assert_eq!(
        updated("a\nb\nc\nd\n", "@@\n-c\n+C\n@@\n a\n b\n+inserted").unwrap(),
        "a\nb\ninserted\nC\nd\n"
    );
}

#[test]
fn next_anchor_ends_the_lines_a_chunk_is_looked_for_in_first() {
    assert_eq!(
        updated(
            "def a():\n    x = 1\ndef b():\n    x = 1\n",
            "@@ def a():\n-    x = 1\n+    x = 2\n@@ def b():\n-    x = 1\n+    x = 3"
        )
        .unwrap(),
        "def a():\n    x = 2\ndef b():\n    x = 3\n"
    );

    // The only match below the search start lies below the next anchor.
    assert_eq!(
        updated(
            "t\n# one\n# two\nt\nz\n",
            "@@ # one\n-t\n+T\n@@ # two\n-z\n+Z"
        )
        .unwrap(),
        "t\n# one\n# two\nT\nZ\n"
    );
}

#[test]
fn anchor_of_a_later_chunk_ends_the_lines_when_the_next_chunk_has_none() {
    assert_eq!(
        updated(
            "fn a() {\n    x\n}\nfn b() {\n    y\n}\nfn c() {\n    x\n}\n",
            "@@ fn a() {\n-    x\n+    X\n@@\n-    y\n+    Y\n@@ fn c() {\n-    x\n+    Z"
        )
        .unwrap(),
        "fn a() {\n    X\n}\nfn b() {\n    Y\n}\nfn c() {\n    Z\n}\n"
    );
}

#[test]
fn anchor_above_the_previous_chunk_leaves_the_search_where_it_is() {
    assert_eq!(
        updated(
            "class X {\n    one = 1;\n    two = 2;\n}\n",
            "@@ class X {\n-    one = 1;\n+    one = 10;\n@@ class X {\n-    two = 2;\n+    two = 20;"
        )
        .unwrap(),
        "class X {\n    one = 10;\n    two = 20;\n}\n"
    );
}

#[test]
fn chunk_line_that_is_only_part_of_one_file_line_is_pointed_out() {
    let long = format!(
        "- [Index](f.md) — {}and the tail that changes",
        "earlier words ".repeat(12)
    );
    assert_eq!(
        error(
            &format!("{long}\nshort line\n"),
            "@@\n-and the tail that changes\n+and the tail that changed"
        ),
        "f.txt: chunk 1 of 1 matches nothing\n  \
         its line `and the tail that changes` is only part of line 1\n  \
         a chunk line has to be the whole line\n  \
         with --fragment, a chunk of one `-` and one `+` line may give a part of one"
    );
}

#[test]
fn fragment_of_one_line_is_replaced_when_fragments_are_allowed() {
    let start = format!("- [Index](f.md) — {}", "earlier words ".repeat(12));
    let (contents, notes) = applied(
        &format!("{start}and the tail that changes\nshort line\n"),
        "@@\n-and the tail that changes\n+and the tail that changed",
        FRAGMENTS,
    )
    .unwrap();

    assert_eq!(
        contents,
        format!("{start}and the tail that changed\nshort line\n")
    );
    assert_eq!(notes, Vec::<String>::new());
}

#[test]
fn fragment_is_tried_after_whole_lines_and_only_for_one_removed_and_one_added_line() {
    assert_eq!(
        applied("tail\nx tail\n", "@@\n-tail\n+TAIL", FRAGMENTS)
            .unwrap()
            .0,
        "TAIL\nx tail\n"
    );

    assert_eq!(
        applied("keep\nx tail y\n", "@@\n keep\n-tail\n+TAIL", FRAGMENTS)
            .unwrap_err()
            .to_string(),
        "f.txt: chunk 1 of 1 matches nothing\n  \
         its line `tail` is only part of line 2\n  \
         a chunk line has to be the whole line\n  \
         with --fragment, a chunk of one `-` and one `+` line may give a part of one"
    );
}

#[test]
fn fragment_follows_the_same_search_steps_as_whole_lines() {
    assert_eq!(
        applied(
            "x tail\n# one\ny tail\n# two\nz tail\n",
            "@@ # one\n-tail\n+TAIL\n@@ # two\n-z tail\n+z",
            FRAGMENTS
        )
        .unwrap()
        .0,
        "x tail\n# one\ny TAIL\n# two\nz\n"
    );
}

#[test]
fn fragment_that_occurs_more_than_once_is_rejected() {
    assert_eq!(
        applied("a tail\nb tail\n", "@@\n-tail\n+TAIL", FRAGMENTS)
            .unwrap_err()
            .to_string(),
        "f.txt: chunk 1 of 1: its line `tail` occurs in 2 places, at lines 1, 2\n  \
         give more of the line so that it occurs once"
    );
}

#[test]
fn fragments_in_different_parts_of_one_line_are_all_replaced() {
    assert_eq!(
        applied(
            "p one two three four\n",
            "@@\n-four\n+FOUR\n@@\n-one\n+ONE",
            FRAGMENTS
        )
        .unwrap()
        .0,
        "p ONE two three FOUR\n"
    );
}

#[test]
fn chunks_that_change_overlapping_parts_of_a_line_are_rejected() {
    assert_eq!(
        applied("p one two\n", "@@\n-one two\n+X\n@@\n-two\n+Y", FRAGMENTS)
            .unwrap_err()
            .to_string(),
        "f.txt: chunk 2 of 2 changes part of line 1 that an earlier chunk already changed"
    );

    assert_eq!(
        applied("p one\n", "@@\n-p one\n+P\n@@\n-one\n+ONE", FRAGMENTS)
            .unwrap_err()
            .to_string(),
        "f.txt: chunk 2 of 2 matches only line 1, which an earlier chunk already matched"
    );

    assert_eq!(
        applied("p one\n", "@@\n-one\n+ONE\n@@\n-p one\n+P", FRAGMENTS)
            .unwrap_err()
            .to_string(),
        "f.txt: chunk 2 of 2 matches only line 1, which an earlier chunk already matched"
    );
}

#[test]
fn added_lines_get_the_indentation_the_chunk_left_out() {
    assert_eq!(
        updated(
            "fn f() {\n    let x = 1;\n    let y = 2;\n}\n",
            "@@\n-let x = 1;\n+let x = 10;\n+\n+let z = 3;\n let y = 2;"
        )
        .unwrap(),
        "fn f() {\n    let x = 10;\n\n    let z = 3;\n    let y = 2;\n}\n"
    );
}

#[test]
fn added_lines_keep_their_indentation_when_the_left_out_part_differs_between_lines() {
    assert_eq!(updated("  a\n    b\n", "@@\n-a\n-b\n+c").unwrap(), "c\n");
}

#[test]
fn all_changes_every_place_a_chunk_matches_and_notes_them() {
    let (contents, notes) = applied("x = 1\ny\nx = 1\n", "@@\n-x = 1\n+x = 2", ALL).unwrap();

    assert_eq!(contents, "x = 2\ny\nx = 2\n");
    assert_eq!(
        notes,
        ["f.txt: chunk 1 of 1 changed 2 places, at lines 1, 3"]
    );
}

#[test]
fn all_stays_within_the_step_that_first_has_a_match() {
    assert_eq!(
        applied(
            "def a():\n  x\n  x\ndef b():\n  x\n",
            "@@ def a():\n-  x\n+  y\n@@ def b():\n-  x\n+  w",
            ALL
        )
        .unwrap()
        .0,
        "def a():\n  y\n  y\ndef b():\n  w\n"
    );
}

#[test]
fn all_with_fragments_replaces_every_occurrence() {
    let both = Matching {
        fragments: true,
        all: true,
    };
    assert_eq!(
        applied("a tail\nb tail tail\n", "@@\n-tail\n+T", both)
            .unwrap()
            .0,
        "a T\nb T T\n"
    );
}

#[test]
fn missing_anchor_is_reported() {
    assert_eq!(
        error("a\nb\n", "@@ def missing():\n-a\n+A"),
        "f.txt: chunk 1 of 1: anchor `@@ def missing():` not found\n  \
         an anchor has to be a whole line, or the start of exactly one line"
    );

    assert_eq!(
        error("a\nb\n", "@@\n-a\n+A\n@@ def missing():\n-b\n+B"),
        "f.txt: chunk 2 of 2: anchor `@@ def missing():` not found\n  \
         an anchor has to be a whole line, or the start of exactly one line"
    );
}

#[test]
fn anchor_may_be_the_start_of_exactly_one_line() {
    let file = "class X {\n    int a() {\n        return 1;\n    }\n\n    \
                private static int b(int x, int y) {\n        return 1;\n    }\n}\n";
    assert_eq!(
        updated(
            file,
            "@@ private static int b(\n-        return 1;\n+        return 2;"
        )
        .unwrap(),
        file.replacen(
            "        return 1;\n    }\n}",
            "        return 2;\n    }\n}",
            1
        )
    );

    assert_eq!(
        error(
            "    static int a() {\n    }\n    static int b() {\n    }\n",
            "@@ static int\n-    }\n+    };"
        ),
        "f.txt: chunk 1 of 1: anchor `@@ static int` is the start of 2 lines, at lines 1, 3\n  \
         give more of the line so that it starts only one"
    );
}

#[test]
fn removed_blank_line_that_the_file_lacks_matches_nothing() {
    assert_eq!(
        error("one\ntwo\n", "@@\n-\n+foo"),
        "f.txt: chunk 1 of 1 matches nothing, and no similar lines were found"
    );
}

#[test]
fn end_of_file_chunk_that_matches_elsewhere_reports_where() {
    assert_eq!(
        error("a\nb\n", "@@\n-a\n+A\n*** End of File"),
        "f.txt: chunk 1 of 1 matches line 1, but not at the end of the file\n  \
         drop `*** End of File` to change it there"
    );
}

#[test]
fn byte_order_mark_stays_in_place_and_out_of_the_first_line() {
    assert_eq!(
        updated(
            "\u{feff}using System;\nclass A {}\n",
            "@@\n-using System;\n+using System.IO;"
        )
        .unwrap(),
        "\u{feff}using System.IO;\nclass A {}\n"
    );
}

#[test]
fn addition_only_chunk_goes_right_after_its_anchor() {
    let file = "class A {\n  int x;\n}\nclass B {\n}\n";
    assert_eq!(
        updated(file, "@@ class A {\n+  int y;").unwrap(),
        "class A {\n  int y;\n  int x;\n}\nclass B {\n}\n"
    );

    assert_eq!(
        updated(
            file,
            "@@ class A {\n-  int x;\n+  int z;\n@@ class A {\n+  int y;"
        )
        .unwrap(),
        "class A {\n  int y;\n  int z;\n}\nclass B {\n}\n"
    );

    assert_eq!(
        updated(file, "@@ class A {\n+  int y;\n*** End of File").unwrap(),
        "class A {\n  int x;\n}\nclass B {\n}\n  int y;\n"
    );
}

#[test]
fn added_lines_that_repeat_the_line_above_them_are_rejected() {
    let file = "fn a() {\n}\n\nfn pct(n: u64) {\n}\n";
    let hint = "\n  to insert before line 4, give it as a context line after the `+` lines";
    assert_eq!(
        error(
            file,
            "@@\n fn pct(n: u64) {\n+fn helper() {}\n+\n+fn pct(n: u64) {"
        ),
        format!("f.txt: chunk 1 of 1 repeats line 4 at the end of its added lines{hint}")
    );

    assert_eq!(
        error(file, "@@ fn pct(\n+fn helper() {}\n+\n+fn pct(n: u64) {"),
        format!("f.txt: chunk 1 of 1 repeats line 4 at the end of its added lines{hint}")
    );
}

#[test]
fn added_lines_may_repeat_a_closing_brace_or_put_the_copy_first() {
    let file = "fn a() {\n}\n";
    assert_eq!(
        updated(file, "@@\n }\n+fn b() {\n+}").unwrap(),
        "fn a() {\n}\nfn b() {\n}\n"
    );

    assert_eq!(
        updated(file, "@@\n+fn a() {\n+}\n fn a() {").unwrap(),
        "fn a() {\n}\nfn a() {\n}\n"
    );
}
