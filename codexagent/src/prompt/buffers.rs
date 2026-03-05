use std::borrow::Cow;

use serde_json::Value;

#[derive(Clone, Copy, Eq, PartialEq)]
enum SegmentKind {
    Plain,
    Agent,
    Reasoning,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SegmentStyle {
    Block,
    Streaming,
}

#[derive(Clone, Copy)]
struct Segment {
    kind: SegmentKind,
    style: SegmentStyle,
    display_start: usize,
}

#[derive(Default)]
pub(super) struct ResponseBuffers {
    segments: Vec<Segment>,
    display: String,
    last_text: String,
}

impl ResponseBuffers {
    pub(super) fn push_fragment(&mut self, text: &str) {
        self.push_fragment_inner(text, SegmentKind::Plain);
    }

    pub(super) fn push_agent_fragment(&mut self, text: &str) {
        self.push_fragment_inner(text, SegmentKind::Agent);
    }

    fn push_fragment_inner(&mut self, text: &str, kind: SegmentKind) {
        if text.is_empty() {
            return;
        }
        let text = strip_bold_blocks(text);
        let text = text.as_ref();
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.last_segment() {
            if last.kind == kind {
                let last_text = self.last_segment_text();
                if last_text == text || last_text.ends_with(text) {
                    return;
                }
                if text.starts_with(last_text) {
                    self.extend_last_segment_text(text, SegmentStyle::Block);
                    return;
                }
                if kind == SegmentKind::Agent && agent_fragment_matches(last_text, text) {
                    return;
                }
            }
        }
        self.append_segment(kind, SegmentStyle::Block, text);
    }

    pub(super) fn push_delta(&mut self, text: &str) {
        self.push_streaming_fragment(text, SegmentKind::Plain);
    }

    pub(super) fn push_reasoning(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let cleaned = strip_bold_markers(trimmed);
        let content = cleaned.as_ref().trim();
        if content.is_empty() {
            return;
        }
        if let Some(last) = self.last_segment() {
            if last.kind == SegmentKind::Reasoning {
                let last_text = self.last_segment_text();
                let existing = last_text.strip_suffix("...").unwrap_or(last_text);
                if existing.ends_with(content) {
                    return;
                }
                if content.starts_with(existing) {
                    self.replace_last_segment_with_suffix(content, "...", SegmentStyle::Block);
                    return;
                }
            }
        }
        self.append_segment_with_suffix(
            SegmentKind::Reasoning,
            SegmentStyle::Block,
            content,
            "...",
        );
    }

    pub(super) fn push_reasoning_delta(&mut self, text: &str) {
        self.push_streaming_fragment(text, SegmentKind::Reasoning);
    }

    fn push_streaming_fragment(&mut self, text: &str, kind: SegmentKind) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.last_segment() {
            if last.kind == kind {
                let last_text = self.last_segment_text();
                let existing = if kind == SegmentKind::Reasoning {
                    last_text.strip_suffix("...").unwrap_or(last_text)
                } else {
                    last_text
                };
                if last_text.ends_with(text) || existing.ends_with(text) {
                    return;
                }
                if text.starts_with(existing) {
                    if kind != SegmentKind::Reasoning && last_text == existing {
                        self.extend_last_segment_text(text, SegmentStyle::Streaming);
                    } else {
                        self.replace_last_segment_text(text, SegmentStyle::Streaming);
                    }
                } else {
                    if kind != SegmentKind::Reasoning && last_text.len() == existing.len() {
                        self.extend_last_segment_suffix(text, SegmentStyle::Streaming);
                    } else {
                        self.append_to_last_segment_text(
                            existing.len(),
                            text,
                            SegmentStyle::Streaming,
                        );
                    }
                }
                return;
            }
        }
        self.append_segment(
            kind,
            if self.segments.is_empty() {
                SegmentStyle::Streaming
            } else {
                SegmentStyle::Block
            },
            text,
        );
    }

    pub(super) fn has_deltas(&self) -> bool {
        self.segments
            .iter()
            .any(|segment| segment.style == SegmentStyle::Streaming)
    }

    pub(super) fn visible_len(&self) -> usize {
        self.display.len()
    }

    pub(super) fn visible_text(&self) -> &str {
        &self.display
    }

    pub(super) fn into_response(self) -> String {
        self.display
    }

    fn last_segment(&self) -> Option<&Segment> {
        self.segments.last()
    }

    fn last_segment_text(&self) -> &str {
        &self.last_text
    }

    fn append_segment(&mut self, kind: SegmentKind, style: SegmentStyle, text: &str) {
        self.append_segment_with_suffix(kind, style, text, "");
    }

    fn append_segment_with_suffix(
        &mut self,
        kind: SegmentKind,
        style: SegmentStyle,
        text: &str,
        suffix: &str,
    ) {
        let display_start = self.display.len();
        self.set_last_segment_text(text, suffix);
        append_segment_display(&mut self.display, kind, style, &self.last_text);
        self.segments.push(Segment {
            kind,
            style,
            display_start,
        });
    }

    fn replace_last_segment_text(&mut self, text: &str, style: SegmentStyle) {
        self.rewrite_last_segment(style, 0, text);
    }

    fn extend_last_segment_text(&mut self, text: &str, style: SegmentStyle) {
        if self.segments.is_empty() {
            return;
        }
        let Some(suffix) = text.strip_prefix(self.last_segment_text()) else {
            self.replace_last_segment_text(text, style);
            return;
        };
        self.extend_last_segment_suffix(suffix, style);
    }

    fn extend_last_segment_suffix(&mut self, suffix: &str, style: SegmentStyle) {
        let Some(last) = self.segments.last().copied() else {
            return;
        };
        if suffix.is_empty() {
            if let Some(last) = self.segments.last_mut() {
                last.style = style;
            }
            return;
        }

        let line_start =
            self.last_text.is_empty() || self.last_text.as_bytes().last() == Some(&b'\n');
        self.last_text.push_str(suffix);
        append_segment_display_continuation(&mut self.display, last.kind, line_start, suffix);

        if let Some(last) = self.segments.last_mut() {
            last.style = style;
        }
    }

    fn replace_last_segment_with_suffix(&mut self, text: &str, suffix: &str, style: SegmentStyle) {
        let Some(last) = self.segments.last() else {
            return;
        };
        let kind = last.kind;
        let display_start = last.display_start;

        self.set_last_segment_text(text, suffix);
        self.display.truncate(display_start);
        append_segment_display(&mut self.display, kind, style, &self.last_text);

        if let Some(last) = self.segments.last_mut() {
            last.style = style;
        }
    }

    fn append_to_last_segment_text(
        &mut self,
        prefix_len: usize,
        suffix: &str,
        style: SegmentStyle,
    ) {
        self.rewrite_last_segment(style, prefix_len, suffix);
    }

    fn rewrite_last_segment(&mut self, style: SegmentStyle, prefix_len: usize, suffix: &str) {
        let Some(last) = self.segments.last() else {
            return;
        };
        let kind = last.kind;
        let display_start = last.display_start;

        self.last_text.truncate(prefix_len);
        self.last_text.push_str(suffix);
        self.display.truncate(display_start);
        append_segment_display(&mut self.display, kind, style, &self.last_text);

        if let Some(last) = self.segments.last_mut() {
            last.style = style;
        }
    }

    fn set_last_segment_text(&mut self, text: &str, suffix: &str) {
        self.last_text.clear();
        self.last_text.reserve(text.len() + suffix.len());
        self.last_text.push_str(text);
        self.last_text.push_str(suffix);
    }
}

pub(super) fn collect_response_text(value: &Value, response: &mut ResponseBuffers) {
    match value {
        Value::Object(map) => {
            let kind = map.get("type").and_then(Value::as_str).unwrap_or("");

            if kind.contains("reasoning") {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    response.push_reasoning(text);
                }
                if let Some(delta) = map.get("delta").and_then(Value::as_str) {
                    response.push_reasoning_delta(delta);
                }
                if let Some(summary) = map.get("summary") {
                    collect_reasoning_text(summary, response);
                }
                return;
            }

            if let Some(text) = map.get("output_text").and_then(Value::as_str) {
                response.push_fragment(text);
            }
            if let Some(delta) = map.get("delta").and_then(Value::as_str) {
                response.push_delta(delta);
            }
            if matches!(
                kind,
                "output_text" | "text" | "agent_message" | "assistant_message"
            ) {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    if kind == "agent_message" {
                        response.push_agent_fragment(text);
                    } else {
                        response.push_fragment(text);
                    }
                }
            }
            if kind == "message" && map.get("role").and_then(Value::as_str) == Some("assistant") {
                if let Some(text) = map.get("text").and_then(Value::as_str) {
                    response.push_fragment(text);
                }
            }
            for key in [
                "content", "contents", "item", "items", "message", "messages", "output",
            ] {
                if let Some(child) = map.get(key) {
                    collect_response_text(child, response);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_response_text(item, response);
            }
        }
        _ => {}
    }
}

fn strip_bold_blocks(text: &str) -> Cow<'_, str> {
    let Some(first) = text.find("**") else {
        return Cow::Borrowed(text);
    };
    let mut result = String::with_capacity(text.len());
    let mut remaining = &text[first..];
    result.push_str(&text[..first]);
    loop {
        remaining = &remaining[2..];
        if let Some(end) = remaining.find("**") {
            remaining = &remaining[end + 2..];
            result.push_str("...\n\n");
        } else {
            result.push_str("**");
            result.push_str(remaining);
            break;
        }
        if let Some(start) = remaining.find("**") {
            result.push_str(&remaining[..start]);
            remaining = &remaining[start..];
        } else {
            result.push_str(remaining);
            break;
        }
    }
    Cow::Owned(result)
}

fn strip_bold_markers(text: &str) -> Cow<'_, str> {
    let Some(first) = text.find("**") else {
        return Cow::Borrowed(text);
    };
    let mut result = String::with_capacity(text.len());
    let mut remaining = &text[first..];
    result.push_str(&text[..first]);
    while let Some(start) = remaining.find("**") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + 2..];
    }
    result.push_str(remaining);
    Cow::Owned(result)
}

fn needs_break(display: &str, style: SegmentStyle, text: &str) -> bool {
    !display.is_empty()
        && style == SegmentStyle::Block
        && !display.ends_with('\n')
        && !text.starts_with('\n')
}

fn append_segment_display(
    display: &mut String,
    kind: SegmentKind,
    style: SegmentStyle,
    text: &str,
) {
    display.reserve(rendered_capacity(display, style, text, kind));
    if needs_break(display, style, text) {
        display.push_str("\n\n");
    }
    match kind {
        SegmentKind::Plain => display.push_str(text),
        SegmentKind::Agent => append_marked(display, '\x1F', text),
        SegmentKind::Reasoning => append_marked(display, '\x1E', text),
    }
}

fn rendered_capacity(display: &str, style: SegmentStyle, text: &str, kind: SegmentKind) -> usize {
    let mut capacity = text.len();
    if needs_break(display, style, text) {
        capacity += 2;
    }
    if kind != SegmentKind::Plain {
        capacity += text.lines().count().max(1);
    }
    capacity
}

fn append_marked(display: &mut String, marker: char, text: &str) {
    append_marked_continuation(display, marker, true, text);
}

fn append_segment_display_continuation(
    display: &mut String,
    kind: SegmentKind,
    line_start: bool,
    text: &str,
) {
    match kind {
        SegmentKind::Plain => display.push_str(text),
        SegmentKind::Agent => append_marked_continuation(display, '\x1F', line_start, text),
        SegmentKind::Reasoning => append_marked_continuation(display, '\x1E', line_start, text),
    }
}

fn append_marked_continuation(
    display: &mut String,
    marker: char,
    mut line_start: bool,
    text: &str,
) {
    for line in text.split_inclusive('\n') {
        if line_start {
            display.push(marker);
        }
        display.push_str(line);
        line_start = line.ends_with('\n');
    }
}

fn agent_fragment_matches(marked: &str, text: &str) -> bool {
    let mut remaining = marked;
    for line in text.split_inclusive('\n') {
        let Some(next) = remaining.strip_prefix(line) else {
            return false;
        };
        remaining = next;
    }
    remaining.is_empty()
}

fn collect_reasoning_text(value: &Value, response: &mut ResponseBuffers) {
    match value {
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                response.push_reasoning(text);
            }
            for key in ["summary", "content", "contents", "items"] {
                if let Some(child) = map.get(key) {
                    collect_reasoning_text(child, response);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_reasoning_text(item, response);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ResponseBuffers, collect_response_text};

    #[test]
    fn preserves_explicit_newline_fragments() {
        let mut response = ResponseBuffers::default();
        response.push_fragment("first line");
        response.push_fragment("\n");
        response.push_fragment("second line");

        assert_eq!(response.into_response(), "first line\nsecond line");
    }

    #[test]
    fn separates_plain_fragments_into_paragraphs() {
        let mut response = ResponseBuffers::default();
        response.push_fragment("first");
        response.push_fragment("second");

        assert_eq!(response.into_response(), "first\n\nsecond");
    }

    #[test]
    fn keeps_agent_and_output_in_event_order() {
        let mut response = ResponseBuffers::default();
        response.push_agent_fragment("Planning repo inspection...");
        response.push_fragment("I'm scanning the codebase.");

        assert_eq!(
            response.into_response(),
            "\x1FPlanning repo inspection...\n\nI'm scanning the codebase."
        );
    }

    #[test]
    fn replaces_streaming_plain_text_with_snapshot() {
        let mut response = ResponseBuffers::default();
        response.push_delta("I'm scanning");
        response.push_fragment("I'm scanning the codebase.");

        assert_eq!(response.into_response(), "I'm scanning the codebase.");
    }

    #[test]
    fn collects_interleaved_items_in_order() {
        let mut response = ResponseBuffers::default();
        collect_response_text(
            &json!({"type":"agent_message","text":"Planning repo inspection..."}),
            &mut response,
        );
        collect_response_text(
            &json!({"type":"assistant_message","text":"I'm scanning the codebase."}),
            &mut response,
        );
        collect_response_text(
            &json!({"type":"agent_message","text":"Inspecting logging and error handling..."}),
            &mut response,
        );

        assert_eq!(
            response.into_response(),
            "\x1FPlanning repo inspection...\n\nI'm scanning the codebase.\n\n\x1FInspecting logging and error handling..."
        );
    }

    #[test]
    fn reasoning_delta_replaces_snapshot_without_duplication() {
        let mut response = ResponseBuffers::default();
        response.push_reasoning("Thinking");
        response.push_reasoning_delta(" harder");

        assert_eq!(response.into_response(), "\x1EThinking harder");
    }
}
