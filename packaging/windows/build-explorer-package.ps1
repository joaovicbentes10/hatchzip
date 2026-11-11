# Builds hatchzip-explorer.msix, the identity package behind the Windows 11
# context menu, into a payload folder that already holds hatchzip-gui.exe and
# hatchzip_explorer.dll (cargo build --release -p hatchzip-gui -p
# hatchzip-explorer). The setup and the MSI then install it, and
# `hatchzip-gui --register-shell` registers it with that folder as its
# external location.
#
#   packaging\windows\build-explorer-package.ps1 dist\payload 0.1.10 -Publisher 'CN=Example' -Certificate cert.pfx -Password secret
#
# or, with a certificate in the current user's store, such as the one
# dev-certificate.ps1 makes, -Thumbprint instead of -Certificate.
#
# Windows only registers the package once it is signed by a certificate it
# trusts, and -Publisher must be that certificate's subject, exactly. With
# neither, the package is left unsigned, for a signing service.

param(
    [Parameter(Mandatory)] [string] $Payload,
    [Parameter(Mandatory)] [string] $Version,
    [Parameter(Mandatory)] [string] $Publisher,
    [string] $Certificate,
    [string] $Password,
    [string] $Thumbprint
)

$ErrorActionPreference = 'Stop'
foreach ($f in 'hatchzip-gui.exe', 'hatchzip_explorer.dll') {
    if (-not (Test-Path (Join-Path $Payload $f))) { throw "$f is missing from $Payload" }
}

# The newest Windows SDK's tools.
$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$sdk = Get-ChildItem $kits -Directory -Filter '10.*' -ErrorAction SilentlyContinue |
    Where-Object { Test-Path (Join-Path $_.FullName 'x64\makeappx.exe') } |
    Sort-Object { [version]$_.Name } | Select-Object -Last 1
if (-not $sdk) { throw "no Windows SDK with makeappx.exe under $kits; install the Windows 11 SDK" }
$tools = Join-Path $sdk.FullName 'x64'

# Package versions are four numbers; 0.2.0-beta.1 becomes 0.2.0.0.
$numbers = @(($Version -split '[-+]')[0].Split('.'))
if ($numbers.Count -ne 3 -or ($numbers | Where-Object { $_ -notmatch '^\d+$' })) { throw "$Version is not a version like 0.1.0" }
$packageVersion = ($numbers + '0') -join '.'

$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$staging = Join-Path ([IO.Path]::GetTempPath()) 'hatchzip-explorer-package'
if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
New-Item -ItemType Directory (Join-Path $staging 'Assets') | Out-Null
$manifest = (Get-Content -Raw -Encoding utf8 (Join-Path $PSScriptRoot 'explorer\AppxManifest.xml')).
    Replace('{VERSION}', $packageVersion).
    Replace('{PUBLISHER}', [Security.SecurityElement]::Escape($Publisher))
[IO.File]::WriteAllText((Join-Path $staging 'AppxManifest.xml'), $manifest)
Copy-Item (Join-Path $root 'assets\icon\png\hatchzip-256.png') (Join-Path $staging 'Assets\logo.png')

$out = Join-Path (Resolve-Path $Payload).Path 'hatchzip-explorer.msix'
# /nv: the DLL and hatchzip-gui.exe the manifest names live in the external
# location, not in the package.
& (Join-Path $tools 'makeappx.exe') pack /o /nv /d $staging /p $out | Out-Host
if ($LASTEXITCODE) { throw "makeappx exited with $LASTEXITCODE" }

if ($Certificate -or $Thumbprint) {
    $arguments = if ($Thumbprint) { @('sign', '/fd', 'SHA256', '/sha1', $Thumbprint) }
        else { @('sign', '/fd', 'SHA256', '/f', (Resolve-Path $Certificate).Path) }
    if ($Password) { $arguments += '/p', $Password }
    & (Join-Path $tools 'signtool.exe') @arguments $out | Out-Host
    if ($LASTEXITCODE) { throw "signtool exited with $LASTEXITCODE" }
}
Write-Host "built $out, version $packageVersion, publisher $Publisher$(if (-not ($Certificate -or $Thumbprint)) { ', unsigned' })"
