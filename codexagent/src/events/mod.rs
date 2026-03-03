pub(crate) enum PromptResult {
    Ok(String, Option<String>),
    Err(String),
}

pub(crate) enum AppEvent {
    PromptStream(u64),
    Prompt(u64, PromptResult),
}
