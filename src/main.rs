use anyhow::{Context, Result};
use camino::Utf8PathBuf;
use clap::{ArgAction, Args, Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;
use tracing::level_filters::LevelFilter;

#[derive(Debug, Parser)]
#[command(
    name = "dvc",
    version,
    about = "Track data files with DVC-style metadata"
)]
struct Cli {
    #[arg(short, long, action = ArgAction::Count, global = true)]
    quiet: u8,
    #[arg(short, long, action = ArgAction::Count, global = true)]
    verbose: u8,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Add(AddArgs),
}

#[derive(Debug, Args)]
struct AddArgs {
    file: PathBuf,
}

fn log_level(quiet: u8, verbose: u8) -> LevelFilter {
    match 3u8.saturating_add(verbose).saturating_sub(quiet) {
        0 => LevelFilter::OFF,
        1 => LevelFilter::ERROR,
        2 => LevelFilter::WARN,
        3 => LevelFilter::INFO,
        4 => LevelFilter::DEBUG,
        _ => LevelFilter::TRACE,
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let level = log_level(cli.quiet, cli.verbose);
    if let Err(error) = tracing_subscriber::fmt().with_max_level(level).try_init() {
        eprintln!("dvc: failed to initialize tracing: {error}");
        return ExitCode::FAILURE;
    }

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let Command::Add(args) = cli.command;
    tracing::trace!(file = %args.file.display(), "running add command");

    let repo_root = std::env::current_dir().context("could not determine current directory")?;
    let repo_root =
        Utf8PathBuf::try_from(repo_root).context("repository path is not valid UTF-8")?;
    let path = Utf8PathBuf::try_from(args.file).context("input path is not valid UTF-8")?;
    tracing::trace!(repository = %repo_root, file = %path, "dispatching add to library");

    let added = dvc_rs::add_file(repo_root, path)?;
    tracing::trace!(file = %added.path, md5 = %added.md5, "add command succeeded");
    println!("Added {} (md5: {})", added.path, added.md5);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::log_level;
    use tracing::level_filters::LevelFilter;

    #[test]
    fn verbosity_flags_adjust_the_log_level_from_info() {
        assert_eq!(log_level(0, 0), LevelFilter::INFO);
        assert_eq!(log_level(0, 1), LevelFilter::DEBUG);
        assert_eq!(log_level(0, 2), LevelFilter::TRACE);
        assert_eq!(log_level(1, 0), LevelFilter::WARN);
        assert_eq!(log_level(2, 0), LevelFilter::ERROR);
        assert_eq!(log_level(3, 0), LevelFilter::OFF);
        assert_eq!(log_level(1, 1), LevelFilter::INFO);
        assert_eq!(log_level(u8::MAX, 0), LevelFilter::OFF);
        assert_eq!(log_level(0, u8::MAX), LevelFilter::TRACE);
    }
}
