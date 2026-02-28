use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

enum CodexLauncher {
    Node { node: PathBuf, script: PathBuf },
    Cmd(PathBuf),
    Direct,
}

pub(super) fn build_codex_command(prompt: &str, session_id: Option<&str>) -> Command {
    match codex_launcher() {
        CodexLauncher::Node { node, script } => {
            let mut command = Command::new(node);
            command.arg(script);
            append_codex_args(&mut command, prompt, session_id);
            command
        }
        CodexLauncher::Cmd(codex_cmd) => {
            let mut command = Command::new("cmd.exe");
            command.arg("/C");
            command.arg(codex_cmd);
            append_codex_args(&mut command, prompt, session_id);
            command
        }
        CodexLauncher::Direct => {
            let mut command = Command::new("codex");
            append_codex_args(&mut command, prompt, session_id);
            command
        }
    }
}

fn codex_script_path() -> Option<PathBuf> {
    let appdata = env::var_os("APPDATA")?;
    let path = PathBuf::from(appdata)
        .join("npm")
        .join("node_modules")
        .join("@openai")
        .join("codex")
        .join("bin")
        .join("codex.js");
    path.exists().then_some(path)
}

fn node_path() -> Option<PathBuf> {
    let appdata = env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("npm").join("node.exe"));
    if let Some(path) = appdata.filter(|path| path.exists()) {
        return Some(path);
    }

    let program_files = env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .map(|path| path.join("nodejs").join("node.exe"));
    if let Some(path) = program_files.filter(|path| path.exists()) {
        return Some(path);
    }

    let local_programs = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("Programs").join("nodejs").join("node.exe"));
    local_programs.filter(|path| path.exists())
}

fn codex_cmd_path() -> Option<PathBuf> {
    let appdata = env::var_os("APPDATA")?;
    let path = PathBuf::from(appdata).join("npm").join("codex.cmd");
    path.exists().then_some(path)
}

fn codex_launcher() -> &'static CodexLauncher {
    static CODEX_LAUNCHER: OnceLock<CodexLauncher> = OnceLock::new();
    CODEX_LAUNCHER.get_or_init(|| {
        if let (Some(node), Some(script)) = (node_path(), codex_script_path()) {
            return CodexLauncher::Node { node, script };
        }
        if let Some(codex_cmd) = codex_cmd_path() {
            return CodexLauncher::Cmd(codex_cmd);
        }
        CodexLauncher::Direct
    })
}

fn append_codex_args(command: &mut Command, prompt: &str, session_id: Option<&str>) {
    command.arg("exec");
    if let Some(sid) = session_id {
        command.arg("resume");
        command.arg(sid);
    }
    command.arg("--json");
    command.arg(prompt);
}
