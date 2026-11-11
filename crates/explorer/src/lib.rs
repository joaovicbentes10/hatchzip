//! The HatchZip submenu of Explorer's Windows 11 context menu.
//!
//! Explorer creates [`EXPLORER_CLSID`] through `DllGetClassObject` in a COM
//! surrogate process, once the identity package in
//! `packaging/windows/explorer` is registered. Every command only starts
//! `hatchzip-gui.exe`, from the folder this DLL is in, with the same flags as
//! the classic menu entries in `hatchzip_shell::ACTIONS`. No archive is read
//! or changed in Explorer's processes.
//!
//! The whole crate is Windows code, so this is the one `cfg(windows)`
//! outside a platform module.
#![cfg(windows)]

use std::ffi::{OsString, c_void};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use hatchzip_core::session;
use hatchzip_shell::{ACTIONS, Action};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_FAIL, E_NOTIMPL, E_POINTER, HMODULE,
    S_FALSE, S_OK,
};
use windows::Win32::System::Com::{CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    GetModuleFileNameW, GetModuleHandleExW,
};
use windows::Win32::UI::Shell::{
    ECF_DEFAULT, ECF_HASSUBCOMMANDS, ECS_ENABLED, ECS_HIDDEN, IEnumExplorerCommand,
    IEnumExplorerCommand_Impl, IExplorerCommand, IExplorerCommand_Impl, IShellItemArray, SHStrDupW,
    SIGDN_FILESYSPATH,
};
use windows_core::{
    BOOL, GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement,
};

pub const CLSID: GUID = GUID::from_u128(0x77539ABD_F880_4E8C_BED0_5047FE4CE5A3);

/// The submenu, in WinRAR's and 7-Zip's order. Every classic entry has a
/// place here; a test checks none is missing.
const ORDER: &[&str] = &[
    "extract-files",
    "extract-to",
    "extract-here",
    "extract-here-keep",
    "test",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    /// The main window, for a single archive.
    Open,
    Run(&'static Action),
}

fn entries() -> Vec<Entry> {
    let mut all = vec![Entry::Open];
    all.extend(
        ORDER
            .iter()
            .filter_map(|id| ACTIONS.iter().find(|a| a.id == *id))
            .map(Entry::Run),
    );
    all
}

fn title(entry: Entry, archives: &[PathBuf]) -> String {
    match entry {
        Entry::Open => "Open with HatchZip".into(),
        // The folder the GUI will create, as WinRAR names its entry.
        Entry::Run(a) if a.id == "extract-to" && archives.len() == 1 => {
            let primary =
                session::locate(&archives[0]).map_or_else(|_| archives[0].clone(), |p| p.0);
            let folder = session::default_output(&primary);
            format!(
                "Extract to \"{}\\\"",
                folder.file_name().unwrap_or_default().to_string_lossy()
            )
        }
        Entry::Run(a) => a.label.into(),
    }
}

fn visible(entry: Entry, archives: &[PathBuf]) -> bool {
    match entry {
        Entry::Open => archives.len() == 1,
        Entry::Run(_) => !archives.is_empty(),
    }
}

/// The GUI's arguments. Every volume of a split archive opens the whole set,
/// so selecting several volumes of one set extracts it once.
fn arguments(entry: Entry, archives: &[PathBuf]) -> Vec<OsString> {
    let mut sets: Vec<PathBuf> = Vec::new();
    for a in archives {
        let primary = session::locate(a).map_or_else(|_| a.clone(), |p| p.0);
        if !sets.contains(&primary) {
            sets.push(primary);
        }
    }
    let flags = match entry {
        Entry::Open => "",
        Entry::Run(a) => a.flag,
    };
    flags
        .split_whitespace()
        .map(OsString::from)
        .chain(sets.into_iter().map(OsString::from))
        .collect()
}

/// `hatchzip-gui.exe`, next to this DLL.
fn gui() -> Option<PathBuf> {
    let mut module = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(gui as *const u16),
            &mut module,
        )
    }
    .ok()?;
    let mut buf = vec![0u16; 32_768];
    let n = unsafe { GetModuleFileNameW(Some(module), &mut buf) } as usize;
    (n > 0)
        .then(|| PathBuf::from(OsString::from_wide(&buf[..n])).with_file_name("hatchzip-gui.exe"))
}

/// The selected files' paths, or `None` when one has no path on disk (an
/// item inside a ZIP folder or a phone), which HatchZip can't open.
fn selection(items: Ref<IShellItemArray>) -> Option<Vec<PathBuf>> {
    let items = items.as_ref()?;
    let count = unsafe { items.GetCount() }.ok()?;
    (0..count)
        .map(|i| {
            let name = unsafe { items.GetItemAt(i)?.GetDisplayName(SIGDN_FILESYSPATH)? };
            let path = PathBuf::from(OsString::from_wide(unsafe { name.as_wide() }));
            unsafe { CoTaskMemFree(Some(name.0 as *const c_void)) };
            Ok(path)
        })
        .collect::<Result<_>>()
        .ok()
}

fn to_pwstr(s: &str) -> Result<PWSTR> {
    let wide: Vec<u16> = s.encode_utf16().chain([0]).collect();
    unsafe { SHStrDupW(PCWSTR(wide.as_ptr())) }
}

/// Keeps the DLL loaded while any object it made is alive.
static LIVE: AtomicUsize = AtomicUsize::new(0);

struct Live;

impl Live {
    fn new() -> Self {
        LIVE.fetch_add(1, Ordering::SeqCst);
        Live
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The "HatchZip" entry when `entry` is `None`, one of its items otherwise.
#[implement(IExplorerCommand)]
struct Command {
    entry: Option<Entry>,
    _live: Live,
}

impl Command {
    fn new(entry: Option<Entry>) -> Self {
        Command {
            entry,
            _live: Live::new(),
        }
    }
}

impl IExplorerCommand_Impl for Command_Impl {
    fn GetTitle(&self, items: Ref<IShellItemArray>) -> Result<PWSTR> {
        match self.entry {
            None => to_pwstr("HatchZip"),
            Some(e) => to_pwstr(&title(e, &selection(items).unwrap_or_default())),
        }
    }

    fn GetIcon(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        match (self.entry, gui()) {
            (None, Some(gui)) => to_pwstr(&format!("{},0", gui.display())),
            _ => Err(E_NOTIMPL.into()),
        }
    }

    fn GetToolTip(&self, _items: Ref<IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(GUID::zeroed())
    }

    fn GetState(&self, items: Ref<IShellItemArray>, _slow_ok: BOOL) -> Result<u32> {
        let archives = selection(items).unwrap_or_default();
        let shown = match self.entry {
            None => !archives.is_empty(),
            Some(e) => visible(e, &archives),
        };
        Ok(if shown {
            ECS_ENABLED.0 as u32
        } else {
            ECS_HIDDEN.0 as u32
        })
    }

    fn Invoke(&self, items: Ref<IShellItemArray>, _bind: Ref<IBindCtx>) -> Result<()> {
        let (Some(entry), Some(archives), Some(gui)) = (self.entry, selection(items), gui()) else {
            return Err(E_FAIL.into());
        };
        std::process::Command::new(gui)
            .args(arguments(entry, &archives))
            .spawn()
            .map(drop)
            .map_err(|_| E_FAIL.into())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(match self.entry {
            None => ECF_HASSUBCOMMANDS.0 as u32,
            Some(_) => ECF_DEFAULT.0 as u32,
        })
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        match self.entry {
            None => Ok(Commands::new(0).into()),
            Some(_) => Err(E_NOTIMPL.into()),
        }
    }
}

#[implement(IEnumExplorerCommand)]
struct Commands {
    next: AtomicUsize,
    _live: Live,
}

impl Commands {
    fn new(next: usize) -> Self {
        Commands {
            next: AtomicUsize::new(next),
            _live: Live::new(),
        }
    }
}

impl IEnumExplorerCommand_Impl for Commands_Impl {
    fn Next(&self, wanted: u32, out: *mut Option<IExplorerCommand>, fetched: *mut u32) -> HRESULT {
        if out.is_null() {
            return E_POINTER;
        }
        let all = entries();
        let mut given = 0;
        while given < wanted as usize {
            let i = self.next.load(Ordering::SeqCst);
            let Some(&e) = all.get(i) else { break };
            self.next.store(i + 1, Ordering::SeqCst);
            unsafe { out.add(given).write(Some(Command::new(Some(e)).into())) };
            given += 1;
        }
        if !fetched.is_null() {
            unsafe { fetched.write(given as u32) };
        }
        if given == wanted as usize {
            S_OK
        } else {
            S_FALSE
        }
    }

    fn Skip(&self, count: u32) -> Result<()> {
        self.next.fetch_add(count as usize, Ordering::SeqCst);
        Ok(())
    }

    fn Reset(&self) -> Result<()> {
        self.next.store(0, Ordering::SeqCst);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumExplorerCommand> {
        Ok(Commands::new(self.next.load(Ordering::SeqCst)).into())
    }
}

#[implement(IClassFactory)]
struct Factory {
    _live: Live,
}

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if object.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { object.write(std::ptr::null_mut()) };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let command: IExplorerCommand = Command::new(None).into();
        unsafe { command.query(iid, object) }.ok()
    }

    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            std::mem::forget(Live::new());
        } else {
            LIVE.fetch_sub(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

/// # Safety
/// Called by COM with valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    unsafe { out.write(std::ptr::null_mut()) };
    if unsafe { *clsid } != CLSID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = Factory { _live: Live::new() }.into();
    unsafe { factory.query(iid, out) }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if LIVE.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

#[cfg(test)]
mod tests;
