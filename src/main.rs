mod config;
mod dns;
mod error;
mod resolve;

use std::path::PathBuf;
use std::time::Duration;

use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_route53::Client;
use backon::{ExponentialBuilder, Retryable};
use clap::Parser;
use tracing::{debug, error, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use config::{Config, GlobalConfig, RecordConfig};
use error::Error;

#[derive(Parser)]
#[command(name = "iface-ddns", about = "Dynamic DNS updater via Amazon Route53")]
struct Cli {
    /// Path to the TOML config file
    #[arg(short, long, default_value = "/etc/iface-ddns/config.toml")]
    config: PathBuf,

    /// Run one update cycle then exit (instead of running as a daemon)
    #[arg(long)]
    once: bool,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    // Use journald when running under systemd (it sets JOURNAL_STREAM), otherwise stderr.
    let under_systemd = std::env::var_os("JOURNAL_STREAM").is_some();
    if under_systemd {
        match tracing_journald::layer() {
            Ok(journald) => {
                tracing_subscriber::registry()
                    .with(env_filter)
                    .with(journald)
                    .init();
            }
            Err(e) => {
                // journald socket unavailable despite JOURNAL_STREAM being set; fall back.
                tracing_subscriber::fmt().with_env_filter(env_filter).init();
                tracing::warn!("journald unavailable, using stderr: {e}");
            }
        }
    } else {
        tracing_subscriber::fmt().with_env_filter(env_filter).init();
    }

    let config = match config::load(&cli.config) {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, path = %cli.config.display(), "failed to load config");
            std::process::exit(1);
        }
    };

    // When credentials are provided in the config file, use from_env() which skips
    // ~/.aws file loading entirely. Otherwise use defaults() for the full credential chain.
    let aws_config = if let (Some(key_id), Some(secret)) = (
        config.global.aws_access_key_id.as_deref(),
        config.global.aws_secret_access_key.as_deref(),
    ) {
        let creds = Credentials::new(
            key_id,
            secret,
            config.global.aws_session_token.clone(),
            None,
            "iface-ddns-config",
        );
        let mut builder = aws_config::from_env().credentials_provider(creds);
        if let Some(region) = config.global.aws_region.as_deref() {
            builder = builder.region(aws_config::Region::new(region.to_string()));
        }
        builder.load().await
    } else {
        let mut builder = aws_config::defaults(BehaviorVersion::latest());
        if let Some(region) = config.global.aws_region.as_deref() {
            builder = builder.region(aws_config::Region::new(region.to_string()));
        }
        builder.load().await
    };
    let client = Client::new(&aws_config);

    info!(
        records = config.record.len(),
        interval_secs = config.global.interval_secs,
        "iface-ddns starting"
    );

    loop {
        run_cycle(&client, &config).await;

        if cli.once {
            break;
        }

        tokio::time::sleep(Duration::from_secs(config.global.interval_secs)).await;
    }
}

/// Run one update cycle over all configured records.
/// Errors are logged per-record; a failure on one record does not block others.
async fn run_cycle(client: &Client, config: &Config) {
    for record in &config.record {
        if let Err(e) = process_record(client, record, &config.global).await {
            error!(
                record = %record.name,
                zone = %record.hosted_zone_id,
                error = %e,
                "failed to process record"
            );
        }
    }
}

/// Resolve the current IP, compare with Route53, and upsert if different.
async fn process_record(
    client: &Client,
    record: &RecordConfig,
    global: &GlobalConfig,
) -> Result<(), Error> {
    let ip = resolve::resolve_ip(record, global).await?;
    let ip_str = ip.to_string();
    let rr_type = record.rr_type();

    let current = dns::get_current_record(
        client,
        &record.hosted_zone_id,
        &record.name,
        rr_type.clone(),
    )
    .await?;

    if current.as_deref() == Some(ip_str.as_str()) {
        debug!(
            record = %record.name,
            ip = %ip_str,
            "no change, skipping update"
        );
        return Ok(());
    }

    if let Some(ref old) = current {
        info!(
            record = %record.name,
            old_ip = %old,
            new_ip = %ip_str,
            "IP changed, updating"
        );
    } else {
        info!(
            record = %record.name,
            ip = %ip_str,
            "record does not exist, creating"
        );
    }

    let zone_id = record.hosted_zone_id.clone();
    let name = record.name.clone();
    let ttl = record.ttl;
    let rr = rr_type.clone();

    (|| async {
        dns::upsert_record(client, &zone_id, &name, rr.clone(), ttl, &ip_str).await
    })
    .retry(
        ExponentialBuilder::default()
            .with_min_delay(Duration::from_secs(1))
            .with_max_delay(Duration::from_secs(60))
            .with_max_times(5),
    )
    .when(|e: &Error| e.is_retryable())
    .notify(|err, dur| {
        warn!(
            record = %name,
            error = %err,
            delay = ?dur,
            "upsert failed, retrying"
        );
    })
    .await?;

    info!(record = %record.name, ip = %ip_str, "record updated successfully");
    Ok(())
}
