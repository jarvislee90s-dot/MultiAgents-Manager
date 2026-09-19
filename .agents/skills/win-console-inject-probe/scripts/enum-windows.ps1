# enum-windows.ps1 — 枚举顶层可见窗口（pid + 标题），用于找探测会话窗口
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Text;
using System.Collections.Generic;
public static class WinEnum {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder text, int count);
    public static List<string> All() {
        var list = new List<string>();
        EnumWindows((h, l) => {
            if (!IsWindowVisible(h)) return true;
            uint pid; GetWindowThreadProcessId(h, out pid);
            var sb = new StringBuilder(512);
            GetWindowTextW(h, sb, 512);
            var t = sb.ToString();
            if (t.Length > 0) list.Add(pid + "\t" + t);
            return true;
        }, IntPtr.Zero);
        return list;
    }
}
"@
$probeNames = @('conhost', 'cmd', 'claude', 'codex', 'kimi', 'node', 'WindowsTerminal', 'OpenConsole')
$probePids = @{} # pid -> procname
foreach ($pn in $probeNames) {
    Get-Process -Name $pn -ErrorAction SilentlyContinue | ForEach-Object { $probePids[[uint32]$_.Id] = $pn }
}
foreach ($line in [WinEnum]::All()) {
    $parts = $line -split "`t", 2
    $wpid = [uint32]$parts[0]
    $title = $parts[1]
    if ($probePids.ContainsKey($wpid)) {
        "{0} pid={1} [{2}] title={3}" -f 'WIN', $wpid, $probePids[$wpid], $title
    }
}
"---- WT/OpenConsole processes ----"
Get-Process -Name WindowsTerminal, OpenConsole -ErrorAction SilentlyContinue | ForEach-Object { "{0} pid={1} mainwin={2}" -f $_.Name, $_.Id, $_.MainWindowHandle }
