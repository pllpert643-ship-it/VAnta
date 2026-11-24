param([switch]$SkipTests, [switch]$Store)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$native = Join-Path $repo 'native'
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
if (-not (Test-Path -LiteralPath $cargo)) { throw 'Install Rust with rustup, then run this script again.' }
$previousRustFlags = $env:RUSTFLAGS
Push-Location $native
try {
    $featureArgs = @()
    if ($Store) {
        $featureArgs = @('--features', 'store')
        $cargoDetails = (& $cargo -vV) -join "`n"
        if ($cargoDetails -match 'host: x86_64-pc-windows-msvc') {
            $env:RUSTFLAGS = ($previousRustFlags + ' -C target-feature=+crt-static').Trim()
        }
    }
    if (-not $SkipTests) {
        & $cargo test --locked -j 4 @featureArgs
        if ($LASTEXITCODE -ne 0) { throw 'Native tests failed.' }
    }
    & $cargo build --release --locked -j 4 @featureArgs
    if ($LASTEXITCODE -ne 0) { throw 'Native build failed.' }
    $folderName = if ($Store) { 'VAnta-Store' } else { 'VAnta-Native' }
    $output = Join-Path $repo ('dist\' + $folderName)
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $native 'target\release\VAntaNative.exe') -Destination $output -Force
    Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination $output -Force
    Copy-Item -LiteralPath (Join-Path $repo 'docs\NATIVE_USAGE.md') -Destination (Join-Path $output 'README.md') -Force
    Copy-Item -LiteralPath (Join-Path $repo 'docs\PRIVACY.md') -Destination $output -Force
    $metadataJson = (& $cargo metadata --locked --format-version 1) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw 'Could not collect dependency license metadata.' }
    if ($PSVersionTable.PSVersion.Major -ge 7) {
        $metadata = ConvertFrom-Json -InputObject $metadataJson -AsHashtable
    } else {
        # Cargo features can contain case-distinct keys (USB/usb), which PS5's
        # ConvertFrom-Json cannot represent. Keep case-sensitive dictionaries.
        Add-Type -AssemblyName System.Web.Extensions
        $jsonReader = New-Object System.Web.Script.Serialization.JavaScriptSerializer
        $jsonReader.MaxJsonLength = [int]::MaxValue
        $metadata = $jsonReader.DeserializeObject($metadataJson)
    }
    $licenses = Join-Path $output 'licenses'
    New-Item -ItemType Directory -Path $licenses -Force | Out-Null
    $notice = @('# Third-party dependencies', '', 'Cargo dependency metadata. Includes transitive and platform-specific packages; not all packages are linked into this Windows executable.', '')
    foreach ($package in ($metadata.packages | Sort-Object name,version)) {
        $notice += ('- {0} {1}: {2}; source {3}' -f $package.name,$package.version,$package.license,$package.repository)
        $sourceDirectory = Split-Path -Parent $package.manifest_path
        $destination = Join-Path $licenses ($package.name + '-' + $package.version)
        foreach ($file in (Get-ChildItem -LiteralPath $sourceDirectory -File -Recurse | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|OFL|UFL)' })) {
            $relative = $file.FullName.Substring($sourceDirectory.Length).TrimStart([char[]]@('\', '/'))
            $target = Join-Path $destination $relative
            New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
            Copy-Item -LiteralPath $file.FullName -Destination $target -Force
        }
    }
    $notice | Set-Content -LiteralPath (Join-Path $output 'THIRD_PARTY_NOTICES.md') -Encoding utf8
    # GNU builds can import MinGW runtime DLLs. Copy only runtime files the binary imports.
    $objdump = Get-Command objdump -ErrorAction SilentlyContinue
    if ($objdump) {
        $imports = & $objdump.Source -p (Join-Path $output 'VAntaNative.exe')
        foreach ($dll in @('libgcc_s_seh-1.dll', 'libstdc++-6.dll', 'libwinpthread-1.dll')) {
            if ($imports -match [regex]::Escape($dll)) {
                $runtimePath = Join-Path (Split-Path -Parent $objdump.Source) $dll
                if (-not (Test-Path -LiteralPath $runtimePath)) { throw "Required GNU runtime missing: $dll" }
                Copy-Item -LiteralPath $runtimePath -Destination $output -Force
            }
        }
    }
    # Crate archives can carry Unix-epoch timestamps, outside ZIP's 1980+ range.
    foreach ($artifact in (Get-ChildItem -LiteralPath $output -File -Recurse)) {
        if ($artifact.LastWriteTimeUtc.Year -lt 1980) { $artifact.LastWriteTimeUtc = [datetime]'1980-01-01T00:00:00Z' }
    }
    $archiveName = if ($Store) { 'VAnta-store-windows-x64.zip' } else { 'VAnta-native-windows-x64.zip' }
    $archive = Join-Path $repo ('dist\' + $archiveName)
    Compress-Archive -LiteralPath $output -DestinationPath $archive -Force
    Write-Output "Built native application: $output"
    Write-Output "Archive: $archive"
} finally {
    $env:RUSTFLAGS = $previousRustFlags
    Pop-Location
}
