use anyhow::Result;
use clap::Subcommand;
use pimalaya_toolbox::terminal::printer::Printer;

use crate::{
    config::{AccountConfig, Config},
    flags::{add::FlagsAddCommand, delete::FlagsDeleteCommand, set::FlagsSetCommand},
};

/// Manage flags through whichever backend the active account has
/// configured.
#[derive(Debug, Subcommand)]
pub enum FlagsCommand {
    Add(FlagsAddCommand),
    Set(FlagsSetCommand),
    #[command(visible_alias = "remove", visible_alias = "rm")]
    Delete(FlagsDeleteCommand),
}

impl FlagsCommand {
    pub fn execute(
        self,
        printer: &mut impl Printer,
        config: Config,
        account_name: String,
        account_config: AccountConfig,
    ) -> Result<()> {
        match self {
            Self::Add(cmd) => cmd.execute(printer, config, account_name, account_config),
            Self::Set(cmd) => cmd.execute(printer, config, account_name, account_config),
            Self::Delete(cmd) => cmd.execute(printer, config, account_name, account_config),
        }
    }
}
