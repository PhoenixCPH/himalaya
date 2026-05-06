//! Interactive configuration wizard.
//!
//! Triggered by `cli::load_or_wizard` when no config file is found
//! ([`pimalaya_config::toml::TomlConfig::from_paths_or_default`]
//! returned `Ok(None)`).
//!
//! Flow:
//!
//! 1. Confirm with the user. Exit if they decline.
//! 2. Ask for an account name and email address.
//! 3. Run discovery — PACC and Mozilla Autoconfig in parallel via
//!    `std::thread::scope`. PACC results take precedence; missing
//!    fields are filled from Autoconfig.
//! 4. Convert any discovery hit into [`WizardImapConfig`] /
//!    [`WizardSmtpConfig`] defaults, hand them to the per-protocol
//!    wizards in [`pimalaya_cli::wizard`].
//! 5. Build a [`Config`], write it to `target`, return it.

use std::{collections::HashMap, path::Path, process::exit, thread};

use anyhow::{anyhow, bail, Result};
use io_discovery::{
    autoconfig::{
        client::DiscoveryAutoconfigClient,
        coroutines::{dns_mx::mx_parent_domain, isp::DiscoveryIsp},
        types::{Autoconfig, SecurityType, Server, ServerType},
    },
    pacc::{
        client::{DiscoveryPaccClient, DiscoveryPaccClientError},
        types::PaccConfig,
    },
};
use io_process::command::Command;
use log::{debug, info};
use pimalaya_cli::wizard::{
    imap::{
        self as imap_wizard, Encryption as ImapEncryption, ImapAuth, ImapSecret, WizardImapConfig,
    },
    smtp::{
        self as smtp_wizard, Encryption as SmtpEncryption, SmtpAuth, SmtpSecret, WizardSmtpConfig,
    },
};
use pimalaya_config::secret::Secret;
use url::Url;

use crate::config::{
    AccountConfig, Config, ImapConfig, SaslConfig, SaslMechanismConfig, SaslPlainConfig, SmtpConfig,
};

/// DNS resolver used by PACC discovery. Cloudflare's `1.1.1.1` is a
/// reasonable default; we'll make this configurable later.
const DEFAULT_RESOLVER: &str = "tcp://1.1.1.1:53";

pub fn run_or_exit(target: &Path) -> Result<Config> {
    let prompt = format!(
        "No configuration found. Create one at {}?",
        target.display(),
    );

    if !pimalaya_cli::prompt::bool(&prompt, true)? {
        exit(0);
    }

    let account_name = pimalaya_cli::prompt::text("Account name:", Some("default"))?;
    let email = pimalaya_cli::prompt::text::<&str>("Email address:", None)?;

    let (local_part, domain) = email
        .split_once('@')
        .ok_or_else(|| anyhow!("Invalid email address `{email}`: missing `@`"))?;

    info!("Looking up provider settings for {domain}…");
    let (imap_defaults, smtp_defaults) = discover(local_part, domain);

    match (&imap_defaults, &smtp_defaults) {
        (None, None) => {
            info!("No provider settings auto-discovered, please enter them manually.");
        }
        (imap, smtp) => {
            info!(
                "Provider settings auto-discovered (imap: {}, smtp: {}).",
                imap.is_some(),
                smtp.is_some(),
            );
        }
    }

    let imap = imap_wizard::run(&account_name, local_part, domain, imap_defaults.as_ref())?;
    let smtp = smtp_wizard::run(&account_name, local_part, domain, smtp_defaults.as_ref())?;

    let account = AccountConfig {
        default: true,
        downloads_dir: None,
        table_preset: None,
        table_arrangement: None,
        envelope: Default::default(),
        imap: Some(imap_to_config(imap)?),
        jmap: None,
        maildir: None,
        smtp: Some(smtp_to_config(smtp)?),
    };

    let config = Config {
        downloads_dir: None,
        table_preset: None,
        table_arrangement: None,
        envelope: Default::default(),
        accounts: HashMap::from([(account_name, account)]),
    };

    config.write(target)?;
    info!("Configuration written to {}.", target.display());

    Ok(config)
}

/// Runs PACC and Mozilla Autoconfig probes in parallel and merges
/// their results. PACC values are preferred when both succeed, with
/// Autoconfig filling in any field PACC didn't return.
fn discover(
    local_part: &str,
    domain: &str,
) -> (Option<WizardImapConfig>, Option<WizardSmtpConfig>) {
    thread::scope(|scope| {
        let pacc_handle = scope.spawn(|| run_pacc(domain));
        let autoconfig_handle = scope.spawn(|| run_autoconfig(local_part, domain));

        let pacc = pacc_handle.join().unwrap_or_else(|_| {
            debug!("PACC discovery thread panicked");
            None
        });
        let autoconfig = autoconfig_handle.join().unwrap_or_else(|_| {
            debug!("Autoconfig discovery thread panicked");
            None
        });

        let (pacc_imap, pacc_smtp) = pacc.as_ref().map(pacc_defaults).unwrap_or((None, None));
        let (autoconfig_imap, autoconfig_smtp) = autoconfig
            .as_ref()
            .map(autoconfig_defaults)
            .unwrap_or((None, None));

        (pacc_imap.or(autoconfig_imap), pacc_smtp.or(autoconfig_smtp))
    })
}

fn run_pacc(domain: &str) -> Option<PaccConfig> {
    let resolver: Url = match DEFAULT_RESOLVER.parse() {
        Ok(url) => url,
        Err(err) => {
            debug!("PACC: invalid default resolver `{DEFAULT_RESOLVER}`: {err}");
            return None;
        }
    };

    let mut client = DiscoveryPaccClient::new(resolver);
    match client.discover(domain) {
        Ok(config) => Some(config),
        Err(DiscoveryPaccClientError::Discovery(err)) => {
            debug!("PACC discovery for {domain} failed: {err}");
            None
        }
        Err(err) => {
            debug!("PACC transport error for {domain}: {err}");
            None
        }
    }
}

/// Tries the Mozilla Autoconfig discovery chain — direct ISP URLs,
/// then MX-derived parent domain ISP URLs. The TXT mailconf and SRV
/// fallbacks from the autoconfig CLI are skipped here; we keep the
/// wizard fast and let manual entry handle the long tail.
fn run_autoconfig(local_part: &str, domain: &str) -> Option<Autoconfig> {
    let resolver: Url = match DEFAULT_RESOLVER.parse() {
        Ok(url) => url,
        Err(err) => {
            debug!("Autoconfig: invalid default resolver `{DEFAULT_RESOLVER}`: {err}");
            return None;
        }
    };

    let mut client = DiscoveryAutoconfigClient::new(resolver);

    if let Some(ac) = try_isp_urls(&mut client, local_part, domain) {
        return Some(ac);
    }

    let mx_parent = match client.mx(domain) {
        Ok(records) => records
            .first()
            .map(|r| r.rdata.exchange.to_string())
            .and_then(|t| mx_parent_domain(&t))
            .filter(|d| d != domain),
        Err(err) => {
            debug!("Autoconfig MX lookup for {domain} failed: {err}");
            None
        }
    };

    if let Some(parent) = mx_parent {
        debug!("Autoconfig: re-trying ISPs against MX parent {parent}");
        if let Some(ac) = try_isp_urls(&mut client, local_part, &parent) {
            return Some(ac);
        }
    }

    None
}

fn try_isp_urls(
    client: &mut DiscoveryAutoconfigClient,
    local_part: &str,
    domain: &str,
) -> Option<Autoconfig> {
    let urls = match DiscoveryIsp::all_urls(local_part, domain) {
        Ok(urls) => urls,
        Err(err) => {
            debug!("Autoconfig: cannot build ISP URLs for {domain}: {err}");
            return None;
        }
    };

    for url in urls {
        match client.isp(url.clone()) {
            Ok(ac) => return Some(ac),
            Err(err) => debug!("Autoconfig ISP attempt at {url} failed: {err}"),
        }
    }

    None
}

fn autoconfig_defaults(ac: &Autoconfig) -> (Option<WizardImapConfig>, Option<WizardSmtpConfig>) {
    let imap = ac
        .email_provider
        .incoming_server
        .iter()
        .find(|s| matches!(s.r#type, ServerType::Imap))
        .and_then(autoconfig_imap);

    let smtp = ac
        .email_provider
        .outgoing_server
        .iter()
        .find(|s| matches!(s.r#type, ServerType::Smtp))
        .and_then(autoconfig_smtp);

    (imap, smtp)
}

fn autoconfig_imap(server: &Server) -> Option<WizardImapConfig> {
    let host = server.hostname.clone()?;
    let encryption = match server.socket_type {
        Some(SecurityType::Tls) => ImapEncryption::Tls,
        Some(SecurityType::Starttls) => ImapEncryption::StartTls,
        _ => ImapEncryption::None,
    };
    let port = server.port.unwrap_or(match encryption {
        ImapEncryption::Tls => 993,
        _ => 143,
    });

    Some(WizardImapConfig {
        host,
        port,
        encryption,
        login: String::new(),
        auth: ImapAuth::Password(ImapSecret::Raw(String::new().into())),
    })
}

fn autoconfig_smtp(server: &Server) -> Option<WizardSmtpConfig> {
    let host = server.hostname.clone()?;
    let encryption = match server.socket_type {
        Some(SecurityType::Tls) => SmtpEncryption::Tls,
        Some(SecurityType::Starttls) => SmtpEncryption::StartTls,
        _ => SmtpEncryption::None,
    };
    let port = server.port.unwrap_or(match encryption {
        SmtpEncryption::Tls => 465,
        SmtpEncryption::StartTls => 587,
        SmtpEncryption::None => 25,
    });

    Some(WizardSmtpConfig {
        host,
        port,
        encryption,
        login: String::new(),
        auth: SmtpAuth::Password(SmtpSecret::Raw(String::new().into())),
    })
}

fn pacc_defaults(config: &PaccConfig) -> (Option<WizardImapConfig>, Option<WizardSmtpConfig>) {
    let imap = config.protocols.imap.as_ref().map(|p| WizardImapConfig {
        host: p.host.clone(),
        port: 993,
        encryption: ImapEncryption::Tls,
        login: String::new(),
        // Placeholder; the user picks their real auth in the wizard.
        // Only the host/port/encryption fields are read as defaults.
        auth: ImapAuth::Password(ImapSecret::Raw(String::new().into())),
    });

    let smtp = config.protocols.smtp.as_ref().map(|p| WizardSmtpConfig {
        host: p.host.clone(),
        port: 465,
        encryption: SmtpEncryption::Tls,
        login: String::new(),
        auth: SmtpAuth::Password(SmtpSecret::Raw(String::new().into())),
    });

    (imap, smtp)
}

fn imap_to_config(w: WizardImapConfig) -> Result<ImapConfig> {
    let scheme = match w.encryption {
        ImapEncryption::Tls => "imaps",
        ImapEncryption::StartTls | ImapEncryption::None => "imap",
    };
    let url = Url::parse(&format!("{scheme}://{}:{}", w.host, w.port))?;
    let starttls = matches!(w.encryption, ImapEncryption::StartTls);
    let sasl = build_sasl_imap(&w.login, w.auth)?;

    Ok(ImapConfig {
        url,
        tls: Default::default(),
        starttls,
        sasl,
    })
}

fn smtp_to_config(w: WizardSmtpConfig) -> Result<SmtpConfig> {
    let scheme = match w.encryption {
        SmtpEncryption::Tls => "smtps",
        SmtpEncryption::StartTls | SmtpEncryption::None => "smtp",
    };
    let url = Url::parse(&format!("{scheme}://{}:{}", w.host, w.port))?;
    let starttls = matches!(w.encryption, SmtpEncryption::StartTls);
    let sasl = build_sasl_smtp(&w.login, w.auth)?;

    Ok(SmtpConfig {
        url,
        tls: Default::default(),
        starttls,
        sasl,
    })
}

fn build_sasl_imap(login: &str, auth: ImapAuth) -> Result<SaslConfig> {
    let ImapAuth::Password(secret) = auth;
    let passwd = match secret {
        ImapSecret::Raw(s) => Secret::Raw(s),
        ImapSecret::Command(cmd) => Secret::Command(parse_cmd(&cmd)?),
    };

    Ok(plain_sasl(login, passwd))
}

fn build_sasl_smtp(login: &str, auth: SmtpAuth) -> Result<SaslConfig> {
    let SmtpAuth::Password(secret) = auth;
    let passwd = match secret {
        SmtpSecret::Raw(s) => Secret::Raw(s),
        SmtpSecret::Command(cmd) => Secret::Command(parse_cmd(&cmd)?),
    };

    Ok(plain_sasl(login, passwd))
}

fn plain_sasl(login: &str, passwd: Secret) -> SaslConfig {
    SaslConfig {
        mechanism: Some(SaslMechanismConfig::Plain),
        login: None,
        plain: Some(SaslPlainConfig {
            authzid: None,
            authcid: login.to_owned(),
            passwd,
        }),
        anonymous: None,
    }
}

fn parse_cmd(cmd: &str) -> Result<Command> {
    if cmd.trim().is_empty() {
        bail!("Empty shell command for secret");
    }
    Ok(Command::new(cmd))
}
