# 功能规格说明：Windows 窗口级终端跳转

**功能分支**：`007-windows-window-jump`

**创建日期**：2026-08-24

**状态**：草稿

**输入**：Windows 兼容性修复后的用户需求——CLI 会话（Claude / Codex / OpenCode / OpenClaw）与 ChatGPT 桌面版（App 形态 Codex 会话）都应支持点击卡片跳转聚焦。macOS 已有 TTY + AppleScript 链路（tmux → iTerm2 → Terminal.app），Windows 目前以 `jump_supported_for` 平台门控整体禁用。经设计决策确认：MVP 做窗口级聚焦（不做标签页级），App 形态跳转一并实现，SessionCard 文案同步接入 i18n。

## 用户场景与测试

### 用户故事 1 — Windows 用户点击卡片跳转到终端（优先级: P1）

用户在 Windows Terminal / PowerShell / Git Bash 等终端里跑着多个 AI CLI 会话，希望点击首页卡片直接聚焦到对应终端窗口，继续人工介入。

**优先级理由**：跳转是会话看板的核心交互（macOS 已有），Windows 缺失等于核心功能减半。

**独立测试**：两个终端窗口分别跑 claude（不同项目），点击其中一个卡片，对应终端窗口被置前并获得焦点。

**验收场景**：

1. **给定** 用户在 Windows Terminal 中运行 `claude`，**当** 点击对应会话卡片，**则** 该 Windows Terminal 窗口被置前获得焦点
2. **给定** 终端窗口处于最小化状态，**当** 点击卡片，**则** 窗口被恢复并置前
3. **给定** 独立 PowerShell 7 / Git Bash (mintty) 窗口中运行 opencode，**当** 点击卡片，**则** 对应窗口被置前
4. **给定** VS Code 集成终端中运行 claude，**当** 点击卡片，**则** VS Code 窗口被置前（终端所在窗口，不要求定位到具体终端面板）
5. **给定** 同一终端窗口内有多个标签页，**当** 点击任一会话卡片，**则** 窗口被置前（MVP 接受停留在当前标签页，标签页级定位为二期）

### 用户故事 2 — 跳转到 ChatGPT 桌面版（优先级: P2）

用户在 ChatGPT 桌面版（内嵌 Codex）里有会话，希望点击 App 形态卡片直接聚焦 ChatGPT 窗口。

**优先级理由**：Windows 上聚焦 GUI 窗口比终端更简单可靠，App 形态没有理由继续禁用。

**独立测试**：ChatGPT 窗口最小化或被其他窗口遮挡时，点击 Codex App 会话卡片，ChatGPT 窗口被恢复并置前。

**验收场景**：

1. **给定** ChatGPT 桌面版在后台运行且有 Codex 会话，**当** 点击该会话卡片，**则** ChatGPT 主窗口被置前
2. **给定** ChatGPT 窗口最小化，**当** 点击卡片，**则** 窗口被恢复并置前

### 用户故事 3 — 提示文案准确且可切换语言（优先级: P3）

不可跳转的卡片（如 macOS 上的 App 形态会话、Linux 上全部会话）点击时应得到准确的原因提示；卡片文案跟随界面语言（中/英）切换。

**优先级理由**：当前文案"桌面 APP 形态不支持终端跳转"对 Windows CLI 会话是错误归因（实为平台未实现），且为硬编码中文。

**独立测试**：切换界面语言为 English 后，卡片的悬停提示与点击 toast 均为英文。

**验收场景**：

1. **给定** 界面语言为中文，**当** 点击不可跳转的卡片，**则** toast 提示"当前平台或形态不支持跳转"
2. **给定** 界面语言为 English，**当** 悬停不可跳转的卡片，**则** 提示为英文（与中文语义一致）
3. **给定** 界面语言切换，**当** 查看会话卡片，**则** 卡片内所有原有硬编码文案（跳转提示等）跟随切换

### 用户故事 4 — macOS 无回归（优先级: P1）

**验收场景**：

1. **给定** macOS 环境，**当** 点击 CLI 会话卡片，**则** 走现有 tmux → iTerm2 → Terminal.app 聚焦链路，行为与修复前一致
2. **给定** macOS 环境，**当** 点击 App 形态卡片，**则** 维持现状（提示不支持，不尝试聚焦）

## 设计

### 1. 核心机制：进程祖先链 + EnumWindows（`src-tauri/src/window/win32.rs`）

统一机制，CLI 与 App 形态共用，无需特判：

```
输入: agent 进程 PID（CLI: claude.exe/codex.exe/...；App: ChatGPT 内嵌 codex.exe）
  1. 沿父进程链收集 PID 集合（agent → shell → 终端宿主 → ... → 直至无父进程）
     - CLI 场景: 链上含 WindowsTerminal.exe / mintty / conhost / Code.exe 等有窗口的宿主
     - App 场景: 链上含 ChatGPT.exe 主进程（codex.exe 的父进程即 ChatGPT 主进程）
  2. EnumWindows 按 Z 序枚举可见顶层窗口
     - GetWindowThreadProcessId 取窗口所属 PID，命中祖先链集合即为候选
     - 排除工具窗口（WS_EX_TOOLWINDOW）与不可见窗口（IsWindowVisible）
     - 取第一个命中（Z 序最上）
  3. 聚焦: IsIconic → ShowWindow(SW_RESTORE)；SetForegroundWindow(hwnd)
```

- 纯 Win32 API 调用，不 spawn 任何子进程（无闪窗风险）
- `focus_session` IPC 命令（`commands/session.rs`）现有实现已用 `System::new_all()` 查进程，父链收集复用该 System 快照
- 前台锁说明：点击卡片时 MAM 自身是前台进程且发生过用户交互，`SetForegroundWindow` 通常被允许；若返回失败，降级为 `SwitchToThisWindow(hwnd, true)` 再试一次，仍失败则返回用户可读错误

### 2. 平台分发（`src-tauri/src/window/mod.rs`）

`focus_terminal_for_pid` 按平台分发：`#[cfg(windows)]` 走 `win32::focus_window_for_pid`（传入父链 PID 集合）；`#[cfg(target_os = "macos")]` 维持现有 TTY 链路不变；其余平台返回明确错误（现状）。Wayland 检测维持现状。

### 3. 跳转能力放开（`src-tauri/src/session/model.rs`）

`jump_supported_for(form)`：Windows → Cli 与 App 均为 true；macOS → 仅 Cli（现状）；其他平台 → false（现状）。同名测试的平台条件断言同步更新。

### 4. 依赖

`[target.'cfg(windows)'.dependencies]` 新增 `windows` crate（仅需 `Win32_Foundation`、`Win32_UI_WindowsAndMessaging` feature），不引入 UI Automation（二期标签页定位才需要）。

### 5. SessionCard 文案与 i18n（`src/components/sessions/SessionCard.tsx`）

- 不可跳转提示统一为单条通用文案（键名如 `sessions.jumpUnsupported`），不再区分具体原因——跳转能力放开后，剩余不可跳转场景（macOS App / Linux 全部）用同一提示即可，消除错误归因
- 卡片内其余硬编码中文（悬停 title、toast 等）一并接入 i18n：`src/i18n/locales/zh.json` / `en.json` 新增对应键，两语言键集保持对齐（现有 93/93 对齐基线不得打破）
- 范围仅限 SessionCard.tsx 一个文件，其余 16 个硬编码组件不在本 spec 范围

### 6. 明确不做（YAGNI）

- Windows Terminal 标签页级定位（UI Automation，二期）
- macOS 的 ChatGPT App 窗口聚焦（AppleScript `activate`，未来按需）
- Linux / Wayland 终端聚焦
- 跨完整性级别（提权终端）的强制置前保障——`SetForegroundWindow` 失败时给出错误提示即可

## 错误处理

- 祖先链上找不到任何有窗口的进程（如 agent 进程父链已全部退出）：返回错误"未找到可聚焦的窗口（终端可能已关闭）"
- `SetForegroundWindow` 与降级尝试均失败：返回错误"窗口聚焦被系统拒绝"，前端 toast 展示
- 所有 Win32 调用失败不得 panic，统一走 `Result<(), String>`

## 测试策略

- 纯逻辑单测：父链收集函数（给定进程树快照 → PID 集合）抽为纯函数可测；`jump_supported_for` 平台条件断言更新（Windows 期待 Cli+App 均可跳）
- Win32 枚举/聚焦部分不做自动化测试（需要真实窗口交互），由人工验证清单覆盖
- 人工验证：验收场景逐条实机执行——Windows Terminal / 独立 PowerShell / Git Bash / VS Code 集成终端 / 最小化恢复 / ChatGPT App / 中英文切换；macOS 回归场景如有条件执行
- 门禁：`cargo test`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`pnpm lint` 全过
