use std::borrow::Cow;

use serde_json::Value;

#[derive(Default)]
pub(super) struct ResponseBuffers {
    reasoning: String,
    joined_fragments: String,
    last_fragment_start: Option<usize>,
    deltas: String,
    content_start: usize,
    display: String,
    display_dirty: bool,
}

impl ResponseBuffers {
    fn last_fragment(&self) -> Option<&str> {
        self.last_fragment_start
            .map(|start| &self.joined_fragments[start..])
    }

    pub(super) fn push_fragment(&mut self, text: &str) {
        self.push_fragment_inner(text, false);
    }

    pub(super) fn push_agent_fragment(&mut self, text: &str) {
        self.push_fragment_inner(text, true);
    }

    fn push_fragment_inner(&mut self, text: &str, agent: bool) {
        if text.trim().is_empty() {
            return;
        }
        let text = strip_bold_blocks(text);
        let text = text.as_ref();
        if text.trim().is_empty() {
            return;
        }
        if agent {
            if self
                .last_fragment()
                .is_some_and(|last| agent_fragment_matches(last, text))
            {
                return;
            }
            if !self.joined_fragments.is_empty() {
                self.joined_fragments.push_str("\n\n");
            }
            let start = self.joined_fragments.len();
            append_agent_fragment(&mut self.joined_fragments, text);
            self.last_fragment_start = Some(start);
            self.display_dirty = true;
            return;
        }
        if self.last_fragment().is_some_and(|last| last == text) {
            return;
        }
        if !self.joined_fragments.is_empty() {
            self.joined_fragments.push_str("\n\n");
        }
        let start = self.joined_fragments.len();
        self.joined_fragments.push_str(text);
        self.last_fragment_start = Some(start);
        self.display_dirty = true;
    }

    pub(super) fn push_delta(&mut self, text: &str) {
        if text.is_empty() || self.deltas.ends_with(text) {
            return;
        }
        if !self.reasoning.is_empty() && self.content_start == self.deltas.len() {
            if !self.deltas.ends_with('\n') {
                self.deltas.push('\n');
            }
            self.deltas.push('\n');
            self.content_start = self.deltas.len();
        }
        self.deltas.push_str(text);
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
        let existing = self.reasoning.strip_suffix("...").unwrap_or(&self.reasoning);
        if existing.ends_with(content) {
            return;
        }
        if !self.reasoning.is_empty() {
            self.reasoning.push('\n');
        }
        self.reasoning.push_str(content);
        self.reasoning.push_str("...");
        if !self.deltas.is_empty() && !self.deltas.ends_with('\n') {
            self.deltas.push('\n');
        }
        self.deltas.push('\x1E');
        self.deltas.push_str(content);
        self.deltas.push_str("...\n");
        self.content_start = self.deltas.len();
        self.display_dirty = true;
    }

    pub(super) fn push_reasoning_delta(&mut self, text: &str) {
        if text.is_empty() || self.reasoning.ends_with(text) {
            return;
        }
        self.reasoning.push_str(text);
        for line in text.split_inclusive('\n') {
            if self.deltas.is_empty() || self.deltas.ends_with('\n') {
                self.deltas.push('\x1E');
            }
            self.deltas.push_str(line);
        }
        self.content_start = self.deltas.len();
        self.display_dirty = true;
    }

    pub(super) fn has_deltas(&self) -> bool {
        !self.deltas.is_empty()
    }

    pub(super) fn visible_len(&mut self) -> usize {
        if !self.deltas.is_empty() {
            return self.deltas.len();
        }
        if self.reasoning.is_empty() {
            return self.joined_fragments.len();
        }
        self.reasoning_display_len() + 1 + self.joined_fragments.len()
    }

    pub(super) fn visible_text(&mut self) -> &str {
        if !self.deltas.is_empty() {
            return &self.deltas;
        }
        if self.reasoning.is_empty() {
            return &self.joined_fragments;
        }
        if self.display_dirty {
            self.rebuild_display();
        }
        &self.display
    }

    fn rebuild_display(&mut self) {
        self.display.clear();
        for line in self.reasoning.split_inclusive('\n') {
            self.display.push('\x1E');
            self.display.push_str(line);
            if !line.ends_with('\n') {
                self.display.push('\n');
            }
        }
        self.display.push('\n');
        self.display.push_str(&self.joined_fragments);
        self.display_dirty = false;
    }

    pub(super) fn into_response(self) -> String {
        let Self {
            reasoning,
            joined_fragments,
            deltas,
            content_start,
            ..
        } = self;

        if reasoning.is_empty() {
            if joined_fragments.is_empty() {
                return if deltas.contains("**") {
                    strip_bold_blocks(&deltas).into_owned()
                } else {
                    deltas
                };
            }
            return joined_fragments;
        }
        let output = if !joined_fragments.is_empty() {
            joined_fragments
        } else if content_start < deltas.len() {
            strip_bold_blocks(&deltas[content_start..]).into_owned()
        } else {
            String::new()
        };
        let mut result = String::with_capacity(reasoning.len() * 2 + output.len());
        for line in reasoning.split_inclusive('\n') {
            result.push('\x1E');
            result.push_str(line);
            if !line.ends_with('\n') {
                result.push('\n');
            }
        }
        result.push('\n');
        result.push_str(&output);
        result
    }

    fn reasoning_display_len(&self) -> usize {
        let mut len = 0;
        for line in self.reasoning.split_inclusive('\n') {
            len += 1 + line.len();
            if !line.ends_with('\n') {
                len += 1;
            }
        }
        len
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

fn append_agent_fragment(result: &mut String, text: &str) {
    result.reserve(text.len() + text.bytes().filter(|&b| b == b'\n').count() + 1);
    for line in text.split_inclusive('\n') {
        result.push('\x1F');
        result.push_str(line);
    }
}

fn agent_fragment_matches(marked: &str, text: &str) -> bool {
    let mut remaining = marked;
    for line in text.split_inclusive('\n') {
        let Some(next) = remaining.strip_prefix('\x1F') else {
            return false;
        };
        let Some(next) = next.strip_prefix(line) else {
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
