param(
    [string]$Zip,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Tag    = 'autobuild-2026-11-14-12-37'
$Asset  = 'ffmpeg-n9.0.2-57-gd838f9f9ae-win64-lgpl-shared-9.0.zip'
$Url    = 'https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/download/autobuild-2026-11-14-12-37/ffmpeg-n9.0.2-57-gd838f9f9ae-win64-lgpl-shared-9.0.zip'
$Sha256 = 'CDB9DD9F94BC3958DD0D7BA39EC61B8E9D0A68351BF2F8BE22178BD8E0A5AA91'

# a. Locate dist and cache folders.
$here  = $PSScriptRoot
$dist  = Join-Path $here 'dist'
$cache = Join-Path $here 'cache'

# b. Skip if the pinned build is already installed.
$buildInfoPath = Join-Path $here 'BUILDINFO.txt'
$avcodec = Get-ChildItem -Path (Join-Path $dist 'bin') -Filter 'avcodec-*.dll' -ErrorAction SilentlyContinue
if ($null -ne $avcodec -and (Test-Path -LiteralPath $buildInfoPath) -and -not $Force) {
    $recorded = Select-String -LiteralPath $buildInfoPath -SimpleMatch $Sha256 -Quiet
    if ($recorded) {
        Write-Host "FFmpeg already installed ($Asset)"
        exit 0
    }
}

# c. Pick the zip: -Zip path, then the cache, then download.
$downloaded = $false
if ($Zip) {
    if (-not (Test-Path -LiteralPath $Zip)) {
        throw "Zip not found: $Zip"
    }
    $zipPath = $Zip
}
else {
    $cacheZip = Join-Path $cache $Asset
    if (Test-Path -LiteralPath $cacheZip) {
        $zipPath = $cacheZip
    }
    else {
        if (-not (Test-Path -LiteralPath $cache)) {
            New-Item -ItemType Directory -Path $cache -Force | Out-Null
        }
        Write-Host "Downloading $Asset"
        Invoke-WebRequest -Uri $Url -OutFile $cacheZip -UseBasicParsing
        $zipPath = $cacheZip
        $downloaded = $true
    }
}

# d. Verify the pinned SHA-256.
$actual = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash
if ($actual -ne $Sha256) {
    if ($downloaded) {
        Remove-Item -LiteralPath $zipPath -Force
    }
    throw "SHA-256 mismatch for ${zipPath}: expected $Sha256, got $actual"
}

# e. Expand to a fresh temp folder, then rebuild dist.
$temp = Join-Path ([IO.Path]::GetTempPath()) ('ffmpeg-' + [Guid]::NewGuid().ToString('N'))
if (Test-Path -LiteralPath $temp) {
    Remove-Item -LiteralPath $temp -Recurse -Force
}
Write-Host 'Extracting zip'
Expand-Archive -LiteralPath $zipPath -DestinationPath $temp -Force

$top = Get-ChildItem -LiteralPath $temp -Directory | Select-Object -First 1
if ($null -eq $top) {
    Remove-Item -LiteralPath $temp -Recurse -Force
    throw 'Unexpected zip layout: no top-level folder found'
}

foreach ($name in @('bin', 'lib', 'include')) {
    if (-not (Test-Path -LiteralPath (Join-Path $top.FullName $name))) {
        Remove-Item -LiteralPath $temp -Recurse -Force
        throw "Unexpected zip layout: missing '$name' in the top-level folder"
    }
}

if (Test-Path -LiteralPath $dist) {
    Remove-Item -LiteralPath $dist -Recurse -Force
}
New-Item -ItemType Directory -Path $dist -Force | Out-Null
foreach ($name in @('bin', 'lib', 'include')) {
    Copy-Item -LiteralPath (Join-Path $top.FullName $name) -Destination $dist -Recurse -Force
}
$license = Join-Path $top.FullName 'LICENSE.txt'
if (Test-Path -LiteralPath $license) {
    Copy-Item -LiteralPath $license -Destination $dist -Force
}

# f. Sanity checks against the pinned configure line.
$ffBin = Join-Path $dist 'bin\ffmpeg.exe'
$buildconf = (@(& $ffBin -hide_banner -buildconf 2>&1) | ForEach-Object { [string]$_ }) -join "`r`n"
if ($buildconf -notmatch '--enable-libmp3lame') {
    throw 'Build check failed: the FFmpeg build does not enable libmp3lame'
}
if ($buildconf -match '--enable-gpl') {
    throw 'Build check failed: the FFmpeg build is GPL, LGPL required'
}
if ($buildconf -match '--enable-nonfree') {
    throw 'Build check failed: the FFmpeg build is nonfree, LGPL required'
}

# g. Version first line.
$versionLine = @(& $ffBin -hide_banner -version | ForEach-Object { [string]$_ })[0]

# h. Write BUILDINFO.txt (ASCII, CRLF).
$lines = @(
    'FFmpeg build used by the Cutmark suite',
    '',
    "Asset: $Asset",
    "Tag: $Tag",
    "URL: $Url",
    "SHA-256: $Sha256",
    '',
    $versionLine,
    '',
    'License: LGPL version 3 or later (the build uses --enable-version3 without --enable-gpl or --enable-nonfree). Linked dynamically; the DLLs ship as loose files beside each exe so a user can replace them.',
    '',
    'Source: FFmpeg git commit d838f9f9ae (https://git.ffmpeg.org/ffmpeg.git, mirror https://github.com/FFmpeg/FFmpeg). Build scripts: https://github.com/FFmpeg-Builds/FFmpeg-Builds',
    '',
    'Configure line',
    '--------------'
) + @($buildconf -split "`r`n")
[IO.File]::WriteAllText($buildInfoPath, ($lines -join "`r`n") + "`r`n", [Text.Encoding]::ASCII)

# i. Clean up and report.
Remove-Item -LiteralPath $temp -Recurse -Force
Write-Host "FFmpeg installed: $Asset"
Write-Host $dist