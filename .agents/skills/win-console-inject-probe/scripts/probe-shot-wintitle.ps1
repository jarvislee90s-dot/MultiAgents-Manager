# probe-shot-wintitle.ps1 — 按窗口标题抓取指定进程的顶层窗口（PrintWindow，锁屏可用）
# 用法：powershell -File probe-shot-wintitle.ps1 -ProcId 33160 -TitleMatch claude -Name W-claude-idle
param(
    [Parameter(Mandatory = $true)][int]$ProcId,
    [Parameter(Mandatory = $true)][string]$TitleMatch,
    [Parameter(Mandatory = $true)][string]$Name,
    [string]$OutDir = "$env:USERPROFILE\probe-opencode\evidence"
)
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Collections.Generic;
public static class WinEnum2 {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint nFlags);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    public static List<IntPtr> WindowsOfPid(uint pid) {
        var list = new List<IntPtr>();
        EnumWindows((h, l) => {
            uint p; GetWindowThreadProcessId(h, out p);
            if (p == pid && IsWindowVisible(h)) list.Add(h);
            return true;
        }, IntPtr.Zero);
        return list;
    }
    public static string Title(IntPtr h) {
        var sb = new StringBuilder(512);
        GetWindowTextW(h, sb, 512);
        return sb.ToString();
    }
}
"@
[void][WinEnum2]::SetProcessDPIAware()
$target = [IntPtr]::Zero
$targetTitle = ""
foreach ($h in [WinEnum2]::WindowsOfPid([uint32]$ProcId)) {
    $t = [WinEnum2]::Title($h)
    if ($t -match $TitleMatch) { $target = $h; $targetTitle = $t; break }
}
if ($target -eq [IntPtr]::Zero) { Write-Output ("SHOT-FAIL pid={0} 无标题匹配 [{1}] 的可见窗口" -f $ProcId, $TitleMatch); exit 1 }
$rect = New-Object WinEnum2+RECT
[void][WinEnum2]::GetWindowRect($target, [ref]$rect)
$w = $rect.Right - $rect.Left; $ht = $rect.Bottom - $rect.Top
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
$ok = [WinEnum2]::PrintWindow($target, $hdc, 2)
[void]$g.ReleaseHdc($hdc)
$g.Dispose()
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
$ts = Get-Date -Format 'yyyyMMdd-HHmmss'
$path = Join-Path $OutDir ("{0}-{1}.png" -f $Name, $ts)
$bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output ("SHOT={0} printwindow={1} title={2}" -f $path, $ok, $targetTitle)
