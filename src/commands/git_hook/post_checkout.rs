use crate::error::io_error;
use crate::fs::{FileSystem, StdFileSystem};
use crate::utils::canonicalize_repo_root;
use crate::{AddError, IoOperation};
use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::Deserialize;
use tracing::trace;

#[derive(Debug, Deserialize)]
struct Metadata {
    outs: Vec<Output>,
}

#[derive(Debug, Deserialize)]
struct Output {
    md5: String,
    path: String,
}

/// Restores outputs from the local cache after a branch checkout.
///
/// File-level checkouts are skipped; no remote data is fetched.
pub fn post_checkout(repo_root: impl AsRef<Utf8Path>, is_branch_checkout: bool) -> Result<()> {
    post_checkout_with_fs(&StdFileSystem, repo_root, is_branch_checkout)
}

fn post_checkout_with_fs(
    filesystem: &impl FileSystem,
    repo_root: impl AsRef<Utf8Path>,
    is_branch_checkout: bool,
) -> Result<()> {
    if !is_branch_checkout {
        trace!("skipping post-checkout for a file checkout");
        return Ok(());
    }

    let repo_root = canonicalize_repo_root(filesystem, repo_root)?;
    let mut metadata_files = Vec::new();
    collect_metadata_files(filesystem, &repo_root, &mut metadata_files)?;
    metadata_files.sort();

    let mut restored = 0;
    for metadata_path in metadata_files {
        trace!(path = %metadata_path, "reading DVC metadata for checkout");
        let metadata_file = filesystem
            .open_read(&metadata_path)
            .map_err(|error| io_error(IoOperation::Open, &metadata_path, error))?;
        let metadata: Metadata =
            serde_yaml::from_reader(metadata_file).map_err(|source| AddError::MetadataParsing {
                path: metadata_path.clone(),
                source,
            })?;

        for output in metadata.outs {
            let output_path = checked_output_path(&output.path)?;
            let digest = checked_md5(&output.md5)?;
            let cache_path = repo_root
                .join(".dvc/cache/files/md5")
                .join(&digest[..2])
                .join(&digest[2..]);

            let destination = repo_root.join(&output_path);
            let parent = destination
                .parent()
                .context("DVC output path has no parent directory")?;
            filesystem
                .create_dir_all(parent)
                .map_err(|error| io_error(IoOperation::CreateDirectory, parent, error))?;

            // Check that path is inside the repo.
            // Canonicalize to detect output paths that escape through a symlink.
            let resolved_parent = filesystem
                .canonicalize(parent)
                .map_err(|error| io_error(IoOperation::Canonicalize, parent, error))?;
            if !resolved_parent.starts_with(&repo_root) {
                bail!("DVC output path resolves outside the repository: {output_path}");
            }
            if filesystem
                .exists(&destination)
                .map_err(|error| io_error(IoOperation::CheckPath, &destination, error))?
            {
                let metadata = filesystem
                    .metadata(&destination)
                    .map_err(|error| io_error(IoOperation::ReadMetadata, &destination, error))?;
                if !metadata.is_file {
                    bail!("DVC output is not a regular file: {output_path}");
                }
                let resolved_destination = filesystem
                    .canonicalize(&destination)
                    .map_err(|error| io_error(IoOperation::Canonicalize, &destination, error))?;
                if !resolved_destination.starts_with(&repo_root) {
                    bail!("DVC output path resolves outside the repository: {output_path}");
                }
            }

            filesystem
                .copy(&cache_path, &destination)
                .with_context(|| {
                    format!("failed to restore DVC output {output_path} from cache {cache_path}")
                })?;
            restored += 1;
            trace!(path = %output_path, cache = %cache_path, "restored DVC output");
        }
    }

    tracing::info!(count = restored, "restored DVC outputs after checkout");
    Ok(())
}

fn collect_metadata_files(
    filesystem: &impl FileSystem,
    directory: &Utf8Path,
    metadata_files: &mut Vec<Utf8PathBuf>,
) -> Result<()> {
    let entries = filesystem
        .read_dir(directory)
        .map_err(|error| io_error(IoOperation::ReadDirectory, directory, error))?;
    for entry in entries {
        if entry.is_dir {
            if entry.path.file_name() != Some(".git") && entry.path.file_name() != Some(".dvc") {
                collect_metadata_files(filesystem, &entry.path, metadata_files)?;
            }
        } else if entry.is_file && entry.path.extension() == Some("dvc") {
            metadata_files.push(entry.path);
        }
    }
    Ok(())
}

fn checked_output_path(path: &str) -> Result<Utf8PathBuf> {
    let path = Utf8PathBuf::from(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                camino::Utf8Component::ParentDir
                    | camino::Utf8Component::RootDir
                    | camino::Utf8Component::Prefix(_)
            )
        })
    {
        bail!("invalid DVC output path: {path}");
    }
    Ok(path)
}

fn checked_md5(md5: &str) -> Result<String> {
    if md5.len() != 32 || !md5.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid MD5 digest in DVC metadata: {md5}");
    }
    Ok(md5.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::post_checkout_with_fs;
    use crate::utils::test_support::MockFileSystem;
    use camino::Utf8PathBuf;

    #[test]
    fn restores_outputs_from_cache_after_branch_checkout() {
        let filesystem = MockFileSystem::new();
        let repo = Utf8PathBuf::from("/repo");
        let digest = "5eb63bbbe01eeed093cb22bb8f5acdc3";
        let cache = repo.join(".dvc/cache/files/md5/5e/b63bbbe01eeed093cb22bb8f5acdc3");
        filesystem.add_file(
            repo.join("data.txt.dvc"),
            format!("outs:\n- md5: {digest}\n  size: 11\n  hash: md5\n  path: data/data.txt\n")
                .as_bytes(),
        );
        filesystem.add_file(cache, b"hello world");
        filesystem.add_file(repo.join("data/data.txt"), b"old contents");

        post_checkout_with_fs(&filesystem, &repo, true).unwrap();

        assert_eq!(
            filesystem.read_file(repo.join("data/data.txt")).unwrap(),
            b"hello world"
        );
    }

    #[test]
    fn skips_file_checkouts() {
        let filesystem = MockFileSystem::new();
        let repo = Utf8PathBuf::from("/repo");
        filesystem.add_file(repo.join("data.txt.dvc"), b"invalid metadata");

        post_checkout_with_fs(&filesystem, &repo, false).unwrap();

        assert_eq!(
            filesystem.read_file(repo.join("data.txt.dvc")).unwrap(),
            b"invalid metadata"
        );
    }

    #[test]
    fn rejects_output_paths_outside_the_repository() {
        let filesystem = MockFileSystem::new();
        let repo = Utf8PathBuf::from("/repo");
        filesystem.add_file(
            repo.join("data.txt.dvc"),
            b"outs:\n- md5: 5eb63bbbe01eeed093cb22bb8f5acdc3\n  path: ../outside.txt\n",
        );

        let error = post_checkout_with_fs(&filesystem, &repo, true).unwrap_err();

        assert!(error.to_string().contains("invalid DVC output path"));
    }
}
