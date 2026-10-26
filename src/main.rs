use camino::Utf8PathBuf;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let command = args.next();
    let path = args.next();

    if command.as_deref() != Some(std::ffi::OsStr::new("add")) || args.next().is_some() {
        eprintln!("Usage: dvc add <file>");
        return ExitCode::from(2);
    }
    let Some(path) = path else {
        eprintln!("Usage: dvc add <file>");
        return ExitCode::from(2);
    };

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
    let Ok(path) = Utf8PathBuf::try_from(PathBuf::from(path)) else {
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
