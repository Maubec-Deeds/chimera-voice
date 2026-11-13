# Build Chimera Voice and install it where Bitwig looks for CLAP plugins.
$ErrorActionPreference = "Stop"
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Set-Location $PSScriptRoot
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "build failed" }
$dest = Join-Path $env:LOCALAPPDATA "Programs\Common\CLAP"
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item "target\release\chimera_voice.dll" (Join-Path $dest "Chimera Voice.clap") -Force
Write-Host "Installed to $dest\Chimera Voice.clap"
