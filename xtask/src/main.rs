use std::error::Error;

use clap::Parser;

mod cli;
mod commands;
mod constants;
mod context;
mod profile;
mod targets;
mod util;

use cli::{Cli, Commands};
use commands::{build, clean, install, uninstall, validate};
use context::Context;
use profile::BuildProfile;

pub(crate) type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ctx = Context::new()?;

    match cli.command {
        Commands::Build(args) => build(&ctx, args)?,
        Commands::Install(args) => install(
            &ctx,
            BuildProfile::from_release(args.release),
            args.scope,
            &args.target,
        )?,
        Commands::Uninstall(args) => uninstall(&ctx, args.scope, &args.target, args.dry_run)?,
        Commands::Validate(args) => {
            validate(&ctx, BuildProfile::from_release(args.release), &args.target)?
        }
        Commands::Clean => clean(&ctx)?,
    }

    Ok(())
}
