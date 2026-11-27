# mend

`mend` applies a multi-file edit written in the `*** Begin Patch` format of [OpenAI Codex](https://github.com/openai/codex). Either every file the patch names is changed, or none is. It is a small synchronous binary for coding agents and scripts, derived from the `apply-patch` crate of Codex.

```sh
mend <<'EOF' && ./gradlew test
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

## Installing

With Rust 1.88 or later, from crates.io:

```sh
cargo install mend
```

Or the latest commit:

```sh
cargo install --git https://github.com/knutwannheden/mend
```

It is tested on Linux and macOS.

## Writing a patch

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

A path is absolute or relative to the current directory. `Add File` creates a file from the `+` lines that follow, and replaces the whole file when the path exists. `Delete File` deletes one. `Update File` changes one through chunks, and an optional `*** Move to:` right after it also renames the file.

Each chunk starts with `@@`. Its context lines start with a space, its removed lines with `-` and its added lines with `+`. The context and removed lines are what the chunk matches in the file, and the added lines go exactly where they stand among them. The chunks of one file may come in any order, and several may name the same anchor.

### Anchors

The text after `@@` is the chunk's anchor. It names a line of the file, whole or by its start, such as a function signature up to its `(`. A bare `@@` gives no anchor.

A chunk is placed below its anchor and above the next anchor of a later chunk, so an anchor tells apart text that occurs more than once. This changes the line in `save` even when `load` has the same one:

```
@@ def save():
-    retries = 3
+    retries = 5
```

An anchor that names several lines stands for the first of them below the previous chunk, and the chunk's own lines have to pick out one place below it.

A chunk whose own lines include its anchor line is placed with that line on the anchor, so it can change the anchor line or add lines before it:

```
@@ def load():
-def load():
+def load(path):
```

A chunk of only `+` lines goes right after its anchor line, so its anchor has to name only one line:

```
@@ import os
+import sys
```

To insert before a line, name it as the anchor and repeat it as a context line after the `+` lines. Before a function with a comment above it, name the function and start the context with the comment's first line, or the added lines go between the comment and the function:

```
@@ def load():
+def exists(path):
+    return os.path.exists(path)
+
 # Reads the saved state.
 def load():
```

`*** End of File` after a chunk makes it match at the end of the file. A chunk of only `+` lines goes there when it ends with `*** End of File` or has no anchor.

## How a chunk finds its place

Each chunk has to match exactly one place. It is looked for in three steps, and the first step that has a match decides:

1. below its own anchor, or below the previous chunk when it has none, and above the next anchor of a later chunk
2. from the same start down to the end of the file
3. in the whole file, so a chunk may come before the previous one in the file

A chunk with its own anchor has to match between that anchor and the next one, so steps 2 and 3 do not apply to it. Of several places there, the one that starts on the line right below its anchor wins.

Lines are compared exactly first, then ignoring trailing whitespace, then ignoring leading and trailing whitespace, then with typographic punctuation folded to ASCII. The strictest comparison that matches anywhere decides. When a chunk's lines all leave out the same indentation that the file's lines have, its added lines get that indentation too.

Two options relax the rules. `--fragment` lets a chunk of one `-` and one `+` line replace part of a line. It applies when no whole line matches, and the part has to occur exactly once where it is looked for. Several chunks may replace different parts of one line. `--all` lets a chunk that matches several places change all of them, and standard error then names the lines.

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

## Mistakes it rejects

Some patches fit the files but seldom do what was meant. They are rejected like a chunk that matches nothing:

- Added lines that end a chunk with a copy of the line just above them, when that line opens a block, one the next line is indented deeper than. That is how an insertion meant to go before the line usually looks.
- Added lines that begin with a copy of the two lines just below them. That is how lines retyped as if they were context look.
- Added lines that go between two lines of a `//`, `# ` or `-- ` comment without being comment lines themselves.
- Added lines right below a line that opens a block, when they are not indented deeper themselves.
- Added lines that would go inside lines another chunk replaces. The two chunks belong together as one.
- A path below a file, such as `a/b` when `a` is a file. A file and a directory of the same name take two patches, one that deletes the file and one that adds the paths under it.
- Changes to both a symbolic link and the file it points to, which are one file under two names.

## Running it

```
mend [-C <dir>] [--check] [--diff] [--fragment] [--all] [--then <command> [--revert]] [PATCH]
```

The patch is read from the `PATCH` argument, or from stdin when there is none.

| Option | Effect |
|---|---|
| `-C <dir>` | Resolves the relative paths in the patch against `<dir>`. |
| `--check` | Reports what the patch would change without writing anything. |
| `--diff` | Prints a diff in git's format instead of the status lines. `git apply -R` reverses it. |
| `--fragment` | Lets a chunk of one `-` and one `+` line replace part of a line. |
| `--all` | Lets a chunk that matches several places change all of them. |
| `--then <command>` | Runs `<command>` with `sh -c` once the patch is applied. When the command fails the edits stay in place. |
| `--revert` | With `--then`, restores every file when the command has finished, whether it failed or succeeded. |

Keep the quotes in `<<'EOF'`. In an unquoted heredoc the shell expands every `$name` in the patch and runs whatever stands in backticks, before `mend` reads it. For a long shared path prefix use `-C <dir>` instead of a shell variable.

To run a build or test only when the patch applied, chain it with `&&` on the heredoc line, as in the first example. A long command can follow the closing `EOF` when the heredoc line ends with `&&`. A command that merely sits on the next line runs even when the patch failed.

`--then` is there for `--revert`, which makes an edit last for one run, such as a mutation check.

```sh
mend --revert --then 'cargo test parser' < mutant.patch
```

## All or nothing

- Every chunk is checked before the first write, so a patch that does not fit or is malformed writes nothing.
- New contents go to temporary files beside their targets, which are then renamed into place. When a rename fails, the files already replaced are put back.
- A termination signal that arrives while the files are written takes effect once they all are.
- With `--revert`, the files are also restored when `mend` receives `SIGHUP`, `SIGINT` or `SIGTERM` while the command runs. The command runs in a process group of its own, and everything in that group is stopped first.
- `--revert` leaves a file alone that changed after the patch was applied, and reports it.

## Output and exit status

Standard output carries one line per changed path, in the order the patch names them. Diagnostics go to standard error.

| Line | Meaning |
|---|---|
| `A <path>` | The path did not exist and now does. |
| `M <path>` | The path existed and its contents changed. |
| `D <path>` | The path existed and is gone. |

A move prints `D` for the source and `A` or `M` for the destination. A path whose contents end up unchanged prints nothing. With `--then`, the command's own output follows these lines.

| Status | Meaning |
|---|---|
| 0 | The patch applied, and the `--then` command succeeded. |
| 1 | The patch does not fit the files. A chunk matched no place or several, a named file is missing, or the patch makes one of the mistakes above. Nothing was written. |
| 2 | The patch or the command line is malformed. Nothing was written. |
| 3 | The `--then` command failed. The edits are kept unless `--revert` was given. |
| 4 | A file could not be read or written, and no file was left changed. Or `--revert` left a file alone because it changed after the patch was applied. |
| 128 + n | With `--revert`, signal `n` arrived while the command ran. The files were restored. |

## Using it from an agent

`mend mcp` serves a `patch` tool over the Model Context Protocol, so an agent passes the patch text as an argument instead of through a quoted heredoc. The tool takes `patch` and optionally `cwd`, `check`, `diff`, `fragment` and `all`, and returns what the command would print. A failed call wrote nothing and is marked as an error. It runs no commands, so `--then` stays with the command line.

For Claude Code:

```sh
claude mcp add --scope user mend -- mend mcp
```

For Codex, in `~/.codex/config.toml`:

```toml
[mcp_servers.mend]
command = "mend"
args = ["mcp"]
```

An agent will not pick up the command by itself, and it does not read `--help` unprompted. Put a section like this in `CLAUDE.md`, `AGENTS.md` or whatever file your agent reads.

````markdown
## Editing files

Use `mend` for hand-written changes to existing files, in place of your edit tool or a script that swaps one piece of text for another. It checks every chunk before it writes, and restores written files if a later write fails.

    mend <<'EOF' && make test
    *** Begin Patch
    *** Update File: src/config.py
    @@ def load():
    -    retries = 3
    +    retries = 5
    *** Add File: notes/retries.md
    +Retries were raised to 5.
    *** End Patch
    EOF

- Always quote the delimiter as `<<'EOF'`. Unquoted, the shell expands `$name`, backticks and `$(…)` inside the patch. For a path prefix, pass `-C <dir>`.
- Chain what depends on the edit with `&&` on the heredoc line. A command on the next line runs even when the patch failed.
- Each chunk matches exactly one place. Added lines go exactly where they stand among the chunk's lines.
- An `@@ <line>` anchor names a line of the file, whole or by its start, such as a signature up to `(`. A chunk that repeats the anchor line is placed around it, so it can change that line or add lines before it. Any other chunk goes below the anchor and above the next one. A chunk of only `+` lines goes right after the anchor, which has to name only one line.
- To insert before a line, give that line as context after the `+` lines. Before a function, that is the first line of its comment or attribute.
- Exit 1 means nothing was written. Stderr names the candidate places or the closest one, so correct the chunk from it.
- `--check --diff` previews a patch. `--revert --then '<command>'` makes a temporary edit such as a mutation check.
- `--fragment` lets one `-` and one `+` line replace part of a long line. `--all` changes every match. `*** Add File:` on an existing path replaces the file. `mend --help` has the rest.
````

## Differences from Codex `apply_patch`

- A patch that fails leaves no file changed. Codex writes each file as it goes.
- A chunk that matches several places is an error. Codex edits the first.
- A chunk may repeat its anchor line, to change it or add lines before it. Codex looks for a chunk's lines only below its anchor.
- A chunk of only `+` lines under an anchor goes right after the anchor line, which has to name only one line. Codex appends it to the end of the file.
- Chunks of one file need not be in file order, and several chunks may name the same anchor. Codex rejects both.
- Standard output has no `Success.` header, and failures have distinct exit statuses.
- A patch has to reach the command through a shell. The heredoc text `<<'EOF'` passed literally as an argument is not unwrapped, and the `*** Environment ID:` line is not accepted.

## Building

```sh
cargo build --release
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

`tests/fixtures/scenarios` holds one directory per behavior, each with the input files, the patch and the expected result.

## License

Apache-2.0. See `LICENSE` and `NOTICE` for the attribution to OpenAI Codex.
