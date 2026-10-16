//! Workspace wall-clock time. Models and people read local ISO times with an
//! explicit offset; storage keeps Unix milliseconds.
use anyhow::{Context, Result, bail};
use jiff::{Timestamp, Zoned, civil::DateTime, tz::TimeZone};
use serde::Deserialize;
use serde_json::Value;
use std::path::Path;

/// A time a tool argument names: a local time string, or Unix milliseconds as
/// earlier versions required.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Moment {
    Unix(i64),
    Local(String),
}

impl Moment {
    pub(crate) fn resolve(&self, zone: &TimeZone) -> Result<i64> {
        match self {
            Self::Unix(ms) => Ok(*ms),
            Self::Local(text) => parse(text, zone),
        }
    }
}

/// Read an optional time argument in the workspace time zone.
pub(crate) fn moment(home: &Path, value: Option<&Value>) -> Result<Option<i64>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let moment: Moment = serde_json::from_value(value.clone())
                .context("time must be a local time such as 2026-11-01T09:00")?;
            Ok(Some(
                moment.resolve(&zone(&agt_config::RuntimeConfig::load(home)?))?,
            ))
        }
    }
}

/// `scheduler.timezone` when set, otherwise the host's zone (UTC if unknown).
pub(crate) fn zone(config: &agt_config::RuntimeConfig) -> TimeZone {
    config
        .scheduler
        .timezone
        .as_deref()
        .and_then(|name| TimeZone::get(name).ok())
        .unwrap_or_else(TimeZone::system)
}

pub(crate) fn zone_name(zone: &TimeZone) -> &str {
    zone.iana_name().unwrap_or("UTC")
}

pub(crate) fn named_zone(name: &str) -> Result<TimeZone> {
    TimeZone::get(name).with_context(|| format!("unknown IANA time zone {name:?}"))
}

pub(crate) fn zoned(ms: i64, zone: &TimeZone) -> Zoned {
    Timestamp::from_millisecond(ms)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_zoned(zone.clone())
}

/// `2026-10-16T15:37:00+08:00`
pub(crate) fn local(ms: i64, zone: &TimeZone) -> String {
    zoned(ms, zone).strftime("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

/// Accept a local wall time (`2026-11-01T09:00`), one with an offset, or one with
/// a bracketed zone. A local time that falls in a DST gap moves forward.
pub(crate) fn parse(text: &str, zone: &TimeZone) -> Result<i64> {
    let text = text.trim();
    if let Ok(zoned) = text.parse::<Zoned>() {
        return Ok(zoned.timestamp().as_millisecond());
    }
    if let Ok(timestamp) = text.parse::<Timestamp>() {
        return Ok(timestamp.as_millisecond());
    }
    match text.parse::<DateTime>() {
        Ok(civil) => Ok(civil.to_zoned(zone.clone())?.timestamp().as_millisecond()),
        Err(_) => bail!("time must look like 2026-11-01T09:00 (local) or include an offset"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_times_carry_the_offset_of_their_zone() {
        let shanghai = named_zone("Asia/Shanghai").unwrap();
        let ms = parse("2026-11-01T09:00", &shanghai).unwrap();
        assert_eq!(local(ms, &shanghai), "2026-11-01T09:00:00+08:00");
        assert_eq!(parse("2026-11-01T01:00:00Z", &shanghai).unwrap(), ms);
        assert_eq!(
            parse("2026-11-01T09:00[Asia/Shanghai]", &TimeZone::UTC).unwrap(),
            ms
        );
        assert!(parse("next monday", &shanghai).is_err());
        assert!(named_zone("Mars/Base").is_err());
    }
}
