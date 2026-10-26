use crate::config;
use anyhow::{bail, Result};
use camino::Utf8Path;
use clap::{Args, Subcommand};
use std::collections::BTreeMap;

#[derive(Debug, Args)]
pub struct RemoteArgs {
    #[arg(long, global = true)]
    local: bool,
    #[command(subcommand)]
    command: RemoteCommand,
}

impl RemoteArgs {
    pub fn run(self, repo_root: impl AsRef<Utf8Path>) -> Result<()> {
        let repo_root = repo_root.as_ref();
        match self.command {
            RemoteCommand::Add(args) => {
                let backend = RemoteBackend::from_url(&args.url)?;
                add_remote(repo_root, self.local, &args.name, &args.url, args.force)?;
                if args.default {
                    set_default(repo_root, self.local, Some(&args.name))?;
                }
                if let RemoteBackend::Stub(protocol) = backend {
                    tracing::warn!(
                        remote = %args.name,
                        protocol,
                        "remote configured; data transfer for this backend is not implemented"
                    );
                }
                println!("Added remote '{}'.", args.name);
            }
            RemoteCommand::Default(args) => {
                if args.unset {
                    set_default(repo_root, self.local, None)?;
                } else if let Some(name) = args.name {
                    ensure_remote_exists(repo_root, self.local, &name)?;
                    set_default(repo_root, self.local, Some(&name))?;
                } else if let Some(name) = get_default(repo_root, self.local)? {
                    println!("{name}");
                } else {
                    bail!("no default remote is configured");
                }
            }
            RemoteCommand::List => {
                let default = get_default(repo_root, self.local)?;
                for (name, url) in list_remotes(repo_root, self.local)? {
                    if default.as_deref() == Some(&name) {
                        println!("{name}\t{url}\t(default)");
                    } else {
                        println!("{name}\t{url}");
                    }
                }
            }
            RemoteCommand::Modify(args) => {
                modify_remote(
                    repo_root,
                    self.local,
                    &args.name,
                    &args.option,
                    args.value.as_deref(),
                    args.unset,
                )?;
            }
            RemoteCommand::Remove(args) => remove_remote(repo_root, self.local, &args.name)?,
            RemoteCommand::Rename(args) => {
                rename_remote(repo_root, self.local, &args.name, &args.new_name)?
            }
        }
        Ok(())
    }
}

#[derive(Debug, Subcommand)]
enum RemoteCommand {
    Add(AddArgs),
    Default(DefaultArgs),
    List,
    Modify(ModifyArgs),
    Remove(NameArgs),
    Rename(RenameArgs),
}

#[derive(Debug, Args)]
struct AddArgs {
    #[arg(short = 'd', long)]
    default: bool,
    #[arg(short = 'f', long)]
    force: bool,
    name: String,
    url: String,
}

#[derive(Debug, Args)]
struct DefaultArgs {
    #[arg(short = 'u', long, conflicts_with = "name")]
    unset: bool,
    name: Option<String>,
}

#[derive(Debug, Args)]
struct NameArgs {
    name: String,
}

#[derive(Debug, Args)]
struct ModifyArgs {
    #[arg(short = 'u', long, conflicts_with = "value")]
    unset: bool,
    name: String,
    option: String,
    value: Option<String>,
}

#[derive(Debug, Args)]
struct RenameArgs {
    name: String,
    new_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A configured remote type and whether its data-access implementation exists.
pub enum RemoteBackend {
    Local,
    Stub(&'static str),
}

impl RemoteBackend {
    /// Classifies a local path or a URL using a DVC-supported storage scheme.
    pub fn from_url(url: &str) -> Result<Self> {
        let scp_url = !url.contains("://") && is_scp_url(url);
        if url.starts_with("file://") || !url.contains("://") && !scp_url {
            return Ok(Self::Local);
        }

        let protocol = if scp_url {
            "ssh"
        } else {
            url.split_once("://")
                .map(|(scheme, _)| scheme)
                .unwrap_or("")
        };

        match protocol {
            "ssh" => Ok(Self::Stub("ssh")),
            "s3" => Ok(Self::Stub("s3")),
            "gs" => Ok(Self::Stub("gs")),
            "gdrive" => Ok(Self::Stub("gdrive")),
            "azure" => Ok(Self::Stub("azure")),
            "oss" => Ok(Self::Stub("oss")),
            "hdfs" | "webhdfs" => Ok(Self::Stub("hdfs")),
            "http" | "https" => Ok(Self::Stub("http")),
            "webdav" | "webdavs" => Ok(Self::Stub("webdav")),
            _ => bail!("unsupported remote URL scheme: {protocol}"),
        }
    }

    /// Returns an explicit error when the backend is only a configuration stub.
    pub fn require_implemented(self) -> Result<()> {
        match self {
            Self::Local => Ok(()),
            Self::Stub(protocol) => {
                bail!("remote data access for {protocol} locations is not implemented")
            }
        }
    }
}

fn is_scp_url(url: &str) -> bool {
    let Some((host, path)) = url.split_once(':') else {
        return false;
    };
    if host.len() == 1 && host.as_bytes()[0].is_ascii_alphabetic() && path.starts_with(['/', '\\'])
    {
        return false;
    }
    !host.is_empty() && !path.is_empty() && !host.contains(['/', '\\'])
}

fn add_remote(repo_root: &Utf8Path, local: bool, name: &str, url: &str, force: bool) -> Result<()> {
    validate_name(name)?;
    let config = config::path(repo_root, local);
    if !force && config::get(&config, &config::remote_section(name), "url")?.is_some() {
        bail!("remote '{name}' already exists; use --force to update its URL");
    }
    config::set_remote(&config, name, "url", url)
}

fn set_default(repo_root: &Utf8Path, local: bool, name: Option<&str>) -> Result<()> {
    let config = config::path(repo_root, local);
    match name {
        Some(name) => config::set_default(&config, name),
        None => config::unset_default(&config),
    }
}

fn get_default(repo_root: &Utf8Path, local: bool) -> Result<Option<String>> {
    if local {
        config::get_default(&config::path(repo_root, true))
    } else {
        Ok(config::get_default(&config::path(repo_root, true))?
            .or(config::get_default(&config::path(repo_root, false))?))
    }
}

fn list_remotes(repo_root: &Utf8Path, local: bool) -> Result<BTreeMap<String, String>> {
    let mut remotes = BTreeMap::new();
    if !local {
        remotes.extend(config::remotes(&config::path(repo_root, false))?);
    }
    remotes.extend(config::remotes(&config::path(repo_root, true))?);
    Ok(remotes)
}

fn ensure_remote_exists(repo_root: &Utf8Path, local: bool, name: &str) -> Result<()> {
    if list_remotes(repo_root, local)?.contains_key(name) {
        Ok(())
    } else {
        bail!("remote '{name}' does not exist")
    }
}

fn modify_remote(
    repo_root: &Utf8Path,
    local: bool,
    name: &str,
    option: &str,
    value: Option<&str>,
    unset: bool,
) -> Result<()> {
    validate_name(name)?;
    if option.is_empty()
        || !option
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        bail!("invalid remote option name: {option}");
    }
    if unset == value.is_some() {
        bail!("provide a value unless --unset is specified");
    }
    let config = config::path(repo_root, local);
    ensure_remote_in_config(&config, name)?;
    if unset {
        config::unset_remote(&config, name, option)
    } else {
        let value = value.expect("validated remote option value");
        if option == "url" {
            RemoteBackend::from_url(value)?;
        }
        config::set_remote(&config, name, option, value)
    }
}

fn remove_remote(repo_root: &Utf8Path, local: bool, name: &str) -> Result<()> {
    validate_name(name)?;
    let config = config::path(repo_root, local);
    ensure_remote_in_config(&config, name)?;
    config::remove_remote(&config, name)?;
    if config::get_default(&config)?.as_deref() == Some(name) {
        config::unset_default(&config)?;
    }
    Ok(())
}

fn rename_remote(repo_root: &Utf8Path, local: bool, name: &str, new_name: &str) -> Result<()> {
    validate_name(name)?;
    validate_name(new_name)?;
    let config = config::path(repo_root, local);
    ensure_remote_in_config(&config, name)?;
    if list_remotes(repo_root, local)?.contains_key(new_name) {
        bail!("remote '{new_name}' already exists");
    }

    config::rename_remote(&config, name, new_name)?;

    if config::get_default(&config)?.as_deref() == Some(name) {
        config::set_default(&config, new_name)?;
    }
    Ok(())
}

fn ensure_remote_in_config(config: &Utf8Path, name: &str) -> Result<()> {
    if config::get(config, &config::remote_section(name), "url")?.is_some() {
        Ok(())
    } else {
        bail!(
            "remote '{name}' does not exist in {}",
            config.file_name().unwrap_or("config")
        )
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        bail!("remote name must contain only ASCII letters, digits, '-' or '_': {name}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        add_remote, get_default, list_remotes, modify_remote, remove_remote, rename_remote,
        set_default, RemoteBackend,
    };
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
                .join(format!("dvc-remote-test-{}-{id}", std::process::id()));
            fs::create_dir_all(path.join(".dvc")).unwrap();
            Self(path)
        }

        fn path(&self) -> &Utf8Path {
            &self.0
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn manages_project_local_remotes() {
        let project = TempProject::new();
        let root = project.path();

        add_remote(root, false, "work", "../remote-store", false).unwrap();
        assert!(add_remote(root, false, "work", "/replacement", false).is_err());
        add_remote(root, false, "work", "/replacement", true).unwrap();
        add_remote(root, false, "work", "../remote-store", true).unwrap();
        modify_remote(root, false, "work", "jobs", Some("4"), false).unwrap();
        set_default(root, false, Some("work")).unwrap();
        assert_eq!(get_default(root, false).unwrap().as_deref(), Some("work"));
        assert_eq!(
            list_remotes(root, false)
                .unwrap()
                .get("work")
                .map(String::as_str),
            Some("../remote-store")
        );

        rename_remote(root, false, "work", "archive").unwrap();
        assert_eq!(
            get_default(root, false).unwrap().as_deref(),
            Some("archive")
        );

        modify_remote(root, false, "archive", "jobs", None, true).unwrap();
        remove_remote(root, false, "archive").unwrap();
        assert!(list_remotes(root, false).unwrap().is_empty());
        assert_eq!(get_default(root, false).unwrap(), None);
    }

    #[test]
    fn local_config_overrides_project_remote() {
        let project = TempProject::new();
        let root = project.path();
        add_remote(root, false, "shared", "/shared", false).unwrap();
        add_remote(root, true, "shared", "/private", false).unwrap();

        assert_eq!(
            list_remotes(root, false)
                .unwrap()
                .get("shared")
                .map(String::as_str),
            Some("/private")
        );
        assert_eq!(
            list_remotes(root, true)
                .unwrap()
                .get("shared")
                .map(String::as_str),
            Some("/private")
        );
    }

    #[test]
    fn renaming_a_remote_updates_only_its_config_scope_default() {
        let project = TempProject::new();
        let root = project.path();
        add_remote(root, false, "shared", "/shared", false).unwrap();
        set_default(root, false, Some("shared")).unwrap();
        add_remote(root, true, "private", "/private", false).unwrap();
        set_default(root, true, Some("private")).unwrap();

        rename_remote(root, false, "shared", "renamed").unwrap();

        assert_eq!(
            get_default(root, false).unwrap().as_deref(),
            Some("private")
        );
        assert_eq!(
            crate::config::get_default(&crate::config::path(root, false))
                .unwrap()
                .as_deref(),
            Some("renamed")
        );
    }

    #[test]
    fn recognizes_local_and_stub_backends() {
        assert_eq!(
            RemoteBackend::from_url("/tmp/store").unwrap(),
            RemoteBackend::Local
        );
        assert_eq!(
            RemoteBackend::from_url("file:///tmp/store").unwrap(),
            RemoteBackend::Local
        );
        assert_eq!(
            RemoteBackend::from_url("ssh://user@example.com/store").unwrap(),
            RemoteBackend::Stub("ssh")
        );
        assert_eq!(
            RemoteBackend::from_url("host:/store").unwrap(),
            RemoteBackend::Stub("ssh")
        );
        assert_eq!(
            RemoteBackend::from_url("s3://bucket/store").unwrap(),
            RemoteBackend::Stub("s3")
        );
        assert!(RemoteBackend::Stub("ssh")
            .require_implemented()
            .unwrap_err()
            .to_string()
            .contains("not implemented"));
        assert!(RemoteBackend::from_url("unknown://store").is_err());
    }
}
