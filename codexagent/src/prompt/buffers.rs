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

struct Segment {
    kind: SegmentKind,
    style: SegmentStyle,
    text: String,
}

#[derive(Default)]
pub(super) struct ResponseBuffers {
    segments: Vec<Segment>,
    display: String,
    display_dirty: bool,
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
        if let Some(last) = self.segments.last_mut() {
            if last.kind == kind {
                if last.text == text || last.text.ends_with(text) {
                    return;
                }
                if text.starts_with(&last.text) {
                    last.text.clear();
                    last.text.push_str(text);
                    last.style = SegmentStyle::Block;
                    self.display_dirty = true;
                    return;
                }
                if kind == SegmentKind::Agent && agent_fragment_matches(&last.text, text) {
                    return;
                }
            }
        }
        self.segments.push(Segment {
            kind,
            style: SegmentStyle::Block,
            text: text.to_owned(),
        });
        self.display_dirty = true;
    }

    pub(super) fn push_delta(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.segments.last_mut() {
            if last.kind == SegmentKind::Plain {
                if last.text.ends_with(text) {
                    return;
                }
                if text.starts_with(&last.text) {
                    last.text.clear();
                    last.text.push_str(text);
                } else {
                    last.text.push_str(text);
                }
                last.style = SegmentStyle::Streaming;
                self.display_dirty = true;
                return;
            }
        }
        self.segments.push(Segment {
            kind: SegmentKind::Plain,
            style: if self.segments.is_empty() {
                SegmentStyle::Streaming
            } else {
                SegmentStyle::Block
            },
            text: text.to_owned(),
        });
        self.display_dirty = true;
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
        if let Some(last) = self.segments.last_mut() {
            if last.kind == SegmentKind::Reasoning {
                let existing = last.text.strip_suffix("...").unwrap_or(&last.text);
                if existing.ends_with(content) {
                    return;
                }
                if content.starts_with(existing) {
                    last.text.clear();
                    last.text.push_str(content);
                    last.text.push_str("...");
                    last.style = SegmentStyle::Block;
                    self.display_dirty = true;
                    return;
                }
            }
        }
        let mut text = String::with_capacity(content.len() + 3);
        text.push_str(content);
        text.push_str("...");
        self.segments.push(Segment {
            kind: SegmentKind::Reasoning,
            style: SegmentStyle::Block,
            text,
        });
        self.display_dirty = true;
    }

    pub(super) fn push_reasoning_delta(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.segments.last_mut() {
            if last.kind == SegmentKind::Reasoning {
                if last.text.ends_with(text) {
                    return;
                }
                if text.starts_with(&last.text) {
                    last.text.clear();
                    last.text.push_str(text);
                } else {
                    last.text.push_str(text);
                }
                last.style = SegmentStyle::Streaming;
                self.display_dirty = true;
                return;
            }
        }
        self.segments.push(Segment {
            kind: SegmentKind::Reasoning,
            style: if self.segments.is_empty() {
                SegmentStyle::Streaming
            } else {
                SegmentStyle::Block
            },
            text: text.to_owned(),
        });
        self.display_dirty = true;
    }

    pub(super) fn has_deltas(&self) -> bool {
        self.segments
            .iter()
            .any(|segment| segment.style == SegmentStyle::Streaming)
    }

    pub(super) fn visible_len(&mut self) -> usize {
        self.visible_text().len()
    }

    pub(super) fn visible_text(&mut self) -> &str {
        if self.display_dirty {
            self.rebuild_display();
        }
        &self.display
    }

    fn rebuild_display(&mut self) {
        self.display.clear();
        for segment in &self.segments {
            if needs_break(&self.display, segment) {
                self.display.push_str("\n\n");
            }
            match segment.kind {
                SegmentKind::Plain => self.display.push_str(&segment.text),
                SegmentKind::Agent => append_marked(&mut self.display, '\x1F', &segment.text),
                SegmentKind::Reasoning => append_marked(&mut self.display, '\x1E', &segment.text),
            }
        }
        self.display_dirty = false;
    }

    pub(super) fn into_response(mut self) -> String {
        if self.display_dirty {
            self.rebuild_display();
        }
        self.display
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

fn needs_break(display: &str, segment: &Segment) -> bool {
    !display.is_empty()
        && segment.style == SegmentStyle::Block
        && !display.ends_with('\n')
        && !segment.text.starts_with('\n')
}

fn append_marked(display: &mut String, marker: char, text: &str) {
    display.reserve(text.len() + text.lines().count().max(1));
    for line in text.split_inclusive('\n') {
        display.push(marker);
        display.push_str(line);
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
}
