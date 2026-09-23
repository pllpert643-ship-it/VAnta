$ErrorActionPreference = 'Stop'

$ProjectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $ProjectRoot
try {
    uv sync --extra dev
    uv run pyinstaller `
        --noconfirm `
        --clean `
        --windowed `
        --name VAnta `
        --paths src `
        --collect-submodules pynvml `
        src/vanta/__main__.py
    Write-Host "VAnta is ready at dist\VAnta\VAnta.exe"
}
finally {
    Pop-Location
}
