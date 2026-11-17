# Build the release zip: dist\ChimeraVoice-<version>-windows-x64.zip, holding the plugin, the face
# tracker, a double-click installer and the licences. The GitHub workflow runs this for each
# version tag; it also works on this computer.
#   ./package.ps1            build, then package
#   ./package.ps1 -NoBuild   package the plugin already in target\release
param([switch]$NoBuild)
$ErrorActionPreference = "Stop"
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Set-Location $PSScriptRoot

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $NoBuild) {
    # cargo reports progress on stderr, which Windows PowerShell 5 would take for an error
    $ErrorActionPreference = "Continue"
    cargo build --release --lib 2>&1 | ForEach-Object { "$_" }
    $ErrorActionPreference = "Stop"
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
}
$dll = "target\release\chimera_voice.dll"
if (-not (Test-Path $dll)) { throw "$dll is missing: build first" }

$name = "ChimeraVoice-$version-windows-x64"
$stage = Join-Path "dist" $name
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage, "$stage\Face tracker", "$stage\Licences" | Out-Null

Copy-Item $dll "$stage\Chimera Voice.clap"
Copy-Item "packaging\Install.cmd", "packaging\Install.ps1", "packaging\READ ME FIRST.txt" $stage
foreach ($f in "facetrack.py", "requirements.txt", "run.ps1", "Start face tracker.cmd") {
    Copy-Item (Join-Path "tools\facetrack" $f) "$stage\Face tracker"
}
Copy-Item "LICENSE" "$stage\Licences\Chimera Voice (MIT).txt"
Copy-Item "vendor\chimera-vocal-synth\LICENSE" "$stage\Licences\CHIMERA engine (MIT).txt"
Copy-Item "vendor\chimera-vocal-synth\public\data\CMUDICT-LICENSE.txt" "$stage\Licences\CMU Pronouncing Dictionary.txt"

$zip = Join-Path "dist" "$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $stage -DestinationPath $zip
Write-Host "Packaged $zip"
