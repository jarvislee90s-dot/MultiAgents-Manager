# 已知族规格表（P1 定族后必读）

> 更新惯例：每探明一个新工具，在本文件追加/修订对应族行，带版本号与日期。**任何「硬约束」都随 TUI 版本漂移，规格必须带版本号 + 复验提示。**

## 版本漂移教训（先读这个）

同场景开源先例 cc-discord-remote 的三条「硬约束」——①scan code 必须非零；②claude 2.1.158+ 忽略合成 VK 方向键；③方向键必须 VT——在 M6R 本机（claude 2.1.251 / codex 0.154.0 / Win11 26200）**全部不成立**：scan=0 全程零失败；VK 方向键经 conhost RAW_VT 转码与 VT 等价；无双写。结论：**别把别家实证当本机事实，也别把本机事实当永久事实**。规格表三要素：版本号、实测值、复验触发条件（大版本升级）。

## 跨族共同基线（M6R 定案，E5 90/90 实证）

| 项 | 值 |
|---|---|
| 分块 | 80 字符/块，每块单次 WriteConsoleInput（160 事件），块间 50ms |
| 提交回车 | 正文块后 150ms 发 Enter（VK 形态） |
| 事件形态 | keydown+keyup 成对；vk=VkKeyScanW(ch)&0xFF；scan=MapVirtualKeyW(vk,0)（非零，保守项——scan=0 亦可用）；UnicodeChar=ch |
| 长度 | 注入层不截断（上限在调用方传输层）；万字符实测 8.1~9.3s |
| 写入校验 | 每次核 BOOL+实写数；written<n 从偏移续写；仅 ok=false 时读错误码 |
| 写入确认 | 会话文件 stamp 命中为主、屏幕为辅；确认失败→失败回执 |
| 背压 | 软背压可选；占用率 >0 持续 >5s 判异常（瞬时积压 ≤~120 事件亚秒消化属正常） |
| 生命周期 | 一次性子进程：FreeConsole→AttachConsole→CONIN$→…→CloseHandle→FreeConsole；进程级串行 |
| 异常恢复 | 宿主选择模式冻结 → PostMessage(宿主窗口, WM_KEYDOWN/WM_KEYUP, VK_ESCAPE) 解冻 |

## A 族 · RAW_VT（mode 0x0208：WINDOW_INPUT|VT_INPUT）

Node/libuv·ink 系 TUI。conhost 处于 VT 输入模式：注入的 KEY_RECORD 会被 conhost **转码为 VT 序列**再投递给应用——两种事件形态在交付层等价。

| 项 | 规格 | 实证成员 |
|---|---|---|
| 字符事件 | 任意形态（vk=0 纯字符可；带 vk/scan 亦可） | claude 2.1.251 ✅ kimi 2.0.0 ✅ opencode 1.18.31 ✅ |
| 回车/控制键 | VK 事件与字符形态皆可；统一 VK（跨族安全） | claude/kimi ✅（信任框 VK 回车即时生效）opencode ✅（P4 双形态对照 HIT） |
| 方向键 | VT 序列（整条单批原子写）或 VK 形态皆生效（转码等价）；注意各工具键位语义不同（opencode 空框上箭头无历史召回，但左移光标双形态实证） | claude/kimi ✅（历史召回实证）opencode ✅（光标插入实证） |
| bracketed-paste | 原生可用（TEXTLEN 精确/框内容零泄漏） | claude/kimi ✅ opencode ✅ |
| busy 期 | 字符排入输入草稿（原生 mid-turn 队列）——claude 未本地复验（引用 M6/ccdr 先例），kimi/codex/opencode 已实测 ✅；opencode busy 打字会与下一次提交合并 | opencode ✅（12 探针字符入草稿保留） |
| 已知修剪 | 尾随空格被 TUI 去除（kimi/opencode 实证；长度对账按语义归一） | kimi ✅ opencode ✅ |
| **消费速率** | claude/kimi 即时排空；**opencode 慢消费（~70 事件/s）——长文必须消费感知节流（背压 DrainTo≤40 事件），深队列下提交回车可能丢失** | opencode ✅（10k 背压 110-183s 全量精确；固定节流 6 万事件队列丢 1 回车） |
| 确认适配 | 会话文件确认每家不同：opencode 为 SQLite（拷贝 db+wal+shm 副本查 part/message 表 data JSON），grep 不可用 | opencode ✅（verify-oc.py 模式） |
| 多实例行为 | opencode 同项目跨窗口草稿实时同步（未提交草稿互串）——引擎/探测单实例假设 | opencode ✅（过程实录） |

## B 族 · crossterm（mode 0x01F0：MOUSE|INSERT|QUICK_EDIT|EXTENDED|AUTO_POSITION）

Rust crossterm 系 TUI，直读 KEY_RECORD、不过 conhost VT 转码。**对事件形态挑剔**：

| 项 | 规格 | 实证成员 |
|---|---|---|
| 字符事件 | 可打印字符任意形态（keyup 被过滤，无双重写入） | codex 0.154.0 ✅ |
| 回车/控制键 | **必须 VK 事件**（vk=0 纯字符 '\r' 被丢弃——「打完字不提交」的根因） | codex ✅（信任框对照实锤） |
| 方向键 | **走 VK 形态**（按家分支；VT 原子批未定案，且裸 ESC 被当按键吃有 cancel 风险） | codex ✅（VK 召回实证） |
| bracketed-paste | **禁用**：序列头两个 ESC 被当 Esc 键吃掉、`[200~`/`[201~` 泄漏为字面文本 | codex ✅（TEXTLEN=92−2 实证） |
| 裸 ESC 字符流 | **绝对禁止**（cancel 语义风险）；拒绝键必须 VK 形态 | codex ⚠ 实证推导 |
| busy 期 | 照常即时消费、字符入草稿 | codex ✅（55 探针全入框） |

## 按家分支速查（注入引擎消费）

| 行为 | A 族 | B 族 |
|---|---|---|
| 回车/Esc/Tab | VK 形态 | VK 形态（**唯一选择**） |
| 方向键 | VT 序列原子批 | VK 形态 |
| bracketed-paste | 可用 | 禁用 |
| 长度对账 | 原文精确 | 原文精确（注意各家修剪） |

## 平台差异（macOS 投影，2026-09-19 Mac 实机验收实证）

macOS 三通道（tmux `send-keys` / iTerm2 `write text` / Terminal `do script`）走官方 API 写文本，**不存在 Windows 的事件形态学问题**（无 vk/scan/按族分支）；族差异在 macOS 的主要投影是**回车提交行为**——且该维度与 Windows 族划分**不完全同构**（回车以文本字符送达，各 TUI 对尾随换行的处理是逐家行为），故 macOS 回车行为必须按工具实测入档，这也是 `mac-verification.md` 四点清单的固定验证点：

| 工具（版本） | macOS 自动回车 | 备注 |
|---|---|---|
| claude 2.1.278 | ✅ 正常提交 | ink 系对 \n 直接提交 |
| codex 0.155.1 | ❌ 被吞（文本滞留输入框） | bracketed-paste 嫌疑；降级口径=手动按一次回车（M3B 文案已内置）；补发 keystroke 方案待 Mac 探测 |
| kimi 2.0.1 | ❌ 被吞（同 codex） | 同上 |
| opencode 1.18.31 | ✅ 正常 | 慢消费者长文预算内（2000 字 3.6s） |

**macOS 平台已知环境项**（非工具差异，处置见 `mac-verification.md`）：TCC 自动化授权（-1743）、iTerm2 3.7.2 + macOS 26 建窗不稳与 AppleScript 引擎卡死（-1712）、锁屏态仅 tmux 通道实测过。
