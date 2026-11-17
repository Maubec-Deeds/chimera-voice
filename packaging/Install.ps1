# Installs Chimera Voice from the downloaded zip: no Rust or build tools needed.
#   - the plugin goes where CLAP hosts look:   %LOCALAPPDATA%\Programs\Common\CLAP\Chimera Voice.clap
#   - the face tracker goes to:               %LOCALAPPDATA%\Programs\Chimera Voice\Face tracker
#     with a Start menu shortcut "Chimera face tracker". Its calibration and Python set-up are kept
#     when you install a newer version over it.
$ErrorActionPreference = "Stop"
$here = $PSScriptRoot

$clapDir = Join-Path $env:LOCALAPPDATA "Programs\Common\CLAP"
New-Item -ItemType Directory -Force $clapDir | Out-Null
try {
    Copy-Item (Join-Path $here "Chimera Voice.clap") (Join-Path $clapDir "Chimera Voice.clap") -Force
} catch {
    throw "Could not replace the plugin. Close Bitwig (or any host using Chimera Voice) and run this again."
}
Write-Host "Plugin installed:        $clapDir\Chimera Voice.clap"

$faceDir = Join-Path $env:LOCALAPPDATA "Programs\Chimera Voice\Face tracker"
New-Item -ItemType Directory -Force $faceDir | Out-Null
Get-ChildItem (Join-Path $here "Face tracker") -File | Copy-Item -Destination $faceDir -Force
Write-Host "Face tracker installed:  $faceDir"

$menu = Join-Path ([Environment]::GetFolderPath("Programs")) "Chimera face tracker.lnk"
$shell = New-Object -ComObject WScript.Shell
$link = $shell.CreateShortcut($menu)
$link.TargetPath = Join-Path $faceDir "Start face tracker.cmd"
$link.WorkingDirectory = $faceDir
$link.Description = "Webcam face control for Chimera Voice"
$link.Save()
Write-Host "Start menu shortcut:     Chimera face tracker"

Write-Host ""
Write-Host "Done. In Bitwig, rescan plugins (Settings > Locations > Rescan) and look for Chimera Voice."
Write-Host "The face tracker needs Python 3.12 from python.org the first time it starts."
