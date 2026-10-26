use camino::{Utf8Path, Utf8PathBuf};
use std::io;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AddError {
    #[error("{operation} {path}: {source}")]
    Io {
        operation: IoOperation,
        path: Utf8PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("file {file} is outside repository root {repository_root}")]
    OutsideRepository {
        file: Utf8PathBuf,
        repository_root: Utf8PathBuf,
    },
    #[error("only regular files can be added: {0}")]
    NotRegularFile(Utf8PathBuf),
    #[error("filesystem returned a path that is not valid UTF-8")]
    NonUtf8Path,
    #[error("failed to serialize metadata for {path}: {source}")]
    MetadataSerialization {
        path: Utf8PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to parse metadata at {path}: {source}")]
    MetadataParsing {
        path: Utf8PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
}

#[derive(Debug, Error)]
pub enum IoOperation {
    #[error("resolve")]
    Canonicalize,
    #[error("inspect")]
    ReadMetadata,
    #[error("read directory")]
    ReadDirectory,
    #[error("open")]
    Open,
    #[error("read")]
    Read,
    #[error("create directory for")]
    CreateDirectory,
    #[error("check")]
    CheckPath,
    #[error("copy to")]
    Copy,
    #[error("create")]
    Create,
    #[error("write")]
    Write,
}

pub(crate) fn io_error(
    operation: IoOperation,
    path: impl AsRef<Utf8Path>,
    source: io::Error,
) -> AddError {
    AddError::Io {
        operation,
        path: path.as_ref().to_path_buf(),
        source,
    }
}
