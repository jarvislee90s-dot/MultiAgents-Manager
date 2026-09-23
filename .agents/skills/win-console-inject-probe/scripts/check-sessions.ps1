# check-sessions.ps1 — 列出最近的 mam-probe 相关 cmd 及其直接子进程（诊断用）
$recent = Get-CimInstance Win32_Process -Filter "Name='cmd.exe'" | Where-Object { $_.CreationDate -gt (Get-Date).AddMinutes(-15) }
foreach ($c in $recent) {
    "CMD pid={0} created={1} cmd={2}" -f $c.ProcessId, $c.CreationDate.ToString('HH:mm:ss'), $c.CommandLine
    Get-CimInstance Win32_Process -Filter "ParentProcessId=$($c.ProcessId)" | ForEach-Object {
        "  child {0} pid={1} created={2}" -f $_.Name, $_.ProcessId, $_.CreationDate.ToString('HH:mm:ss')
    }
}
if (-not $recent) { "NO recent cmd.exe in last 15min" }
