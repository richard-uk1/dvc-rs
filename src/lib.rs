use md5::{Digest, Md5};
use serde::Serialize;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AddError {
    #[error("{operation} {path}: {source}")]
    Io {
        operation: IoOperation,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("file {file} is outside repository root {repository_root}")]
    OutsideRepository {
        file: PathBuf,
        repository_root: PathBuf,
    },
    #[error("only regular files can be added: {0}")]
    NotRegularFile(PathBuf),
    #[error("file path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    #[error("failed to serialize metadata for {path}: {source}")]
    MetadataSerialization {
        path: PathBuf,
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

fn io_error(operation: IoOperation, path: impl Into<PathBuf>, source: io::Error) -> AddError {
    AddError::Io {
        operation,
        path: path.into(),
        source,
    }
}

#[derive(Debug, Serialize)]
struct Metadata {
    outs: Vec<Output>,
}

#[derive(Debug, Serialize)]
struct Output {
    md5: String,
    size: u64,
    hash: &'static str,
    path: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AddedFile {
    pub path: PathBuf,
    pub metadata_path: PathBuf,
    pub cache_path: PathBuf,
    pub md5: String,
    pub size: u64,
}

pub fn add_file(
    repo_root: impl AsRef<Path>,
    input: impl AsRef<Path>,
) -> Result<AddedFile, AddError> {
    let repo_root = fs::canonicalize(repo_root.as_ref())
        .map_err(|error| io_error(IoOperation::Canonicalize, repo_root.as_ref(), error))?;
    let input = input.as_ref();
    let source_candidate = if input.is_absolute() {
        input.to_path_buf()
    } else {
        repo_root.join(input)
    };
    let source = fs::canonicalize(&source_candidate)
        .map_err(|error| io_error(IoOperation::Canonicalize, &source_candidate, error))?;
    let relative_path =
        source
            .strip_prefix(&repo_root)
            .map_err(|_| AddError::OutsideRepository {
                file: source.clone(),
                repository_root: repo_root.clone(),
            })?;
    let file_type = fs::metadata(&source)
        .map_err(|error| io_error(IoOperation::ReadMetadata, &source, error))?;
    if !file_type.is_file() {
        return Err(AddError::NotRegularFile(source));
    }
    let metadata_path_text = relative_path
        .to_str()
        .ok_or_else(|| AddError::NonUtf8Path(relative_path.to_path_buf()))?;
    let metadata_path_text = metadata_path_text.replace('\\', "/");

    let mut hasher = Md5::new();
    let mut file =
        File::open(&source).map_err(|error| io_error(IoOperation::Open, &source, error))?;
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| io_error(IoOperation::Read, &source, error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let hash = format!("{:x}", hasher.finalize());
    let size = file_type.len();

    let dvc_dir = repo_root.join(".dvc");
    let cache_path = dvc_dir
        .join("cache")
        .join("files")
        .join("md5")
        .join(&hash[..2])
        .join(&hash[2..]);
    let cache_parent = cache_path.parent().expect("cache path has a parent");
    fs::create_dir_all(cache_parent)
        .map_err(|error| io_error(IoOperation::CreateDirectory, cache_parent, error))?;
    let cache_exists = cache_path
        .try_exists()
        .map_err(|error| io_error(IoOperation::CheckPath, &cache_path, error))?;
    if source != cache_path && !cache_exists {
        fs::copy(&source, &cache_path)
            .map_err(|error| io_error(IoOperation::Copy, &cache_path, error))?;
    }

    let gitignore = dvc_dir.join(".gitignore");
    fs::create_dir_all(&dvc_dir)
        .map_err(|error| io_error(IoOperation::CreateDirectory, &dvc_dir, error))?;
    let gitignore_exists = gitignore
        .try_exists()
        .map_err(|error| io_error(IoOperation::CheckPath, &gitignore, error))?;
    if !gitignore_exists {
        fs::write(&gitignore, "/cache/\n")
            .map_err(|error| io_error(IoOperation::Write, &gitignore, error))?;
    }

    let mut metadata_path = OsString::from(source.as_os_str());
    metadata_path.push(".dvc");
    let metadata_path = PathBuf::from(metadata_path);
    let metadata = Metadata {
        outs: vec![Output {
            md5: hash.clone(),
            size,
            hash: "md5",
            path: metadata_path_text,
        }],
    };
    let metadata_file = File::create(&metadata_path)
        .map_err(|error| io_error(IoOperation::Create, &metadata_path, error))?;
    serde_yaml::to_writer(metadata_file, &metadata).map_err(|source| {
        AddError::MetadataSerialization {
            path: metadata_path.clone(),
            source,
        }
    })?;

    Ok(AddedFile {
        path: relative_path.to_path_buf(),
        metadata_path: metadata_path
            .strip_prefix(&repo_root)
            .unwrap_or(&metadata_path)
            .to_path_buf(),
        cache_path: cache_path
            .strip_prefix(&repo_root)
            .unwrap_or(&cache_path)
            .to_path_buf(),
        md5: hash,
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::{add_file, AddError, IoOperation};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("dvc-rs-test-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn adds_file_to_cache_and_writes_metadata() {
        let repo = TempDir::new();
        fs::create_dir_all(repo.path().join("data")).unwrap();
        fs::write(repo.path().join("data/input.txt"), "hello world").unwrap();

        let added = add_file(repo.path(), "data/input.txt").unwrap();

        assert_eq!(added.md5, "5eb63bbbe01eeed093cb22bb8f5acdc3");
        assert_eq!(added.size, 11);
        assert_eq!(added.path, PathBuf::from("data/input.txt"));
        assert_eq!(
            added.cache_path,
            PathBuf::from(".dvc/cache/files/md5/5e/b63bbbe01eeed093cb22bb8f5acdc3")
        );
        assert_eq!(
            fs::read(repo.path().join(&added.cache_path)).unwrap(),
            b"hello world"
        );
        assert!(repo.path().join("data/input.txt").is_file());

        let metadata = fs::read_to_string(repo.path().join(&added.metadata_path)).unwrap();
        assert!(metadata.contains("md5: 5eb63bbbe01eeed093cb22bb8f5acdc3"));
        assert!(metadata.contains("path: data/input.txt"));
        assert_eq!(
            fs::read_to_string(repo.path().join(".dvc/.gitignore")).unwrap(),
            "/cache/\n"
        );
    }

    #[test]
    fn rejects_paths_outside_repository() {
        let repo = TempDir::new();
        let outside = TempDir::new();
        fs::write(outside.path().join("input.txt"), "data").unwrap();

        let error = add_file(repo.path(), outside.path().join("input.txt")).unwrap_err();

        assert!(matches!(error, AddError::OutsideRepository { .. }));
    }

    #[test]
    fn rejects_directories() {
        let repo = TempDir::new();
        fs::create_dir(repo.path().join("data")).unwrap();

        let error = add_file(repo.path(), "data").unwrap_err();

        assert!(matches!(error, AddError::NotRegularFile(_)));
    }

    #[test]
    fn missing_file_preserves_io_error_context() {
        let repo = TempDir::new();

        let error = add_file(repo.path(), "missing.txt").unwrap_err();

        assert!(matches!(
            error,
            AddError::Io {
                operation: IoOperation::Canonicalize,
                ..
            }
        ));
        assert!(std::error::Error::source(&error).is_some());
    }
}
