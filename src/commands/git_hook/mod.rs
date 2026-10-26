use anyhow::{bail, Result};
use camino::Utf8Path;
use clap::{Args, Subcommand};
use std::ffi::OsString;

pub mod post_checkout;

#[derive(Debug, Args)]
pub struct GitHookArgs {
    #[command(subcommand)]
    command: GitHookCommand,
}

impl GitHookArgs {
    pub fn run(self, repo_root: impl AsRef<Utf8Path>) -> Result<()> {
        match self.command {
            GitHookCommand::PostCheckout(args) => {
                let is_branch_checkout = match args.branch_checkout.as_str() {
                    "0" => false,
                    "1" => true,
                    _ => bail!("post-checkout flag must be 0 or 1"),
                };
                post_checkout::post_checkout(repo_root, is_branch_checkout)
            }
            GitHookCommand::PreCommit(_) => not_implemented("pre-commit"),
            GitHookCommand::PrePush(_) => not_implemented("pre-push"),
            GitHookCommand::MergeDriver(_) => not_implemented("merge-driver"),
        }
    }
}

#[derive(Debug, Subcommand)]
enum GitHookCommand {
    PostCheckout(PostCheckoutArgs),
    PreCommit(HookArgs),
    PrePush(HookArgs),
    MergeDriver(MergeDriverArgs),
}

#[derive(Debug, Args)]
struct PostCheckoutArgs {
    #[arg(value_name = "PREVIOUS_HEAD")]
    _previous_head: OsString,
    #[arg(value_name = "NEW_HEAD")]
    _new_head: OsString,
    /// Git passes 1 for a branch checkout and 0 for a file checkout.
    #[arg(value_name = "BRANCH_CHECKOUT")]
    branch_checkout: String,
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
