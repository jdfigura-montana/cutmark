param([switch]$Full)

# Packages excluded from clippy and test. Starts empty. Add a package only with a comment saying why and when it comes off.
$Exclude = @()

# Tests that time out under full parallel load: both wait on background workers with a fixed timeout and pass when run alone. They run alone, one thread, after the package's other tests.
# The video track fixes the tests; then remove the entry.
$Isolated = @{ "cutmark-video-ui" = @("perf_ui::", "media_ui::offline_badges_proxy_toggle_and_dialogs") }

# The video UI tests share render workers. With every core running a test, some miss their frame deadline. 4 threads passes reliably at the same speed.
$TestThreads = @{ "cutmark-video-ui" = 4 }

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
    $threadArgs = @()
    if ($TestThreads.ContainsKey($p)) { $threadArgs = @("--test-threads=$($TestThreads[$p])") }
    if ($Isolated.ContainsKey($p)) {
        $skipArgs = @()
        foreach ($f in $Isolated[$p]) { $skipArgs += @("--skip", $f) }
        Invoke-Step "test $p" (@("test", "-p", $p, "--") + $skipArgs + $threadArgs)
        foreach ($f in $Isolated[$p]) {
            Invoke-Step "test $p (isolated $f)" @("test", "-p", $p, "--test", "it", $f, "--", "--test-threads=1")
        }
    } else {
        if ($threadArgs.Count -gt 0) {
            Invoke-Step "test $p" (@("test", "-p", $p, "--") + $threadArgs)
        } else {
            Invoke-Step "test $p" @("test", "-p", $p)
        }
    }
}
Invoke-Step "deny" @("deny", "check", "--hide-inclusion-graph")
Write-Host "ALL CHECKS PASSED"
