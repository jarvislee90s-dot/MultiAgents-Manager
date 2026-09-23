# ConIn.ps1 — M6R 修订版核心库（修订基线：旧 mam-probe\ConIn.ps1，2026-09-18）
# E0.2/E0.3 修订点：
#   1) 新增 GetConsoleMode / GetNumberOfConsoleInputEvents / PeekConsoleInput / CloseHandle / VkKeyScanW / MapVirtualKeyW
#   2) 所有 WriteConsoleInput 调用点统一经 Send-Records：必记 BOOL / 实写数 / GetLastWin32Error + 写前占用率
#   3) ESC 一律 [char]27 构造（PS 5.1 不认识 `e 转义——M6 bracketed-paste 实验作废根因）
#   4) 部分写处理：ok=true 且 written<分片长 → 按实写数推进偏移续写，不得整片前进
#   5) 日志追加式（Add-Content 到 $global:ConInLog），文件名由调用脚本给 run-id，永不覆盖
#   6) 退出必经 Close-ConIn（CloseHandle + FreeConsole 复位）——防滞留附加态（M9R 评审 P1-1 同源教训）
# 保留 M6 实测修正（勿回退）：
#   - CONIN$ 经 CreateFileW 打开（AttachConsole 后 GetStdHandle 是陈旧句柄，必败）
#   - INPUT_RECORD 拍平为 20 字节 Sequential blittable（嵌套 union 显式布局封送报 0x8007007A）
#   - 本库永不导入/调用 FlushConsoleInputBuffer（会吃掉目标控制台真实键盘输入）
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Collections.Generic;

public static class ConIn
{
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool FreeConsole();

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool AttachConsole(uint dwProcessId);

    [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern IntPtr CreateFileW(string lpFileName, uint dwDesiredAccess, uint dwShareMode, IntPtr lpSecurityAttributes, uint dwCreationDisposition, uint dwFlagsAndAttributes, IntPtr hTemplateFile);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool WriteConsoleInput(IntPtr hConsoleInput, INPUT_RECORD[] lpBuffer, uint nLength, out uint lpNumberOfEventsWritten);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool ReadConsoleInput(IntPtr hConsoleInput, INPUT_RECORD[] lpBuffer, uint nLength, out uint lpNumberOfEventsRead);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool PeekConsoleInput(IntPtr hConsoleInput, INPUT_RECORD[] lpBuffer, uint nLength, out uint lpNumberOfEventsRead);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool GetConsoleMode(IntPtr hConsoleInput, out uint lpMode);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool GetNumberOfConsoleInputEvents(IntPtr hConsoleInput, out uint lpcNumberOfEvents);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr hObject);

    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern short VkKeyScanW(char ch);

    [DllImport("user32.dll")]
    public static extern uint MapVirtualKeyW(uint uCode, uint uMapType);

    [StructLayout(LayoutKind.Sequential)]
    public struct INPUT_RECORD
    {
        public ushort EventType;
        public ushort Reserved;
        public int bKeyDown;            // Win32 BOOL
        public ushort wRepeatCount;
        public ushort wVirtualKeyCode;
        public ushort wVirtualScanCode;
        public char UnicodeChar;
        public uint dwControlKeyState;
    }

    public const ushort KEY_EVENT_TYPE = 0x0001;
    public const ushort VK_RETURN = 0x0D;
    public const ushort VK_BACK = 0x08;
    public const uint GENERIC_READ = 0x80000000;
    public const uint GENERIC_WRITE = 0x40000000;
    public const uint FILE_SHARE_READ = 0x1;
    public const uint FILE_SHARE_WRITE = 0x2;
    public const uint OPEN_EXISTING = 3;

    // FreeConsole+AttachConsole 后 GetStdHandle 返回旧控制台的陈旧句柄；
    // 必须以 CONIN$ 打开「附加后的目标控制台输入缓冲区」（GENERIC_READ|GENERIC_WRITE）。
    public static IntPtr OpenConIn()
    {
        return CreateFileW("CONIN$", GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, IntPtr.Zero, OPEN_EXISTING, 0, IntPtr.Zero);
    }

    // 可打印文本 → 字符事件。withScan=true 时 vk=VkKeyScanW 派生、scan=MapVirtualKeyW 非零派生（ccdr 硬约束）；
    // withScan=false 时 vk=0/scan=0（VT 序列字符流形态）。
    public static INPUT_RECORD[] TextToRecords(string text, bool paired, bool withScan)
    {
        var records = new List<INPUT_RECORD>();
        foreach (char c in text)
        {
            ushort vk = 0; ushort scan = 0;
            if (withScan)
            {
                short v = VkKeyScanW(c);
                if (v != -1) { vk = (ushort)(v & 0xFF); scan = (ushort)MapVirtualKeyW(vk, 0); }
            }
            if (paired)
            {
                records.Add(MakeRecord(true, vk, scan, c));
                records.Add(MakeRecord(false, vk, scan, c));
            }
            else
            {
                records.Add(MakeRecord(true, vk, scan, c));
            }
        }
        return records.ToArray();
    }

    // 固定 vk/scan 的字符事件（Enter=VK_RETURN'\r'、退格=VK_BACK'\b'、VT 序列 vk=0/scan=0 皆可走此口）
    public static INPUT_RECORD[] CharsToRecords(string text, ushort vk, ushort scan, bool paired)
    {
        var records = new List<INPUT_RECORD>();
        foreach (char c in text)
        {
            if (paired)
            {
                records.Add(MakeRecord(true, vk, scan, c));
                records.Add(MakeRecord(false, vk, scan, c));
            }
            else
            {
                records.Add(MakeRecord(true, vk, scan, c));
            }
        }
        return records.ToArray();
    }

    public static INPUT_RECORD[] EnterRecords(bool paired)
    {
        return CharsToRecords("\r", VK_RETURN, (ushort)MapVirtualKeyW(VK_RETURN, 0), paired);
    }

    public static INPUT_RECORD[] BackspaceRecords(int count, bool paired)
    {
        return CharsToRecords(new string('\b', count), VK_BACK, (ushort)MapVirtualKeyW(VK_BACK, 0), paired);
    }

    private static INPUT_RECORD MakeRecord(bool down, ushort vk, ushort scan, char c)
    {
        var r = new INPUT_RECORD();
        r.EventType = KEY_EVENT_TYPE;
        r.bKeyDown = down ? 1 : 0;
        r.wRepeatCount = 1;
        r.wVirtualKeyCode = vk;
        r.wVirtualScanCode = scan;
        r.UnicodeChar = c;
        return r;
    }
}
"@

# 追加式日志：由调用脚本先 Set-ConInLog 设置 run-id 文件名；永不覆盖
function Set-ConInLog([string]$Path) {
    $global:ConInLog = $Path
    $dir = Split-Path -Parent $Path
    if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
}

function Write-Log([string]$Message) {
    if (-not $global:ConInLog) { $global:ConInLog = "$env:USERPROFILE\mam-probe-m6r\logs\conin-default.log" }
    $line = "{0} {1}" -f (Get-Date -Format 'yyyy-MM-ddTHH:mm:ss.fff'), $Message
    Add-Content -Path $global:ConInLog -Value $line
    # Write-Host（信息流）：不污染函数返回管道；控制台可见供 bash 编排读取
    Write-Host $line
}

function New-RunId {
    return Get-Date -Format 'yyyyMMdd-HHmmss'
}

# 生命周期：FreeConsole → AttachConsole(目标) → CONIN$。任一步失败返回 $null（err 已记日志）
function Open-TargetConsole([uint32]$TargetPid) {
    [void][ConIn]::FreeConsole()
    $attached = [ConIn]::AttachConsole($TargetPid)
    $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    if (-not $attached) {
        Write-Log ("ATTACH FAIL target={0} err={1}" -f $TargetPid, $err)
        return $null
    }
    Write-Log ("ATTACH OK target={0}" -f $TargetPid)
    $h = [ConIn]::OpenConIn()
    if ($h -eq [IntPtr]::Zero -or $h -eq [IntPtr](-1)) {
        Write-Log ("OPEN CONIN FAIL err={0}" -f [Runtime.InteropServices.Marshal]::GetLastWin32Error())
        [void][ConIn]::FreeConsole()
        return $null
    }
    Write-Log "OPEN CONIN OK"
    return $h
}

# 退出必经：CloseHandle + FreeConsole 复位（防滞留附加态）
function Close-ConIn([IntPtr]$Handle) {
    if ($Handle -ne [IntPtr]::Zero -and $Handle -ne [IntPtr](-1)) { [void][ConIn]::CloseHandle($Handle) }
    $ok = [ConIn]::FreeConsole()
    Write-Log ("DETACH FreeConsole ok={0}" -f $ok)
}

# 占用率（不消费事件，无副作用）
function Get-Occupancy([IntPtr]$Handle) {
    $n = [uint32]0
    [void][ConIn]::GetNumberOfConsoleInputEvents($Handle, [ref]$n)
    return [int]$n
}

# 输入模式标志位解码（E1 判据核心：LINE_INPUT / VT_INPUT）
function Get-ModeString([IntPtr]$Handle) {
    $m = [uint32]0
    $ok = [ConIn]::GetConsoleMode($Handle, [ref]$m)
    $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    if (-not $ok) { return "GetConsoleMode FAIL err=$err" }
    $names = @()
    if ($m -band 0x0001) { $names += 'PROCESSED_INPUT' }
    if ($m -band 0x0002) { $names += 'LINE_INPUT' }
    if ($m -band 0x0004) { $names += 'ECHO_INPUT' }
    if ($m -band 0x0008) { $names += 'WINDOW_INPUT' }
    if ($m -band 0x0010) { $names += 'MOUSE_INPUT' }
    if ($m -band 0x0020) { $names += 'INSERT_MODE' }
    if ($m -band 0x0040) { $names += 'QUICK_EDIT' }
    if ($m -band 0x0080) { $names += 'EXTENDED_FLAGS' }
    if ($m -band 0x0100) { $names += 'AUTO_POSITION' }
    if ($m -band 0x0200) { $names += 'VT_INPUT' }
    return ("mode=0x{0:X4} [{1}]" -f $m, ($names -join '|'))
}

# E0.3 修订版分块写入：逐调用记 ok/written/err/写前占用率；部分写按实写数推进；失败重试有预算
# 返回 @{ Written; Total; Calls; FailCalls; FirstFailErr }
function Send-Records {
    param(
        [IntPtr]$Handle,
        [ConIn+INPUT_RECORD[]]$Records,
        [int]$Chunk = 2,            # 每次调用写入的事件数（成对事件=2 事件/字符）
        [int]$PaceMs = 8,           # 块间间隔
        [int]$MaxRetry = 1100,      # 失败重试预算（80ms/次 ≈ 88s，对齐 M6 原条件）
        [int]$RetryMs = 80,
        [switch]$SingleCall,        # 整条记录单批原子写（VT 序列硬约束）
        [string]$Tag = "SEND"
    )
    $total = $Records.Length
    $writtenTotal = 0
    $calls = 0
    $failCalls = 0
    $firstFailErr = -1
    $retries = 0
    $i = 0
    while ($i -lt $total) {
        if ($SingleCall) { $end = $total } else { $end = [Math]::Min($i + $Chunk, $total) }
        $slice = New-Object 'ConIn+INPUT_RECORD[]' ($end - $i)
        [Array]::Copy($Records, $i, $slice, 0, $end - $i)
        $occBefore = Get-Occupancy $Handle
        $w = [uint32]0
        $ok = $false
        $err = 0
        $exMsg = ""
        try {
            $ok = [ConIn]::WriteConsoleInput($Handle, $slice, [uint32]$slice.Length, [ref]$w)
            $err = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        } catch {
            $exMsg = $_.Exception.Message
            $err = -1
        }
        $calls++
        if ($exMsg -ne "") {
            Write-Log ("{0} call={1} off={2}/{3} EXCEPTION={4}" -f $Tag, $calls, $i, $total, $exMsg)
        } else {
            Write-Log ("{0} call={1} off={2}/{3} n={4} ok={5} written={6} err={7} occBefore={8}" -f $Tag, $calls, $i, $total, $slice.Length, $ok, $w, $err, $occBefore)
        }
        if ($ok) {
            # E0.3 修订：ok=true 但 written<分片长 = 部分写，按实写数推进（不得整片前进）
            $writtenTotal += [int]$w
            $i += [int]$w
            if (-not $SingleCall) { Start-Sleep -Milliseconds $PaceMs }
        } else {
            $failCalls++
            if ($firstFailErr -lt 0) { $firstFailErr = $err }
            $retries++
            if ($retries -gt $MaxRetry) {
                Write-Log ("{0} RETRY_BUDGET_EXHAUSTED at off={1}/{2}" -f $Tag, $i, $total)
                break
            }
            Start-Sleep -Milliseconds $RetryMs
        }
    }
    return @{ Written = $writtenTotal; Total = $total; Calls = $calls; FailCalls = $failCalls; FirstFailErr = $firstFailErr }
}
