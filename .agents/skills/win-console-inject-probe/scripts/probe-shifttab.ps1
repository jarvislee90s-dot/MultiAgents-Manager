# probe-shifttab.ps1 — shift+tab 修饰位形态注入（一次性子进程：Attach→写→Free）
#
# 探测专用——生产 shift+tab 走 engine.rs 2 记录形态（shift_tab_records）；本脚本供
# probe-key2 缺修饰位场景使用（2026-10-09 风险预测试批验证 4/4 生效）。
#
# 形态：4 记录单批原子写（Send-Records -SingleCall）——
#   shift↓(0x10/0x2A, 无字符, SHIFT_PRESSED) → tab↓(0x09/0x0F, '\t', SHIFT_PRESSED)
#   → tab↑(0x09/0x0F, '\t', SHIFT_PRESSED) → shift↑(0x10/0x2A, 无字符, 0)
# 参考：C:\Users\bunny\mam-probe-m6r\mp-shifttab.ps1（同形态）；键位/修饰位常量对齐
# ConIn.ps1 既有约定。日志经 Set-ConInLog 落 mam-probe-m6r/logs/（与 probe-key2 同套）。
#
# 用法：probe-shifttab -TargetPid 1234 [-Label st] [-RunId yyyyMMdd-HHmmss]
param(
    [Parameter(Mandatory = $true)][uint32]$TargetPid,
    [string]$Label = "st",
    [string]$RunId = (Get-Date -Format 'yyyyMMdd-HHmmss')
)
$ErrorActionPreference = 'Stop'
. "$env:USERPROFILE\mam-probe-m6r\ConIn.ps1"
Set-ConInLog ("$env:USERPROFILE\mam-probe-m6r\logs\probe-shifttab-{0}.log" -f $RunId)

$h = Open-TargetConsole $TargetPid
if (-not $h) { Write-Output ("SHIFTTAB label={0} RESULT=OPEN_FAIL" -f $Label); exit 1 }

$VK_SHIFT = [uint16]0x10; $SCAN_SHIFT = [uint16]0x2A
$VK_TAB = [uint16]0x09; $SCAN_TAB = [uint16]0x0F
$SHIFT_PRESSED = [uint32]0x0010

function Mk([bool]$down, [uint16]$vk, [uint16]$scan, [char]$c, [uint32]$ctrl) {
    $r = New-Object 'ConIn+INPUT_RECORD'
    $r.EventType = [ConIn]::KEY_EVENT_TYPE
    $r.bKeyDown = if ($down) { 1 } else { 0 }
    $r.wRepeatCount = 1
    $r.wVirtualKeyCode = $vk
    $r.wVirtualScanCode = $scan
    $r.UnicodeChar = $c
    $r.dwControlKeyState = $ctrl
    return $r
}

# 4 记录一个数组、一次调用整批写入（原子——中途不落半拍，读取侧见全序）。
# 修饰位逐条对齐 mp-shifttab.ps1 实测定案（4/4 生效）：shift↓ 自身也带
# SHIFT_PRESSED（事件发生时键已按下，与真键盘记录一致）。
$recs = @(
    (Mk $true $VK_SHIFT $SCAN_SHIFT ([char]0) $SHIFT_PRESSED),
    (Mk $true $VK_TAB $SCAN_TAB "`t" $SHIFT_PRESSED),
    (Mk $false $VK_TAB $SCAN_TAB "`t" $SHIFT_PRESSED),
    (Mk $false $VK_SHIFT $SCAN_SHIFT ([char]0) ([uint32]0))
)
$r = Send-Records -Handle $h -Records $recs -SingleCall -Tag ("SHIFTTAB {0}" -f $Label)
Close-ConIn $h
Write-Output ("SHIFTTAB label={0} written={1}/{2}" -f $Label, $r.Written, $r.Total)
