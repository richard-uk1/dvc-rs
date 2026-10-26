# Repository instructions

## Project state

This repository is a Rust package named `dvc-rs`, exposing the `dvc_rs` library and the `dvc-rs` executable. The library implements file addition; the CLI delegates to it.

## Build, test, and lint

Run these commands from the repository root:

- `cargo build` to compile the library.
- `cargo run --bin dvc-rs -- add <file>` to add one file from the current directory.
- `cargo run --bin dvc-rs -- list` to list tracked output paths from local `.dvc` metadata.
- `cargo run --bin dvc-rs -- push [--remote <name>]` to push tracked cache objects to a configured local remote.
- `cargo run --bin dvc-rs -- install` to install DVC-style Git hooks in a DVC Git repository.
- `cargo run --bin dvc-rs -- remote --help` to manage DVC remote configuration.
- `cargo run --bin dvc-rs -- git-hook --help` to inspect the Git hook command and its hook handlers.
- `cargo run --bin dvc-rs -- git-hook post-checkout <PREVIOUS_HEAD> <NEW_HEAD> <BRANCH_CHECKOUT>` to restore cached outputs after a branch checkout.
- Use `-v`/`--verbose` to increase logging (`-vv` enables trace); use `-q`/`--quiet` to decrease it (`-qqq` disables logs). Each repeated flag shifts the log level by one step from `info`; matching quiet and verbose counts cancel.
- `cargo test` to run the test suite; use `cargo test <test_name>` to run matching tests.
- `cargo fmt --check` to check formatting.
- `cargo clippy --all-targets --all-features` to run Clippy.

## Architecture and conventions

`src/commands/add.rs` implements file addition; `src/commands/list.rs` reads output paths from recursively discovered `.dvc` metadata, skipping `.git` and `.dvc` internals, and returns sorted unique paths. Both use the `FileSystem` abstraction in `src/fs.rs` and shared repository-root canonicalization from `src/utils.rs`. Production calls use `StdFileSystem`; command tests inject the in-memory mock. The crate root re-exports the add/list APIs. Add hashes raw contents with DVC's default MD5 algorithm, copies into `.dvc/cache/files/md5/<first-two-hash-chars>/<remaining-hash>`, and writes a YAML `<file>.dvc` metadata file while leaving the original in place. The `.dvc/.gitignore` file excludes the generated cache while leaving metadata trackable.

`src/commands/install.rs` installs `post-checkout`, `pre-commit`, and `pre-push` hooks plus the DVC merge-driver Git config. It uses the `FileSystem` abstraction for file operations, requires an existing `.dvc` directory, honors `core.hooksPath`, and refuses to overwrite any target hook. The installed hooks invoke `dvc-rs git-hook` subcommands.

`src/commands/remote.rs` manages project and local DVC remote configuration (`add`, `default`, `list`, `modify`, `remove`, and `rename`). Local paths and `file://` locations are classified as local; documented cloud and self-hosted URL schemes are accepted as configuration stubs, but data transfer for those backends is not implemented.

`src/commands/push.rs` pushes unique MD5 cache objects referenced by project `.dvc` metadata to the selected or default local remote. It skips objects already present and reports missing local cache objects; remote backends other than local paths and `file://` are not implemented.

`src/config.rs` parses DVC INI config into named sections and writes the structured config back, similar to DVC's ConfigObj-based load/normalize/save flow. Remote sections use `[remote "name"]`; option and section lookups are case-insensitive.

`src/commands/git_hook/post_checkout.rs` restores outputs from the local MD5 cache after a branch `post-checkout`, skipping file-level checkouts and rejecting output paths outside the repository. The `pre-commit`, `pre-push`, and `merge-driver` handlers in `src/commands/git_hook/mod.rs` remain explicit not-implemented stubs.

`src/main.rs` defines CLI arguments with Clap derive, initializes the tracing subscriber in `main` using repeatable quiet/verbose flags (defaulting to `info`), and delegates command execution to `run`. `main` logs command errors through `tracing` before returning a failure exit code. The library logs add start and completion at `info`, with operational details at `trace`; never log file contents.

`src/error.rs` defines the public `AddError` and `IoOperation` types. Keep failure categories typed and retain underlying errors as sources rather than collapsing them into strings.
