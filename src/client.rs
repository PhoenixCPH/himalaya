//! Cross-protocol [`EmailClient`] for the shared subcommands
//! (`mailboxes`, `envelopes`, `flags`, `messages`).
//!
//! Wraps [`io_email::client::EmailClient`] and bundles the
//! account-level rendering settings (downloads dir, table preset and
//! arrangement) the shared commands need alongside the I/O client.
//! Implements [`Deref`]/[`DerefMut`] onto the inner client so
//! callers can call its methods directly:
//!
//! ```ignore
//! let mut client = EmailClient::new(config, account_config, backend)?;
//! let mailboxes = client.list_mailboxes()?;
//! ```
//!
//! Construction is backend-asymmetric (IMAP needs TLS + SASL, JMAP
//! needs an HTTP credential, Maildir just needs a root path). We
//! delegate to the transitional [`ImapSession`] / [`JmapSession`]
//! helpers for the handshake/auth flow then bridge the resulting
//! `(stream, context)` pairs into [`io_imap::client::ImapClient`] /
//! [`io_jmap::client::JmapClient`] via their `from_parts`
//! constructors.
//!
//! [`ImapSession`]: crate::imap::session::ImapSession
//! [`JmapSession`]: crate::jmap::session::JmapSession

use std::{
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use anyhow::{bail, Result};
use comfy_table::ContentArrangement;

use crate::{
    account::Account,
    cli::BackendArg,
    config::{AccountConfig, Config},
};

pub struct EmailClient {
    inner: io_email::client::EmailClient,
    #[allow(dead_code)]
    pub downloads_dir: PathBuf,
    pub table_preset: String,
    pub table_arrangement: ContentArrangement,
    pub datetime_fmt: String,
    pub datetime_local_tz: bool,
}

impl EmailClient {
    /// Selects the first backend in `imap → jmap → maildir` order
    /// whose config block is present and whose [`BackendArg`] filter
    /// allows it. Bails when nothing matches. SMTP is omitted on
    /// purpose: none of the shared read-side operations have an SMTP
    /// implementation.
    pub fn new(
        config: Config,
        mut account_config: AccountConfig,
        backend: BackendArg,
    ) -> Result<Self> {
        #[cfg(feature = "imap")]
        if backend.allows_imap() {
            if let Some(imap_config) = account_config.imap.take() {
                use crate::imap::session::ImapSession;
                use io_imap::client::ImapClient;

                let account = Account::new(config, account_config, imap_config)?;
                let session = ImapSession::new(
                    account.backend.url.clone(),
                    account.backend.tls.clone().try_into()?,
                    account.backend.starttls,
                    account.backend.sasl.clone().try_into()?,
                )?;
                let client = ImapClient::from_parts(session.stream, session.context);
                return Ok(Self {
                    inner: client.into(),
                    downloads_dir: account.downloads_dir,
                    table_preset: account.table_preset,
                    table_arrangement: account.table_arrangement,
                    datetime_fmt: account.datetime_fmt,
                    datetime_local_tz: account.datetime_local_tz,
                });
            }
        }

        #[cfg(feature = "jmap")]
        if backend.allows_jmap() {
            if let Some(jmap_config) = account_config.jmap.take() {
                use crate::jmap::session::JmapSession;
                use io_jmap::client::JmapClient;

                let account = Account::new(config, account_config, jmap_config)?;
                let session = JmapSession::new(
                    account.backend.server.clone(),
                    account.backend.tls.clone().try_into()?,
                    account.backend.auth.clone().try_into()?,
                )?;
                let client =
                    JmapClient::from_parts(session.stream, session.http_auth, session.session);
                return Ok(Self {
                    inner: client.into(),
                    downloads_dir: account.downloads_dir,
                    table_preset: account.table_preset,
                    table_arrangement: account.table_arrangement,
                    datetime_fmt: account.datetime_fmt,
                    datetime_local_tz: account.datetime_local_tz,
                });
            }
        }

        #[cfg(feature = "maildir")]
        if backend.allows_maildir() {
            if let Some(maildir_config) = account_config.maildir.take() {
                use io_maildir::client::MaildirClient;

                let account = Account::new(config, account_config, maildir_config)?;
                let client = MaildirClient::new(account.backend.root.clone());
                return Ok(Self {
                    inner: client.into(),
                    downloads_dir: account.downloads_dir,
                    table_preset: account.table_preset,
                    table_arrangement: account.table_arrangement,
                    datetime_fmt: account.datetime_fmt,
                    datetime_local_tz: account.datetime_local_tz,
                });
            }
        }

        bail!("no backend matching `{backend}` is configured for this account")
    }
}

impl Deref for EmailClient {
    type Target = io_email::client::EmailClient;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for EmailClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
