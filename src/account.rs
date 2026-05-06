use std::{env::temp_dir, path::PathBuf};

use crate::config::{AccountConfig, Config};
use anyhow::Result;
use comfy_table::{presets, ContentArrangement};
use dirs::download_dir;

const DEFAULT_DATETIME_FMT: &str = "%F %R%:z";

#[derive(Clone, Debug)]
pub struct Account<B: Clone> {
    pub backend: B,
    pub downloads_dir: PathBuf,

    pub table_preset: String,
    pub table_arrangement: ContentArrangement,

    pub datetime_fmt: String,
    pub datetime_local_tz: bool,
}

impl<B: Clone> Account<B> {
    pub fn new(config: Config, account_config: AccountConfig, backend: B) -> Result<Self> {
        Ok(Self {
            backend,

            downloads_dir: account_config
                .downloads_dir
                .as_ref()
                .and_then(|dir| dir.to_str())
                .and_then(|dir| shellexpand::full(dir).ok())
                .map(|dir| PathBuf::from(dir.to_string()))
                .or(config
                    .downloads_dir
                    .as_ref()
                    .and_then(|dir| dir.to_str())
                    .and_then(|dir| shellexpand::full(dir).ok())
                    .map(|dir| PathBuf::from(dir.to_string())))
                .or(download_dir())
                .unwrap_or_else(temp_dir),

            table_preset: config
                .table_preset
                .or(account_config.table_preset)
                .unwrap_or(presets::UTF8_FULL_CONDENSED.to_string()),
            table_arrangement: config
                .table_arrangement
                .or(account_config.table_arrangement)
                .unwrap_or_default()
                .into(),

            datetime_fmt: config
                .envelope
                .list
                .datetime_fmt
                .or(account_config.envelope.list.datetime_fmt)
                .unwrap_or_else(|| DEFAULT_DATETIME_FMT.to_string()),
            datetime_local_tz: config
                .envelope
                .list
                .datetime_local_tz
                .or(account_config.envelope.list.datetime_local_tz)
                .unwrap_or(false),
        })
    }
}
