use std::path::PathBuf;

use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{SHCreateShellItemArrayFromIDLists, SHParseDisplayName};
use windows_core::HSTRING;

use super::*;

#[test]
fn the_dll_and_the_manifest_share_one_clsid() {
    assert_eq!(
        format!("{CLSID:?}").to_uppercase(),
        hatchzip_shell::EXPLORER_CLSID
    );
}

#[test]
fn every_classic_entry_is_in_the_submenu() {
    for a in ACTIONS {
        assert!(ORDER.contains(&a.id), "{} is missing from ORDER", a.id);
    }
    assert_eq!(entries().len(), ACTIONS.len() + 1);
}

/// Empty files standing in for archives: nothing here reads them.
fn folder(names: &[&str]) -> (tempfile::TempDir, Vec<PathBuf>) {
    let dir = tempfile::Builder::new()
        .prefix("Jogos de ação ")
        .tempdir()
        .unwrap();
    let paths = names
        .iter()
        .map(|n| {
            let p = dir.path().join(n);
            std::fs::write(&p, b"").unwrap();
            p
        })
        .collect();
    (dir, paths)
}

fn action(id: &str) -> Entry {
    Entry::Run(ACTIONS.iter().find(|a| a.id == id).unwrap())
}

#[test]
fn extract_to_names_the_folder_the_gui_creates() {
    let (_dir, p) = folder(&["Meu Jogo.7z.001", "Meu Jogo.7z.002", "pacote.tar.gz"]);
    let to = action("extract-to");
    assert_eq!(title(to, &p[..1]), "Extract to \"Meu Jogo\\\"");
    assert_eq!(title(to, &p[1..2]), "Extract to \"Meu Jogo\\\"");
    assert_eq!(title(to, &p[2..]), "Extract to \"pacote\\\"");
    // Several archives each get their own folder.
    assert_eq!(title(to, &p), "Extract to a new folder");
    assert_eq!(title(Entry::Open, &p[..1]), "Open with HatchZip");
}

#[test]
fn open_is_for_one_archive_only() {
    let (_dir, p) = folder(&["a.zip", "b.zip"]);
    assert!(visible(Entry::Open, &p[..1]));
    assert!(!visible(Entry::Open, &p));
    assert!(visible(action("extract-here"), &p));
    assert!(!visible(action("extract-here"), &[]));
}

#[test]
fn the_gui_gets_the_classic_flags_and_each_set_once() {
    let (_dir, p) = folder(&["Meu Jogo.7z.001", "Meu Jogo.7z.002", "Ação & cia.zip"]);
    let args = arguments(action("extract-here-keep"), &p);
    let first = std::path::absolute(&p[0]).unwrap();
    assert_eq!(
        args,
        [
            OsString::from("--keep-archive"),
            "--extract-here".into(),
            first.into(),
            p[2].clone().into(),
        ]
    );
    assert_eq!(
        arguments(Entry::Open, &p[2..]),
        [OsString::from(p[2].clone())]
    );
    assert_eq!(arguments(action("test"), &p[2..])[0], "--test");
}

/// The selection as Explorer hands it over.
fn shell_items(paths: &[PathBuf]) -> IShellItemArray {
    let mut ids = Vec::new();
    for p in paths {
        let mut id: *mut ITEMIDLIST = std::ptr::null_mut();
        unsafe { SHParseDisplayName(&HSTRING::from(p.as_os_str()), None, &mut id, 0, None) }
            .unwrap();
        ids.push(id as *const ITEMIDLIST);
    }
    let items = unsafe { SHCreateShellItemArrayFromIDLists(&ids) }.unwrap();
    for id in ids {
        unsafe { CoTaskMemFree(Some(id as *const c_void)) };
    }
    items
}

fn take(s: PWSTR) -> String {
    let text = unsafe { s.to_string() }.unwrap();
    unsafe { CoTaskMemFree(Some(s.0 as *const c_void)) };
    text
}

/// Through COM, as Explorer does: the class factory, the "HatchZip" entry
/// and its submenu, for one archive and for several.
#[test]
fn explorer_sees_the_submenu() {
    let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let (_dir, p) = folder(&["Meu Jogo.7z.001", "outro.zip"]);

    let mut factory: Option<IClassFactory> = None;
    let hr = unsafe {
        DllGetClassObject(
            &CLSID,
            &IClassFactory::IID,
            &mut factory as *mut _ as *mut *mut c_void,
        )
    };
    assert_eq!(hr, S_OK);
    let factory = factory.unwrap();
    let top: IExplorerCommand = unsafe { factory.CreateInstance(None) }.unwrap();
    let wrong = GUID::from_u128(1);
    let mut none: *mut c_void = std::ptr::null_mut();
    assert_eq!(
        unsafe { DllGetClassObject(&wrong, &IClassFactory::IID, &mut none) },
        CLASS_E_CLASSNOTAVAILABLE
    );

    let one = shell_items(&p[..1]);
    assert_eq!(take(unsafe { top.GetTitle(&one) }.unwrap()), "HatchZip");
    assert_eq!(
        unsafe { top.GetFlags() }.unwrap(),
        ECF_HASSUBCOMMANDS.0 as u32
    );
    assert_eq!(
        unsafe { top.GetState(&one, false) }.unwrap(),
        ECS_ENABLED.0 as u32
    );

    let menu = |items: &IShellItemArray| {
        let list = unsafe { top.EnumSubCommands() }.unwrap();
        let mut shown = Vec::new();
        loop {
            let mut next = [None];
            let mut got = 0;
            let _ = unsafe { list.Next(&mut next, Some(&mut got)) };
            let Some(cmd) = next[0].take().filter(|_| got == 1) else {
                break;
            };
            if unsafe { cmd.GetState(items, true) }.unwrap() == ECS_ENABLED.0 as u32 {
                shown.push(take(unsafe { cmd.GetTitle(items) }.unwrap()));
            }
        }
        shown
    };
    assert_eq!(
        menu(&one),
        [
            "Open with HatchZip",
            "Extract files…",
            "Extract to \"Meu Jogo\\\"",
            "Extract here",
            "Extract here (keep archive)",
            "Test archive",
        ]
    );
    assert_eq!(
        menu(&shell_items(&p)),
        [
            "Extract files…",
            "Extract to a new folder",
            "Extract here",
            "Extract here (keep archive)",
            "Test archive",
        ]
    );

    drop((top, factory));
    assert_eq!(DllCanUnloadNow(), S_OK);
}
