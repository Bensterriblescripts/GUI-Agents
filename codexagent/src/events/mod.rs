use crate::runtime::ContextMenuSelection;

pub(crate) enum PromptResult {
    Ok(String, Option<String>),
    Err(String),
}

pub(crate) enum CodexCheckResult {
    Ready,
    NotInstalled { node_available: bool },
}

pub(crate) enum AppEvent {
    PromptStream(u64),
    Prompt(u64, PromptResult),
    CodexCheck(CodexCheckResult),
    CodexInstallOutput(String),
    CodexInstallDone(Result<(), String>),
    ContextMenuSelection(Result<ContextMenuSelection, String>),
}
