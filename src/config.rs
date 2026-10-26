//! DVC INI config loading, named-section handling, and serialization.

use anyhow::{bail, Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use configparser::ini::Ini;
use std::collections::BTreeMap;
use std::fs;

pub(crate) fn path(repo_root: &Utf8Path, local: bool) -> Utf8PathBuf {
    repo_root
        .join(".dvc")
        .join(if local { "config.local" } else { "config" })
}

pub(crate) fn get(config_path: &Utf8Path, section: &str, key: &str) -> Result<Option<String>> {
    let config = load(config_path)?;
    Ok(config.get(section, key))
}

pub(crate) fn get_default(config_path: &Utf8Path) -> Result<Option<String>> {
    get(config_path, "core", "remote")
}

pub(crate) fn set_default(config_path: &Utf8Path, name: &str) -> Result<()> {
    set(config_path, "core", "remote", name)
}

pub(crate) fn unset_default(config_path: &Utf8Path) -> Result<()> {
    unset(config_path, "core", "remote")
}

pub(crate) fn remotes(config_path: &Utf8Path) -> Result<BTreeMap<String, String>> {
    let config = load(config_path)?;
    let remotes = config
        .get_map_ref()
        .iter()
        .filter_map(|(section, options)| {
            let name = section
                .strip_prefix("remote \"")
                .and_then(|section| section.strip_suffix('"'))?;
            let url = options.get("url")?.as_deref()?;
            Some((name.to_owned(), url.to_owned()))
        })
        .collect();
    Ok(remotes)
}

pub(crate) fn set(config_path: &Utf8Path, section: &str, key: &str, value: &str) -> Result<()> {
    let mut config = load(config_path)?;
    config.setstr(section, key, Some(value));
    save(config_path, &config)
}

pub(crate) fn set_remote(config_path: &Utf8Path, name: &str, key: &str, value: &str) -> Result<()> {
    set(config_path, &remote_section(name), key, value)
}

pub(crate) fn unset(config_path: &Utf8Path, section: &str, key: &str) -> Result<()> {
    let mut config = load(config_path)?;
    if config.remove_key(section, key).is_none() {
        bail_missing_key(config_path, section, key)?;
    }
    save(config_path, &config)
}

pub(crate) fn unset_remote(config_path: &Utf8Path, name: &str, key: &str) -> Result<()> {
    unset(config_path, &remote_section(name), key)
}

pub(crate) fn remove_remote(config_path: &Utf8Path, name: &str) -> Result<()> {
    let mut config = load(config_path)?;
    if config.remove_section(&remote_section(name)).is_none() {
        bail_missing_section(config_path, &remote_section(name))?;
    }
    save(config_path, &config)
}

pub(crate) fn rename_remote(config_path: &Utf8Path, name: &str, new_name: &str) -> Result<()> {
    let mut config = load(config_path)?;
    let old_section = remote_section(name);
    let new_section = remote_section(new_name);
    let values = config
        .remove_section(&old_section)
        .with_context(|| format!("remote '{name}' does not exist in {config_path}"))?;
    if config.get_map_ref().contains_key(&new_section) {
        bail!("remote '{new_name}' already exists in {config_path}");
    }
    for (key, value) in values {
        config.set(&new_section, &key, value);
    }
    save(config_path, &config)
}

pub(crate) fn remote_section(name: &str) -> String {
    format!("remote \"{}\"", name.to_ascii_lowercase())
}

fn load(config_path: &Utf8Path) -> Result<Ini> {
    let mut config = Ini::new();
    match fs::read_to_string(config_path) {
        Ok(contents) => {
            let contents = normalize_named_sections(&contents);
            config.read(contents).map_err(|error| {
                anyhow::anyhow!("failed to parse DVC config {config_path}: {error}")
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read DVC config {config_path}"));
        }
    }
    Ok(config)
}

fn normalize_named_sections(contents: &str) -> String {
    // ConfigObj quotes named sections with spaces; configparser expects bare INI headers.
    contents
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if let Some(section) = trimmed
                .strip_prefix("['")
                .and_then(|section| section.strip_suffix("']"))
                .filter(|section| {
                    ["remote \"", "machine \"", "db \""]
                        .iter()
                        .any(|prefix| section.starts_with(prefix))
                })
            {
                format!("[{section}]")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn save(config_path: &Utf8Path, config: &Ini) -> Result<()> {
    let parent = config_path
        .parent()
        .context("DVC config path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create DVC config directory {parent}"))?;
    fs::write(config_path, config.writes())
        .with_context(|| format!("failed to write DVC config {config_path}"))
}

fn bail_missing_key(config_path: &Utf8Path, section: &str, key: &str) -> Result<()> {
    anyhow::bail!("config option '{section}.{key}' does not exist in {config_path}")
}

fn bail_missing_section(config_path: &Utf8Path, section: &str) -> Result<()> {
    anyhow::bail!("config section '{section}' does not exist in {config_path}")
}

#[cfg(test)]
mod tests {
    use super::{get, remotes, remove_remote, rename_remote, set, unset};
    use camino::Utf8PathBuf;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    struct TempConfig(Utf8PathBuf);

    impl TempConfig {
        fn new() -> Self {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let path = Utf8PathBuf::try_from(std::env::temp_dir())
                .expect("temporary directory path is UTF-8")
                .join(format!(
                    "dvc-config-test-{}-{id}/config",
                    std::process::id()
                ));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            Self(path)
        }
    }

    impl Drop for TempConfig {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

    #[test]
    fn parses_dvc_named_remote_sections_case_insensitively() {
        let config_path = TempConfig::new();
        fs::write(
            &config_path.0,
            "[core]\nremote = Main\n\n['remote \"MyRemote\"']\nurl = /tmp/store\njobs = 4\n",
        )
        .unwrap();

        assert_eq!(
            get(&config_path.0, "core", "remote").unwrap().as_deref(),
            Some("Main")
        );
        assert_eq!(
            remotes(&config_path.0)
                .unwrap()
                .get("myremote")
                .map(String::as_str),
            Some("/tmp/store")
        );
        assert_eq!(
            get(&config_path.0, "remote \"myremote\"", "jobs")
                .unwrap()
                .as_deref(),
            Some("4")
        );
    }

    #[test]
    fn updates_preserve_other_sections_and_values() {
        let config_path = TempConfig::new();
        fs::write(
            &config_path.0,
            "[core]\nremote = store\n\n[remote \"store\"]\nurl = /tmp/store\njobs = 4\n\n[cache]\ntype = copy\n",
        )
        .unwrap();

        set(&config_path.0, "remote \"store\"", "jobs", "8").unwrap();
        rename_remote(&config_path.0, "store", "archive").unwrap();
        assert_eq!(
            get(&config_path.0, "remote \"archive\"", "jobs")
                .unwrap()
                .as_deref(),
            Some("8")
        );
        unset(&config_path.0, "remote \"archive\"", "jobs").unwrap();
        remove_remote(&config_path.0, "archive").unwrap();

        assert_eq!(
            get(&config_path.0, "cache", "type").unwrap().as_deref(),
            Some("copy")
        );
        assert!(remotes(&config_path.0).unwrap().is_empty());
    }
}
