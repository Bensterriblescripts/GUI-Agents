use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::logging;

pub(crate) const APP_NAME: &str = "codexagent";
pub(crate) const APP_DISPLAY_NAME: &str = "Codex Agent";
pub(crate) const APP_USER_MODEL_ID: &str = "Codex.Agent";
pub(crate) const DEFAULT_MODEL: &str = "gpt-5.3-codex";
pub(crate) const DEFAULT_NOTIFICATIONS_ENABLED: bool = true;
pub(crate) const WINDOW_PADDING: f32 = 36.0;
pub(crate) const WINDOW_BOTTOM_PADDING: f32 = 44.0;
pub(crate) const LINE_HEIGHT: f32 = 20.0;
pub(crate) const TEXT_FONT_SIZE: f32 = 14.0;
pub(crate) const AUTO_EXPAND_VISIBLE_ROWS: usize = 120;
pub(crate) const MAX_VISIBLE_ROWS: usize = 160;
pub(crate) const DEFAULT_WINDOW_WIDTH: f32 = 864.0;
pub(crate) const DEFAULT_WINDOW_HEIGHT: f32 =
    58.0 + LINE_HEIGHT + WINDOW_PADDING + WINDOW_BOTTOM_PADDING;
pub(crate) const MIN_WINDOW_WIDTH: f32 = 504.0;
pub(crate) const MIN_WINDOW_HEIGHT: f32 = 88.0;
pub(crate) const MAX_WINDOW_HEIGHT: f32 = 3323.0;
pub(crate) const CARD_INNER_PADDING_X: f32 = 36.0;
pub(crate) const CANCEL_BUTTON_WIDTH: f32 = 84.0;
pub(crate) const CANCEL_BUTTON_HEIGHT: f32 = 24.0;
pub(crate) const CANCELLED_BOTTOM_PADDING: f32 = 6.0;
pub(crate) const TEXT_EDIT_MARGIN_X: f32 = 8.0;
pub(crate) const MIN_TEXT_WRAP_WIDTH: f32 = 24.0;
pub(crate) const RESIZE_HANDLE_SIZE: f32 = 14.0;
pub(crate) const HIDDEN_MARKDOWN_FONT_SIZE: f32 = 0.5;
pub(crate) const CODEX_CONFIG_CONTENTS: &[u8] = b"approval_policy = \"never\"\nnetwork_access = \"enabled\"\nmodel = \"gpt-5.3-codex\"\nmodel_reasoning_effort = \"high\"\nsandbox_mode = \"danger-full-access\"";
pub(crate) const CODEX_AGENTS_CONTENTS: &[u8] = b"Windows 11\nBe concise. Guess if ambiguous; don\xe2\x80\x99t ask.\nAvoid frameworks unless already present. Avoid comments.\nNever commit or push.\nDon\xe2\x80\x99t read git history (git log/show/etc) unless explicitly asked.\n";
pub(crate) const PROMPT_SCROLL_ID: &str = "prompt-scroll";
pub(crate) const PROMPT_HISTORY_PATH: &str = r"C:\Local\Config\CodexAgent.history";
pub(crate) const MAX_PROMPT_HISTORY: usize = 100;
pub(crate) const PENDING_ANIMATION_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(400);
pub(crate) const CANCELLED_TEXT: &str = "cancelled";

const NOTIFICATION_SETTING_KEY: &str = "notification";

#[derive(Clone, Default)]
pub(crate) struct PromptHistory {
    pub(crate) prompts: Vec<String>,
}

#[allow(dead_code)]
pub(crate) fn read_config() -> io::Result<HashMap<String, String>> {
    let path = default_config_path();
    logging::log_result(read_config_path(&path), |error| {
        format!("failed to read config {}: {}", path.display(), error)
    })
}

#[allow(dead_code)]
pub(crate) fn write() -> io::Result<()> {
    let settings = read_config()?;
    let path = default_config_path();
    logging::log_result(overwrite_config(&path, &settings), |error| {
        format!("failed to write config {}: {}", path.display(), error)
    })
}

pub(crate) fn load_notifications_enabled() -> io::Result<bool> {
    let path = default_config_path();
    let mut settings = read_config_path(&path)?;
    let enabled = settings
        .get(NOTIFICATION_SETTING_KEY)
        .and_then(|value| parse_notification_value(value))
        .unwrap_or(DEFAULT_NOTIFICATIONS_ENABLED);
    let expected = notification_setting_value(enabled);
    if settings.get(NOTIFICATION_SETTING_KEY).map(String::as_str) != Some(expected) {
        settings.insert(NOTIFICATION_SETTING_KEY.to_owned(), expected.to_owned());
        logging::log_result(overwrite_config(&path, &settings), |error| {
            format!(
                "failed to persist notification setting to {}: {}",
                path.display(),
                error
            )
        })?;
    }
    Ok(enabled)
}

pub(crate) fn set_notifications_enabled(enabled: bool) -> io::Result<bool> {
    write_setting(
        NOTIFICATION_SETTING_KEY,
        notification_setting_value(enabled),
    )?;
    Ok(enabled)
}

#[allow(dead_code)]
pub(crate) fn write_setting(label: &str, value: &str) -> io::Result<()> {
    if label.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "label cannot be empty",
        ));
    }
    if value.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "value cannot be empty",
        ));
    }

    let path = default_config_path();
    let mut settings = read_config_path(&path)?;
    settings.insert(label.to_owned(), value.to_owned());
    logging::log_result(overwrite_config(&path, &settings), |error| {
        format!(
            "failed to write setting {} to {}: {}",
            label,
            path.display(),
            error
        )
    })
}

#[allow(dead_code)]
pub(crate) fn write_settings(new_config: &HashMap<String, String>) -> io::Result<()> {
    if new_config.is_empty() {
        return Ok(());
    }

    let path = default_config_path();
    let mut settings = read_config_path(&path)?;
    settings.extend(
        new_config
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    logging::log_result(overwrite_config(&path, &settings), |error| {
        format!("failed to write settings to {}: {}", path.display(), error)
    })
}

pub(crate) fn load_prompt_history() -> io::Result<PromptHistory> {
    let path = Path::new(PROMPT_HISTORY_PATH);
    let settings = logging::log_result(read_config_path(path), |error| {
        format!(
            "failed to read prompt history {}: {}",
            path.display(),
            error
        )
    })?;
    logging::log_result(prompt_history_from_settings(&settings), |error| {
        format!(
            "failed to parse prompt history {}: {}",
            path.display(),
            error
        )
    })
}

pub(crate) fn save_prompt_history(history: &PromptHistory) -> io::Result<()> {
    save_prompt_history_prompts(&history.prompts)
}

pub(crate) fn save_prompt_history_prompts(prompts: &[String]) -> io::Result<()> {
    let path = Path::new(PROMPT_HISTORY_PATH);
    logging::log_result(
        overwrite_config(
            Path::new(PROMPT_HISTORY_PATH),
            &prompt_history_to_settings_slice(prompts),
        ),
        |error| {
            format!(
                "failed to save prompt history {}: {}",
                path.display(),
                error
            )
        },
    )
}

#[allow(dead_code)]
fn default_config_path() -> PathBuf {
    PathBuf::from(r"C:\Local\Config").join("CodexAgent.ini")
}

fn notification_setting_value(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

fn parse_notification_value(value: &str) -> Option<bool> {
    if value.eq_ignore_ascii_case("on")
        || value.eq_ignore_ascii_case("true")
        || value == "1"
        || value.eq_ignore_ascii_case("yes")
    {
        return Some(true);
    }
    if value.eq_ignore_ascii_case("off")
        || value.eq_ignore_ascii_case("false")
        || value == "0"
        || value.eq_ignore_ascii_case("no")
    {
        return Some(false);
    }
    None
}

fn read_config_path(path: &Path) -> io::Result<HashMap<String, String>> {
    let raw_config = get_config(path)?;
    Ok(parse_config(&raw_config))
}

fn parse_config(raw_config: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();

    for raw_line in raw_config.split(|byte| *byte == b'\n') {
        let line = match std::str::from_utf8(raw_line) {
            Ok(line) => line.trim(),
            Err(_) => continue,
        };
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(index) = line.find('=') else {
            continue;
        };
        let key = line[..index].trim();
        let mut value = line[index + 1..].trim();
        if value.len() >= 2 {
            let quoted = (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''));
            if quoted {
                value = &value[1..value.len() - 1];
            }
        }
        out.insert(key.to_owned(), value.to_owned());
    }

    out
}

fn get_config(path: &Path) -> io::Result<Vec<u8>> {
    ensure_path(path)?;
    fs::read(path)
}

fn overwrite_config(path: &Path, current: &HashMap<String, String>) -> io::Result<()> {
    ensure_path(path)?;

    let mut keys: Vec<_> = current.keys().collect();
    keys.sort_unstable();

    let total_len = keys.iter().fold(0usize, |len, key| {
        len + key.len() + 1 + current.get(*key).map_or(0, String::len) + 1
    });
    let mut buffer = String::with_capacity(total_len);
    for key in keys {
        if let Some(value) = current.get(key) {
            buffer.push_str(key);
            buffer.push('=');
            buffer.push_str(value);
            buffer.push('\n');
        }
    }

    fs::write(path, buffer)
}

fn ensure_path(path: &Path) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no parent: {}", path.display()),
        ));
    };
    fs::create_dir_all(parent)?;
    if !path.exists() {
        let _ = fs::File::create(path)?;
    }
    Ok(())
}

fn prompt_history_to_settings_slice(prompts: &[String]) -> HashMap<String, String> {
    let mut settings = HashMap::with_capacity((!prompts.is_empty()) as usize);

    if !prompts.is_empty() {
        let mut prompts_hex = String::with_capacity(prompt_history_hex_len(prompts));
        for (index, prompt) in prompts.iter().enumerate() {
            if index != 0 {
                prompts_hex.push(',');
            }
            append_hex(&mut prompts_hex, prompt.as_bytes());
        }
        settings.insert("prompts_hex".to_owned(), prompts_hex);
    }

    settings
}

fn prompt_history_from_settings(settings: &HashMap<String, String>) -> io::Result<PromptHistory> {
    let prompts = match settings.get("prompts_hex") {
        Some(value) if !value.is_empty() => parse_prompt_history_entries(value)?,
        _ => legacy_prompt_history_entries(settings)?,
    };
    let mut prompts = prompts;
    trim_prompt_history(&mut prompts);

    Ok(PromptHistory { prompts })
}

pub(crate) fn trim_prompt_history(prompts: &mut Vec<String>) {
    if prompts.len() > MAX_PROMPT_HISTORY {
        let overflow = prompts.len() - MAX_PROMPT_HISTORY;
        prompts.drain(0..overflow);
    }
}

fn parse_prompt_ranges(value: &str) -> io::Result<Vec<(usize, usize)>> {
    let mut ranges = Vec::new();

    for item in value.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let Some((start, end)) = item.split_once(':') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid prompt range",
            ));
        };
        let start = start.trim().parse::<usize>().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid prompt range start: {}", error),
            )
        })?;
        let end = end.trim().parse::<usize>().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid prompt range end: {}", error),
            )
        })?;
        ranges.push((start, end));
    }

    Ok(ranges)
}

fn parse_prompt_history_entries(value: &str) -> io::Result<Vec<String>> {
    let mut prompts = Vec::new();

    for item in value.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        prompts.push(decode_hex_to_string(item)?);
    }

    Ok(prompts)
}

fn prompt_history_entries_from_output(
    output: &str,
    prompt_ranges: &[(usize, usize)],
) -> io::Result<Vec<String>> {
    let mut prompts = Vec::with_capacity(prompt_ranges.len());

    for &(start, end) in prompt_ranges {
        let Some(prompt) = output.get(start..end) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "prompt range is out of bounds",
            ));
        };
        if !prompt.is_empty() {
            prompts.push(prompt.to_owned());
        }
    }

    Ok(prompts)
}

fn legacy_prompt_history_entries(settings: &HashMap<String, String>) -> io::Result<Vec<String>> {
    let output = match settings.get("output_hex") {
        Some(value) if !value.is_empty() => decode_hex_to_string(value)?,
        _ => return Ok(Vec::new()),
    };
    let prompt_ranges = match settings.get("prompt_ranges") {
        Some(value) if !value.is_empty() => parse_prompt_ranges(value)?,
        _ => return Ok(Vec::new()),
    };

    for &(start, end) in &prompt_ranges {
        if start > end || end > output.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "prompt range is out of bounds",
            ));
        }
        if !output.is_char_boundary(start) || !output.is_char_boundary(end) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "prompt range is not aligned to UTF-8 boundaries",
            ));
        }
    }

    prompt_history_entries_from_output(&output, &prompt_ranges)
}

fn append_hex(out: &mut String, bytes: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.reserve(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
}

fn prompt_history_hex_len(prompts: &[String]) -> usize {
    prompts.iter().map(|prompt| prompt.len() * 2).sum::<usize>() + prompts.len().saturating_sub(1)
}

fn decode_hex_to_string(value: &str) -> io::Result<String> {
    if value.len() % 2 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "hex value has odd length",
        ));
    }

    let mut bytes = Vec::with_capacity(value.len() / 2);
    let mut index = 0usize;
    while index < value.len() {
        let byte = u8::from_str_radix(&value[index..index + 2], 16).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid hex value: {}", error),
            )
        })?;
        bytes.push(byte);
        index += 2;
    }

    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}
