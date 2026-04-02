use aws_sdk_route53::{
    Client,
    types::{Change, ChangeAction, ChangeBatch, ResourceRecord, ResourceRecordSet, RrType},
};

use crate::error::Error;

/// Query Route53 for the current value of a DNS record.
///
/// Returns `None` if the record does not exist, or `Some(value)` with the
/// first resource record value (e.g. an IP address string) if it does.
pub async fn get_current_record(
    client: &Client,
    zone_id: &str,
    name: &str,
    rr_type: RrType,
) -> Result<Option<String>, Error> {
    let resp = client
        .list_resource_record_sets()
        .hosted_zone_id(zone_id)
        .start_record_name(name)
        .start_record_type(rr_type.clone())
        .max_items(1)
        .send()
        .await
        .map_err(|e| Error::Route53(e.to_string()))?;

    let sets = resp.resource_record_sets();
    if sets.is_empty() {
        return Ok(None);
    }

    let set = &sets[0];

    // Route53 returns names with a trailing dot; normalise both sides.
    let returned_name = normalize_name(set.name());
    let queried_name = normalize_name(name);
    if returned_name != queried_name {
        return Ok(None);
    }

    if set.r#type() != &rr_type {
        return Ok(None);
    }

    let value = set
        .resource_records()
        .first()
        .map(|rr| rr.value().to_string());

    Ok(value)
}

/// Upsert a DNS A/AAAA record to the given IP value.
pub async fn upsert_record(
    client: &Client,
    zone_id: &str,
    name: &str,
    rr_type: RrType,
    ttl: i64,
    value: &str,
) -> Result<(), Error> {
    let resource_record = ResourceRecord::builder()
        .value(value)
        .build()
        .map_err(|e| Error::Route53(e.to_string()))?;

    let rrs = ResourceRecordSet::builder()
        .name(name)
        .r#type(rr_type)
        .ttl(ttl)
        .resource_records(resource_record)
        .build()
        .map_err(|e| Error::Route53(e.to_string()))?;

    let change = Change::builder()
        .action(ChangeAction::Upsert)
        .resource_record_set(rrs)
        .build()
        .map_err(|e| Error::Route53(e.to_string()))?;

    let batch = ChangeBatch::builder()
        .changes(change)
        .build()
        .map_err(|e| Error::Route53(e.to_string()))?;

    client
        .change_resource_record_sets()
        .hosted_zone_id(zone_id)
        .change_batch(batch)
        .send()
        .await
        .map_err(|e| Error::Route53(e.to_string()))?;

    Ok(())
}

fn normalize_name(name: &str) -> &str {
    name.strip_suffix('.').unwrap_or(name)
}
