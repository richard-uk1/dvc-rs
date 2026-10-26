# Repository instructions

## Project state

This repository is a Rust package named `dvc-rs`, exposing the `dvc_rs` library and the `dvc` executable. The library implements file addition; the CLI delegates to it.

## Build, test, and lint

Run these commands from the repository root:

- `cargo build` to compile the library.
- `cargo run -- add <file>` to add one file from the current directory.
- `cargo run -- list` to list tracked output paths from local `.dvc` metadata.
- Use `-v`/`--verbose` to increase logging (`-vv` enables trace); use `-q`/`--quiet` to decrease it (`-qqq` disables logs). Each repeated flag shifts the log level by one step from `info`; matching quiet and verbose counts cancel.
- `cargo test` to run the test suite; use `cargo test <test_name>` to run matching tests.
- `cargo fmt --check` to check formatting.
- `cargo clippy --all-targets --all-features` to run Clippy.

## Architecture and conventions

`src/commands/add.rs` implements file addition; `src/commands/list.rs` reads output paths from recursively discovered `.dvc` metadata, skipping `.git` and `.dvc` internals, and returns sorted unique paths. Both use the `FileSystem` abstraction in `src/fs.rs` and shared repository-root canonicalization from `src/utils.rs`. Production calls use `StdFileSystem`; command tests inject the in-memory mock. The crate root re-exports the add/list APIs. Add hashes raw contents with DVC's default MD5 algorithm, copies into `.dvc/cache/files/md5/<first-two-hash-chars>/<remaining-hash>`, and writes a YAML `<file>.dvc` metadata file while leaving the original in place. The `.dvc/.gitignore` file excludes the generated cache while leaving metadata trackable.

`src/main.rs` defines CLI arguments with Clap derive, initializes the tracing subscriber in `main` using repeatable quiet/verbose flags (defaulting to `info`), and delegates command execution to `run`. `main` logs command errors through `tracing` before returning a failure exit code. The library logs add start and completion at `info`, with operational details at `trace`; never log file contents.

`src/error.rs` defines the public `AddError` and `IoOperation` types. Keep failure categories typed and retain underlying errors as sources rather than collapsing them into strings.
