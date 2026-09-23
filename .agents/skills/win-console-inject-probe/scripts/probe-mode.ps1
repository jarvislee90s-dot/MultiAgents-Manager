# probe-mode.ps1 — E1 一次性采样：附加目标控制台 → GetConsoleMode 标志位 + 占用率 → 记日志 → 复位退出
# 用法：powershell -File probe-mode.ps1 -TargetPid 1234 -Label claude-conhost-idle [-RunId auto]
param(
    [Parameter(Mandatory = $true)][uint32]$TargetPid,
    [Parameter(Mandatory = $true)][string]$Label,
    [string]$RunId = (Get-Date -Format 'yyyyMMdd-HHmmss')
)
. "$env:USERPROFILE\mam-probe-m6r\ConIn.ps1"
Set-ConInLog ("$env:USERPROFILE\mam-probe-m6r\logs\probe-mode-{0}.log" -f $RunId)
$h = Open-TargetConsole $TargetPid
if (-not $h) { Write-Output ("MODE label={0} RESULT=OPEN_FAIL" -f $Label); exit 1 }
$mode = Get-ModeString $h
$occ = Get-Occupancy $h
Write-Log ("MODE label={0} target={1} {2} occupancy={3}" -f $Label, $TargetPid, $mode, $occ)
Close-ConIn $h
Write-Output ("MODE label={0} {1} OCC={2}" -f $Label, $mode, $occ)
