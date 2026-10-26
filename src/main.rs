use camino::Utf8PathBuf;
use clap::{Args, Parser, Subcommand};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(
    name = "dvc",
    version,
    about = "Track data files with DVC-style metadata"
)]
struct Cli {
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

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Command::Add(args) = cli.command;

    let repo_root = match env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("dvc: could not determine current directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let Ok(repo_root) = Utf8PathBuf::try_from(repo_root) else {
        eprintln!("dvc: repository path is not valid UTF-8");
        return ExitCode::FAILURE;
    };
    let Ok(path) = Utf8PathBuf::try_from(args.file) else {
        eprintln!("dvc: input path is not valid UTF-8");
        return ExitCode::FAILURE;
    };

    match dvc_rs::add_file(repo_root, path) {
        Ok(added) => {
            println!("Added {} (md5: {})", added.path, added.md5);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("dvc: {error}");
            ExitCode::FAILURE
        }
    }
}
