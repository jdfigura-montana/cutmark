param([switch]$Full)

# Packages excluded from clippy and test. Starts empty. Add a package only with a comment saying why and when it comes off.
$Exclude = @()

$ErrorActionPreference = "Continue"
Set-Location (Split-Path $PSScriptRoot -Parent)

$packages = (cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).packages | ForEach-Object { $_.name } | Where-Object { $Exclude -notcontains $_ } | Sort-Object

function Invoke-Step {
    param([string]$Name, [string[]]$CargoArgs)
    Write-Host "RUN  $Name"
    if ($Full) {
        & cargo @CargoArgs
        $code = $LASTEXITCODE
    } else {
        $out = & cargo @CargoArgs 2>&1 | ForEach-Object { $_.ToString() }
        $code = $LASTEXITCODE
        if ($code -ne 0) { $out | Select-Object -Last 40 }
    }
    if ($code -ne 0) {
        Write-Host "FAIL $Name (exit $code)"
        exit 1
    }
    Write-Host "PASS $Name"
}

Invoke-Step "fmt" @("fmt", "--all", "--check")
# One cargo invocation per package: building cutmark-audio (eframe glow) together with the video crates (eframe wgpu) unifies eframe features and breaks the build.
foreach ($p in $packages) {
    Invoke-Step "clippy $p" @("clippy", "-p", $p, "--all-targets", "--", "-D", "warnings")
}
foreach ($p in $packages) {
    Invoke-Step "test $p" @("test", "-p", $p)
}
Invoke-Step "deny" @("deny", "check", "--hide-inclusion-graph")
Write-Host "ALL CHECKS PASSED"
