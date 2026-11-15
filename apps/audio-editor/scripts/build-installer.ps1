# Build the Windows installer: release exe + Inno Setup package.
# Usage: powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$ws = Split-Path -Parent (Split-Path -Parent $root)
Set-Location $root

# 1. Refuse to build while the app is running (its exe cannot be replaced).
$exe = Join-Path $ws 'target\release\cutmark-audio.exe'
if (Get-Process -Name 'cutmark-audio' -ErrorAction SilentlyContinue) {
    throw "cutmark-audio.exe is running. Close CutMark Audio Editor and try again."
}

# 2. Release build.
$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path
cargo build --release -p cutmark-audio
if ($LASTEXITCODE -ne 0) { throw "cargo build --release -p cutmark-audio failed" }

# 3. Version from Cargo.toml.
$cargo = Get-Content (Join-Path $root 'Cargo.toml') -Raw
$version = [regex]::Match($cargo, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if (-not $version) { throw "could not read version from Cargo.toml" }

# 4. Locate ISCC.exe in any "Inno Setup *" folder under the usual roots.
$iscc = $null
foreach ($rootDir in @(${env:ProgramFiles}, ${env:ProgramFiles(x86)}, "$env:LOCALAPPDATA\Programs")) {
    if (-not $rootDir) { continue }
    $iscc = Get-ChildItem $rootDir -Directory -Filter 'Inno Setup *' -ErrorAction SilentlyContinue |
        ForEach-Object { Join-Path $_.FullName 'ISCC.exe' } |
        Where-Object { Test-Path $_ } |
        Select-Object -First 1
    if ($iscc) { break }
}
if (-not $iscc) {
    throw "ISCC.exe (Inno Setup 6+) not found under Program Files, Program Files (x86) or $env:LOCALAPPDATA\Programs. Install Inno Setup from https://jrsoftware.org/isinfo.php"
}

# 5. Compile the installer.
& $iscc (Join-Path $root 'installer\cutmark.iss') "/DAppVersion=$version" "/DRoot=$root\" "/DBinDir=$ws\target\release\"
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }

# 6. Report the result.
$out = Get-Item (Join-Path $root "dist\CutMark-Setup-$version.exe")
Write-Host ''
Write-Host "Installer: $($out.FullName)"
Write-Host "Size: $([math]::Round($out.Length / 1MB, 2)) MB"
Write-Host "LastWriteTime: $($out.LastWriteTime)"
