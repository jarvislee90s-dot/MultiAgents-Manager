# probe-oc-paced.ps1 — opencode 慢速消费者专用：消费感知节流（每块写入后轮询占用率回落再续）
# 用法：probe-oc-paced.ps1 -TargetPid 35604 -Stamp PX-oc -Length 10000 [-Block 80] [-DrainTo 40] [-DrainTimeoutMs 20000]
param(
    [Parameter(Mandatory = $true)][uint32]$TargetPid,
    [Parameter(Mandatory = $true)][string]$Stamp,
    [Parameter(Mandatory = $true)][int]$Length,
    [int]$Block = 80,
    [int]$PaceMs = 0,
    [int]$DrainTo = 40,
    [int]$DrainTimeoutMs = 20000,
    [int]$SubmitDelayMs = 300,
    [string]$RunId = (Get-Date -Format 'yyyyMMdd-HHmmss')
)
$ErrorActionPreference = 'Stop'
. "$env:USERPROFILE\probe-opencode\ConIn.ps1"
Set-ConInLog ("$env:USERPROFILE\probe-opencode\logs\probe-oc-paced-{0}.log" -f $RunId)

$prefix = "reply just OK ["
$body = $prefix + $Stamp + "]"
if ($body.Length -gt $Length) { Write-Output "PACED-FAIL bodyLen>$Length"; exit 1 }
$pattern = "The quick brown fox jumps over the lazy dog 0123456789. "
$fillerLen = $Length - $body.Length
$filler = ($pattern * [Math]::Ceiling($fillerLen / $pattern.Length))
$msg = $body + $filler.Substring(0, $fillerLen)

$h = Open-TargetConsole $TargetPid
if (-not $h) { Write-Output "PACED OPEN_FAIL"; exit 1 }
$recs = [ConIn]::TextToRecords($msg, $true, $true)
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$totalWaitMs = 0
$i = 0
$callN = 0
while ($i -lt $recs.Length) {
    $end = [Math]::Min($i + $Block * 2, $recs.Length)
    $slice = New-Object 'ConIn+INPUT_RECORD[]' ($end - $i)
    [Array]::Copy($recs, $i, $slice, 0, $end - $i)
    $w = [uint32]0
    $ok = [ConIn]::WriteConsoleInput($h, $slice, [uint32]$slice.Length, [ref]$w)
    $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    $callN++
    Write-Log ("PACED call={0} off={1}/{2} n={3} ok={4} written={5} err={6}" -f $callN, $i, $recs.Length, $slice.Length, $ok, $w, $err)
    if ($ok) { $i += [int]$w } else { Start-Sleep -Milliseconds 80; continue }
    # 消费感知：等占用率回落到阈值以下再发下一块
    $wsw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($wsw.Elapsed.TotalMilliseconds -lt $DrainTimeoutMs) {
        Start-Sleep -Milliseconds 50
        $occ = Get-Occupancy $h
        if ($occ -le $DrainTo) { break }
    }
    $totalWaitMs += [int]$wsw.Elapsed.TotalMilliseconds
}
$injMs = [int]$sw.Elapsed.TotalMilliseconds
Start-Sleep -Milliseconds $SubmitDelayMs
$scanE = [uint16][ConIn]::MapVirtualKeyW([ConIn]::VK_RETURN, 0)
$erecs = [ConIn]::CharsToRecords("`r", [ConIn]::VK_RETURN, $scanE, $true)
$ew = [uint32]0
$eok = [ConIn]::WriteConsoleInput($h, $erecs, [uint32]$erecs.Length, [ref]$ew)
$eerr = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
Write-Log ("PACED ENTER stamp={0} ok={1} written={2} err={3}" -f $Stamp, $eok, $ew, $eerr)
$occEnd = Get-Occupancy $h
Close-ConIn $h
Write-Log ("PACED SUMMARY stamp={0} len={1} calls={2} injMs={3} drainWaitMs={4} occEnd={5} submit={6}/{7} err={8}" -f $Stamp, $msg.Length, $callN, $injMs, $totalWaitMs, $occEnd, $eok, $ew, $eerr)
Write-Output ("PACED stamp={0} len={1} calls={2} injMs={3} drainWaitMs={4} occEnd={5} submit={6}" -f $Stamp, $msg.Length, $callN, $injMs, $totalWaitMs, $occEnd, $eok)
