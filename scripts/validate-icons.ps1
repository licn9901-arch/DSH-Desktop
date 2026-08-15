$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing

$iconsDir = Join-Path $PSScriptRoot '..\src-tauri\icons'
$expectedHashes = [ordered]@{
    'whale.svg' = '9E983B4F649C25C6CA0623A50BE1A6E705FD8F49F756638B980FA13B40575AB6'
    'icon.png'  = '0518574CCA49B23C50F78F15075CE07080F70B8B52A3BE575C0FD9F2803771DE'
    'icon.ico'  = '1E01FB7B71B7CFC306E1704500722A4F3641C816E4497CA2A8F980CD2981ED71'
}

# 固定哈希可以防止构建过程无意替换用户确认过的鲸鱼视觉资产。
foreach ($entry in $expectedHashes.GetEnumerator()) {
    $path = Join-Path $iconsDir $entry.Key
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Icon asset is missing: $path"
    }

    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
    if ($actual -ne $entry.Value) {
        throw "Icon asset hash mismatch: $($entry.Key). Expected $($entry.Value), got $actual."
    }
}

$pngPath = Join-Path $iconsDir 'icon.png'
$png = [System.Drawing.Bitmap]::FromFile($pngPath)
try {
    if ($png.Width -ne 512 -or $png.Height -ne 512) {
        throw "icon.png must be 512x512, got $($png.Width)x$($png.Height)."
    }

    $corners = @(
        $png.GetPixel(0, 0),
        $png.GetPixel($png.Width - 1, 0),
        $png.GetPixel(0, $png.Height - 1),
        $png.GetPixel($png.Width - 1, $png.Height - 1)
    )
    if ($corners.Where({ $_.A -ne 0 }).Count -gt 0) {
        throw 'icon.png must keep a transparent background.'
    }
}
finally {
    $png.Dispose()
}

# 直接读取 ICO 目录项，避免依赖只会返回单帧的系统图标 API。
$icoPath = Join-Path $iconsDir 'icon.ico'
$stream = [System.IO.File]::OpenRead($icoPath)
$reader = [System.IO.BinaryReader]::new($stream)
try {
    $reserved = $reader.ReadUInt16()
    $type = $reader.ReadUInt16()
    $count = $reader.ReadUInt16()
    if ($reserved -ne 0 -or $type -ne 1) {
        throw 'icon.ico has an invalid ICO header.'
    }

    $actualSizes = @()
    for ($index = 0; $index -lt $count; $index++) {
        $width = [int]$reader.ReadByte()
        $height = [int]$reader.ReadByte()
        $reader.ReadBytes(14) | Out-Null
        if ($width -eq 0) { $width = 256 }
        if ($height -eq 0) { $height = 256 }
        if ($width -ne $height) {
            throw "icon.ico contains a non-square frame: ${width}x${height}."
        }
        $actualSizes += $width
    }

    $expectedSizes = @(16, 24, 32, 48, 64, 128, 256)
    $actualSizes = @($actualSizes | Sort-Object -Unique)
    if (($actualSizes -join ',') -ne ($expectedSizes -join ',')) {
        throw "icon.ico sizes must be $($expectedSizes -join ','); got $($actualSizes -join ',')."
    }
}
finally {
    $reader.Dispose()
    $stream.Dispose()
}

Write-Host 'Icon assets are valid and unchanged.'
