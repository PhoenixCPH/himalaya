use anyhow::Result;
use clap::Subcommand;
use pimalaya_cli::printer::Printer;

use crate::{
    cli::BackendArg,
    config::{AccountConfig, Config},
    mailboxes::list::MailboxListCommand,
};

/// Shared API to manage mailboxes for the active account.
///
/// A mailbox is a message container.
#[derive(Debug, Subcommand)]
pub enum MailboxCommand {
    #[command(visible_alias = "ls")]
    List(MailboxListCommand),
}

impl MailboxCommand {
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
