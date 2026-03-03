pub(crate) const APP_NAME: &str = "codexagent";
pub(crate) const APP_DISPLAY_NAME: &str = "Codex Agent";
pub(crate) const APP_USER_MODEL_ID: &str = "Codex.Agent";
pub(crate) const DEFAULT_MODEL: &str = "gpt-5.3-codex";
pub(crate) const WINDOW_PADDING: f32 = 36.0;
pub(crate) const WINDOW_BOTTOM_PADDING: f32 = 44.0;
pub(crate) const LINE_HEIGHT: f32 = 20.0;
pub(crate) const TEXT_FONT_SIZE: f32 = 14.0;
pub(crate) const MAX_VISIBLE_ROWS: usize = 160;
pub(crate) const DEFAULT_WINDOW_WIDTH: f32 = 720.0;
pub(crate) const DEFAULT_WINDOW_HEIGHT: f32 =
    58.0 + LINE_HEIGHT + WINDOW_PADDING + WINDOW_BOTTOM_PADDING;
pub(crate) const MIN_WINDOW_WIDTH: f32 = 420.0;
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
pub(crate) const PENDING_ANIMATION_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(400);
pub(crate) const CANCELLED_TEXT: &str = "cancelled";
