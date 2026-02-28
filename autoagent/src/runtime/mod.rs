use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::config::{CODEX_AGENTS_CONTENTS, CODEX_CONFIG_CONTENTS};
use crate::logging;

pub(crate) fn ensure_codex_files() -> io::Result<()> {
    logging::trace("ensuring codex files");
    let Some(user_profile) = env::var_os("USERPROFILE") else {
        logging::trace("USERPROFILE not set; skipping codex file setup");
        return Ok(());
    };

    let codex_dir = PathBuf::from(user_profile).join(".codex");
    let config_path = codex_dir.join("config.toml");
    let agents_path = codex_dir.join("AGENTS.md");

    if !config_path.exists() || !agents_path.exists() {
        fs::create_dir_all(&codex_dir)?;
    }

    write_file_if_missing(&config_path, CODEX_CONFIG_CONTENTS)?;
    write_file_if_missing(&agents_path, CODEX_AGENTS_CONTENTS)?;

    logging::trace(format!("codex files ready in {}", codex_dir.display()));
    Ok(())
}

pub(crate) fn current_cwd_text() -> String {
    env::current_dir()
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

fn write_file_if_missing(path: &Path, contents: &[u8]) -> io::Result<()> {
    match OpenOptions::new().create_new(true).write(true).open(path) {
        Ok(mut file) => {
            logging::trace(format!("creating {}", path.display()));
            file.write_all(contents)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}
