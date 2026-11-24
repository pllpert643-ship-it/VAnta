param()
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$assetRoot = Join-Path (Split-Path -Parent $PSScriptRoot) 'packaging\store\Assets'
New-Item -ItemType Directory -Path $assetRoot -Force | Out-Null
foreach ($size in @(44, 50, 150, 300)) {
    $scale = 4
    $canvas = New-Object System.Drawing.Bitmap ($size * $scale), ($size * $scale)
    $graphics = [System.Drawing.Graphics]::FromImage($canvas)
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $graphics.Clear([System.Drawing.Color]::FromArgb(17,20,30))
    $pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(169,141,255)), ([single]($size * $scale * 0.115))
    $pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $pen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $points = [System.Drawing.PointF[]]@(
        [System.Drawing.PointF]::new(($size * $scale * 0.25), ($size * $scale * 0.27)),
        [System.Drawing.PointF]::new(($size * $scale * 0.50), ($size * $scale * 0.75)),
        [System.Drawing.PointF]::new(($size * $scale * 0.75), ($size * $scale * 0.27))
    )
    $graphics.DrawLines($pen, $points)
    $image = New-Object System.Drawing.Bitmap $size, $size
    $outGraphics = [System.Drawing.Graphics]::FromImage($image)
    $outGraphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $outGraphics.DrawImage($canvas, 0, 0, $size, $size)
    $fileName = switch ($size) { 44 { 'Square44x44Logo.png' } 50 { 'StoreLogo.png' } 150 { 'Square150x150Logo.png' } 300 { 'Square150x150Logo.scale-200.png' } }
    $image.Save((Join-Path $assetRoot $fileName), [System.Drawing.Imaging.ImageFormat]::Png)
    $outGraphics.Dispose(); $image.Dispose(); $pen.Dispose(); $graphics.Dispose(); $canvas.Dispose()
}
