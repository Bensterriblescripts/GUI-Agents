use std::sync::OnceLock;

use eframe::egui::{
    Color32, FontId,
    text::{LayoutJob, TextFormat},
};
use eframe::epaint::text::Galley;

use crate::config::{
    CANCELLED_BOTTOM_PADDING, CANCELLED_TEXT, HIDDEN_MARKDOWN_FONT_SIZE, LINE_HEIGHT,
    MIN_TEXT_WRAP_WIDTH, TEXT_FONT_SIZE,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum OutputLineKind {
    #[default]
    Normal,
    Error,
    Reasoning,
    Agent,
}

struct MarkdownFormats {
    plain_new: TextFormat,
    plain_old: TextFormat,
    code_new: TextFormat,
    code_old: TextFormat,
    hidden: TextFormat,
    cancelled: TextFormat,
    cancelled_spacer: TextFormat,
    reasoning: TextFormat,
    reasoning_code: TextFormat,
    agent: TextFormat,
    agent_code: TextFormat,
}

pub(super) fn pending_dots(step: u128) -> &'static str {
    match step % 3 {
        0 => ".",
        1 => "..",
        _ => "...",
    }
}

pub(super) fn trim_string_in_place(text: &mut String) -> bool {
    if text.trim().is_empty() {
        return false;
    }
    let start = text.len() - text.trim_start().len();
    let end = text.trim_end().len();
    if end < text.len() {
        text.truncate(end);
    }
    if start > 0 {
        text.drain(..start);
    }
    true
}

pub(super) fn markdown_layout_job(
    text: &str,
    wrap_width: f32,
    prompt_ranges: &[(usize, usize)],
    response_start: usize,
    line_kinds: &[(usize, OutputLineKind)],
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width.max(MIN_TEXT_WRAP_WIDTH);
    let formats = markdown_formats();

    let mut in_code = false;
    let mut byte_offset = 0usize;
    let mut prompt_range_index = 0usize;
    let mut line_kind_index = 0usize;
    for line in text.split_inclusive('\n') {
        while prompt_range_index < prompt_ranges.len()
            && byte_offset >= prompt_ranges[prompt_range_index].1
        {
            prompt_range_index += 1;
        }
        let is_prompt = prompt_ranges
            .get(prompt_range_index)
            .is_some_and(|&(start, end)| byte_offset >= start && byte_offset < end);
        let is_old = !is_prompt && byte_offset < response_start;
        let line_kind =
            if line_kind_index < line_kinds.len() && line_kinds[line_kind_index].0 == byte_offset {
                line_kind_index += 1;
                line_kinds[line_kind_index - 1].1
            } else {
                OutputLineKind::Normal
            };
        let (rest, is_reasoning, is_agent, is_error) = if line.starts_with('\x1D') {
            job.append("\x1D", 0.0, formats.hidden.clone());
            (&line[1..], false, false, true)
        } else if line.starts_with('\x1E') {
            job.append("\x1E", 0.0, formats.hidden.clone());
            (&line[1..], true, false, false)
        } else if line.starts_with('\x1F') {
            job.append("\x1F", 0.0, formats.hidden.clone());
            (&line[1..], false, true, false)
        } else {
            (
                line,
                line_kind == OutputLineKind::Reasoning,
                line_kind == OutputLineKind::Agent,
                line_kind == OutputLineKind::Error,
            )
        };
        let content = rest.strip_suffix('\n').unwrap_or(rest).trim_start();
        if content.starts_with("```") {
            let fence = rest.strip_suffix('\n').unwrap_or(rest);
            job.append(fence, 0.0, formats.hidden.clone());
            in_code = !in_code;
            if !in_code {
                job.append("\n\n", 0.0, formats.plain_new.clone());
            }
            byte_offset += line.len();
            continue;
        }
        if !in_code && rest.strip_suffix('\n').unwrap_or(rest) == CANCELLED_TEXT {
            job.append(rest, 0.0, formats.cancelled.clone());
            if !rest.ends_with('\n') {
                job.append("\n", 0.0, formats.cancelled_spacer.clone());
            }
            byte_offset += line.len();
            continue;
        }
        let format = if is_error {
            &formats.cancelled
        } else if is_reasoning {
            if in_code {
                &formats.reasoning_code
            } else {
                &formats.reasoning
            }
        } else if is_agent {
            if in_code {
                &formats.agent_code
            } else {
                &formats.agent
            }
        } else if in_code {
            if is_old {
                &formats.code_old
            } else {
                &formats.code_new
            }
        } else {
            if is_old {
                &formats.plain_old
            } else {
                &formats.plain_new
            }
        };
        if in_code || is_reasoning || is_error {
            job.append(rest, 0.0, format.clone());
        } else if is_horizontal_rule(content) {
            job.append(rest, 0.0, formats.hidden.clone());
        } else {
            let icf = if is_old {
                &formats.code_old
            } else {
                &formats.code_new
            };
            let hdr = header_prefix_len(content);
            let ws = rest.len() - rest.trim_start().len();
            if hdr > 0 {
                job.append(&rest[..ws + hdr], 0.0, formats.hidden.clone());
                append_markdown_line(&mut job, &rest[ws + hdr..], format, icf, &formats.hidden);
            } else {
                append_markdown_line(&mut job, rest, format, icf, &formats.hidden);
            }
        }
        byte_offset += line.len();
    }

    if text.is_empty() {
        job.append("", 0.0, formats.plain_new.clone());
    }

    job
}

pub(super) fn prepare_output_display(
    text: &str,
    prompt_ranges: &[(usize, usize)],
    response_start: usize,
    clean_text: &mut String,
    clean_prompt_ranges: &mut Vec<(usize, usize)>,
    line_kinds: &mut Vec<(usize, OutputLineKind)>,
) -> usize {
    let mut raw_offset = 0usize;
    let mut clean_offset = 0usize;
    clean_text.clear();
    if clean_text.capacity() < text.len() {
        clean_text.reserve(text.len() - clean_text.capacity());
    }
    clean_prompt_ranges.clear();
    clean_prompt_ranges.resize(prompt_ranges.len(), (0, 0));
    line_kinds.clear();
    let mut next_prompt_index = 0usize;
    let mut next_prompt_is_end = false;
    let mut response_mapped = None;

    for line in text.split_inclusive('\n') {
        let raw_line_start = raw_offset;
        let clean_line_start = clean_offset;
        let (kind, marker_len) = output_line_kind(line, true);
        let raw_content_start = raw_line_start + marker_len;
        let clean_line = &line[marker_len..];
        let raw_line_end = raw_line_start + line.len();
        while let Some((raw_point, point)) = next_output_display_point(
            prompt_ranges,
            response_start,
            next_prompt_index,
            next_prompt_is_end,
            response_mapped.is_none(),
        ) {
            if raw_point > raw_line_end {
                break;
            }
            let mapped = clean_line_start + raw_point.saturating_sub(raw_content_start);
            match point {
                OutputDisplayPoint::PromptStart(index) => {
                    clean_prompt_ranges[index].0 = mapped;
                    next_prompt_is_end = true;
                }
                OutputDisplayPoint::PromptEnd(index) => {
                    clean_prompt_ranges[index].1 = mapped;
                    next_prompt_index += 1;
                    next_prompt_is_end = false;
                }
                OutputDisplayPoint::Response => {
                    response_mapped = Some(mapped);
                }
            }
        }
        if kind != OutputLineKind::Normal {
            line_kinds.push((clean_line_start, kind));
        }
        clean_text.push_str(clean_line);
        raw_offset = raw_line_end;
        clean_offset += clean_line.len();
    }

    while let Some((_, point)) = next_output_display_point(
        prompt_ranges,
        response_start,
        next_prompt_index,
        next_prompt_is_end,
        response_mapped.is_none(),
    ) {
        match point {
            OutputDisplayPoint::PromptStart(index) => {
                clean_prompt_ranges[index].0 = clean_offset;
                next_prompt_is_end = true;
            }
            OutputDisplayPoint::PromptEnd(index) => {
                clean_prompt_ranges[index].1 = clean_offset;
                next_prompt_index += 1;
                next_prompt_is_end = false;
            }
            OutputDisplayPoint::Response => {
                response_mapped = Some(clean_offset);
            }
        }
    }

    response_mapped.unwrap_or(clean_offset)
}

pub(super) fn append_output_display(
    text: &str,
    mut line_start: bool,
    clean_text: &mut String,
    line_kinds: &mut Vec<(usize, OutputLineKind)>,
) {
    if text.is_empty() {
        return;
    }

    let mut clean_offset = clean_text.len();
    for line in text.split_inclusive('\n') {
        let clean_line_start = clean_offset;
        let (kind, marker_len) = output_line_kind(line, line_start);
        let clean_line = &line[marker_len..];
        if kind != OutputLineKind::Normal {
            line_kinds.push((clean_line_start, kind));
        }
        clean_text.push_str(clean_line);
        clean_offset += clean_line.len();
        line_start = line.ends_with('\n');
    }
}

pub(super) fn response_separator_y(
    galley: &Galley,
    response_start_char_index: usize,
) -> Option<f32> {
    if response_start_char_index == 0 {
        return None;
    }
    let rect = galley.pos_from_ccursor(eframe::egui::text::CCursor::new(response_start_char_index));
    Some((rect.top() - LINE_HEIGHT * 0.5).max(0.0))
}

fn output_line_kind(line: &str, line_start: bool) -> (OutputLineKind, usize) {
    if !line_start {
        return (OutputLineKind::Normal, 0);
    }
    match line.as_bytes().first().copied() {
        Some(0x1D) => (OutputLineKind::Error, 1),
        Some(0x1E) => (OutputLineKind::Reasoning, 1),
        Some(0x1F) => (OutputLineKind::Agent, 1),
        _ => (OutputLineKind::Normal, 0),
    }
}

fn markdown_formats() -> &'static MarkdownFormats {
    static FORMATS: OnceLock<MarkdownFormats> = OnceLock::new();
    FORMATS.get_or_init(|| {
        let old_color = Color32::from_rgb(140, 145, 155);
        MarkdownFormats {
            plain_new: TextFormat {
                font_id: FontId::proportional(TEXT_FONT_SIZE),
                color: Color32::WHITE,
                ..Default::default()
            },
            plain_old: TextFormat {
                font_id: FontId::proportional(TEXT_FONT_SIZE),
                color: old_color,
                ..Default::default()
            },
            code_new: TextFormat {
                font_id: FontId::monospace(TEXT_FONT_SIZE),
                color: Color32::from_rgba_unmultiplied(188, 194, 202, 220),
                ..Default::default()
            },
            code_old: TextFormat {
                font_id: FontId::monospace(TEXT_FONT_SIZE),
                color: Color32::from_rgb(130, 140, 150),
                ..Default::default()
            },
            hidden: TextFormat {
                font_id: FontId::monospace(HIDDEN_MARKDOWN_FONT_SIZE),
                color: Color32::TRANSPARENT,
                ..Default::default()
            },
            cancelled: TextFormat {
                font_id: FontId::proportional(TEXT_FONT_SIZE),
                color: Color32::from_rgb(255, 96, 96),
                italics: true,
                ..Default::default()
            },
            cancelled_spacer: TextFormat {
                font_id: FontId::monospace(HIDDEN_MARKDOWN_FONT_SIZE),
                line_height: Some(CANCELLED_BOTTOM_PADDING),
                color: Color32::TRANSPARENT,
                ..Default::default()
            },
            reasoning: TextFormat {
                font_id: FontId::proportional(TEXT_FONT_SIZE),
                color: Color32::from_rgb(130, 135, 145),
                ..Default::default()
            },
            reasoning_code: TextFormat {
                font_id: FontId::monospace(TEXT_FONT_SIZE),
                color: Color32::from_rgb(130, 140, 150),
                ..Default::default()
            },
            agent: TextFormat {
                font_id: FontId::proportional(TEXT_FONT_SIZE),
                color: Color32::WHITE,
                ..Default::default()
            },
            agent_code: TextFormat {
                font_id: FontId::monospace(TEXT_FONT_SIZE),
                color: Color32::from_rgba_unmultiplied(188, 194, 202, 220),
                ..Default::default()
            },
        }
    })
}

#[derive(Clone, Copy)]
enum OutputDisplayPoint {
    PromptStart(usize),
    PromptEnd(usize),
    Response,
}

fn next_output_display_point(
    prompt_ranges: &[(usize, usize)],
    response_start: usize,
    next_prompt_index: usize,
    next_prompt_is_end: bool,
    response_pending: bool,
) -> Option<(usize, OutputDisplayPoint)> {
    let prompt_point = prompt_ranges.get(next_prompt_index).map(|&(start, end)| {
        if next_prompt_is_end {
            (end, OutputDisplayPoint::PromptEnd(next_prompt_index))
        } else {
            (start, OutputDisplayPoint::PromptStart(next_prompt_index))
        }
    });
    match (prompt_point, response_pending) {
        (Some((offset, point)), true) if response_start < offset => {
            Some((response_start, OutputDisplayPoint::Response))
        }
        (Some(prompt_point), _) => Some(prompt_point),
        (None, true) => Some((response_start, OutputDisplayPoint::Response)),
        (None, false) => None,
    }
}

fn is_horizontal_rule(trimmed: &str) -> bool {
    let bytes = trimmed.as_bytes();
    if bytes.len() < 3 {
        return false;
    }
    let c = bytes[0];
    if c != b'-' && c != b'*' && c != b'_' {
        return false;
    }
    let mut count = 0u32;
    for &b in bytes {
        if b == c {
            count += 1;
        } else if b != b' ' {
            return false;
        }
    }
    count >= 3
}

fn header_prefix_len(trimmed: &str) -> usize {
    let bytes = trimmed.as_bytes();
    let mut n = 0;
    while n < bytes.len() && bytes[n] == b'#' {
        n += 1;
    }
    if n == 0 || n > 6 || n >= bytes.len() || bytes[n] != b' ' {
        return 0;
    }
    n + 1
}

fn should_show_link_target(target: &str) -> bool {
    let bytes = target.as_bytes();
    target.starts_with('/')
        || (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
}

fn append_markdown_line(
    job: &mut LayoutJob,
    line: &str,
    format: &TextFormat,
    code_format: &TextFormat,
    hidden: &TextFormat,
) {
    let mut remaining = line;
    while !remaining.is_empty() {
        let bold = remaining.find("**");
        let tick = remaining.find('`');
        let bracket = remaining.find('[');
        let mut at = remaining.len();
        let mut kind = 0u8;
        if let Some(p) = bold {
            if p < at {
                at = p;
                kind = 1;
            }
        }
        if let Some(p) = tick {
            if p < at {
                at = p;
                kind = 2;
            }
        }
        if let Some(p) = bracket {
            if p < at {
                at = p;
                kind = 3;
            }
        }
        if kind == 0 {
            job.append(remaining, 0.0, format.clone());
            break;
        }
        if at > 0 {
            job.append(&remaining[..at], 0.0, format.clone());
        }
        remaining = &remaining[at..];
        match kind {
            1 => {
                job.append("**", 0.0, hidden.clone());
                remaining = &remaining[2..];
                if let Some(end) = remaining.find("**") {
                    job.append(&remaining[..end], 0.0, format.clone());
                    job.append("**", 0.0, hidden.clone());
                    remaining = &remaining[end + 2..];
                } else {
                    job.append("**", 0.0, format.clone());
                    break;
                }
            }
            2 => {
                let inner = &remaining[1..];
                if let Some(end) = inner.find('`') {
                    if end > 0 {
                        job.append("`", 0.0, hidden.clone());
                        job.append(&inner[..end], 0.0, code_format.clone());
                        job.append("`", 0.0, hidden.clone());
                        remaining = &inner[end + 1..];
                    } else {
                        job.append("`", 0.0, format.clone());
                        remaining = inner;
                    }
                } else {
                    job.append("`", 0.0, format.clone());
                    remaining = inner;
                }
            }
            3 => {
                let inner = &remaining[1..];
                let valid = inner.find(']').and_then(|be| {
                    let after = &inner[be + 1..];
                    if after.starts_with('(') {
                        after.find(')').map(|pe| (be, be + 1 + pe + 1))
                    } else {
                        None
                    }
                });
                if let Some((text_end, hide_end)) = valid {
                    let target = &inner[text_end + 2..hide_end - 1];
                    if should_show_link_target(target) {
                        job.append("[", 0.0, code_format.clone());
                        job.append(&inner[..text_end], 0.0, format.clone());
                        job.append("](", 0.0, code_format.clone());
                        job.append(target, 0.0, code_format.clone());
                        job.append(")", 0.0, code_format.clone());
                    } else {
                        job.append("[", 0.0, hidden.clone());
                        job.append(&inner[..text_end], 0.0, format.clone());
                        job.append(&inner[text_end..hide_end], 0.0, hidden.clone());
                    }
                    remaining = &inner[hide_end..];
                } else {
                    job.append("[", 0.0, format.clone());
                    remaining = &remaining[1..];
                }
            }
            _ => unreachable!(),
        }
    }
}
