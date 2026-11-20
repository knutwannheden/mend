mod file_update;
mod parser;
mod plan;
mod seek_sequence;
mod streaming_parser;
mod text_file;

use std::io;
use std::path::Path;

pub use parser::Hunk;
pub use parser::ParseError;
use parser::ParseError::*;
pub use parser::UpdateFileChunk;
pub use parser::parse_patch;
pub use plan::FileChange;
pub use plan::Plan;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApplyPatchError {
    #[error(transparent)]
    ParseError(#[from] ParseError),
    #[error("{context}: {source}")]
    IoError {
        context: String,
        #[source]
        source: io::Error,
    },
    /// The patch does not fit the files it names.
    #[error("{0}")]
    DoesNotApply(String),
}

impl ApplyPatchError {
    fn io(context: String, source: io::Error) -> Self {
        Self::IoError { context, source }
    }
}

/// Applies the patch and prints the result to stdout/stderr.
pub fn apply_patch(
    patch: &str,
    cwd: &Path,
    stdout: &mut impl io::Write,
    stderr: &mut impl io::Write,
) -> Result<(), ApplyPatchError> {
    let hunks = match parse_patch(patch) {
        Ok(hunks) => hunks,
        Err(e) => {
            let _ = match &e {
                InvalidPatchError(message) => writeln!(stderr, "Invalid patch: {message}"),
                InvalidHunkError {
                    message,
                    line_number,
                } => writeln!(
                    stderr,
                    "Invalid patch hunk on line {line_number}: {message}"
                ),
            };
            return Err(ApplyPatchError::ParseError(e));
        }
    };

    let applied = Plan::compute(&hunks, cwd).and_then(|mut plan| {
        plan.commit()?;
        Ok(plan)
    });
    match applied {
        Ok(plan) => {
            for change in plan.changes() {
                let _ = writeln!(stdout, "{} {}", change.status(), change.label().display());
            }
            Ok(())
        }
        Err(error) => {
            let _ = writeln!(stderr, "{error}");
            Err(error)
        }
    }
}
