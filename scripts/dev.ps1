# Dev loop with hot reload:
#   - crates/*/src/*.rs or any Cargo.toml change -> rebuild + restart the app (blending auto-starts)
#   - crates/edgeblend-gpu/src/blend.hlsl change  -> reloaded live inside the running app, no restart
# Usage:  .\scripts\dev.ps1      Stop: Ctrl+C (also closes the app)

$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
$exe = Join-Path $root 'target\release\edgeblend.exe'

function Get-Stamp {
    $files = @(Get-Item -Path Cargo.toml)
    $files += Get-ChildItem -Path crates -Filter Cargo.toml -Recurse -File
    $files += Get-ChildItem -Path crates -Filter *.rs -Recurse -File |
        Where-Object { $_.DirectoryName -match '[\\/]src$' }
    $files |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1 -ExpandProperty LastWriteTime
}

function Stop-App {
    Get-Process edgeblend -ErrorAction SilentlyContinue | ForEach-Object {
        $_.CloseMainWindow() | Out-Null
        if (-not $_.WaitForExit(2000)) { Stop-Process -Id $_.Id -Force }
    }
}

$last = $null
try {
    while ($true) {
        $stamp = Get-Stamp
        if ($stamp -ne $last) {
            $last = $stamp
            Stop-App
            Write-Host "[dev] building..." -ForegroundColor Cyan
            cargo build --release
            if ($LASTEXITCODE -eq 0) {
                Start-Process $exe -ArgumentList '--start' -NoNewWindow | Out-Null
                Write-Host "[dev] running. Edit crates/ to reload, Ctrl+C to quit." -ForegroundColor Green
            } else {
                Write-Host "[dev] build failed; waiting for the next change." -ForegroundColor Red
            }
        }
        Start-Sleep -Milliseconds 700
    }
} finally {
    Stop-App
}
