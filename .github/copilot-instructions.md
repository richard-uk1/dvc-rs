# Repository instructions

## Project state

This repository is a Rust package named `dvc-rs`, exposing the `dvc_rs` library and the `dvc` executable. The library implements file addition; the CLI delegates to it.

## Build, test, and lint

Run these commands from the repository root:

- `cargo build` to compile the library.
- `cargo run -- add <file>` to add one file from the current directory.
- `cargo test` to run the test suite; use `cargo test <test_name>` to run matching tests.
- `cargo fmt --check` to check formatting.
- `cargo clippy --all-targets --all-features` to run Clippy.

## Architecture and conventions

`src/lib.rs` owns the add operation and uses Camino UTF-8 path types throughout its public API: it validates that the input is a regular file inside the supplied repository root, hashes its raw contents with DVC's default MD5 algorithm, copies it into `.dvc/cache/files/md5/<first-two-hash-chars>/<remaining-hash>`, and writes a YAML `<file>.dvc` metadata file. The original file stays in place. The `.dvc/.gitignore` file excludes the generated cache while leaving metadata trackable.

`src/main.rs` defines CLI arguments with Clap derive and delegates file handling to the library. Keep CLI argument handling thin so library behavior can be tested independently.

`src/error.rs` defines the public `AddError` and `IoOperation` types. `src/lib.rs` re-exports them and owns file-add behavior. Keep failure categories typed and retain underlying errors as sources rather than collapsing them into strings.
