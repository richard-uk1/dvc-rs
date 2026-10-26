use crate::error::io_error;
use crate::fs::FileSystem;
use crate::{AddError, IoOperation};
use camino::{Utf8Path, Utf8PathBuf};

pub(crate) fn canonicalize_repo_root(
    filesystem: &impl FileSystem,
    repo_root: impl AsRef<Utf8Path>,
) -> Result<Utf8PathBuf, AddError> {
    let repo_root = repo_root.as_ref();
    let canonical = filesystem
        .canonicalize(repo_root)
        .map_err(|error| io_error(IoOperation::Canonicalize, repo_root, error))?;
    Ok(canonical)
}

#[cfg(test)]
pub(crate) mod test_support {
    pub(crate) use crate::fs::mock::MockFileSystem;
}
