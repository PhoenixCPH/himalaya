#[cfg(any(feature = "imap", feature = "jmap"))]
use std::io::{Read, Write};
#[cfg(feature = "maildir")]
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Result};
use clap::Parser;
use io_email::coroutines::flag_add::{FlagAdd, FlagAddArg, FlagAddResult};
use pimalaya_toolbox::terminal::printer::{Message, Printer};

use crate::{
    account::Account,
    config::{AccountConfig, Config},
    flags::arg::{FlagsArg, MailboxFlag, MessageIdsArg, SequenceFlag},
};

#[cfg(any(feature = "imap", feature = "jmap"))]
const READ_BUFFER_SIZE: usize = 16 * 1024;

/// Add flag(s) to message(s) for the active account.
#[derive(Debug, Parser)]
pub struct FlagsAddCommand {
    #[command(flatten)]
    pub ids: MessageIdsArg,
    #[command(flatten)]
    pub flags: FlagsArg,
    #[command(flatten)]
    pub mailbox: MailboxFlag,
    #[command(flatten)]
    pub seq: SequenceFlag,
}

impl FlagsAddCommand {
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
            drive_jmap(&account, &self.ids.inner, &self.flags.inner)?;
            return printer.out(Message::new("Flag(s) successfully added"));
        }

        #[cfg(feature = "imap")]
        if let Some(imap_config) = account_config.imap.take() {
            let account = Account::new(config, account_config, imap_config)?;
            drive_imap(
                &account,
                &self.mailbox.inner,
                &self.ids.inner,
                &self.flags.inner,
                self.seq.sequence,
            )?;
            return printer.out(Message::new("Flag(s) successfully added"));
        }

        #[cfg(feature = "maildir")]
        if let Some(maildir_config) = account_config.maildir.take() {
            let account = Account::new(config, account_config, maildir_config)?;
            drive_maildir(&account, &self.mailbox.inner, &self.ids.inner, &self.flags.inner)?;
            return printer.out(Message::new("Flag(s) successfully added"));
        }

        bail!("no compatible backend (jmap, imap, maildir) configured for this account")
    }
}

#[cfg(feature = "jmap")]
fn drive_jmap(
    account: &Account<crate::config::JmapConfig>,
    ids: &[String],
    flags: &[crate::flags::arg::FlagArg],
) -> Result<()> {
    use io_jmap::rfc8621::email_set::{JmapEmailSet, JmapEmailSetArgs};
    use pimalaya_toolbox::stream::jmap::JmapSession;

    let mut jmap = JmapSession::new(
        account.backend.server.clone(),
        account.backend.tls.clone().try_into()?,
        account.backend.auth.clone().try_into()?,
    )?;

    let mut args = JmapEmailSetArgs::default();
    for id in ids {
        for flag in flags {
            args.set_keyword(id.clone(), flag.jmap());
        }
    }

    let inner = JmapEmailSet::new(&jmap.session, &jmap.http_auth, args)?;
    let mut coroutine = FlagAdd::new(inner);
    let mut buf = [0u8; READ_BUFFER_SIZE];
    let mut arg: Option<FlagAddArg<'_>> = None;

    loop {
        match coroutine.resume(arg.take()) {
            FlagAddResult::Ok => return Ok(()),
            FlagAddResult::WantsBytesRead => {
                let n = jmap.stream.read(&mut buf)?;
                arg = Some(FlagAddArg::Bytes(&buf[..n]));
            }
            FlagAddResult::WantsBytesWrite(bytes) => {
                jmap.stream.write_all(&bytes)?;
                arg = None;
            }
            FlagAddResult::Err(err) => bail!(err),
            #[allow(unreachable_patterns)]
            other => bail!("unexpected I/O request from JMAP: {other:?}"),
        }
    }
}

#[cfg(feature = "imap")]
fn drive_imap(
    account: &Account<crate::config::ImapConfig>,
    mailbox: &str,
    ids: &[String],
    flags: &[crate::flags::arg::FlagArg],
    sequence: bool,
) -> Result<()> {
    use io_imap::{
        rfc3501::{select::*, store::ImapMessageStore},
        types::{flag::StoreType, mailbox::Mailbox, sequence::SequenceSet},
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

    let sequence_set: SequenceSet = ids.join(",").as_str().try_into()?;
    let imap_flags = flags.iter().map(|f| f.imap()).collect();
    let inner = ImapMessageStore::new(imap.context, sequence_set, StoreType::Add, imap_flags, !sequence);

    let mut coroutine = FlagAdd::new(inner);
    let mut arg: Option<FlagAddArg<'_>> = None;

    loop {
        match coroutine.resume(arg.take()) {
            FlagAddResult::Ok => return Ok(()),
            FlagAddResult::WantsBytesRead => {
                let n = imap.stream.read(&mut buf)?;
                arg = Some(FlagAddArg::Bytes(&buf[..n]));
            }
            FlagAddResult::WantsBytesWrite(bytes) => {
                imap.stream.write_all(&bytes)?;
                arg = None;
            }
            FlagAddResult::Err(err) => bail!(err),
            #[allow(unreachable_patterns)]
            other => bail!("unexpected I/O request from IMAP: {other:?}"),
        }
    }
}

#[cfg(feature = "maildir")]
fn drive_maildir(
    account: &Account<crate::config::MaildirConfig>,
    mailbox: &str,
    ids: &[String],
    flags: &[crate::flags::arg::FlagArg],
) -> Result<()> {
    use io_maildir::{
        coroutines::flags_add::MaildirFlagsAdd,
        flag::{Flag, Flags},
        maildir::Maildir,
    };

    use crate::maildir::runtime;

    let path = account.backend.root.join(mailbox);
    let maildir = Maildir::try_from(path)?;
    let maildir_flags: Flags = flags.iter().map(|f| Flag::from(f)).collect();

    for id in ids {
        let inner = MaildirFlagsAdd::new(maildir.clone(), id.as_str(), maildir_flags.clone());
        let mut coroutine = FlagAdd::new(inner);
        let mut arg: Option<FlagAddArg<'_>> = None;

        loop {
            match coroutine.resume(arg.take()) {
                FlagAddResult::Ok => break,
                FlagAddResult::WantsDirRead(paths) => {
                    arg = Some(FlagAddArg::DirRead(read_dirs(&paths)?));
                }
                FlagAddResult::WantsRename(pairs) => {
                    runtime::rename(pairs)?;
                    arg = Some(FlagAddArg::Rename);
                }
                FlagAddResult::Err(err) => bail!(err),
                #[allow(unreachable_patterns)]
                other => bail!("unexpected I/O request from Maildir: {other:?}"),
            }
        }
    }

    Ok(())
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
