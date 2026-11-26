use std::io::Read;
use std::io::Write;
use std::process::Command;
use std::process::ExitCode;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use patchwork::ApplyPatchError;
use patchwork::Matching;
use patchwork::ParseError;
use patchwork::Plan;

mod mcp;

const USAGE: &str = "Usage: patchwork [-C <dir>] [--check] [--diff] [--fragment] [--all] [--then <command> [--revert]] [PATCH]";

const HELP: &str = "\
Applies a patch in the Codex apply_patch format, read from stdin or from PATCH.
Either every file the patch names is changed, or none is.

  patchwork <<'EOF' && make test
  *** Begin Patch
  *** Update File: src/config.py
  @@ def load():
  -    retries = 3
  +    retries = 5
  *** Add File: notes/retries.md
  +Retries were raised to 5.
  *** Delete File: notes/old.md
  *** End Patch
  EOF

  Keep the quotes in <<'EOF'. Without them the shell expands every $name in the
  patch and runs whatever stands in backticks before patchwork reads it.

Patch format:
  *** Update File: <path>  Changes a file through one or more chunks
  *** Move to: <path>      Right after Update File: also renames the file
  *** Add File: <path>     Creates a file from the `+` lines that follow, or replaces all of an existing one
  *** Delete File: <path>  Deletes a file
  @@                       Starts a chunk
  @@ <line>                Starts a chunk at <line>: around it when the chunk repeats
                           it, else below it. <line> may be given as just the start
                           of exactly one line
  *** End of File          After a chunk: it must match at the end of the file

  A chunk line starts with a space (context), `-` (removed) or `+` (added).
  A chunk of only `+` lines goes right after its @@ anchor line, or at the end of the
  file when it has no anchor or ends with *** End of File.
  To insert before a line, give that line as a context line after the `+` lines.
  Before a function with a comment above it, that line is the comment's first line.
  Added lines that end a chunk with a copy of the line just above them are rejected
  when that line opens a block, one the next line is indented deeper than.
  So are added lines that begin with a copy of the two lines just below them.
  So are added lines that go between two lines of a `//`, `# ` or `-- ` comment
  without being comment lines themselves.
  So are added lines right below a line that opens a block, one the next line is
  indented deeper than, when they are not indented deeper themselves.
  A path is absolute or relative to the current directory, inside a repository or not.
  For a long shared prefix use -C <dir>, which leaves the heredoc quoted.

Matching:
  Each chunk must match exactly one place. It is looked for below its @@ anchor (or
  the chunk before it) and above the next @@ anchor of a later chunk, then down to the
  end of the file, then in the whole file. The first of these that has a match
  decides, so an anchor before and after a chunk tells apart text that occurs more
  than once.
  A chunk with its own @@ anchor has to match between that anchor and the next one.
  Of several places there, the one that starts right below its anchor wins.
  A chunk whose own lines include its anchor line is placed with that line on the
  anchor, so it may change the anchor line or add lines before it.
  Whitespace at the ends and starts of lines is ignored only when no place matches
  exactly.
  When the `-` lines all leave out the same indentation that the file has, the `+`
  lines get it too.
  When a chunk matches no place or several, nothing is written and stderr names
  the candidate lines, or the closest place and its first differing line.

Running a command after the patch:
  `patchwork <<'EOF' && <command>` runs <command> only when the patch applied.
  For a long command, end the line with `&&` and write it after the closing EOF.
  A command that merely follows on the next line runs even when the patch failed.
  --then with --revert is for an edit that lasts one run, such as a mutation check.

Options:
  -C <dir>          Resolve the relative paths in the patch against <dir>
  --check           Report what the patch would change without writing anything
  --diff            Print a diff in git's format instead of one status line per file
  --fragment        Let a chunk of one `-` and one `+` line replace part of a line when no
                    whole line matches; the part must occur once where it is looked for
  --all             Let a chunk that matches several places change all of them; stderr
                    then names the lines
  --then <command>  Run <command> with `sh -c` once the patch is applied, keeping the edits if it fails
  --revert          With --then: restore every file when <command> has finished or is interrupted
  -h, --help        Print this help
  -V, --version     Print the version

As a tool:
  `patchwork mcp` serves a `patch` tool over the Model Context Protocol on stdin and
  stdout. It takes the patch text as an argument, along with `cwd`, `check`, `diff`,
  `fragment` and `all`, and returns what the command would print.

Output:
  One line per changed path on stdout: `A <path>`, `M <path>` or `D <path>`.

Exit status:
  0  the patch applied
  1  the patch does not fit the files; nothing was written
  2  the patch or the command line is malformed; nothing was written
  3  the --then command failed; the edits are kept unless --revert was given
  4  a file could not be read, written or restored; stderr names it
";

const DOES_NOT_APPLY: u8 = 1;
const MALFORMED: u8 = 2;
const THEN_FAILED: u8 = 3;
const IO_FAILURE: u8 = 4;

#[derive(Default)]
struct Options {
    check: bool,
    diff: bool,
    then: Option<String>,
    revert: bool,
    fragment: bool,
    all: bool,
    dir: Option<String>,
    patch: Option<String>,
}

fn main() -> ExitCode {
    let usage_error = |message: &str| {
        eprintln!("{message}\n{USAGE}\nRun `patchwork --help` for the patch format and examples.");
        ExitCode::from(MALFORMED)
    };
    let Ok(args) = std::env::args_os()
        .skip(1)
        .map(std::ffi::OsString::into_string)
        .collect::<Result<Vec<String>, _>>()
    else {
        return usage_error("Arguments have to be valid UTF-8");
    };
    if args == ["mcp"] {
        return mcp::serve();
    }
    let mut options = match parse_args(args.into_iter()) {
        Ok(Some(options)) => options,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => return usage_error(&message),
    };
    if let Some(dir) = &options.dir
        && !std::path::Path::new(dir).is_dir()
    {
        return usage_error(&format!("-C: {dir} is not a directory"));
    }
    if options.patch.is_none() {
        let mut patch = String::new();
        if let Err(error) = std::io::stdin().read_to_string(&mut patch) {
            eprintln!("Failed to read the patch from stdin: {error}");
            return ExitCode::from(IO_FAILURE);
        }
        options.patch = Some(patch);
    }
    let patch = options.patch.take().unwrap_or_default();
    if patch.trim().is_empty() {
        return usage_error("No patch was given");
    }
    match run(&patch, options) {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("{}", describe(&error));
            ExitCode::from(match error {
                ApplyPatchError::DoesNotApply(_) => DOES_NOT_APPLY,
                ApplyPatchError::ParseError(_) => MALFORMED,
                ApplyPatchError::IoError { .. } | ApplyPatchError::Restore(_) => IO_FAILURE,
            })
        }
    }
}

/// The message for `error`, as stderr and the tool give it.
fn describe(error: &ApplyPatchError) -> String {
    match error {
        ApplyPatchError::ParseError(ParseError::InvalidPatchError(message)) => {
            format!("Invalid patch: {message}")
        }
        ApplyPatchError::ParseError(ParseError::InvalidHunkError {
            message,
            line_number,
        }) => format!("Invalid patch hunk on line {line_number}: {message}"),
        other => other.to_string(),
    }
}

/// Returns `None` when an option such as `--help` has been answered in full.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => options.check = true,
            "--diff" => options.diff = true,
            "--revert" => options.revert = true,
            "--fragment" => options.fragment = true,
            "--all" => options.all = true,
            "-C" => options.dir = Some(args.next().ok_or("-C needs a directory")?),
            "--then" => {
                let command = args.next().ok_or("--then needs a command")?;
                options.then = Some(command);
            }
            "-h" | "--help" => {
                print!("{USAGE}\n\n{HELP}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("patchwork {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            option if option.starts_with('-') => return Err(format!("Unknown option: {option}")),
            _ if options.patch.is_some() => {
                return Err("Only one PATCH argument is accepted".to_string());
            }
            _ => options.patch = Some(arg),
        }
    }
    if options.revert && options.then.is_none() {
        return Err("--revert needs --then".to_string());
    }
    if options.check && options.then.is_some() {
        return Err("--check writes nothing, so it cannot be combined with --then".to_string());
    }
    Ok(Some(options))
}

fn run(patch: &str, options: Options) -> Result<ExitCode, ApplyPatchError> {
    let io_error = |context: &str| {
        let context = context.to_string();
        move |source| ApplyPatchError::IoError { context, source }
    };
    let cwd =
        std::env::current_dir().map_err(io_error("Failed to determine the current directory"))?;

    let hunks = patchwork::parse_patch(patch)?;
    let base = match &options.dir {
        Some(dir) => cwd.join(dir),
        None => cwd,
    };
    let matching = Matching {
        fragments: options.fragment,
        all: options.all,
    };
    let mut plan = Plan::compute(&hunks, &base, matching)?;
    // Registered before the first write, so no signal can end the process with files left changed.
    let signal = match options.revert {
        true => Some(watch_signals().map_err(io_error("Failed to watch for signals"))?),
        false => None,
    };
    if !options.check {
        plan.commit()?;
    }

    let mut stdout = std::io::stdout().lock();
    for change in plan.changes() {
        let written = if options.diff {
            write!(stdout, "{}", change.unified_diff())
        } else {
            writeln!(stdout, "{} {}", change.status(), change.label().display())
        };
        // A reader that closes the pipe early has seen what it wanted.
        if written.is_err() {
            break;
        }
    }
    let _ = stdout.flush();
    drop(stdout);
    for note in plan.notes() {
        eprintln!("{note}");
    }

    let Some(command) = options.then else {
        return Ok(ExitCode::SUCCESS);
    };
    let outcome = run_command(&command, signal.as_deref());
    let kept = if options.revert {
        plan.restore()?;
        "the files were restored"
    } else {
        "the edits were kept"
    };
    match outcome.map_err(io_error("Failed to run the --then command"))? {
        Outcome::Finished(status) if status.success() => Ok(ExitCode::SUCCESS),
        Outcome::Finished(status) => {
            let how = match status.code() {
                Some(code) => format!("exited with status {code}"),
                None => "was terminated by a signal".to_string(),
            };
            eprintln!("patchwork: the --then command {how}; {kept}");
            Ok(ExitCode::from(THEN_FAILED))
        }
        Outcome::Interrupted(signal) => {
            eprintln!("patchwork: interrupted by signal {signal}; {kept}");
            Ok(ExitCode::from(128 + signal as u8))
        }
    }
}

enum Outcome {
    Finished(ExitStatus),
    /// This process received the signal with the given number while the command ran.
    Interrupted(usize),
}

/// Returns a cell that holds the number of the last termination signal received, or zero.
#[cfg(unix)]
fn watch_signals() -> std::io::Result<Arc<AtomicUsize>> {
    use signal_hook::consts::SIGHUP;
    use signal_hook::consts::SIGINT;
    use signal_hook::consts::SIGTERM;

    let received = Arc::new(AtomicUsize::new(0));
    for signal in [SIGHUP, SIGINT, SIGTERM] {
        signal_hook::flag::register_usize(signal, Arc::clone(&received), signal as usize)?;
    }
    Ok(received)
}

#[cfg(not(unix))]
fn watch_signals() -> std::io::Result<Arc<AtomicUsize>> {
    Ok(Arc::new(AtomicUsize::new(0)))
}

/// Runs `command` in a shell, stopping it early when `signal` reports a termination signal.
fn run_command(command: &str, signal: Option<&AtomicUsize>) -> std::io::Result<Outcome> {
    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let mut child = Command::new(shell)
        .args([flag, command])
        .stdin(Stdio::null())
        .spawn()?;
    let Some(signal) = signal else {
        return child.wait().map(Outcome::Finished);
    };
    loop {
        let finished = child.try_wait()?;
        // Ctrl-C reaches the command and this process together, so the command may have ended
        // by the time the signal shows up here. That still counts as an interruption.
        let received = signal.load(Ordering::Relaxed);
        if received != 0 {
            if finished.is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Ok(Outcome::Interrupted(received));
        }
        if let Some(status) = finished {
            return Ok(Outcome::Finished(status));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
