# 探测脚本套件（M6R 验证版 + opencode 实测迭代）

来源：MAM M6R 探测（2026-09-18，经独立评审逐行核验）+ opencode 实测（2026-09-19）新增三件与 WT 定位修复。PowerShell 5.1 环境。

## 路径约定（先读）

脚本内多处引用探测主目录 `$env:USERPROFILE\mam-probe-m6r\`（dot-source ConIn.ps1、logs/、evidence/）。两种用法：

1. 该目录存在（MAM 机器）→ 直接用；
2. 不存在 → 先建 `%USERPROFILE%\mam-probe-m6r\{logs,evidence}\`，或把脚本内该路径批量替换为本次探测目录（每脚本仅 1~3 处）。

**新工具适配点（务必检查）**：
- `launch-session.ps1` 的 ValidateSet 目前为 claude/codex/kimi/opencode——新 CLI 需加一行或手动起会话（拓扑：`conhost cmd /k <cli>`，用 PowerShell Start-Process 起手；直接 spawn 会得 0x80070005）。**WT 分支定位用启动前进程快照差分**（v2 已修——cmd 命令行不含项目目录，勿按 CommandLine 匹配）；
- `verify-stamp.ps1` 的会话文件根路径是 JSONL 三家硬编码——新工具在 P2 定位其会话文件后改 `$roots` 映射（这是预期工作，不是缺陷）；
- **非文本存储（SQLite 等）grep 不可用**：拷贝 db+wal+shm 副本后查询对账，参考实现 `verify-oc.py`（opencode），勿查活库。

## 脚本清单

| 脚本 | 用途 | 关键参数 |
|---|---|---|
| `ConIn.ps1` | FFI 基座（dot-source，不直接跑）：附加/CONIN$/写记录/读模式/读占用/复位 | Send-Records、Get-Mode、Get-Occ、Send-Keys 等（读源码顶部注释） |
| `launch-session.ps1` | 起探测会话（conhost/wt × 四家 CLI），记 PID 台账（WT 分支快照差分定位 v2） | -HostKind -Cli -ProjDir |
| `check-sessions.ps1` | 列当前探测会话进程 | — |
| `t0-env.ps1` | 环境基线记录（OS/PS/WT/CLI 版本） | — |
| `probe-mode.ps1` | **P1 定族**：采目标输入模式标志 | -TargetPid -Label |
| `probe-e5.ps1` | **P3 长度阶梯**：80/50ms/150ms 参数化注入 + 会话 stamp | -TargetPid -Stamp -Length -Block -PaceMs -SubmitDelayMs |
| `probe-key2.ps1` | **P4/P5 单键**：VK/字符/方向键/BP 各形态对照 | -TargetPid -Vk/-Char/-VtName（down/up/left/right/esc/enter/bpOn/bpOff） |
| `probe-type.ps1` | 打字/退格清理/busy 探针 | -TargetPid -Text -Backspace -DownOnly |
| `probe-oc-paced.ps1` | **背压节流注入**（慢消费者用：每块后轮询占用率回落再续） | -TargetPid -Stamp -Length -DrainTo |
| `observe-occ.ps1` | 缓冲占用率曲线（后台采样） | -TargetPid -Seconds -Tag |
| `probe-postmsg.ps1` | **冻结解冻**：向宿主窗口 PostMessage 真键盘消息 | -Hwnd -Vk |
| `enum-windows.ps1` | 枚举窗口找宿主 hwnd（Win11 26200 conhost 窗口归 cmd 进程） | — |
| `probe-shot-win.ps1` | PrintWindow 按窗口截图（锁屏可用） | -ProcId -Name |
| `probe-shot-wintitle.ps1` | 按窗口标题截图（**WT 标签页**用——窗口归属 WindowsTerminal 进程，按 PID 抓不到目标标签） | -Title -Name |
| `verify-stamp.ps1` | 会话文件 stamp 命中对账（JSONL 家族，写入确认根基） | -Stamp -Cli |
| `verify-oc.py` | SQLite 存储对账参考实现（opencode：拷贝 db+wal+shm 副本查副本） | -Stamp |

## 通用纪律（与 SKILL.md 八条铁律一致）

- 日志追加式带 run-id 命名，永不覆盖；
- 写入必记 BOOL/实写数/错误码（仅 ok=false 时读 err）；
- ok=true 且实写<分片长 → 从偏移续写；
- 结束 taskkill 清场；探测进程不滞留附加态（ConIn 自带 FreeConsole 复位）；
- PowerShell 5.1：ESC 用 `[char]27`；严禁经命令行传 `\r` 转义（脚本已内置 -VtName enter / -EnterVK 类开关）。
