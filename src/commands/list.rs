use crate::error::io_error;
use crate::utils::canonicalize_repo_root;
use crate::{AddError, IoOperation};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use std::fs::{self, File};
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
    tracing::info!(repository = %repo_root.as_ref(), "listing tracked files");
    let repo_root = canonicalize_repo_root(repo_root)?;
    let mut metadata_files = Vec::new();
    collect_metadata_files(&repo_root, &repo_root, &mut metadata_files)?;
    metadata_files.sort();

    let mut paths = Vec::new();
    for metadata_path in metadata_files {
        trace!(path = %metadata_path, "reading DVC metadata");
        let metadata_file_path = repo_root.join(&metadata_path);
        let metadata_file = File::open(&metadata_file_path)
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

fn collect_metadata_files(
    repo_root: &Utf8Path,
    directory: &Utf8Path,
    metadata_files: &mut Vec<Utf8PathBuf>,
) -> Result<(), AddError> {
    let entries = fs::read_dir(directory)
        .map_err(|error| io_error(IoOperation::ReadDirectory, directory, error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| io_error(IoOperation::ReadDirectory, directory, error))?;
        let path = Utf8PathBuf::try_from(entry.path()).map_err(|_| AddError::NonUtf8Path)?;
        let file_type = entry
            .file_type()
            .map_err(|error| io_error(IoOperation::ReadMetadata, &path, error))?;
        if file_type.is_dir() {
            if entry.file_name() != ".git" && entry.file_name() != ".dvc" {
                collect_metadata_files(repo_root, &path, metadata_files)?;
            }
        } else if file_type.is_file() && path.extension() == Some("dvc") {
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
    use super::list_files;
    use crate::utils::test_support::TempDir;
    use crate::AddError;
    use camino::Utf8PathBuf;
    use std::fs;

    #[test]
    fn lists_unique_paths_from_nested_metadata() {
        let repo = TempDir::new();
        fs::create_dir_all(repo.path().join("nested")).unwrap();
        fs::create_dir_all(repo.path().join(".dvc/cache")).unwrap();
        fs::create_dir_all(repo.path().join(".git")).unwrap();
        fs::write(
            repo.path().join("a.dvc"),
            "outs:\n- md5: aaa\n  size: 1\n  hash: md5\n  path: data/a\n",
        )
        .unwrap();
        fs::write(
            repo.path().join("nested/b.dvc"),
            "outs:\n- md5: bbb\n  size: 2\n  hash: md5\n  path: data/b\n- md5: ccc\n  size: 3\n  hash: md5\n  path: data/a\n",
        )
        .unwrap();
        fs::write(repo.path().join(".dvc/ignored.dvc"), "invalid: [").unwrap();
        fs::write(repo.path().join(".git/ignored.dvc"), "invalid: [").unwrap();

        let files = list_files(repo.path()).unwrap();

        assert_eq!(
            files,
            vec![Utf8PathBuf::from("data/a"), Utf8PathBuf::from("data/b")]
        );
    }

    #[test]
    fn reports_invalid_metadata() {
        let repo = TempDir::new();
        fs::write(repo.path().join("broken.dvc"), "outs: [").unwrap();

        let error = list_files(repo.path()).unwrap_err();

        assert!(matches!(error, AddError::MetadataParsing { .. }));
    }
}
