use std::io::Read;
use std::io::Write;
use std::process::ExitCode;

use patchwork::ApplyPatchError;
use patchwork::ParseError;
use patchwork::Plan;

const USAGE: &str = "\
Usage: patchwork [--check] [--diff] [PATCH]

Applies a patch in the Codex apply_patch format, read from PATCH or from stdin.
Either every file the patch names is changed, or none is.

Options:
  --check        Report what the patch would change without writing anything
  --diff         Print a unified diff of the changes instead of one status line per file
  -h, --help     Print this help
  -V, --version  Print the version

Exit status:
  0  the patch applied
  1  the patch does not fit the files; nothing was written
  2  the patch or the command line is malformed; nothing was written
  4  a file could not be read or written; no file was left changed
";

const DOES_NOT_APPLY: u8 = 1;
const MALFORMED: u8 = 2;
const IO_FAILURE: u8 = 4;

#[derive(Default)]
struct Options {
    check: bool,
    diff: bool,
    patch: Option<String>,
}

fn main() -> ExitCode {
    let usage_error = |message: &str| {
        eprintln!("{message}\n\n{USAGE}");
        ExitCode::from(MALFORMED)
    };
    let mut options = match parse_args(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => return usage_error(&message),
    };
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
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            match &error {
                ApplyPatchError::ParseError(ParseError::InvalidPatchError(message)) => {
                    eprintln!("Invalid patch: {message}")
                }
                ApplyPatchError::ParseError(ParseError::InvalidHunkError {
                    message,
                    line_number,
                }) => eprintln!("Invalid patch hunk on line {line_number}: {message}"),
                other => eprintln!("{other}"),
            }
            ExitCode::from(match error {
                ApplyPatchError::DoesNotApply(_) => DOES_NOT_APPLY,
                ApplyPatchError::ParseError(_) => MALFORMED,
                ApplyPatchError::IoError { .. } => IO_FAILURE,
            })
        }
    }
}

/// Returns `None` when an option such as `--help` has been answered in full.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options::default();
    for arg in args {
        match arg.as_str() {
            "--check" => options.check = true,
            "--diff" => options.diff = true,
            "-h" | "--help" => {
                print!("{USAGE}");
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
    Ok(Some(options))
}

fn run(patch: &str, options: Options) -> Result<(), ApplyPatchError> {
    let cwd = std::env::current_dir().map_err(|source| ApplyPatchError::IoError {
        context: "Failed to determine the current directory".to_string(),
        source,
    })?;

    let hunks = patchwork::parse_patch(patch)?;
    let mut plan = Plan::compute(&hunks, &cwd)?;
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
    Ok(())
}
