#![cfg(windows)]
#![allow(unsafe_op_in_unsafe_fn)]

use std::ffi::{OsStr, OsString, c_void};
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use windows_sys::Win32::Foundation::{BOOL, HMODULE, MAX_PATH};
use windows_sys::Win32::System::Com::{CoTaskMemAlloc, CoTaskMemFree};
use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows_sys::Win32::UI::Shell::{ECS_ENABLED, SIGDN_FILESYSPATH, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows_sys::core::{GUID, HRESULT, PWSTR};

const DLL_PROCESS_ATTACH: u32 = 1;
const S_OK: HRESULT = 0;
const S_FALSE: HRESULT = 1;
const E_NOINTERFACE: HRESULT = 0x8000_4002u32 as i32;
const E_POINTER: HRESULT = 0x8000_4003u32 as i32;
const E_FAIL: HRESULT = 0x8000_4005u32 as i32;
const E_NOTIMPL: HRESULT = 0x8000_4001u32 as i32;
const CLASS_E_CLASSNOTAVAILABLE: HRESULT = 0x8004_0111u32 as i32;
const CLASS_E_NOAGGREGATION: HRESULT = 0x8004_0110u32 as i32;

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_ICLASS_FACTORY: GUID = GUID::from_u128(0x00000001_0000_0000_c000_000000000046);
const IID_IEXPLORER_COMMAND: GUID = GUID::from_u128(0xa08ce4d0_fa25_44ab_b57c_c7b1c323e0b9);
const CLSID_CODEX_CONTEXT_MENU: GUID = GUID::from_u128(0x8c4e113e_d4f4_4cdb_a1f9_2c6368fd8f24);
const GUID_CODEX_COMMAND: GUID = GUID::from_u128(0x17b2da11_d4be_49c7_8c52_5f0f3d45f8f7);

static DLL_MODULE: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static DLL_REF_COUNT: AtomicU32 = AtomicU32::new(0);

#[repr(C)]
struct UnknownVtbl {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

#[repr(C)]
struct ClassFactoryVtbl {
    base: UnknownVtbl,
    create_instance: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *const GUID,
        *mut *mut c_void,
    ) -> HRESULT,
    lock_server: unsafe extern "system" fn(*mut c_void, BOOL) -> HRESULT,
}

#[repr(C)]
struct ExplorerCommandVtbl {
    base: UnknownVtbl,
    get_title: unsafe extern "system" fn(*mut c_void, *mut ShellItemArray, *mut PWSTR) -> HRESULT,
    get_icon: unsafe extern "system" fn(*mut c_void, *mut ShellItemArray, *mut PWSTR) -> HRESULT,
    get_tool_tip:
        unsafe extern "system" fn(*mut c_void, *mut ShellItemArray, *mut PWSTR) -> HRESULT,
    get_canonical_name: unsafe extern "system" fn(*mut c_void, *mut GUID) -> HRESULT,
    get_state:
        unsafe extern "system" fn(*mut c_void, *mut ShellItemArray, BOOL, *mut i32) -> HRESULT,
    invoke: unsafe extern "system" fn(*mut c_void, *mut ShellItemArray, *mut c_void) -> HRESULT,
    enum_sub_commands: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct ShellItemArrayVtbl {
    base: UnknownVtbl,
    bind_to_handler: usize,
    get_property_store: usize,
    get_property_description_list: usize,
    get_attributes: usize,
    get_count: unsafe extern "system" fn(*mut ShellItemArray, *mut u32) -> HRESULT,
    get_item_at:
        unsafe extern "system" fn(*mut ShellItemArray, u32, *mut *mut ShellItem) -> HRESULT,
    enum_items: usize,
}

#[repr(C)]
struct ShellItemVtbl {
    base: UnknownVtbl,
    bind_to_handler: usize,
    get_parent: usize,
    get_display_name: unsafe extern "system" fn(*mut ShellItem, i32, *mut PWSTR) -> HRESULT,
    get_attributes: usize,
    compare: usize,
}

#[repr(C)]
struct ClassFactory {
    vtbl: *const ClassFactoryVtbl,
    refs: AtomicU32,
}

#[repr(C)]
struct ExplorerCommand {
    vtbl: *const ExplorerCommandVtbl,
    refs: AtomicU32,
}

#[repr(C)]
struct ShellItemArray {
    vtbl: *const ShellItemArrayVtbl,
}

#[repr(C)]
struct ShellItem {
    vtbl: *const ShellItemVtbl,
}

static CLASS_FACTORY_VTBL: ClassFactoryVtbl = ClassFactoryVtbl {
    base: UnknownVtbl {
        query_interface: class_factory_query_interface,
        add_ref: class_factory_add_ref,
        release: class_factory_release,
    },
    create_instance: class_factory_create_instance,
    lock_server: class_factory_lock_server,
};

static EXPLORER_COMMAND_VTBL: ExplorerCommandVtbl = ExplorerCommandVtbl {
    base: UnknownVtbl {
        query_interface: explorer_command_query_interface,
        add_ref: explorer_command_add_ref,
        release: explorer_command_release,
    },
    get_title: explorer_command_get_title,
    get_icon: explorer_command_get_icon,
    get_tool_tip: explorer_command_get_tool_tip,
    get_canonical_name: explorer_command_get_canonical_name,
    get_state: explorer_command_get_state,
    invoke: explorer_command_invoke,
    enum_sub_commands: explorer_command_enum_sub_commands,
};

#[unsafe(no_mangle)]
pub extern "system" fn DllMain(module: HMODULE, reason: u32, _: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        DLL_MODULE.store(module, Ordering::Relaxed);
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllCanUnloadNow() -> HRESULT {
    if DLL_REF_COUNT.load(Ordering::Acquire) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    if rclsid.is_null() || riid.is_null() || !guid_eq(rclsid, &CLSID_CODEX_CONTEXT_MENU) {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    if !guid_eq(riid, &IID_ICLASS_FACTORY) && !guid_eq(riid, &IID_IUNKNOWN) {
        return E_NOINTERFACE;
    }

    DLL_REF_COUNT.fetch_add(1, Ordering::AcqRel);
    let factory = Box::new(ClassFactory {
        vtbl: &CLASS_FACTORY_VTBL,
        refs: AtomicU32::new(1),
    });
    *ppv = Box::into_raw(factory) as *mut c_void;
    S_OK
}

unsafe extern "system" fn class_factory_query_interface(
    this: *mut c_void,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    if riid.is_null() {
        return E_NOINTERFACE;
    }
    if guid_eq(riid, &IID_IUNKNOWN) || guid_eq(riid, &IID_ICLASS_FACTORY) {
        class_factory_add_ref(this);
        *ppv = this;
        return S_OK;
    }
    E_NOINTERFACE
}

unsafe extern "system" fn class_factory_add_ref(this: *mut c_void) -> u32 {
    let factory = &*(this as *mut ClassFactory);
    factory.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn class_factory_release(this: *mut c_void) -> u32 {
    let factory = &*(this as *mut ClassFactory);
    let refs = factory.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if refs == 0 {
        DLL_REF_COUNT.fetch_sub(1, Ordering::AcqRel);
        drop(Box::from_raw(this as *mut ClassFactory));
    }
    refs
}

unsafe extern "system" fn class_factory_create_instance(
    _: *mut c_void,
    outer: *mut c_void,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    if !outer.is_null() {
        return CLASS_E_NOAGGREGATION;
    }
    if riid.is_null() {
        return E_NOINTERFACE;
    }
    if !guid_eq(riid, &IID_IUNKNOWN) && !guid_eq(riid, &IID_IEXPLORER_COMMAND) {
        return E_NOINTERFACE;
    }

    DLL_REF_COUNT.fetch_add(1, Ordering::AcqRel);
    let command = Box::new(ExplorerCommand {
        vtbl: &EXPLORER_COMMAND_VTBL,
        refs: AtomicU32::new(1),
    });
    *ppv = Box::into_raw(command) as *mut c_void;
    S_OK
}

unsafe extern "system" fn class_factory_lock_server(_: *mut c_void, lock: BOOL) -> HRESULT {
    if lock != 0 {
        DLL_REF_COUNT.fetch_add(1, Ordering::AcqRel);
    } else {
        DLL_REF_COUNT.fetch_sub(1, Ordering::AcqRel);
    }
    S_OK
}

unsafe extern "system" fn explorer_command_query_interface(
    this: *mut c_void,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    *ppv = null_mut();
    if riid.is_null() {
        return E_NOINTERFACE;
    }
    if guid_eq(riid, &IID_IUNKNOWN) || guid_eq(riid, &IID_IEXPLORER_COMMAND) {
        explorer_command_add_ref(this);
        *ppv = this;
        return S_OK;
    }
    E_NOINTERFACE
}

unsafe extern "system" fn explorer_command_add_ref(this: *mut c_void) -> u32 {
    let command = &*(this as *mut ExplorerCommand);
    command.refs.fetch_add(1, Ordering::AcqRel) + 1
}

unsafe extern "system" fn explorer_command_release(this: *mut c_void) -> u32 {
    let command = &*(this as *mut ExplorerCommand);
    let refs = command.refs.fetch_sub(1, Ordering::AcqRel) - 1;
    if refs == 0 {
        DLL_REF_COUNT.fetch_sub(1, Ordering::AcqRel);
        drop(Box::from_raw(this as *mut ExplorerCommand));
    }
    refs
}

unsafe extern "system" fn explorer_command_get_title(
    _: *mut c_void,
    _: *mut ShellItemArray,
    name: *mut PWSTR,
) -> HRESULT {
    alloc_out_string("Launch Codex", name)
}

unsafe extern "system" fn explorer_command_get_icon(
    _: *mut c_void,
    _: *mut ShellItemArray,
    name: *mut PWSTR,
) -> HRESULT {
    let Some(path) = codex_exe_path() else {
        if !name.is_null() {
            *name = null_mut();
        }
        return E_FAIL;
    };
    alloc_out_os_str(path.as_os_str(), name)
}

unsafe extern "system" fn explorer_command_get_tool_tip(
    _: *mut c_void,
    _: *mut ShellItemArray,
    name: *mut PWSTR,
) -> HRESULT {
    alloc_out_string("Open Codex in this folder", name)
}

unsafe extern "system" fn explorer_command_get_canonical_name(
    _: *mut c_void,
    guid: *mut GUID,
) -> HRESULT {
    if guid.is_null() {
        return E_POINTER;
    }
    *guid = GUID_CODEX_COMMAND;
    S_OK
}

unsafe extern "system" fn explorer_command_get_state(
    _: *mut c_void,
    _: *mut ShellItemArray,
    _: BOOL,
    state: *mut i32,
) -> HRESULT {
    if state.is_null() {
        return E_POINTER;
    }
    *state = ECS_ENABLED;
    S_OK
}

unsafe extern "system" fn explorer_command_invoke(
    _: *mut c_void,
    items: *mut ShellItemArray,
    _: *mut c_void,
) -> HRESULT {
    launch_codex(selected_path(items).as_deref())
}

unsafe extern "system" fn explorer_command_enum_sub_commands(
    _: *mut c_void,
    commands: *mut *mut c_void,
) -> HRESULT {
    if !commands.is_null() {
        *commands = null_mut();
    }
    E_NOTIMPL
}

unsafe fn alloc_out_string(value: &str, out: *mut PWSTR) -> HRESULT {
    alloc_out_wide(value.encode_utf16(), value.encode_utf16().count(), out)
}

unsafe fn alloc_out_os_str(value: &OsStr, out: *mut PWSTR) -> HRESULT {
    alloc_out_wide(value.encode_wide(), value.encode_wide().count(), out)
}

unsafe fn alloc_out_wide<I>(wide: I, len: usize, out: *mut PWSTR) -> HRESULT
where
    I: Iterator<Item = u16>,
{
    if out.is_null() {
        return E_POINTER;
    }
    *out = alloc_wide(wide, len);
    if (*out).is_null() { E_FAIL } else { S_OK }
}

unsafe fn alloc_wide<I>(wide: I, len: usize) -> PWSTR
where
    I: Iterator<Item = u16>,
{
    let bytes = (len + 1) * size_of::<u16>();
    let ptr = CoTaskMemAlloc(bytes) as *mut u16;
    if ptr.is_null() {
        return null_mut();
    }
    for (index, unit) in wide.enumerate() {
        *ptr.add(index) = unit;
    }
    *ptr.add(len) = 0;
    ptr
}

unsafe fn selected_path(items: *mut ShellItemArray) -> Option<String> {
    if items.is_null() {
        return None;
    }

    let mut count = 0;
    if ((*(*items).vtbl).get_count)(items, &mut count) != S_OK || count == 0 {
        return None;
    }

    let mut item: *mut ShellItem = null_mut();
    if ((*(*items).vtbl).get_item_at)(items, 0, &mut item) != S_OK || item.is_null() {
        return None;
    }

    let mut display_name: PWSTR = null_mut();
    let hr = ((*(*item).vtbl).get_display_name)(item, SIGDN_FILESYSPATH, &mut display_name);
    ((*(*item).vtbl).base.release)(item as *mut c_void);
    if hr != S_OK || display_name.is_null() {
        return None;
    }

    let value = wide_ptr_to_string(display_name);
    CoTaskMemFree(display_name as *const c_void);
    value
}

unsafe fn wide_ptr_to_string(ptr: PWSTR) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    Some(String::from_utf16_lossy(std::slice::from_raw_parts(
        ptr, len,
    )))
}

unsafe fn launch_codex(cwd: Option<&str>) -> HRESULT {
    let Some(exe_path) = codex_exe_path() else {
        return E_FAIL;
    };

    let arguments = cwd.map(|cwd| format!("--show --cwd {}", quote_arg(cwd)));
    let exe_wide = to_wide(exe_path.as_os_str().to_string_lossy().as_ref());
    let args_wide = arguments.as_deref().map(to_wide);

    let instance = ShellExecuteW(
        null_mut(),
        null(),
        exe_wide.as_ptr(),
        args_wide.as_ref().map_or(null(), |value| value.as_ptr()),
        null(),
        SW_SHOWNORMAL,
    );
    if instance as isize <= 32 {
        E_FAIL
    } else {
        S_OK
    }
}

fn codex_exe_path() -> Option<PathBuf> {
    let module = DLL_MODULE.load(Ordering::Relaxed) as HMODULE;
    if module.is_null() {
        return None;
    }

    let mut buffer = vec![0u16; MAX_PATH as usize];
    let len =
        unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if len == 0 || len >= buffer.len() {
        return None;
    }
    buffer.truncate(len);

    let mut path = PathBuf::from(OsString::from_wide(&buffer));
    path.set_file_name("codexagent.exe");
    Some(path)
}

fn guid_eq(left: *const GUID, right: &GUID) -> bool {
    if left.is_null() {
        return false;
    }
    let left = unsafe { *left };
    left.data1 == right.data1
        && left.data2 == right.data2
        && left.data3 == right.data3
        && left.data4 == right.data4
}

fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn quote_arg(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    if !value.contains([' ', '\t', '"']) {
        return value.to_owned();
    }

    let mut quoted = String::with_capacity(quoted_arg_capacity(value));
    quoted.push('"');
    let mut backslashes = 0usize;

    for ch in value.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                push_backslashes(&mut quoted, backslashes * 2 + 1);
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                if backslashes != 0 {
                    push_backslashes(&mut quoted, backslashes);
                    backslashes = 0;
                }
                quoted.push(ch);
            }
        }
    }

    if backslashes != 0 {
        push_backslashes(&mut quoted, backslashes * 2);
    }

    quoted.push('"');
    quoted
}

fn quoted_arg_capacity(value: &str) -> usize {
    let mut extra = 2usize;
    let mut backslashes = 0usize;
    for ch in value.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                extra += backslashes + 1;
                backslashes = 0;
            }
            _ => backslashes = 0,
        }
    }
    value.len() + extra + backslashes
}

fn push_backslashes(out: &mut String, count: usize) {
    out.reserve(count);
    for _ in 0..count {
        out.push('\\');
    }
}
