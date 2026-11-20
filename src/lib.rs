mod file_update;
mod parser;
mod plan;
mod seek_sequence;
mod streaming_parser;
mod text_file;

use std::io;

pub use parser::Hunk;
pub use parser::ParseError;
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
    /// Files changed by the patch could not be put back.
    #[error("{0}")]
    Restore(String),
}

impl ApplyPatchError {
    fn io(context: String, source: io::Error) -> Self {
        Self::IoError { context, source }
    }
}
