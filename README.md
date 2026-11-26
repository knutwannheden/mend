# patchwork

`patchwork` applies a multi-file edit written in the `*** Begin Patch` format of [OpenAI Codex](https://github.com/openai/codex). Either every file the patch names is changed, or none is. It is a small synchronous binary for coding agents and scripts, derived from the `apply-patch` crate of Codex.

```sh
patchwork <<'EOF' && ./gradlew test
*** Begin Patch
*** Update File: src/Greeter.java
@@ class Greeter
-    String greeting = "hello";
+    String greeting = "hi";
*** Add File: docs/greeting.md
+The greeting is "hi".
*** End Patch
EOF
```

```
M src/Greeter.java
A docs/greeting.md
```

## Usage

```
patchwork [-C <dir>] [--check] [--diff] [--fragment] [--all] [--then <command> [--revert]] [PATCH]
```

The patch is read from the `PATCH` argument, or from stdin when there is none. A relative path in the patch is resolved against the current directory.

| Option | Effect |
|---|---|
| `-C <dir>` | Resolves the relative paths in the patch against `<dir>`. |
| `--check` | Reports what the patch would change without writing anything. |
| `--diff` | Prints a diff in git's format instead of the status lines. `git apply -R` reverses it. |
| `--fragment` | Lets a chunk of one `-` and one `+` line replace part of a line. It applies when no whole line matches, and the part has to occur exactly once where it is looked for. Several chunks may replace different parts of one line. |
| `--all` | Lets a chunk that matches several places change all of them. Standard error then names the lines. |
| `--then <command>` | Runs `<command>` with `sh -c` once the patch is applied. When the command fails the edits stay in place. |
| `--revert` | With `--then`, restores every file when the command has finished, whether it failed or succeeded. |

Keep the quotes in `<<'EOF'`. In an unquoted heredoc the shell expands every `$name` in the patch and runs whatever stands in backticks, before `patchwork` reads it. For a long shared path prefix use `-C <dir>` instead of a shell variable.

To run a build or test only when the patch applied, chain it with `&&` on the heredoc line, as in the example above. A long command can follow the closing `EOF` when the heredoc line ends with `&&`. A command that merely sits on the next line runs even when the patch failed.

`--then` is there for `--revert`, which makes an edit last for one run, such as a mutation check. The files are also restored when `patchwork` receives `SIGHUP`, `SIGINT` or `SIGTERM` while the command runs.

```sh
patchwork --revert --then 'cargo test parser' < mutant.patch
```

## Output

Standard output carries one line per changed path, in the order the patch names them. Diagnostics go to standard error.

| Line | Meaning |
|---|---|
| `A <path>` | The path did not exist and now does. |
| `M <path>` | The path existed and its contents changed. |
| `D <path>` | The path existed and is gone. |

A move prints `D` for the source and `A` or `M` for the destination. A path whose contents end up unchanged prints nothing. With `--then`, the command's own output follows these lines.

## Exit status

| Status | Meaning |
|---|---|
| 0 | The patch applied, and the `--then` command succeeded. |
| 1 | The patch does not fit the files. A chunk matched no place or several, or a named file is missing. Nothing was written. |
| 2 | The patch or the command line is malformed. Nothing was written. |
| 3 | The `--then` command failed. The edits are kept unless `--revert` was given. |
| 4 | A file could not be read or written, and no file was left changed. Or `--revert` left a file alone because it changed after the patch was applied. |
| 128 + n | With `--revert`, signal `n` arrived while the command ran. The files were restored. |

## Matching

Each chunk of an `Update File` hunk has to match exactly one place. It is looked for in three steps, and the first step that has a match decides:

1. below its own `@@` anchor, or below the previous chunk when it has none, and above the next `@@` anchor of a later chunk
2. from the same start down to the end of the file
3. in the whole file, so a chunk may come before the previous one in the file, and may repeat its anchor

An anchor before and after a chunk therefore tells apart text that occurs more than once. Lines are compared exactly first, then ignoring trailing whitespace, then ignoring leading and trailing whitespace, then with typographic punctuation folded to ASCII. The strictest comparison that matches anywhere decides.

A chunk with its own anchor names a section, so steps 2 and 3 do not apply to it: a match only below the next anchor, or above its own, is rejected. Of several places in its section, the one that starts on the line right below its anchor wins. A chunk whose own lines include its anchor line is placed with that line on the anchor, so it may change or remove the anchor line, or add lines before it.

When a chunk's lines all leave out the same indentation that the file's lines have, its added lines get that indentation too.

A chunk that matches several places is rejected with their line numbers.

```
src/app.py: chunk 1 of 1 matches 2 places, at lines 14, 52
  add context lines or an `@@` anchor line so that it matches only one
```

A chunk that matches nothing reports where it comes closest.

```
src/main.rs: chunk 2 of 3 matches nothing
  closest is at line 40 (2 of 3 lines match); line 41 differs:
    patch:     let x = foo(bar);
    file:      let x = foo(baz);
```

## Patch format

```
*** Begin Patch
*** Add File: <path>
+<line>
*** Delete File: <path>
*** Update File: <path>
*** Move to: <new path>
@@ <anchor line>
 <context line>
-<removed line>
+<added line>
*** End of File
*** End Patch
```

`*** Move to:`, the `@@` anchor and `*** End of File` are optional. An `@@` anchor gives a whole line, or just the start of exactly one line, such as a method signature up to its `(`. A chunk without removed or context lines puts its added lines right after its `@@` anchor line, or at the end of the file when it has no anchor or ends with `*** End of File`. To insert before a line, give that line as a context line after the `+` lines. Before a function with a comment above it, that line is the comment's first line. Added lines that end a chunk with a copy of the line just above them are rejected, since that is how an insertion meant to go before the line usually looks. So are added lines that begin with a copy of the two lines just below them, which is how lines retyped as if they were context look, and added lines that go between two lines of a `//`, `# ` or `-- ` comment without being comment lines themselves. `Add File` on a path that exists replaces its whole contents. `tests/fixtures/scenarios` holds one directory per behavior, each with the input files, the patch and the expected result.

## Differences from Codex `apply_patch`

- A patch that fails leaves no file changed. Codex writes each file as it goes.
- A chunk that matches several places is an error. Codex edits the first.
- A chunk of only `+` lines under an `@@` anchor goes right after the anchor line. Codex appends it to the end of the file.
- Chunks of one file need not be in file order, and several chunks may name the same `@@` anchor. Codex rejects both.
- Standard output has no `Success.` header, and failures have distinct exit statuses.
- A patch has to reach the command through a shell. The heredoc text `<<'EOF'` passed literally as an argument is not unwrapped, and the `*** Environment ID:` line is not accepted.

## Instructions for your agent

An agent will not pick the tool up by itself, and it does not read `--help` unprompted. Put a section like this in `CLAUDE.md`, `AGENTS.md` or whatever file your agent reads.

````markdown
## Editing files

To change an existing file, pipe a patch to `patchwork` instead of using your edit tool or a script that swaps one piece of text for another. It checks every chunk before writing anything, and restores the files already written if a later write fails.

    patchwork <<'EOF' && make test
    *** Begin Patch
    *** Update File: src/config.py
    @@ def load():
    -    retries = 3
    +    retries = 5
    *** Add File: notes/retries.md
    +Retries were raised to 5.
    *** End Patch
    EOF

- Run whatever depends on the edit with `&&` on the heredoc line. A command on the next line runs even when the patch failed.
- Always quote the heredoc delimiter, as in `<<'EOF'`. Unquoted, the shell expands every `$name` and runs whatever stands in backticks inside the patch. For a shared path prefix pass `-C <dir>`.
- Each chunk must match exactly one place. An `@@ <line>` anchor names a section: the chunk has to match below that line and above the next anchor. It may give just the start of its line, such as a method signature up to `(`, as long as only one line starts that way. A chunk of only `+` lines goes right after its anchor line. To insert before a line, give that line as a context line after the `+` lines. Before a function with a comment above it, that line is the comment's first line. On exit 1 nothing was written, and stderr says where the chunk came closest, so correct the chunk from that message.
- For a temporary edit such as a mutation check, use `--revert --then '<command>'`. It restores the files once the command has finished.
- `*** Add File:` on a path that exists replaces the whole file.
- To change a few words inside a long line, pass `--fragment` and give only that part as one `-` line and one `+` line.
- To change every place a chunk matches, pass `--all`.
- Preview a patch with `--check --diff`, which prints the change as a diff without writing it. `patchwork --help` has the rest.
````

## Building

```sh
cargo build --release
cargo test
```

## License

Apache-2.0. See `LICENSE` and `NOTICE` for the attribution to OpenAI Codex.
