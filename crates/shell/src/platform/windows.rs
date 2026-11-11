//! Explorer's classic context menu, through per-user registry keys under
//! `HKCU\Software\Classes\SystemFileAssociations`. They add a HatchZip
//! submenu to each archive type without taking over its file association.
//!
//! Windows 11 shows the classic menu under "Show more options". Its modern
//! menu only takes commands from a package, so Windows 11 also gets the
//! identity package `hatchzip-explorer.msix`, registered with the GUI's
//! folder as its external location: Explorer then loads
//! `hatchzip_explorer.dll` from that folder, and its commands start the GUI
//! with the same flags as the classic entries.

use std::path::Path;

use anyhow::{Context, Result};
use windows::Foundation::Uri;
use windows::Management::Deployment::{AddPackageOptions, DeploymentResult, PackageManager};
use windows::core::HSTRING;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use crate::{ACTIONS, ALL_EXTENSIONS, Dirs, EXPLORER_PACKAGE, EXPLORER_PACKAGE_FILE, EXTENSIONS};

const BASE: &str = r"Software\Classes\SystemFileAssociations";

/// The first Windows 11 build. Windows 10 has no modern menu, and the
/// package's MinVersion refuses it anyway.
const WINDOWS_11: u32 = 22000;

fn menu_key(ext: &str) -> String {
    format!(r"{BASE}\.{ext}\shell\HatchZip")
}

pub fn current_dirs() -> Result<Dirs> {
    Ok(Dirs {
        data: std::env::temp_dir(),
    })
}

pub fn register(_dirs: &Dirs, gui: &Path) -> Result<Vec<String>> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let exe = gui.to_string_lossy();
    let mut written = Vec::new();
    for ext in EXTENSIONS {
        let path = menu_key(ext);
        // Items are numbered in their key names, so an older registration
        // whose numbers have shifted would leave duplicates behind.
        let _ = hkcu.delete_subkey_all(&path);
        let (menu, _) = hkcu
            .create_subkey(&path)
            .with_context(|| format!("creating HKCU\\{path}"))?;
        menu.set_value("MUIVerb", &"HatchZip")?;
        menu.set_value("Icon", &format!("\"{exe}\",0"))?;
        // An empty SubCommands value turns the entry into a submenu built
        // from its own `shell` key.
        menu.set_value("SubCommands", &"")?;
        for (i, a) in ACTIONS.iter().enumerate() {
            // Explorer orders submenu items by key name.
            let (item, _) = menu.create_subkey(format!(r"shell\{:02}{}", i + 1, a.id))?;
            item.set_value("MUIVerb", &a.label)?;
            let (command, _) = item.create_subkey("command")?;
            command.set_value("", &format!("\"{exe}\" {} \"%1\"", a.flag))?;
        }
        written.push(format!("Explorer: .{ext} files"));
    }
    written.push(register_package(gui)?);
    Ok(written)
}

/// Registers the identity package that sits next to `gui`, replacing any
/// earlier registration. A build without the package, such as `cargo run`,
/// keeps the classic menu only.
fn register_package(gui: &Path) -> Result<String> {
    if windows_build() < WINDOWS_11 {
        return Ok("Explorer: no Windows 11 menu on this version of Windows".into());
    }
    let dir = gui
        .parent()
        .context("finding the folder hatchzip-gui.exe is in")?;
    let msix = dir.join(EXPLORER_PACKAGE_FILE);
    if !msix.is_file() {
        return Ok(format!(
            "Explorer: Windows 11 menu skipped, {EXPLORER_PACKAGE_FILE} is not next to hatchzip-gui.exe"
        ));
    }
    // A version can't be added twice, and an older registration may point at
    // a folder the app has moved out of.
    remove_package()?;
    let add = || -> windows::core::Result<()> {
        let options = AddPackageOptions::new()?;
        options.SetExternalLocationUri(&Uri::CreateUri(&HSTRING::from(file_uri(dir)))?)?;
        deployed(
            PackageManager::new()?
                .AddPackageByUriAsync(&Uri::CreateUri(&HSTRING::from(file_uri(&msix)))?, &options)?
                .join()?,
        )
    };
    add().with_context(|| {
        format!(
            "adding the Windows 11 menu from {}. The classic menu, under \"Show more options\", \
             was added. A package signed with a certificate this PC doesn't trust is refused \
             (0x800B0109); Event Viewer, under Microsoft, Windows, AppxDeployment-Server, \
             has the details",
            msix.display()
        )
    })?;
    Ok("Explorer: Windows 11 menu".into())
}

/// Removes every registration of the identity package for this user.
fn remove_package() -> Result<()> {
    init_winrt();
    let remove = || -> windows::core::Result<()> {
        let manager = PackageManager::new()?;
        // The publisher depends on who signed the package, so this matches
        // the name alone. An empty user is the current one.
        for package in manager.FindPackagesByUserSecurityId(&HSTRING::new())? {
            let id = package.Id()?;
            if id.Name()? == EXPLORER_PACKAGE {
                deployed(manager.RemovePackageAsync(&id.FullName()?)?.join()?)?;
            }
        }
        Ok(())
    };
    remove().context("removing the Windows 11 menu package")
}

/// A deployment that fails still completes; the error is in its result.
fn deployed(result: DeploymentResult) -> windows::core::Result<()> {
    let code = result.ExtendedErrorCode()?;
    if code.is_ok() {
        return Ok(());
    }
    Err(windows::core::Error::new(
        code,
        result.ErrorText()?.to_string_lossy(),
    ))
}

/// WinRT calls need an apartment. Keeping the multithreaded one alive lets
/// them run from any thread without changing the calling thread's.
fn init_winrt() {
    let _ = unsafe { windows::Win32::System::Com::CoIncrementMTAUsage() };
}

fn windows_build() -> u32 {
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber"))
        .ok()
        .and_then(|b| b.parse().ok())
        .unwrap_or(0)
}

/// A path as the `file:` URI the package manager takes. Spaces and other
/// ASCII punctuation are percent-encoded; accented letters stay as they
/// are, since the package manager decodes `%C3%A7` as two Latin-1
/// characters instead of `ç`.
fn file_uri(path: &Path) -> String {
    let path = path.to_string_lossy();
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    let (prefix, rest) = match path.strip_prefix(r"UNC\").or(path.strip_prefix(r"\\")) {
        Some(share) => ("file://", share),
        None => ("file:///", path),
    };
    let mut uri = String::from(prefix);
    for c in rest.replace('\\', "/").chars() {
        if !c.is_ascii() || c.is_ascii_alphanumeric() || "-._~/:".contains(c) {
            uri.push(c);
        } else {
            uri.push_str(&format!("%{:02X}", c as u8));
        }
    }
    uri
}

pub fn unregister(_dirs: &Dirs) -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for ext in ALL_EXTENSIONS {
        match hkcu.delete_subkey_all(menu_key(ext)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(e).with_context(|| format!("removing HKCU\\{}", menu_key(ext)));
            }
            _ => {}
        }
    }
    remove_package()
}

pub fn is_registered(_dirs: &Dirs) -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(menu_key(EXTENSIONS[0]))
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_become_file_uris() {
        assert_eq!(
            file_uri(Path::new(r"C:\Users\João\Meus Jogos\a#b%.msix")),
            "file:///C:/Users/João/Meus%20Jogos/a%23b%25.msix"
        );
        assert_eq!(
            file_uri(Path::new(r"\\?\C:\Program Files\HatchZip")),
            "file:///C:/Program%20Files/HatchZip"
        );
        assert_eq!(
            file_uri(Path::new(r"\\server\apps\HatchZip")),
            "file://server/apps/HatchZip"
        );
        assert_eq!(
            file_uri(Path::new(r"\\?\UNC\server\apps")),
            "file://server/apps"
        );
    }
}
