# probe-shot-win.ps1 — 按窗口截图（PrintWindow，PW_RENDERFULLCONTENT）：锁屏下仍可抓窗口内容
# 用法：powershell -File probe-shot-win.ps1 -ProcId 33740 -Name C-claude-first
param(
    [Parameter(Mandatory = $true)][int]$ProcId,
    [Parameter(Mandatory = $true)][string]$Name,
    [string]$OutDir = "$env:USERPROFILE\mam-probe-m6r\evidence"
)
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class WinCap {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint nFlags);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder text, int count);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
[void][WinCap]::SetProcessDPIAware()
$p = Get-Process -Id $ProcId -ErrorAction Stop
$h = $p.MainWindowHandle
if ($h -eq [IntPtr]::Zero) { Write-Output ("SHOT-FAIL pid={0} 无可见主窗口" -f $ProcId); exit 1 }
$rect = New-Object WinCap+RECT
[void][WinCap]::GetWindowRect($h, [ref]$rect)
$w = $rect.Right - $rect.Left
$ht = $rect.Bottom - $rect.Top
if ($w -le 0 -or $ht -le 0) { Write-Output ("SHOT-FAIL pid={0} 窗口尺寸异常 {1}x{2}" -f $ProcId, $w, $ht); exit 1 }
$sb = New-Object System.Text.StringBuilder 512
[void][WinCap]::GetWindowTextW($h, $sb, 512)
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
$ok = [WinCap]::PrintWindow($h, $hdc, 2)
[void]$g.ReleaseHdc($hdc)
$g.Dispose()
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$ts = Get-Date -Format 'yyyyMMdd-HHmmss'
$path = Join-Path $OutDir ("{0}-{1}.png" -f $Name, $ts)
$bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output ("SHOT={0} win={1} [{2}x{3}] printwindow={4} title={5}" -f $path, $h, $w, $ht, $ok, $sb.ToString())
