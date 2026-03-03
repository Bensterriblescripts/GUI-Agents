use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

use crate::config::{
    APP_DISPLAY_NAME, APP_USER_MODEL_ID, CODEX_AGENTS_CONTENTS, CODEX_CONFIG_CONTENTS,
    DEFAULT_MODEL,
};
use crate::logging;

const INSTALL_PATH: &str = r"C:\Local\Software\codexagent.exe";
const LEGACY_SHORTCUT_NAMES: &[&str] = &[];

pub(crate) fn ensure_app_identity() {
    let wide: Vec<u16> = APP_USER_MODEL_ID
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let hr = unsafe { SetCurrentProcessExplicitAppUserModelID(wide.as_ptr()) };
    if hr == 0 {
        logging::trace("set AppUserModelID");
    } else {
        logging::error(format!(
            "SetCurrentProcessExplicitAppUserModelID failed: 0x{:08x}",
            hr
        ));
    }
    ensure_start_menu_shortcut();
}

fn ensure_start_menu_shortcut() {
    let Some(appdata) = env::var_os("APPDATA") else {
        return;
    };
    let programs_dir = PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs");
    let lnk_path = programs_dir.join(format!("{}.lnk", APP_DISPLAY_NAME));

    for legacy_name in LEGACY_SHORTCUT_NAMES {
        let legacy_path = programs_dir.join(format!("{}.lnk", legacy_name));
        if !legacy_path.exists() {
            continue;
        }
        if let Err(e) = fs::remove_file(&legacy_path) {
            logging::error(format!(
                "failed to remove legacy shortcut {}: {}",
                legacy_path.display(),
                e
            ));
        } else {
            logging::trace(format!(
                "removed legacy shortcut: {}",
                legacy_path.display()
            ));
        }
    }

    let lnk = lnk_path.display().to_string();
    let script = format!(
        concat!(
            "$lnk='{lnk}';$exe='{exe}';$id='{id}';",
            "$ws=New-Object -ComObject WScript.Shell;",
            "$sc=$ws.CreateShortcut($lnk);",
            "$sc.TargetPath=$exe;$sc.Save();",
            "Add-Type -TypeDefinition @\"\n",
            "using System;using System.Runtime.InteropServices;\n",
            "public static class SA{{\n",
            "  [ComImport,Guid(\"\"886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99\"\"),InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]\n",
            "  interface IPS{{void GC(out uint c);void GA(uint i,out Guid k);void GV(ref PK k,out PV v);void SV(ref PK k,ref PV v);void Co();}}\n",
            "  [StructLayout(LayoutKind.Sequential,Pack=4)]public struct PK{{public Guid f;public uint p;}}\n",
            "  [StructLayout(LayoutKind.Sequential)]public struct PV{{public ushort vt;ushort r1,r2,r3;public IntPtr d;IntPtr pad;}}\n",
            "  [DllImport(\"\"shell32.dll\"\",CharSet=CharSet.Unicode,PreserveSig=false)]\n",
            "  static extern void SHGetPropertyStoreFromParsingName(string p,IntPtr b,int f,[MarshalAs(UnmanagedType.LPStruct)]Guid r,[MarshalAs(UnmanagedType.Interface)]out IPS s);\n",
            "  public static void Set(string path,string appId){{\n",
            "    IPS s;SHGetPropertyStoreFromParsingName(path,IntPtr.Zero,2,new Guid(\"\"886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99\"\"),out s);\n",
            "    var k=new PK{{f=new Guid(\"\"9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3\"\"),p=5}};\n",
            "    var v=new PV{{vt=31,d=Marshal.StringToCoTaskMemUni(appId)}};s.SV(ref k,ref v);s.Co();Marshal.FreeCoTaskMem(v.d);\n",
            "  }}\n",
            "}}\n",
            "\"@;[SA]::Set($lnk,$id)",
        ),
        lnk = lnk,
        exe = INSTALL_PATH,
        id = APP_USER_MODEL_ID,
    );

    match Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(out) if out.status.success() => {
            logging::trace(format!("created start menu shortcut: {}", lnk));
        }
        Ok(out) => {
            let err = String::from_utf8_lossy(&out.stderr);
            logging::error(format!("shortcut creation failed: {}", err.trim()));
        }
        Err(e) => {
            logging::error(format!("failed to run powershell for shortcut: {}", e));
        }
    }
}

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

pub(crate) fn current_model() -> String {
    codex_config_path()
        .and_then(|path| match fs::read_to_string(&path) {
            Ok(contents) => Some(contents),
            Err(error) => {
                logging::error(format!(
                    "failed to read codex config {}: {}",
                    path.display(),
                    error
                ));
                None
            }
        })
        .and_then(|contents| parse_model(&contents))
        .unwrap_or_else(|| DEFAULT_MODEL.to_owned())
}

pub(crate) fn set_model(model: &str) -> io::Result<String> {
    ensure_codex_files()?;
    let Some(path) = codex_config_path() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "USERPROFILE not set",
        ));
    };
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    fs::write(&path, replace_model(&contents, model))?;
    Ok(model.to_owned())
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

fn codex_config_path() -> Option<PathBuf> {
    env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|path| path.join(".codex").join("config.toml"))
}

fn parse_model(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim() != "model" {
            return None;
        }
        let value = value.trim();
        if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
            return None;
        }
        Some(value[1..value.len() - 1].to_owned())
    })
}

fn replace_model(contents: &str, model: &str) -> String {
    let mut updated = String::with_capacity(contents.len().max(model.len() + 16));
    let mut found = false;
    let replacement = format!("model = {}", quoted_value(model));

    for raw_line in contents.split_inclusive('\n') {
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let is_model = line
            .split_once('=')
            .is_some_and(|(key, _)| key.trim() == "model");
        if is_model {
            if !found {
                updated.push_str(&replacement);
                updated.push('\n');
                found = true;
            }
            continue;
        }
        updated.push_str(line);
        updated.push('\n');
    }

    if !found {
        updated.push_str(&replacement);
        updated.push('\n');
    }

    updated
}

fn quoted_value(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::{parse_model, replace_model};

    #[test]
    fn replace_model_updates_existing_entry() {
        let contents = "approval_policy = \"never\"\nmodel = \"gpt-5\"\n";
        assert_eq!(
            replace_model(contents, "gpt-5.2-codex"),
            "approval_policy = \"never\"\nmodel = \"gpt-5.2-codex\"\n"
        );
    }

    #[test]
    fn replace_model_appends_missing_entry() {
        let contents = "approval_policy = \"never\"\n";
        assert_eq!(
            replace_model(contents, "gpt-5.2-codex"),
            "approval_policy = \"never\"\nmodel = \"gpt-5.2-codex\"\n"
        );
    }

    #[test]
    fn parse_model_reads_configured_value() {
        let contents = "approval_policy = \"never\"\nmodel = \"gpt-5.2-codex\"\n";
        assert_eq!(parse_model(contents).as_deref(), Some("gpt-5.2-codex"));
    }
}
