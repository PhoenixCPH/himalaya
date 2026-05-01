#[cfg(any(feature = "imap", feature = "jmap"))]
use std::io::{Read, Write};
#[cfg(feature = "maildir")]
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

use anyhow::{bail, Result};
use clap::Parser;
use io_email::coroutines::mailbox_list::{MailboxList, MailboxListArg, MailboxListResult};
use pimalaya_toolbox::terminal::printer::Printer;

use crate::{
    account::Account,
    config::{AccountConfig, Config},
    mailboxes::table::MailboxesTable,
};

#[cfg(any(feature = "imap", feature = "jmap"))]
const READ_BUFFER_SIZE: usize = 16 * 1024;

/// List mailboxes for the active account, regardless of the
/// underlying backend (IMAP, JMAP or Maildir).
#[derive(Debug, Parser)]
pub struct MailboxesListCommand;

impl MailboxesListCommand {
    pub fn execute(
        self,
        printer: &mut impl Printer,
        config: Config,
        account_name: String,
        mut account_config: AccountConfig,
    ) -> Result<()> {
        let _ = account_name;

        #[cfg(feature = "jmap")]
        if let Some(jmap_config) = account_config.jmap.take() {
            let account = Account::new(config, account_config, jmap_config)?;
            let mailboxes = drive_jmap(&account)?;
            return printer.out(MailboxesTable {
                preset: account.table_preset,
                arrangement: account.table_arrangement,
                mailboxes,
            });
        }

        #[cfg(feature = "imap")]
        if let Some(imap_config) = account_config.imap.take() {
            let account = Account::new(config, account_config, imap_config)?;
            let mailboxes = drive_imap(&account)?;
            return printer.out(MailboxesTable {
                preset: account.table_preset,
                arrangement: account.table_arrangement,
                mailboxes,
            });
        }

        #[cfg(feature = "maildir")]
        if let Some(maildir_config) = account_config.maildir.take() {
            let account = Account::new(config, account_config, maildir_config)?;
            let mailboxes = drive_maildir(&account)?;
            return printer.out(MailboxesTable {
                preset: account.table_preset,
                arrangement: account.table_arrangement,
                mailboxes,
            });
        }

        bail!("no compatible backend (jmap, imap, maildir) configured for this account")
    }
}

#[cfg(feature = "jmap")]
fn drive_jmap(account: &Account<crate::config::JmapConfig>) -> Result<Vec<io_email::Mailbox>> {
    use io_jmap::rfc8621::mailbox_query::JmapMailboxQuery;
    use pimalaya_toolbox::stream::jmap::JmapSession;

    let mut jmap = JmapSession::new(
        account.backend.server.clone(),
        account.backend.tls.clone().try_into()?,
        account.backend.auth.clone().try_into()?,
    )?;

    let inner =
        JmapMailboxQuery::new(&jmap.session, &jmap.http_auth, None, None, None, None, None)?;

    let mut coroutine = MailboxList::new(inner);
    let mut buf = [0u8; READ_BUFFER_SIZE];
    let mut arg: Option<MailboxListArg<'_>> = None;

    loop {
        match coroutine.resume(arg.take()) {
            MailboxListResult::Ok(mailboxes) => return Ok(mailboxes),
            MailboxListResult::WantsBytesRead => {
                let n = jmap.stream.read(&mut buf)?;
                arg = Some(MailboxListArg::Bytes(&buf[..n]));
            }
            MailboxListResult::WantsBytesWrite(bytes) => {
                jmap.stream.write_all(&bytes)?;
                arg = None;
            }
            #[allow(unreachable_patterns)]
            other => bail!("unexpected I/O request from JMAP: {other:?}"),
        }
    }
}

#[cfg(feature = "imap")]
fn drive_imap(account: &Account<crate::config::ImapConfig>) -> Result<Vec<io_email::Mailbox>> {
    use io_imap::{
        rfc3501::list::ImapMailboxList,
        types::mailbox::{ListMailbox, Mailbox},
    };
    use pimalaya_toolbox::stream::imap::ImapSession;

    let mut imap = ImapSession::new(
        account.backend.url.clone(),
        account.backend.tls.clone().try_into()?,
        account.backend.starttls,
        account.backend.sasl.clone().try_into()?,
    )?;

    let reference: Mailbox<'static> = "".try_into()?;
    let pattern: ListMailbox<'static> = "*".try_into()?;
    let inner = ImapMailboxList::new(imap.context, reference, pattern);

    let mut coroutine = MailboxList::new(inner);
    let mut buf = [0u8; READ_BUFFER_SIZE];
    let mut arg: Option<MailboxListArg<'_>> = None;

    loop {
        match coroutine.resume(arg.take()) {
            MailboxListResult::Ok(mailboxes) => return Ok(mailboxes),
            MailboxListResult::WantsBytesRead => {
                let n = imap.stream.read(&mut buf)?;
                arg = Some(MailboxListArg::Bytes(&buf[..n]));
            }
            MailboxListResult::WantsBytesWrite(bytes) => {
                imap.stream.write_all(&bytes)?;
                arg = None;
            }
            #[allow(unreachable_patterns)]
            other => bail!("unexpected I/O request from IMAP: {other:?}"),
        }
    }
}

#[cfg(feature = "maildir")]
fn drive_maildir(
    account: &Account<crate::config::MaildirConfig>,
) -> Result<Vec<io_email::Mailbox>> {
    use io_maildir::coroutines::maildir_list::MaildirList;

    let inner = MaildirList::new(&account.backend.root);
    let mut coroutine = MailboxList::new(inner);
    let mut arg: Option<MailboxListArg<'_>> = None;

    loop {
        match coroutine.resume(arg.take()) {
            MailboxListResult::Ok(mailboxes) => return Ok(mailboxes),
            MailboxListResult::WantsDirRead(paths) => {
                arg = Some(MailboxListArg::DirRead(read_dirs(&paths)?));
            }
            #[allow(unreachable_patterns)]
            other => bail!("unexpected I/O request from Maildir: {other:?}"),
        }
    }
}

#[cfg(feature = "maildir")]
fn read_dirs(paths: &BTreeSet<String>) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let mut out = BTreeMap::new();

    for path in paths {
        let mut entries = BTreeSet::new();
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let entry_path = entry.path();
            if let Some(s) = entry_path.to_str() {
                entries.insert(s.to_owned());
            }
        }
        out.insert(path.clone(), entries);
    }

    Ok(out)
}
