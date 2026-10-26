use crate::error::io_error;
use crate::{AddError, IoOperation};
use camino::{Utf8Path, Utf8PathBuf};
use std::fs;

pub(crate) fn canonicalize_repo_root(
    repo_root: impl AsRef<Utf8Path>,
) -> Result<Utf8PathBuf, AddError> {
    let repo_root = repo_root.as_ref();
    let canonical = fs::canonicalize(repo_root)
        .map_err(|error| io_error(IoOperation::Canonicalize, repo_root, error))?;
    Utf8PathBuf::try_from(canonical).map_err(|_| AddError::NonUtf8Path)
}

#[cfg(test)]
pub(crate) mod test_support {
    use camino::{Utf8Path, Utf8PathBuf};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    pub(crate) struct TempDir(Utf8PathBuf);

    impl TempDir {
        pub(crate) fn new() -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = Utf8PathBuf::try_from(std::env::temp_dir())
                .expect("temporary directory path is UTF-8")
                .join(format!("dvc-rs-test-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        pub(crate) fn path(&self) -> &Utf8Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
