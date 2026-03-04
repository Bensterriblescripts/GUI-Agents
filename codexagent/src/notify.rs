use std::ffi::c_void;
use std::mem;
use std::ptr;
use std::thread;
use std::time::Duration;

use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO, NIIF_NOSOUND, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, IDI_APPLICATION, LoadIconW,
};

use crate::logging;

type HWND = *mut c_void;

const APP_ICON_RESOURCE_ID: u16 = 1;
const NOTIFY_ICON_ID: u32 = 1;

pub(crate) fn try_capture_hwnd(hwnd: &mut HWND) {
    if !(*hwnd).is_null() {
        return;
    }
    let h = unsafe { GetForegroundWindow() };
    if !h.is_null() {
        *hwnd = h;
        logging::trace(format!("captured hwnd: {h:?}"));
    }
}

pub(crate) fn prompt_completed(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    if !should_show_notification(hwnd, unsafe { GetForegroundWindow() }) {
        logging::trace("skipping prompt completion notification while window is focused");
        return;
    }
    show_balloon(hwnd);
}

pub(crate) fn cleanup(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }
    remove_tray_icon(hwnd);
}

fn show_balloon(hwnd: HWND) {
    let icon = load_app_icon();
    if icon.is_null() {
        logging::error("LoadIconW returned a null icon handle");
    }
    let mut data: NOTIFYICONDATAW = unsafe { mem::zeroed() };
    data.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = NOTIFY_ICON_ID;
    data.uFlags = NIF_ICON | NIF_TIP | NIF_INFO;
    data.hIcon = icon;
    data.dwInfoFlags = NIIF_INFO | NIIF_NOSOUND;
    encode_wide_into("Codex Agent", &mut data.szTip);
    encode_wide_into("Finished", &mut data.szInfo);
    encode_wide_into("Codex Agent", &mut data.szInfoTitle);
    unsafe {
        Shell_NotifyIconW(NIM_DELETE, &data);
        if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
            logging::error("Shell_NotifyIconW failed to add prompt completion balloon");
        }
    }
    let hwnd_raw = hwnd as usize;
    thread::spawn(move || {
        let _ = logging::catch_panic("notify cleanup thread", || {
            thread::sleep(Duration::from_secs(10));
            remove_tray_icon(hwnd_raw as HWND);
        });
    });
}

fn load_app_icon() -> *mut c_void {
    let module = unsafe { GetModuleHandleW(ptr::null()) };
    let resource = APP_ICON_RESOURCE_ID as usize as *const u16;
    let icon = unsafe { LoadIconW(module, resource) };
    if icon.is_null() {
        unsafe { LoadIconW(ptr::null_mut(), IDI_APPLICATION) }
    } else {
        icon
    }
}

fn remove_tray_icon(hwnd: HWND) {
    let mut data: NOTIFYICONDATAW = unsafe { mem::zeroed() };
    data.cbSize = mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = NOTIFY_ICON_ID;
    unsafe {
        if Shell_NotifyIconW(NIM_DELETE, &data) == 0 {
            logging::trace("Shell_NotifyIconW did not remove tray icon");
        }
    }
}

fn should_show_notification(hwnd: HWND, foreground: HWND) -> bool {
    hwnd != foreground
}

fn encode_wide_into(s: &str, dst: &mut [u16]) {
    if dst.is_empty() {
        return;
    }
    let mut len = 0usize;
    for (i, ch) in s.encode_utf16().enumerate() {
        if i + 1 >= dst.len() {
            break;
        }
        dst[i] = ch;
        len = i + 1;
    }
    dst[len] = 0;
    for slot in &mut dst[len + 1..] {
        *slot = 0;
    }
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::{encode_wide_into, should_show_notification};

    #[test]
    fn encode_wide_into_zero_terminates_and_clears_tail() {
        let mut buffer = [42u16; 8];
        encode_wide_into("abc", &mut buffer);
        assert_eq!(buffer[..4], [97, 98, 99, 0]);
        assert!(buffer[4..].iter().all(|&value| value == 0));
    }

    #[test]
    fn should_show_notification_only_when_window_is_not_foreground() {
        let hwnd = ptr::dangling_mut::<std::ffi::c_void>();
        let other = ptr::with_exposed_provenance_mut::<std::ffi::c_void>(2);
        assert!(!should_show_notification(hwnd, hwnd));
        assert!(should_show_notification(hwnd, other));
    }
}
