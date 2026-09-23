# probe-key2.ps1 — M6R 通用单键/VT 序列注入（一次性子进程：Attach→写→Free）
# 用法：
#   按键形态：probe-key2 -TargetPid 1234 -Vk 0x0D -Scan 0x1C -Char "\r" [-DownOnly]
#   VT 形态：  probe-key2 -TargetPid 1234 -VtName down|up|left|right|esc|enter   （ESC 由 [char]27 构造，整批原子写）
param(
    [Parameter(Mandatory = $true)][uint32]$TargetPid,
    [string]$Label = "key",
    [uint16]$Vk = 0,
    [uint16]$Scan = 0,
    [string]$Char = "",
    [ValidateSet('', 'down', 'up', 'left', 'right', 'esc', 'enter', 'bpOn', 'bpOff')]
    [string]$VtName = "",
    [switch]$DownOnly,
    [switch]$EnterVK,
    [string]$RunId = (Get-Date -Format 'yyyyMMdd-HHmmss')
)
$ErrorActionPreference = 'Stop'
. "$env:USERPROFILE\mam-probe-m6r\ConIn.ps1"
Set-ConInLog ("$env:USERPROFILE\mam-probe-m6r\logs\probe-key2-{0}.log" -f $RunId)
$ESC = [char]27
$h = Open-TargetConsole $TargetPid
if (-not $h) { Write-Output ("KEY label={0} RESULT=OPEN_FAIL" -f $Label); exit 1 }

$paired = (-not $DownOnly)
if ($VtName -ne '') {
    $seq = switch ($VtName) {
        'down'  { "$ESC[B" }
        'up'    { "$ESC[A" }
        'right' { "$ESC[C" }
        'left'  { "$ESC[D" }
        'esc'   { "$ESC" }
        'enter' { "`r" }
        'bpOn'  { "$ESC[200~" }
        'bpOff' { "$ESC[201~" }
    }
    $recs = [ConIn]::CharsToRecords($seq, 0, 0, $paired)
    $r = Send-Records -Handle $h -Records $recs -SingleCall -Tag ("KEYVT {0}" -f $Label)
} elseif ($EnterVK) {
    # 真·回车（VK_RETURN + 真实扫描码 + '\r'，全部在 PS 内部构造——严禁经命令行传转义序列）
    $scanE = [uint16][ConIn]::MapVirtualKeyW([ConIn]::VK_RETURN, 0)
    $recs = [ConIn]::CharsToRecords("`r", [ConIn]::VK_RETURN, $scanE, $paired)
    $r = Send-Records -Handle $h -Records $recs -SingleCall -Tag ("KEYVK-ENTER {0}" -f $Label)
} else {
    $c = [char]0
    if ($Char.Length -gt 0) { $c = $Char[0] }
    $recs = [ConIn]::CharsToRecords("$c", $Vk, $Scan, $paired)
    $r = Send-Records -Handle $h -Records $recs -SingleCall -Tag ("KEY {0}" -f $Label)
}
Close-ConIn $h
Write-Output ("KEY label={0} written={1}/{2}" -f $Label, $r.Written, $r.Total)
