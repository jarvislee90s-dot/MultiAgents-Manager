# verify-stamp.ps1 — 地面真值：在各 CLI 会话存储中搜索 stamp，返回命中与提交文本长度
# 用法：verify-stamp.ps1 -Stamp E2-20260918-181200-1 [-Cli claude|codex|kimi|all]
param(
    [Parameter(Mandatory = $true)][string]$Stamp,
    [ValidateSet('all', 'claude', 'codex', 'kimi')][string]$Cli = 'all'
)
$roots = @{
    claude = "$env:USERPROFILE\.claude\projects\C--Users-bunny-AppData-Local-Temp-mam-probe-m6r-proj"
    codex  = "$env:USERPROFILE\.codex\sessions"
    kimi   = "$env:USERPROFILE\.kimi-code\sessions\wd_mam-probe-m6r-proj_81a87464acd4"
}
$targets = if ($Cli -eq 'all') { @('claude', 'codex', 'kimi') } else { @($Cli) }
foreach ($c in $targets) {
    $hit = $false
    $files = Get-ChildItem $roots[$c] -Recurse -Filter *.jsonl -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending
    foreach ($f in $files) {
        $m = Select-String -Path $f.FullName -Pattern $Stamp -SimpleMatch -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($m) {
            $hit = $true
            # 提取含 stamp 的 user 文本长度（取该行 stamp 前后文本近似：报告行内 stamp 位置与行片段）
            $line = $m.Line
            $idx = $line.IndexOf($Stamp)
            $ctx = $line.Substring([Math]::Max(0, $idx - 40), [Math]::Min(240, $line.Length - [Math]::Max(0, $idx - 40)))
            "HIT cli=$c file=$($f.Name) time=$($f.LastWriteTime.ToString('HH:mm:ss'))"
            # 送达文本长度定量：抽 "text":"…stamp…" 或 "content":"…stamp…" 字段（claude 用 content）
            $pat = '"(?:text|content)":"([^"]*' + [regex]::Escape($Stamp) + '[^"]*)"'
            if ($line -match $pat) { "  TEXTLEN=$($Matches[1].Length)" } else { "  TEXTLEN=? (字段未匹配)" }
            "  CTX: $ctx"
            break
        }
    }
    if (-not $hit) { "MISS cli=$c (files=$($files.Count))" }
}
