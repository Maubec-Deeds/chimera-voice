# Start the Chimera face tracker. The first run sets up its own Python environment here
# (needs Python 3.12 from python.org; about 300 MB, a few minutes) and downloads the face model.
# When a newer version needs other packages, they are installed on its first start.
# Any arguments are passed on, e.g.  ./run.ps1 --camera 1
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot
$py = Join-Path $PSScriptRoot ".venv\Scripts\python.exe"
$stamp = Join-Path $PSScriptRoot ".venv\installed-requirements.txt"
$wanted = Get-Content -Raw (Join-Path $PSScriptRoot "requirements.txt")
if (-not (Test-Path $py)) {
    Write-Host "Setting up the tracker's Python environment (first run only)..."
    py -3.12 -m venv .venv
    if ($LASTEXITCODE -ne 0) { throw "Python 3.12 is needed: install it from python.org, then run this again." }
}
if (-not (Test-Path $stamp) -or (Get-Content -Raw $stamp) -ne $wanted) {
    Write-Host "Installing the tracker's packages..."
    & $py -m pip install --disable-pip-version-check -r requirements.txt
    if ($LASTEXITCODE -ne 0) { throw "Installing the tracker's packages failed." }
    Set-Content -NoNewline -Path $stamp -Value $wanted
}
& $py -I (Join-Path $PSScriptRoot "facetrack.py") @args
