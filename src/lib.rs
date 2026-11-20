mod file_update;
mod parser;
mod seek_sequence;
mod streaming_parser;
mod text_file;

use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use file_update::derive_new_contents_from_chunks;
pub use parser::Hunk;
pub use parser::ParseError;
use parser::ParseError::*;
pub use parser::UpdateFileChunk;
pub use parser::parse_patch;
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
    /// Error that occurs while computing replacements when applying patch chunks
    #[error("{0}")]
    ComputeReplacements(String),
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

    match apply_hunks_to_files(&hunks, cwd) {
        Ok(affected) => {
            let _ = print_summary(&affected, stdout);
            Ok(())
        }
        Err(error) => {
            let _ = writeln!(stderr, "{error}");
            Err(error)
        }
    }
}

/// Tracks file paths affected by applying a patch, preserving the path spelling
/// from the patch for user-facing summaries.
pub struct AffectedPaths {
    pub added: Vec<PathBuf>,
    pub modified: Vec<PathBuf>,
    pub deleted: Vec<PathBuf>,
}

/// Apply the hunks to the filesystem, returning which files were added, modified, or deleted.
/// Returns an error if the patch could not be applied.
fn apply_hunks_to_files(hunks: &[Hunk], cwd: &Path) -> Result<AffectedPaths, ApplyPatchError> {
    if hunks.is_empty() {
        return Err(ApplyPatchError::ComputeReplacements(
            "No files were modified.".to_string(),
        ));
    }

    let mut added: Vec<PathBuf> = Vec::new();
    let mut modified: Vec<PathBuf> = Vec::new();
    let mut deleted: Vec<PathBuf> = Vec::new();

    for hunk in hunks {
        let affected_path = hunk.path().to_path_buf();
        let path = hunk.resolve_path(cwd);
        match hunk {
            Hunk::AddFile { contents, .. } => {
                write_file_creating_parents(&path, contents)?;
                added.push(affected_path);
            }
            Hunk::DeleteFile { .. } => {
                remove_file(&path, "Failed to delete file")?;
                deleted.push(affected_path);
            }
            Hunk::UpdateFile {
                move_path, chunks, ..
            } => {
                let original_contents = fs::read_to_string(&path).map_err(|source| {
                    ApplyPatchError::io(
                        format!("Failed to read file to update {}", path.display()),
                        source,
                    )
                })?;
                let new_contents = derive_new_contents_from_chunks(
                    &path.display().to_string(),
                    &original_contents,
                    chunks,
                )?;
                if let Some(dest) = move_path {
                    write_file_creating_parents(&cwd.join(dest), &new_contents)?;
                    remove_file(&path, "Failed to remove original")?;
                } else {
                    fs::write(&path, new_contents).map_err(|source| {
                        ApplyPatchError::io(
                            format!("Failed to write file {}", path.display()),
                            source,
                        )
                    })?;
                }
                modified.push(affected_path);
            }
        }
    }
    Ok(AffectedPaths {
        added,
        modified,
        deleted,
    })
}

fn remove_file(path: &Path, context: &str) -> Result<(), ApplyPatchError> {
    let fail = || ApplyPatchError::ComputeReplacements(format!("{context} {}", path.display()));
    if fs::metadata(path).map_err(|_| fail())?.is_dir() {
        return Err(fail());
    }
    fs::remove_file(path).map_err(|_| fail())
}

fn write_file_creating_parents(path: &Path, contents: &str) -> Result<(), ApplyPatchError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            ApplyPatchError::io(
                format!("Failed to create parent directories for {}", path.display()),
                source,
            )
        })?;
    }
    fs::write(path, contents).map_err(|source| {
        ApplyPatchError::io(format!("Failed to write file {}", path.display()), source)
    })
}

/// Print the summary of changes in git-style format.
pub fn print_summary(affected: &AffectedPaths, out: &mut impl io::Write) -> io::Result<()> {
    writeln!(out, "Success. Updated the following files:")?;
    for path in &affected.added {
        writeln!(out, "A {}", path.display())?;
    }
    for path in &affected.modified {
        writeln!(out, "M {}", path.display())?;
    }
    for path in &affected.deleted {
        writeln!(out, "D {}", path.display())?;
    }
    Ok(())
}
