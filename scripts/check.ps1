param([switch]$Full)

# Packages excluded from clippy and test. Starts empty. Add a package only with a comment saying why and when it comes off.
$Exclude = @()

$ErrorActionPreference = "Continue"
Set-Location (Split-Path $PSScriptRoot -Parent)

$excludeArgs = @()
foreach ($p in $Exclude) { $excludeArgs += "--exclude"; $excludeArgs += $p }

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
Invoke-Step "clippy" (@("clippy", "--workspace", "--all-targets") + $excludeArgs + @("--", "-D", "warnings"))
Invoke-Step "test" (@("test", "--workspace") + $excludeArgs)
Invoke-Step "deny" @("deny", "check", "--hide-inclusion-graph")
Write-Host "ALL CHECKS PASSED"
