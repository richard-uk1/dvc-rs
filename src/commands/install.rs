use crate::fs::{FileSystem, StdFileSystem};
use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use std::process::{Command, Output};
use tracing::trace;

const HOOKS: [&str; 3] = ["post-checkout", "pre-commit", "pre-push"];

pub fn install(repo_root: impl AsRef<Utf8Path>) -> Result<()> {
    install_with_fs(&StdFileSystem, repo_root)
}

pub fn install_with_fs(
    filesystem: &impl FileSystem,
    repo_root: impl AsRef<Utf8Path>,
) -> Result<()> {
    let repo_root = repo_root.as_ref();
    let dvc_dir = repo_root.join(".dvc");
    if !filesystem
        .exists(&dvc_dir)
        .with_context(|| format!("failed to check DVC directory {}", dvc_dir))?
    {
        bail!("not a DVC project: .dvc directory does not exist");
    }
    if !filesystem
        .metadata(&dvc_dir)
        .with_context(|| format!("failed to inspect DVC directory {}", dvc_dir))?
        .is_dir
    {
        bail!("not a DVC project: .dvc directory does not exist");
    }

    let repo_root = filesystem
        .canonicalize(repo_root)
        .context("failed to resolve repository root")?;
    let hooks_path = configured_hooks_path(&repo_root)?;

    // Check every target before writing so an existing hook cannot leave a partial install.
    for hook in HOOKS {
        let hook_path = hooks_path.join(hook);
        if filesystem
            .exists(&hook_path)
            .with_context(|| format!("failed to check Git hook {}", hook_path))?
        {
            bail!(
                "Git hook already exists: {}. Remove it or combine it manually before installing",
                hook_path
            );
        }
    }

    filesystem
        .create_dir_all(&hooks_path)
        .with_context(|| format!("failed to create Git hooks directory {}", hooks_path))?;
    for hook in HOOKS {
        let hook_path = hooks_path.join(hook);
        let contents = format!("#!/bin/sh\nexec dvc-rs git-hook {hook} \"$@\"\n");
        filesystem
            .write(&hook_path, contents.as_bytes())
            .with_context(|| format!("failed to write Git hook {}", hook_path))?;
        filesystem
            .set_executable(&hook_path)
            .with_context(|| format!("failed to make Git hook executable: {}", hook_path))?;
    }

    git(
        &repo_root,
        ["config", "--local", "merge.dvc.name", "DVC merge driver"],
    )?;
    git(
        &repo_root,
        [
            "config",
            "--local",
            "merge.dvc.driver",
            "dvc-rs git-hook merge-driver --ancestor %O --our %A --their %B",
        ],
    )?;

    tracing::info!(hooks = ?HOOKS, hooks_path = %hooks_path, "installed Git hooks");
    Ok(())
}

fn configured_hooks_path(repo_root: &Utf8Path) -> Result<Utf8PathBuf> {
    let output = Command::new("git")
        .args(["config", "--path", "--local", "--get", "core.hooksPath"])
        .current_dir(repo_root)
        .output()
        .context("failed to query Git hooks path")?;
    if output.status.success() {
        let configured =
            String::from_utf8(output.stdout).context("Git hooks path is not valid UTF-8")?;
        let configured = Utf8PathBuf::from(configured.trim());
        return Ok(if configured.is_absolute() {
            configured
        } else {
            repo_root.join(configured)
        });
    }
    // Git exits with status 1 when the optional setting is absent.
    if !output.status.code().is_some_and(|code| code == 1) {
        return Err(git_error("git config --get core.hooksPath", output));
    }

    let hooks_path = git_output(repo_root, ["rev-parse", "--git-path", "hooks"])?;
    let hooks_path = Utf8PathBuf::from(hooks_path.trim());
    Ok(if hooks_path.is_absolute() {
        hooks_path
    } else {
        repo_root.join(hooks_path)
    })
}

fn git<const N: usize>(repo_root: &Utf8Path, args: [&str; N]) -> Result<()> {
    let mut command = Command::new("git");
    command.args(args);
    let description = format!("git {}", args.join(" "));
    trace!("running ${description}");
    let output = command
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to run {description}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(git_error(&description, output))
    }
}

fn git_output<const N: usize>(repo_root: &Utf8Path, args: [&str; N]) -> Result<String> {
    let mut command = Command::new("git");
    command.args(args);
    let description = format!("git {}", args.join(" "));
    trace!("running ${description}");
    let output = command
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("failed to run {description}"))?;
    if output.status.success() {
        String::from_utf8(output.stdout).context("Git returned output that is not valid UTF-8")
    } else {
        Err(git_error(&description, output))
    }
}

fn git_error(description: &str, output: Output) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::anyhow!("{description} failed: {}", stderr.trim())
}

#[cfg(test)]
mod tests {
    use super::install_with_fs;
    use crate::fs::FileSystem;
    use crate::utils::test_support::MockFileSystem;
    use camino::Utf8Path;
    use std::fs;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempDir(camino::Utf8PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = camino::Utf8PathBuf::try_from(std::env::temp_dir())
                .expect("temporary directory path is UTF-8")
                .join(format!("dvc-install-test-{}-{id}", std::process::id()));
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

    fn init_dvc_git_repo() -> TempDir {
        let repo = TempDir::new();
        fs::create_dir(repo.path().join(".dvc")).unwrap();
        let output = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        repo
    }

    #[test]
    fn installs_hooks_and_merge_driver() {
        let repo = init_dvc_git_repo();
        let filesystem = MockFileSystem::new();
        filesystem.add_dir(repo.path());
        filesystem.add_dir(repo.path().join(".dvc"));

        install_with_fs(&filesystem, repo.path()).unwrap();

        for hook in ["post-checkout", "pre-commit", "pre-push"] {
            let hook_path = repo.path().join(".git/hooks").join(hook);
            let contents = String::from_utf8(filesystem.read_file(&hook_path).unwrap()).unwrap();
            assert!(contents.contains(&format!("dvc-rs git-hook {hook}")));
            assert!(filesystem.is_executable(&hook_path));
        }
        assert_eq!(
            git_config_value(repo.path(), "merge.dvc.name"),
            "DVC merge driver"
        );
        assert_eq!(
            git_config_value(repo.path(), "merge.dvc.driver"),
            "dvc-rs git-hook merge-driver --ancestor %O --our %A --their %B"
        );
    }

    #[test]
    fn refuses_existing_hooks_without_partial_install() {
        let repo = init_dvc_git_repo();
        let filesystem = MockFileSystem::new();
        filesystem.add_dir(repo.path());
        filesystem.add_dir(repo.path().join(".dvc"));
        let pre_commit = repo.path().join(".git/hooks/pre-commit");
        filesystem.add_file(&pre_commit, b"existing hook");

        let error = install_with_fs(&filesystem, repo.path()).unwrap_err();

        assert!(error.to_string().contains("Git hook already exists"));
        assert_eq!(filesystem.read_file(&pre_commit).unwrap(), b"existing hook");
        assert!(!filesystem
            .exists(&repo.path().join(".git/hooks/post-checkout"))
            .unwrap());
        assert!(!filesystem
            .exists(&repo.path().join(".git/hooks/pre-push"))
            .unwrap());
    }

    #[test]
    fn honors_configured_hooks_path() {
        let repo = init_dvc_git_repo();
        let output = Command::new("git")
            .args(["config", "--local", "core.hooksPath", "custom-hooks"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        let filesystem = MockFileSystem::new();
        filesystem.add_dir(repo.path());
        filesystem.add_dir(repo.path().join(".dvc"));

        install_with_fs(&filesystem, repo.path()).unwrap();

        assert!(filesystem
            .read_file(repo.path().join("custom-hooks/post-checkout"))
            .is_some());
        assert!(filesystem
            .read_file(repo.path().join("custom-hooks/pre-commit"))
            .is_some());
        assert!(filesystem
            .read_file(repo.path().join("custom-hooks/pre-push"))
            .is_some());
    }

    #[test]
    fn requires_a_dvc_project() {
        let repo = TempDir::new();
        let output = Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(output.status.success());

        let filesystem = MockFileSystem::new();
        filesystem.add_dir(repo.path());
        let error = install_with_fs(&filesystem, repo.path()).unwrap_err();

        assert!(error.to_string().contains(".dvc directory does not exist"));
    }

    fn git_config_value(repo: &Utf8Path, key: &str) -> String {
        let output = Command::new("git")
            .args(["config", "--local", "--get", key])
            .current_dir(repo)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
}
