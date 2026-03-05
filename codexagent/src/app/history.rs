use crate::config::trim_prompt_history;

use super::{CodexAgentApp, SLASH_COMMANDS, SlashCommand};

const SLASH_COMMAND_PANEL_TOP_SPACING: f32 = 6.0;
const SLASH_COMMAND_PANEL_ROW_HEIGHT: f32 = 28.0;
const SLASH_COMMAND_PANEL_ROW_SPACING: f32 = 4.0;
const SLASH_COMMAND_PANEL_PADDING_Y: f32 = 8.0;

impl CodexAgentApp {
    pub(super) fn slash_command_query(&self) -> Option<&str> {
        let input = self.input.trim();
        let query = input.strip_prefix('/')?;
        if query.contains(char::is_whitespace) {
            return None;
        }
        Some(query)
    }

    pub(super) fn should_show_slash_command(&self, command: &SlashCommand) -> bool {
        self.slash_command_query()
            .is_some_and(|query| command.name.starts_with(query))
    }

    pub(super) fn slash_command_count(&self) -> usize {
        SLASH_COMMANDS
            .iter()
            .filter(|command| self.should_show_slash_command(command))
            .count()
    }

    pub(super) fn clear_picker_selection(&mut self) {
        self.picker_selection = None;
    }

    pub(super) fn reset_prompt_history_navigation(&mut self) {
        self.prompt_history_index = None;
        self.prompt_history_draft = None;
    }

    pub(super) fn push_prompt_history(&mut self, prompt: &str) {
        if prompt.is_empty() {
            return;
        }
        if self
            .prompt_history
            .last()
            .is_some_and(|entry| entry == prompt)
        {
            self.reset_prompt_history_navigation();
            return;
        }
        self.prompt_history.push(prompt.to_owned());
        trim_prompt_history(&mut self.prompt_history);
        self.reset_prompt_history_navigation();
    }

    pub(super) fn browse_prompt_history(&mut self, newer: bool) -> bool {
        if self.prompt_history.is_empty() {
            return false;
        }
        match self.prompt_history_index {
            Some(index) if newer => {
                if index + 1 >= self.prompt_history.len() {
                    self.prompt_history_index = None;
                    self.input = self.prompt_history_draft.take().unwrap_or_default();
                } else {
                    let next = index + 1;
                    self.prompt_history_index = Some(next);
                    self.input = self.prompt_history[next].clone();
                }
            }
            Some(index) => {
                let next = index.saturating_sub(1);
                self.prompt_history_index = Some(next);
                self.input = self.prompt_history[next].clone();
            }
            None if newer => return false,
            None => {
                self.prompt_history_draft = Some(self.input.clone());
                let next = self.prompt_history.len() - 1;
                self.prompt_history_index = Some(next);
                self.input = self.prompt_history[next].clone();
            }
        }
        self.pending_input_focus = true;
        self.refresh_after_input_change();
        true
    }

    pub(super) fn picker_selection(&self) -> Option<usize> {
        self.picker_selection
            .filter(|selection| *selection < self.picker_item_count())
    }

    pub(super) fn move_picker_selection(&mut self, offset: isize) -> bool {
        let count = self.picker_item_count();
        if count == 0 {
            self.picker_selection = None;
            return false;
        }
        let next = match self.picker_selection() {
            Some(selection) => {
                let last = count.saturating_sub(1) as isize;
                (selection as isize + offset).clamp(0, last) as usize
            }
            None if offset < 0 => count - 1,
            None => 0,
        };
        self.picker_selection = Some(next);
        true
    }

    pub(super) fn activate_picker_selection(&mut self) -> bool {
        let Some(selection) = self.picker_selection() else {
            return false;
        };
        let mut index = 0;
        for command in SLASH_COMMANDS.iter() {
            if !self.should_show_slash_command(command) {
                continue;
            }
            if index == selection {
                self.select_slash_command(command.name);
                return true;
            }
            index += 1;
        }
        false
    }

    pub(super) fn picker_item_count(&self) -> usize {
        self.slash_command_count()
    }

    pub(super) fn command_panel_height(&self) -> f32 {
        let count = self.picker_item_count();
        if count == 0 {
            return 0.0;
        }
        SLASH_COMMAND_PANEL_TOP_SPACING
            + SLASH_COMMAND_PANEL_PADDING_Y * 2.0
            + SLASH_COMMAND_PANEL_ROW_HEIGHT * count as f32
            + SLASH_COMMAND_PANEL_ROW_SPACING * count.saturating_sub(1) as f32
    }
}
