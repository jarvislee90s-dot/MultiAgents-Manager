# probe-postmsg.ps1 — 向 conhost 窗口 PostMessage WM_KEYDOWN/WM_KEYUP（真键盘消息路径，选择 UI 监听此路径）
# 用法：probe-postmsg.ps1 -Hwnd 42666276 -Vk 0x1B [-Scan 0x01]
param(
    [Parameter(Mandatory = $true)][long]$Hwnd,
    [Parameter(Mandatory = $true)][uint32]$Vk,
    [uint32]$Scan = 0,
    [string]$RunId = (Get-Date -Format 'yyyyMMdd-HHmmss')
)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class PostMsg {
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PostMessageW(IntPtr hWnd, uint Msg, IntPtr wParam, IntPtr lParam);
}
"@
$h = [IntPtr]$Hwnd
$WM_KEYDOWN = 0x0100
$WM_KEYUP = 0x0101
$lparamDown = [IntPtr](1 -bor ([int]$Scan -shl 16))   # repeat=1, scancode
$r1 = [PostMsg]::PostMessageW($h, $WM_KEYDOWN, [IntPtr]$Vk, $lparamDown)
Start-Sleep -Milliseconds 50
$lparamUp = [IntPtr](1 -bor ([int]$Scan -shl 16) -bor (1 -shl 30) -bor (1 -shl 31))
$r2 = [PostMsg]::PostMessageW($h, $WM_KEYUP, [IntPtr]$Vk, $lparamUp)
Add-Content -Path "$env:USERPROFILE\mam-probe-m6r\logs\probe-postmsg-$RunId.log" -Value ("{0} POSTMSG hwnd={1} vk=0x{2:X2} down={3} up={4}" -f (Get-Date -Format o), $Hwnd, $Vk, $r1, $r2)
Write-Output ("POSTMSG vk=0x{0:X2} down={1} up={2}" -f $Vk, $r1, $r2)
