#requires -version 5
<#
Fullscreen regression probe for Octos.

Creates a borderless, topmost, monitor-sized test window (like a fullscreen game),
runs verify-desktop.ps1 while it is open, closes it, then runs verify again.

Expected:
  while fullscreen: OctosWallpaperWindow visible=False; log monitor ... fullscreen=true
  after close:      OctosWallpaperWindow visible=True;  log monitor ... fullscreen=false

Usage:
  powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\fullscreen-probe.ps1

Requires verify-desktop.ps1 in the same directory.
It only creates a temporary local window; it does not modify the system.
#>

Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class FullscreenProbeDpi {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
[FullscreenProbeDpi]::SetProcessDPIAware() | Out-Null

$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$form = New-Object System.Windows.Forms.Form
$form.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::None
$form.StartPosition = [System.Windows.Forms.FormStartPosition]::Manual
$form.Bounds = New-Object System.Drawing.Rectangle(0, 0, $screen.Width, $screen.Height)
$form.BackColor = [System.Drawing.Color]::Black
$form.TopMost = $true
$form.Show()

function Pump-For([int]$milliseconds) {
    $until = (Get-Date).AddMilliseconds($milliseconds)
    while ((Get-Date) -lt $until) {
        [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 50
    }
}

try {
    Pump-For 2000
    Write-Output "=== verify while fullscreen (window $($screen.Width)x$($screen.Height)) ==="
    & powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'verify-desktop.ps1')
} finally {
    $form.Close()
}

Pump-For 2000
Write-Output "=== verify after fullscreen window closed ==="
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'verify-desktop.ps1')
