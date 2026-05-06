use anyhow::Result;
use clap::Subcommand;
use pimalaya_cli::printer::Printer;

use crate::{
    cli::BackendArg,
    config::{AccountConfig, Config},
    flags::{add::FlagAddCommand, remove::FlagRemoveCommand, set::FlagSetCommand},
};

/// Shared API to manage message flags for the active account.
///
/// A flag is acting like a tag, giving information about the state or
/// kind of a message.
#[derive(Debug, Subcommand)]
pub enum FlagCommand {
    Add(FlagAddCommand),
    Set(FlagSetCommand),
    #[command(visible_alias = "rm")]
    Remove(FlagRemoveCommand),
}

impl FlagCommand {
    pub fn execute(
        self,
        printer: &mut impl Printer,
        config: Config,
        account_config: AccountConfig,
        backend: BackendArg,
    ) -> Result<()> {
        match self {
            Self::Add(cmd) => cmd.execute(printer, config, account_config, backend),
            Self::Set(cmd) => cmd.execute(printer, config, account_config, backend),
            Self::Remove(cmd) => cmd.execute(printer, config, account_config, backend),
        }
    }
}
