use std::path::Path;

use aws_sdk_route53::types::RrType;
use serde::Deserialize;

use crate::error::Error;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub global: GlobalConfig,
    #[serde(default)]
    pub record: Vec<RecordConfig>,
}

#[derive(Debug, Deserialize)]
pub struct GlobalConfig {
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    #[serde(default = "default_web_url")]
    pub web_url: String,
    #[serde(default = "default_web_timeout")]
    pub web_timeout_secs: u64,
    /// AWS credentials. If absent, the SDK's default credential chain is used.
    pub aws_access_key_id: Option<String>,
    pub aws_secret_access_key: Option<String>,
    /// Optional session token (required when using temporary credentials / STS).
    pub aws_session_token: Option<String>,
    pub aws_region: Option<String>,
}

fn default_interval() -> u64 {
    300
}

fn default_web_url() -> String {
    "https://ifconfig.me".to_string()
}

fn default_web_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecordConfig {
    pub hosted_zone_id: String,
    pub name: String,
    pub interface: String,
    #[serde(default)]
    pub mode: ResolveMode,
    pub record_type: RecordType,
    #[serde(default = "default_ttl")]
    pub ttl: i64,
    /// Overrides the global web_url for this record.
    pub web_url: Option<String>,
}

fn default_ttl() -> i64 {
    300
}

impl RecordConfig {
    pub fn effective_web_url<'a>(&'a self, global: &'a GlobalConfig) -> &'a str {
        self.web_url.as_deref().unwrap_or(&global.web_url)
    }

    pub fn rr_type(&self) -> RrType {
        match self.record_type {
            RecordType::A => RrType::A,
            RecordType::Aaaa => RrType::Aaaa,
        }
    }

}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResolveMode {
    #[default]
    Direct,
    Web,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub enum RecordType {
    A,
    #[serde(rename = "AAAA")]
    Aaaa,
}

pub fn load(path: &Path) -> Result<Config, Error> {
    let contents = std::fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("failed to read {}: {e}", path.display())))?;
    let config: Config = toml::from_str(&contents)?;

    if config.record.is_empty() {
        return Err(Error::Config("no [[record]] entries defined".to_string()));
    }
    for r in &config.record {
        if r.name.is_empty() {
            return Err(Error::Config("record name must not be empty".to_string()));
        }
        if r.hosted_zone_id.is_empty() {
            return Err(Error::Config(
                "record hosted_zone_id must not be empty".to_string(),
            ));
        }
        if r.interface.is_empty() {
            return Err(Error::Config(
                "record interface must not be empty".to_string(),
            ));
        }
        if r.ttl <= 0 {
            return Err(Error::Config(format!(
                "record '{}' has invalid ttl {}",
                r.name, r.ttl
            )));
        }
    }

    Ok(config)
}
