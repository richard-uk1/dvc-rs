pub mod error;

pub use error::{AddError, IoOperation};

use crate::error::io_error;
use camino::{Utf8Path, Utf8PathBuf};
use md5::{Digest, Md5};
use serde::Serialize;
use std::fs::{self, File};
use std::io::Read;
use tracing::trace;

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
    pub path: Utf8PathBuf,
    pub metadata_path: Utf8PathBuf,
    pub cache_path: Utf8PathBuf,
    pub md5: String,
    pub size: u64,
}

pub fn add_file(
    repo_root: impl AsRef<Utf8Path>,
    input: impl AsRef<Utf8Path>,
) -> Result<AddedFile, AddError> {
    tracing::info!(
        repository = %repo_root.as_ref(),
        input = %input.as_ref(),
        "adding file"
    );
    let repo_root = fs::canonicalize(repo_root.as_ref())
        .map_err(|error| io_error(IoOperation::Canonicalize, repo_root.as_ref(), error))?;
    let repo_root = Utf8PathBuf::try_from(repo_root).map_err(|_| AddError::NonUtf8Path)?;
    let input = input.as_ref();
    let source_candidate = if input.is_absolute() {
        input.to_path_buf()
    } else {
        repo_root.join(input)
    };
    let source = fs::canonicalize(&source_candidate)
        .map_err(|error| io_error(IoOperation::Canonicalize, &source_candidate, error))?;
    let source = Utf8PathBuf::try_from(source).map_err(|_| AddError::NonUtf8Path)?;
    trace!(source = %source, "resolved input path");
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
    trace!(source = %source, size = file_type.len(), "validated input file");
    let metadata_path_text = relative_path.as_str().replace('\\', "/");

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
    trace!(source = %source, md5 = %hash, size, "computed file hash");

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
        trace!(source = %source, cache = %cache_path, "copying file into cache");
        fs::copy(&source, &cache_path)
            .map_err(|error| io_error(IoOperation::Copy, &cache_path, error))?;
    } else {
        trace!(
            source = %source,
            cache = %cache_path,
            cache_exists,
            "cache copy not required"
        );
    }

    let gitignore = dvc_dir.join(".gitignore");
    fs::create_dir_all(&dvc_dir)
        .map_err(|error| io_error(IoOperation::CreateDirectory, &dvc_dir, error))?;
    let gitignore_exists = gitignore
        .try_exists()
        .map_err(|error| io_error(IoOperation::CheckPath, &gitignore, error))?;
    if !gitignore_exists {
        trace!(path = %gitignore, "creating cache gitignore");
        fs::write(&gitignore, "/cache/\n")
            .map_err(|error| io_error(IoOperation::Write, &gitignore, error))?;
    }

    let metadata_path = Utf8PathBuf::from(format!("{source}.dvc"));
    let metadata = Metadata {
        outs: vec![Output {
            md5: hash.clone(),
            size,
            hash: "md5",
            path: metadata_path_text,
        }],
    };
    trace!(path = %metadata_path, "writing DVC metadata");
    let metadata_file = File::create(&metadata_path)
        .map_err(|error| io_error(IoOperation::Create, &metadata_path, error))?;
    serde_yaml::to_writer(metadata_file, &metadata).map_err(|source| {
        AddError::MetadataSerialization {
            path: metadata_path.clone(),
            source,
        }
    })?;

    let added = AddedFile {
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
    };
    tracing::info!(
        path = %added.path,
        cache = %added.cache_path,
        metadata = %added.metadata_path,
        md5 = %added.md5,
        size = added.size,
        "file added"
    );
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::{add_file, AddError, IoOperation};
    use camino::{Utf8Path, Utf8PathBuf};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempDir(Utf8PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = Utf8PathBuf::try_from(std::env::temp_dir())
                .expect("temporary directory path is UTF-8")
                .join(format!("dvc-rs-test-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Utf8Path {
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
        assert_eq!(added.path, Utf8PathBuf::from("data/input.txt"));
        assert_eq!(
            added.cache_path,
            Utf8PathBuf::from(".dvc/cache/files/md5/5e/b63bbbe01eeed093cb22bb8f5acdc3")
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
