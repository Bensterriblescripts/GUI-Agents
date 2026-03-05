use std::env;
use std::io::{self, BufRead};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use eframe::egui;
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

use crate::events::{AppEvent, CodexCheckResult};
use crate::logging;

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

pub(crate) fn check_codex_availability() -> CodexCheckResult {
    if codex_script_path().is_some() || codex_cmd_path().is_some() {
        return CodexCheckResult::Ready;
    }
    match Command::new("codex")
        .arg("--version")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => {
            if output.status.success() {
                return CodexCheckResult::Ready;
            }
            CodexCheckResult::NotInstalled {
                node_available: has_node(),
            }
        }
        Err(_) => CodexCheckResult::NotInstalled {
            node_available: has_node(),
        },
    }
}

pub(crate) fn run_full_install(
    node_available: bool,
    tx: &mpsc::Sender<AppEvent>,
    ctx: &egui::Context,
    install_stdin: &Arc<Mutex<Option<ChildStdin>>>,
) -> Result<(), String> {
    if !node_available {
        send_install_output(tx, ctx, "Installing Node.js via winget...\n".to_owned());
        run_node_install(tx, ctx, install_stdin)?;
        {
            let mut guard = install_stdin.lock().unwrap_or_else(|e| e.into_inner());
            *guard = None;
        }
        send_install_output(tx, ctx, "\nNode.js installed.\n".to_owned());
    }

    if !node_available {
        send_install_output(tx, ctx, "\nInstalling Codex CLI...\n\n".to_owned());
    }
    run_codex_install(tx, ctx)
}

fn run_node_install(
    tx: &mpsc::Sender<AppEvent>,
    ctx: &egui::Context,
    install_stdin: &Arc<Mutex<Option<ChildStdin>>>,
) -> Result<(), String> {
    let mut child = Command::new("winget")
        .args(["install", "OpenJS.NodeJS", "--accept-package-agreements"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            let message = format!("winget: {}", e);
            logging::error(format!("failed to start Node.js install: {}", message));
            message
        })?;

    let stdin = child.stdin.take();
    {
        let mut guard = install_stdin.lock().unwrap_or_else(|e| e.into_inner());
        *guard = stdin;
    }

    read_child_output(&mut child, tx, ctx);

    let status = child.wait().map_err(|e| {
        let message = e.to_string();
        logging::error(format!(
            "failed while waiting for winget install: {}",
            message
        ));
        message
    })?;
    if status.success() {
        Ok(())
    } else {
        let message = format!("winget install exited with {}", status);
        logging::error(message.clone());
        Err(message)
    }
}

fn run_codex_install(tx: &mpsc::Sender<AppEvent>, ctx: &egui::Context) -> Result<(), String> {
    let npm = find_npm();
    let mut child = Command::new(&npm)
        .args(["i", "-g", "@openai/codex@latest"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            let message = format!("{}: {}", npm.display(), e);
            logging::error(format!("failed to start Codex CLI install: {}", message));
            message
        })?;

    read_child_output(&mut child, tx, ctx);

    let status = child.wait().map_err(|e| {
        let message = e.to_string();
        logging::error(format!(
            "failed while waiting for Codex CLI install: {}",
            message
        ));
        message
    })?;
    if status.success() {
        Ok(())
    } else {
        let message = format!("npm install exited with {}", status);
        logging::error(message.clone());
        Err(message)
    }
}

fn read_child_output(
    child: &mut std::process::Child,
    tx: &mpsc::Sender<AppEvent>,
    ctx: &egui::Context,
) {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let tx_out = tx.clone();
    let ctx_out = ctx.clone();
    let stdout_handle = stdout.map(|out| {
        thread::spawn(move || {
            forward_child_output(io::BufReader::new(out), "stdout", &tx_out, &ctx_out);
        })
    });
    if let Some(err) = stderr {
        forward_child_output(io::BufReader::new(err), "stderr", tx, ctx);
    }
    if let Some(handle) = stdout_handle {
        if handle.join().is_err() {
            logging::error("installer stdout reader thread panicked");
        }
    }
}

fn send_install_output(tx: &mpsc::Sender<AppEvent>, ctx: &egui::Context, line: String) {
    if tx.send(AppEvent::CodexInstallOutput(line)).is_err() {
        logging::error("failed to deliver install output to app");
    }
    ctx.request_repaint();
}

fn forward_child_output<R: BufRead>(
    reader: R,
    stream_name: &str,
    tx: &mpsc::Sender<AppEvent>,
    ctx: &egui::Context,
) {
    for line in reader.lines() {
        match line {
            Ok(line) => send_install_output(tx, ctx, line),
            Err(error) => {
                logging::error(format!(
                    "failed to read installer {}: {}",
                    stream_name, error
                ));
                break;
            }
        }
    }
}

pub(crate) fn has_node() -> bool {
    if node_path().is_some() {
        return true;
    }
    Command::new("node")
        .arg("--version")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn find_npm() -> PathBuf {
    if let Some(pf) = env::var_os("ProgramFiles") {
        let path = PathBuf::from(&pf).join("nodejs").join("npm.cmd");
        if path.exists() {
            return path;
        }
    }
    if let Some(local) = env::var_os("LOCALAPPDATA") {
        let path = PathBuf::from(local)
            .join("Programs")
            .join("nodejs")
            .join("npm.cmd");
        if path.exists() {
            return path;
        }
    }
    if let Some(appdata) = env::var_os("APPDATA") {
        let path = PathBuf::from(&appdata).join("npm").join("npm.cmd");
        if path.exists() {
            return path;
        }
    }
    PathBuf::from("npm")
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

fn codex_launcher() -> CodexLauncher {
    if let (Some(node), Some(script)) = (node_path(), codex_script_path()) {
        return CodexLauncher::Node { node, script };
    }
    if let Some(codex_cmd) = codex_cmd_path() {
        return CodexLauncher::Cmd(codex_cmd);
    }
    CodexLauncher::Direct
}

fn append_codex_args(command: &mut Command, prompt: &str, session_id: Option<&str>) {
    command.arg("exec");
    if session_id.is_some() {
        command.arg("resume");
    }
    command.arg("--skip-git-repo-check");
    command.arg("--json");
    if let Some(sid) = session_id {
        command.arg(sid);
    }
    command.arg(prompt);
}
