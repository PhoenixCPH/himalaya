#[cfg(feature = "jmap")]
use std::io::{Read, Write};
#[cfg(feature = "maildir")]
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Result};
use clap::Parser;
use io_email::coroutines::envelope_list::{EnvelopeList, EnvelopeListArg, EnvelopeListResult};
use pimalaya_toolbox::terminal::printer::Printer;

use crate::{
    account::Account,
    config::{AccountConfig, Config},
    envelopes::table::EnvelopesTable,
};

#[cfg(feature = "jmap")]
const READ_BUFFER_SIZE: usize = 16 * 1024;

/// List envelopes for the active account, regardless of the
/// underlying backend (JMAP or Maildir).
///
/// IMAP envelope listing is not yet wrapped here because it requires a
/// stateful SELECT + FETCH flow; use `himalaya imap envelope list`
/// instead.
#[derive(Debug, Parser)]
pub struct EnvelopesListCommand {
    /// Path or name of the Maildir mailbox (Maildir backend only).
    #[arg(long = "mailbox", short = 'm', value_name = "PATH")]
    pub mailbox: Option<String>,
}

impl EnvelopesListCommand {
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
            let envelopes = drive_jmap(&account)?;
            return printer.out(EnvelopesTable {
                preset: account.table_preset,
                arrangement: account.table_arrangement,
                envelopes,
            });
        }

        #[cfg(feature = "maildir")]
        if let Some(maildir_config) = account_config.maildir.take() {
            let account = Account::new(config, account_config, maildir_config)?;
            let envelopes = drive_maildir(&account, self.mailbox.as_deref())?;
            return printer.out(EnvelopesTable {
                preset: account.table_preset,
                arrangement: account.table_arrangement,
                envelopes,
            });
        }

        #[cfg(feature = "imap")]
        if account_config.imap.is_some() {
            bail!("IMAP envelope listing is not exposed via the cross-protocol command; use `himalaya imap envelope list` instead")
        }

        bail!("no compatible backend (jmap, maildir) configured for this account")
    }
}

#[cfg(feature = "jmap")]
fn drive_jmap(account: &Account<crate::config::JmapConfig>) -> Result<Vec<io_email::Envelope>> {
    use io_jmap::rfc8621::email_query::JmapEmailQuery;
    use pimalaya_toolbox::stream::jmap::JmapSession;

    let mut jmap = JmapSession::new(
        account.backend.server.clone(),
        account.backend.tls.clone().try_into()?,
        account.backend.auth.clone().try_into()?,
    )?;

    let inner =
        JmapEmailQuery::new(&jmap.session, &jmap.http_auth, None, None, None, None, None)?;

    let mut coroutine = EnvelopeList::new(inner);
    let mut buf = [0u8; READ_BUFFER_SIZE];
    let mut arg: Option<EnvelopeListArg<'_>> = None;

    loop {
        match coroutine.resume(arg.take()) {
            EnvelopeListResult::Ok(envelopes) => return Ok(envelopes),
            EnvelopeListResult::WantsBytesRead => {
                let n = jmap.stream.read(&mut buf)?;
                arg = Some(EnvelopeListArg::Bytes(&buf[..n]));
            }
            EnvelopeListResult::WantsBytesWrite(bytes) => {
                jmap.stream.write_all(&bytes)?;
                arg = None;
            }
            #[allow(unreachable_patterns)]
            other => bail!("unexpected I/O request from JMAP: {other:?}"),
        }
    }
}

#[cfg(feature = "maildir")]
fn drive_maildir(
    account: &Account<crate::config::MaildirConfig>,
    mailbox: Option<&str>,
) -> Result<Vec<io_email::Envelope>> {
    use io_maildir::{coroutines::message_list::MaildirMessagesList, maildir::Maildir};

    let path = match mailbox {
        Some(name) => account.backend.root.join(name),
        None => account.backend.root.clone(),
    };
    let maildir = Maildir::try_from(path)?;

    let inner = MaildirMessagesList::new(maildir);
    let mut coroutine = EnvelopeList::new(inner);
    let mut arg: Option<EnvelopeListArg<'_>> = None;

    loop {
        match coroutine.resume(arg.take()) {
            EnvelopeListResult::Ok(envelopes) => return Ok(envelopes),
            EnvelopeListResult::WantsDirRead(paths) => {
                arg = Some(EnvelopeListArg::DirRead(read_dirs(&paths)?));
            }
            EnvelopeListResult::WantsFileRead(paths) => {
                arg = Some(EnvelopeListArg::FileRead(read_files(&paths)?));
            }
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
