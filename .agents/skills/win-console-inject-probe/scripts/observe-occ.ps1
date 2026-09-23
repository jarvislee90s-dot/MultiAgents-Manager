# observe-occ.ps1 — E3 占用率观察器：附加目标控制台，每 50ms 采 GetNumberOfConsoleInputEvents，与写入日志按时间戳对齐
# 用法（后台起，随后跑注入）：powershell -File observe-occ.ps1 -TargetPid 43856 -Seconds 60 -Tag E3-claude
param(
    [Parameter(Mandatory = $true)][uint32]$TargetPid,
    [int]$Seconds = 60,
    [string]$Tag = "occ",
    [string]$RunId = (Get-Date -Format 'yyyyMMdd-HHmmss')
)
$ErrorActionPreference = 'Stop'
. "$env:USERPROFILE\mam-probe-m6r\ConIn.ps1"
Set-ConInLog ("$env:USERPROFILE\mam-probe-m6r\logs\observe-occ-{0}.log" -f $RunId)
$h = Open-TargetConsole $TargetPid
if (-not $h) { Write-Output "OBS OPEN_FAIL"; exit 1 }
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$n = [uint32]0
while ($sw.Elapsed.TotalSeconds -lt $Seconds) {
    [void][ConIn]::GetNumberOfConsoleInputEvents($h, [ref]$n)
    Write-Log ("OCC {0} t={1}ms occ={2}" -f $Tag, [int]$sw.Elapsed.TotalMilliseconds, $n)
    Start-Sleep -Milliseconds 50
}
Close-ConIn $h
Write-Output ("OBS DONE {0} samples over {1}s" -f $Tag, $Seconds)
