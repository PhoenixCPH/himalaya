use anyhow::Result;
use clap::Subcommand;
use pimalaya_cli::printer::Printer;

use crate::{
    attachments::{download::AttachmentDownloadCommand, list::AttachmentListCommand},
    cli::BackendArg,
    config::{AccountConfig, Config},
};

/// Shared API to manage attachments for the active account.
///
/// An attachment is a binary part of a message.
#[derive(Debug, Subcommand)]
pub enum AttachmentCommand {
    #[command(visible_alias = "ls")]
    List(AttachmentListCommand),
    #[command(visible_alias = "dl")]
    Download(AttachmentDownloadCommand),
}

impl AttachmentCommand {
    pub fn execute(
        self,
        printer: &mut impl Printer,
        config: Config,
        account_config: AccountConfig,
        backend: BackendArg,
    ) -> Result<()> {
        match self {
            Self::List(cmd) => cmd.execute(printer, config, account_config, backend),
            Self::Download(cmd) => cmd.execute(printer, config, account_config, backend),
        }
    }
}
