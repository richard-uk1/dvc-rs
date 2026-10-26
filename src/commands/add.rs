use crate::error::io_error;
use crate::fs::{FileSystem, StdFileSystem};
use crate::utils::canonicalize_repo_root;
use crate::{AddError, IoOperation};
use camino::{Utf8Path, Utf8PathBuf};
use md5::{Digest, Md5};
use serde::Serialize;
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
    add_file_with_fs(&StdFileSystem, repo_root, input)
}

pub fn add_file_with_fs(
    filesystem: &impl FileSystem,
    repo_root: impl AsRef<Utf8Path>,
    input: impl AsRef<Utf8Path>,
) -> Result<AddedFile, AddError> {
    tracing::info!(
        repository = %repo_root.as_ref(),
        input = %input.as_ref(),
        "adding file"
    );
    let repo_root = canonicalize_repo_root(filesystem, repo_root)?;
    let input = input.as_ref();
    let source_candidate = if input.is_absolute() {
        input.to_path_buf()
    } else {
        repo_root.join(input)
    };
    let source = filesystem
        .canonicalize(&source_candidate)
        .map_err(|error| io_error(IoOperation::Canonicalize, &source_candidate, error))?;
    trace!(source = %source, "resolved input path");
    let relative_path =
        source
            .strip_prefix(&repo_root)
            .map_err(|_| AddError::OutsideRepository {
                file: source.clone(),
                repository_root: repo_root.clone(),
            })?;
    let file_type = filesystem
        .metadata(&source)
        .map_err(|error| io_error(IoOperation::ReadMetadata, &source, error))?;
    if !file_type.is_file {
        return Err(AddError::NotRegularFile(source));
    }
    trace!(source = %source, size = file_type.len, "validated input file");
    let metadata_path_text = relative_path.as_str().replace('\\', "/");

    let mut hasher = Md5::new();
    let mut file = filesystem
        .open_read(&source)
        .map_err(|error| io_error(IoOperation::Open, &source, error))?;
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
    let size = file_type.len;
    trace!(source = %source, md5 = %hash, size, "computed file hash");

    let dvc_dir = repo_root.join(".dvc");
    let cache_path = dvc_dir
        .join("cache")
        .join("files")
        .join("md5")
        .join(&hash[..2])
        .join(&hash[2..]);
    let cache_parent = cache_path.parent().expect("cache path has a parent");
    filesystem
        .create_dir_all(cache_parent)
        .map_err(|error| io_error(IoOperation::CreateDirectory, cache_parent, error))?;
    let cache_exists = filesystem
        .exists(&cache_path)
        .map_err(|error| io_error(IoOperation::CheckPath, &cache_path, error))?;
    if source != cache_path && !cache_exists {
        trace!(source = %source, cache = %cache_path, "copying file into cache");
        filesystem
            .copy(&source, &cache_path)
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
    filesystem
        .create_dir_all(&dvc_dir)
        .map_err(|error| io_error(IoOperation::CreateDirectory, &dvc_dir, error))?;
    let gitignore_exists = filesystem
        .exists(&gitignore)
        .map_err(|error| io_error(IoOperation::CheckPath, &gitignore, error))?;
    if !gitignore_exists {
        trace!(path = %gitignore, "creating cache gitignore");
        filesystem
            .write(&gitignore, b"/cache/\n")
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
    let metadata_yaml =
        serde_yaml::to_string(&metadata).map_err(|source| AddError::MetadataSerialization {
            path: metadata_path.clone(),
            source,
        })?;
    filesystem
        .write(&metadata_path, metadata_yaml.as_bytes())
        .map_err(|error| io_error(IoOperation::Write, &metadata_path, error))?;

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
    use super::add_file_with_fs;
    use crate::utils::test_support::MockFileSystem;
    use crate::{AddError, IoOperation};
    use camino::Utf8PathBuf;

    #[test]
    fn adds_file_to_cache_and_writes_metadata() {
        let fs = MockFileSystem::new();
        let repo = Utf8PathBuf::from("/repo");
        fs.add_dir(&repo);
        fs.add_file(repo.join("data/input.txt"), b"hello world");

        let added = add_file_with_fs(&fs, &repo, "data/input.txt").unwrap();

        assert_eq!(added.md5, "5eb63bbbe01eeed093cb22bb8f5acdc3");
        assert_eq!(added.size, 11);
        assert_eq!(added.path, Utf8PathBuf::from("data/input.txt"));
        assert_eq!(
            added.cache_path,
            Utf8PathBuf::from(".dvc/cache/files/md5/5e/b63bbbe01eeed093cb22bb8f5acdc3")
        );
        assert_eq!(
            fs.read_file(repo.join(&added.cache_path)).unwrap(),
            b"hello world".to_vec()
        );
        assert!(fs.read_file(repo.join("data/input.txt")).is_some());

        let metadata =
            String::from_utf8(fs.read_file(repo.join(&added.metadata_path)).unwrap()).unwrap();
        assert!(metadata.contains("md5: 5eb63bbbe01eeed093cb22bb8f5acdc3"));
        assert!(metadata.contains("path: data/input.txt"));
        assert_eq!(
            fs.read_file(repo.join(".dvc/.gitignore")).unwrap(),
            b"/cache/\n"
        );
    }

    #[test]
    fn rejects_paths_outside_repository() {
        let fs = MockFileSystem::new();
        let repo = Utf8PathBuf::from("/repo");
        let outside = Utf8PathBuf::from("/outside");
        fs.add_dir(&repo);
        fs.add_file(outside.join("input.txt"), b"data");

        let error = add_file_with_fs(&fs, &repo, outside.join("input.txt")).unwrap_err();

        assert!(matches!(error, AddError::OutsideRepository { .. }));
    }

    #[test]
    fn rejects_directories() {
        let fs = MockFileSystem::new();
        let repo = Utf8PathBuf::from("/repo");
        fs.add_dir(repo.join("data"));

        let error = add_file_with_fs(&fs, &repo, "data").unwrap_err();

        assert!(matches!(error, AddError::NotRegularFile(_)));
    }

    #[test]
    fn missing_file_preserves_io_error_context() {
        let fs = MockFileSystem::new();
        let repo = Utf8PathBuf::from("/repo");
        fs.add_dir(&repo);

        let error = add_file_with_fs(&fs, &repo, "missing.txt").unwrap_err();

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
