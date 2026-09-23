# probe-type.ps1 — 输入文本（不发回车）或退格清理（E1 武装态/注入用）
# 用法：probe-type -TargetPid 1234 -Text "hello"    （逐字符成对事件，scan=0，M6 基线形态）
#       probe-type -TargetPid 1234 -Backspace 5     （退格 5 次）
param(
    [Parameter(Mandatory = $true)][uint32]$TargetPid,
    [string]$Text = "",
    [int]$Backspace = 0,
    [string]$Label = "type",
    [int]$PaceMs = 8,
    [switch]$DownOnly,
    [string]$RunId = (Get-Date -Format 'yyyyMMdd-HHmmss')
)
$ErrorActionPreference = 'Stop'
. "$env:USERPROFILE\mam-probe-m6r\ConIn.ps1"
Set-ConInLog ("$env:USERPROFILE\mam-probe-m6r\logs\probe-type-{0}.log" -f $RunId)
$h = Open-TargetConsole $TargetPid
if (-not $h) { Write-Output ("TYPE label={0} RESULT=OPEN_FAIL" -f $Label); exit 1 }
$totalWritten = 0; $totalEvents = 0
if ($Text -ne "") {
    $recs = [ConIn]::TextToRecords($Text, (-not $DownOnly), $false)
    $r = Send-Records -Handle $h -Records $recs -Chunk 2 -PaceMs $PaceMs -Tag ("TYPE {0}" -f $Label)
    $totalWritten += $r.Written; $totalEvents += $r.Total
}
if ($Backspace -gt 0) {
    $recs = [ConIn]::BackspaceRecords($Backspace, $true)
    $r = Send-Records -Handle $h -Records $recs -Chunk 2 -PaceMs $PaceMs -Tag ("BKSP {0}" -f $Label)
    $totalWritten += $r.Written; $totalEvents += $r.Total
}
$occ = Get-Occupancy $h
Close-ConIn $h
Write-Output ("TYPE label={0} written={1}/{2} occAfter={3}" -f $Label, $totalWritten, $totalEvents, $occ)
