use anyhow::Result;
use clap::Parser;
use io_email::flag::Flag;
use pimalaya_cli::printer::{Message, Printer};

use crate::{
    cli::BackendArg,
    client::EmailClient,
    config::{AccountConfig, Config},
    flags::arg::{FlagsArg, MailboxIdArg, MessageIdsArg},
};

/// Replace flag(s) of message(s) for the active account.
#[derive(Debug, Parser)]
pub struct FlagSetCommand {
    #[command(flatten)]
    pub mailbox_id: MailboxIdArg,
    #[command(flatten)]
    pub message_ids: MessageIdsArg,
    #[command(flatten)]
    pub flags: FlagsArg,
}

impl FlagSetCommand {
    pub fn execute(
        self,
        printer: &mut impl Printer,
        config: Config,
        account_config: AccountConfig,
        backend: BackendArg,
    ) -> Result<()> {
        let mut client = EmailClient::new(config, account_config, backend)?;

        let ids: Vec<&str> = self.message_ids.inner.iter().map(String::as_str).collect();
        let flags: Vec<Flag> = self.flags.inner.iter().map(Into::into).collect();

        client.set_flags(&self.mailbox_id.inner, &ids, &flags)?;

        let message = Message::new("Flag(s) successfully set");
        printer.out(message)
    }
}
