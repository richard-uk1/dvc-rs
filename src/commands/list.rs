use crate::error::io_error;
use crate::fs::{FileSystem, StdFileSystem};
use crate::utils::canonicalize_repo_root;
use crate::{AddError, IoOperation};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use tracing::trace;

#[derive(Debug, Deserialize)]
struct Metadata {
    outs: Vec<Output>,
}

#[derive(Debug, Deserialize)]
struct Output {
    path: String,
}

pub fn list_files(repo_root: impl AsRef<Utf8Path>) -> Result<Vec<Utf8PathBuf>, AddError> {
    list_files_with_fs(&StdFileSystem, repo_root)
}

pub fn list_files_with_fs(
    filesystem: &impl FileSystem,
    repo_root: impl AsRef<Utf8Path>,
) -> Result<Vec<Utf8PathBuf>, AddError> {
    tracing::info!(repository = %repo_root.as_ref(), "listing tracked files");
    let repo_root = canonicalize_repo_root(filesystem, repo_root)?;
    let mut metadata_files = Vec::new();
    collect_metadata_files(filesystem, &repo_root, &repo_root, &mut metadata_files)?;
    metadata_files.sort();

    let mut paths = Vec::new();
    for metadata_path in metadata_files {
        trace!(path = %metadata_path, "reading DVC metadata");
        let metadata_file_path = repo_root.join(&metadata_path);
        let metadata_file = filesystem
            .open_read(&metadata_file_path)
            .map_err(|error| io_error(IoOperation::Open, &metadata_path, error))?;
        let metadata: Metadata =
            serde_yaml::from_reader(metadata_file).map_err(|source| AddError::MetadataParsing {
                path: metadata_path.clone(),
                source,
            })?;
        paths.extend(metadata.outs.into_iter().map(|output| output.path));
    }

    paths.sort();
    paths.dedup();
    tracing::info!(count = paths.len(), "listed tracked files");
    Ok(paths.into_iter().map(Utf8PathBuf::from).collect())
}

pub(crate) fn collect_metadata_files(
    filesystem: &impl FileSystem,
    repo_root: &Utf8Path,
    directory: &Utf8Path,
    metadata_files: &mut Vec<Utf8PathBuf>,
) -> Result<(), AddError> {
    let entries = filesystem
        .read_dir(directory)
        .map_err(|error| io_error(IoOperation::ReadDirectory, directory, error))?;
    for entry in entries {
        let path = entry.path;
        if entry.is_dir {
            if path.file_name() != Some(".git") && path.file_name() != Some(".dvc") {
                collect_metadata_files(filesystem, repo_root, &path, metadata_files)?;
            }
        } else if entry.is_file && path.extension() == Some("dvc") {
            let relative_path = path
                .strip_prefix(repo_root)
                .unwrap_or(path.as_path())
                .to_path_buf();
            metadata_files.push(relative_path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::list_files_with_fs;
    use crate::utils::test_support::MockFileSystem;
    use crate::AddError;
    use camino::Utf8PathBuf;

    #[test]
    fn lists_unique_paths_from_nested_metadata() {
        let fs = MockFileSystem::new();
        let repo = Utf8PathBuf::from("/repo");
        fs.add_dir(repo.join("nested"));
        fs.add_dir(repo.join(".dvc/cache"));
        fs.add_dir(repo.join(".git"));
        fs.add_file(
            repo.join("a.dvc"),
            b"outs:\n- md5: aaa\n  size: 1\n  hash: md5\n  path: data/a\n",
        );
        fs.add_file(
            repo.join("nested/b.dvc"),
            b"outs:\n- md5: bbb\n  size: 2\n  hash: md5\n  path: data/b\n- md5: ccc\n  size: 3\n  hash: md5\n  path: data/a\n",
        );
        fs.add_file(repo.join(".dvc/ignored.dvc"), b"invalid: [");
        fs.add_file(repo.join(".git/ignored.dvc"), b"invalid: [");

        let files = list_files_with_fs(&fs, &repo).unwrap();

        assert_eq!(
            files,
            vec![Utf8PathBuf::from("data/a"), Utf8PathBuf::from("data/b")]
        );
    }

    #[test]
    fn reports_invalid_metadata() {
        let fs = MockFileSystem::new();
        let repo = Utf8PathBuf::from("/repo");
        fs.add_file(repo.join("broken.dvc"), b"outs: [");

        let error = list_files_with_fs(&fs, &repo).unwrap_err();

        assert!(matches!(error, AddError::MetadataParsing { .. }));
    }
}
