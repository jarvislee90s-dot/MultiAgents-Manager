# t0-env.ps1 — E0.4 环境记录（对齐旧 T0-env 格式，落 mam-probe-m6r\evidence\t0-env.txt）
$ErrorActionPreference = 'Continue'
$root = "$env:USERPROFILE\mam-probe-m6r"
New-Item -ItemType Directory -Force -Path "$root\evidence" | Out-Null

$log = @()
$log += "=== M6R 环境基线 $(Get-Date -Format o) ==="
$log += "OS: $((Get-CimInstance Win32_OperatingSystem | ForEach-Object { $_.Caption + ' build ' + $_.BuildNumber + ' (' + $_.OSArchitecture + ')' }))"
$log += "OSVersion: $($PSVersionTable.OSVersion)"
$log += "PSVersion: $($PSVersionTable.PSVersion)"
$log += "默认终端 DelegationConsole: $((Get-ItemProperty 'HKCU:\Console\%%Startup' -ErrorAction SilentlyContinue).DelegationConsole)"
$log += "默认终端 DelegationTerminal: $((Get-ItemProperty 'HKCU:\Console\%%Startup' -ErrorAction SilentlyContinue).DelegationTerminal)"
$wt = Get-AppxPackage Microsoft.WindowsTerminal -ErrorAction SilentlyContinue
$log += "WT 版本: $($wt.Version)"
$log += "WT PackageFullName: $($wt.PackageFullName)"
$log += "conhost 路径: $((Get-Command conhost -ErrorAction SilentlyContinue).Source)"
$log += "wt 路径: $((Get-Command wt -ErrorAction SilentlyContinue).Source)"
$log += "claude: $((claude --version) 2>&1)"
$log += "codex: $((codex --version) 2>&1)"
$log += "kimi: $((kimi --version) 2>&1)"
$log += "claude 路径: $((Get-Command claude -ErrorAction SilentlyContinue).Source)"
$log += "codex 路径: $((Get-Command codex -ErrorAction SilentlyContinue).Source)"
$log += "kimi 路径: $((Get-Command kimi -ErrorAction SilentlyContinue).Source)"
$log += "临时项目目录: $env:TEMP\mam-probe-m6r-proj"
$log | Tee-Object -FilePath "$root\evidence\t0-env.txt"
