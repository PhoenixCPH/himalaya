use anyhow::Result;
use clap::Subcommand;
use pimalaya_cli::printer::Printer;

use crate::{
    cli::BackendArg,
    config::{AccountConfig, Config},
    envelopes::list::EnvelopeListCommand,
};

/// Shared API to manage envelopes for the active account.
///
/// An envelope is a message headers subset. It is usually small, and
/// contains enough information to have an overall understanding of
/// what a message is about.
#[derive(Debug, Subcommand)]
pub enum EnvelopeCommand {
    #[command(visible_alias = "ls")]
    List(EnvelopeListCommand),
}

impl EnvelopeCommand {
    pub fn execute(
        self,
        printer: &mut impl Printer,
        config: Config,
        account_config: AccountConfig,
        backend: BackendArg,
    ) -> Result<()> {
        match self {
            Self::List(cmd) => cmd.execute(printer, config, account_config, backend),
        }
    }
}
