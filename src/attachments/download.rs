use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Result};
use clap::Parser;
use mail_parser::{MessageParser, MimeHeaders};
use pimalaya_cli::printer::{Message, Printer};

use crate::{
    account::Account,
    cli::BackendArg,
    config::{AccountConfig, Config},
    flags::arg::MailboxIdArg,
};

/// Download specific attachments of a single message to disk.
///
/// The attachment ids are the 1-based positions reported by
/// `attachments list`. Pass one or more ids to fetch exactly those
/// parts. Inline parts are addressable by their id too — the id you
/// see in `attachments list --inline` is the same id you pass here.
///
/// The destination directory defaults to the account's
/// `downloads-dir` config (falling back to the global one, then the
/// platform's standard downloads directory). Pass `--dir <PATH>` to
/// override.
#[derive(Debug, Parser)]
pub struct AttachmentDownloadCommand {
    #[command(flatten)]
    pub mailbox_id: MailboxIdArg,

    /// Identifier of the message.
    #[arg(value_name = "MESSAGE-ID")]
    pub message_id: String,

    /// Attachment identifier(s) to download.
    ///
    /// Omit identifiers to download all attachments.
    #[arg(value_name = "ATTACHMENT-ID", num_args = 0..)]
    pub attachment_ids: Vec<String>,

    /// Destination directory.
    ///
    /// Overrides the account/global `downloads-dir` config.
    #[arg(long, short, value_name = "PATH")]
    pub dir: Option<PathBuf>,
}

impl AttachmentDownloadCommand {
    pub fn execute(
        self,
        printer: &mut impl Printer,
        config: Config,
        account_config: AccountConfig,
        backend: BackendArg,
    ) -> Result<()> {
        let raw = crate::messages::fetch::fetch_raw(
            &config,
            &account_config,
            backend,
            &self.mailbox_id.inner,
            &self.message_id,
        )?;

        let Some(message) = MessageParser::new().parse(&raw) else {
            bail!("Failed to parse RFC 5322 message");
        };

        let account = Account::new(config, account_config, ())?;
        let dir = self.dir.clone().unwrap_or(account.downloads_dir);

        if !dir.exists() {
            fs::create_dir_all(&dir)?;
        }

        let wanted_all = self.attachment_ids.is_empty();
        let mut remaining: BTreeSet<String> = self.attachment_ids.iter().cloned().collect();
        let mut written = Vec::new();

        for (index, part) in message.attachments().enumerate() {
            let id = (index + 1).to_string();
            if !wanted_all && !remaining.remove(&id) {
                continue;
            }

            let filename = part
                .attachment_name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("attachment-{id}"));
            let safe = sanitize(&filename);
            let path = unique_path(&dir, &safe);

            fs::write(&path, part.contents())?;
            written.push(path.display().to_string());
        }

        if !remaining.is_empty() {
            let missing: Vec<String> = remaining.into_iter().collect();
            bail!(
                "no attachment with id {} on message `{}`",
                missing.join(", "),
                self.message_id,
            );
        }

        printer.out(Message::new(format!(
            "Downloaded {} attachment(s):\n - {}",
            written.len(),
            written.join("\n - ")
        )))
    }
}

/// Strips path separators and parent traversals so a hostile filename
/// header can't escape the download directory.
fn sanitize(name: &str) -> String {
    let trimmed = name.trim();
    let cleaned: String = trimmed
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            _ => c,
        })
        .collect();
    let cleaned = cleaned.trim_start_matches('.').trim();
    if cleaned.is_empty() {
        "attachment".to_string()
    } else {
        cleaned.to_string()
    }
}

/// Returns a path inside `dir` that doesn't already exist by suffixing
/// `(1)`, `(2)`, … to the stem when needed.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }

    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };

    for n in 1..1024 {
        let candidate = dir.join(format!("{stem} ({n}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(name)
}
