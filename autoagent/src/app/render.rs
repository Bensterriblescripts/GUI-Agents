use eframe::egui::{
    self, Color32, FontId,
    text::{LayoutJob, TextFormat},
};

use crate::config::{
    CANCELLED_BOTTOM_PADDING, CANCELLED_TEXT, HIDDEN_MARKDOWN_FONT_SIZE, LINE_HEIGHT,
    MIN_TEXT_WRAP_WIDTH, TEXT_FONT_SIZE,
};

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
) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width.max(MIN_TEXT_WRAP_WIDTH);

    let old_color = Color32::from_rgb(140, 145, 155);
    let plain_new = TextFormat {
        font_id: FontId::proportional(TEXT_FONT_SIZE),
        color: Color32::WHITE,
        ..Default::default()
    };
    let plain_old = TextFormat {
        font_id: FontId::proportional(TEXT_FONT_SIZE),
        color: old_color,
        ..Default::default()
    };
    let code_new = TextFormat {
        font_id: FontId::monospace(TEXT_FONT_SIZE),
        color: Color32::from_rgba_unmultiplied(188, 194, 202, 220),
        ..Default::default()
    };
    let code_old = TextFormat {
        font_id: FontId::monospace(TEXT_FONT_SIZE),
        color: Color32::from_rgb(130, 140, 150),
        ..Default::default()
    };
    let hidden = TextFormat {
        font_id: FontId::monospace(HIDDEN_MARKDOWN_FONT_SIZE),
        color: Color32::TRANSPARENT,
        ..Default::default()
    };
    let cancelled = TextFormat {
        font_id: FontId::proportional(TEXT_FONT_SIZE),
        color: Color32::from_rgb(255, 96, 96),
        italics: true,
        ..Default::default()
    };
    let cancelled_spacer = TextFormat {
        font_id: FontId::monospace(HIDDEN_MARKDOWN_FONT_SIZE),
        line_height: Some(CANCELLED_BOTTOM_PADDING),
        color: Color32::TRANSPARENT,
        ..Default::default()
    };
    let reasoning = TextFormat {
        font_id: FontId::proportional(TEXT_FONT_SIZE),
        color: Color32::from_rgb(130, 135, 145),
        ..Default::default()
    };
    let reasoning_code = TextFormat {
        font_id: FontId::monospace(TEXT_FONT_SIZE),
        color: Color32::from_rgb(130, 140, 150),
        ..Default::default()
    };

    let mut in_code = false;
    let mut byte_offset = 0usize;
    for line in text.split_inclusive('\n') {
        let is_prompt = prompt_ranges.iter().any(|&(s, e)| byte_offset >= s && byte_offset < e);
        let is_old = !is_prompt && byte_offset < response_start;
        let (rest, is_reasoning, is_agent, is_error) = if line.starts_with('\x1D') {
            job.append("\x1D", 0.0, hidden.clone());
            (&line[1..], false, false, true)
        } else if line.starts_with('\x1E') {
            job.append("\x1E", 0.0, hidden.clone());
            (&line[1..], true, false, false)
        } else if line.starts_with('\x1F') {
            job.append("\x1F", 0.0, hidden.clone());
            (&line[1..], false, true, false)
        } else {
            (line, false, false, false)
        };
        let content = rest.strip_suffix('\n').unwrap_or(rest).trim_start();
        if content.starts_with("```") {
            let fence = rest.strip_suffix('\n').unwrap_or(rest);
            job.append(fence, 0.0, hidden.clone());
            in_code = !in_code;
            if !in_code {
                job.append("\n\n", 0.0, plain_new.clone());
            }
            byte_offset += line.len();
            continue;
        }
        if !in_code && rest.strip_suffix('\n').unwrap_or(rest) == CANCELLED_TEXT {
            job.append(rest, 0.0, cancelled.clone());
            if !rest.ends_with('\n') {
                job.append("\n", 0.0, cancelled_spacer.clone());
            }
            byte_offset += line.len();
            continue;
        }
        let format = if is_error {
            &cancelled
        } else if is_reasoning {
            if in_code { &reasoning_code } else { &reasoning }
        } else if is_agent {
            if in_code { &code_new } else { &plain_new }
        } else if in_code {
            if is_old { &code_old } else { &code_new }
        } else {
            if is_old { &plain_old } else { &plain_new }
        };
        if in_code || is_reasoning || is_error {
            job.append(rest, 0.0, format.clone());
        } else if is_horizontal_rule(content) {
            job.append(rest, 0.0, hidden.clone());
        } else {
            let icf = if is_old { &code_old } else { &code_new };
            let hdr = header_prefix_len(content);
            let ws = rest.len() - rest.trim_start().len();
            if hdr > 0 {
                job.append(&rest[..ws + hdr], 0.0, hidden.clone());
                append_markdown_line(&mut job, &rest[ws + hdr..], format, icf, &hidden);
            } else {
                append_markdown_line(&mut job, rest, format, icf, &hidden);
            }
        }
        byte_offset += line.len();
    }

    if text.is_empty() {
        job.append("", 0.0, plain_new);
    }

    job
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
                    job.append(&remaining[..end], 0.0, hidden.clone());
                    job.append("**", 0.0, hidden.clone());
                    remaining = &remaining[end + 2..];
                    job.append("...\n\n", 0.0, format.clone());
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
                    job.append("[", 0.0, hidden.clone());
                    job.append(&inner[..text_end], 0.0, format.clone());
                    job.append(&inner[text_end..hide_end], 0.0, hidden.clone());
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

pub(super) fn text_metrics(text: &str, wrap_width: f32, ctx: &egui::Context) -> (usize, f32) {
    ctx.fonts(|fonts| {
        let galley = fonts.layout_job(markdown_layout_job(text, wrap_width, &[], 0));
        let rows = galley.rows.len().max(1);
        let height = galley.size().y.max(LINE_HEIGHT);
        (rows, height)
    })
}
