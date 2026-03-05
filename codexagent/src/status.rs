use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset, Weekday};

use crate::logging;

pub(crate) fn current_usage_text() -> String {
    match collect_usage() {
        Ok(status) => format_status(status),
        Err(error) => {
            logging::error(format!("failed to read usage: {}", error));
            format!("Failed to read usage: {}", error)
        }
    }
}

#[derive(Clone, Copy)]
struct RateLimit {
    used_percent: f64,
    window_minutes: u64,
    resets_at: Option<OffsetDateTime>,
}

impl RateLimit {
    fn from_raw(value: RawRateLimit) -> Option<Self> {
        Some(Self {
            used_percent: value.used_percent?,
            window_minutes: value.window_minutes?,
            resets_at: value
                .resets_at
                .and_then(|timestamp| OffsetDateTime::from_unix_timestamp(timestamp).ok()),
        })
    }
}

#[derive(Default)]
struct UsageStatus {
    primary: Option<RateLimit>,
    secondary: Option<RateLimit>,
}

struct RateLimitStatus {
    timestamp: OffsetDateTime,
    limits: UsageStatus,
}

#[derive(Deserialize)]
struct SessionEvent<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<&'a str>,
    #[serde(borrow)]
    timestamp: Option<&'a str>,
    #[serde(borrow)]
    payload: Option<SessionPayload<'a>>,
}

#[derive(Deserialize)]
struct SessionPayload<'a> {
    #[serde(rename = "type", borrow)]
    kind: Option<&'a str>,
    #[serde(borrow)]
    timestamp: Option<&'a str>,
    rate_limits: Option<RawUsageStatus>,
}

#[derive(Clone, Copy, Deserialize)]
struct RawRateLimit {
    used_percent: Option<f64>,
    window_minutes: Option<u64>,
    resets_at: Option<i64>,
}

#[derive(Deserialize)]
struct RawUsageStatus {
    primary: Option<RawRateLimit>,
    secondary: Option<RawRateLimit>,
}

fn collect_usage() -> io::Result<UsageStatus> {
    let Some(sessions_dir) = sessions_dir() else {
        return Ok(UsageStatus::default());
    };
    if !sessions_dir.exists() {
        return Ok(UsageStatus::default());
    }

    let mut latest = None;
    let mut dirs = vec![sessions_dir];

    while let Some(dir) = dirs.pop() {
        let entries = logging::log_result(fs::read_dir(&dir), |error| {
            format!(
                "failed to read sessions directory {}: {}",
                dir.display(),
                error
            )
        })?;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    logging::error(format!(
                        "failed to read directory entry in {}: {}",
                        dir.display(),
                        error
                    ));
                    continue;
                }
            };
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    logging::error(format!(
                        "failed to read file type for {}: {}",
                        path.display(),
                        error
                    ));
                    continue;
                }
            };
            if file_type.is_dir() {
                dirs.push(path);
                continue;
            }
            if !path.extension().is_some_and(|ext| ext == "jsonl") {
                continue;
            }
            let Some(status) = (match read_session_usage(&path) {
                Ok(status) => status,
                Err(error) => {
                    logging::error(format!(
                        "failed to read session usage from {}: {}",
                        path.display(),
                        error
                    ));
                    continue;
                }
            }) else {
                continue;
            };
            if latest
                .as_ref()
                .is_none_or(|current: &RateLimitStatus| status.timestamp > current.timestamp)
            {
                latest = Some(status);
            }
        }
    }

    Ok(latest.map(|status| status.limits).unwrap_or_default())
}

fn sessions_dir() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|path| path.join(".codex").join("sessions"))
}

fn read_session_usage(path: &Path) -> io::Result<Option<RateLimitStatus>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut latest = None;
    let mut line = String::new();
    let mut reader = reader;

    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<SessionEvent<'_>>(trimmed) else {
            continue;
        };
        let Some(status) = session_rate_limits(&value) else {
            continue;
        };
        if latest
            .as_ref()
            .is_none_or(|current: &RateLimitStatus| status.timestamp > current.timestamp)
        {
            latest = Some(status);
        }
    }

    Ok(latest)
}

fn session_timestamp(value: &SessionEvent<'_>) -> Option<OffsetDateTime> {
    let timestamp = value
        .timestamp
        .or_else(|| value.payload.as_ref().and_then(|payload| payload.timestamp))?;
    OffsetDateTime::parse(timestamp, &Rfc3339).ok()
}

fn session_rate_limits(value: &SessionEvent<'_>) -> Option<RateLimitStatus> {
    if value.kind? != "event_msg" {
        return None;
    }
    let payload = value.payload.as_ref()?;
    if payload.kind? != "token_count" {
        return None;
    }
    let rate_limits = payload.rate_limits.as_ref()?;
    let primary = rate_limits.primary.and_then(RateLimit::from_raw);
    let secondary = rate_limits.secondary.and_then(RateLimit::from_raw);
    if primary.is_none() && secondary.is_none() {
        return None;
    }
    Some(RateLimitStatus {
        timestamp: session_timestamp(value)?,
        limits: UsageStatus { primary, secondary },
    })
}

fn format_status(status: UsageStatus) -> String {
    let now = OffsetDateTime::now_utc();
    let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    format_status_at(status, now, local_offset)
}

fn format_status_at(status: UsageStatus, now: OffsetDateTime, local_offset: UtcOffset) -> String {
    if status.primary.is_none() && status.secondary.is_none() {
        return "No local rate-limit status found.".to_owned();
    }

    let mut lines = String::new();
    if let Some(primary) = status.primary {
        lines.push_str(&format_limit("Daily:", primary, now, local_offset));
    }
    if let Some(secondary) = status.secondary {
        if !lines.is_empty() {
            lines.push('\n');
        }
        lines.push_str(&format_limit("Weekly:", secondary, now, local_offset));
    }
    lines
}

fn format_limit(
    label: &str,
    limit: RateLimit,
    now: OffsetDateTime,
    local_offset: UtcOffset,
) -> String {
    format!(
        "`{label:<7} {} remaining ({})`",
        format_percent((100.0 - limit.used_percent).clamp(0.0, 100.0)),
        format_reset(limit, now, local_offset),
    )
}

fn format_reset(limit: RateLimit, now: OffsetDateTime, local_offset: UtcOffset) -> String {
    if let Some(resets_at) = limit.resets_at {
        let remaining_seconds = (resets_at - now).whole_seconds().max(0);
        let resets_local = resets_at.to_offset(local_offset).date();
        let now_local = now.to_offset(local_offset).date();
        return if remaining_seconds == 0 {
            "Resets now".to_owned()
        } else if resets_local == now_local {
            "Resets Today".to_owned()
        } else {
            format!(
                "Resets on {} at {}",
                format_local_day(resets_at, local_offset),
                format_local_time(resets_at, local_offset)
            )
        };
    }
    format!("Window {}", format_duration_minutes(limit.window_minutes))
}

fn format_local_day(datetime: OffsetDateTime, local_offset: UtcOffset) -> &'static str {
    match datetime.to_offset(local_offset).weekday() {
        Weekday::Monday => "Monday",
        Weekday::Tuesday => "Tuesday",
        Weekday::Wednesday => "Wednesday",
        Weekday::Thursday => "Thursday",
        Weekday::Friday => "Friday",
        Weekday::Saturday => "Saturday",
        Weekday::Sunday => "Sunday",
    }
}

fn format_local_time(datetime: OffsetDateTime, local_offset: UtcOffset) -> String {
    let local = datetime.to_offset(local_offset);
    let hour = local.hour();
    let display_hour = match hour % 12 {
        0 => 12,
        value => value,
    };
    let suffix = if hour < 12 { "AM" } else { "PM" };
    format!("{}:{:02} {}", display_hour, local.minute(), suffix)
}

fn format_duration_minutes(minutes: u64) -> String {
    let days = minutes / (24 * 60);
    let hours = (minutes % (24 * 60)) / 60;
    let mins = minutes % 60;
    if days > 0 && hours == 0 && mins == 0 {
        return format!("{}d", days);
    }
    if days == 0 && hours > 0 && mins == 0 {
        return format!("{}h", hours);
    }
    if days == 0 && hours == 0 {
        return format!("{}m", mins);
    }
    if days == 0 {
        return format!("{}h {}m", hours, mins);
    }
    if mins == 0 {
        return format!("{}d {}h", days, hours);
    }
    format!("{}d {}h {}m", days, hours, mins)
}

fn format_percent(value: f64) -> String {
    if value.fract() == 0.0 {
        return format!("{:.0}%", value);
    }
    format!("{:.1}%", value)
}
