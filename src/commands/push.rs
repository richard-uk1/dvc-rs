use crate::commands::list::collect_metadata_files;
use crate::commands::remote::RemoteBackend;
use crate::config;
use crate::error::io_error;
use crate::fs::{FileSystem, StdFileSystem};
use crate::utils::canonicalize_repo_root;
use crate::{AddError, IoOperation};
use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use clap::Args;
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Debug, Args)]
pub struct PushArgs {
    #[arg(short = 'r', long)]
    remote: Option<String>,
}

impl PushArgs {
    pub fn run(self, repo_root: impl AsRef<Utf8Path>) -> Result<PushResult> {
        push(repo_root, self.remote.as_deref())
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PushResult {
    pub uploaded: usize,
    pub already_present: usize,
}

#[derive(Debug, Deserialize)]
struct Metadata {
    outs: Vec<Output>,
}

#[derive(Debug, Deserialize)]
struct Output {
    md5: String,
}

pub fn push(repo_root: impl AsRef<Utf8Path>, remote_name: Option<&str>) -> Result<PushResult> {
    push_with_fs(&StdFileSystem, repo_root, remote_name)
}

fn push_with_fs(
    filesystem: &impl FileSystem,
    repo_root: impl AsRef<Utf8Path>,
    remote_name: Option<&str>,
) -> Result<PushResult> {
    let repo_root = canonicalize_repo_root(filesystem, repo_root)?;
    let remote_name = resolve_remote_name(&repo_root, remote_name)?;
    let remote_url = resolve_remote_url(&repo_root, &remote_name)?;
    let backend = RemoteBackend::from_url(&remote_url)?;
    backend.require_implemented()?;

    let remote_root = local_remote_path(&repo_root, &remote_url)?;
    filesystem
        .create_dir_all(&remote_root)
        .map_err(|error| io_error(IoOperation::CreateDirectory, &remote_root, error))?;
    let canonical_remote_root = filesystem
        .canonicalize(&remote_root)
        .map_err(|error| io_error(IoOperation::Canonicalize, &remote_root, error))?;

    let mut metadata_files = Vec::new();
    collect_metadata_files(filesystem, &repo_root, &repo_root, &mut metadata_files)?;
    metadata_files.sort();

    let mut result = PushResult::default();
    let mut pushed_digests = BTreeSet::new();
    for metadata_path in metadata_files {
        tracing::trace!(path = %metadata_path, "reading DVC metadata for push");
        let metadata_file_path = repo_root.join(&metadata_path);
        let metadata_file = filesystem
            .open_read(&metadata_file_path)
            .map_err(|error| io_error(IoOperation::Open, &metadata_path, error))?;
        let metadata: Metadata =
            serde_yaml::from_reader(metadata_file).map_err(|source| AddError::MetadataParsing {
                path: metadata_path.clone(),
                source,
            })?;

        for output in metadata.outs {
            let digest = checked_md5(&output.md5)?;
            if !pushed_digests.insert(digest.clone()) {
                continue;
            }

            let cache_path = repo_root
                .join(".dvc/cache/files/md5")
                .join(&digest[..2])
                .join(&digest[2..]);
            let cache_metadata = filesystem
                .metadata(&cache_path)
                .with_context(|| format!("local cache object is missing: {cache_path}"))?;
            if !cache_metadata.is_file {
                bail!("local cache object is not a regular file: {cache_path}");
            }
            let canonical_cache_path = filesystem
                .canonicalize(&cache_path)
                .map_err(|error| io_error(IoOperation::Canonicalize, &cache_path, error))?;
            if !canonical_cache_path.starts_with(&repo_root) {
                bail!("local cache object resolves outside the repository: {cache_path}");
            }

            let remote_path = remote_root
                .join("files")
                .join("md5")
                .join(&digest[..2])
                .join(&digest[2..]);
            let remote_parent = remote_path
                .parent()
                .context("remote cache object path has no parent directory")?;
            filesystem
                .create_dir_all(remote_parent)
                .map_err(|error| io_error(IoOperation::CreateDirectory, remote_parent, error))?;
            let canonical_remote_parent = filesystem
                .canonicalize(remote_parent)
                .map_err(|error| io_error(IoOperation::Canonicalize, remote_parent, error))?;
            if !canonical_remote_parent.starts_with(&canonical_remote_root) {
                bail!("remote cache path resolves outside the remote: {remote_path}");
            }

            if filesystem
                .exists(&remote_path)
                .map_err(|error| io_error(IoOperation::CheckPath, &remote_path, error))?
            {
                let metadata = filesystem
                    .metadata(&remote_path)
                    .map_err(|error| io_error(IoOperation::ReadMetadata, &remote_path, error))?;
                if !metadata.is_file {
                    bail!("remote cache object is not a regular file: {remote_path}");
                }
                result.already_present += 1;
                tracing::trace!(md5 = %digest, "remote cache object already exists");
                continue;
            }

            filesystem
                .copy(&cache_path, &remote_path)
                .with_context(|| {
                    format!("failed to push cache object {digest} to {remote_path}")
                })?;
            result.uploaded += 1;
            tracing::trace!(md5 = %digest, destination = %remote_path, "pushed cache object");
        }
    }

    tracing::info!(
        remote = %remote_name,
        uploaded = result.uploaded,
        already_present = result.already_present,
        "pushed DVC cache objects"
    );
    Ok(result)
}

fn resolve_remote_name(repo_root: &Utf8Path, requested: Option<&str>) -> Result<String> {
    if let Some(name) = requested {
        return Ok(name.to_ascii_lowercase());
    }

    config::get_default(&config::path(repo_root, true))?
        .or(config::get_default(&config::path(repo_root, false))?)
        .context("no default remote is configured; specify one with --remote")
}

fn resolve_remote_url(repo_root: &Utf8Path, name: &str) -> Result<String> {
    let section = config::remote_section(name);
    config::get(&config::path(repo_root, true), &section, "url")?
        .or(config::get(
            &config::path(repo_root, false),
            &section,
            "url",
        )?)
        .with_context(|| format!("remote '{name}' does not exist"))
}

fn local_remote_path(repo_root: &Utf8Path, url: &str) -> Result<Utf8PathBuf> {
    let configured_path = if let Some(path) = url.strip_prefix("file://") {
        let path = if let Some(path) = path.strip_prefix("localhost/") {
            format!("/{path}")
        } else if path.starts_with('/') {
            path.to_owned()
        } else {
            bail!("file remote URL must use a local absolute path: {url}");
        };
        Utf8PathBuf::from(decode_url_path(&path)?)
    } else {
        Utf8PathBuf::from(url)
    };

    if configured_path.is_absolute() {
        Ok(configured_path)
    } else {
        Ok(repo_root.join(configured_path))
    }
}

fn decode_url_path(path: &str) -> Result<String> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = bytes
                .get(index + 1..index + 3)
                .context("invalid percent escape in file remote URL")?;
            let value = std::str::from_utf8(hex)
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .context("invalid percent escape in file remote URL")?;
            decoded.push(value);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).context("file remote URL path is not valid UTF-8")
}

fn checked_md5(md5: &str) -> Result<String> {
    if md5.len() != 32 || !md5.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid MD5 digest in DVC metadata: {md5}");
    }
    Ok(md5.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::{push, PushResult};
    use crate::config;
    use camino::{Utf8Path, Utf8PathBuf};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempProject(Utf8PathBuf);

    impl TempProject {
        fn new() -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = Utf8PathBuf::try_from(std::env::temp_dir())
                .expect("temporary directory path is UTF-8")
                .join(format!("dvc-push-test-{}-{id}", std::process::id()));
            fs::create_dir_all(path.join(".dvc")).unwrap();
            Self(path)
        }

        fn path(&self) -> &Utf8Path {
            &self.0
        }

        fn add_output(&self, metadata_name: &str, digest: &str, output_path: &str) {
            let metadata_path = self.path().join(metadata_name);
            fs::write(
                &metadata_path,
                format!("outs:\n- md5: {digest}\n  size: 11\n  hash: md5\n  path: {output_path}\n"),
            )
            .unwrap();
            let cache_path = self
                .path()
                .join(".dvc/cache/files/md5")
                .join(&digest[..2])
                .join(&digest[2..]);
            fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
            fs::write(cache_path, b"hello world").unwrap();
        }

        fn configure_remote(&self, name: &str, url: &str, default: bool) {
            let config_path = config::path(self.path(), false);
            config::set_remote(&config_path, name, "url", url).unwrap();
            if default {
                config::set_default(&config_path, name).unwrap();
            }
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn pushes_unique_cache_objects_to_the_default_remote() {
        let project = TempProject::new();
        let digest = "5eb63bbbe01eeed093cb22bb8f5acdc3";
        project.configure_remote("origin", "remote-store", true);
        project.add_output("data-a.dvc", digest, "data/a");
        project.add_output("data-b.dvc", digest, "data/b");

        let result = push(project.path(), None).unwrap();

        assert_eq!(
            result,
            PushResult {
                uploaded: 1,
                already_present: 0,
            }
        );
        assert_eq!(
            fs::read(
                project
                    .path()
                    .join("remote-store/files/md5/5e/b63bbbe01eeed093cb22bb8f5acdc3")
            )
            .unwrap(),
            b"hello world"
        );
    }

    #[test]
    fn skips_objects_already_in_the_remote() {
        let project = TempProject::new();
        let digest = "5eb63bbbe01eeed093cb22bb8f5acdc3";
        project.configure_remote("origin", "remote-store", true);
        project.add_output("data.dvc", digest, "data/a");
        let remote_object = project
            .path()
            .join("remote-store/files/md5/5e/b63bbbe01eeed093cb22bb8f5acdc3");
        fs::create_dir_all(remote_object.parent().unwrap()).unwrap();
        fs::write(&remote_object, b"hello world").unwrap();

        let result = push(project.path(), None).unwrap();

        assert_eq!(
            result,
            PushResult {
                uploaded: 0,
                already_present: 1,
            }
        );
    }

    #[test]
    fn explicit_remote_and_file_urls_are_supported() {
        let project = TempProject::new();
        let digest = "5eb63bbbe01eeed093cb22bb8f5acdc3";
        let remote_root = project.path().join("remote store");
        let encoded_remote_url = format!("file://{}", remote_root.as_str().replace(' ', "%20"));
        project.configure_remote("origin", "unused-store", true);
        project.configure_remote("secondary", &encoded_remote_url, false);
        project.add_output("data.dvc", digest, "data");

        let result = push(project.path(), Some("SECONDARY")).unwrap();

        assert_eq!(result.uploaded, 1);
        assert!(remote_root
            .join("files/md5/5e/b63bbbe01eeed093cb22bb8f5acdc3")
            .is_file());
        assert!(!project.path().join("unused-store").exists());
    }

    #[test]
    fn requires_a_default_or_explicit_local_remote() {
        let project = TempProject::new();

        assert!(push(project.path(), None)
            .unwrap_err()
            .to_string()
            .contains("no default remote"));

        project.configure_remote("origin", "ssh://example.com/data", true);
        assert!(push(project.path(), None)
            .unwrap_err()
            .to_string()
            .contains("not implemented"));
    }

    #[test]
    fn reports_missing_cache_objects() {
        let project = TempProject::new();
        project.configure_remote("origin", "remote-store", true);
        fs::write(
            project.path().join("data.dvc"),
            "outs:\n- md5: 5eb63bbbe01eeed093cb22bb8f5acdc3\n  path: data\n",
        )
        .unwrap();

        assert!(push(project.path(), None)
            .unwrap_err()
            .to_string()
            .contains("local cache object is missing"));
    }
}
