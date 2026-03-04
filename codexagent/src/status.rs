use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

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
}

impl RateLimit {
    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            used_percent: value.get("used_percent")?.as_f64()?,
            window_minutes: value.get("window_minutes")?.as_u64()?,
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

    for line in reader.lines() {
        let line = line?;
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
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

fn session_timestamp(value: &Value) -> Option<OffsetDateTime> {
    let timestamp = value.get("timestamp").and_then(Value::as_str).or_else(|| {
        value
            .get("payload")
            .and_then(|payload| payload.get("timestamp"))
            .and_then(Value::as_str)
    })?;
    OffsetDateTime::parse(timestamp, &Rfc3339).ok()
}

fn session_rate_limits(value: &Value) -> Option<RateLimitStatus> {
    if value.get("type")?.as_str()? != "event_msg" {
        return None;
    }
    let payload = value.get("payload")?;
    if payload.get("type")?.as_str()? != "token_count" {
        return None;
    }
    let rate_limits = payload.get("rate_limits")?;
    let primary = rate_limits.get("primary").and_then(RateLimit::from_value);
    let secondary = rate_limits.get("secondary").and_then(RateLimit::from_value);
    if primary.is_none() && secondary.is_none() {
        return None;
    }
    Some(RateLimitStatus {
        timestamp: session_timestamp(value)?,
        limits: UsageStatus { primary, secondary },
    })
}

fn format_status(status: UsageStatus) -> String {
    if status.primary.is_none() && status.secondary.is_none() {
        return "No local rate-limit status found.".to_owned();
    }

    let mut lines = String::new();
    if let Some(primary) = status.primary {
        lines.push_str(&format_limit("Daily", primary));
    }
    if let Some(secondary) = status.secondary {
        if !lines.is_empty() {
            lines.push('\n');
        }
        lines.push_str(&format_limit("Weekly", secondary));
    }
    lines
}

fn format_limit(label: &str, limit: RateLimit) -> String {
    format!(
        "{} (Resets in {}): {}",
        label,
        format_window(limit.window_minutes),
        format_percent((100.0 - limit.used_percent).clamp(0.0, 100.0)),
    )
}

fn format_window(minutes: u64) -> String {
    if minutes % (24 * 60) == 0 {
        return format!("{}d", minutes / (24 * 60));
    }
    if minutes % 60 == 0 {
        return format!("{}h", minutes / 60);
    }
    format!("{}m", minutes)
}

fn format_percent(value: f64) -> String {
    if value.fract() == 0.0 {
        return format!("{:.0}%", value);
    }
    format!("{:.1}%", value)
}

#[cfg(test)]
mod tests {
    use super::{RateLimit, format_percent, format_status, format_window};

    #[test]
    fn formats_window_labels() {
        assert_eq!(format_window(300), "5h");
        assert_eq!(format_window(10080), "7d");
        assert_eq!(format_window(45), "45m");
    }

    #[test]
    fn formats_remaining_percent_without_trailing_decimal() {
        assert_eq!(format_percent(82.0), "82%");
        assert_eq!(format_percent(82.5), "82.5%");
    }

    #[test]
    fn formats_remaining_status() {
        let text = format_status(super::UsageStatus {
            primary: Some(RateLimit {
                used_percent: 18.0,
                window_minutes: 300,
            }),
            secondary: Some(RateLimit {
                used_percent: 6.0,
                window_minutes: 10080,
            }),
        });
        assert_eq!(
            text,
            "Daily (Resets in 5h): 82%\nWeekly (Resets in 7d): 94%"
        );
    }
}
