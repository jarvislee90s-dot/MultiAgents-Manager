# 功能规格说明：OpenCode / OpenClaw 会话匹配修复

**功能分支**：`006-opencode-openclaw-session-matching`

**创建日期**：2026-08-24

**状态**：草稿

**输入**：Windows 兼容性修复两轮（591d3c1..0fdbd5c）后的实机诊断结论。进程层匹配（`exe_matches` 三级来源）已验证有效，但 OpenCode 会话卡片在 Windows 上仍不显示；OpenClaw 解析器存在同款代码，一并修复。

## 问题根因（实机取证）

诊断时 OpenCode 进程被正确发现（`find_opencode_processes` 返回 1 个，form=Cli，cwd=`E:\LLMproject\deepseek-harness\`），但 `get_opencode_sessions` 返回 0 个会话。失败在解析器内部的进程↔会话匹配，两个独立缺陷：

1. **路径分隔符不一致**：OpenCode 的 SQLite 数据库（`~/.local/share/opencode/opencode.db`）存储**正斜杠**路径（实测 `project.worktree = 'E:/LLMproject/deepseek-harness/deepseek-harness'`），而 sysinfo 返回的进程 cwd 是**反斜杠 + 尾部分隔符**（`E:\LLMproject\deepseek-harness\`）。现有归一化函数 `normalize_cwd_for_match`（上一轮引入）只做去尾部分隔符 + Windows 小写，**没有统一分隔符方向**，两侧仍不可比。
2. **匹配方向缺陷**：现有规则为 `cwd == worktree || cwd.starts_with(worktree + "/")`（进程 cwd 等于或在 worktree 之下）。实机场景中用户在 `E:/LLMproject/deepseek-harness` 启动 opencode，project 的 worktree 却是嵌套子目录 `.../deepseek-harness/deepseek-harness`——进程 cwd 是 worktree 的**祖先**，规则匹配不上。而 `session.directory`（`E:/LLMproject/deepseek-harness`）与进程 cwd 是精确对应的，语义上"会话在哪个目录启动"才是正确的匹配键。

`openclaw_parser.rs` 的 workspace 匹配（`cwd == workspace || starts_with`）为同款代码，存在相同缺陷。

Claude / Codex 解析器上一轮已修复且实测有效，本 spec 不改动其行为（仅共享归一化函数的增强，见下）。

## 用户场景与测试

### 用户故事 1 — Windows 用户看到 OpenCode / OpenClaw 会话（优先级: P1）

用户在 Windows 终端里运行 `opencode`（或 OpenClaw），期望 MultiAgents Manager 首页像 Claude / Codex 一样实时显示其会话卡片与运行状态。

**优先级理由**：四个工具中两个在 Windows 上完全不可见，属于功能失效。

**独立测试**：在终端启动 opencode 并开始对话后 5 秒内，首页出现 OpenCode 会话卡片，状态随对话进展变化。

**验收场景**：

1. **给定** 用户在 PowerShell 中 `cd E:\某项目` 后运行 `opencode` 并发送一条消息，**当** 查看首页，**则** 出现 OpenCode 会话卡片，项目名显示为目录名
2. **给定** 用户以小写盘符 `cd e:\某项目` 启动 opencode，**当** 查看首页，**则** 卡片同样出现（路径大小写不影响匹配）
3. **给定** 用户在项目父目录启动 opencode 而 opencode 将 worktree 登记为子目录（嵌套 git 场景），**当** 查看首页，**则** 卡片出现（按 `session.directory` 匹配成功）
4. **给定** 用户在项目子目录中启动 opencode（worktree 是其祖先），**当** 查看首页，**则** 卡片出现（worktree 前缀规则仍然生效）
5. **给定** macOS 用户正常运行 opencode，**当** 查看首页，**则** 行为与修复前一致（无回归）

### 用户故事 2 — 会话状态实时性与准确性（优先级: P2）

OpenCode 会话卡片的状态（处理中 / 等待输入等）应随实际对话进展更新，复用现有轮询机制。

**优先级理由**：只显示卡片不更新状态等于静态假信息。

**独立测试**：向 opencode 发送一个需要工具调用的任务，卡片状态在轮询周期内从"处理中"变为"等待输入"。

**验收场景**：

1. **给定** OpenCode 会话卡片已显示，**当** opencode 执行工具调用，**则** 卡片状态为处理中；任务完成后变为等待输入
2. **给定** 用户退出 opencode，**当** 下一轮轮询完成，**则** 卡片消失

## 设计

### 1. 归一化函数增强：统一分隔符方向

`normalize_cwd_for_match`（`src-tauri/src/monitor/parser.rs`）增加 `\` → `/` 替换，变为三步：去尾部分隔符 → 统一为正斜杠 → Windows 下转小写。同时将可见性改为 `pub(crate)`（OpenCode/OpenClaw 解析器需引用）。

**对 Claude / Codex 的无影响论证**：归一化始终同时作用于匹配的两侧（进程 cwd 侧与会话记录侧），同规归一化不改变相等性判定；Claude 的目录名转换 `convert_path_to_dir_name` 将所有非字母数字字符映射为 `-`，与分隔符方向无关；`project_name_from_path` 已兼容两种分隔符；Windows 下 git 命令接受正斜杠路径。

**既有测试更新**：`normalize_cwd_tests` 中关于保留反斜杠的断言需同步改为期望正斜杠输出。

### 2. OpenCode 匹配重构（`opencode_parser.rs`）

匹配优先级调整为：

1. **主匹配（新增）**：按 `session.directory` 精确匹配——查询最近 200 条会话（`ORDER BY time_updated DESC LIMIT 200`），在 Rust 侧对 `session.directory` 与进程 cwd 分别归一化后比较相等，取 `time_updated` 最新且匹配的一条（归一化比较提取纯函数 `cwd_equivalent(a: &str, b: &str) -> bool` 便于单测）。
2. **回退匹配（保留）**：现有 project worktree 前缀规则（`cwd == worktree || cwd 在 worktree 之下`），两侧均归一化后比较。
3. **global 会话回退（保留）**：现有逻辑，传入的 cwd 做同样的归一化。

匹配语义提炼为纯函数并补充单元测试：正斜杠/反斜杠等价、尾部分隔符等价、Windows 大小写等价、Unix 大小写敏感、祖先/后代前缀关系。

### 3. OpenClaw 匹配同步修复（`openclaw_parser.rs`）

workspace 匹配接入同一归一化与纯函数，规则结构与 OpenCode 回退规则一致。

### 4. 明确不做（YAGNI）

- 不改动 OpenCode 的 SQLite 读取方式（只读连接、busy_timeout 维持现状）
- 不做跨工具的统一"会话匹配框架"抽象——三个解析器各自接入共享纯函数即可
- 不处理 OpenCode db 中 worktree 与 directory 均不匹配进程 cwd 的历史脏数据场景（自然落入无卡片，与 macOS 行为一致）

## 错误处理

- 数据库打开失败 / 查询失败：维持现状（debug 日志 + 返回空），不向用户报错
- 进程 cwd 为 None：落入 unmatched 路径，行为与 Claude/Codex 一致
- 所有新逻辑不得 panic：纯函数对空串、根路径、UNC 路径返回保守结果

## 测试策略

- 纯函数单测（`cwd_equivalent`、增强后的 `normalize_cwd_for_match`）：覆盖两平台分隔符/大小写/尾部分隔符组合，Unix 用例锁定大小写敏感不回归
- 集成回归：`cargo test` 全量通过；Claude/Codex 既有测试无变化
- 人工验证：验收场景 1-5 实机执行（Windows + 如有条件 macOS）
