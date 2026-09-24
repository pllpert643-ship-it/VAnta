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
    $ArchivePath = Join-Path $ProjectRoot "dist\VAnta-windows-x64.zip"
    if (Test-Path $ArchivePath) {
        Remove-Item -LiteralPath $ArchivePath -Force
    }
    Compress-Archive -Path (Join-Path $ProjectRoot "dist\VAnta") -DestinationPath $ArchivePath
    Write-Host "VAnta is ready at dist\VAnta\VAnta.exe"
    Write-Host "Release archive is ready at dist\VAnta-windows-x64.zip"
}
finally {
    Pop-Location
}
