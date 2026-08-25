# Build supermd-windows-x64.zip and SuperMD-Setup-<ver>.exe into dist/.
# Usage: pwsh scripts/bundle_windows.ps1 [-Version <ver>]
param([string]$Version = "0.0.0-dev")
$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Set-Location $Root

cargo build --release
if ($LASTEXITCODE -ne 0) { exit 1 }

New-Item -ItemType Directory -Force -Path dist | Out-Null
# The plain zip carries the binary plus the default plugins (seeded on
# first run) when build_plugins.sh has staged them.
$zipStage = "dist/zip-staging"
if (Test-Path $zipStage) { Remove-Item -Recurse -Force $zipStage }
New-Item -ItemType Directory -Force -Path $zipStage | Out-Null
Copy-Item target/release/supermd.exe $zipStage/
if (Test-Path "dist/default-plugins") {
    Copy-Item -Recurse "dist/default-plugins" "$zipStage/plugins"
}
Compress-Archive -Force -Path "$zipStage/*" `
    -DestinationPath "dist/supermd-windows-x64.zip"
Remove-Item -Recurse -Force $zipStage

iscc /DAppVersion=$Version scripts/windows/supermd.iss
if ($LASTEXITCODE -ne 0) { exit 1 }

Get-ChildItem dist | Format-Table Name, Length
