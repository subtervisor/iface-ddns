use std::net::IpAddr;
use std::time::Duration;

use tracing::debug;

use crate::config::{GlobalConfig, RecordConfig, RecordType, ResolveMode};
use crate::error::Error;

/// Resolve the current public IP for a record, using the configured mode.
pub async fn resolve_ip(record: &RecordConfig, global: &GlobalConfig) -> Result<IpAddr, Error> {
    match record.mode {
        ResolveMode::Direct => resolve_direct(&record.interface, &record.record_type),
        ResolveMode::Web => {
            let url = record.effective_web_url(global);
            resolve_web(&record.interface, url, &record.record_type, global.web_timeout_secs).await
        }
    }
}

/// Read an IP address directly from the interface.
fn resolve_direct(interface: &str, record_type: &RecordType) -> Result<IpAddr, Error> {
    let addrs =
        if_addrs::get_if_addrs().map_err(|e| Error::Config(format!("get_if_addrs: {e}")))?;

    let mut found_iface = false;
    for iface in &addrs {
        if iface.name != interface {
            continue;
        }
        found_iface = true;
        let ip = iface.addr.ip();
        match record_type {
            RecordType::A if ip.is_ipv4() => return Ok(ip),
            RecordType::Aaaa if ip.is_ipv6() && !is_link_local_v6(ip) => return Ok(ip),
            _ => {}
        }
    }

    if !found_iface {
        return Err(Error::InterfaceNotFound {
            interface: interface.to_string(),
        });
    }

    Err(Error::NoAddress {
        interface: interface.to_string(),
        addr_type: match record_type {
            RecordType::A => "IPv4",
            RecordType::Aaaa => "IPv6",
        },
    })
}

/// Send an HTTP request through the given interface to discover the public IP.
///
/// We find a local address of the correct family on the interface and bind
/// the HTTP client to it, ensuring the request routes through that interface.
async fn resolve_web(
    interface: &str,
    web_url: &str,
    record_type: &RecordType,
    timeout_secs: u64,
) -> Result<IpAddr, Error> {
    let local_addr = resolve_direct(interface, record_type)?;

    debug!(interface=%interface, web_url=%web_url, record_type=%record_type, timeout=%timeout_secs, "Fetching external IP");

    let client = reqwest::Client::builder()
        .local_address(local_addr)
        .user_agent("curl/8.0")
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(Error::WebResolve)?;

    let response = client.get(web_url).send().await?;
    let body = response.text().await?;
    let trimmed = body.trim();

    let ip: IpAddr = trimmed
        .parse()
        .map_err(|_| Error::InvalidWebIp(trimmed.to_string()))?;

    // Validate the returned address family matches what we expect.
    match record_type {
        RecordType::A if !ip.is_ipv4() => {
            return Err(Error::InvalidWebIp(format!(
                "expected IPv4, got '{trimmed}'"
            )));
        }
        RecordType::Aaaa if !ip.is_ipv6() => {
            return Err(Error::InvalidWebIp(format!(
                "expected IPv6, got '{trimmed}'"
            )));
        }
        _ => {}
    }

    Ok(ip)
}

fn is_link_local_v6(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            (segments[0] & 0xffc0) == 0xfe80
        }
        _ => false,
    }
}
