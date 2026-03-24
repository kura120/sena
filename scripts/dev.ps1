# Sena — launch daemon-bus and UI for development
# Usage: .\scripts\dev.ps1 [-DaemonOnly] [-UiOnly]
param(
    [switch]$DaemonOnly,
    [switch]$UiOnly
)

$ErrorActionPreference = "Stop"
$RootDir = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)

$daemonJob = $null
$uiJob = $null

function Start-Daemon {
    Write-Host "[sena] starting daemon-bus..." -ForegroundColor Cyan
    $script:daemonJob = Start-Process -FilePath "cargo" `
        -ArgumentList "run", "--bin", "daemon-bus" `
        -WorkingDirectory $RootDir `
        -PassThru -NoNewWindow
    Write-Host "[sena] daemon-bus PID: $($script:daemonJob.Id)" -ForegroundColor Green
}

function Start-UI {
    Write-Host "[sena] starting ui (tauri dev)..." -ForegroundColor Cyan
    $script:uiJob = Start-Process -FilePath "pnpm" `
        -ArgumentList "tauri", "dev" `
        -WorkingDirectory (Join-Path $RootDir "ui") `
        -PassThru -NoNewWindow
    Write-Host "[sena] ui PID: $($script:uiJob.Id)" -ForegroundColor Green
}

function Stop-All {
    Write-Host ""
    Write-Host "[sena] shutting down..." -ForegroundColor Yellow
    if ($script:daemonJob -and -not $script:daemonJob.HasExited) {
        Stop-Process -Id $script:daemonJob.Id -Force -ErrorAction SilentlyContinue
    }
    if ($script:uiJob -and -not $script:uiJob.HasExited) {
        Stop-Process -Id $script:uiJob.Id -Force -ErrorAction SilentlyContinue
    }
    Write-Host "[sena] stopped." -ForegroundColor Green
}

try {
    if ($DaemonOnly) {
        Start-Daemon
        $script:daemonJob.WaitForExit()
    }
    elseif ($UiOnly) {
        Start-UI
        $script:uiJob.WaitForExit()
    }
    else {
        Start-Daemon
        # Give the daemon a moment to bind its gRPC port
        Start-Sleep -Seconds 2
        Start-UI

        Write-Host "[sena] press Ctrl+C to stop both processes" -ForegroundColor DarkGray
        # Wait for either process to exit
        while ((-not $script:daemonJob.HasExited) -and (-not $script:uiJob.HasExited)) {
            Start-Sleep -Milliseconds 500
        }
    }
}
finally {
    Stop-All
}
