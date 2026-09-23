# probe-e5.ps1 — E5 ccdr 参数组合验证：80 字符/块 + 块间 50ms + 正文后 150ms 提交回车
# 事件形态：keydown+keyup 成对；vk=VkKeyScanW(ch)&0xFF；scan=MapVirtualKeyW 非零派生（ccdr 硬约束）
# Enter：VK_RETURN + 派生扫描码 + '\r'（控制键必须走 VK 事件——E1 已实证 codex 丢弃 vk=0 控制字符）
# 用法：probe-e5.ps1 -TargetPid 43856 -Stamp E5A-1 -Length 200 [-Block 80] [-PaceMs 50]
param(
    [Parameter(Mandatory = $true)][uint32]$TargetPid,
    [Parameter(Mandatory = $true)][string]$Stamp,
    [Parameter(Mandatory = $true)][int]$Length,
    [int]$Block = 80,
    [int]$PaceMs = 50,
    [int]$SubmitDelayMs = 150,
    [switch]$NoSubmit,
    [string]$RunId = (Get-Date -Format 'yyyyMMdd-HHmmss')
)
$ErrorActionPreference = 'Stop'
. "$env:USERPROFILE\mam-probe-m6r\ConIn.ps1"
Set-ConInLog ("$env:USERPROFILE\mam-probe-m6r\logs\probe-e5-{0}.log" -f $RunId)

$prefix = "reply just OK ["
$body = $prefix + $Stamp + "]"
if ($body.Length -gt $Length) { Write-Output "E5-FAIL stamp=$Stamp bodyLen>$Length"; exit 1 }
# 填充用可变模式（更接近真实长文），凑足精确长度
$pattern = "The quick brown fox jumps over the lazy dog 0123456789. "
$fillerLen = $Length - $body.Length
$filler = ($pattern * [Math]::Ceiling($fillerLen / $pattern.Length))
$msg = $body + $filler.Substring(0, $fillerLen)

$h = Open-TargetConsole $TargetPid
if (-not $h) { Write-Output ("E5 stamp={0} RESULT=OPEN_FAIL" -f $Stamp); exit 1 }
$recs = [ConIn]::TextToRecords($msg, $true, $true)   # withScan=true：vk=VkKeyScanW、scan=MapVirtualKeyW 非零
$sw = [System.Diagnostics.Stopwatch]::StartNew()
$r = Send-Records -Handle $h -Records $recs -Chunk ([Math]::Max(1, $Block * 2)) -PaceMs $PaceMs -Tag "E5 $Stamp"
$injMs = [int]$sw.Elapsed.TotalMilliseconds
if (-not $NoSubmit) {
    Start-Sleep -Milliseconds $SubmitDelayMs
    $scanE = [uint16][ConIn]::MapVirtualKeyW([ConIn]::VK_RETURN, 0)
    $erecs = [ConIn]::CharsToRecords("`r", [ConIn]::VK_RETURN, $scanE, $true)
    $ew = [uint32]0
    $eok = [ConIn]::WriteConsoleInput($h, $erecs, [uint32]$erecs.Length, [ref]$ew)
    $eerr = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    $submit = "ok=$eok written=$ew err=$eerr"
    Write-Log ("E5 ENTER stamp={0} {1}" -f $Stamp, $submit)
} else { $submit = "skipped" }
Close-ConIn $h
Write-Log ("E5 SUMMARY stamp={0} len={1} events={2} written={3}/{4} failCalls={5} firstErr={6} injMs={7} submit={8}" -f $Stamp, $msg.Length, $r.Total, $r.Written, $r.Total, $r.FailCalls, $r.FirstFailErr, $injMs, $submit)
Write-Output ("E5 stamp={0} len={1} written={2}/{3} failCalls={4} firstErr={5} injMs={6} submit={7}" -f $Stamp, $msg.Length, $r.Written, $r.Total, $r.FailCalls, $r.FirstFailErr, $injMs, $submit)
