use anyhow::{bail, Result};
use clap::{Args, Subcommand};
use std::ffi::OsString;

#[derive(Debug, Args)]
pub struct GitHookArgs {
    #[command(subcommand)]
    command: GitHookCommand,
}

impl GitHookArgs {
    pub fn run(self) -> Result<()> {
        match self.command {
            GitHookCommand::PostCheckout(_) => not_implemented("post-checkout"),
            GitHookCommand::PreCommit(_) => not_implemented("pre-commit"),
            GitHookCommand::PrePush(_) => not_implemented("pre-push"),
            GitHookCommand::MergeDriver(_) => not_implemented("merge-driver"),
        }
    }
}

#[derive(Debug, Subcommand)]
enum GitHookCommand {
    PostCheckout(HookArgs),
    PreCommit(HookArgs),
    PrePush(HookArgs),
    MergeDriver(MergeDriverArgs),
}

#[derive(Debug, Args)]
struct HookArgs {
    #[arg(num_args = 0.., allow_hyphen_values = true, trailing_var_arg = true)]
    args: Vec<OsString>,
}

#[derive(Debug, Args)]
struct MergeDriverArgs {
    #[arg(long)]
    ancestor: OsString,
    #[arg(long)]
    our: OsString,
    #[arg(long)]
    their: OsString,
}

fn not_implemented(hook: &str) -> Result<()> {
    bail!("git-hook {hook} is not implemented")
}
