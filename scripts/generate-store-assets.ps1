param()
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$repo = Split-Path -Parent $PSScriptRoot
$assetRoot = Join-Path $repo 'packaging\store\Assets'
$nativeRoot = Join-Path $repo 'native\assets'
New-Item -ItemType Directory -Path $assetRoot -Force | Out-Null
$source = [System.Drawing.Image]::FromFile((Join-Path $nativeRoot 'vanta-logo.png'))
function Get-LogoPng([int]$size) {
    $canvas = New-Object System.Drawing.Bitmap $size, $size
    $graphics = [System.Drawing.Graphics]::FromImage($canvas)
    $stream = New-Object System.IO.MemoryStream
    try {
        $graphics.Clear([System.Drawing.Color]::Transparent)
        $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
        $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $ratio = [Math]::Min($size / $source.Width, $size / $source.Height)
        $width = [int][Math]::Round($source.Width * $ratio)
        $height = [int][Math]::Round($source.Height * $ratio)
        $graphics.DrawImage($source, [int](($size - $width) / 2), [int](($size - $height) / 2), $width, $height)
        $canvas.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
        return ,$stream.ToArray()
    } finally {
        $stream.Dispose(); $graphics.Dispose(); $canvas.Dispose()
    }
}
try {
    foreach ($size in @(44, 50, 150, 300)) {
        $fileName = switch ($size) { 44 { 'Square44x44Logo.png' } 50 { 'StoreLogo.png' } 150 { 'Square150x150Logo.png' } 300 { 'Square150x150Logo.scale-200.png' } }
        [System.IO.File]::WriteAllBytes((Join-Path $assetRoot $fileName), (Get-LogoPng $size))
    }
    [System.IO.File]::WriteAllBytes((Join-Path $nativeRoot 'vanta-logo-512.png'), (Get-LogoPng 512))
    [System.IO.File]::WriteAllBytes((Join-Path $nativeRoot 'vanta-icon-256.png'), (Get-LogoPng 256))
    # Windows ICO directory containing PNG images at each standard display size.
    $sizes = @(16, 24, 32, 48, 64, 128, 256)
    $frames = @($sizes | ForEach-Object { ,(Get-LogoPng $_) })
    $iconStream = New-Object System.IO.MemoryStream
    $writer = New-Object System.IO.BinaryWriter $iconStream
    try {
        $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$sizes.Count)
        $offset = 6 + 16 * $sizes.Count
        for ($index = 0; $index -lt $sizes.Count; $index++) {
            $dimension = if ($sizes[$index] -eq 256) { 0 } else { $sizes[$index] }
            $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
            $writer.Write([byte]0); $writer.Write([byte]0)
            $writer.Write([uint16]1); $writer.Write([uint16]32)
            $writer.Write([uint32]$frames[$index].Length); $writer.Write([uint32]$offset)
            $offset += $frames[$index].Length
        }
        foreach ($frame in $frames) { $writer.Write([byte[]]$frame) }
        $writer.Flush()
        [System.IO.File]::WriteAllBytes((Join-Path $nativeRoot 'vanta.ico'), $iconStream.ToArray())
    } finally { $writer.Dispose(); $iconStream.Dispose() }
} finally { $source.Dispose() }
