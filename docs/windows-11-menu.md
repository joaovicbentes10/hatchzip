# The Windows 11 context menu

Windows 11 shows classic registry menu entries only under **Show more options**. Its modern menu takes commands from packaged apps only, so HatchZip adds a second, packaged entry next to the classic one:

| Part | What it is |
|---|---|
| `crates/explorer` | `hatchzip_explorer.dll`, a COM class implementing `IExplorerCommand`. The top-level **HatchZip** entry returns `ECF_HASSUBCOMMANDS` and lists its items through `EnumSubCommands`. |
| `packaging/windows/explorer/AppxManifest.xml` | An identity package (a sparse package, or package with external location). It holds no program. It declares the DLL as a `windows.comServer` surrogate server and binds it to each archive type with `windows.fileExplorerContextMenus`. |
| `crates/shell` | `register` writes the classic keys as before, then, on Windows 11, registers `hatchzip-explorer.msix` from the folder `hatchzip-gui.exe` is in, with that folder as its external location. `unregister` removes both. |

The submenu has **Open with HatchZip** (one archive only), **Extract files…**, **Extract to "name\\"**, **Extract here**, **Extract here (keep archive)** and **Test archive**. The folder name in **Extract to** is the one the GUI creates, from `session::default_output` and `session::locate`. With several archives selected, the entry reads **Extract to a new folder**, and each archive gets its own folder.

Each item only starts `hatchzip-gui.exe`, from the DLL's folder, with the same flags as the classic entry. Nothing is extracted, read or changed inside Explorer or its COM surrogate. Every selected file goes to one GUI process, and selecting several volumes of one split archive passes the set once. The **Remove the archive during extraction** setting applies as it does for the classic entries, since the GUI reads it.

Windows 10 keeps the classic menu only. The package's `MinVersion` is 10.0.22000.0, and `register` does not try to add it there.

## Build

```powershell
cargo build --release -p hatchzip -p hatchzip-gui -p hatchzip-explorer --features rar
# a payload folder, as release.yml stages it, plus the DLL
Copy-Item target\release\hatchzip_explorer.dll dist\payload\
packaging\windows\build-explorer-package.ps1 dist\payload 0.1.10 -Publisher 'CN=Example' -Certificate cert.pfx -Password secret
```

`build-explorer-package.ps1` needs the Windows SDK (`makeappx.exe`, `signtool.exe`). `-Publisher` must be the signing certificate's subject, exactly. The setup takes the whole payload folder, and `build-msi.ps1` adds the DLL and the package when they are there. Both installers already run `--register-shell` after installing and `--unregister-shell` before removing.

## Signing

Windows registers the package only when it is signed by a certificate the PC trusts. A release needs a certificate from a public authority, such as the one the programs are signed with, and `-Publisher` set to its subject. `release.yml` does not build or sign the package yet.

To try it on one PC, `packaging\windows\dev-certificate.ps1`, run as administrator, makes a self-signed certificate in the current user's store and trusts it under the machine's Trusted People. The Microsoft docs say the current user's Trusted People is enough; on build 26300 the package manager still refused the package with 0x800B0109 until the certificate was in the machine's. The script prints the thumbprint to sign with:

```powershell
packaging\windows\dev-certificate.ps1
packaging\windows\build-explorer-package.ps1 dist\payload 0.1.10 -Publisher 'CN=HatchZip Dev' -Thumbprint <thumbprint>
dist\payload\hatchzip-gui.exe --register-shell
```

To undo it, delete "HatchZip Dev" in `certmgr.msc` under Personal, and in `certlm.msc` under Trusted People.

`Get-AppxPackage HatchZip.ContextMenu` shows the registration. If the menu doesn't appear, restart Explorer. Registration errors are logged in Event Viewer, under Applications and Services Logs, Microsoft, Windows, AppxDeployment-Server.

## What has been verified

On Windows 11 Pro, build 26300, with a payload in a folder named with spaces and accents:

- `cargo test` runs the COM class the way Explorer does: `DllGetClassObject`, the class factory, the **HatchZip** entry and the items `EnumSubCommands` returns, with a real `IShellItemArray` of one and of two files. The titles, the hidden **Open** for several files and the GUI's arguments are checked. No archive is read.
- The DLL exports `DllGetClassObject` and `DllCanUnloadNow`. `makeappx` accepts the manifest.
- `--register-shell` writes the classic menu, then registers the package from the accented path. An unsigned package is refused with 0x800B0100, and the command fails with that message.
- Signed with the development certificate, the package registers. `CoCreateInstance` of the class starts a `dllhost.exe` that loads `hatchzip_explorer.dll` from the external location, so the DLL needs no `msix` element in a manifest of its own.
- In Explorer, right-clicking a ZIP shows **HatchZip** in the Windows 11 menu, without **Show more options**, and **Test archive** opens the GUI's test window, which tested the archive.
- `--unregister-shell` removes the package and the classic keys. The surrogate had already unloaded the DLL by then, and the DLL file could be replaced.

Not verified yet: updating or uninstalling while the surrogate still holds the DLL, the extract entries from the modern menu, and multiple selections in Explorer itself (the COM test covers two files).

## Limitations

- The labels are English, like the classic menu's.
- The package lists every extension, `.rar` and `.r00` included. A build without RAR support shows the menu on RAR files, and the GUI then refuses them.
- x64 only. ARM64 Windows has not been tried.
- Explorer's COM surrogate (`dllhost.exe`) holds the DLL for a while after the menu was shown. Updating or uninstalling in that time may ask to close the COM Surrogate, or leave the DLL behind until the next sign-in.
- A selection whose paths go past Windows' 32,767-character command line fails to start the GUI.
- Items without a path on disk, such as files inside a ZIP folder or on a phone, get no HatchZip entry.
