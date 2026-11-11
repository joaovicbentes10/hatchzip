# Makes a self-signed certificate for trying the Windows 11 menu on this PC,
# and trusts it on this PC. Never ship a package signed with it: other PCs
# don't trust it, and they shouldn't. Run it as administrator: the package
# manager only accepts certificates under the machine's Trusted People,
# whatever the Microsoft docs say about the current user's (tried on build
# 26300: 0x800B0109).
#
#   packaging\windows\dev-certificate.ps1
#
# The key stays in the current user's certificate store, so there is no key
# file to keep safe. Build the package with the thumbprint it prints:
#
#   packaging\windows\build-explorer-package.ps1 dist\payload 0.1.10 -Publisher 'CN=HatchZip Dev' -Thumbprint <thumbprint>
#
# To undo, delete "HatchZip Dev" in certmgr.msc under Personal, and in
# certlm.msc under Trusted People.

param([string] $Subject = 'CN=HatchZip Dev')

$ErrorActionPreference = 'Stop'
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).
    IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $admin) { throw 'run this as administrator: the certificate goes under the machine''s Trusted People' }
# Reuses the one an earlier run made.
$cert = Get-ChildItem Cert:\CurrentUser\My | Where-Object Subject -eq $Subject | Select-Object -First 1
if (-not $cert) {
    # Code signing usage, and no basic constraints, as MSIX signing requires.
    $cert = New-SelfSignedCertificate -Type Custom -Subject $Subject -KeyUsage DigitalSignature `
        -FriendlyName 'HatchZip Dev' -CertStoreLocation Cert:\CurrentUser\My `
        -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}')
}
$cer = Join-Path ([IO.Path]::GetTempPath()) 'hatchzip-dev.cer'
Export-Certificate -Cert $cert -FilePath $cer | Out-Null
Import-Certificate -FilePath $cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople | Out-Null
Remove-Item $cer
Write-Host "made and trusted $Subject on this PC, thumbprint $($cert.Thumbprint)"
