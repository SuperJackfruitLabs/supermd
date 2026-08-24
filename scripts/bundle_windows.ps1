# Build supermd-windows-x64.zip and SuperMD-Setup-<ver>.exe into dist/.
# Usage: pwsh scripts/bundle_windows.ps1 [-Version <ver>]
param([string]$Version = "0.0.0-dev")
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

cargo build --release
if ($LASTEXITCODE -ne 0) { exit 1 }

New-Item -ItemType Directory -Force -Path dist | Out-Null
Compress-Archive -Force -Path target/release/supermd.exe `
    -DestinationPath "dist/supermd-windows-x64.zip"

iscc /DAppVersion=$Version scripts/windows/supermd.iss
if ($LASTEXITCODE -ne 0) { exit 1 }

Get-ChildItem dist | Format-Table Name, Length
