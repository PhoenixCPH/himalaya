use std::{
    borrow::Cow,
    collections::HashSet,
    io::{stdin, BufRead, IsTerminal, Read, Write},
};

use anyhow::{bail, Result};
use clap::Parser;
use io_email::coroutines::message_send::{MessageSend, MessageSendArg, MessageSendResult};
use io_smtp::{
    rfc5321::types::{
        domain::Domain, ehlo_domain::EhloDomain, forward_path::ForwardPath, local_part::LocalPart,
        mailbox::Mailbox, reverse_path::ReversePath,
    },
    send::SmtpMessageSend,
};
use mail_parser::{Addr, Address, HeaderName, HeaderValue, MessageParser};
use pimalaya_toolbox::terminal::printer::{Message, Printer};

use crate::{
    account::Account,
    config::{AccountConfig, Config},
};

const READ_BUFFER_SIZE: usize = 8 * 1024;

/// Send a message via SMTP for the active account.
#[derive(Debug, Parser)]
pub struct MessagesSendCommand {
    /// The raw message, including headers and body.
    #[arg(trailing_var_arg = true)]
    #[arg(name = "message", value_name = "MESSAGE")]
    pub message: Vec<String>,
}

impl MessagesSendCommand {
    pub fn execute(
        self,
        printer: &mut impl Printer,
        config: Config,
        account_name: String,
        mut account_config: AccountConfig,
    ) -> Result<()> {
        let _ = account_name;

        let Some(smtp_config) = account_config.smtp.take() else {
            bail!("no SMTP backend configured for this account")
        };

        let account = Account::new(config, account_config, smtp_config)?;
        let mut smtp = pimalaya_toolbox::stream::smtp::SmtpSession::new(
            account.backend.url.clone(),
            account.backend.tls.clone().try_into()?,
            account.backend.starttls,
            account.backend.sasl.clone().try_into()?,
        )?;

        let raw = if stdin().is_terminal() || printer.is_json() {
            self.message
                .join(" ")
                .replace('\r', "")
                .replace('\n', "\r\n")
        } else {
            stdin()
                .lock()
                .lines()
                .map_while(Result::ok)
                .collect::<Vec<String>>()
                .join("\r\n")
        };

        let (reverse_path, forward_paths) = into_smtp_msg(raw.as_bytes())?;

        let inner = SmtpMessageSend::new(reverse_path, forward_paths, raw.into_bytes());
        let mut coroutine = MessageSend::new(inner);
        let mut buf = [0u8; READ_BUFFER_SIZE];
        let mut arg: Option<MessageSendArg<'_>> = None;

        loop {
            match coroutine.resume(arg.take()) {
                MessageSendResult::Ok => break,
                MessageSendResult::WantsBytesRead => {
                    let n = smtp.stream.read(&mut buf)?;
                    arg = Some(MessageSendArg::Bytes(&buf[..n]));
                }
                MessageSendResult::WantsBytesWrite(bytes) => {
                    smtp.stream.write_all(&bytes)?;
                    arg = None;
                }
                MessageSendResult::Err(err) => bail!(err),
            }
        }

        printer.out(Message::new("Message successfully sent"))
    }
}

fn into_smtp_msg<'a>(msg: &[u8]) -> Result<(ReversePath<'a>, Vec<ForwardPath<'a>>)> {
    let Some(parsed) = MessageParser::new().parse_headers(msg) else {
        bail!("Invalid message to send")
    };

    let mut mail_from = None;
    let mut rcpt_to = HashSet::new();

    for header in parsed.headers() {
        let key = &header.name;
        let val = header.value();

        match key {
            HeaderName::From => match val {
                HeaderValue::Address(Address::List(addrs)) => {
                    if let Some(email) = addrs.first().and_then(find_valid_email) {
                        mail_from = email.to_string().into();
                    }
                }
                HeaderValue::Address(Address::Group(groups)) => {
                    if let Some(group) = groups.first() {
                        if let Some(email) = group.addresses.first().and_then(find_valid_email) {
                            mail_from = email.to_string().into();
                        }
                    }
                }
                _ => (),
            },
            HeaderName::To | HeaderName::Cc | HeaderName::Bcc => match val {
                HeaderValue::Address(Address::List(addrs)) => {
                    rcpt_to.extend(addrs.iter().filter_map(find_valid_email));
                }
                HeaderValue::Address(Address::Group(groups)) => {
                    rcpt_to.extend(
                        groups
                            .iter()
                            .flat_map(|group| group.addresses.iter())
                            .filter_map(find_valid_email),
                    );
                }
                _ => (),
            },
            _ => (),
        };
    }

    let Some(mail_from) = mail_from else {
        bail!("The message does not contain any sender");
    };

    if rcpt_to.is_empty() {
        bail!("The message does not contain any recipient");
    }

    let Some((local, domain)) = mail_from.split_once('@') else {
        bail!("The message contains an invalid sender");
    };

    let mbox = Mailbox {
        local_part: LocalPart(Cow::Owned(local.to_owned())),
        domain: EhloDomain::Domain(Domain(Cow::Owned(domain.to_owned()))),
    };

    let reverse_path = ReversePath::Mailbox(mbox);

    let mut forward_paths = Vec::new();

    for rcpt in rcpt_to {
        let Some((local, domain)) = rcpt.split_once('@') else {
            bail!("The message contains an invalid recipient: {rcpt}");
        };

        let mbox = Mailbox {
            local_part: LocalPart(Cow::Owned(local.to_owned())),
            domain: EhloDomain::Domain(Domain(Cow::Owned(domain.to_owned()))),
        };

        forward_paths.push(ForwardPath(mbox))
    }

    Ok((reverse_path, forward_paths))
}

fn find_valid_email(addr: &Addr) -> Option<String> {
    match &addr.address {
        None => None,
        Some(email) => {
            let email = email.trim();
            if email.is_empty() {
                None
            } else {
                Some(email.to_string())
            }
        }
    }
}
