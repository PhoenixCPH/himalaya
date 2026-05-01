use std::fmt;
#[cfg(feature = "imap")]
use std::io::{Read, Write};
#[cfg(feature = "maildir")]
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Result};
use clap::Parser;
use io_email::coroutines::message_get::{MessageGet, MessageGetArg, MessageGetResult};
use mail_parser::Message;
use pimalaya_toolbox::terminal::printer::Printer;
use serde::Serialize;

use crate::{
    account::Account,
    config::{AccountConfig, Config},
};

#[cfg(feature = "imap")]
const READ_BUFFER_SIZE: usize = 16 * 1024;

/// Get a parsed message from the active account.
///
/// JMAP message retrieval is not yet wrapped here because the protocol
/// exposes parsed `Email` objects directly; obtaining the raw RFC822
/// form requires a separate `Blob/download` round trip not yet covered
/// by io-jmap. Use `himalaya jmap email get` instead.
#[derive(Debug, Parser)]
pub struct MessagesGetCommand {
    /// Identifier of the message (IMAP UID or Maildir filename id).
    #[arg(value_name = "ID")]
    pub id: String,

    /// Mailbox name or path (IMAP mailbox / Maildir path).
    #[arg(long = "mailbox", short = 'm', value_name = "NAME", default_value = "Inbox")]
    pub mailbox: String,

    /// Treat the IMAP id as a sequence number instead of a UID.
    #[arg(long, visible_alias = "seq")]
    pub sequence: bool,
}

impl MessagesGetCommand {
    pub fn execute(
        self,
        printer: &mut impl Printer,
        config: Config,
        account_name: String,
        mut account_config: AccountConfig,
    ) -> Result<()> {
        let _ = account_name;

        #[cfg(feature = "imap")]
        if let Some(imap_config) = account_config.imap.take() {
            let account = Account::new(config, account_config, imap_config)?;
            let message = drive_imap(&account, &self.mailbox, &self.id, self.sequence)?;
            return printer.out(MessageView(message));
        }

        #[cfg(feature = "maildir")]
        if let Some(maildir_config) = account_config.maildir.take() {
            let account = Account::new(config, account_config, maildir_config)?;
            let message = drive_maildir(&account, &self.mailbox, &self.id)?;
            return printer.out(MessageView(message));
        }

        #[cfg(feature = "jmap")]
        if account_config.jmap.is_some() {
            bail!("JMAP message get is not exposed via the cross-protocol command; use `himalaya jmap email get` instead")
        }

        bail!("no compatible backend (imap, maildir) configured for this account")
    }
}

#[cfg(feature = "imap")]
fn drive_imap(
    account: &Account<crate::config::ImapConfig>,
    mailbox: &str,
    id: &str,
    sequence: bool,
) -> Result<Message<'static>> {
    use std::num::NonZeroU32;

    use io_imap::{
        rfc3501::{fetch::ImapMessageFetchFirst, select::*},
        types::{
            fetch::{MacroOrMessageDataItemNames, MessageDataItemName},
            mailbox::Mailbox,
        },
    };
    use pimalaya_toolbox::stream::imap::ImapSession;

    let mut imap = ImapSession::new(
        account.backend.url.clone(),
        account.backend.tls.clone().try_into()?,
        account.backend.starttls,
        account.backend.sasl.clone().try_into()?,
    )?;

    let mailbox: Mailbox<'static> = mailbox.to_owned().try_into()?;
    let mut select = ImapMailboxSelect::new(imap.context, mailbox);
    let mut buf = [0u8; READ_BUFFER_SIZE];
    let mut arg: Option<&[u8]> = None;

    imap.context = loop {
        match select.resume(arg.take()) {
            ImapMailboxSelectResult::Ok { context, .. } => break context,
            ImapMailboxSelectResult::WantsRead => {
                let n = imap.stream.read(&mut buf)?;
                arg = Some(&buf[..n]);
            }
            ImapMailboxSelectResult::WantsWrite(bytes) => {
                imap.stream.write_all(&bytes)?;
                arg = None;
            }
            ImapMailboxSelectResult::Err { err, .. } => bail!(err),
        }
    };

    let id: NonZeroU32 = id.parse()?;
    let item_names =
        MacroOrMessageDataItemNames::MessageDataItemNames(vec![MessageDataItemName::BodyExt {
            section: None,
            partial: None,
            peek: true,
        }]);

    let inner = ImapMessageFetchFirst::new(imap.context, id, item_names, !sequence);
    let mut coroutine = MessageGet::new(inner);
    let mut arg: Option<MessageGetArg<'_>> = None;

    loop {
        match coroutine.resume(arg.take()) {
            MessageGetResult::Ok(message) => return Ok(message),
            MessageGetResult::WantsBytesRead => {
                let n = imap.stream.read(&mut buf)?;
                arg = Some(MessageGetArg::Bytes(&buf[..n]));
            }
            MessageGetResult::WantsBytesWrite(bytes) => {
                imap.stream.write_all(&bytes)?;
                arg = None;
            }
            MessageGetResult::Err(err) => bail!(err),
            #[allow(unreachable_patterns)]
            other => bail!("unexpected I/O request from IMAP: {other:?}"),
        }
    }
}

#[cfg(feature = "maildir")]
fn drive_maildir(
    account: &Account<crate::config::MaildirConfig>,
    mailbox: &str,
    id: &str,
) -> Result<Message<'static>> {
    use io_maildir::{coroutines::message_get::MaildirMessageGet, maildir::Maildir};

    let path = account.backend.root.join(mailbox);
    let maildir = Maildir::try_from(path)?;

    let inner = MaildirMessageGet::new(maildir, id);
    let mut coroutine = MessageGet::new(inner);
    let mut arg: Option<MessageGetArg<'_>> = None;

    loop {
        match coroutine.resume(arg.take()) {
            MessageGetResult::Ok(message) => return Ok(message),
            MessageGetResult::WantsDirRead(paths) => {
                arg = Some(MessageGetArg::DirRead(read_dirs(&paths)?));
            }
            MessageGetResult::WantsFileRead(paths) => {
                arg = Some(MessageGetArg::FileRead(read_files(&paths)?));
            }
            MessageGetResult::Err(err) => bail!(err),
            #[allow(unreachable_patterns)]
            other => bail!("unexpected I/O request from Maildir: {other:?}"),
        }
    }
}

#[cfg(feature = "maildir")]
fn read_dirs(paths: &BTreeSet<String>) -> Result<BTreeMap<String, BTreeSet<String>>> {
    use std::fs;

    let mut out = BTreeMap::new();

    for path in paths {
        let mut entries = BTreeSet::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if let Some(s) = entry.path().to_str() {
                entries.insert(s.to_owned());
            }
        }
        out.insert(path.clone(), entries);
    }

    Ok(out)
}

#[cfg(feature = "maildir")]
fn read_files(paths: &BTreeSet<String>) -> Result<BTreeMap<String, Vec<u8>>> {
    use std::fs;

    let mut out = BTreeMap::new();

    for path in paths {
        out.insert(path.clone(), fs::read(path)?);
    }

    Ok(out)
}

#[derive(Serialize)]
#[serde(transparent)]
pub struct MessageView(Message<'static>);

impl fmt::Display for MessageView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for header in self.0.headers() {
            writeln!(f, "{}: {:?}", header.name.as_str(), header.value)?;
        }

        writeln!(f)?;

        for (i, part) in self.0.text_bodies().enumerate() {
            if i > 0 {
                writeln!(f)?;
                writeln!(f)?;
            }

            if let Some(contents) = part.text_contents() {
                write!(f, "{}", contents.trim_end())?;
            }
        }

        Ok(())
    }
}
