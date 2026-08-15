# Converts src-tauri/icons/whale.svg into icon.png and a multi-size icon.ico.
#
# The SVG is rasterized by Microsoft Edge (Chromium) in headless mode, so we
# get the browser's exact SVG rendering instead of approximating the vector
# path with System.Drawing. System.Drawing is only used afterwards for
# high-quality downsampling and ICO packaging.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$iconsDir = Join-Path $PSScriptRoot '..\src-tauri\icons'
New-Item -ItemType Directory -Force -Path $iconsDir | Out-Null

$svgPath = Join-Path $iconsDir 'whale.svg'
if (-not (Test-Path -LiteralPath $svgPath)) {
    throw "Whale SVG source not found: $svgPath"
}
$svgText = [System.IO.File]::ReadAllText($svgPath)

function Find-MsEdge {
    $roots = @(
        [Environment]::GetFolderPath('ProgramFilesX86'),
        [Environment]::GetFolderPath('ProgramFiles')
    )

    foreach ($root in $roots) {
        if (-not [string]::IsNullOrWhiteSpace($root)) {
            $candidate = Join-Path $root 'Microsoft\Edge\Application\msedge.exe'
            if (Test-Path -LiteralPath $candidate) {
                return $candidate
            }
        }
    }

    return $null
}

function Write-RenderHtml {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Svg,
        [Parameter(Mandatory = $true)][string]$Background
    )

    # 1024px canvas, whale drawn at 800px with even 112px margins. The margin
    # keeps the whale recognizable at 16/24/32px after downsampling.
    $html = @"
<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>
  html, body {
    margin: 0;
    padding: 0;
    width: 1024px;
    height: 1024px;
    background: $Background;
  }
  svg {
    display: block;
    width: 800px;
    height: 800px;
    margin: 112px;
  }
</style>
</head>
<body>$Svg</body>
</html>
"@

    [System.IO.File]::WriteAllText(
        $Path,
        $html,
        (New-Object System.Text.UTF8Encoding($false))
    )
}

function Invoke-EdgeRender {
    param(
        [Parameter(Mandatory = $true)][string]$Edge,
        [Parameter(Mandatory = $true)][string]$EdgeProfile,
        [Parameter(Mandatory = $true)][string]$HtmlPath,
        [Parameter(Mandatory = $true)][string]$OutputPath,
        [Parameter(Mandatory = $true)][string]$Background
    )

    Write-RenderHtml -Path $HtmlPath -Svg $svgText -Background $Background

    $uri = ([System.Uri]$HtmlPath).AbsoluteUri
    $edgeArgs = @(
        '--headless',
        '--disable-gpu',
        '--hide-scrollbars',
        '--force-device-scale-factor=1',
        '--default-background-color=00000000',
        '--window-size=1024,1024',
        "--user-data-dir=$EdgeProfile",
        '--no-first-run',
        '--disable-extensions',
        "--screenshot=$OutputPath",
        $uri
    )

    & $Edge @edgeArgs | Out-Null

    if ($LASTEXITCODE -ne 0) {
        throw "Microsoft Edge SVG rendering failed with exit code $LASTEXITCODE."
    }

    # The versionless msedge.exe is a launcher stub; give the real browser a
    # moment to finish writing the screenshot if it is still running.
    $waitUntil = (Get-Date).AddSeconds(20)
    while (-not (Test-Path -LiteralPath $OutputPath) -and (Get-Date) -lt $waitUntil) {
        Start-Sleep -Milliseconds 250
    }
    if (-not (Test-Path -LiteralPath $OutputPath)) {
        throw 'Microsoft Edge did not create the rendered icon image.'
    }
}

function New-ResizedBitmap {
    param(
        [Parameter(Mandatory = $true)][System.Drawing.Bitmap]$Source,
        [Parameter(Mandatory = $true)][int]$Size
    )

    $bitmap = New-Object System.Drawing.Bitmap(
        $Size,
        $Size,
        [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
    )
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)

    try {
        $graphics.Clear([System.Drawing.Color]::Transparent)
        $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
        $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
        $graphics.DrawImage($Source, 0, 0, $Size, $Size)
        return $bitmap
    }
    finally {
        $graphics.Dispose()
    }
}

$edge = Find-MsEdge
if (-not $edge) {
    throw 'Microsoft Edge was not found. The icon generator now uses Edge headless to rasterize whale.svg.'
}

$workDir = Join-Path $iconsDir '.render-tmp'
if (Test-Path -LiteralPath $workDir) {
    Remove-Item -LiteralPath $workDir -Recurse -Force -ErrorAction SilentlyContinue
}
New-Item -ItemType Directory -Force -Path $workDir | Out-Null

$profileDir = Join-Path $workDir 'edge-profile'
$htmlPath = Join-Path $workDir 'render.html'
$masterPath = Join-Path $workDir 'whale-master.png'

# First try a transparent canvas. If the installed Edge version screenshots
# that as opaque black, fall back to a white canvas (never leave a black box).
Invoke-EdgeRender -Edge $edge -EdgeProfile $profileDir -HtmlPath $htmlPath -OutputPath $masterPath -Background 'transparent'

$master = [System.Drawing.Bitmap]::FromFile($masterPath)
$cornerPixel = $master.GetPixel(2, 2)
if ($cornerPixel.A -eq 255 -and $cornerPixel.R -le 24 -and $cornerPixel.G -le 24 -and $cornerPixel.B -le 24) {
    $master.Dispose()
    Remove-Item -LiteralPath $masterPath -Force -ErrorAction SilentlyContinue
    Write-Host 'Edge rendered a black background; retrying with a white canvas.'
    Invoke-EdgeRender -Edge $edge -EdgeProfile $profileDir -HtmlPath $htmlPath -OutputPath $masterPath -Background '#ffffff'
    $master = [System.Drawing.Bitmap]::FromFile($masterPath)
}

# icon.png: 512px source used by Tauri for the default window icon.
$pngPath = Join-Path $iconsDir 'icon.png'
$png512 = New-ResizedBitmap -Source $master -Size 512
try {
    $png512.Save($pngPath, [System.Drawing.Imaging.ImageFormat]::Png)
}
finally {
    $png512.Dispose()
}

# icon.ico: valid multi-frame ICO with PNG-compressed entries.
$sizes = @(256, 128, 64, 48, 32, 24, 16)
$pngFrames = @()

foreach ($frameSize in $sizes) {
    $frameBitmap = New-ResizedBitmap -Source $master -Size $frameSize
    $frameStream = New-Object System.IO.MemoryStream
    try {
        $frameBitmap.Save($frameStream, [System.Drawing.Imaging.ImageFormat]::Png)
        $pngFrames += ,$frameStream.ToArray()
    }
    finally {
        $frameStream.Dispose()
        $frameBitmap.Dispose()
    }
}

$icoPath = Join-Path $iconsDir 'icon.ico'
$icoStream = New-Object System.IO.MemoryStream
$icoWriter = New-Object System.IO.BinaryWriter($icoStream)

try {
    # ICO header: reserved, type=icon, image count.
    $icoWriter.Write([uint16]0)
    $icoWriter.Write([uint16]1)
    $icoWriter.Write([uint16]$pngFrames.Count)

    # ICO directory entries. PNG frames are declared as 32bpp icons; a zero
    # dimension byte means 256px (Windows ICO convention).
    $frameOffset = 6 + (16 * $pngFrames.Count)
    for ($i = 0; $i -lt $pngFrames.Count; $i++) {
        $frameBytes = $pngFrames[$i]
        $frameSize = $sizes[$i]

        if ($frameSize -ge 256) {
            $entryWidth = 0
            $entryHeight = 0
        }
        else {
            $entryWidth = $frameSize
            $entryHeight = $frameSize
        }

        $icoWriter.Write([byte]$entryWidth)
        $icoWriter.Write([byte]$entryHeight)
        $icoWriter.Write([byte]0)    # color count
        $icoWriter.Write([byte]0)    # reserved
        $icoWriter.Write([uint16]1)  # planes
        $icoWriter.Write([uint16]32) # bits per pixel
        $icoWriter.Write([uint32]$frameBytes.Length)
        $icoWriter.Write([uint32]$frameOffset)
        $frameOffset += $frameBytes.Length
    }

    foreach ($frameBytes in $pngFrames) {
        $icoWriter.Write($frameBytes)
    }

    $icoWriter.Flush()
    [System.IO.File]::WriteAllBytes($icoPath, $icoStream.ToArray())
}
finally {
    $icoWriter.Dispose()
    $icoStream.Dispose()
}

$master.Dispose()
Remove-Item -LiteralPath $workDir -Recurse -Force -ErrorAction SilentlyContinue

Write-Host "Black whale icons rendered from $svgPath and written to $iconsDir"
