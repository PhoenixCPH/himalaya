use anyhow::Result;
use clap::Subcommand;
use pimalaya_toolbox::terminal::printer::Printer;

use crate::{
    config::{AccountConfig, Config},
    envelopes::list::EnvelopesListCommand,
};

/// List envelopes through whichever backend the active account has
/// configured.
#[derive(Debug, Subcommand)]
pub enum EnvelopesCommand {
    #[command(visible_alias = "ls")]
    List(EnvelopesListCommand),
}

impl EnvelopesCommand {
    pub fn execute(
        self,
        printer: &mut impl Printer,
        config: Config,
        account_name: String,
        account_config: AccountConfig,
    ) -> Result<()> {
        match self {
            Self::List(cmd) => cmd.execute(printer, config, account_name, account_config),
        }
    }
}
