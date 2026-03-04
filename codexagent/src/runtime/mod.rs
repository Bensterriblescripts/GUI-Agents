use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE};
use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CreateMutexW};
use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

use crate::config::{
    APP_DISPLAY_NAME, APP_USER_MODEL_ID, CODEX_AGENTS_CONTENTS, CODEX_CONFIG_CONTENTS,
    DEFAULT_MODEL,
};
use crate::logging;

#[repr(C)]
struct PropertyKey {
    fmtid: [u8; 16],
    pid: u32,
}

// {9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}, pid 5
const PKEY_APPUSERMODEL_ID: PropertyKey = PropertyKey {
    fmtid: [
        0x55, 0x28, 0x4C, 0x9F, 0x79, 0x9F, 0x39, 0x4B, 0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5,
        0xF3,
    ],
    pid: 5,
};

#[repr(C)]
struct PropVariant {
    vt: u16,
    reserved: [u16; 3],
    data: [usize; 2],
}

#[repr(C)]
struct IPropertyStoreVtbl {
    query_interface: usize,
    add_ref: unsafe extern "system" fn(*mut IPropertyStoreRaw) -> u32,
    release: unsafe extern "system" fn(*mut IPropertyStoreRaw) -> u32,
    get_count: usize,
    get_at: usize,
    get_value: usize,
    set_value: unsafe extern "system" fn(
        *mut IPropertyStoreRaw,
        *const PropertyKey,
        *const PropVariant,
    ) -> i32,
    commit: unsafe extern "system" fn(*mut IPropertyStoreRaw) -> i32,
}

#[repr(C)]
struct IPropertyStoreRaw {
    vtbl: *const IPropertyStoreVtbl,
}

const VT_LPWSTR: u16 = 31;

unsafe extern "system" {
    fn SHGetPropertyStoreForWindow(
        hwnd: *mut std::ffi::c_void,
        riid: *const [u8; 16],
        ppv: *mut *mut IPropertyStoreRaw,
    ) -> i32;
}

// {886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99}
const IID_IPROPERTYSTORE: [u8; 16] = [
    0xEB, 0x8E, 0x6D, 0x88, 0xF2, 0x8C, 0x46, 0x44, 0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99,
];

pub(crate) fn set_window_app_id(hwnd: *mut std::ffi::c_void) {
    if hwnd.is_null() {
        return;
    }
    unsafe {
        let mut store: *mut IPropertyStoreRaw = std::ptr::null_mut();
        let hr = SHGetPropertyStoreForWindow(hwnd, &IID_IPROPERTYSTORE, &mut store);
        if hr != 0 || store.is_null() {
            logging::error(format!("SHGetPropertyStoreForWindow failed: 0x{:08x}", hr));
            return;
        }

        let mut wide: Vec<u16> = APP_USER_MODEL_ID
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let pv = PropVariant {
            vt: VT_LPWSTR,
            reserved: [0; 3],
            data: [wide.as_mut_ptr() as usize, 0],
        };

        let vtbl = &*(*store).vtbl;
        let hr = (vtbl.set_value)(store, &PKEY_APPUSERMODEL_ID, &pv);
        if hr != 0 {
            logging::error(format!("IPropertyStore::SetValue failed: 0x{:08x}", hr));
        }
        (vtbl.commit)(store);
        (vtbl.release)(store);
    }
}

const INSTALL_PATH: &str = r"C:\Local\Software\codexagent.exe";
const LEGACY_SHORTCUT_NAMES: &[&str] = &[];
const INSTANCE_MUTEX_NAME: &str = "Local\\CodexAgent.Instance";

#[derive(Clone, Debug, Default)]
pub(crate) struct LaunchRequest {
    pub(crate) cwd: Option<PathBuf>,
}

pub(crate) struct InstanceMutex(HANDLE);

impl Drop for InstanceMutex {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub(crate) fn acquire_instance_mutex() -> Option<InstanceMutex> {
    let name: Vec<u16> = INSTANCE_MUTEX_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return None;
    }
    let first = unsafe { GetLastError() } != ERROR_ALREADY_EXISTS;
    if first {
        clear_codex_state();
    }
    Some(InstanceMutex(handle))
}

const CODEX_STATE_FILES: &[&str] = &[
    "models_cache.json",
    "state_5.sqlite",
    "state_5.sqlite-shm",
    "state_5.sqlite-wal",
];

fn clear_codex_state() {
    let Some(home) = env::var_os("USERPROFILE") else {
        return;
    };
    let codex_dir = PathBuf::from(home).join(".codex");

    let sessions_dir = codex_dir.join("sessions");
    if sessions_dir.exists() {
        if let Ok(entries) = fs::read_dir(&sessions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    for name in CODEX_STATE_FILES {
        let _ = fs::remove_file(codex_dir.join(name));
    }

    logging::trace("cleared codex state");
}

pub(crate) fn apply_launch_request(request: &LaunchRequest) {
    if let Some(path) = request.cwd.as_deref() {
        set_process_cwd(path);
    }
}

fn set_process_cwd(path: &Path) {
    match env::set_current_dir(path) {
        Ok(()) => logging::trace(format!("set working directory to {}", path.display())),
        Err(error) => logging::error(format!(
            "failed to set working directory to {}: {}",
            path.display(),
            error
        )),
    }
}

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

    let exe_path = env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| INSTALL_PATH.to_string());
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
        exe = exe_path,
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
        logging::log_result(fs::create_dir_all(&codex_dir), |error| {
            format!(
                "failed to create codex directory {}: {}",
                codex_dir.display(),
                error
            )
        })?;
    }

    logging::log_result(
        write_file_if_missing(&config_path, CODEX_CONFIG_CONTENTS),
        |error| {
            format!(
                "failed to ensure codex config {}: {}",
                config_path.display(),
                error
            )
        },
    )?;
    logging::log_result(
        write_file_if_missing(&agents_path, CODEX_AGENTS_CONTENTS),
        |error| {
            format!(
                "failed to ensure codex agents file {}: {}",
                agents_path.display(),
                error
            )
        },
    )?;

    logging::trace(format!("codex files ready in {}", codex_dir.display()));
    Ok(())
}

pub(crate) fn current_cwd_text() -> String {
    match env::current_dir() {
        Ok(path) => path.display().to_string(),
        Err(error) => {
            logging::error(format!(
                "failed to read current working directory: {}",
                error
            ));
            String::new()
        }
    }
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
    logging::log_result(ensure_codex_files(), |error| {
        format!(
            "failed to prepare codex files before setting model: {}",
            error
        )
    })?;
    let Some(path) = codex_config_path() else {
        logging::error("failed to set model: USERPROFILE not set");
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "USERPROFILE not set",
        ));
    };
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            logging::error(format!(
                "failed to read codex config {} while setting model: {}",
                path.display(),
                error
            ));
            return Err(error);
        }
    };
    logging::log_result(fs::write(&path, replace_model(&contents, model)), |error| {
        format!(
            "failed to write codex config {} while setting model: {}",
            path.display(),
            error
        )
    })?;
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
