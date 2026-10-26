use std::env;
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
    match dvc_rs::add_file(repo_root, path) {
        Ok(added) => {
            println!("Added {} (md5: {})", added.path.display(), added.md5);
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("dvc: {error}");
            ExitCode::FAILURE
        }
    }
}
