use std::fmt;

use anyhow::{bail, Result};
use clap::Parser;
use comfy_table::{Cell, ContentArrangement, Row, Table};
use mail_parser::{MessageParser, MessagePart, MimeHeaders};
use pimalaya_cli::printer::Printer;
use serde::Serialize;

use crate::{
    account::Account,
    cli::BackendArg,
    config::{AccountConfig, Config},
    flags::arg::MailboxIdArg,
};

/// List the attachments carried by a single message in the active
/// account.
///
/// Each row carries a 1-based `ID` matching the position of the part
/// in mail_parser's attachment iteration order. The `ID` is stable
/// regardless of the `--inline` filter — listing only the attachment
/// parts and listing every non-body part assign the same id to the
/// same underlying part. So if a message has parts `1=attachment,
/// 2=attachment, 3=inline, 4=attachment`, the default listing shows
/// `1 2 4` and `--inline` shows `1 2 3 4`.
///
/// Pass `--inline` to surface inline parts (typically embedded images
/// referenced by HTML bodies via `cid:`).
#[derive(Debug, Parser)]
pub struct AttachmentListCommand {
    #[command(flatten)]
    pub mailbox_id: MailboxIdArg,
    /// Identifier of the message.
    #[arg(value_name = "MESSAGE-ID")]
    pub message_id: String,
    /// Include parts with `Content-Disposition: inline`.
    #[arg(long, short)]
    pub inline: bool,
}

impl AttachmentListCommand {
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

        let mut attachments = Vec::new();
        for (index, part) in message.attachments().enumerate() {
            let inline = is_inline(part);
            if inline && !self.inline {
                continue;
            }

            attachments.push(Attachment {
                id: (index + 1).to_string(),
                filename: part.attachment_name().map(str::to_owned),
                mime: mime_string(part),
                size: part.contents().len() as u64,
                inline,
            });
        }

        // Reuse the active account's table styling. Constructing
        // an `Account<()>` is enough to read the preset/arrangement.
        let account = Account::new(config, account_config, ())?;

        let attachments = Attachments {
            preset: account.table_preset,
            arrangement: account.table_arrangement,
            with_inline: self.inline,
            attachments,
        };

        printer.out(attachments)
    }
}

fn is_inline(part: &MessagePart<'_>) -> bool {
    part.content_disposition()
        .map(|cd| cd.c_type.eq_ignore_ascii_case("inline"))
        .unwrap_or(false)
}

fn mime_string(part: &MessagePart<'_>) -> Option<String> {
    let ct = part.content_type()?;
    Some(match ct.c_subtype.as_deref() {
        Some(sub) => format!("{}/{}", ct.c_type, sub),
        None => ct.c_type.to_string(),
    })
}

/// One row of the `attachments list` output.
#[derive(Clone, Debug, Serialize)]
pub struct Attachment {
    /// 1-based linear index in mail-parser's attachment iteration
    /// order. Stable across the `--inline` filter.
    pub id: String,
    /// Filename from `Content-Disposition: filename=` (or
    /// `Content-Type: name=`), RFC 2231-decoded. `None` when the
    /// source provides no name.
    pub filename: Option<String>,
    /// MIME type (e.g. `"application/pdf"`). `None` when the source
    /// omits the `Content-Type` header.
    pub mime: Option<String>,
    /// Size in bytes of the decoded part body.
    pub size: u64,
    /// `true` when the part carries `Content-Disposition: inline`.
    pub inline: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Attachments {
    #[serde(skip)]
    pub preset: String,
    #[serde(skip)]
    pub arrangement: ContentArrangement,
    #[serde(skip)]
    pub with_inline: bool,
    pub attachments: Vec<Attachment>,
}

impl fmt::Display for Attachments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut table = Table::new();

        let mut header = vec![
            Cell::new("ID"),
            Cell::new("FILENAME"),
            Cell::new("TYPE"),
            Cell::new("SIZE"),
        ];
        if self.with_inline {
            header.push(Cell::new("INLINE"));
        }

        table
            .load_preset(&self.preset)
            .set_content_arrangement(self.arrangement.clone())
            .set_header(Row::from(header))
            .add_rows(self.attachments.iter().map(|a| {
                let mut row = Row::new();
                row.max_height(1);
                row.add_cell(Cell::new(&a.id));
                row.add_cell(Cell::new(a.filename.as_deref().unwrap_or("")));
                row.add_cell(Cell::new(a.mime.as_deref().unwrap_or("")));
                row.add_cell(Cell::new(human_size(a.size)));
                if self.with_inline {
                    row.add_cell(Cell::new(if a.inline { "yes" } else { "no" }));
                }
                row
            }));

        writeln!(f)?;
        writeln!(f, "{table}")
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}
