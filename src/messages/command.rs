use anyhow::Result;
use clap::Subcommand;
use pimalaya_toolbox::terminal::printer::Printer;

use crate::config::{AccountConfig, Config};
#[cfg(any(feature = "imap", feature = "maildir"))]
use crate::messages::get::MessagesGetCommand;
#[cfg(feature = "smtp")]
use crate::messages::send::MessagesSendCommand;

/// Manage messages through whichever backend the active account has
/// configured.
#[derive(Debug, Subcommand)]
pub enum MessagesCommand {
    #[cfg(any(feature = "imap", feature = "maildir"))]
    Get(MessagesGetCommand),
    #[cfg(feature = "smtp")]
    Send(MessagesSendCommand),
}

impl MessagesCommand {
    pub fn execute(
        self,
        printer: &mut impl Printer,
        config: Config,
        account_name: String,
        account_config: AccountConfig,
    ) -> Result<()> {
        match self {
            #[cfg(any(feature = "imap", feature = "maildir"))]
            Self::Get(cmd) => cmd.execute(printer, config, account_name, account_config),
            #[cfg(feature = "smtp")]
            Self::Send(cmd) => cmd.execute(printer, config, account_name, account_config),
        }
    }
}
