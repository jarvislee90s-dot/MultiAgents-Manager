# launch-session.ps1 — E0.5 会话准备 v2：轻量定位（-PassThru 拿宿主 pid → 逐层直查子进程，无全树 BFS）
# 定位规则：conhost(-PassThru) → cmd → TUI 主进程（<cli>.exe 精确名优先；node/bun 按精确包路径匹配）
param(
    [Parameter(Mandatory = $true)][ValidateSet('conhost', 'wt')][string]$HostKind,
    [Parameter(Mandatory = $true)][ValidateSet('claude', 'codex', 'kimi', 'opencode')][string]$Cli,
    [string]$ProjDir = "$env:TEMP\probe-proj",
    [string]$RunId = (Get-Date -Format 'yyyyMMdd-HHmmss')
)
$ErrorActionPreference = 'Continue'
$root = "$env:USERPROFILE\mam-probe-m6r"
New-Item -ItemType Directory -Force -Path "$root\evidence\sessions" | Out-Null
if (-not (Test-Path $ProjDir)) { New-Item -ItemType Directory -Force -Path $ProjDir | Out-Null }

$hostProc = $null
$cmd = $null
$tui = $null

$beforeCmds = @(Get-CimInstance Win32_Process -Filter "Name='cmd.exe'" | Select-Object -ExpandProperty ProcessId)
$t0 = Get-Date

if ($HostKind -eq 'conhost') {
    # M6/M9 稳定拓扑：conhost cmd /k claude。参数必须数组传递（单字符串会被整体引号包裹导致 CreateProcess 失败）
    $hostProc = Start-Process -FilePath 'conhost.exe' -ArgumentList @('cmd.exe', '/k', $Cli) -WorkingDirectory $ProjDir -PassThru
} else {
    # M6 教训：wt.exe 参数需单字符串（cmd 内 cd）；wt 立即退出，不 PassThru
    Start-Process -FilePath 'wt.exe' -ArgumentList ("-d `"$ProjDir`" cmd /k $Cli") | Out-Null
}

# 1) 找 cmd（conhost 宿主：直查 conhost 子进程；WT：启动前快照差分+创建时间——cmd 命令行不含项目目录，勿按 CommandLine 匹配）
$deadline = (Get-Date).AddSeconds(25)
while ((Get-Date) -lt $deadline -and -not $cmd) {
    Start-Sleep -Milliseconds 1200
    if ($HostKind -eq 'conhost') {
        $cmd = Get-CimInstance Win32_Process -Filter "ParentProcessId=$($hostProc.Id)" -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -eq 'cmd.exe' } | Select-Object -First 1
    } else {
        $cmd = Get-CimInstance Win32_Process -Filter "Name='cmd.exe'" -ErrorAction SilentlyContinue |
            Where-Object { $beforeCmds -notcontains $_.ProcessId -and $_.CreationDate -ge $t0.AddSeconds(-2) } | Select-Object -First 1
    }
}

# 2) 找 TUI 主进程（cmd 直接子进程 → 命中即停；最多再等 40s 等 CLI 启动）
if ($cmd) {
    $deadline = (Get-Date).AddSeconds(40)
    while ((Get-Date) -lt $deadline -and -not $tui) {
        Start-Sleep -Milliseconds 1500
        $kids = Get-CimInstance Win32_Process -Filter "ParentProcessId=$($cmd.ProcessId)" -ErrorAction SilentlyContinue
        foreach ($k in $kids) {
            if ($k.Name -eq "$Cli.exe") { $tui = $k; break }
            if ($k.Name -in @('node.exe', 'bun.exe')) {
                $pkg = 'claude-code'
                if ($Cli -ne 'claude') { $pkg = $Cli }
                if ($k.CommandLine -match $pkg) { $tui = $k; break }
            }
        }
    }
}

$tree = @()
$tree += "=== {0} × {1} launched {2} proj={3}" -f $HostKind, $Cli, (Get-Date -Format o), $ProjDir
if ($hostProc) { $tree += "HOST_PID={0} ({1})" -f $hostProc.Id, 'conhost' }
if ($cmd) { $tree += "CMD_PID={0}" -f $cmd.ProcessId } else { $tree += "CMD_PID= miss" }
if ($tui) {
    $tree += "TARGET_PID={0} NAME={1}" -f $tui.ProcessId, $tui.Name
} elseif ($cmd) {
    $tree += "TARGET_PID={0} NAME=cmd.exe(FALLBACK same console)" -f $cmd.ProcessId
} else {
    $tree += "TARGET_PID= FAIL"
}
$outFile = "$root\evidence\sessions\$HostKind-$Cli-$RunId.txt"
$tree | Tee-Object -FilePath $outFile
