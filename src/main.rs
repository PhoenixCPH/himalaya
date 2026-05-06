mod account;
#[cfg(any(feature = "imap", feature = "jmap", feature = "maildir"))]
mod attachments;
mod cli;
mod client;
mod config;
mod envelopes;
mod flags;
#[cfg(feature = "imap")]
mod imap;
#[cfg(feature = "jmap")]
mod jmap;
mod mailboxes;
#[cfg(feature = "maildir")]
mod maildir;
mod messages;
#[cfg(feature = "smtp")]
mod smtp;
mod wizard;

use clap::Parser;
use pimalaya_cli::{error::ErrorReport, log::Logger, printer::StdoutPrinter};

use crate::cli::HimalayaCli;

fn main() {
    let cli = HimalayaCli::parse();

    Logger::init(&cli.log);

    let mut printer = StdoutPrinter::new(&cli.json);
    let config = cli.config_paths.as_ref();
    let account = cli.account.name.as_deref();
    let backend = cli.backend;
    let result = cli.command.execute(&mut printer, config, account, backend);

    ErrorReport::eval(&mut printer, result)
}
